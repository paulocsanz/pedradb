// Verus proof of the durable-term step (RFC-0158 P0.2 / F125/F127).
//
// Source of truth for production remains `src/vote_kernel.rs` (same rules,
// cloned into pedradb-store; the catalog clone `vote_raft_store` pins token
// equality). This file is the machine-checked theorem: the closed-form
// decision, plus the safety lemma — the surviving term rises above the
// previous one only when the hard state was durable.
//
// Build/verify (requires verus, e.g. ~/.local/verus/verus-arm64-macos):
//   verus durable_term.rs --crate-type=lib --multiple-errors 10
//
// Or: ../../../scripts/verus_durable_term.sh
//
// Do not link this into the production crate — it is a twin of the pure
// kernel kept for proof, with a CI script that diffs the match arms
// against vote_kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `PersistOutcome` in vote_kernel.rs (axiom of the environment).
pub enum PersistOutcome {
    Ok,
    Err,
}

/// Mirrors `DurableTerm` in vote_kernel.rs.
pub enum DurableTerm {
    Keep,
    Raised,
    Restored,
}

/// Spec of the durable-term step (closed form).
pub open spec fn durable_term_spec(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> DurableTerm {
    if incoming_term <= current_term {
        DurableTerm::Keep
    } else if persist == PersistOutcome::Ok {
        DurableTerm::Raised
    } else {
        DurableTerm::Restored
    }
}

/// Exec twin of `vote_kernel::durable_term_if_newer` (same match shape).
pub fn durable_term_if_newer(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> (out: DurableTerm)
    ensures
        out == durable_term_spec(current_term, incoming_term, persist),
{
    match (incoming_term > current_term, persist) {
        (false, _) => DurableTerm::Keep,
        (true, PersistOutcome::Ok) => DurableTerm::Raised,
        (true, PersistOutcome::Err) => DurableTerm::Restored,
    }
}

/// The term the process may act at after the step (spec).
pub open spec fn surviving_term_spec(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> u64 {
    match durable_term_spec(current_term, incoming_term, persist) {
        DurableTerm::Keep => current_term,
        DurableTerm::Raised => incoming_term,
        DurableTerm::Restored => current_term,
    }
}

/// Exec form of `surviving_term_spec`.
pub fn surviving_term(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
) -> (t: u64)
    ensures
        t == surviving_term_spec(current_term, incoming_term, persist),
{
    match durable_term_if_newer(current_term, incoming_term, persist) {
        DurableTerm::Keep => current_term,
        DurableTerm::Raised => incoming_term,
        DurableTerm::Restored => current_term,
    }
}

/// Safety (F125/F127): the surviving term rises above the previous one only
/// when the hard state was durable. The undurable raise is unreachable.
proof fn term_rises_only_when_durable(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
)
    ensures
        surviving_term_spec(current_term, incoming_term, persist) > current_term
            ==> persist == PersistOutcome::Ok,
{}

/// Safety: a Restored step keeps the previous term — the process never acts
/// at a term that did not hit disk.
proof fn restored_keeps_previous_term(
    current_term: u64,
    incoming_term: u64,
    persist: PersistOutcome,
)
    ensures
        durable_term_spec(current_term, incoming_term, persist) == DurableTerm::Restored
            ==> surviving_term_spec(current_term, incoming_term, persist) == current_term,
{}

} // verus!
