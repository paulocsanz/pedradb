//! 2PC glue decision kernel (RFC-0056 P1.4 — crash dictionary on the
//! multi-range transaction cleanup path).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_tx_glue.sh
//!
//! Pure decision only. Production (`lib.rs::StoreCluster::tx_finish`) calls
//! [`tx_range_action`] per range when a `TxnCommit` propose fails mid-TX; the
//! durable effects (majority revert entry, local intent cleanup, fencing)
//! stay in `lib.rs` (unverified crate, CapybaraKV/OSDI'25 shape).
//!
//! Atomicity contract being decided (F47/F34):
//! - A range whose `TxnCommit` already reached majority **must** be undone by
//!   a majority `TxnRevert` on the same raft log — a local-only cleanup
//!   leaves the user-key apply visible and the TX stops being atomic.
//! - A range that never committed is cleaned up locally (revert intents +
//!   preimages); proposing anything on the raft log would be noise.

#![forbid(unsafe_code)]

macro_rules! tx_range_action_body {
    ($range_committed:expr, $tx_failed:expr) => {
        if !$tx_failed {
            TxRangeAction::KeepCommitted
        } else if $range_committed {
            TxRangeAction::MajorityRevert
        } else {
            TxRangeAction::LocalRevert
        }
    };
}

macro_rules! tx_range_action_as_is_body {
    ($range_committed:expr, $tx_failed:expr) => {
        if !$tx_failed {
            TxRangeAction::KeepCommitted
        } else {
            let _ = $range_committed;
            TxRangeAction::LocalRevert
        }
    };
}

/// Per-range cleanup action after a `tx_finish` outcome.
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn tx_range_action(range_committed: bool, tx_failed: bool) -> TxRangeAction {
    tx_range_action_body!(range_committed, tx_failed)
}

/// AS-IS mutant: cleanup is always local. A range that already
/// majority-committed keeps its user-key apply visible forever (only the
/// local node's intents are dropped) — the TX stops being all-or-nothing.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn tx_range_action_as_is_local_only(_range_committed: bool, tx_failed: bool) -> TxRangeAction {
    tx_range_action_as_is_body!(_range_committed, tx_failed)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// Same variants as the rustc enum above (cfg-split so Verus does not see Debug).
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TxRangeAction {
    KeepCommitted,
    MajorityRevert,
    LocalRevert,
}

pub open spec fn tx_range_spec(range_committed: bool, tx_failed: bool) -> TxRangeAction {
    if !tx_failed {
        TxRangeAction::KeepCommitted
    } else if range_committed {
        TxRangeAction::MajorityRevert
    } else {
        TxRangeAction::LocalRevert
    }
}

/// AS-IS local-only cleanup: a range that already majority-committed is
/// cleaned up locally — its user-key apply stays visible forever.
pub open spec fn tx_range_as_is(range_committed: bool, tx_failed: bool) -> TxRangeAction {
    if !tx_failed {
        TxRangeAction::KeepCommitted
    } else {
        TxRangeAction::LocalRevert
    }
}

/// F47/F34 kernel: a failed TX undoes each already-majority-committed
/// range with a majority TxnRevert (atomicity on the raft log), and each
/// never-committed range locally.
#[verifier::when_used_as_spec(tx_range_spec)]
pub fn tx_range_action(range_committed: bool, tx_failed: bool) -> (a: TxRangeAction)
    ensures
        a == tx_range_spec(range_committed, tx_failed),
        !tx_failed ==> a == TxRangeAction::KeepCommitted,
        tx_failed && range_committed ==> a == TxRangeAction::MajorityRevert,
        tx_failed && !range_committed ==> a == TxRangeAction::LocalRevert,
{
    tx_range_action_body!(range_committed, tx_failed)
}

pub fn tx_range_action_as_is_local_only(range_committed: bool, tx_failed: bool) -> (a: TxRangeAction)
    ensures
        a == tx_range_as_is(range_committed, tx_failed),
{
    tx_range_action_as_is_body!(range_committed, tx_failed)
}

/// P1.4 named lemma (F47): a range whose `TxnCommit` reached majority inside
/// a failed TX is undone on the same raft log — the apply cannot stay
/// visible, or the TX is not all-or-nothing.
proof fn lemma_committed_range_gets_majority_revert()
    ensures
        tx_range_action(true, true) == TxRangeAction::MajorityRevert,
{
}

/// P1.4 named lemma (F34): a range that never committed is cleaned up
/// locally — intents and preimages revert, nothing to revert on the log.
proof fn lemma_uncommitted_range_gets_local_revert()
    ensures
        tx_range_action(false, true) == TxRangeAction::LocalRevert,
{
}

/// P1.4 named lemma (no false revert): a successful TX keeps every range.
proof fn lemma_success_keeps_every_range(range_committed: bool)
    ensures
        tx_range_action(range_committed, false) == TxRangeAction::KeepCommitted,
{
}

/// Teeth: the local-only AS-IS mutant leaves a majority-committed apply
/// visible exactly where the fixed kernel schedules the majority revert —
/// the atomicity bug the kernel exists to prevent.
proof fn lemma_mutant_leaves_majority_apply_visible()
    ensures
        tx_range_as_is(true, true) == TxRangeAction::LocalRevert,
        tx_range_action(true, true) == TxRangeAction::MajorityRevert,
        tx_range_as_is(true, true) != tx_range_action(true, true),
        tx_range_as_is(true, true) != tx_range_spec(true, true),
{
}

} // verus!

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

    #[test]
    fn tx_range_action_on_live_majority_is_not_ok() {
        assert_eq!(tx_range_action(true, true), TxRangeAction::MajorityRevert);
        assert_eq!(
            tx_range_action_as_is_local_only(true, true),
            TxRangeAction::LocalRevert,
            "AS-IS dente: majority commit cleaned locally"
        );
    }
}
