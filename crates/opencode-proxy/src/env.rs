//! Publishing our own address into the process environment.
//!
//! Why this module exists at all, in the words of the two upstream facts verified in
//! this session:
//!
//! - `reqwest::Config::default()` sets `auto_sys_proxy: true` (reqwest 0.12.28,
//!   `async_impl/client.rs:309`), and `ClientBuilder::build()` reads the system proxy
//!   matcher from the environment at that moment (`client.rs:419`).
//! - `hyper_util`'s `Matcher::from_env()` is **eager**: it calls `Builder::from_env().build()`
//!   (`proxy/matcher.rs:95`), and `Builder::from_env()` reads `REQUEST_METHOD`
//!   synchronously (`proxy/matcher.rs:230`).
//!
//! So the export must happen before ANY `reqwest::Client::new()`, and
//! `REQUEST_METHOD` must be handled: when it is set, hyper-util sets `is_cgi` and the
//! matcher goes **silent**, ignoring every proxy variable. A CGI-style leftover
//! `REQUEST_METHOD` in a server process therefore disables proxying with no error at all.

use std::env;

/// Set by [`export_proxy_env`] so `start.sh` and health can confirm ordering.
pub const EXPORTED_PORT_ENV: &str = "OPENCODE_PROXY_LISTEN_PORT";

/// Point the process at our local listener.
///
/// Returns the URL that was published, or `None` when disabled (in which case the
/// environment is left untouched — deliberately, so a disable is a true bypass rather
/// than a pointer at a port nothing is listening on).
pub fn export_proxy_env(port: u16) -> Option<String> {
    if port == 0 {
        return None;
    }

    // Is_cgi in hyper-util's matcher makes it ignore proxies entirely. Clearing it is
    // the fix; refusing to start would be the alternative and is worse.
    if env::var_os("REQUEST_METHOD").is_some() {
        tracing::warn!(
            "REQUEST_METHOD was set, which makes hyper-util ignore proxy variables; unsetting it for proxy resolution"
        );
        env::remove_var("REQUEST_METHOD");
    }

    let url = format!("http://127.0.0.1:{port}");
    // `ALL_PROXY` alone is not enough: reqwest's `Matcher::from_env` checks
    // lowercase first, and an inherited lowercase `all_proxy` would win over ours.
    for key in ["ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
        env::set_var(key, &url);
    }

    // Loopback must never be proxied, or the control plane's calls into this very
    // server would recurse back through this listener.
    let no_proxy = crate::config::ProxyConfig::no_proxy_value();
    for key in ["NO_PROXY", "no_proxy"] {
        env::set_var(key, no_proxy);
    }
    env::set_var(EXPORTED_PORT_ENV, port.to_string());

    Some(url)
}

/// Undo [`export_proxy_env`]. Used by tests so they do not leak a loopback proxy into
/// unrelated cases in the same process.
pub fn clear_proxy_env() {
    for key in ["ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
        env::remove_var(key);
    }
    env::remove_var(EXPORTED_PORT_ENV);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process environment is GLOBAL and `cargo test` runs tests in parallel
    /// threads. These three tests all read and write `ALL_PROXY` and friends, so
    /// without this lock they race: whichever runs last wins, and the losers see
    /// `NotPresent` from an unwrap on a variable their sibling just removed.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn publishes_all_proxy_variants_and_no_proxy() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let url = export_proxy_env(19400).expect("should export");
        assert_eq!(url, "http://127.0.0.1:19400");
        for key in ["ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
            assert_eq!(env::var(key).unwrap(), url, "{key} not published");
        }
        assert!(env::var("NO_PROXY").unwrap().contains("127.0.0.1"));
        assert_eq!(env::var(EXPORTED_PORT_ENV).unwrap(), "19400");
        clear_proxy_env();
    }

    #[test]
    fn clears_request_method_so_the_matcher_does_not_go_silent() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        env::set_var("REQUEST_METHOD", "GET");
        let _ = export_proxy_env(19401);
        assert!(env::var_os("REQUEST_METHOD").is_none(), "REQUEST_METHOD must be removed");
        clear_proxy_env();
    }

    #[test]
    fn zero_port_exports_nothing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        clear_proxy_env();
        assert!(export_proxy_env(0).is_none());
        assert!(env::var_os("ALL_PROXY").is_none());
    }
}