//! Stateright model over the **real** [`pedradb_raft::commit_kernel`] (F10 / F11 / F23).
//!
//! Faithfulness: recover watermark, §5.4.2 commit, and client ACK are
//! production `commit_kernel`, not a paraphrase. Persist of `RAFT_COMMIT`
//! and apply are axioms (`WouldCommit` ⇒ durable in this state).
//!
//! The store copy is the catalog clone (`commit_raft_store`); tokens must
//! stay identical. This model runs the raft body.
//!
//! - **Inv-no-suffix:** reopen never promotes `log_last` over durable commit (F10).
//! - **Inv-reapply:** reopen `last_applied` is 0 (F10).
//! - **Inv-current-term:** majority alone does not commit a prev-term index (F23).
//! - **Inv-ack-commit:** client Ok only if commit covers the index (F11).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_raft::{
    may_commit_at, may_commit_at_as_is, propose_ack_ok, propose_ack_ok_as_is, recover_commit,
    recover_commit_as_is, recover_last_applied, recover_last_applied_as_is,
};
use stateright::{Checker, Model, Property};

const LOG_CAP: u64 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    term: u64,
    log_last: u64,
    last_term: u64,
    commit: u64,
    durable_commit: u64,
    applied: u64,
    recovered_suffix: bool,
    skipped_reapply: bool,
    committed_prev_term: bool,
    acked_uncommitted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Append,
    BumpTerm,
    TryCommit { majority: bool },
    Ack,
    CrashReopen,
}

#[derive(Clone)]
struct CommitModel {
    fixed: bool,
}

impl CommitModel {
    fn recover(&self, loaded: u64, last: u64) -> u64 {
        if self.fixed {
            recover_commit(loaded, last)
        } else {
            recover_commit_as_is(loaded, last)
        }
    }

    fn last_applied(&self, last: u64) -> u64 {
        if self.fixed {
            recover_last_applied()
        } else {
            recover_last_applied_as_is(last)
        }
    }

    fn may(&self, index_term: u64, current: u64, majority: bool) -> bool {
        if self.fixed {
            may_commit_at(index_term, current, majority)
        } else {
            may_commit_at_as_is(index_term, current, majority)
        }
    }

    fn ack(&self, index: u64, commit: u64) -> bool {
        if self.fixed {
            propose_ack_ok(index, commit)
        } else {
            propose_ack_ok_as_is(index, commit)
        }
    }
}

impl Model for CommitModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            term: 1,
            log_last: 0,
            last_term: 0,
            commit: 0,
            durable_commit: 0,
            applied: 0,
            recovered_suffix: false,
            skipped_reapply: false,
            committed_prev_term: false,
            acked_uncommitted: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.log_last < LOG_CAP {
            actions.push(Act::Append);
        }
        if state.term == 1 {
            actions.push(Act::BumpTerm);
        }
        if state.log_last > 0 {
            actions.push(Act::TryCommit { majority: true });
            actions.push(Act::TryCommit { majority: false });
            actions.push(Act::Ack);
        }
        actions.push(Act::CrashReopen);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Append => {
                if state.log_last >= LOG_CAP {
                    return None;
                }
                next.log_last = state.log_last + 1;
                next.last_term = state.term;
            }
            Act::BumpTerm => {
                if state.term != 1 {
                    return None;
                }
                next.term = 2;
            }
            Act::TryCommit { majority } => {
                if state.log_last == 0 {
                    return None;
                }
                if self.may(state.last_term, state.term, majority) {
                    if state.last_term != state.term {
                        next.committed_prev_term = true;
                    }
                    next.commit = state.log_last;
                    next.durable_commit = state.log_last;
                }
            }
            Act::Ack => {
                if state.log_last == 0 {
                    return None;
                }
                if self.ack(state.log_last, state.commit) && state.commit < state.log_last {
                    next.acked_uncommitted = true;
                }
            }
            Act::CrashReopen => {
                let c = self.recover(state.durable_commit, state.log_last);
                if c > state.durable_commit {
                    next.recovered_suffix = true;
                }
                next.commit = c;
                let a = self.last_applied(state.log_last);
                if a != 0 {
                    next.skipped_reapply = true;
                }
                next.applied = a;
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-suffix", inv_no_suffix),
            Property::always("Inv-reapply", inv_reapply),
            Property::always("Inv-current-term", inv_current_term),
            Property::always("Inv-ack-commit", inv_ack_commit),
            Property::sometimes("non-vacuity-append", non_vacuity_append),
            Property::sometimes("non-vacuity-commit", non_vacuity_commit),
        ]
    }
}

fn inv_no_suffix(_: &CommitModel, s: &St) -> bool {
    !s.recovered_suffix
}

fn inv_reapply(_: &CommitModel, s: &St) -> bool {
    !s.skipped_reapply
}

fn inv_current_term(_: &CommitModel, s: &St) -> bool {
    !s.committed_prev_term
}

fn inv_ack_commit(_: &CommitModel, s: &St) -> bool {
    !s.acked_uncommitted
}

fn non_vacuity_append(_: &CommitModel, s: &St) -> bool {
    s.log_last > 0
}

fn non_vacuity_commit(_: &CommitModel, s: &St) -> bool {
    s.durable_commit > 0
}

#[test]
fn fixed_commit_holds() {
    let checker = CommitModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_promotes_suffix() {
    let checker = CommitModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-suffix").is_some(),
        "AS-IS recover must promote log_last over durable commit (F10 teeth)"
    );
}

#[test]
fn as_is_skips_reapply() {
    let checker = CommitModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-reapply").is_some(),
        "AS-IS last_applied must jump to log_last (F10 teeth)"
    );
}

#[test]
fn as_is_commits_prev_term() {
    let checker = CommitModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-current-term").is_some(),
        "AS-IS majority must commit a prev-term index (F23 teeth)"
    );
}

#[test]
fn as_is_acks_uncommitted() {
    let checker = CommitModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-ack-commit").is_some(),
        "AS-IS must ACK before commit covers the index (F11 teeth)"
    );
}
