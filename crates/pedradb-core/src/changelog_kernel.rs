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
}
