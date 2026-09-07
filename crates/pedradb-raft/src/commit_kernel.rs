//! Pure commit / reopen watermarks (Beyond-style kernel, F10 / F18 / F23).
//!
//! **Single artifact (pair `commit_raft`):** this file is what `rustc` links
//! *and* what Verus proves (`cfg(verus_keep_ghost)`). Pair
//! `raft_recover_applied` still has a twin-cópia until its turn.
//!
//!   ./scripts/verus_commit_recover.sh
//!
//! # Contract
//!
//! - **No I/O, no clock, no RNG.**
//! - Production open + `update_commit_index` call these helpers.
//! - Persist of `RAFT_COMMIT` and apply are **caller + axiom**.
//!
//! The rustc bodies stay token-identical with the clone in
//! `pedradb-store::commit_kernel` (`catalog` `commit_raft_store`). Verus
//! proofs sit in the `cfg(verus_keep_ghost)` block above them.
//!
//! Spec page: `determinismo/pedradb-dst/formal/F10-F23-commit.md`.

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
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

/// F10 teeth: AS-IS can promote an uncommitted suffix (truncated-log
/// recovery must not treat log.len() as commit — NATS #7587 class).
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

/// F10: durable commit only, never `log.len()`. Cap by last on-disk index.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn recover_commit(loaded_commit: u64, log_last: u64) -> u64 {
    loaded_commit.min(log_last)
}

/// F10: always re-apply `1..=commit` after open (do not jump `last_applied`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn recover_last_applied() -> u64 {
    0
}

/// Raft §5.4.2 / F18 / F23: commit index `N` only if majority **and**
/// `log[N].term == current_term`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn may_commit_at(index_term: u64, current_term: u64, has_majority: bool) -> bool {
    has_majority && index_term == current_term
}

/// F11: client `Ok(index)` only if `commit_index` already covers `index`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn propose_ack_ok(index: u64, commit_index: u64) -> bool {
    commit_index >= index
}

/// AS-IS F10: treat the whole log as committed.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn recover_commit_as_is(_loaded_commit: u64, log_last: u64) -> u64 {
    log_last
}

/// AS-IS F10: skip re-apply (strand committed-but-unapplied).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn recover_last_applied_as_is(log_last: u64) -> u64 {
    log_last
}

/// AS-IS F23: majority alone commits a previous-term index.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn may_commit_at_as_is(_index_term: u64, _current_term: u64, has_majority: bool) -> bool {
    has_majority
}

/// AS-IS F11: ACK as soon as the entry is appended (ignore commit).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn propose_ack_ok_as_is(_index: u64, _commit_index: u64) -> bool {
    true
}

/// F10: commit watermark only moves forward.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn should_advance_commit(new_idx: u64, current: u64) -> bool {
    new_idx > current
}

/// AS-IS: always "advance" — would rewind commit when `new_idx < current`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn should_advance_commit_as_is(_new_idx: u64, _current: u64) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_caps_to_log() {
        assert_eq!(recover_commit(5, 3), 3);
        assert_eq!(recover_commit(2, 9), 2);
        assert_eq!(recover_commit(0, 0), 0);
    }

    #[test]
    fn recover_last_applied_is_zero() {
        assert_eq!(recover_last_applied(), 0);
    }

    #[test]
    fn as_is_false_commits_suffix() {
        assert_eq!(recover_commit_as_is(1, 5), 5);
        assert!(recover_commit_as_is(1, 5) > recover_commit(1, 5));
        assert_eq!(recover_last_applied_as_is(5), 5);
    }

    #[test]
    fn may_commit_requires_current_term() {
        assert!(may_commit_at(3, 3, true));
        assert!(!may_commit_at(2, 3, true));
        assert!(!may_commit_at(3, 3, false));
    }

    #[test]
    fn as_is_commits_prev_term() {
        assert!(may_commit_at_as_is(2, 3, true));
        assert_ne!(may_commit_at(2, 3, true), may_commit_at_as_is(2, 3, true));
    }

    #[test]
    fn propose_ack_requires_commit() {
        assert!(propose_ack_ok(3, 3));
        assert!(propose_ack_ok(3, 4));
        assert!(!propose_ack_ok(3, 2));
        assert!(propose_ack_ok_as_is(3, 2));
        assert_ne!(propose_ack_ok(3, 2), propose_ack_ok_as_is(3, 2));
    }

    #[test]
    fn theorem_recover_and_commit_on_finite_domain() {
        const B: u64 = 5;
        let mut n = 0u64;
        for loaded in 0..B {
            for last in 0..B {
                let c = recover_commit(loaded, last);
                assert!(c <= loaded && c <= last);
                assert_eq!(c, loaded.min(last));
                if loaded < last {
                    assert!(recover_commit_as_is(loaded, last) > c);
                }
                n += 1;
            }
        }
        for it in 0..B {
            for ct in 0..B {
                for maj in [false, true] {
                    let d = may_commit_at(it, ct, maj);
                    assert_eq!(d, maj && it == ct);
                    if maj && it != ct {
                        assert!(may_commit_at_as_is(it, ct, maj));
                        assert!(!d);
                    }
                    n += 1;
                }
            }
        }
        for idx in 0..B {
            for commit in 0..B {
                assert_eq!(propose_ack_ok(idx, commit), commit >= idx);
                assert!(propose_ack_ok_as_is(idx, commit));
                if commit < idx {
                    assert_ne!(propose_ack_ok(idx, commit), propose_ack_ok_as_is(idx, commit));
                }
                n += 1;
            }
        }
        for new_idx in 0..B {
            for current in 0..B {
                assert_eq!(should_advance_commit(new_idx, current), new_idx > current);
                assert!(should_advance_commit_as_is(new_idx, current));
                if new_idx <= current {
                    assert_ne!(
                        should_advance_commit(new_idx, current),
                        should_advance_commit_as_is(new_idx, current)
                    );
                }
                n += 1;
            }
        }
        assert_eq!(n, B * B + B * B * 2 + B * B + B * B);
    }
}
