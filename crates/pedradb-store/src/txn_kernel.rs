//! Pure TX apply / discard / revert decisions (RFC-0002 P9–P10 / F47 / F34 / F52).
//!
//! **Single artifact (pair `txn`):** this file is what `rustc` links *and*
//! what Verus proves (`cfg(verus_keep_ghost)`). Other pairs on this file
//! (`revert_clears_status`, `discard_cut`, …) still have a twin-cópia
//! until their turns.
//!
//!   ./scripts/verus_txn_kernel.sh
//!
//! Production [`crate::apply_txn_commit`] / [`crate::apply_txn_revert`] /
//! [`crate::StoreCluster::discard_uncommitted_from`] call these helpers.
//! Persist and raft majority are **axioms**.
//!
//! The rustc bodies stay byte-stable so non-`single_artifact` twins still
//! token-match. Verus proofs sit in the `cfg(verus_keep_ghost)` block
//! above them (last-wins for lint is the rustc body).

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
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

/// F47 teeth / Khan 2606.17182 SSI abort-step: abort never materialises.
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

/// What a raft `TxnCommit` apply must do (F47).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnCommitAction {
    /// Status is abort — restore preimage, keep fence.
    Revert,
    /// Materialise intents (normal commit).
    Materialise,
}

/// F47: fenced abort never materialises user keys.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn txn_commit_action(status_is_abort: bool) -> TxnCommitAction {
    if status_is_abort {
        TxnCommitAction::Revert
    } else {
        TxnCommitAction::Materialise
    }
}

/// AS-IS F47: ignore abort fence (heal/elect installs the aborted TX).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn txn_commit_action_as_is(_status_is_abort: bool) -> TxnCommitAction {
    TxnCommitAction::Materialise
}

#[cfg(test)]
mod three_teeth {
    use super::*;

    #[test]
    fn txn_commit_action_on_live_abort_is_not_ok() {
        assert_eq!(txn_commit_action(true), TxnCommitAction::Revert);
        assert_eq!(
            txn_commit_action_as_is(true),
            TxnCommitAction::Materialise,
            "AS-IS dente: abort materialises"
        );
    }
}

/// After revert, may we delete the txn status key?
///
/// F47: if the status was abort, keep the fence so a later `TxnCommit` replay
/// still sees abort.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn revert_clears_status(status_is_abort: bool, pairs_empty: bool) -> bool {
    pairs_empty && !status_is_abort
}

/// AS-IS: revert always drops status when pairs are gone (fence evaporates).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn revert_clears_status_as_is(_status_is_abort: bool, pairs_empty: bool) -> bool {
    pairs_empty
}

/// Lowest index that may be discarded (never at or below commit).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn discard_cut(from_index: u64, commit: u64) -> u64 {
    from_index.max(commit.saturating_add(1))
}

/// AS-IS: cut at `from_index` even if that is committed.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn discard_cut_as_is(from_index: u64, _commit: u64) -> u64 {
    from_index
}

/// How to restore one user key from the prepare-time preimage (F34).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertUserAction {
    /// Preimage was `Some(v)` — put `v` back.
    RestoreValue,
    /// Preimage was `None` — key did not exist at prepare; delete.
    RestoreAbsent,
    /// No preimage record (peer never prepared) — leave the user key alone.
    LeaveUntouched,
}

/// F34: restore preimage; never blind-delete; missing record is not "absent".
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn revert_user_action(had_pre_record: bool, pre_was_absent: bool) -> RevertUserAction {
    if !had_pre_record {
        RevertUserAction::LeaveUntouched
    } else if pre_was_absent {
        RevertUserAction::RestoreAbsent
    } else {
        RevertUserAction::RestoreValue
    }
}

/// AS-IS F34: always delete the user key.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn revert_user_action_as_is(_had_pre_record: bool, _pre_was_absent: bool) -> RevertUserAction {
    RevertUserAction::RestoreAbsent
}

/// F52: rewrite SI hist tip after a successful Pedra restore (not reserved keys).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn should_repair_si_hist(restored: bool, is_reserved: bool) -> bool {
    restored && !is_reserved
}

/// AS-IS F52: never touch hist (SI / Pedra split after reopen).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn should_repair_si_hist_as_is(_restored: bool, _is_reserved: bool) -> bool {
    false
}

/// F35: leftover prepared TX after crash is aborted (no coordinator log).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn leftover_txn_is_aborted() -> bool {
    true
}

/// AS-IS F35: leave intents live (immortal Conflict + id reuse).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn leftover_txn_is_aborted_as_is() -> bool {
    false
}

/// F35: never reuse a txn id still on disk / in the durable counter.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_txn_id_after(max_seen: u64) -> u64 {
    max_seen.saturating_add(1).max(1)
}

/// AS-IS F35: always restart the counter at 1.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn next_txn_id_as_is(_max_seen: u64) -> u64 {
    1
}

/// F36: SI generation / watermark come from durable max, not RAM 0.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn recover_si_generation(loaded_max: u64) -> u64 {
    loaded_max
}

/// AS-IS F36: generation evaporates on reopen.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn recover_si_generation_as_is(_loaded_max: u64) -> u64 {
    0
}

/// F50: a failed prepare step must abort already-durable intents on earlier ranges.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn prepare_error_aborts_earlier() -> bool {
    true
}

/// AS-IS F50: `?` on NotLeader returns without cleanup (immortal Conflict).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn prepare_error_aborts_earlier_as_is() -> bool {
    false
}

/// Result of reserving one SI generation (F49).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiGenReserve {
    /// New `commit_generation` after the reserve.
    pub next_current: u64,
    /// Value stamped on the raft entry / hist.
    pub reserved: u64,
}

/// F49: advance the counter **and** return that value (distinct outstanding gens).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn reserve_si_gen(current: u64) -> SiGenReserve {
    let n = current.saturating_add(1);
    SiGenReserve {
        next_current: n,
        reserved: n,
    }
}

/// AS-IS F49: compute `current+1` but leave the counter unmoved (collision).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn reserve_si_gen_as_is(current: u64) -> SiGenReserve {
    SiGenReserve {
        next_current: current,
        reserved: current.saturating_add(1),
    }
}

/// Undo a reserve only if nothing else reserved after us (propose failed).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn unreserve_si_gen(current: u64, stamped: u64) -> u64 {
    if stamped > 0 && current == stamped {
        stamped.saturating_sub(1)
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_reverts() {
        assert_eq!(txn_commit_action(true), TxnCommitAction::Revert);
        assert_eq!(txn_commit_action(false), TxnCommitAction::Materialise);
    }

    #[test]
    fn as_is_materialises_aborted() {
        assert_eq!(txn_commit_action_as_is(true), TxnCommitAction::Materialise);
        assert_ne!(txn_commit_action(true), txn_commit_action_as_is(true));
    }

    #[test]
    fn revert_keeps_abort_fence() {
        assert!(!revert_clears_status(true, true));
        assert!(revert_clears_status(false, true));
        assert!(!revert_clears_status(true, false));
        assert!(revert_clears_status_as_is(true, true));
    }

    #[test]
    fn discard_never_cuts_committed() {
        assert_eq!(discard_cut(3, 5), 6);
        assert_eq!(discard_cut(8, 5), 8);
        assert_eq!(discard_cut(0, 0), 1);
        assert_eq!(discard_cut_as_is(3, 5), 3);
        assert!(discard_cut_as_is(3, 5) <= 5);
        assert!(discard_cut(3, 5) > 5);
    }

    #[test]
    fn theorem_txn_on_bool_domain() {
        for abort in [false, true] {
            let a = txn_commit_action(abort);
            assert_eq!(a == TxnCommitAction::Revert, abort);
            if abort {
                assert_eq!(txn_commit_action_as_is(abort), TxnCommitAction::Materialise);
            }
            for empty in [false, true] {
                assert_eq!(revert_clears_status(abort, empty), empty && !abort);
            }
        }
        for from in 0u64..6 {
            for commit in 0u64..6 {
                let c = discard_cut(from, commit);
                assert!(c > commit || commit == u64::MAX);
                assert!(c >= from);
            }
        }
        for had in [false, true] {
            for absent in [false, true] {
                let a = revert_user_action(had, absent);
                if !had {
                    assert_eq!(a, RevertUserAction::LeaveUntouched);
                } else if absent {
                    assert_eq!(a, RevertUserAction::RestoreAbsent);
                } else {
                    assert_eq!(a, RevertUserAction::RestoreValue);
                }
                assert_eq!(
                    revert_user_action_as_is(had, absent),
                    RevertUserAction::RestoreAbsent
                );
            }
        }
        for restored in [false, true] {
            for reserved in [false, true] {
                assert_eq!(
                    should_repair_si_hist(restored, reserved),
                    restored && !reserved
                );
                assert!(!should_repair_si_hist_as_is(restored, reserved));
            }
        }
    }

    #[test]
    fn missing_preimage_leaves_key() {
        assert_eq!(
            revert_user_action(false, false),
            RevertUserAction::LeaveUntouched
        );
        assert_eq!(
            revert_user_action_as_is(false, false),
            RevertUserAction::RestoreAbsent
        );
    }

    #[test]
    fn repair_hist_only_when_restored_user_key() {
        assert!(should_repair_si_hist(true, false));
        assert!(!should_repair_si_hist(true, true));
        assert!(!should_repair_si_hist(false, false));
        assert!(!should_repair_si_hist_as_is(true, false));
    }

    #[test]
    fn leftover_aborted_not_immortal() {
        assert!(leftover_txn_is_aborted());
        assert!(!leftover_txn_is_aborted_as_is());
    }

    #[test]
    fn next_txn_never_reuses() {
        assert_eq!(next_txn_id_after(0), 1);
        assert_eq!(next_txn_id_after(7), 8);
        assert_eq!(next_txn_id_as_is(7), 1);
        assert!(next_txn_id_after(7) > 7);
    }

    #[test]
    fn si_generation_survives() {
        assert_eq!(recover_si_generation(9), 9);
        assert_eq!(recover_si_generation_as_is(9), 0);
    }

    #[test]
    fn prepare_fail_aborts() {
        assert!(prepare_error_aborts_earlier());
        assert!(!prepare_error_aborts_earlier_as_is());
    }

    #[test]
    fn reserve_advances_and_differs() {
        let a = reserve_si_gen(3);
        let b = reserve_si_gen(a.next_current);
        assert_eq!(a.reserved, 4);
        assert_eq!(a.next_current, 4);
        assert_eq!(b.reserved, 5);
        assert_ne!(a.reserved, b.reserved);
        let as_a = reserve_si_gen_as_is(3);
        let as_b = reserve_si_gen_as_is(as_a.next_current);
        assert_eq!(as_a.reserved, as_b.reserved);
    }

    #[test]
    fn unreserve_only_if_still_ours() {
        assert_eq!(unreserve_si_gen(4, 4), 3);
        assert_eq!(unreserve_si_gen(5, 4), 5);
        assert_eq!(unreserve_si_gen(4, 0), 4);
    }
}
