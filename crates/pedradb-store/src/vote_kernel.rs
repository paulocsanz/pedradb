//! Pure RequestVote decision (RFC-0152 P0 / F15).
//!
//! Same rules as `pedradb-raft::vote_kernel`. Store does not depend on
//! `pedradb-raft`; keep the two bodies identical (drift-trap: harness grid).

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

/// Whether the follower may still vote for `candidate_id` in this term.
#[must_use]
pub fn can_vote(voted_for: Option<u64>, candidate_id: u64) -> bool {
    // Match, not `Option ==`: Aeneas has no model of `PartialEq<Option<u64>>`
    // (extract would axiom it). `u64 == u64` is in the Lean std.
    match voted_for {
        None => true,
        Some(v) => v == candidate_id,
    }
}

/// Raft §5.4.1 log up-to-date (candidate at least as new as local).
#[must_use]
pub fn log_up_to_date(
    my_last_term: u64,
    my_last_index: u64,
    cand_last_term: u64,
    cand_last_index: u64,
) -> bool {
    cand_last_term > my_last_term
        || (cand_last_term == my_last_term && cand_last_index >= my_last_index)
}

/// Pure RequestVote decision.
#[must_use]
pub fn vote_decision(i: VoteInputs) -> VoteDecision {
    if i.candidate_term != i.current_term {
        return VoteDecision::Deny;
    }
    if can_vote(i.voted_for, i.candidate_id)
        && log_up_to_date(
            i.last_log_term,
            i.last_log_index,
            i.candidate_last_log_term,
            i.candidate_last_log_index,
        )
    {
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

/// Persist result the handler sees (axiom of the environment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistOutcome {
    /// `persist_hard` returned Ok.
    Ok,
    /// `persist_hard` returned Err.
    Err,
}

/// F15 protocol: Grant on the wire only if the kernel would grant **and** persist Ok.
///
/// This is the refinement of `handle_request_vote_with_persist` minus I/O.
/// `sent_grant ⇒ persist == Ok`.
#[must_use]
pub fn grant_after_persist(decision: VoteDecision, persist: PersistOutcome) -> bool {
    // Match, not `==` on enums: derived PartialEq extracts to discriminant
    // `Result` wrappers that Lean cannot `cases` through.
    matches!(
        (decision, persist),
        (VoteDecision::WouldGrant, PersistOutcome::Ok)
    )
}

/// AS-IS F15: grant as soon as the kernel says so, persist is ignored.
#[must_use]
pub fn grant_after_persist_as_is(decision: VoteDecision, persist: PersistOutcome) -> bool {
    let _ = persist;
    decision == VoteDecision::WouldGrant
}

/// F125/F127 outcome of stepping to a newer term under a persist result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableTerm {
    /// Incoming term is not newer — no step.
    Keep,
    /// Newer term and hard state durable: follow at the new term.
    Raised,
    /// Newer term but persist failed: restore term/vote, force Follower,
    /// clear `leader_id` — never act at a term that is not on disk.
    Restored,
}

/// F125/F127: the term rises only when hard state is durable.
/// `Raised ⇒ persist == Ok`; `Restored ⇒ previous term/vote survive`.
#[must_use]
pub fn durable_term_if_newer(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> DurableTerm {
    // Match, not `==` on enums: derived PartialEq extracts to discriminant
    // `Result` wrappers that Lean cannot `cases` through.
    match (incoming_term > current_term, persist) {
        (false, _) => DurableTerm::Keep,
        (true, PersistOutcome::Ok) => DurableTerm::Raised,
        (true, PersistOutcome::Err) => DurableTerm::Restored,
    }
}

/// AS-IS F125/F127 mutant: keep the raised term even when persist failed —
/// the process acts at a term that never hit disk. Used to prove teeth.
#[must_use]
pub fn durable_term_if_newer_as_is(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> DurableTerm {
    let _ = persist;
    if incoming_term > current_term {
        DurableTerm::Raised
    } else {
        DurableTerm::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale_log() -> VoteInputs {
        VoteInputs {
            current_term: 5,
            voted_for: None,
            last_log_term: 3,
            last_log_index: 10,
            candidate_term: 5,
            candidate_id: 1,
            candidate_last_log_term: 1,
            candidate_last_log_index: 1,
        }
    }

    #[test]
    fn as_is_mutant_differs_on_stale_log() {
        assert_eq!(vote_decision(stale_log()), VoteDecision::Deny);
        assert_eq!(
            vote_decision_as_is_ignore_log_and_vote(stale_log()),
            VoteDecision::WouldGrant
        );
    }

    #[test]
    fn grant_after_persist_implies_ok() {
        assert!(grant_after_persist(
            VoteDecision::WouldGrant,
            PersistOutcome::Ok
        ));
        assert!(!grant_after_persist(
            VoteDecision::WouldGrant,
            PersistOutcome::Err
        ));
        assert!(!grant_after_persist(VoteDecision::Deny, PersistOutcome::Ok));
        assert!(grant_after_persist_as_is(
            VoteDecision::WouldGrant,
            PersistOutcome::Err
        ));
    }

    #[test]
    fn durable_term_keeps_on_stale_or_equal_term() {
        assert_eq!(
            durable_term_if_newer(5, 4, PersistOutcome::Ok),
            DurableTerm::Keep
        );
        assert_eq!(
            durable_term_if_newer(5, 5, PersistOutcome::Err),
            DurableTerm::Keep
        );
    }

    #[test]
    fn durable_term_raises_only_on_newer_and_ok() {
        assert_eq!(
            durable_term_if_newer(5, 6, PersistOutcome::Ok),
            DurableTerm::Raised
        );
    }

    #[test]
    fn durable_term_restores_on_newer_and_err() {
        assert_eq!(
            durable_term_if_newer(5, 6, PersistOutcome::Err),
            DurableTerm::Restored
        );
    }

    /// Mutation: the undurable raise must differ from the fixed rule (teeth).
    #[test]
    fn durable_term_as_is_mutant_keeps_undurable_raise() {
        assert_eq!(
            durable_term_if_newer_as_is(5, 6, PersistOutcome::Err),
            DurableTerm::Raised,
            "mutant keeps a raised term that never hit disk (teeth)"
        );
        assert_eq!(
            durable_term_if_newer(5, 6, PersistOutcome::Err),
            DurableTerm::Restored
        );
    }
}
