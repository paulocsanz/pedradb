//! Pure commit / reopen watermarks (Beyond-style kernel, F10 / F18 / F23).
//!
//! # Contract
//!
//! - **No I/O, no clock, no RNG.**
//! - Production open + `update_commit_index` call these helpers.
//! - Persist of `RAFT_COMMIT` and apply are **caller + axiom**.
//!
//! Spec page: `determinismo/pedradb-dst/formal/F10-F23-commit.md`.

#![forbid(unsafe_code)]

/// F10: durable commit only, never `log.len()`. Cap by last on-disk index.
#[must_use]
pub fn recover_commit(loaded_commit: u64, log_last: u64) -> u64 {
    loaded_commit.min(log_last)
}

/// F10: always re-apply `1..=commit` after open (do not jump `last_applied`).
#[must_use]
pub fn recover_last_applied() -> u64 {
    0
}

/// Raft §5.4.2 / F18 / F23: commit index `N` only if majority **and**
/// `log[N].term == current_term`.
#[must_use]
pub fn may_commit_at(index_term: u64, current_term: u64, has_majority: bool) -> bool {
    has_majority && index_term == current_term
}

/// F11: client `Ok(index)` only if `commit_index` already covers `index`.
#[must_use]
pub fn propose_ack_ok(index: u64, commit_index: u64) -> bool {
    commit_index >= index
}

/// AS-IS F10: treat the whole log as committed.
#[must_use]
pub fn recover_commit_as_is(_loaded_commit: u64, log_last: u64) -> u64 {
    log_last
}

/// AS-IS F10: skip re-apply (strand committed-but-unapplied).
#[must_use]
pub fn recover_last_applied_as_is(log_last: u64) -> u64 {
    log_last
}

/// AS-IS F23: majority alone commits a previous-term index.
#[must_use]
pub fn may_commit_at_as_is(_index_term: u64, _current_term: u64, has_majority: bool) -> bool {
    has_majority
}

/// AS-IS F11: ACK as soon as the entry is appended (ignore commit).
#[must_use]
pub fn propose_ack_ok_as_is(_index: u64, _commit_index: u64) -> bool {
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
        assert_eq!(n, B * B + B * B * 2);
    }
}
