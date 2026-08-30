//! Pure changelog SST-rebuild gate (RFC-0002 P22 / F53).
//!
//! Production [`crate::db::Db::maybe_rebuild_feed_from_live`] calls this.
//! Scan of MemTable ∪ SSTs and persist of `CHANGELOG` are caller + axiom.

#![forbid(unsafe_code)]

/// Rebuild a last-per-key feed from MemTable ∪ SSTs when the loaded+WAL
/// changelog is empty but the DB already has a durable sequence.
///
/// After `flush` the WAL is truncated; a missing `CHANGELOG` must not leave
/// fold/journal with `changes_after(0) == []` while SST keys are live.
#[must_use]
pub fn changelog_needs_sst_rebuild(feed_empty: bool, last_sequence: u64) -> bool {
    feed_empty && last_sequence > 0
}

/// AS-IS F53: WAL-only rebuild — never consult SST/Mem even when the feed
/// is empty after a truncated WAL.
#[must_use]
pub fn changelog_needs_sst_rebuild_as_is(_feed_empty: bool, _last_sequence: u64) -> bool {
    false
}

/// Default durable-commit interval between CHANGELOG cache stores (RFC-0031).
pub const DEFAULT_CHANGELOG_INTERVAL: u64 = 64;

/// Whether the commit path should persist the CHANGELOG cache (RFC-0031 P0.1).
///
/// The on-disk CHANGELOG is a cache rebuilt from WAL (RFC-0019). Persisting
/// it is never a durability gate. `interval == 0` means never on the commit
/// path (flush / close / checkpoint still force a store). `interval >= 1`
/// persists when `commits_since >= interval`.
#[must_use]
pub fn changelog_should_store(commits_since: u64, interval: u64) -> bool {
    interval > 0 && commits_since >= interval
}

/// AS-IS RFC-0031: every durable commit stores (pre-debounce).
#[must_use]
pub fn changelog_should_store_as_is(commits_since: u64, _interval: u64) -> bool {
    commits_since >= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_when_feed_empty_and_seq_live() {
        assert!(changelog_needs_sst_rebuild(true, 1));
        assert!(changelog_needs_sst_rebuild(true, u64::MAX));
        assert!(!changelog_needs_sst_rebuild_as_is(true, 1));
    }

    #[test]
    fn skip_fresh_db() {
        assert!(!changelog_needs_sst_rebuild(true, 0));
        assert!(!changelog_needs_sst_rebuild(false, 0));
    }

    #[test]
    fn skip_when_feed_already_has_entries() {
        assert!(!changelog_needs_sst_rebuild(false, 5));
        assert!(!changelog_needs_sst_rebuild(false, u64::MAX));
    }

    #[test]
    fn theorem_on_small_domain() {
        let mut n = 0u32;
        for feed_empty in [false, true] {
            for last in 0u64..8 {
                let d = changelog_needs_sst_rebuild(feed_empty, last);
                assert_eq!(d, feed_empty && last > 0);
                assert!(!changelog_needs_sst_rebuild_as_is(feed_empty, last));
                if d {
                    assert!(feed_empty);
                    assert!(last > 0);
                    assert_ne!(d, changelog_needs_sst_rebuild_as_is(feed_empty, last));
                }
                n += 1;
            }
        }
        assert_eq!(n, 2 * 8);
    }

    #[test]
    fn debounce_interval_zero_never_on_commit_path() {
        assert!(!changelog_should_store(0, 0));
        assert!(!changelog_should_store(1, 0));
        assert!(!changelog_should_store(u64::MAX, 0));
    }

    #[test]
    fn debounce_interval_one_is_every_durable_commit() {
        assert!(!changelog_should_store(0, 1));
        assert!(changelog_should_store(1, 1));
        assert!(changelog_should_store(2, 1));
    }

    #[test]
    fn debounce_default_fires_at_n() {
        assert!(!changelog_should_store(63, DEFAULT_CHANGELOG_INTERVAL));
        assert!(changelog_should_store(64, DEFAULT_CHANGELOG_INTERVAL));
        assert!(changelog_should_store(65, DEFAULT_CHANGELOG_INTERVAL));
    }

    #[test]
    fn debounce_as_is_ignores_interval() {
        assert!(!changelog_should_store_as_is(0, 64));
        assert!(changelog_should_store_as_is(1, 64));
        assert!(changelog_should_store_as_is(1, 0));
    }

    #[test]
    fn debounce_theorem_on_small_domain() {
        let mut n = 0u32;
        for interval in 0u64..8 {
            for since in 0u64..16 {
                let d = changelog_should_store(since, interval);
                assert_eq!(d, interval > 0 && since >= interval);
                let as_is = changelog_should_store_as_is(since, interval);
                assert_eq!(as_is, since >= 1);
                if interval == 0 {
                    assert!(!d);
                }
                n += 1;
            }
        }
        assert_eq!(n, 8 * 16);
    }
}
