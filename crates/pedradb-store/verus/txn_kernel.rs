// Verus proof of TX abort-fence + discard cut (RFC-0002 P9 / F47).
// Twin of `src/txn_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_txn_kernel.sh

use vstd::prelude::*;

verus! {

pub enum TxnCommitAction {
    Revert,
    Materialise,
}

pub open spec fn txn_commit_action_spec(status_is_abort: bool) -> TxnCommitAction {
    if status_is_abort {
        TxnCommitAction::Revert
    } else {
        TxnCommitAction::Materialise
    }
}

pub fn txn_commit_action(status_is_abort: bool) -> (d: TxnCommitAction)
    ensures
        d == txn_commit_action_spec(status_is_abort),
        status_is_abort ==> d == TxnCommitAction::Revert,
        !status_is_abort ==> d == TxnCommitAction::Materialise,
{
    if status_is_abort {
        TxnCommitAction::Revert
    } else {
        TxnCommitAction::Materialise
    }
}

pub open spec fn txn_commit_action_as_is(_status_is_abort: bool) -> TxnCommitAction {
    TxnCommitAction::Materialise
}

proof fn lemma_as_is_materialises_abort()
    ensures
        txn_commit_action_spec(true) == TxnCommitAction::Revert,
        txn_commit_action_as_is(true) == TxnCommitAction::Materialise,
{
}

pub fn revert_clears_status(status_is_abort: bool, pairs_empty: bool) -> (d: bool)
    ensures
        d == (pairs_empty && !status_is_abort),
        status_is_abort ==> !d,
{
    pairs_empty && !status_is_abort
}

pub open spec fn revert_clears_status_as_is(_abort: bool, pairs_empty: bool) -> bool {
    pairs_empty
}

proof fn lemma_as_is_drops_fence()
    ensures
        !{ revert_clears_status_as_is(true, true) == false },
        revert_clears_status_as_is(true, true),
{
}

pub open spec fn sat_add1(x: u64) -> u64 {
    if x == u64::MAX {
        x
    } else {
        (x + 1) as u64
    }
}

pub fn discard_cut(from_index: u64, commit: u64) -> (c: u64)
    ensures
        c >= from_index,
        commit < u64::MAX ==> c > commit,
{
    let floor = if commit == u64::MAX {
        commit
    } else {
        commit + 1
    };
    if from_index >= floor {
        from_index
    } else {
        floor
    }
}

pub open spec fn discard_cut_as_is(from_index: u64, _commit: u64) -> u64 {
    from_index
}

proof fn lemma_as_is_can_cut_committed(from_index: u64, commit: u64)
    requires
        from_index <= commit,
        commit < u64::MAX,
    ensures
        discard_cut_as_is(from_index, commit) <= commit,
{
}

pub enum RevertUserAction {
    RestoreValue,
    RestoreAbsent,
    LeaveUntouched,
}

pub open spec fn revert_user_action_spec(had_pre: bool, pre_absent: bool) -> RevertUserAction {
    if !had_pre {
        RevertUserAction::LeaveUntouched
    } else if pre_absent {
        RevertUserAction::RestoreAbsent
    } else {
        RevertUserAction::RestoreValue
    }
}

pub fn revert_user_action(had_pre_record: bool, pre_was_absent: bool) -> (d: RevertUserAction)
    ensures
        d == revert_user_action_spec(had_pre_record, pre_was_absent),
        !had_pre_record ==> d == RevertUserAction::LeaveUntouched,
{
    if !had_pre_record {
        RevertUserAction::LeaveUntouched
    } else if pre_was_absent {
        RevertUserAction::RestoreAbsent
    } else {
        RevertUserAction::RestoreValue
    }
}

pub open spec fn revert_user_action_as_is(_had: bool, _absent: bool) -> RevertUserAction {
    RevertUserAction::RestoreAbsent
}

proof fn lemma_as_is_deletes_missing_pre()
    ensures
        revert_user_action_spec(false, false) == RevertUserAction::LeaveUntouched,
        revert_user_action_as_is(false, false) == RevertUserAction::RestoreAbsent,
{
}

pub fn should_repair_si_hist(restored: bool, is_reserved: bool) -> (d: bool)
    ensures
        d == (restored && !is_reserved),
{
    restored && !is_reserved
}

pub open spec fn should_repair_si_hist_as_is(_r: bool, _res: bool) -> bool {
    false
}

proof fn lemma_as_is_skips_hist_repair()
    ensures
        should_repair_si_hist_as_is(true, false) == false,
{
}

} // verus!
