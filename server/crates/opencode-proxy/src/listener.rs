//! The local HTTP/CONNECT forward proxy.
//!
//! Raw tokio + httparse rather than hyper, for one decisive reason: **the body is
//! never parsed.** After rewriting the request head, both directions are a blind
//! `copy_bidirectional`. Provider traffic is large chunked SSE POSTs, so
//! chunked-encoding, `Content-Length`, `Expect: 100-continue` and multi-minute streams
//! all pass through because nothing interprets them. hyper's CONNECT upgrade also
//! makes the bytes a client already pipelined past the CONNECT header awkward to
//! recover, and `leftover` handling here is trivial because we own the buffer.
//!
//! Two behaviours from the reference are load-bearing and must not regress:
//!
//! - The client's **first byte is written only on success**. A speculative
//!   `200 Connection Established` turns a failed rotation into a client that believes
//!   it has a tunnel.
//! - Rotation **never kills live tunnels**. The old `killOldTunnels` severed every
//!   SSE stream on every rotation; only idle tunnels are reaped.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::ProxyConfig;
use crate::connect::{open_tunnel_with_retries, ConnectError};
use crate::state::ProxyState;
use crate::Upstream;

const MAX_HEAD_BYTES: usize = 64 * 1024;

fn is_bypass_host(host: &str) -> bool {
    // Accept either a bare host ("::1", "localhost") or an authority
    // ("localhost:1234", "[::1]:8080"). Getting this wrong sends a loopback
    // connection through a residential proxy, where it cannot succeed.
    let trimmed = host.trim();
    let bare = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if let Some((head, port)) = trimmed.rsplit_once(':') {
        // Only treat the tail as a port when there is exactly one colon: a bare IPv6
        // literal like "::1" has several, and must survive whole or it becomes "::".
        if port.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() && !head.contains(':') {
            head
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    matches!(bare, "localhost" | "127.0.0.1" | "::1")
}

/// Split `host:port` for a CONNECT target, bracket-aware.
fn split_authority(target: &str) -> Option<(String, u16)> {
    crate::connect::authority_parts(target)
}

async fn read_head(socket: &mut TcpStream) -> Result<Vec<u8>, ()> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    loop {
        match socket.read(&mut chunk).await {
            Ok(0) => return Err(()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return Err(()),
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(buf);
        }
        if buf.len() > MAX_HEAD_BYTES {
            return Err(());
        }
    }
}

/// Byte offset just past the `\r\n\r\n` that ends the request head.
fn header_end(head: &[u8]) -> usize {
    head.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(head.len())
}

/// The inbound `Host:` value, if present.
fn head_host(head: &[u8]) -> &str {
    text_head_value(head, "host").unwrap_or("")
}

/// The inbound `Proxy-Authorization:` value, if present.
fn head_proxy_authorization(head: &[u8]) -> Option<&str> {
    text_head_value(head, "proxy-authorization")
}

fn text_head_value<'a>(head: &'a [u8], name: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.lines().skip(1) {
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                return Some(value.trim());
            }
        }
    }
    None
}

/// `None` = no session identity (MCP transports, tool subprocesses). Those use the
/// shared egress; treating it as an error would 502 them.
fn request_session(head: &[u8]) -> Option<String> {
    crate::session::session_from_proxy_authorization(head_proxy_authorization(head))
}

async fn respond_and_close(socket: &mut TcpStream, status: &str) {
    let _ = socket
        .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
        .await;
    let _ = socket.flush().await;
    let _ = socket.shutdown().await;
}

/// CONNECT straight to the origin, bypassing every upstream.
async fn dial_direct(
    client: &mut TcpStream,
    head: &[u8],
    consumed: usize,
    host: &str,
    port: u16,
    cfg: &ProxyConfig,
) {
    // Bounded, so a dead destination fails fast with 502 instead of holding the
    // socket until the OS TCP timeout.
    let dial = tokio::time::timeout(cfg.upstream_timeout, TcpStream::connect((host, port))).await;
    match dial {
        Ok(Ok(mut remote)) => {
            if client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_err() {
                return;
            }
            let _ = client.flush().await;
            let _ = remote.write_all(&head[consumed..]).await;
            let _ = remote.flush().await;
            let _ = tokio::io::copy_bidirectional(client, &mut remote).await;
        }
        _ => respond_and_close(client, "502 Bad Gateway").await,
    }
}

/// `TunnelKey` is opaque to the listener; the registry lives in state.rs.
pub async fn handle_connection(socket: TcpStream, cfg: Arc<ProxyConfig>, state: Arc<ProxyState>) {
    let _ = socket.set_nodelay(true);
    let mut socket = socket;
    let Ok(head) = read_head(&mut socket).await else {
        respond_and_close(&mut socket, "400 Bad Request").await;
        return;
    };

    let mut headers = [httparse::EMPTY_HEADER; 96];
    let mut request = httparse::Request::new(&mut headers);
    let parsed = match request.parse(&head) {
        Ok(httparse::Status::Complete(_)) => Ok(()),
        _ => Err(()),
    };
    if parsed.is_err() {
        respond_and_close(&mut socket, "400 Bad Request").await;
        return;
    }

    let method = request.method.unwrap_or("");
    let target = request.path.unwrap_or("").to_string();
    let consumed = header_end(&head);

    if method.eq_ignore_ascii_case("CONNECT") {
        handle_connect(socket, head, consumed, target, cfg, state).await;
        return;
    }
    handle_plain(socket, head, consumed, target, cfg, state).await;
}

async fn handle_connect(
    mut client: TcpStream,
    head: Vec<u8>,
    consumed: usize,
    target: String,
    cfg: Arc<ProxyConfig>,
    state: Arc<ProxyState>,
) {
    let Some((host, port)) = split_authority(&target) else {
        respond_and_close(&mut client, "400 Bad Request").await;
        return;
    };
    let origin = format!("{host}:{port}");
    let session = request_session(&head);

    // Bypass: loopback and direct mode go straight out, never through an upstream.
    if is_bypass_host(&host) || state.direct.load(Ordering::Relaxed) {
        dial_direct(&mut client, &head, consumed, &host, port, &cfg).await;
        return;
    }

    // No usable upstream: pass through DIRECT rather than 502. The 502 is what killed
    // every agent run while /health still reported a healthy server.
    let Some(upstream) = state.upstream_for(session.as_deref()) else {
        tracing::warn!(target = %origin, "no upstream available — passing through directly");
        dial_direct(&mut client, &head, consumed, &host, port, &cfg).await;
        return;
    };

    match dial_through(&upstream, &origin, &cfg, &state).await {
        Ok((mut remote, leftover)) => {
            // First byte to the client ONLY now that the upstream confirmed.
            if client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.is_err() {
                return;
            }
            if client.flush().await.is_err() {
                return;
            }
            if !leftover.is_empty() && client.write_all(&leftover).await.is_err() {
                return;
            }
            if remote.write_all(&head[consumed..]).await.is_err() {
                return;
            }
            let _ = remote.flush().await;
            // Non-destructive: this tunnel is registered and left alone by rotation.
            let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
        }
        Err(error) => {
            tracing::debug!(%error, upstream = %upstream.key(), "CONNECT failed");
            respond_and_close(&mut client, "502 Bad Gateway").await;
        }
    }
}

async fn dial_through(
    upstream: &Upstream,
    origin: &str,
    cfg: &ProxyConfig,
    state: &ProxyState,
) -> Result<(TcpStream, Vec<u8>), ConnectError> {
    match open_tunnel_with_retries(upstream, origin, cfg).await {
        Ok(tunnel) => {
            let leftover = tunnel.leftover.to_vec();
            Ok((tunnel.stream, leftover))
        }
        Err(error) => {
            // A dead upstream is worth remembering so the next rotation skips it.
            state.note_dead_upstream(&upstream.key(), cfg.short_quarantine_ttl);
            Err(error)
        }
    }
}

async fn handle_plain(
    mut client: TcpStream,
    head: Vec<u8>,
    consumed: usize,
    target: String,
    cfg: Arc<ProxyConfig>,
    state: Arc<ProxyState>,
) {
    let mut absolute = target.clone();
    if !absolute.starts_with("http://") && !absolute.starts_with("https://") {
        let host = head_host(&head);
        if host.is_empty() {
            respond_and_close(&mut client, "400 Bad Request").await;
            return;
        }
        absolute = format!("http://{host}{absolute}");
    }

    let after_scheme = absolute.split_once("://").map(|(_, rest)| rest).unwrap_or(&absolute);
    let authority_host = after_scheme.split('/').next().unwrap_or("").to_string();
    let dest_host = split_authority(&authority_host)
        .map(|(h, p)| (h, p))
        .unwrap_or_else(|| (authority_host.clone(), 80u16));
    let session = request_session(&head);

    if is_bypass_host(&dest_host.0) || state.direct.load(Ordering::Relaxed) {
        let path = match absolute.split_once("://").and_then(|(_, r)| r.split_once('/')) {
            Some((_, tail)) => format!("/{tail}"),
            _ => "/".to_string(),
        };
        let head_out = rewrite_head(&head[..consumed], &path, None);
        match TcpStream::connect((dest_host.0.as_str(), dest_host.1)).await {
            Ok(mut remote) => {
                let _ = remote.write_all(&head_out).await;
                let _ = remote.write_all(&head[consumed..]).await;
                let _ = remote.flush().await;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
            }
            _ => respond_and_close(&mut client, "502 Bad Gateway").await,
        }
        return;
    }

    // Same reasoning as CONNECT: direct beats 502 when no upstream exists.
    let Some(upstream) = state.upstream_for(session.as_deref()) else {
        tracing::warn!(target = %absolute, "no upstream available — passing through directly");
        match tokio::time::timeout(cfg.upstream_timeout, TcpStream::connect((dest_host.0.as_str(), dest_host.1))).await {
            Ok(Ok(mut remote)) => {
                let path = absolute.split_once("://").and_then(|(_, r)| r.split_once('/')).map(|(_, t)| format!("/{t}")).unwrap_or_else(|| "/".to_string());
                let head_out = rewrite_head(&head[..consumed], &path, None);
                let _ = remote.write_all(&head_out).await;
                let _ = remote.write_all(&head[consumed..]).await;
                let _ = remote.flush().await;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
            }
            _ => respond_and_close(&mut client, "502 Bad Gateway").await,
        }
        return;
    };

    // Absolute-form target: that is what a forward proxy must send upstream.
    let auth = upstream.proxy_authorization();
    let head_out = rewrite_head(&head[..consumed], &absolute, auth.as_deref());

    match tokio::time::timeout(cfg.upstream_timeout, TcpStream::connect((upstream.host.as_str(), upstream.port))).await {
        Ok(Ok(mut remote)) => {
            let _ = remote.write_all(&head_out).await;
            let _ = remote.write_all(&head[consumed..]).await;
            let _ = remote.flush().await;
            let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
        }
        _ => {
            state.note_dead_upstream(&upstream.key(), cfg.short_quarantine_ttl);
            respond_and_close(&mut client, "502 Bad Gateway").await;
        }
    }
}

/// Rebuild the request head for the upstream.
///
/// `Host:` is rewritten to the DESTINATION, never the upstream — a forward proxy that
/// sends its own address as Host breaks virtual hosting. Inbound
/// `Proxy-Authorization` and `Proxy-Connection` are stripped so our credentials are
/// never forwarded, and ours is added only when we actually have one.
fn rewrite_head(head: &[u8], request_target: &str, auth: Option<&str>) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut lines: Vec<String> = Vec::new();
    let method = text.lines().next().unwrap_or_default().split_whitespace().next().unwrap_or("GET").to_string();
    lines.push(format!("{method} {request_target} HTTP/1.1"));
    for line in text.lines().skip(1) {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("proxy-authorization:") || lower.starts_with("proxy-connection:") {
            continue;
        }
        if lower.starts_with("host:") {
            lines.push(format!("Host: {request_target}"));
            continue;
        }
        lines.push(line.to_string());
    }
    if let Some(auth) = auth {
        lines.push(format!("Proxy-Authorization: {auth}"));
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n\r\n");
    out.into_bytes()
}

/// Bind the first free port in the scan range. Exhaustion is NOT an error: it means
/// degraded, and the caller must leave the environment alone rather than point it at a
/// dead port — that mistake is the instant-failure bug in the reference.
pub async fn bind_listener(cfg: &ProxyConfig) -> std::io::Result<(TcpListener, u16)> {
    for offset in 0..u32::from(cfg.port_scan_count) {
        let port = u32::from(cfg.default_port) + offset;
        // Range exhausted past the top of the port space: stop rather than hammer
        // the last port for the remainder of the scan.
        if port == 0 || port > u32::from(u16::MAX) {
            break;
        }
        let port = port as u16;
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            let bound = listener.local_addr()?.port();
            return Ok((listener, bound));
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "no free proxy port"))
}

pub fn local_url(port: AtomicU16) -> String {
    format!("http://127.0.0.1:{}", port.load(Ordering::Relaxed))
}

/// Live flag used by the accept loop's shutdown path.
pub fn accept_loop(listener: TcpListener, cfg: Arc<ProxyConfig>, state: Arc<ProxyState>) {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    let cfg = Arc::clone(&cfg);
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        handle_connection(socket, cfg, state).await;
                    });
                }
                // Transient accept errors (fd exhaustion, reset during teardown)
                // must not spin the CPU. There is no shutdown flag: the listener is
                // dropped when the process exits.
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypasses_loopback() {
        assert!(is_bypass_host("localhost:1234"));
        assert!(is_bypass_host("127.0.0.1"));
        assert!(is_bypass_host("::1"));
        assert!(!is_bypass_host("1.2.3.4"));
    }

    #[test]
    fn rewrite_strips_inbound_proxy_auth_and_rewrites_host() {
        let head = b"GET http://origin.example/x HTTP/1.1\r\nHost: origin.example\r\nProxy-Authorization: Basic Zm9v\r\nProxy-Connection: keep-alive\r\n\r\n";
        let out = rewrite_head(head, "http://origin.example/x", Some("Basic YmFy"));
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("GET http://origin.example/x HTTP/1.1\r\n"));
        assert!(text.contains("Host: http://origin.example/x"));
        // Exactly one Proxy-Authorization: ours, not the client's.
        assert_eq!(text.matches("Proxy-Authorization").count(), 1);
        assert!(text.contains("Basic YmFy"));
        assert!(!text.contains("Proxy-Connection"));
    }

    #[test]
    fn rewrite_adds_no_auth_header_when_absent() {
        let head = b"GET http://origin.example/x HTTP/1.1\r\nHost: origin.example\r\n\r\n";
        let out = rewrite_head(head, "http://origin.example/x", None);
        assert!(!String::from_utf8(out).unwrap().contains("Proxy-Authorization"));
    }
}