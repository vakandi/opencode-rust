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