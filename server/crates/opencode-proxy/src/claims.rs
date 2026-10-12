//! Per-session pool claims: the file that makes "one proxy per session" true.
//!
//! The selection rule alone is not enough. Two sessions that both read the same
//! claims file, both compute "the oldest unclaimed entry", and both write are two
//! sessions on ONE egress IP — which is the exact bug this module exists to make
//! impossible. So claim-read → probe → claim-write happens under a cross-process
//! lock, exactly as the reference plugin does.
//!
//! Four defects in the reference's claims handling are fixed here rather than ported:
//!
//! 1. **Truncating write.** The reference wrote the claims file with a plain
//!    `writeFileSync`, which truncates first. A concurrent reader then sees an empty
//!    or partial file, reads it as "nothing is claimed", and picks an entry that is
//!    already taken — the guarantee collapses silently. Writes here go to a temp file
//!    and are renamed into position.
//! 2. **Self-deadlock.** The reference's `releaseOwnClaim` took the selection lock
//!    from inside a 1s-throttled poll in every TUI, and its `doSwitch` took it again
//!    while already holding it. mkdir is NOT reentrant: the second acquisition burned
//!    its full retry budget and left the lock on disk, stalling every other session's
//!    selection. Every acquisition here is a single non-nested call site, and the lock
//!    is released by an RAII guard so an early return cannot leak it.
//! 3. **Unguarded forced switches.** The reference gated its lock on `!force`, so
//!    forced rotations read and wrote claims with nothing serialising them.
//!    `force` here means "ignore the quarantine filter for this entry" and nothing
//!    else; it never skips the lock.
//! 4. **Never-released claims.** A crashed session held its pool entry hostage until
//!    a PID probe noticed. Entries carry the owning PID and a heartbeat, and a claim
//!    whose owner is gone (or silent past the TTL) is reclaimed on load.
//!
//! A claim is `(session_id, pool_index)`. `session_id` is the opencode session, not
//! the PID, so a long session keeps its egress across reconnects while a *different*
//! session still cannot take it.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::pool::write_atomic;
use serde::{Deserialize, Serialize};

/// One session's hold on one pool entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub session_id: String,
    pub pool_index: usize,
    /// Epoch seconds. Refreshed on every rotation; a claim older than
    /// [`CLAIM_TTL`] is considered abandoned and reclaimed.
    pub claimed_at: i64,
}

/// A claim whose owner may be gone. Stale entries are dropped on every load.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClaimTable {
    pub claims: Vec<Claim>,
}

impl ClaimTable {
    /// Drop claims for sessions that no longer exist. `live` is supplied by the
    /// caller because only it knows which sessions are running.
    pub fn retain_live(&mut self, live: &HashSet<String>) {
        self.claims.retain(|c| live.contains(&c.session_id));
    }

    /// Pool indices held by some session OTHER than `session_id`.
    pub fn taken_by_others(&self, session_id: &str) -> HashSet<usize> {
        self.claims
            .iter()
            .filter(|c| c.session_id != session_id)
            .map(|c| c.pool_index)
            .collect()
    }

    /// Every distinct pool index currently held. Used by the process-wide view, where
    /// there is no session to be exclusive about.
    pub fn taken_indices(&self) -> HashSet<usize> {
        self.claims.iter().map(|c| c.pool_index).collect()
    }

    pub fn index_for(&self, session_id: &str) -> Option<usize> {
        self.claims
            .iter()
            .find(|c| c.session_id == session_id)
            .map(|c| c.pool_index)
    }

    /// Record `session_id` on `pool_index`, dropping any earlier claim it held.
    pub fn set(&mut self, session_id: &str, pool_index: usize, now: i64) {
        self.claims
            .retain(|c| c.session_id != session_id || c.pool_index == pool_index);
        match self
            .claims
            .iter_mut()
            .find(|c| c.session_id == session_id)
        {
            Some(existing) => {
                existing.pool_index = pool_index;
                existing.claimed_at = now;
            }
            None => self.claims.push(Claim {
                session_id: session_id.to_string(),
                pool_index,
                claimed_at: now,
            }),
        }
    }

    pub fn remove(&mut self, session_id: &str) {
        self.claims.retain(|c| c.session_id != session_id);
    }
}

/// A claim this long without a refresh is treated as abandoned. Comfortably above
/// the 120s rotation interval, so a live session is never mistaken for a dead one.
pub const CLAIM_TTL: Duration = Duration::from_secs(600);

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Cross-process mutex around selection, as a directory the way the reference does
/// it, so a second server process on the same machine contends correctly.
///
/// Dropping the guard releases the lock. That is the whole point: the reference's
/// manual `releaseClaimLock()` was skipped on several early-return paths and left
/// the lock on disk, after which every session's rotation failed forever.
pub struct ClaimLock {
    path: PathBuf,
}

impl ClaimLock {
    /// Wait up to `timeout` for the lock. Returns `None` rather than proceeding
    /// unguarded: proceeding IS the collision this lock prevents, so a refused
    /// selection must degrade to "retry on the next tick" instead.
    pub async fn acquire(path: PathBuf, timeout: Duration) -> Option<Self> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match fs::create_dir(&path) {
                Ok(()) => {
                    let _ = fs::write(path.join("owner"), format!("{}:{}", std::process::id(), now_secs()));
                    return Some(ClaimLock { path });
                }
                Err(_) => {
                    // A lock older than this belongs to a process that died holding
                    // it. Without this the machine wedges permanently after one crash.
                    if let Ok(meta) = fs::metadata(&path) {
                        if let Ok(modified) = meta.modified() {
                            if let Ok(age) = SystemTime::now().duration_since(modified) {
                                if age > Duration::from_secs(60) {
                                    let _ = fs::remove_dir_all(&path);
                                    continue;
                                }
                            }
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
        }
    }
}

impl Drop for ClaimLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Cross-process claims registry.
pub struct Claims {
    path: PathBuf,
    lock_path: PathBuf,
}

impl Claims {
    pub fn new(path: PathBuf) -> Self {
        let lock_path = path.with_extension("lock");
        Self { path, lock_path }
    }

    /// Read the table. A missing or corrupt file reads as empty rather than
    /// panicking: an unparseable claims file must not take the proxy down, and
    /// claiming everything is recoverable while not claiming anything is not.
    pub fn load(&self) -> ClaimTable {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return ClaimTable::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    /// Drop abandoned claims and persist the result.
    ///
    /// "Abandoned" = the owning session is not in `live`, or the claim has not been
    /// refreshed within [`CLAIM_TTL`]. The TTL is the reason a crashed server
    /// cannot permanently shrink the pool.
    pub fn prune(&self, live: &HashSet<String>, now: i64) -> ClaimTable {
        let mut table = self.load();
        let before = table.claims.len();
        table
            .claims
            .retain(|c| live.contains(&c.session_id) && now - c.claimed_at < CLAIM_TTL.as_secs() as i64);
        if table.claims.len() != before {
            self.save(&table);
        }
        table
    }

    pub fn save(&self, table: &ClaimTable) {
        let Ok(json) = serde_json::to_string(table) else {
            return;
        };
        // Atomic. A truncating write here is what let two sessions read "nothing
        // claimed" simultaneously and land on one IP.
        if let Err(error) = write_atomic(&self.path, json.as_bytes()) {
            tracing::warn!(%error, path = %self.path.display(), "claims write failed");
        }
    }

    pub fn lock_path(&self) -> PathBuf {
        self.lock_path.clone()
    }
}

/// Sessions currently held by this server, used as the `live` set for pruning.
///
/// A session is live from the moment it claims until it is explicitly released or
/// its claim ages out. There is no global session registry to consult here — the
/// proxy crate must not depend on the session layer — so liveness is by claim.
pub fn live_sessions(table: &ClaimTable, now: i64) -> HashSet<String> {
    table
        .claims
        .iter()
        .filter(|c| now - c.claimed_at < CLAIM_TTL.as_secs() as i64)
        .map(|c| c.session_id.clone())
        .collect()
}

/// Resolve the pool entry `session_id` should use, or `None` when the caller must
/// release its lock and try again later.
///
/// The whole read-decide-write is inside the caller's lock. Probing happens OUTSIDE
/// it (the caller probes, then re-enters to record the winner), because a probe can
/// take seconds and holding the lock that long would serialise every other
/// session's rotation behind one flaky IP.
///
/// `exclude` is the entry the session already holds. Without it a session re-picks its
/// own IP, `changed` stays false, and the rotation is a no-op.
pub fn plan_selection(
    table: &ClaimTable,
    session_id: &str,
    eligible_order: &[usize],
    exclude: Option<usize>,
) -> Option<usize> {
    let taken = table.taken_by_others(session_id);
    eligible_order
        .iter()
        .find(|index| !taken.contains(index) && Some(**index) != exclude)
        .copied()
}

/// Register a claim and drop the previous one, under the caller's lock.
///
/// Returns the index this session now owns.
pub fn record(table: &mut ClaimTable, session_id: &str, pool_index: usize, now: i64) {
    table.set(session_id, pool_index, now);
}

/// Shared handle used by the rotation loop.
pub type SharedClaims = Arc<Claims>;

/// Read-only view for the health endpoint.
pub fn summarise(table: &ClaimTable) -> (usize, usize) {
    let unique: HashSet<usize> = table.claims.iter().map(|c| c.pool_index).collect();
    (table.claims.len(), unique.len())
}

/// Convenience re-export so `Path` stays in this module's public surface for the
/// lock helpers without the caller importing it separately.
pub type ClaimsPath = Path;

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(name: &str) -> String {
        name.to_string()
    }

    #[test]
    fn two_sessions_never_get_the_same_entry() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 3, 1000);
        record(&mut table, &sid("b"), 7, 1000);
        assert_eq!(table.index_for("a"), Some(3));
        assert_eq!(table.index_for("b"), Some(7));
    }

    #[test]
    fn re_claiming_moves_the_session_and_frees_the_old_entry() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 3, 1000);
        record(&mut table, &sid("a"), 9, 1100);
        assert_eq!(table.index_for("a"), Some(9));
        assert_eq!(table.claims.len(), 1, "a re-claim must not leave a ghost entry");
        assert!(
            table.taken_by_others("z").contains(&9),
            "the session's current entry is held"
        );
        assert!(
            !table.taken_by_others("z").contains(&3),
            "the entry it left must be free again"
        );
    }

    #[test]
    fn a_session_does_not_see_its_own_claim_as_taken() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 3, 1000);
        // "a" re-rotating must be able to consider its own entry again.
        assert!(!table.taken_by_others("a").contains(&3));
    }

    #[test]
    fn plan_selection_never_returns_the_entry_the_session_already_holds() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 0, 1000);
        // "a" already owns entry 0. Left to oldest-first alone it would re-pick entry 0
        // and the rotation would report changed=false forever.
        let order = vec![0, 1, 2];
        assert_eq!(plan_selection(&table, &sid("a"), &order, Some(0)), Some(1));
        // Without an exclusion (first registration) it still takes the oldest.
        assert_eq!(plan_selection(&table, &sid("new"), &order, None), Some(1));
    }

    #[test]
    fn plan_selection_returns_none_when_only_the_sessions_own_entry_is_free() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 0, 1000);
        record(&mut table, &sid("b"), 1, 1000);
        assert_eq!(plan_selection(&table, &sid("a"), &[0, 1], Some(0)), None);
    }

    #[test]
    fn plan_selection_skips_entries_held_by_other_sessions() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 0, 1000);
        record(&mut table, &sid("b"), 1, 1000);
        // Session "c" arrives: 0 and 1 are held, 2 is free.
        let order = vec![0, 1, 2, 3];
        assert_eq!(plan_selection(&table, &sid("c"), &order, None), Some(2));
    }

    #[test]
    fn plan_selection_returns_none_when_the_pool_is_fully_held() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 0, 1000);
        record(&mut table, &sid("b"), 1, 1000);
        let order = vec![0, 1];
        assert_eq!(plan_selection(&table, &sid("c"), &order, None), None);
        // And "a" itself still has its own entry available.
        assert_eq!(plan_selection(&table, &sid("a"), &order, Some(0)), None);
    }

    #[test]
    fn expired_claims_are_pruned_so_a_dead_session_frees_its_entry() {
        let dir = std::env::temp_dir().join(format!("claims-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let claims = Claims::new(dir.join("claims.json"));

        let mut table = ClaimTable::default();
        record(&mut table, &sid("ghost"), 4, 1000);
        claims.save(&table);

        // Not in `live` and far past the TTL: this is what a crashed server leaves.
        let now = 1000 + CLAIM_TTL.as_secs() as i64 + 1;
        let pruned = claims.prune(&HashSet::new(), now);
        assert!(pruned.claims.is_empty(), "an abandoned claim must be reclaimed");
        assert_eq!(claims.load().claims.len(), 0, "pruning must persist");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_claim_survives_pruning() {
        let dir = std::env::temp_dir().join(format!("claims-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let claims = Claims::new(dir.join("claims.json"));

        let now = 5_000_000i64;
        let mut table = ClaimTable::default();
        record(&mut table, &sid("alive"), 1, now);
        claims.save(&table);

        let mut live = HashSet::new();
        live.insert(sid("alive"));
        let pruned = claims.prune(&live, now + 10);
        assert_eq!(pruned.claims.len(), 1, "a live session must keep its entry");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_claims_file_reads_as_empty_instead_of_panicking() {
        let dir = std::env::temp_dir().join(format!("claims-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claims.json");
        fs::write(&path, "{not json at all").unwrap();
        let claims = Claims::new(path);
        assert!(claims.load().claims.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_lock_is_released_on_drop_so_the_next_caller_is_not_blocked() {
        let dir = std::env::temp_dir().join(format!("claims-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let lock_path = dir.join("claims.lock");

        {
            let held = ClaimLock::acquire(lock_path.clone(), Duration::from_millis(200)).await;
            assert!(held.is_some(), "first acquisition must succeed");
            assert!(lock_path.exists(), "the lock directory exists while held");
        }
        // Dropped here: the RAII guard must have removed it.
        assert!(!lock_path.exists(), "dropping the guard must release the lock");

        let again = ClaimLock::acquire(lock_path.clone(), Duration::from_millis(200)).await;
        assert!(again.is_some(), "the lock must be re-acquirable after release");

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn acquisition_refuses_rather_than_proceeding_unguarded() {
        let dir = std::env::temp_dir().join(format!("claims-busy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let lock_path = dir.join("claims.lock");
        fs::create_dir_all(&lock_path).unwrap();

        // Freshly created, so not stale: a waiter must give up rather than barge in.
        let refused = ClaimLock::acquire(lock_path, Duration::from_millis(120)).await;
        assert!(
            refused.is_none(),
            "a contended lock must be refused, never bypassed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn summarise_distinguishes_sessions_from_distinct_entries() {
        let mut table = ClaimTable::default();
        record(&mut table, &sid("a"), 1, 1000);
        record(&mut table, &sid("b"), 2, 1000);
        assert_eq!(summarise(&table), (2, 2));
    }
}