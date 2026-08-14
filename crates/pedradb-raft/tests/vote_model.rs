//! Stateright model over the **real** [`pedradb_raft::vote_decision`] kernel (RFC-0002 P1.3).
//!
//! Faithfulness: every vote outcome is produced by production `vote_kernel`, not a paraphrase.
//! The model only owns multi-node structure (who asks whom) and the **persist axiom**
//! (WouldGrant ⇒ voted_for durable in this state — we model successful persist).
//!
//! Properties:
//! - **Inv-vote-once:** in a fixed term, a node never grants two different candidates
//!   (FIXED kernel). AS-IS mutant can violate.
//! - Non-vacuity: at least one grant is reachable under FIXED.

use pedradb_raft::{vote_decision, vote_kernel, VoteDecision, VoteInputs};
use stateright::{Checker, Model, Property};

const N: usize = 3;
const TERM: u64 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
struct Node {
    /// Last granted candidate in TERM, if any (models durable voted_for after Ok persist).
    voted_for: Option<u64>,
    last_log_term: u64,
    last_log_index: u64,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct St {
    nodes: [Node; N],
    /// True if some node granted two distinct candidates (Inv broken latch).
    double_vote: bool,
    /// Witness: at least one WouldGrant observed.
    saw_grant: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    /// Candidate `c` (1..=N) solicits vote from follower `f` (0..N).
    RequestVote { candidate: u64, follower: usize },
}

#[derive(Clone)]
struct VoteModel {
    /// When false, use AS-IS mutant that ignores log + existing vote (F15 teeth).
    fixed: bool,
}

impl VoteModel {
    fn decide(&self, i: VoteInputs) -> VoteDecision {
        if self.fixed {
            vote_decision(i)
        } else {
            vote_kernel::vote_decision_as_is_ignore_log_and_vote(i)
        }
    }
}

impl Model for VoteModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        // Small log diversity so up_to_date can deny.
        let mut nodes = [Node::default(); N];
        for (i, n) in nodes.iter_mut().enumerate() {
            n.last_log_term = 1;
            n.last_log_index = i as u64; // 0,1,2
        }
        vec![St {
            nodes,
            double_vote: false,
            saw_grant: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        for follower in 0..N {
            for candidate in 1..=(N as u64) {
                actions.push(Act::RequestVote {
                    candidate,
                    follower,
                });
            }
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let Act::RequestVote {
            candidate,
            follower,
        } = action;
        let mut next = state.clone();
        let f = &mut next.nodes[follower];
        // Candidate log = that node's own log (each node may candidacy with its log).
        let cand_node = state.nodes[(candidate as usize - 1).min(N - 1)];
        let inputs = VoteInputs {
            current_term: TERM,
            voted_for: f.voted_for,
            last_log_term: f.last_log_term,
            last_log_index: f.last_log_index,
            candidate_term: TERM,
            candidate_id: candidate,
            candidate_last_log_term: cand_node.last_log_term,
            candidate_last_log_index: cand_node.last_log_index,
        };
        match self.decide(inputs) {
            VoteDecision::Deny => {}
            VoteDecision::WouldGrant => {
                // Axiom: persist succeeds → durable vote (protocol F15 modeled as atomic).
                if let Some(prev) = f.voted_for {
                    if prev != candidate {
                        next.double_vote = true;
                    }
                }
                f.voted_for = Some(candidate);
                next.saw_grant = true;
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-vote-once", inv_vote_once),
            Property::sometimes("non-vacuity-grant", non_vacuity_grant),
        ]
    }
}

fn inv_vote_once(_: &VoteModel, s: &St) -> bool {
    !s.double_vote
}

fn non_vacuity_grant(_: &VoteModel, s: &St) -> bool {
    s.saw_grant
}

#[test]
fn fixed_vote_decision_no_double_vote() {
    let checker = VoteModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_mutant_can_double_vote() {
    // Mutant ignores existing vote → double_vote latch reachable.
    let checker = VoteModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-vote-once").is_some(),
        "AS-IS mutant must produce a counterexample for Inv-vote-once (teeth)"
    );
}
