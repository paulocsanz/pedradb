//! Stateright model over the **real** [`pedradb_raft::ae_entry_action`] kernel (F16 / RFC-0002 P1.1+P1.3).
//!
//! Faithfulness: every log step is decided by production `ae_kernel`, not a paraphrase.
//! Model owns only a single follower log + commit_index and incoming AE entries.
//!
//! - **Inv-F16:** never truncate/replace an index `≤ commit_index`.
//! - AS-IS mutant rewrites committed indexes → counterexample required.

use pedradb_raft::ae_kernel::{
    ae_entry_action, ae_entry_action_as_is_rewrite_committed, AeEntryAction,
};
use stateright::{Checker, Model, Property};

/// Compact log: term at each index 1..=len (index 0 unused).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    /// terms[i] = term of entry at index i+1; len = last_log_index
    terms: Vec<u64>,
    commit_index: u64,
    /// Latch: attempted rewrite at/before commit.
    rewrote_committed: bool,
    saw_refuse: bool,
    saw_truncate: bool,
}

impl St {
    fn last_index(&self) -> u64 {
        self.terms.len() as u64
    }
    fn term_at(&self, index: u64) -> Option<u64> {
        if index == 0 || index as usize > self.terms.len() {
            None
        } else {
            Some(self.terms[index as usize - 1])
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    /// Leader offers entry (index, term). Index may equal an existing slot (conflict) or last+1.
    Offer { index: u64, term: u64 },
    /// Advance commit (leader commit gossip) up to last_index, not past.
    BumpCommit,
}

#[derive(Clone)]
struct AeModel {
    fixed: bool,
}

impl AeModel {
    fn decide(
        &self,
        entry_index: u64,
        entry_term: u64,
        existing: Option<u64>,
        commit: u64,
        last: u64,
    ) -> AeEntryAction {
        if self.fixed {
            ae_entry_action(entry_index, entry_term, existing, commit, last)
        } else {
            ae_entry_action_as_is_rewrite_committed(entry_index, entry_term, existing, commit, last)
        }
    }
}

impl Model for AeModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        // Log: indices 1,2,3 with terms 1,1,2; commit through 2.
        vec![St {
            terms: vec![1, 1, 2],
            commit_index: 2,
            rewrote_committed: false,
            saw_refuse: false,
            saw_truncate: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.push(Act::BumpCommit);
        let last = state.last_index();
        // Conflicts and appends in a small term/index box.
        for index in 1..=(last + 1).min(5) {
            for term in 1..=4u64 {
                actions.push(Act::Offer { index, term });
            }
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::BumpCommit => {
                let last = next.last_index();
                if next.commit_index < last {
                    next.commit_index += 1;
                }
            }
            Act::Offer { index, term } => {
                let existing = next.term_at(index);
                let last = next.last_index();
                let act = self.decide(index, term, existing, next.commit_index, last);
                match act {
                    AeEntryAction::Keep => {}
                    AeEntryAction::Append => {
                        next.terms.push(term);
                    }
                    AeEntryAction::TruncateAndInstall => {
                        if index <= next.commit_index {
                            next.rewrote_committed = true;
                        }
                        // Truncate from index and install.
                        let keep = (index as usize).saturating_sub(1);
                        next.terms.truncate(keep);
                        next.terms.push(term);
                        next.saw_truncate = true;
                    }
                    AeEntryAction::Refuse => {
                        next.saw_refuse = true;
                    }
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-F16-no-rewrite-committed", inv_f16),
            Property::sometimes("non-vacuity-refuse", non_vacuity_refuse),
        ]
    }
}

fn inv_f16(_: &AeModel, s: &St) -> bool {
    !s.rewrote_committed
}

fn non_vacuity_refuse(_: &AeModel, s: &St) -> bool {
    s.saw_refuse
}

#[test]
fn fixed_ae_never_rewrites_committed() {
    let checker = AeModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_mutant_rewrites_committed() {
    let checker = AeModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-F16-no-rewrite-committed").is_some(),
        "AS-IS mutant must counterexample Inv-F16 (teeth)"
    );
}
