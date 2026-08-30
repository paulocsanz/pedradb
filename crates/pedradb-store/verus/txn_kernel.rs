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

pub fn leftover_txn_is_aborted() -> (d: bool)
    ensures
        d,
{
    true
}

pub open spec fn leftover_txn_is_aborted_as_is() -> bool {
    false
}

proof fn lemma_as_is_leaves_intents()
    ensures
        leftover_txn_is_aborted_as_is() == false,
{
}

pub fn next_txn_id_after(max_seen: u64) -> (n: u64)
    ensures
        n == (if sat_add1(max_seen) > 1 {
            sat_add1(max_seen)
        } else {
            1
        }),
        max_seen < u64::MAX ==> n > max_seen,
        n >= 1,
{
    let s = if max_seen == u64::MAX {
        max_seen
    } else {
        max_seen + 1
    };
    if s > 1 {
        s
    } else {
        1
    }
}

pub fn recover_si_generation(loaded_max: u64) -> (g: u64)
    ensures
        g == loaded_max,
{
    loaded_max
}

pub open spec fn recover_si_generation_as_is(_loaded: u64) -> u64 {
    0
}

proof fn lemma_as_is_evaporates_si(loaded: u64)
    requires
        loaded > 0,
    ensures
        recover_si_generation_as_is(loaded) == 0,
        recover_si_generation_as_is(loaded) != loaded,
{
}

pub fn prepare_error_aborts_earlier() -> (d: bool)
    ensures
        d,
{
    true
}

pub open spec fn prepare_error_aborts_earlier_as_is() -> bool {
    false
}

proof fn lemma_as_is_skips_prepare_abort()
    ensures
        prepare_error_aborts_earlier_as_is() == false,
{
}

pub struct SiGenReserve {
    pub next_current: u64,
    pub reserved: u64,
}

pub fn reserve_si_gen(current: u64) -> (r: SiGenReserve)
    ensures
        r.next_current == sat_add1(current),
        r.reserved == r.next_current,
        current < u64::MAX ==> r.reserved > current,
{
    let n = if current == u64::MAX {
        current
    } else {
        current + 1
    };
    SiGenReserve {
        next_current: n,
        reserved: n,
    }
}

pub open spec fn reserve_si_gen_as_is(current: u64) -> (u64, u64) {
    (current, sat_add1(current))
}

proof fn lemma_as_is_collides(current: u64)
    requires
        current < u64::MAX,
    ensures
        reserve_si_gen_as_is(current).0 == current,
        reserve_si_gen_as_is(current).1 == current + 1,
        ({
            let again = reserve_si_gen_as_is(reserve_si_gen_as_is(current).0);
            again.1 == reserve_si_gen_as_is(current).1
        }),
{
}

pub fn unreserve_si_gen(current: u64, stamped: u64) -> (n: u64)
    ensures
        (stamped > 0 && current == stamped) ==> n == (if stamped == 0 {
            0
        } else {
            (stamped - 1) as u64
        }),
        !(stamped > 0 && current == stamped) ==> n == current,
{
    if stamped > 0 && current == stamped {
        stamped - 1
    } else {
        current
    }
}

} // verus!
