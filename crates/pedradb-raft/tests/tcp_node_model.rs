//! RFC-0056 P1.3: the raft node's TCP loop as a state machine.
//!
//! One Stateright action = ONE wire frame processed by the node:
//!   read frame → dispatch to the production kernel → persist → send reply.
//! The reduction claim: the loop's scheduling adds no state beyond (frame,
//! node state); frame interleaving is captured by the model choosing the
//! next frame, and the persist outcome is part of the environment
//! (frame-carried). This refines the per-kernel models (vote, AE, commit,
//! apply) into the node's actual dispatch shape — the same production
//! kernels are called, now inside one RPC-shaped step.
//!
//! Invariants (each asserted on the observable RPC log + node state):
//! - **Inv-vote-once:** no two candidates granted in the term.
//! - **Inv-F16:** never rewrite an index ≤ commit.
//! - **Inv-F11-no-dirty-ack:** every RespOk reply to a Propose is sent only
//!   when commit already covers the index at send time.
//! - **Inv-send-after-persist:** a grant reply (WouldGrant) is sent only
//!   after the vote persisted Ok — the loop never answers before the disk.
//! - **Inv-apply-contiguous:** the applied log is a contiguous prefix.
//! - **Inv-applied-le-commit:** never apply uncommitted.
//!
//! AS-IS loop mutants must counterexample (teeth at the loop level, not just
//! the kernel level):
//! - `SendGrantBeforePersist` — replies the grant computed at read time and
//!   ignores the persist outcome (Inv-send-after-persist).
//! - `DirtyAckLoop` — replies RespOk without consulting `propose_ack_ok`
//!   (Inv-F11).
//! - `SkipAeGuard` — dispatches AppendEntries without `ae_entry_action`,
//!   installing over committed indexes (Inv-F16).
//!
//! RFC-0056 P2.2 — liveness (IronFleet-class) under eventual synchrony. The
//! eventual properties are checked by `Property::eventually` on behaviors
//! bounded by `MAX_STEPS` (stateright BFS evaluates eventuality at the
//! terminal states of path-acyclic models, so the step bound IS what makes
//! the check sound and non-vacuous). The claim is therefore precise bounded
//! liveness: **every** behavior of at most `MAX_STEPS` steps satisfies the
//! eventuality within the bound, and the axioms below guarantee it within
//! `ES_BOUND + 1` steps. They are **not theorems**: each holds only under
//! three AXIOMS named here and in the TCB
//! (`docs/formal/one-hundred-percent.md`):
//! - **AXIOM ES-1 (bounded adversary / eventual synchrony):** the
//!   environment is adversarial for at most `ES_BOUND` steps — crash-restarts
//!   are finite, no infinite partition. After that, persist never fails.
//! - **AXIOM ES-2 (internal drain):** once synchronized, the node's apply
//!   loop runs to quiescence before the next frame is read (internal
//!   progress is not schedulable away).
//! - **AXIOM ES-3 (live retrying candidate):** once synchronized, while the
//!   node has not granted any vote, a live candidate's RequestVote keeps
//!   being delivered.
//! Teeth: without the axioms (unrestricted model) BOTH eventual properties
//! are refuted; without ES-3 alone, `Evt-election` is refuted while
//! `Evt-apply-quiescence` still holds (each axiom is load-bearing); a loop
//! mutant that breaks the drain is caught even with all axioms on.

use pedradb_raft::ae_kernel::{ae_entry_action, AeEntryAction};
use pedradb_raft::apply_kernel::{apply_advance, ApplyAction};
use pedradb_raft::{
    grant_after_persist, may_commit_at, propose_ack_ok, vote_decision, PersistOutcome,
    VoteDecision, VoteInputs,
};
use stateright::{Checker, Model, Property};

const TERM: u64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum LoopMutant {
    Fixed,
    /// Reply WouldGrant computed at read time; persist outcome ignored.
    SendGrantBeforePersist,
    /// Reply RespOk to every Propose without the commit-cover check.
    DirtyAckLoop,
    /// Install AppendEntries without the AE kernel guard.
    SkipAeGuard,
}

/// Wire frame read by the node (one per step). The environment picks the
/// frame — that is the interleaving the TCP loop actually experiences — and
/// carries the persist outcome where the step persists.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Frame {
    ReqVote { candidate: u64, persist_ok: bool },
    AppendEntries { index: u64, term: u64 },
    /// Leader ack path: client asks whether index i may be acknowledged.
    Propose { index: u64 },
    /// Internal drain: apply committed prefix (still one loop step: the node
    /// reads its commit state, dispatches apply_advance, persists the cursor,
    /// replies nothing).
    ApplyTick,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct NodeSt {
    voted_for: Option<u64>,
    /// terms[i] = term at index i+1.
    terms: Vec<u64>,
    commit: u64,
    last_applied: u64,
    applied: Vec<u64>,
    // Invariant witness flags (kept instead of an RPC log: the log would
    // make every state unique and the BFS unbounded).
    double_vote: bool,
    rewrote_committed: bool,
    /// Set at SEND time: a RespOk left the node for an index its commit did
    /// not cover (the final state's commit may have advanced later, so the
    /// flag is the faithful witness).
    acked_uncommitted: bool,
    /// Set at SEND time: a grant reply left the node although the vote had
    /// not persisted Ok.
    granted_without_persist: bool,
    saw_grant_reply: bool,
    saw_ack_ok: bool,
}

impl NodeSt {
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

#[derive(Clone)]
struct TcpNodeModel {
    mutant: LoopMutant,
}

impl TcpNodeModel {
    /// Dispatch: the vote kernel, then persist, then send.
    fn step_vote(&self, st: &mut NodeSt, candidate: u64, persist_ok: bool) {
        let inputs = VoteInputs {
            current_term: TERM,
            voted_for: st.voted_for,
            last_log_term: st.term_at(st.last_index()).unwrap_or(0),
            last_log_index: st.last_index(),
            candidate_term: TERM,
            candidate_id: candidate,
            candidate_last_log_term: TERM,
            candidate_last_log_index: st.last_index(),
        };
        let d = vote_decision(inputs);
        let p = if persist_ok {
            PersistOutcome::Ok
        } else {
            PersistOutcome::Err
        };
        let granted = grant_after_persist(d, p);
        let send_grant = match self.mutant {
            // Fixed: the reply is sent AFTER the persist outcome is known —
            // a failed persist answers Deny (the disk is the truth).
            LoopMutant::Fixed => granted,
            // Mutant: the reply was computed at read time; the persist
            // failure arrives after the frame went out.
            LoopMutant::SendGrantBeforePersist => d == VoteDecision::WouldGrant,
            _ => granted,
        };
        if send_grant {
            if !persist_ok {
                st.granted_without_persist = true;
            }
            Self::record_grant(st, candidate);
            st.saw_grant_reply = true;
        }
    }

    fn record_grant(st: &mut NodeSt, candidate: u64) {
        if let Some(prev) = st.voted_for {
            if prev != candidate {
                st.double_vote = true;
            }
        }
        st.voted_for = Some(candidate);
    }

    /// Dispatch: the AE kernel guards the install; persist; reply Ok.
    /// Returns whether the node answered Ok (the reply itself is a wire
    /// artifact; the invariant witnesses are the state flags).
    fn step_append(&self, st: &mut NodeSt, index: u64, term: u64) -> bool {
        let existing = st.term_at(index);
        let last = st.last_index();
        let action = match self.mutant {
            // Mutant: dispatch skips the kernel — unguarded install.
            LoopMutant::SkipAeGuard => AeEntryAction::TruncateAndInstall,
            _ => ae_entry_action(index, term, existing, st.commit, last),
        };
        match action {
            AeEntryAction::Keep => true,
            AeEntryAction::Append => {
                st.terms.push(term);
                true
            }
            AeEntryAction::TruncateAndInstall => {
                if index <= st.commit {
                    st.rewrote_committed = true;
                }
                let keep = (index as usize).saturating_sub(1);
                st.terms.truncate(keep);
                st.terms.push(term);
                true
            }
            AeEntryAction::Refuse => false,
        }
    }

    /// Leader commit rule on a successful AppendEntries round (the commit
    /// decision is still the production kernel).
    fn advance_commit(&self, st: &mut NodeSt) {
        let last = st.last_index();
        if last == 0 {
            return;
        }
        let index_term = st.term_at(last).unwrap_or(0);
        if may_commit_at(index_term, TERM, true) {
            st.commit = last;
        }
    }

    /// Dispatch: the ack gate kernel decides what may be answered.
    fn step_propose(&self, st: &mut NodeSt, index: u64) {
        let ok = match self.mutant {
            // Mutant: reply Ok without consulting the commit-cover gate.
            LoopMutant::DirtyAckLoop => true,
            _ => propose_ack_ok(index, st.commit),
        };
        if ok {
            // Witness at send time: an Ok for an uncovered index is a dirty
            // ack even if commit catches up later.
            if st.commit < index {
                st.acked_uncommitted = true;
            }
            st.saw_ack_ok = true;
        }
    }

    /// Apply loop: production `apply_advance` over the committed prefix.
    fn step_apply(&self, st: &mut NodeSt) {
        let next_index = st.last_applied + 1;
        let present = st.term_at(next_index).is_some();
        match apply_advance(st.last_applied, st.commit, present) {
            ApplyAction::Done | ApplyAction::Stop => {}
            ApplyAction::Apply => {
                if present {
                    st.applied.push(st.term_at(next_index).unwrap_or(0));
                    st.last_applied = next_index;
                }
            }
        }
    }

    /// AXIOM ES-2 (internal drain): apply until quiescence, one production
    /// `apply_advance` per entry.
    fn drain_apply(&self, st: &mut NodeSt) {
        loop {
            let before = st.last_applied;
            self.step_apply(st);
            if st.last_applied == before {
                break;
            }
        }
    }

    /// One loop step, shared by the safety model and the liveness model:
    /// read (frame) → dispatch to the production kernel → persist → send.
    fn dispatch(&self, st: &mut NodeSt, action: Frame) {
        match action {
            Frame::ReqVote {
                candidate,
                persist_ok,
            } => self.step_vote(st, candidate, persist_ok),
            Frame::AppendEntries { index, term } => {
                if self.step_append(st, index, term) {
                    self.advance_commit(st);
                }
            }
            Frame::Propose { index } => self.step_propose(st, index),
            Frame::ApplyTick => self.step_apply(st),
        }
    }
}

impl Model for TcpNodeModel {
    type State = NodeSt;
    type Action = Frame;

    fn init_states(&self) -> Vec<Self::State> {
        vec![NodeSt {
            voted_for: None,
            terms: vec![1],
            commit: 1,
            last_applied: 0,
            applied: Vec::new(),
            double_vote: false,
            rewrote_committed: false,
            acked_uncommitted: false,
            granted_without_persist: false,
            saw_grant_reply: false,
            saw_ack_ok: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        for candidate in 1..=2u64 {
            actions.push(Frame::ReqVote {
                candidate,
                persist_ok: true,
            });
            actions.push(Frame::ReqVote {
                candidate,
                persist_ok: false,
            });
        }
        let last = state.last_index();
        for index in 1..=(last + 1).min(3) {
            for term in 1..=2u64 {
                actions.push(Frame::AppendEntries { index, term });
            }
        }
        if last > 0 {
            actions.push(Frame::Propose { index: last });
            actions.push(Frame::Propose { index: last + 1 });
        }
        actions.push(Frame::ApplyTick);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        // One loop step: read (action) → dispatch → persist → send (reply
        // is computed inside the step; only its invariant witness survives).
        self.dispatch(&mut next, action);
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-vote-once", |_, s: &NodeSt| !s.double_vote),
            Property::always("Inv-F16-no-rewrite-committed", |_, s: &NodeSt| {
                !s.rewrote_committed
            }),
            Property::always("Inv-F11-no-dirty-ack", |_, s: &NodeSt| {
                // Witnessed at send time: every RespOk left the node with
                // commit already covering the index.
                !s.acked_uncommitted
            }),
            Property::always("Inv-send-after-persist", |_, s: &NodeSt| {
                // Witnessed at send time: a grant reply implies the vote
                // persisted Ok first.
                !s.granted_without_persist
            }),
            Property::always("Inv-apply-contiguous", |_, s: &NodeSt| {
                s.applied.len() as u64 == s.last_applied
            }),
            Property::always("Inv-applied-le-commit", |_, s: &NodeSt| {
                s.last_applied <= s.commit
            }),
            Property::sometimes("non-vacuity-grant-reply", |_, s: &NodeSt| {
                s.saw_grant_reply
            }),
            Property::sometimes("non-vacuity-ack-reply", |_, s: &NodeSt| s.saw_ack_ok),
        ]
    }
}

#[test]
fn fixed_tcp_node_invariants() {
    let checker = TcpNodeModel {
        mutant: LoopMutant::Fixed,
    }
    .checker()
    .spawn_bfs()
    .join();
    checker.assert_properties();
}

#[test]
fn as_is_send_grant_before_persist_is_caught() {
    let checker = TcpNodeModel {
        mutant: LoopMutant::SendGrantBeforePersist,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-send-after-persist").is_some(),
        "send-before-persist must counterexample Inv-send-after-persist"
    );
}

#[test]
fn as_is_dirty_ack_is_caught() {
    let checker = TcpNodeModel {
        mutant: LoopMutant::DirtyAckLoop,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-F11-no-dirty-ack").is_some(),
        "dirty-ack loop must counterexample Inv-F11-no-dirty-ack"
    );
}

#[test]
fn as_is_skip_ae_guard_is_caught() {
    let checker = TcpNodeModel {
        mutant: LoopMutant::SkipAeGuard,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Inv-F16-no-rewrite-committed").is_some(),
        "skip-AE-guard must counterexample Inv-F16"
    );
}

// ===========================================================================
// RFC-0056 P2.2: liveness under eventual synchrony (named axioms ES-1..ES-3).
// ===========================================================================

/// Adversarial scheduling budget: AXIOM ES-1 says it is finite.
const ES_BOUND: u32 = 2;

/// Behavior length bound. Every maximal behavior includes at least
/// `MAX_STEPS - ES_BOUND` synchronized steps, so the eventuality claims are
/// checked non-vacuously at the terminal states of every path.
const MAX_STEPS: u32 = ES_BOUND + 2;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct LiveSt {
    st: NodeSt,
    /// Remaining adversarial steps. Frozen (never consumed) in the
    /// unrestricted model so its state space stays finite.
    budget: u32,
    /// Remaining steps; `actions` is empty at 0, making every behavior
    /// finite (path-acyclic) — the precondition for stateright's BFS
    /// eventuality check.
    steps: u32,
}

#[derive(Clone)]
struct LivenessModel {
    /// No axioms: full frame set forever, apply is env-scheduled, no drain.
    /// Must refute both eventual properties (the axioms are necessary).
    unrestricted: bool,
    /// AXIOM ES-3 on/off.
    live_retry: bool,
    /// Loop-level liveness mutant: the node never drains (AXIOM ES-2 broken
    /// in the implementation, not the environment).
    broken_drain: bool,
}

impl LivenessModel {
    fn fixed() -> Self {
        Self {
            unrestricted: false,
            live_retry: true,
            broken_drain: false,
        }
    }

    fn node(&self) -> TcpNodeModel {
        TcpNodeModel {
            mutant: LoopMutant::Fixed,
        }
    }

    fn init_node_st() -> NodeSt {
        NodeSt {
            voted_for: None,
            terms: vec![1],
            commit: 1,
            last_applied: 0,
            applied: Vec::new(),
            double_vote: false,
            rewrote_committed: false,
            acked_uncommitted: false,
            granted_without_persist: false,
            saw_grant_reply: false,
            saw_ack_ok: false,
        }
    }
}

impl Model for LivenessModel {
    type State = LiveSt;
    type Action = Frame;

    fn init_states(&self) -> Vec<Self::State> {
        vec![LiveSt {
            st: Self::init_node_st(),
            budget: ES_BOUND,
            steps: MAX_STEPS,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.steps == 0 {
            // Terminal: the behavior ends here, and stateright checks at
            // this point that every eventuality already held along the path.
            return;
        }
        let s = &state.st;
        if self.unrestricted || state.budget > 0 {
            // Adversarial regime: any frame, persist may fail, apply is
            // env-scheduled (the pre-axiom world).
            for candidate in 1..=2u64 {
                actions.push(Frame::ReqVote {
                    candidate,
                    persist_ok: true,
                });
                actions.push(Frame::ReqVote {
                    candidate,
                    persist_ok: false,
                });
            }
            let last = s.last_index();
            for index in 1..=(last + 1).min(3) {
                for term in 1..=2u64 {
                    actions.push(Frame::AppendEntries { index, term });
                }
            }
            if last > 0 {
                actions.push(Frame::Propose { index: last });
                actions.push(Frame::Propose { index: last + 1 });
            }
            actions.push(Frame::ApplyTick);
            return;
        }
        // Synchronized regime (AXIOM ES-1 exhausted): persist never fails,
        // apply is internal (drain after the frame, no ApplyTick frame).
        if self.live_retry && !s.saw_grant_reply {
            // AXIOM ES-3: a live candidate keeps retrying until granted.
            for candidate in 1..=2u64 {
                actions.push(Frame::ReqVote {
                    candidate,
                    persist_ok: true,
                });
            }
            return;
        }
        for candidate in 1..=2u64 {
            actions.push(Frame::ReqVote {
                candidate,
                persist_ok: true,
            });
        }
        let last = s.last_index();
        for index in 1..=(last + 1).min(3) {
            for term in 1..=2u64 {
                actions.push(Frame::AppendEntries { index, term });
            }
        }
        if last > 0 {
            actions.push(Frame::Propose { index: last });
            actions.push(Frame::Propose { index: last + 1 });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let node = self.node();
        let mut next = LiveSt {
            st: state.st.clone(),
            budget: state.budget,
            steps: state.steps,
        };
        node.dispatch(&mut next.st, action);
        next.steps -= 1;
        if !self.unrestricted {
            if next.budget > 0 {
                next.budget -= 1;
            }
            if next.budget == 0 && !self.broken_drain {
                // AXIOM ES-2: drain to quiescence before the next frame.
                node.drain_apply(&mut next.st);
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            // Safety survives the axioms unchanged (axioms buy liveness,
            // never safety).
            Property::always("Inv-vote-once", |_, s: &LiveSt| !s.st.double_vote),
            Property::always("Inv-F16-no-rewrite-committed", |_, s: &LiveSt| {
                !s.st.rewrote_committed
            }),
            Property::always("Inv-F11-no-dirty-ack", |_, s: &LiveSt| {
                !s.st.acked_uncommitted
            }),
            Property::always("Inv-send-after-persist", |_, s: &LiveSt| {
                !s.st.granted_without_persist
            }),
            Property::always("Inv-apply-contiguous", |_, s: &LiveSt| {
                s.st.applied.len() as u64 == s.st.last_applied
            }),
            Property::always("Inv-applied-le-commit", |_, s: &LiveSt| {
                s.st.last_applied <= s.st.commit
            }),
            // Liveness: every behavior eventually applies all committed
            // entries, and every behavior eventually answers a vote.
            Property::eventually("Evt-apply-quiescence", |_, s: &LiveSt| {
                s.st.last_applied == s.st.commit && s.st.commit >= 1
            }),
            Property::eventually("Evt-election", |_, s: &LiveSt| s.st.saw_grant_reply),
        ]
    }
}

#[test]
fn liveness_holds_under_eventual_synchrony_axioms() {
    let checker = LivenessModel::fixed()
        .checker()
        .spawn_bfs()
        .join();
    checker.assert_properties();
}

#[test]
fn liveness_is_refuted_without_axioms() {
    // The unrestricted model (no ES-1/ES-2/ES-3): the environment may
    // persist-fail forever and never schedule ApplyTick or ReqVote — some
    // maximal behavior ends without the property ever holding, refuting
    // BOTH eventual properties. The axioms are necessary, never theorems.
    let checker = LivenessModel {
        unrestricted: true,
        live_retry: true,
        broken_drain: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Evt-apply-quiescence").is_some(),
        "unrestricted model must counterexample Evt-apply-quiescence"
    );
    assert!(
        checker.discovery("Evt-election").is_some(),
        "unrestricted model must counterexample Evt-election"
    );
}

#[test]
fn liveness_election_needs_live_retry_axiom() {
    // ES-1 + ES-2 on, ES-3 OFF: the synchronized environment may avoid
    // ReqVote forever, so election is refuted while apply still drains.
    let checker = LivenessModel {
        unrestricted: false,
        live_retry: false,
        broken_drain: false,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Evt-election").is_some(),
        "without ES-3 the election eventuality must be refuted"
    );
    assert!(
        checker.discovery("Evt-apply-quiescence").is_none(),
        "ES-1 + ES-2 alone must still guarantee apply quiescence"
    );
}

#[test]
fn liveness_mutant_broken_drain_is_caught() {
    // All axioms ON but the loop never drains (implementation bug): the
    // liveness property must fail even under eventual synchrony.
    let checker = LivenessModel {
        unrestricted: false,
        live_retry: true,
        broken_drain: true,
    }
    .checker()
    .spawn_bfs()
    .join();
    assert!(
        checker.discovery("Evt-apply-quiescence").is_some(),
        "broken-drain loop must counterexample Evt-apply-quiescence"
    );
}
