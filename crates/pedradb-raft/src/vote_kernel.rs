//! Pure RequestVote decision (Beyond-style kernel).
//!
//! # Contract
//!
//! - **No I/O, no clock, no RNG.** All facts are arguments.
//! - Production [`crate::rpc_request_vote`] / `handle_request_vote` calls
//!   [`vote_decision`] then applies the **persist-before-grant** protocol (F15).
//! - A Stateright / Verus model must call **this same function**, not a paraphrase.
//!
//! # Decision vs protocol
//!
//! | Piece | Where |
//! |-------|--------|
//! | `can_vote ∧ log_up_to_date` | this kernel → [`VoteDecision::WouldGrant`] / [`Deny`] |
//! | Grant only after `persist_hard` Ok | caller (`handle_request_vote`) |
//! | Persist may fail / lie | **axiom** — World / FailingEnv / det_io; never a theorem fact |
//!
//! Spec page: `determinismo/pedradb-dst/formal/F15-vote-decision.md`.

#![forbid(unsafe_code)]

/// Inputs for a RequestVote decision after the follower has already adopted
/// `args.term` into hard state when `args.term > current_term` (caller-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteInputs {
    /// Follower current term (post step-down if candidate term was higher).
    pub current_term: u64,
    /// Follower `voted_for` in this term (`None` if free or just stepped down).
    pub voted_for: Option<u64>,
    /// Follower last log term (0 if empty).
    pub last_log_term: u64,
    /// Follower last log index (0 if empty).
    pub last_log_index: u64,
    /// Candidate term.
    pub candidate_term: u64,
    /// Candidate id.
    pub candidate_id: u64,
    /// Candidate last log term.
    pub candidate_last_log_term: u64,
    /// Candidate last log index.
    pub candidate_last_log_index: u64,
}

/// Outcome of the pure vote rule (Raft §5.2 / §5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteDecision {
    /// Caller may persist `voted_for = candidate` and, **only if persist Ok**, set `vote_granted`.
    WouldGrant,
    /// Do not grant (stale term, already voted elsewhere, or candidate log not up-to-date).
    Deny,
}

/// Pure RequestVote decision.
///
/// # Invariant (post-condition of the rule)
///
/// - `WouldGrant` ⇒ `candidate_term == current_term`
///   ∧ (`voted_for` is `None` ∨ `voted_for == Some(candidate_id)`)
///   ∧ candidate log is at least as up-to-date as local log.
/// - `Deny` ⇒ not all of the above.
///
/// # Does not cover
///
/// Durability of the vote, network delivery, or step-down side effects — those are
/// the caller's protocol + environment axioms (F15).
#[must_use]
pub fn vote_decision(i: VoteInputs) -> VoteDecision {
    if i.candidate_term != i.current_term {
        return VoteDecision::Deny;
    }
    let can_vote = i.voted_for.is_none() || i.voted_for == Some(i.candidate_id);
    let up_to_date = i.candidate_last_log_term > i.last_log_term
        || (i.candidate_last_log_term == i.last_log_term
            && i.candidate_last_log_index >= i.last_log_index);
    if can_vote && up_to_date {
        VoteDecision::WouldGrant
    } else {
        VoteDecision::Deny
    }
}

/// AS-IS / mutant: grant whenever the term matches, **ignoring** log up-to-date
/// and existing vote. Used only to prove the model/kernel invariant has teeth.
#[must_use]
pub fn vote_decision_as_is_ignore_log_and_vote(i: VoteInputs) -> VoteDecision {
    if i.candidate_term == i.current_term {
        VoteDecision::WouldGrant
    } else {
        VoteDecision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> VoteInputs {
        VoteInputs {
            current_term: 5,
            voted_for: None,
            last_log_term: 3,
            last_log_index: 10,
            candidate_term: 5,
            candidate_id: 1,
            candidate_last_log_term: 3,
            candidate_last_log_index: 10,
        }
    }

    #[test]
    fn grants_when_free_and_log_ok() {
        assert_eq!(vote_decision(base()), VoteDecision::WouldGrant);
    }

    #[test]
    fn denies_stale_term() {
        let mut i = base();
        i.candidate_term = 4;
        assert_eq!(vote_decision(i), VoteDecision::Deny);
    }

    #[test]
    fn denies_already_voted_other() {
        let mut i = base();
        i.voted_for = Some(2);
        assert_eq!(vote_decision(i), VoteDecision::Deny);
    }

    #[test]
    fn grants_same_candidate_again() {
        let mut i = base();
        i.voted_for = Some(1);
        assert_eq!(vote_decision(i), VoteDecision::WouldGrant);
    }

    #[test]
    fn denies_stale_candidate_log_term() {
        let mut i = base();
        i.candidate_last_log_term = 2;
        assert_eq!(vote_decision(i), VoteDecision::Deny);
    }

    #[test]
    fn denies_shorter_log_same_term() {
        let mut i = base();
        i.candidate_last_log_index = 9;
        assert_eq!(vote_decision(i), VoteDecision::Deny);
    }

    #[test]
    fn grants_longer_log_same_term() {
        let mut i = base();
        i.candidate_last_log_index = 11;
        assert_eq!(vote_decision(i), VoteDecision::WouldGrant);
    }

    /// Mutation: broken rule must differ from fixed on at least one input
    /// (model-check teeth / non-vacuous invariant).
    #[test]
    fn as_is_mutant_differs_on_stale_log() {
        let mut i = base();
        i.candidate_last_log_term = 1;
        assert_eq!(vote_decision(i), VoteDecision::Deny);
        assert_eq!(
            vote_decision_as_is_ignore_log_and_vote(i),
            VoteDecision::WouldGrant,
            "mutant must WouldGrant where fixed Denies (teeth)"
        );
    }

    #[test]
    fn as_is_mutant_differs_on_double_vote() {
        let mut i = base();
        i.voted_for = Some(9);
        assert_eq!(vote_decision(i), VoteDecision::Deny);
        assert_eq!(
            vote_decision_as_is_ignore_log_and_vote(i),
            VoteDecision::WouldGrant
        );
    }
}
