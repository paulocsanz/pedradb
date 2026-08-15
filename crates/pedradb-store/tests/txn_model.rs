//! Stateright model over the **real** [`pedradb_store`] txn kernel (F47 / F34 / discard cut).
//!
//! Faithfulness: every apply / revert / discard decision is production
//! `txn_kernel`, not a paraphrase. The model owns only a one-txn status key,
//! one user key, and a commit watermark.
//!
//! - **Inv-abort-fence:** an abort status never materialises user keys.
//! - **Inv-discard:** never cut at or below `commit`.
//! - **Inv-preimage:** missing prepare record is not treated as "was absent".
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_store::{
    discard_cut, discard_cut_as_is, leftover_txn_is_aborted, leftover_txn_is_aborted_as_is,
    revert_clears_status, revert_clears_status_as_is, revert_user_action, revert_user_action_as_is,
    txn_commit_action, txn_commit_action_as_is, RevertUserAction, TxnCommitAction,
};
use stateright::{Checker, Model, Property};

const COMMIT: u64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Status {
    None,
    Abort,
    Commit,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    status: Status,
    materialised_abort: bool,
    discarded_committed: bool,
    blind_deleted: bool,
    saw_revert: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Apply,
    ClearStatus,
    RevertUser { had_pre: bool, pre_absent: bool },
    Discard { from: u64 },
}

#[derive(Clone)]
struct TxnModel {
    /// When false, use the AS-IS mutants (F47 / F34 / discard cut).
    fixed: bool,
}

impl TxnModel {
    fn commit_action(&self, abort: bool) -> TxnCommitAction {
        if self.fixed {
            txn_commit_action(abort)
        } else {
            txn_commit_action_as_is(abort)
        }
    }

    fn leftover_abort(&self) -> bool {
        if self.fixed {
            leftover_txn_is_aborted()
        } else {
            leftover_txn_is_aborted_as_is()
        }
    }

    fn clears_status(&self, abort: bool, pairs_empty: bool) -> bool {
        if self.fixed {
            revert_clears_status(abort, pairs_empty)
        } else {
            revert_clears_status_as_is(abort, pairs_empty)
        }
    }

    fn revert_user(&self, had_pre: bool, pre_absent: bool) -> RevertUserAction {
        if self.fixed {
            revert_user_action(had_pre, pre_absent)
        } else {
            revert_user_action_as_is(had_pre, pre_absent)
        }
    }

    fn cut(&self, from: u64) -> u64 {
        if self.fixed {
            discard_cut(from, COMMIT)
        } else {
            discard_cut_as_is(from, COMMIT)
        }
    }

    fn is_abort(&self, status: Status) -> bool {
        match status {
            Status::Abort => true,
            Status::Commit => false,
            Status::None => self.leftover_abort(),
        }
    }
}

impl Model for TxnModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![
            St {
                status: Status::Abort,
                materialised_abort: false,
                discarded_committed: false,
                blind_deleted: false,
                saw_revert: false,
            },
            St {
                status: Status::None,
                materialised_abort: false,
                discarded_committed: false,
                blind_deleted: false,
                saw_revert: false,
            },
            St {
                status: Status::Commit,
                materialised_abort: false,
                discarded_committed: false,
                blind_deleted: false,
                saw_revert: false,
            },
        ]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.push(Act::Apply);
        actions.push(Act::ClearStatus);
        for had_pre in [false, true] {
            for pre_absent in [false, true] {
                actions.push(Act::RevertUser {
                    had_pre,
                    pre_absent,
                });
            }
        }
        for from in 1..=4 {
            actions.push(Act::Discard { from });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Apply => {
                let abort = self.is_abort(state.status);
                match self.commit_action(abort) {
                    TxnCommitAction::Materialise => {
                        if abort {
                            next.materialised_abort = true;
                        }
                    }
                    TxnCommitAction::Revert => next.saw_revert = true,
                }
            }
            Act::ClearStatus => {
                let abort = state.status == Status::Abort;
                if self.clears_status(abort, true) {
                    if abort {
                        // Fence evaporated; a later Apply still sees leftover.
                        next.status = Status::None;
                    } else {
                        next.status = Status::None;
                    }
                }
            }
            Act::RevertUser {
                had_pre,
                pre_absent,
            } => {
                if !had_pre
                    && self.revert_user(had_pre, pre_absent) == RevertUserAction::RestoreAbsent
                {
                    next.blind_deleted = true;
                }
            }
            Act::Discard { from } => {
                if self.cut(from) <= COMMIT {
                    next.discarded_committed = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-abort-fence", inv_abort_fence),
            Property::always("Inv-discard", inv_discard),
            Property::always("Inv-preimage", inv_preimage),
            Property::sometimes("non-vacuity-revert", non_vacuity_revert),
        ]
    }
}

fn inv_abort_fence(_: &TxnModel, s: &St) -> bool {
    !s.materialised_abort
}

fn inv_discard(_: &TxnModel, s: &St) -> bool {
    !s.discarded_committed
}

fn inv_preimage(_: &TxnModel, s: &St) -> bool {
    !s.blind_deleted
}

fn non_vacuity_revert(_: &TxnModel, s: &St) -> bool {
    s.saw_revert
}

#[test]
fn fixed_txn_kernel_holds() {
    let checker = TxnModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_mutant_breaks_abort_fence() {
    let checker = TxnModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-abort-fence").is_some(),
        "AS-IS must materialise an aborted TX (F47 teeth)"
    );
}

#[test]
fn as_is_mutant_cuts_committed() {
    let checker = TxnModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-discard").is_some(),
        "AS-IS discard_cut must cut ≤ commit (teeth)"
    );
}

#[test]
fn as_is_mutant_blind_deletes() {
    let checker = TxnModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-preimage").is_some(),
        "AS-IS revert must treat missing preimage as absent (F34 teeth)"
    );
}
