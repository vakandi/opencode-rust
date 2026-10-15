//! Probe-gated adoption.
//!
//! ## Why this is a full HTTPS request and not a CONNECT handshake
//!
//! The previous probe opened a CONNECT tunnel and accepted a `200` as proof the proxy
//! was alive. That proves nothing: a residential proxy whose quota is spent, or that has
//! blacklisted the destination, still completes the handshake and then silently drops
//! every byte. The old code adopted those proxies, the health endpoint showed a green
//! upstream, and every real provider request failed with `error sending request` — a
//! logged-and-fine state that produced nothing.
//!
//! So the probe now performs a real end-to-end request through the tunnel and accepts
//! only a real HTTP response. It costs one TLS handshake per candidate, which is the
//! price of telling "the proxy answered" apart from "the proxy works".
//!
//! The failure stays graded, because the consequence for the pool differs sharply: a 407
//! is deterministic, while a reset or a timeout says the IP is blocked right now and may
//! well work again shortly.

use crate::config::ProxyConfig;
use crate::Upstream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    Ok,
    /// Credentials rejected by this upstream. Deterministic: retrying cannot help.
    RejectedAuth,
    /// Dead, blocked, or refusing right now. May be transient.
    Unreachable,
    /// Slow past the probe budget. Says nothing about long-term health.
    TimedOut,
}

impl ProbeOutcome {
    /// All failure modes quarantine, with the SHORT TTL.
    ///
    /// A 407 is deterministic. A timeout is not — this pool is residential and a slow IP
    /// can be fine a minute later — but on a pool with many dead entries, ignoring
    /// timeouts makes oldest-first re-probe the same dead entries every tick and never
    /// advance past them. Short TTL, so a recovering IP returns on its own.
    pub fn earns_quarantine(self) -> bool {
        matches!(
            self,
            ProbeOutcome::RejectedAuth | ProbeOutcome::Unreachable | ProbeOutcome::TimedOut
        )
    }
}

fn client_for(upstream: &Upstream, cfg: &ProxyConfig) -> reqwest::Result<reqwest::Client> {
    let mut proxy = reqwest::Proxy::all(format!("http://{}:{}", upstream.host, upstream.port))?;
    if upstream.has_credentials() {
        proxy = proxy.basic_auth(&upstream.user, &upstream.pass);
    }
    reqwest::Client::builder()
        .proxy(proxy)
        .timeout(cfg.probe_timeout)
        .connect_timeout(cfg.probe_timeout)
        .build()
}

/// Issue one real request through the proxy and classify the result.
pub async fn probe_upstream(upstream: &Upstream, cfg: &ProxyConfig) -> ProbeOutcome {
    let target = cfg.probe_target.clone();
    let url = if target.starts_with("http") {
        target
    } else {
        format!("https://{target}")
    };

    let client = match client_for(upstream, cfg) {
        Ok(client) => client,
        Err(error) => {
            tracing::info!(upstream = %upstream.key(), %error, "probe FAILED: client build");
            return ProbeOutcome::Unreachable;
        }
    };

    match client.get(&url).send().await {
        Ok(response) => {
            let status = response.status();
            // The status line is the whole verdict; draining the body would pull
            // megabytes of model JSON on every rotation.
            drop(response);
            if status.is_success() || status.is_redirection() {
                tracing::info!(upstream = %upstream.key(), %url, status = %status, "probe OK");
                ProbeOutcome::Ok
            } else if status.as_u16() == 407 {
                tracing::info!(upstream = %upstream.key(), "probe FAILED: credentials rejected (407)");
                ProbeOutcome::RejectedAuth
            } else {
                // The tunnel carried real bytes, so the proxy works — but this exit is
                // blocked or out of quota for this destination right now.
                tracing::info!(upstream = %upstream.key(), status = %status, "probe FAILED: destination refused this exit");
                ProbeOutcome::Unreachable
            }
        }
        Err(error) => {
            if error.status().map(|s| s.as_u16()) == Some(407) {
                tracing::info!(upstream = %upstream.key(), "probe FAILED: credentials rejected (407)");
                return ProbeOutcome::RejectedAuth;
            }
            tracing::info!(upstream = %upstream.key(), %error, "probe FAILED: request did not complete");
            ProbeOutcome::Unreachable
        }
    }
}

/// Whether an exit delivers a streaming body incrementally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamOutcome {
    /// The body arrived progressively: the first chunk landed well before the last.
    Streams,
    /// The whole body was released in one piece, so a turn can only ever render complete.
    Buffers,
    /// The check could not conclude — rate limited, refused, or no bytes at all.
    Unknown,
}

impl StreamOutcome {
    /// Only a conclusive verdict may act on an upstream. An unknown verdict is the
    /// common case on a free tier that answers 429 to a probe, and treating that as
    /// "buffers" would quarantine a fleet of working exits.
    pub fn is_conclusive(self) -> bool {
        matches!(self, StreamOutcome::Streams | StreamOutcome::Buffers)
    }
}

/// Ask the exit for a streamed answer and time the chunks.
///
/// A residential proxy that completes CONNECT and TLS can still hold the response body
/// and release it in one piece when the upstream goes quiet. Every request then succeeds
/// and the UI paints the finished turn, which is indistinguishable from a model that
/// answers instantly — and the `/models` probe cannot see it, because that body is small
/// enough that any working proxy returns it whole.
pub async fn probe_streams(upstream: &Upstream, cfg: &ProxyConfig) -> StreamOutcome {
    if !cfg.stream_probe_enabled || cfg.stream_probe_key.is_empty() {
        return StreamOutcome::Unknown;
    }
    let client = match client_for(upstream, cfg) {
        Ok(client) => client,
        Err(_) => return StreamOutcome::Unknown,
    };
    let url = format!("https://{}", cfg.stream_probe_target.trim_start_matches("https://"));
    let body = serde_json::json!({
        "model": cfg.stream_probe_model,
        "messages": [{"role": "user", "content": "Count from 1 to 10."}],
        "stream": true,
        "max_tokens": 24,
    });

    let mut response = match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", cfg.stream_probe_key))
        .header("Accept", "text/event-stream")
        .timeout(cfg.stream_probe_timeout)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return StreamOutcome::Unknown,
    };
    if !response.status().is_success() {
        // 429 included: the exit works, the free tier is simply busy right now.
        tracing::debug!(upstream = %upstream.key(), status = %response.status(), "stream probe inconclusive");
        return StreamOutcome::Unknown;
    }

    // `Response::chunk()` rather than `bytes_stream()`: this crate has no futures
    // dependency, and one chunk at a time is all the timing needs.
    let mut content_seen = false;
    let mut chunks = 0usize;
    let mut first_byte: Option<std::time::Instant> = None;
    let mut last_byte: Option<std::time::Instant> = None;
    loop {
        let bytes = match response.chunk().await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(_) => break,
        };
        let now = std::time::Instant::now();
        first_byte.get_or_insert(now);
        last_byte = Some(now);
        if !content_seen && bytes.windows(6).any(|w| w == b"content") {
            content_seen = true;
            chunks += 1;
        }
    }
    if chunks < 2 || !content_seen {
        return StreamOutcome::Unknown;
    }
    let (Some(first), Some(last)) = (first_byte, last_byte) else {
        return StreamOutcome::Unknown;
    };
    let spread = last.duration_since(first);
    if spread >= cfg.stream_probe_min_spread {
        tracing::info!(upstream = %upstream.key(), spread_ms = spread.as_millis() as u64, "stream probe: incremental");
        StreamOutcome::Streams
    } else {
        tracing::info!(upstream = %upstream.key(), spread_ms = spread.as_millis() as u64, "stream probe: body released at once");
        StreamOutcome::Buffers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_success_escapes_quarantine() {
        assert!(!ProbeOutcome::Ok.earns_quarantine());
        assert!(ProbeOutcome::RejectedAuth.earns_quarantine());
        assert!(ProbeOutcome::Unreachable.earns_quarantine());
        assert!(ProbeOutcome::TimedOut.earns_quarantine());
    }

    #[tokio::test]
    async fn a_refused_proxy_never_probes_ok() {
        // The regression this module exists for: a CONNECT handshake alone used to be
        // enough. A port that refuses instantly must not report success.
        let mut cfg = ProxyConfig::default();
        cfg.probe_timeout = std::time::Duration::from_millis(800);
        let dead = Upstream::new("127.0.0.1", 1, "u", "p");
        assert_ne!(probe_upstream(&dead, &cfg).await, ProbeOutcome::Ok);
    }
}