//! Dead / quota-exhausted IPs, with expiry.
//!
//! Two tiers, both from the reference: a full 24 h for an IP whose quota is spent
//! (it will still be spent tomorrow), and 10 minutes for a transient like a 407 or a
//! failed probe.
//!
//! The reference never garbage-collects, so the file grows forever whenever the pool
//! is swapped. Here a key survives only if it is BOTH still in the pool AND unexpired,
//! which bounds the file across pool swaps.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use std::collections::HashMap;

use crate::pool::write_atomic;

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Clone, Debug, Default)]
pub struct Quarantine {
    entries: HashMap<String, i64>,
}

impl Quarantine {
    pub fn load(path: &Path, pool_keys: &HashSet<String>, now: i64) -> Self {
        let raw = std::fs::read_to_string(path).unwrap_or_default();
        let parsed: HashMap<String, i64> = serde_json::from_str(&raw).unwrap_or_default();
        // GC on every load: an entry for an IP that left the pool is dead weight, and
        // an expired entry is dead weight whether or not the pool changed.
        let entries = parsed
            .into_iter()
            .filter(|(key, expiry)| pool_keys.contains(key) && *expiry > now)
            .collect();
        Self { entries }
    }

    pub fn active(&self, now: i64) -> HashSet<String> {
        self.entries.iter().filter(|(_, e)| **e > now).map(|(k, _)| k.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn quarantine(&mut self, key: &str, ttl: Duration, now: i64) {
        if key.is_empty() || key.ends_with(":0") {
            return;
        }
        self.entries.insert(key.to_string(), now + ttl.as_secs() as i64);
    }

    pub fn clear(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub fn save(&self, path: &Path, pool_keys: &HashSet<String>, now: i64) -> std::io::Result<()> {
        let filtered: HashMap<&String, &i64> = self
            .entries
            .iter()
            .filter(|(k, e)| pool_keys.contains(*k) && **e > now)
            .collect();
        write_atomic(path, serde_json::to_string(&filtered).unwrap_or_else(|_| "{}".into()).as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_entries() {
        let mut q = Quarantine::default();
        q.quarantine("a:1", Duration::from_secs(10), 1000);
        assert!(q.active(1005).contains("a:1"));
        assert!(!q.active(1011).contains("a:1"));
    }

    #[test]
    fn empty_keys_are_never_quarantined() {
        let mut q = Quarantine::default();
        q.quarantine("", Duration::from_secs(10), 0);
        q.quarantine("h:0", Duration::from_secs(10), 0);
        assert!(q.is_empty());
    }

    #[test]
    fn gc_drops_keys_that_left_the_pool() {
        let path = std::env::temp_dir().join(format!("q-{}.json", uuid_like()));
        let pool: HashSet<String> = ["kept:1".to_string()].into_iter().collect();
        let raw = r#"{"kept:1":9999999999,"gone:2":9999999999}"#;
        std::fs::write(&path, raw).unwrap();
        let q = Quarantine::load(&path, &pool, now_secs());
        let active = q.active(now_secs());
        assert!(active.contains("kept:1"), "in-pool unexpired entry must survive GC");
        assert!(!active.contains("gone:2"), "entry for an IP that left the pool must be dropped");
        let _ = std::fs::remove_file(&path);
    }

    fn uuid_like() -> u128 {
        std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    }
}
