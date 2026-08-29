// Verus proof of commit recover + may_commit_at (RFC-0002 P7 / F10 / F23).
//
// Twin of `src/commit_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_commit_recover.sh

use vstd::prelude::*;

verus! {

pub open spec fn recover_commit_spec(loaded: u64, log_last: u64) -> u64 {
    if loaded <= log_last {
        loaded
    } else {
        log_last
    }
}

pub fn recover_commit(loaded_commit: u64, log_last: u64) -> (c: u64)
    ensures
        c == recover_commit_spec(loaded_commit, log_last),
        c <= loaded_commit,
        c <= log_last,
{
    if loaded_commit <= log_last {
        loaded_commit
    } else {
        log_last
    }
}

pub fn recover_last_applied() -> (a: u64)
    ensures
        a == 0,
{
    0
}

pub open spec fn recover_commit_as_is(_loaded: u64, log_last: u64) -> u64 {
    log_last
}

/// F10 teeth: AS-IS can promote an uncommitted suffix.
proof fn lemma_as_is_promotes_suffix(loaded: u64, log_last: u64)
    requires
        loaded < log_last,
    ensures
        recover_commit_spec(loaded, log_last) == loaded,
        recover_commit_as_is(loaded, log_last) == log_last,
        recover_commit_as_is(loaded, log_last) > recover_commit_spec(loaded, log_last),
{
}

pub open spec fn may_commit_at_spec(index_term: u64, current_term: u64, has_majority: bool) -> bool {
    has_majority && index_term == current_term
}

pub fn may_commit_at(index_term: u64, current_term: u64, has_majority: bool) -> (d: bool)
    ensures
        d == may_commit_at_spec(index_term, current_term, has_majority),
        d ==> has_majority && index_term == current_term,
{
    has_majority && index_term == current_term
}

pub open spec fn may_commit_at_as_is(_index_term: u64, _current_term: u64, has_majority: bool) -> bool {
    has_majority
}

proof fn lemma_as_is_commits_prev_term(index_term: u64, current_term: u64)
    requires
        index_term != current_term,
    ensures
        !may_commit_at_spec(index_term, current_term, true),
        may_commit_at_as_is(index_term, current_term, true),
{
}

pub open spec fn propose_ack_ok_spec(index: u64, commit_index: u64) -> bool {
    commit_index >= index
}

pub fn propose_ack_ok(index: u64, commit_index: u64) -> (d: bool)
    ensures
        d == propose_ack_ok_spec(index, commit_index),
        d ==> commit_index >= index,
{
    commit_index >= index
}

pub open spec fn propose_ack_ok_as_is(_index: u64, _commit_index: u64) -> bool {
    true
}

proof fn lemma_as_is_acks_uncommitted(index: u64, commit_index: u64)
    requires
        commit_index < index,
    ensures
        !propose_ack_ok_spec(index, commit_index),
        propose_ack_ok_as_is(index, commit_index),
{
}

} // verus!
