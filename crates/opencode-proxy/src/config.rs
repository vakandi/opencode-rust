//! Tunables. Every default is traceable to the reference plugin, and the two that
//! Wael changed deliberately are called out.

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ProxyConfig {
    /// Wael: rotate every 2 minutes. The reference shipped 240s.
    pub rotate_interval: Duration,
    /// Deliberately DISJOINT from the TS fleet's 18888-19387. The desktop shell's
    /// proxy sits at 18891, inside that range, so sharing it would race the other
    /// fleet for the same slots.
    pub default_port: u16,
    pub port_scan_count: u16,
    pub upstream_timeout: Duration,
    pub probe_timeout: Duration,
    pub probe_target: String,
    pub max_probes_per_switch: usize,
    pub tunnel_attempts: u32,
    pub tunnel_backoff_ms: [u64; 2],
    pub tunnel_idle: Duration,
    pub quarantine_ttl: Duration,
    pub short_quarantine_ttl: Duration,
    pub enabled: bool,
    pub debug: bool,
    /// Whether an adopted upstream must also be shown to deliver a streamed body
    /// progressively. Off by default: the check costs one real completion per
    /// candidate, and on a free tier that answers 429 to probes it concludes nothing.
    pub stream_probe_enabled: bool,
    pub stream_probe_target: String,
    pub stream_probe_model: String,
    /// Credential for the streaming probe. Empty means the probe cannot authenticate
    /// and therefore never concludes — which is the safe default, because a verdict it
    /// cannot earn must not be allowed to quarantine a working exit.
    pub stream_probe_key: String,
    pub stream_probe_timeout: Duration,
    /// Below this gap between the first and last body chunk the response counts as
    /// having been released in one piece, which is what breaks live streaming.
    pub stream_probe_min_spread: Duration,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            rotate_interval: Duration::from_secs(120),
            default_port: 19400,
            port_scan_count: 500,
            upstream_timeout: Duration::from_secs(15),
            // A proxy that genuinely reaches the API answers in ~5s. A 4s deadline
            // discarded working exits, which is how a healthy pool looked dead.
            probe_timeout: Duration::from_secs(10),
            // The real API endpoint: probing the bare host passes for a proxy that then
            // fails every model call.
            probe_target: "opencode.ai/zen/v1/models".to_string(),
            // The rotation budget must fit inside rotate_interval or sessions queue
            // behind each other and the fleet stops rotating on schedule.
            // N sessions x (probes_per_switch x probe_timeout) < rotate_interval.
            // 8 x 4s = 32s per session leaves room for a handful of sessions per tick.
            max_probes_per_switch: 8,
            tunnel_attempts: 3,
            tunnel_backoff_ms: [150, 400],
            tunnel_idle: Duration::from_secs(120),
            quarantine_ttl: Duration::from_secs(24 * 60 * 60),
            short_quarantine_ttl: Duration::from_secs(10 * 60),
            enabled: true,
            debug: false,
            stream_probe_enabled: false,
            stream_probe_target: "opencode.ai/zen/v1/chat/completions".to_string(),
            stream_probe_model: "space-bunny-free".to_string(),
            stream_probe_key: String::new(),
            stream_probe_timeout: Duration::from_secs(30),
            stream_probe_min_spread: Duration::from_millis(250),
        }
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).filter(|v| *v > 0).unwrap_or(default)
}

impl ProxyConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if std::env::var("OPENCODE_PROXY_DISABLE").is_ok() {
            cfg.enabled = false;
        }
        if std::env::var("OPENCODE_PROXY_DEBUG").is_ok() {
            cfg.debug = true;
        }
        if std::env::var("OPENCODE_PROXY_STREAM_PROBE").is_ok() {
            cfg.stream_probe_enabled = true;
        }
        if let Ok(key) = std::env::var("OPENCODE_PROXY_STREAM_KEY") {
            cfg.stream_probe_key = key.trim().to_string();
        }
        if let Ok(model) = std::env::var("OPENCODE_PROXY_STREAM_MODEL") {
            let model = model.trim();
            if !model.is_empty() {
                cfg.stream_probe_model = model.to_string();
            }
        }
        cfg.rotate_interval = Duration::from_secs(env_u64("OPENCODE_PROXY_ROTATE_SECS", 120));
        cfg.upstream_timeout = Duration::from_millis(env_u64(
            "OPENCODE_PROXY_UPSTREAM_TIMEOUT_MS",
            15_000,
        ));
        cfg.probe_timeout = Duration::from_millis(env_u64("OPENCODE_PROXY_PROBE_TIMEOUT_MS", 10_000));
        if let Ok(target) = std::env::var("OPENCODE_PROXY_PROBE_TARGET") {
            let target = target.trim();
            if !target.is_empty() {
                cfg.probe_target = target.to_string();
            }
        }
        // A port outside the unprivileged range, or one already reserved, falls back
        // rather than erroring: a bad env value must not stop the server booting.
        let requested = env_u64("OPENCODE_PROXY_PORT", cfg.default_port as u64);
        if (1024..65536).contains(&requested) {
            cfg.default_port = requested as u16;
        }
        cfg
    }

    /// The local proxy must never point at loopback, or the control plane's calls to
    /// our own server would recurse through it.
    pub fn no_proxy_value() -> &'static str {
        "127.0.0.1,localhost,::1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_interval_is_two_minutes_by_default() {
        assert_eq!(ProxyConfig::default().rotate_interval, Duration::from_secs(120));
    }

    #[test]
    fn default_port_does_not_collide_with_the_ts_fleet() {
        // The TS plugin scans 18888-19387 and the desktop proxy lives at 18891.
        let port = ProxyConfig::default().default_port;
        assert!(port >= 19400, "port {port} overlaps the TS fleet range");
    }

    #[test]
    fn no_proxy_covers_ipv6_loopback() {
        assert!(ProxyConfig::no_proxy_value().contains("::1"));
    }
}