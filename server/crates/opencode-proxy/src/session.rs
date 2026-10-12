//! Carrying a session identity from the provider client to the proxy listener.
//!
//! The listener is one process-wide socket. A `CONNECT` request carries no session
//! identity of its own, so with a single shared client every session would egress on
//! one IP — the exact failure this crate exists to prevent.
//!
//! The fix uses the one channel a forward proxy already has: proxy credentials.
//! `http://ses_abc@127.0.0.1:19400` makes `reqwest` send
//! `Proxy-Authorization: Basic base64("ses_abc:")`, which the listener decodes back to
//! the session id. No new port, no header rewriting, and the listener already had to
//! parse the request head.
//!
//! ## Why credentials and not a custom header
//!
//! A custom header would need to be attached to every provider request, but the
//! credential travels on the proxy handshake itself, so it survives connection
//! pooling, keep-alive reuse and redirects for free. A pooled connection opened by
//! session A and reused by session B cannot happen: the pool is per-client, and the
//! client is per-session.
//!
//! ## The collision this depends on
//!
//! A session id that decodes to garbage must fall back to the process-wide egress
//! rather than 502. Refusing would break any client that has no session identity at
//! all — MCP transports, tool subprocesses — which are legitimately shared.

use base64::Engine as _;

/// Prefix marking one of our session credentials, so a real upstream credential can
/// never be mistaken for a session id (and vice versa).
const SESSION_PREFIX: &str = "s-";

/// Build the proxy URL a session's client must dial.
///
/// `s-<id>` must be ONE user half, not split across the colon: the decoder reads only
/// the user side.
pub fn proxy_url_for_session(port: u16, session_id: &str) -> String {
    let user = format!("{SESSION_PREFIX}{}", encode_component(session_id));
    format!("http://{user}:@127.0.0.1:{port}")
}

/// The process-wide URL for clients with no session identity.
pub fn proxy_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Percent-encode the characters that would break URL parsing. Session ids are
/// `ses_` + hex today, but an id from another source must not be able to inject a
/// port, an `@`, or a path.
fn encode_component(raw: &str) -> String {
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

fn decode_component(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = raw.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Recover the session id from an inbound `Proxy-Authorization` value.
///
/// Returns `None` for a missing, non-Basic, foreign, or malformed header — every one
/// of which must fall back to the shared egress rather than fail the request.
pub fn session_from_proxy_authorization(header: Option<&str>) -> Option<String> {
    let value = header?.trim();
    let encoded = value.strip_prefix("Basic ").or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let raw = String::from_utf8(decoded).ok()?;
    // The userinfo is `user:password`. Only the user half is ours; a password half is
    // ignored, so an upstream-shaped credential cannot be mistaken for a session.
    let user = raw.split(':').next()?;
    let user = decode_component(user)?;
    user.strip_prefix(SESSION_PREFIX).filter(|id| !id.is_empty()).map(|id| id.to_string())
}

/// The inbound header name, lowercased because our own parser is case-insensitive.
pub const PROXY_AUTHORIZATION: &str = "proxy-authorization";

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_for(session_id: &str) -> String {
        let url = proxy_url_for_session(19400, session_id);
        let rest = url.trim_start_matches("http://").split('@').next().unwrap();
        let (user, password) = rest.split_once(':').unwrap();
        let raw = format!("{user}:{password}");
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        )
    }

    #[test]
    fn a_session_id_round_trips_through_the_proxy_url() {
        let url = proxy_url_for_session(19400, "ses_01ABCdef");
        assert!(url.starts_with("http://s-ses_01ABCdef:@127.0.0.1:19400"), "{url}");
        assert_eq!(session_from_proxy_authorization(Some(&auth_for("ses_01ABCdef"))), Some("ses_01ABCdef".to_string()));
    }

    #[test]
    fn session_ids_with_url_unsafe_characters_survive() {
        // A colon in the id would otherwise split the userinfo at the wrong place.
        let id = "ses_a:b@c/d";
        assert_eq!(session_from_proxy_authorization(Some(&auth_for(id))), Some(id.to_string()));
    }

    #[test]
    fn a_foreign_credential_is_not_mistaken_for_a_session() {
        let raw = "user:realpassword";
        let header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        );
        assert_eq!(session_from_proxy_authorization(Some(&header)), None);
    }

    #[test]
    fn absent_or_malformed_headers_fall_back_rather_than_fail() {
        assert_eq!(session_from_proxy_authorization(None), None);
        assert_eq!(session_from_proxy_authorization(Some("")), None);
        assert_eq!(session_from_proxy_authorization(Some("Bearer token")), None);
        assert_eq!(session_from_proxy_authorization(Some("Basic !!!!not-base64!!!!")), None);
        assert_eq!(session_from_proxy_authorization(Some("Basic ")), None);
    }

    #[test]
    fn an_empty_session_id_is_refused() {
        // "s-" with nothing after it must not resolve to the empty-string session,
        // which would collide with every other anonymous client.
        let raw = "s-:";
        let header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        );
        assert_eq!(session_from_proxy_authorization(Some(&header)), None);
    }

    #[test]
    fn a_password_half_is_ignored_so_a_foreign_shape_cannot_inject() {
        let raw = "s-ses_1:anything-at-all";
        let header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        );
        assert_eq!(session_from_proxy_authorization(Some(&header)), Some("ses_1".to_string()));
    }

    #[test]
    fn the_bare_proxy_url_carries_no_session() {
        assert_eq!(proxy_url(19400), "http://127.0.0.1:19400");
    }
}