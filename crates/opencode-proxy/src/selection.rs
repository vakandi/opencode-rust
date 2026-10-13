//! Which upstream to try next. Pure function, no I/O, so it is exhaustively testable.
//!
//! The rule Wael asked for is "oldest used first", and the reference adds two
//! refinements that are load-bearing:
//!
//! - **Never-used entries sort first**, because their `ts` is 0.
//! - **Unclaimed before claimed**, so the pool spreads out. A pool smaller than the
//!   fleet forces sharing, and a shared IP that still answers beats a dead one nobody
//!   claimed.
//!
//! `sort_by_key` is STABLE in Rust, which is what makes ties deterministic: entries
//! sharing a `last` second keep pool-file order, so successive rotations walk the tied
//! group from the lowest index up. Swapping this for `sort_unstable_by` would change
//! rotation behaviour with no compile error — that is why this note exists.

use std::collections::HashSet;

use crate::pool::PoolEntry;

/// Pool indices in probe order.
pub fn order_candidates(
    entries: &[PoolEntry],
    quarantined: &HashSet<String>,
    claimed: &HashSet<usize>,
    manual: Option<usize>,
) -> Vec<usize> {
    // A manual pick that cannot be honoured must NOT silently become a different IP.
    // The reference treats an unusable index as "no preference" and rotates anyway,
    // which moves egress somewhere the operator never chose. Refuse instead.
    if let Some(index) = manual {
        match entries.get(index) {
            Some(entry) if !quarantined.contains(&entry.key()) => return vec![index],
            _ => return Vec::new(),
        }
    }

    let mut eligible: Vec<&PoolEntry> =
        entries.iter().filter(|e| !quarantined.contains(&e.key())).collect();
    eligible.sort_by_key(|e| e.ts());

    let mut order: Vec<usize> = Vec::with_capacity(eligible.len());
    for entry in eligible.iter().filter(|e| !claimed.contains(&e.index)) {
        order.push(entry.index);
    }
    for entry in eligible.iter().filter(|e| claimed.contains(&e.index)) {
        order.push(entry.index);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::parse;

    fn entries(lines: &str) -> Vec<PoolEntry> {
        parse(lines)
    }

    #[test]
    fn oldest_first() {
        let pool = entries(
            "a:1:u:p |last:2026-10-07 12:00:00\nb:2:u:p |last:2026-10-07 10:00:00\nc:3:u:p |last:2026-10-07 11:00:00\n",
        );
        let order = order_candidates(&pool, &HashSet::new(), &HashSet::new(), None);
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn never_used_entries_sort_first() {
        let pool = entries("used:1:u:p |last:2026-10-07 10:00:00\nfresh:2:u:p\n");
        let order = order_candidates(&pool, &HashSet::new(), &HashSet::new(), None);
        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn same_second_ties_break_on_lowest_pool_index_and_are_stable() {
        let pool = entries(
            "a:1:u:p |last:2026-10-07 10:00:00\nb:2:u:p |last:2026-10-07 10:00:00\nc:3:u:p |last:2026-10-07 10:00:00\n",
        );
        let no_quarantine: HashSet<String> = HashSet::new();
        let no_claims: HashSet<usize> = HashSet::new();
        let first = order_candidates(&pool, &no_quarantine, &no_claims, None);
        assert_eq!(first, vec![0, 1, 2]);
        // Repeat: a stable sort must not reshuffle between calls.
        assert_eq!(order_candidates(&pool, &no_quarantine, &no_claims, None), first);
    }

    #[test]
    fn unclaimed_entries_precede_claimed_ones_even_when_older() {
        let pool = entries(
            "claimed_old:1:u:p |last:2026-10-07 09:00:00\nfree_new:2:u:p |last:2026-10-07 11:00:00\n",
        );
        let claimed: HashSet<usize> = [0].into_iter().collect();
        let order = order_candidates(&pool, &HashSet::new(), &claimed, None);
        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn quarantined_entries_are_never_offered() {
        let pool = entries("bad:1:u:p |last:2026-10-07 09:00:00\ngood:2:u:p\n");
        let quarantined: HashSet<String> = ["bad:1".to_string()].into_iter().collect();
        let order = order_candidates(&pool, &quarantined, &HashSet::new(), None);
        assert_eq!(order, vec![1]);
    }

    #[test]
    fn manual_index_out_of_range_is_refused_not_substituted() {
        let pool = entries("a:1:u:p\n");
        assert!(order_candidates(&pool, &HashSet::new(), &HashSet::new(), Some(99)).is_empty());
    }

    #[test]
    fn manual_index_that_is_quarantined_is_refused() {
        let pool = entries("a:1:u:p\n");
        let quarantined: HashSet<String> = ["a:1".to_string()].into_iter().collect();
        assert!(order_candidates(&pool, &quarantined, &HashSet::new(), Some(0)).is_empty());
    }

    #[test]
    fn manual_index_is_honoured_even_when_not_oldest() {
        let pool = entries("old:1:u:p |last:2020-01-01 00:00:00\nnew:2:u:p |last:2030-01-01 00:00:00\n");
        let order = order_candidates(&pool, &HashSet::new(), &HashSet::new(), Some(1));
        assert_eq!(order, vec![1]);
    }
}