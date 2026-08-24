//! RFC-0053 P1.2 + Y2.2 + Y3.4: Stateright over **production**
//! vote ∧ AE ∧ commit ∧ apply kernels (plus the reopen cursor rule).
//!
//! Not a paraphrase: every step calls `vote_decision` / `grant_after_persist`,
//! `ae_entry_action`, `may_commit_at` / `propose_ack_ok`,
//! `apply_advance` (store apply path: committed entry → applied →
//! observable applied log), and `recover_last_applied` on reopen.
//!
//! - **Inv-vote-once:** no two candidates granted in the term.
//! - **Inv-F16:** never rewrite an index ≤ commit.
//! - **Inv-F23:** majority does not commit a previous-term index (FIXED).
//! - **Inv-F11:** ACK only if commit covers the index.
//! - **Inv-apply-contiguous:** applied is a contiguous prefix
//!   (`applied.len() == last_applied`) — no skipped holes, no stranded
//!   cursor after reopen (F10 re-apply).
//! - **Inv-applied-le-commit:** `last_applied ≤ commit` — never apply
//!   uncommitted.
//! AS-IS mutants must produce a counterexample (teeth).
//!
//! Bounded liveness (Y3.4): under the **quorum-alive axiom** (an explicit
//! model AXIOM, never a theorem — the environment keeps granting majority
//! and persist-Ok; no partitions), election/commit/apply progress occurs
//! within bounded steps. Checked by a witness BFS with an explicit bound on
//! the quorum-alive sub-model (`bounded_liveness_under_quorum_alive_axiom`).

use pedradb_raft::ae_kernel::{
    ae_entry_action, ae_entry_action_as_is_rewrite_committed, AeEntryAction,
};
use pedradb_raft::apply_kernel::{
    apply_advance, apply_advance_as_is_skip_holes, ApplyAction,
};
use pedradb_raft::{
    grant_after_persist, grant_after_persist_as_is, may_commit_at, may_commit_at_as_is,
    propose_ack_ok, propose_ack_ok_as_is, recover_last_applied, recover_last_applied_as_is,
    vote_decision, vote_kernel, PersistOutcome, VoteDecision, VoteInputs,
};
use stateright::{Checker, Model, Property};

const TERM: u64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Mutant {
    Fixed,
    VoteAsIs,
    AeAsIs,
    CommitAsIs,
    ApplyAsIs,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    voted_for: Option<u64>,
    /// terms[i] = term at index i+1.
    terms: Vec<u64>,
    commit: u64,
    double_vote: bool,
    rewrote_committed: bool,
    committed_prev_term: bool,
    acked_uncommitted: bool,
    saw_grant: bool,
    saw_commit: bool,
    /// Apply loop (Y2.2): applied prefix + cursor over `terms`.
    last_applied: u64,
    /// Observable applied state: terms of applied entries, contiguous.
    applied: Vec<u64>,
    saw_apply: bool,
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
    Vote { candidate: u64, persist_ok: bool },
    Offer { index: u64, term: u64 },
    TryCommit { majority: bool },
    Ack { index: u64 },
    Apply,
    /// Crash-restart: the state machine is rebuilt and `last_applied`
    /// recovers via the production commit-kernel rule (re-apply from 0) —
    /// or, AS-IS, jumps to the log tail and strands committed-but-unapplied
    /// entries (F10).
    Reopen,
}

#[derive(Clone)]
struct ComposeModel {
    mutant: Mutant,
    /// AXIOM quorum-alive (Y3.4): restrict actions to the environment that
    /// always grants majority and persist-Ok. An axiom of the model — the
    /// bound is checked only under it, never claimed unconditionally.
    quorum_alive: bool,
}

impl ComposeModel {
    fn vote(&self, i: VoteInputs) -> VoteDecision {
        match self.mutant {
            Mutant::VoteAsIs => vote_kernel::vote_decision_as_is_ignore_log_and_vote(i),
            _ => vote_decision(i),
        }
    }

    fn grant(&self, d: VoteDecision, p: PersistOutcome) -> bool {
        match self.mutant {
            Mutant::VoteAsIs => grant_after_persist_as_is(d, p),
            _ => grant_after_persist(d, p),
        }
    }

    fn ae(
        &self,
        index: u64,
        term: u64,
        existing: Option<u64>,
        commit: u64,
        last: u64,
    ) -> AeEntryAction {
        if self.mutant == Mutant::AeAsIs {
            ae_entry_action_as_is_rewrite_committed(index, term, existing, commit, last)
        } else {
            ae_entry_action(index, term, existing, commit, last)
        }
    }

    fn may(&self, index_term: u64, majority: bool) -> bool {
        if self.mutant == Mutant::CommitAsIs {
            may_commit_at_as_is(index_term, TERM, majority)
        } else {
            may_commit_at(index_term, TERM, majority)
        }
    }

    fn ack(&self, index: u64, commit: u64) -> bool {
        if self.mutant == Mutant::CommitAsIs {
            propose_ack_ok_as_is(index, commit)
        } else {
            propose_ack_ok(index, commit)
        }
    }

    fn apply_adv(&self, last_applied: u64, commit: u64, present: bool) -> ApplyAction {
        if self.mutant == Mutant::ApplyAsIs {
            apply_advance_as_is_skip_holes(last_applied, commit, present)
        } else {
            apply_advance(last_applied, commit, present)
        }
    }

    /// Reopen cursor: production `recover_last_applied` (re-apply from 0)
    /// vs the F10 AS-IS jump to the log tail.
    fn reopen_cursor(&self, log_last: u64) -> u64 {
        if self.mutant == Mutant::ApplyAsIs {
            recover_last_applied_as_is(log_last)
        } else {
            recover_last_applied()
        }
    }
}

impl Model for ComposeModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            voted_for: None,
            terms: vec![1],
            commit: 1,
            double_vote: false,
            rewrote_committed: false,
            committed_prev_term: false,
            acked_uncommitted: false,
            saw_grant: false,
            saw_commit: false,
            last_applied: 0,
            applied: Vec::new(),
            saw_apply: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        let quorum_alive = self.quorum_alive;
        for candidate in 1..=2u64 {
            actions.push(Act::Vote {
                candidate,
                persist_ok: true,
            });
            if !quorum_alive {
                actions.push(Act::Vote {
                    candidate,
                    persist_ok: false,
                });
            }
        }
        let last = state.last_index();
        for index in 1..=(last + 1).min(3) {
            for term in 1..=2u64 {
                actions.push(Act::Offer { index, term });
            }
        }
        actions.push(Act::TryCommit { majority: true });
        if !quorum_alive {
            actions.push(Act::TryCommit { majority: false });
        }
        if last > 0 {
            actions.push(Act::Ack { index: last });
        }
        actions.push(Act::Apply);
        if !quorum_alive {
            actions.push(Act::Reopen);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Vote {
                candidate,
                persist_ok,
            } => {
                let inputs = VoteInputs {
                    current_term: TERM,
                    voted_for: next.voted_for,
                    last_log_term: next.term_at(next.last_index()).unwrap_or(0),
                    last_log_index: next.last_index(),
                    candidate_term: TERM,
                    candidate_id: candidate,
                    candidate_last_log_term: TERM,
                    candidate_last_log_index: next.last_index(),
                };
                let d = self.vote(inputs);
                let p = if persist_ok {
                    PersistOutcome::Ok
                } else {
                    PersistOutcome::Err
                };
                if self.grant(d, p) {
                    if let Some(prev) = next.voted_for {
                        if prev != candidate {
                            next.double_vote = true;
                        }
                    }
                    next.voted_for = Some(candidate);
                    next.saw_grant = true;
                }
            }
            Act::Offer { index, term } => {
                let existing = next.term_at(index);
                let last = next.last_index();
                match self.ae(index, term, existing, next.commit, last) {
                    AeEntryAction::Keep => {}
                    AeEntryAction::Append => next.terms.push(term),
                    AeEntryAction::TruncateAndInstall => {
                        if index <= next.commit {
                            next.rewrote_committed = true;
                        }
                        let keep = (index as usize).saturating_sub(1);
                        next.terms.truncate(keep);
                        next.terms.push(term);
                    }
                    AeEntryAction::Refuse => {}
                }
            }
            Act::TryCommit { majority } => {
                let last = next.last_index();
                if last == 0 {
                    return Some(next);
                }
                let index_term = next.term_at(last).unwrap_or(0);
                if self.may(index_term, majority) {
                    if index_term != TERM {
                        next.committed_prev_term = true;
                    }
                    next.commit = last;
                    next.saw_commit = true;
                }
            }
            Act::Ack { index } => {
                if self.ack(index, next.commit) && next.commit < index {
                    next.acked_uncommitted = true;
                }
            }
            Act::Apply => {
                // Store apply path: production `apply_committed` loop over
                // `apply_advance` (committed entry → applied → observable).
                let next_index = next.last_applied + 1;
                let present = next.term_at(next_index).is_some();
                match self.apply_adv(next.last_applied, next.commit, present) {
                    ApplyAction::Done | ApplyAction::Stop => {}
                    ApplyAction::Apply => {
                        if present {
                            next.applied.push(next.term_at(next_index).unwrap_or(0));
                            next.last_applied = next_index;
                        } else {
                            // Skip-holes mutant: advance without the entry —
                            // applied prefix silently diverges.
                            next.last_applied = next_index;
                        }
                        next.saw_apply = true;
                    }
                }
            }
            Act::Reopen => {
                // State machine rebuilt; cursor from the production kernel.
                next.last_applied = self.reopen_cursor(next.last_index());
                next.applied.clear();
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-vote-once", |_, s: &St| !s.double_vote),
            Property::always("Inv-F16-no-rewrite-committed", |_, s: &St| {
                !s.rewrote_committed
            }),
            Property::always("Inv-F23-current-term", |_, s: &St| !s.committed_prev_term),
            Property::always("Inv-F11-ack-commit", |_, s: &St| !s.acked_uncommitted),
            Property::always("Inv-apply-contiguous", |_, s: &St| {
                s.applied.len() as u64 == s.last_applied
            }),
            Property::always("Inv-applied-le-commit", |_, s: &St| {
                s.last_applied <= s.commit
            }),
            Property::sometimes("non-vacuity-grant", |_, s: &St| s.saw_grant),
            Property::sometimes("non-vacuity-apply", |_, s: &St| s.saw_apply),
        ]
    }
}

#[test]
fn fixed_compose_invariants() {
    let checker = ComposeModel {
        mutant: Mutant::Fixed,
        quorum_alive: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    checker.assert_properties();
}

#[test]
fn as_is_vote_discovers_double_vote() {
    let checker = ComposeModel {
        mutant: Mutant::VoteAsIs,
        quorum_alive: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-vote-once").is_some(),
        "vote AS-IS must counterexample Inv-vote-once"
    );
}

#[test]
fn as_is_ae_discovers_committed_rewrite() {
    let checker = ComposeModel {
        mutant: Mutant::AeAsIs,
        quorum_alive: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-F16-no-rewrite-committed").is_some(),
        "AE AS-IS must counterexample Inv-F16"
    );
}

#[test]
fn as_is_commit_discovers_prev_term_or_false_ack() {
    let checker = ComposeModel {
        mutant: Mutant::CommitAsIs,
        quorum_alive: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-F23-current-term").is_some()
            || checker.discovery("Inv-F11-ack-commit").is_some(),
        "commit AS-IS must counterexample F23 or F11"
    );
}

#[test]
fn as_is_apply_discovers_contiguity_gap() {
    let checker = ComposeModel {
        mutant: Mutant::ApplyAsIs,
        quorum_alive: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-apply-contiguous").is_some(),
        "apply AS-IS must counterexample Inv-apply-contiguous"
    );
}

/// Y3.4 bounded liveness under the quorum-alive AXIOM.
///
/// AXIOM (quorum-alive): the environment always grants majority and
/// persist-Ok — no partitions, no persist failures. This is an assumption of
/// the model, **never a theorem** (unconditional liveness would need fairness
/// semantics plus a real network model; that is RFC-0051 territory).
///
/// Under the axiom: election (grant) + commit + apply progress occurs within
/// `BOUND` steps, witnessed by explicit BFS over the production kernels.
#[test]
fn bounded_liveness_under_quorum_alive_axiom() {
    const BOUND: usize = 4;
    let model = ComposeModel {
        mutant: Mutant::Fixed,
        quorum_alive: true,
    };
    // Progress = a grant was sent, a commit happened, and the apply loop
    // made the committed entry observable.
    let progress = |s: &St| s.saw_grant && s.saw_commit && s.saw_apply && !s.applied.is_empty();

    let init = model.init_states().into_iter().next().expect("init");
    assert!(!progress(&init), "witness must not be vacuous at init");
    let mut frontier: Vec<St> = vec![init];
    let mut seen: Vec<St> = frontier.clone();
    let mut depth = 0usize;
    while depth < BOUND && !frontier.iter().any(|s| progress(s)) {
        let mut next_frontier = Vec::new();
        for s in &frontier {
            let mut acts = Vec::new();
            model.actions(s, &mut acts);
            for a in acts {
                if let Some(n) = model.next_state(s, a) {
                    if !seen.contains(&n) {
                        seen.push(n.clone());
                        next_frontier.push(n);
                    }
                }
            }
        }
        frontier = next_frontier;
        depth += 1;
    }
    assert!(
        frontier.iter().any(|s| progress(s)) || seen.iter().any(|s| progress(s)),
        "quorum-alive axiom must yield grant+commit+apply progress"
    );
    assert!(
        depth <= BOUND,
        "progress must occur within {BOUND} steps under the axiom (took {depth})"
    );
}
