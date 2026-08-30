//! Pure TX apply / discard / revert decisions (RFC-0002 P9–P10 / F47 / F34 / F52).
//!
//! Production [`crate::apply_txn_commit`] / [`crate::apply_txn_revert`] /
//! [`crate::StoreCluster::discard_uncommitted_from`] call these helpers.
//! Persist and raft majority are **axioms**.

#![forbid(unsafe_code)]

/// What a raft `TxnCommit` apply must do (F47).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnCommitAction {
    /// Status is abort — restore preimage, keep fence.
    Revert,
    /// Materialise intents (normal commit).
    Materialise,
}

/// F47: fenced abort never materialises user keys.
#[must_use]
pub fn txn_commit_action(status_is_abort: bool) -> TxnCommitAction {
    if status_is_abort {
        TxnCommitAction::Revert
    } else {
        TxnCommitAction::Materialise
    }
}

/// AS-IS F47: ignore abort fence (heal/elect installs the aborted TX).
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
#[must_use]
pub fn revert_clears_status(status_is_abort: bool, pairs_empty: bool) -> bool {
    pairs_empty && !status_is_abort
}

/// AS-IS: revert always drops status when pairs are gone (fence evaporates).
#[must_use]
pub fn revert_clears_status_as_is(_status_is_abort: bool, pairs_empty: bool) -> bool {
    pairs_empty
}

/// Lowest index that may be discarded (never at or below commit).
#[must_use]
pub fn discard_cut(from_index: u64, commit: u64) -> u64 {
    from_index.max(commit.saturating_add(1))
}

/// AS-IS: cut at `from_index` even if that is committed.
#[must_use]
pub fn discard_cut_as_is(from_index: u64, _commit: u64) -> u64 {
    from_index
}

/// How to restore one user key from the prepare-time preimage (F34).
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
#[must_use]
pub fn revert_user_action_as_is(_had_pre_record: bool, _pre_was_absent: bool) -> RevertUserAction {
    RevertUserAction::RestoreAbsent
}

/// F52: rewrite SI hist tip after a successful Pedra restore (not reserved keys).
#[must_use]
pub fn should_repair_si_hist(restored: bool, is_reserved: bool) -> bool {
    restored && !is_reserved
}

/// AS-IS F52: never touch hist (SI / Pedra split after reopen).
#[must_use]
pub fn should_repair_si_hist_as_is(_restored: bool, _is_reserved: bool) -> bool {
    false
}

/// F35: leftover prepared TX after crash is aborted (no coordinator log).
#[must_use]
pub fn leftover_txn_is_aborted() -> bool {
    true
}

/// AS-IS F35: leave intents live (immortal Conflict + id reuse).
#[must_use]
pub fn leftover_txn_is_aborted_as_is() -> bool {
    false
}

/// F35: never reuse a txn id still on disk / in the durable counter.
#[must_use]
pub fn next_txn_id_after(max_seen: u64) -> u64 {
    max_seen.saturating_add(1).max(1)
}

/// AS-IS F35: always restart the counter at 1.
#[must_use]
pub fn next_txn_id_as_is(_max_seen: u64) -> u64 {
    1
}

/// F36: SI generation / watermark come from durable max, not RAM 0.
#[must_use]
pub fn recover_si_generation(loaded_max: u64) -> u64 {
    loaded_max
}

/// AS-IS F36: generation evaporates on reopen.
#[must_use]
pub fn recover_si_generation_as_is(_loaded_max: u64) -> u64 {
    0
}

/// F50: a failed prepare step must abort already-durable intents on earlier ranges.
#[must_use]
pub fn prepare_error_aborts_earlier() -> bool {
    true
}

/// AS-IS F50: `?` on NotLeader returns without cleanup (immortal Conflict).
#[must_use]
pub fn prepare_error_aborts_earlier_as_is() -> bool {
    false
}

/// Result of reserving one SI generation (F49).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiGenReserve {
    /// New `commit_generation` after the reserve.
    pub next_current: u64,
    /// Value stamped on the raft entry / hist.
    pub reserved: u64,
}

/// F49: advance the counter **and** return that value (distinct outstanding gens).
#[must_use]
pub fn reserve_si_gen(current: u64) -> SiGenReserve {
    let n = current.saturating_add(1);
    SiGenReserve {
        next_current: n,
        reserved: n,
    }
}

/// AS-IS F49: compute `current+1` but leave the counter unmoved (collision).
#[must_use]
pub fn reserve_si_gen_as_is(current: u64) -> SiGenReserve {
    SiGenReserve {
        next_current: current,
        reserved: current.saturating_add(1),
    }
}

/// Undo a reserve only if nothing else reserved after us (propose failed).
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
