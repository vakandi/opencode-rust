//! Shared, observable proxy state. Every field is atomically readable so the health
//! endpoint can report egress without touching the rotation loop.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::Upstream;

/// One session's held egress.
#[derive(Clone, Debug)]
pub struct SessionEgress {
    pub session_id: String,
    pub upstream: Upstream,
    pub pool_index: usize,
    pub adopted_at: i64,
}

pub struct ProxyState {
    pub cfg: Arc<crate::config::ProxyConfig>,
    pub pool_path: std::path::PathBuf,
    /// Per-session egress. The whole point of this map: sessions must not share an
    /// IP, so a single `current` cannot exist. Kept in sync with the claims file.
    pub sessions: Mutex<HashMap<String, SessionEgress>>,
    /// Fallback for connections that carry no session identity (MCP transports,
    /// tool subprocesses, anything not built by the per-session client factory).
    pub current: Mutex<Option<Upstream>>,
    pub claims: crate::claims::Claims,
    pub current_port: AtomicU16,
    pub last_switch: AtomicI64,
    pub next_switch: AtomicI64,
    pub tunnels_open: AtomicU64,
    pub tunnels_total: AtomicU64,
    pub rotations_total: AtomicU64,
    pub probes_total: AtomicU64,
    pub direct: AtomicBool,
    pub quarantine: Mutex<crate::quarantine::Quarantine>,
}

impl ProxyState {
    pub fn new(cfg: Arc<crate::config::ProxyConfig>, pool_path: std::path::PathBuf) -> Self {
        let keys = crate::pool::pool_keys(&crate::pool::load(&pool_path));
        let quarantine = crate::quarantine::Quarantine::load(
            &crate::pool::quarantine_file(),
            &keys,
            crate::quarantine::now_secs(),
        );
        Self {
            cfg,
            claims: crate::claims::Claims::new(crate::pool::claims_file()),
            pool_path,
            sessions: Mutex::new(HashMap::new()),
            current: Mutex::new(None),
            current_port: AtomicU16::new(0),
            last_switch: AtomicI64::new(0),
            next_switch: AtomicI64::new(0),
            tunnels_open: AtomicU64::new(0),
            tunnels_total: AtomicU64::new(0),
            rotations_total: AtomicU64::new(0),
            probes_total: AtomicU64::new(0),
            direct: AtomicBool::new(false),
            quarantine: Mutex::new(quarantine),
        }
    }

    pub fn current_upstream(&self) -> Option<Upstream> {
        self.current.lock().ok().and_then(|c| c.clone())
    }

    /// The upstream serving `session_id`, falling back to the process-wide one so a
    /// connection without a session identity still gets residential egress rather
    /// than a direct connection to the provider.
    pub fn upstream_for(&self, session_id: Option<&str>) -> Option<Upstream> {
        if let Some(id) = session_id {
            if let Ok(sessions) = self.sessions.lock() {
                if let Some(entry) = sessions.get(id) {
                    return Some(entry.upstream.clone());
                }
            }
        }
        self.current_upstream()
    }

    pub fn egress_for(&self, session_id: &str) -> Option<SessionEgress> {
        self.sessions.lock().ok().and_then(|s| s.get(session_id).cloned())
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.lock().map(|s| s.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn set_port(&self, port: u16) {
        self.current_port.store(port, Ordering::Relaxed);
    }

    /// Install a session's egress. Returns false when the map lock is poisoned, which
    /// must never take down a request that is otherwise fine.
    pub fn adopt_for_session(
        &self,
        session_id: &str,
        upstream: Upstream,
        pool_index: usize,
        at: i64,
    ) -> bool {
        match self.sessions.lock() {
            Ok(mut sessions) => {
                sessions.insert(
                    session_id.to_string(),
                    SessionEgress {
                        session_id: session_id.to_string(),
                        upstream,
                        pool_index,
                        adopted_at: at,
                    },
                );
                true
            }
            Err(_) => false,
        }
    }

    pub fn set_upstream(&self, upstream: Option<Upstream>) {
        if let Ok(mut slot) = self.current.lock() {
            *slot = upstream;
        }
    }

    /// Forget a session's claim so a stopped session stops holding a pool entry.
    pub fn release_session(&self, session_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(session_id);
        }
        let mut table = self.claims.load();
        table.remove(session_id);
        self.claims.save(&table);
    }

    /// Record a dead egress. Best-effort: a poisoned or contended lock must never take
    /// down a request that is already failing for another reason.
    pub fn note_dead_upstream(&self, key: &str, ttl: std::time::Duration) {
        let now = crate::quarantine::now_secs();
        if let Ok(mut q) = self.quarantine.lock() {
            q.quarantine(key, ttl, now);
            let _ = q.save(&crate::pool::quarantine_file(), &crate::pool::pool_keys(&crate::pool::load(&self.pool_path)), now);
        }
    }

    pub fn active_quarantine(&self) -> HashSet<String> {
        self.quarantine.lock().map(|q| q.active(crate::quarantine::now_secs())).unwrap_or_default()
    }

    pub fn claimed_indices(&self) -> HashSet<usize> {
        self.claims.load().taken_indices()
    }

    /// Bypass every upstream and egress on this host's real IP. Takes effect on the
    /// next connection, so a VPN can be turned on without restarting the server.
    pub fn set_direct(&self, direct: bool) {
        self.direct.store(direct, Ordering::Relaxed);
    }

    pub fn is_direct(&self) -> bool {
        self.direct.load(Ordering::Relaxed)
    }

    pub fn rotations_total(&self) -> u64 {
        self.rotations_total.load(Ordering::Relaxed)
    }

    pub fn probes_total(&self) -> u64 {
        self.probes_total.load(Ordering::Relaxed)
    }

    pub fn note_tunnel_opened(&self) {
        self.tunnels_open.fetch_add(1, Ordering::Relaxed);
        self.tunnels_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_rotation(&self, at: i64, next: i64) {
        self.rotations_total.fetch_add(1, Ordering::Relaxed);
        self.last_switch.store(at, Ordering::Relaxed);
        self.next_switch.store(next, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let upstream = self.current_upstream();
        let sessions = self.sessions.lock().map(|s| s.values().cloned().collect::<Vec<_>>()).unwrap_or_default();
        let mut per_session: Vec<serde_json::Value> = sessions
            .iter()
            .map(|e| {
                serde_json::json!({
                    "session_id": e.session_id,
                    "upstream": e.upstream.key(),
                    "pool_index": e.pool_index,
                    "adopted_at": e.adopted_at,
                })
            })
            .collect();
        per_session.sort_by(|a, b| {
            a["session_id"].as_str().cmp(&b["session_id"].as_str())
        });
        let distinct: HashSet<String> = sessions.iter().map(|e| e.upstream.key()).collect();
        let (claim_rows, claim_entries) = crate::claims::summarise(&self.claims.load());
        serde_json::json!({
            "enabled": self.cfg.enabled,
            "port": self.current_port.load(Ordering::Relaxed),
            "upstream": upstream.as_ref().map(|u| u.key()),
            "upstream_authenticated": upstream.map(|u| u.has_credentials()).unwrap_or(false),
            "last_switch": self.last_switch.load(Ordering::Relaxed),
            "next_switch": self.next_switch.load(Ordering::Relaxed),
            "rotate_interval_secs": self.cfg.rotate_interval.as_secs(),
            "tunnels_open": self.tunnels_open.load(Ordering::Relaxed),
            "tunnels_total": self.tunnels_total.load(Ordering::Relaxed),
            "rotations_total": self.rotations_total.load(Ordering::Relaxed),
            "probes_total": self.probes_total.load(Ordering::Relaxed),
            "quarantined": self.quarantine.lock().map(|q| q.len()).unwrap_or(0),
            "direct": self.direct.load(Ordering::Relaxed),
            "sessions": per_session,
            "session_count": sessions.len(),
            // Distinct egress IPs actually in use. Equal to session_count is the
            // healthy state; fewer means sessions are sharing an IP.
            "distinct_session_upstreams": distinct.len(),
            "claims": claim_rows,
            "claimed_entries": claim_entries,
        })
    }
}