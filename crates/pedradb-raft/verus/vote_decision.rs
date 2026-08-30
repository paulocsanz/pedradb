// Verus proof of the pure RequestVote decision (RFC-0002 P1.4 / F15).
//
// Source of truth for production remains `src/vote_kernel.rs` (same rules).
// This file is the machine-checked theorem: the closed-form iff.
//
// Build/verify (requires verus on PATH, e.g. ~/.local/verus/verus-arm64-macos):
//   verus vote_decision.rs --crate-type=lib --multiple-errors 10
//
// Or: ../../../scripts/verus_vote_decision.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel
// kept for proof, with a CI script that diffs the match arms against vote_kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `VoteDecision` in vote_kernel.rs.
pub enum VoteDecision {
    WouldGrant,
    Deny,
}

/// Mirrors `can_vote`.
pub open spec fn can_vote(voted_for: Option<u64>, candidate_id: u64) -> bool {
    match voted_for {
        None => true,
        Some(v) => v == candidate_id,
    }
}

/// Mirrors `log_up_to_date` (Raft §5.4.1).
pub open spec fn log_up_to_date(
    my_last_term: u64,
    my_last_index: u64,
    cand_last_term: u64,
    cand_last_index: u64,
) -> bool {
    cand_last_term > my_last_term
        || (cand_last_term == my_last_term && cand_last_index >= my_last_index)
}

/// Spec of the decision (closed form).
pub open spec fn should_grant(
    current_term: u64,
    voted_for: Option<u64>,
    last_log_term: u64,
    last_log_index: u64,
    candidate_term: u64,
    candidate_id: u64,
    candidate_last_log_term: u64,
    candidate_last_log_index: u64,
) -> bool {
    candidate_term == current_term
        && can_vote(voted_for, candidate_id)
        && log_up_to_date(
            last_log_term,
            last_log_index,
            candidate_last_log_term,
            candidate_last_log_index,
        )
}

/// Executable decision — must match `pedradb_raft::vote_decision` bit-for-bit.
pub fn vote_decision(
    current_term: u64,
    voted_for: Option<u64>,
    last_log_term: u64,
    last_log_index: u64,
    candidate_term: u64,
    candidate_id: u64,
    candidate_last_log_term: u64,
    candidate_last_log_index: u64,
) -> (d: VoteDecision)
    ensures
        (d == VoteDecision::WouldGrant) == should_grant(
            current_term,
            voted_for,
            last_log_term,
            last_log_index,
            candidate_term,
            candidate_id,
            candidate_last_log_term,
            candidate_last_log_index,
        ),
{
    if candidate_term != current_term {
        return VoteDecision::Deny;
    }
    let can = match voted_for {
        None => true,
        Some(v) => v == candidate_id,
    };
    let up = candidate_last_log_term > last_log_term
        || (candidate_last_log_term == last_log_term
            && candidate_last_log_index >= last_log_index);
    if can && up {
        VoteDecision::WouldGrant
    } else {
        VoteDecision::Deny
    }
}

/// Mirrors `PersistOutcome` in vote_kernel.rs.
pub enum PersistOutcome {
    Ok,
    Err,
}

/// F15: wire grant only if the kernel would grant **and** persist Ok.
pub open spec fn grant_after_persist_spec(d: VoteDecision, p: PersistOutcome) -> bool {
    match (d, p) {
        (VoteDecision::WouldGrant, PersistOutcome::Ok) => true,
        _ => false,
    }
}

/// Executable protocol bit — production `handle_request_vote_with_persist` calls this.
///
/// `ensures g ==> persist Ok` is the caller refinement (sent_grant ⇒ disk Ok).
pub fn grant_after_persist(decision: VoteDecision, persist: PersistOutcome) -> (g: bool)
    ensures
        g == grant_after_persist_spec(decision, persist),
        g ==> persist == PersistOutcome::Ok,
{
    match (decision, persist) {
        (VoteDecision::WouldGrant, PersistOutcome::Ok) => true,
        (VoteDecision::WouldGrant, PersistOutcome::Err) => false,
        (VoteDecision::Deny, PersistOutcome::Ok) => false,
        (VoteDecision::Deny, PersistOutcome::Err) => false,
    }
}

/// AS-IS F15: ignore persist (teeth: grants on Err).
pub fn grant_after_persist_as_is(decision: VoteDecision, persist: PersistOutcome) -> (g: bool)
    ensures
        g == (decision == VoteDecision::WouldGrant),
{
    let _ = persist;
    match decision {
        VoteDecision::WouldGrant => true,
        VoteDecision::Deny => false,
    }
}

} // verus!
