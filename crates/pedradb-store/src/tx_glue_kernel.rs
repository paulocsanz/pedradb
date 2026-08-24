//! 2PC glue decision kernel (RFC-0056 P1.4 — crash dictionary on the
//! multi-range transaction cleanup path).
//!
//! Pure decision only. Production (`lib.rs::StoreCluster::tx_finish`) calls
//! [`tx_range_action`] per range when a `TxnCommit` propose fails mid-TX; the
//! durable effects (majority revert entry, local intent cleanup, fencing)
//! stay in `lib.rs`.
//!
//! Atomicity contract being decided (F47/F34):
//! - A range whose `TxnCommit` already reached majority **must** be undone by
//!   a majority `TxnRevert` on the same raft log — a local-only cleanup
//!   leaves the user-key apply visible and the TX stops being atomic.
//! - A range that never committed is cleaned up locally (revert intents +
//!   preimages); proposing anything on the raft log would be noise.

/// Per-range cleanup action after a `tx_finish` outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxRangeAction {
    /// TX succeeded: every committed range stays committed.
    KeepCommitted,
    /// TX failed but this range already majority-committed `TxnCommit`:
    /// needs a majority `TxnRevert` on the same raft log (F47).
    MajorityRevert,
    /// TX failed and this range never committed: local intent/preimage
    /// revert (F34).
    LocalRevert,
}

/// Decide how one range of a failed (or succeeded) multi-range TX is cleaned
/// up.
#[must_use]
pub fn tx_range_action(range_committed: bool, tx_failed: bool) -> TxRangeAction {
    if !tx_failed {
        TxRangeAction::KeepCommitted
    } else if range_committed {
        TxRangeAction::MajorityRevert
    } else {
        TxRangeAction::LocalRevert
    }
}

/// AS-IS mutant: cleanup is always local. A range that already
/// majority-committed keeps its user-key apply visible forever (only the
/// local node's intents are dropped) — the TX stops being all-or-nothing.
#[must_use]
pub fn tx_range_action_as_is_local_only(_range_committed: bool, tx_failed: bool) -> TxRangeAction {
    if !tx_failed {
        TxRangeAction::KeepCommitted
    } else {
        TxRangeAction::LocalRevert
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_keeps_every_range() {
        assert_eq!(tx_range_action(false, false), TxRangeAction::KeepCommitted);
        assert_eq!(tx_range_action(true, false), TxRangeAction::KeepCommitted);
    }

    #[test]
    fn committed_range_gets_majority_revert() {
        assert_eq!(tx_range_action(true, true), TxRangeAction::MajorityRevert);
    }

    #[test]
    fn uncommitted_range_gets_local_revert() {
        assert_eq!(tx_range_action(false, true), TxRangeAction::LocalRevert);
    }

    #[test]
    fn theorem_tx_range_on_finite_domain() {
        // 2×2 domain: the fixed decision is total and the AS-IS mutant
        // diverges exactly on the atomicity-critical input — a range that
        // majority-committed inside a failed TX.
        for range_committed in [false, true] {
            for tx_failed in [false, true] {
                let a = tx_range_action(range_committed, tx_failed);
                let m = tx_range_action_as_is_local_only(range_committed, tx_failed);
                if range_committed && tx_failed {
                    assert_eq!(a, TxRangeAction::MajorityRevert);
                    assert_eq!(m, TxRangeAction::LocalRevert);
                    assert_ne!(a, m, "mutant must diverge on committed-but-failed");
                } else {
                    assert_eq!(a, m);
                }
            }
        }
    }
}
