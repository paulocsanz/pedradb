// Verus proof of the 2PC per-range cleanup decision (RFC-0056 P1.4).
// Twin of `src/tx_glue_kernel.rs::tx_range_action`. Not linked into production.
//
//   ./scripts/verus_tx_glue.sh

use vstd::prelude::*;

verus! {

/// Same variants as production `tx_glue_kernel::TxRangeAction`.
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

/// F47/F34 kernel twin: a failed TX undoes each already-majority-committed
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
    if !tx_failed {
        TxRangeAction::KeepCommitted
    } else if range_committed {
        TxRangeAction::MajorityRevert
    } else {
        TxRangeAction::LocalRevert
    }
}

// ---------------------------------------------------------------------------
// Named lemmas (crash dictionary — RFC-0056 P1.4)
// ---------------------------------------------------------------------------

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

fn main() {}

} // verus!
