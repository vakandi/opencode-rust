//! Per-session HTTP clients.
//!
//! Every provider used to call `Client::new()` in its constructor, which snapshots the
//! proxy environment ONCE at registry build time. Every session then shared that one
//! client, so every session egressed from the same residential IP no matter how many
//! proxies the pool held.
//!
//! `reqwest` reads the proxy from a client-level config, not per request, so the only
//! way to give a session its own egress is to give it its own `Client`. Clients are
//! therefore cached per session and built against that session's proxy URL.
//!
//! ## Why the client survives rotation
//!
//! The proxy URL points at the local listener's port, and rotation changes which
//! upstream that port tunnels to — the port never changes. So a client's proxy URL stays
//! valid across every rotation, and a session never needs a new client. That is what
//! makes a per-session cache cheap rather than a leak.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use reqwest::Client;

/// The proxy URL this process published, and the port it listens on.
static PROXY_PORT: OnceLock<u16> = OnceLock::new();

/// Register the listening port. Must run before any client is built: a client built
/// before this point would snapshot the environment without our proxy and egress
/// directly, which is the "the proxy never activated" failure.
pub fn set_proxy_port(port: u16) {
    let _ = PROXY_PORT.set(port);
}

pub fn proxy_port() -> Option<u16> {
    PROXY_PORT.get().copied()
}

/// True when a local proxy is available to route through.
pub fn proxy_active() -> bool {
    PROXY_PORT.get().is_some_and(|port| *port > 0)
}

/// One client per session, built on first use.
#[derive(Default)]
pub struct SessionClients {
    clients: Mutex<HashMap<String, Arc<Client>>>,
}

/// Cached clients outlive the session only as long as the map entry; an entry is
/// dropped when its session is released, so a long-lived server does not accumulate
/// one client per session ever seen.
static CLIENTS: OnceLock<SessionClients> = OnceLock::new();

fn registry() -> &'static SessionClients {
    CLIENTS.get_or_init(SessionClients::default)
}

/// Build the client for `session_id`, or a shared fallback when there is no proxy.
///
/// The fallback is a single process-wide client: connections with no session identity
/// (MCP transports, tool subprocesses) must still work, and they are not session-scoped.
pub fn for_session(session_id: &str) -> Arc<Client> {
    let Some(port) = proxy_port().filter(|p| *p > 0) else {
        return shared_direct();
    };
    // An empty id makes the URL "http://s-:@…"; the listener then holds the CONNECT
    // open until the caller gives up, so the turn dies as a network error.
    if session_id.is_empty() {
        return shared_direct();
    }
    let mut guard = registry().clients.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(existing) = guard.get(session_id) {
        return Arc::clone(existing);
    }
    let client = Arc::new(build(&session_proxy_url(port, session_id)));
    guard.insert(session_id.to_string(), Arc::clone(&client));
    client
}

fn session_proxy_url(port: u16, session_id: &str) -> String {
    // The session id travels in the URL's user half; the proxy decodes it and routes
    // the tunnel to that session's own upstream. The `s-` prefix is what tells the
    // listener "this is a session identity"; without it the id reads as garbage and
    // the request silently falls back to the process-wide egress.
    let user = percent_encode(&format!("s-{session_id}"));
    format!("http://{user}:@127.0.0.1:{port}")
}

fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn build(proxy_url: &str) -> Client {
    let exclusions = reqwest::NoProxy::from_string("127.0.0.1,localhost,::1").unwrap_or_default();
    match reqwest::Proxy::all(proxy_url) {
        Ok(proxy) => builder().proxy(proxy.no_proxy(Some(exclusions))).build().unwrap_or_else(|_| Client::new()),
        Err(_) => builder().build().unwrap_or_else(|_| Client::new()),
    }
}

fn builder() -> reqwest::ClientBuilder {
    Client::builder()
        // Provider responses are long SSE streams. A read timeout short enough to catch
        // a dead IP would also cut a healthy stream mid-answer.
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(30))
}

fn shared_direct() -> Arc<Client> {
    static DIRECT: OnceLock<Arc<Client>> = OnceLock::new();
    DIRECT.get_or_init(|| Arc::new(Client::new())).clone()
}

/// Drop a finished session's client so its pool entry and connection pool are freed.
pub fn release_session(session_id: &str) {
    if let Ok(mut guard) = registry().clients.lock() {
        guard.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a port, every session collapses onto one shared client. Building does not
    /// dial, so an unbound port is fine here.
    fn ensure_port_registered() {
        set_proxy_port(19499);
    }

    #[test]
    fn session_urls_carry_the_session_id_in_the_user_half() {
        let url = session_proxy_url(19400, "ses_abc");
        assert_eq!(url, "http://s-ses_abc:@127.0.0.1:19400");
        let authority = url.trim_start_matches("http://").split('@').next().unwrap();
        let (user, password) = authority.split_once(':').unwrap();
        assert_eq!(user, "s-ses_abc", "the proxy only reads an id that carries its s- marker");
        assert_eq!(password, "");
    }

    #[test]
    fn unsafe_characters_are_encoded_so_the_url_cannot_be_broken() {
        // A colon in the id would otherwise split the userinfo at the wrong place and
        // silently route to the wrong session.
        let url = session_proxy_url(19400, "ses_a:b@c");
        let authority = url.trim_start_matches("http://").split('@').next().unwrap();
        let (user, password) = authority.rsplit_once(':').unwrap();
        assert_eq!(user, "s-ses_a%3Ab%40c", "both the colon and the @ must be encoded");
        assert_eq!(password, "");
    }

    #[test]
    fn an_empty_session_id_never_builds_a_proxied_url() {
        ensure_port_registered();
        // "http://s-:@…" made the listener hold the CONNECT open until the call timed
        // out, so any turn sent without a session id failed as a network error.
        let direct = Arc::as_ptr(&shared_direct());
        assert!(Arc::ptr_eq(&for_session(""), &shared_direct()));
        assert_eq!(direct, Arc::as_ptr(&shared_direct()), "the fallback must be the shared client");
    }

    #[test]
    fn a_session_reuses_one_client_across_calls() {
        ensure_port_registered();
        let a = for_session("ses_x");
        let b = for_session("ses_x");
        assert!(Arc::ptr_eq(&a, &b), "the same session must reuse one client");
    }

    #[test]
    fn different_sessions_get_different_clients() {
        ensure_port_registered();
        let a = for_session("ses_one");
        let b = for_session("ses_two");
        assert!(!Arc::ptr_eq(&a, &b), "two sessions must not share one client");
    }

    #[test]
    fn releasing_a_session_drops_its_client() {
        ensure_port_registered();
        let before = for_session("ses_release_me");
        release_session("ses_release_me");
        let after = for_session("ses_release_me");
        assert!(!Arc::ptr_eq(&before, &after), "a released session must rebuild its client");
    }
}