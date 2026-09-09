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
//! Spec page: `determinismo/pedradb-dst/specs/f15-vote-decision.md`.

#![forbid(unsafe_code)]

//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_vote.sh`). A flattened stand-in view of `VoteInputs`
//! is a model twin — not last-wins (deleted).
//!
//!   ./scripts/aeneas_vote.sh --required


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
///
/// # Post-condition (theorem statement — RFC-0002 P1.4)
///
/// ```text
/// ensures
///   (vote_decision(i) == WouldGrant) <==>
///     i.candidate_term == i.current_term
///     && can_vote(i.voted_for, i.candidate_id)
///     && log_up_to_date(i.last_log_term, i.last_log_index,
///                       i.candidate_last_log_term, i.candidate_last_log_index)
/// ```
///
/// Machine-checked:
/// - finite universe: [`tests::theorem_vote_decision_iff_on_finite_domain`]
/// - Aeneas of rustc `vote_decision(VoteInputs)` (`scripts/aeneas_vote.sh`)
///
/// See `determinismo/pedradb-dst/formal/P1.4-vote-theorem.md`.
///
/// # Does not cover
///
/// Durability of the vote, network delivery, or step-down — caller + axioms (F15).
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

/// Spec predicate: outcome matches the closed-form rule (bidirectional).
#[must_use]
pub fn vote_decision_spec(i: VoteInputs, d: VoteDecision) -> bool {
    let grant = i.candidate_term == i.current_term
        && can_vote(i.voted_for, i.candidate_id)
        && log_up_to_date(
            i.last_log_term,
            i.last_log_index,
            i.candidate_last_log_term,
            i.candidate_last_log_index,
        );
    match d {
        VoteDecision::WouldGrant => grant,
        VoteDecision::Deny => !grant,
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
    fn vote_kernel_has_no_verus_cartoon() {
        let src = include_str!("vote_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "flattened stand-in is not last-wins of rustc VoteInputs"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
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

    /// P1.4 finite-domain theorem: ∀ inputs in U, `vote_decision` ⇔ closed-form spec.
    ///
    /// Universe sized for CI (~few hundred ms): terms/ids/logs in `{0..3}`.
    /// This is not ∀u64; it is a machine-checked proof on U and a regression
    /// that the implementation matches the `ensures` statement above.
    #[test]
    fn theorem_vote_decision_iff_on_finite_domain() {
        const B: u64 = 4; // domain 0..B for each numeric field
        let mut n = 0u64;
        for current_term in 0..B {
            for candidate_term in 0..B {
                for candidate_id in 0..B {
                    for last_log_term in 0..B {
                        for last_log_index in 0..B {
                            for cand_lt in 0..B {
                                for cand_li in 0..B {
                                    // voted_for: None or Some(v) for v in 0..B
                                    for vf in 0..=B {
                                        let voted_for = if vf == B { None } else { Some(vf) };
                                        let i = VoteInputs {
                                            current_term,
                                            voted_for,
                                            last_log_term,
                                            last_log_index,
                                            candidate_term,
                                            candidate_id,
                                            candidate_last_log_term: cand_lt,
                                            candidate_last_log_index: cand_li,
                                        };
                                        let d = vote_decision(i);
                                        assert!(
                                            vote_decision_spec(i, d),
                                            "spec broken at {i:?} → {d:?}"
                                        );
                                        n += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // B^7 * (B+1) = 4^7 * 5 = 81920
        assert_eq!(n, B.pow(7) * (B + 1));
    }
}
