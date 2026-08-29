//! Pure raft-log compact decisions (RFC-0002 P13 / F27 / F28).
//!
//! Production [`crate::StoreCluster::maybe_compact_logs`] /
//! [`crate::RangePeer::compact_through`] call these helpers.
//! Persist of snap/log and AE catch-up are **axioms**.

#![forbid(unsafe_code)]

/// F28: every configured member counts in `min(applied)`, even if partitioned.
///
/// Offline peers freeze the watermark so compact cannot drop entries they
/// still need (no install-snapshot path).
#[must_use]
pub fn peer_counts_for_compact(_is_participating: bool) -> bool {
    true
}

/// AS-IS F28: only live/participating peers — compact past offline applied.
#[must_use]
pub fn peer_counts_for_compact_as_is(is_participating: bool) -> bool {
    is_participating
}

/// F27: no compact at applied 0 (empty / nothing durable to drop).
#[must_use]
pub fn compact_ready(min_applied: u64) -> bool {
    min_applied > 0
}

/// Whether this peer may drop `index <= through`.
///
/// Refuses when `through` is not in the log or already covered by the snapshot.
#[must_use]
pub fn may_compact_through(snapshot_index: u64, through: u64, term_at_through: u64) -> bool {
    if through == 0 || through <= snapshot_index {
        return false;
    }
    if term_at_through == 0 {
        return false;
    }
    true
}

/// AS-IS: compact even when the entry is missing (`term_at == 0`).
#[must_use]
pub fn may_compact_through_as_is(snapshot_index: u64, through: u64, _term_at: u64) -> bool {
    through > 0 && through > snapshot_index
}

/// Floor for `next_index` / `match_index` after compact through `n`.
#[must_use]
pub fn compact_index_floor(through: u64) -> u64 {
    through.saturating_add(1)
}

/// Cap compact so an applied still-active joint stays until leave.
///
/// `unleft_joint` is the log index of a `MembershipJoint` with `old != new`
/// and no later applied leave (`old == new`). Compact through `j - 1`.
#[must_use]
pub fn compact_through_unleft(through: u64, unleft_joint: Option<u64>) -> u64 {
    match unleft_joint {
        Some(j) if j > 0 && j <= through => j.saturating_sub(1),
        _ => through,
    }
}

/// AS-IS: compact past an un-left joint (the 0096/0100 hole).
#[must_use]
pub fn compact_through_unleft_as_is(through: u64, _unleft_joint: Option<u64>) -> u64 {
    through
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_peer_still_counts() {
        assert!(peer_counts_for_compact(false));
        assert!(peer_counts_for_compact(true));
        assert!(!peer_counts_for_compact_as_is(false));
        assert_ne!(
            peer_counts_for_compact(false),
            peer_counts_for_compact_as_is(false)
        );
    }

    #[test]
    fn zero_applied_not_ready() {
        assert!(!compact_ready(0));
        assert!(compact_ready(3));
    }

    #[test]
    fn missing_term_blocks_compact() {
        assert!(!may_compact_through(0, 5, 0));
        assert!(may_compact_through(0, 5, 2));
        assert!(!may_compact_through(5, 5, 2));
        assert!(may_compact_through_as_is(0, 5, 0));
    }

    #[test]
    fn floor_after_compact() {
        assert_eq!(compact_index_floor(7), 8);
        assert_eq!(compact_index_floor(u64::MAX), u64::MAX);
    }

    #[test]
    fn unleft_joint_caps_through() {
        assert_eq!(compact_through_unleft(5, Some(3)), 2);
        assert_eq!(compact_through_unleft(5, Some(5)), 4);
        assert_eq!(compact_through_unleft(5, Some(6)), 5);
        assert_eq!(compact_through_unleft(5, None), 5);
        assert_eq!(compact_through_unleft(5, Some(0)), 5);
        assert_eq!(compact_through_unleft_as_is(5, Some(3)), 5);
    }

    #[test]
    fn theorem_compact_on_finite_domain() {
        for part in [false, true] {
            assert!(peer_counts_for_compact(part));
        }
        for snap in 0u64..5 {
            for through in 0u64..5 {
                for term in 0u64..3 {
                    let d = may_compact_through(snap, through, term);
                    assert_eq!(d, through > 0 && through > snap && term != 0);
                    if through > snap && through > 0 && term == 0 {
                        assert!(may_compact_through_as_is(snap, through, term));
                        assert!(!d);
                    }
                }
            }
        }
    }

    #[test]
    fn compact_through_unleft_on_live_joint_is_not_ok() {
        assert_eq!(compact_through_unleft(5, Some(3)), 2);
        assert_eq!(
            compact_through_unleft_as_is(5, Some(3)),
            5,
            "AS-IS dente: compact past unleft joint"
        );
    }
}
