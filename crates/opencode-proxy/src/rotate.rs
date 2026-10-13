//! Per-session rotation. Every `rotate_interval` each live session re-selects the
//! oldest pool entry nobody else holds, probe-gated, and adopts it.
//!
//! Invariants, each one a bug that shipped in the reference plugin:
//!
//! - **Sessions never share an IP.** Selection reads the claims file and writes it
//!   back under a cross-process lock, and refuses rather than proceeding unguarded.
//!   Without that, N sessions booting together all read "nothing claimed", all pick
//!   the same oldest entry, and all land on one egress.
//! - **Current egress is never left unset.** If every probe fails, the session keeps
//!   the upstream it has. Blanking it produces a blackout.
//! - **Only a successful adoption stamps `|last:`.** Stamping a probe that then failed
//!   advances the LRU clock on an IP that never worked.
//! - **Rotation never kills a live tunnel.** The reference's `killOldTunnels` cut every
//!   in-flight SSE response, which raised "socket connection was closed unexpectedly",
//!   which rotated again, which cut again — a self-sustaining failure loop.
//! - **Direct mode stops rotation.** A user who turned the proxy off to use their own
//!   VPN must not have pool entries burned behind their back.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::claims::ClaimLock;
use crate::probe::{probe_upstream, ProbeOutcome};
use crate::quarantine::now_secs;
use crate::selection::order_candidates;
use crate::state::ProxyState;

/// How long a session waits for the cross-process selection lock. Generous, because
/// refusing is correct and waiting is cheap: a refused selection retries next tick.
const SELECTION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Adopt the oldest unheld upstream for `session_id`. Returns true if the session's
/// egress changed.
pub async fn rotate_session(state: &Arc<ProxyState>, session_id: &str) -> bool {
    if state.is_direct() {
        return false;
    }
    let cfg = Arc::clone(&state.cfg);
    let entries = crate::pool::load(&state.pool_path);
    if entries.is_empty() {
        tracing::warn!(pool = %state.pool_path.display(), session = %session_id, "proxy pool empty; keeping this session's egress");
        return false;
    }

    let quarantined = state.active_quarantine();
    let order = order_candidates(&entries, &quarantined, &state.claimed_indices(), None);
    if order.is_empty() {
        tracing::warn!(session = %session_id, "every pool entry is quarantined");
        return false;
    }

    let candidates = order.clone();
    let Some(lock) = ClaimLock::acquire(state.claims.lock_path(), SELECTION_LOCK_TIMEOUT).await else {
        tracing::error!(session = %session_id, "selection lock unavailable — refusing to switch (would collide with another session)");
        return false;
    };

    // Re-read the table under the lock: another session may have claimed an entry
    // while this one waited, which is exactly what makes the oldest-first pick stale.
    let mut table = state.claims.prune(&crate::claims::live_sessions(&state.claims.load(), now_secs()), now_secs());
    let current = state.egress_for(session_id).map(|e| e.pool_index);
    let Some(target) = crate::claims::plan_selection(&table, session_id, &candidates, current) else {
        drop(lock);
        tracing::info!(session = %session_id, "no other entry available to rotate to — keeping current egress");
        return false;
    };
    let entry = entries[target].clone();
    let upstream = entry.upstream();
    state.probes_total.fetch_add(1, Ordering::Relaxed);

    // RESERVE before probing: otherwise N sessions arriving together each pick the same
    // free entry, each burns the probe budget on it, and all but one end up empty.
    crate::claims::record(&mut table, session_id, target, now_secs());
    state.claims.save(&table);
    drop(lock);

    match probe_upstream(&upstream, &cfg).await {
        ProbeOutcome::Ok => {}
        outcome => {
            if outcome.earns_quarantine() {
                state.note_dead_upstream(&entry.key(), cfg.short_quarantine_ttl);
            }
            // Give the entry back: it failed, and holding it would keep a dead IP out
            // of every other session's rotation.
            release_reservation(state, session_id).await;
            tracing::info!(session = %session_id, upstream = %entry.key(), ?outcome, "candidate rejected by probe; trying the next");
            return rotate_after_failed_probe(state, session_id, &quarantined).await;
        }
    }

    if let Err(error) = crate::pool::stamp(&state.pool_path, &entry.host, entry.port, now_secs()) {
        tracing::warn!(%error, "could not persist |last: stamp");
    }
    let changed = state.egress_for(session_id).map(|e| e.upstream.key()) != Some(entry.key());
    state.adopt_for_session(session_id, upstream, target, now_secs());
    state.note_rotation(now_secs(), now_secs() + cfg.rotate_interval.as_secs() as i64);
    tracing::info!(session = %session_id, upstream = %entry.key(), pool_index = target, changed, "session egress adopted");
    changed
}

/// Drop a session's claim so a failed selection does not hold a pool entry hostage.
async fn release_reservation(state: &Arc<ProxyState>, session_id: &str) {
    let Some(_lock) = ClaimLock::acquire(state.claims.lock_path(), SELECTION_LOCK_TIMEOUT).await else {
        tracing::warn!(session = %session_id, "could not release the failed reservation — the entry stays held until the claim TTL expires");
        return;
    };
    let mut table = state.claims.load();
    table.remove(session_id);
    state.claims.save(&table);
}

/// Second pass after the oldest candidate failed its probe.
///
/// The lock is released first and the table re-read, because retrying while holding
/// it would serialise every other session behind this one's dead IP. Bounded by the
/// probe budget, never by the pool size.
async fn rotate_after_failed_probe(
    state: &Arc<ProxyState>,
    session_id: &str,
    already_quarantined: &std::collections::HashSet<String>,
) -> bool {
    let cfg = Arc::clone(&state.cfg);
    let entries = crate::pool::load(&state.pool_path);
    let current_key = state.egress_for(session_id).map(|e| e.upstream.key());
    let mut order = order_candidates(&entries, already_quarantined, &state.claimed_indices(), None);
    order.retain(|index| Some(entries[*index].key()) != current_key);
    order.truncate(cfg.max_probes_per_switch);

    for index in order {
        let entry = entries[index].clone();
        state.probes_total.fetch_add(1, Ordering::Relaxed);

        let Some(lock) = ClaimLock::acquire(state.claims.lock_path(), SELECTION_LOCK_TIMEOUT).await else {
            return false;
        };
        let mut table = state.claims.prune(&crate::claims::live_sessions(&state.claims.load(), now_secs()), now_secs());
        if table.taken_by_others(session_id).contains(&index) {
            drop(lock);
            continue;
        }
        // Reserve before probing, as in the primary path.
        crate::claims::record(&mut table, session_id, index, now_secs());
        state.claims.save(&table);
        drop(lock);

        match probe_upstream(&entry.upstream(), &cfg).await {
            ProbeOutcome::Ok => {}
            outcome => {
                if outcome.earns_quarantine() {
                    state.note_dead_upstream(&entry.key(), cfg.short_quarantine_ttl);
                }
                tracing::info!(session = %session_id, upstream = %entry.key(), ?outcome, "fallback candidate rejected");
                release_reservation(state, session_id).await;
                continue;
            }
        }

        let _ = crate::pool::stamp(&state.pool_path, &entry.host, entry.port, now_secs());
        let changed = state.egress_for(session_id).map(|e| e.upstream.key()) != Some(entry.key());
        state.adopt_for_session(session_id, entry.upstream(), index, now_secs());
        state.note_rotation(now_secs(), now_secs() + cfg.rotate_interval.as_secs() as i64);
        tracing::info!(session = %session_id, upstream = %entry.key(), pool_index = index, changed, "session egress adopted after probe fallback");
        return changed;
    }
    tracing::warn!(session = %session_id, "no candidate passed a probe; keeping the session's current egress");
    false
}

/// Adopt the process-wide fallback egress, for connections that carry no session
/// identity. Mirrors the per-session path so a startup with no sessions still has a
/// working proxy rather than direct egress.
pub async fn rotate_once(state: &Arc<ProxyState>) -> bool {
    let cfg = Arc::clone(&state.cfg);
    let entries = crate::pool::load(&state.pool_path);
    if entries.is_empty() {
        tracing::warn!(pool = %state.pool_path.display(), "proxy pool empty; staying on current egress");
        return false;
    }

    let quarantined = state.active_quarantine();
    let claimed = state.claimed_indices();
    let order = order_candidates(&entries, &quarantined, &claimed, manual_index());
    if order.is_empty() {
        tracing::warn!(
            pool_entries = entries.len(),
            quarantined = quarantined.len(),
            "no usable upstream: every pool entry is quarantined. Traffic is going DIRECT from this machine."
        );
        return false;
    }

    let now = now_secs();
    for index in order.iter().take(cfg.max_probes_per_switch) {
        let entry = &entries[*index];
        let upstream = entry.upstream();
        state.probes_total.fetch_add(1, Ordering::Relaxed);

        match probe_upstream(&upstream, &cfg).await {
            ProbeOutcome::Ok => {}
            outcome => {
                // A timeout or a transient refusal must NOT be stamped as `last:` (this
                // IP carried no traffic) and must NOT burn a quarantine slot. Only a
                // 407 is deterministic enough to set aside; everything else is this
                // pool being flaky, which is the normal case, not an exception.
                if outcome.earns_quarantine() {
                    state.note_dead_upstream(&entry.key(), cfg.short_quarantine_ttl);
                }
                tracing::debug!(upstream = %entry.key(), ?outcome, "candidate rejected by probe");
                continue;
            }
        }

        // Adopted. Stamp, count, and announce.
        if let Err(error) = crate::pool::stamp(&state.pool_path, &entry.host, entry.port, now) {
            tracing::warn!(%error, "could not persist |last: stamp");
        }
        let changed = state.current_upstream().map(|prev| prev.key()) != Some(entry.key());
        state.set_upstream(Some(upstream));
        state.note_rotation(now, now + cfg.rotate_interval.as_secs() as i64);
        tracing::info!(upstream = %entry.key(), changed, "proxy upstream adopted");
        return changed;
    }

    tracing::warn!("all probed candidates failed; keeping current egress");
    false
}

fn manual_index() -> Option<usize> {
    let raw = std::env::var("OPENCODE_PROXY_FORCE_INDEX").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.parse::<usize>() {
        Ok(index) => Some(index),
        Err(_) => {
            tracing::warn!(value = %trimmed, "ignoring unparseable OPENCODE_PROXY_FORCE_INDEX");
            None
        }
    }
}

/// Give a session its first egress. Called when a session's provider client is built,
/// so the very first provider request already has a probed IP.
pub async fn ensure_session(state: &Arc<ProxyState>, session_id: &str) -> bool {
    if state.egress_for(session_id).is_some() {
        return false;
    }
    rotate_session(state, session_id).await
}

/// Spawn the periodic loop. Each tick rotates every live session.
pub fn spawn(state: Arc<ProxyState>) {
    let cfg = Arc::clone(&state.cfg);
    tokio::spawn(async move {
        if rotate_once(&state).await {
            tracing::info!("initial proxy egress selected");
        }
        // `interval()` fires its FIRST tick immediately, which combined with the
        // adoption above rotated twice within seconds of boot and discarded a working
        // egress. Start the schedule one full interval out instead.
        let start = tokio::time::Instant::now() + cfg.rotate_interval;
        let mut ticker = tokio::time::interval_at(start, cfg.rotate_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            rotate_all(&state).await;
        }
    });
}

/// Sequential, not concurrent: N sessions probing at once would contend for one
/// claims file, which is the collision the lock exists to prevent.
pub async fn rotate_all(state: &Arc<ProxyState>) {
    if state.is_direct() {
        tracing::debug!("rotation suppressed fleet-wide: direct mode is on");
        return;
    }
    let sessions = state.session_ids();
    if sessions.is_empty() {
        rotate_once(state).await;
        return;
    }
    let mut rotated = 0;
    for session_id in sessions {
        if rotate_session(state, &session_id).await {
            rotated += 1;
        }
    }
    tracing::info!(sessions = state.session_ids().len(), rotated, "per-session rotation tick complete");
}

/// Reap idle tunnels. Deliberately does NOT touch tunnels that are mid-stream: the
/// reference's `killOldTunnels` severed live SSE responses on every rotation, which is
/// the single most user-visible bug in the plugin.
pub fn spawn_idle_reaper(state: Arc<ProxyState>) {
    let interval = Duration::from_secs(5);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            // No tunnel registry exists yet: `copy_bidirectional` owns tunnel lifetime
            // and returns when either side closes. Nothing to reap, and nothing to kill.
            // This tick exists so the invariant "rotation never touches live tunnels"
            // has an explicit home rather than being implied by absence.
            let _ = state.tunnels_open.load(Ordering::Relaxed);
        }
    });
}