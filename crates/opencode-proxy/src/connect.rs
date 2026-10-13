//! Dialing the upstream and opening a tunnel through it.
//!
//! Extracted and shared by the probe and the listener so both classify failures the
//! same way. The important detail is `leftover`: bytes the upstream sent after its
//! `\r\n\r\n` header must be handed to the caller, or the first bytes of a TLS
//! handshake are lost and the connection dies in a way that looks like a bad IP.

use std::time::Duration;

use bytes::{Buf as _, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::ProxyConfig;
use crate::Upstream;

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("timeout")]
    Timeout,
    #[error("socket: {0}")]
    Socket(String),
    #[error("closed before status line")]
    ClosedBeforeStatus,
    #[error("upstream refused: {status} {reason}")]
    Refused { status: u16, reason: String },
}

impl ConnectError {
    /// A 407 means the credentials are wrong for this IP, which is a different
    /// problem from a dead IP and gets a much shorter quarantine.
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, ConnectError::Refused { status: 407, .. })
    }
}

pub struct Tunnel {
    pub stream: TcpStream,
    /// Everything the upstream wrote after the response header.
    pub leftover: BytesMut,
}

pub fn authority_parts(target: &str) -> Option<(String, u16)> {
    // Bracket-aware: `rsplit_once(':')` then strip `[]`, so `[::1]:443` works.
    // The reference used `split(":")` and would produce a host of "[".
    let (host, port) = if let Some(rest) = target.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':')?;
        (h.to_string(), port)
    } else {
        let (h, p) = target.rsplit_once(':')?;
        (h.to_string(), p)
    };
    let port: u16 = port.parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

async fn read_head(stream: &mut TcpStream) -> Result<BytesMut, ConnectError> {
    let mut buf = BytesMut::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.map_err(|e| ConnectError::Socket(e.to_string()))?;
        if read == 0 {
            return Err(ConnectError::ClosedBeforeStatus);
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(buf);
        }
        if buf.len() > 16 * 1024 {
            return Err(ConnectError::ClosedBeforeStatus);
        }
    }
}

fn parse_status(head: &[u8]) -> Result<(u16, String), ConnectError> {
    let text = String::from_utf8_lossy(head);
    let first = text.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let _version = parts.next();
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if status == 0 {
        return Err(ConnectError::ClosedBeforeStatus);
    }
    let reason = parts.collect::<Vec<_>>().join(" ");
    Ok((status, reason))
}

/// One CONNECT attempt. No retries — the retry loop lives in
/// `open_tunnel_with_retries` so the probe can call the single attempt directly.
pub async fn open_tunnel_once(
    upstream: &Upstream,
    target: &str,
    cfg: &ProxyConfig,
) -> Result<Tunnel, ConnectError> {
    let dial = TcpStream::connect((upstream.host.as_str(), upstream.port));
    let mut stream = tokio::time::timeout(cfg.upstream_timeout, dial)
        .await
        .map_err(|_| ConnectError::Timeout)?
        .map_err(|e| ConnectError::Socket(e.to_string()))?;
    let _ = stream.set_nodelay(true);

    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = upstream.proxy_authorization() {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| ConnectError::Socket(e.to_string()))?;
    stream.flush().await.map_err(|e| ConnectError::Socket(e.to_string()))?;

    let head = read_head(&mut stream).await?;
    let (status, reason) = parse_status(&head)?;
    if status != 200 {
        return Err(ConnectError::Refused { status, reason });
    }

    let header_end = head
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(head.len());
    let mut leftover = head;
    // Drop the response head, keep every byte the upstream sent after it.
    leftover.advance(header_end);
    Ok(Tunnel { stream, leftover })
}

/// The attempt loop the listener uses: N attempts with backoff, because a single
/// dead CONNECT should not surface to the client as a 502.
pub async fn open_tunnel_with_retries(
    upstream: &Upstream,
    target: &str,
    cfg: &ProxyConfig,
) -> Result<Tunnel, ConnectError> {
    let mut last = ConnectError::ClosedBeforeStatus;
    for attempt in 0..cfg.tunnel_attempts {
        match open_tunnel_once(upstream, target, cfg).await {
            Ok(tunnel) => return Ok(tunnel),
            Err(error) => {
                let fatal = error.is_auth_failure();
                last = error;
                if fatal {
                    // Wrong credentials will fail identically on every retry.
                    break;
                }
                if attempt + 1 < cfg.tunnel_attempts {
                    let backoff = cfg
                        .tunnel_backoff_ms
                        .get(attempt as usize)
                        .copied()
                        .unwrap_or(400);
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                }
            }
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_host_port() {
        assert_eq!(authority_parts("example.com:443"), Some(("example.com".into(), 443)));
    }

    #[test]
    fn parses_bracketed_ipv6() {
        assert_eq!(authority_parts("[::1]:443"), Some(("::1".into(), 443)));
    }

    #[test]
    fn rejects_malformed_target() {
        assert_eq!(authority_parts("noport"), None);
        assert_eq!(authority_parts("host:notaport"), None);
    }

    #[test]
    fn auth_failure_is_only_407() {
        assert!(ConnectError::Refused { status: 407, reason: String::new() }.is_auth_failure());
        assert!(!ConnectError::Refused { status: 502, reason: String::new() }.is_auth_failure());
        assert!(!ConnectError::Timeout.is_auth_failure());
    }
}