//! Pure commit / reopen watermarks (RFC-0002 P7 / F10 / F23).
//!
//! Same rules as `pedradb-raft::commit_kernel`. Store does not depend on
//! `pedradb-raft`; keep the two bodies identical (drift-trap: harness grid).

#![forbid(unsafe_code)]

/// F10: durable commit only, never `log.len()`.
#[must_use]
pub fn recover_commit(loaded_commit: u64, log_last: u64) -> u64 {
    loaded_commit.min(log_last)
}

/// Raft §5.4.2 / F23: majority **and** current-term index.
#[must_use]
pub fn may_commit_at(index_term: u64, current_term: u64, has_majority: bool) -> bool {
    has_majority && index_term == current_term
}

/// F11: client `Ok` only if `commit` already covers `index`.
#[must_use]
pub fn propose_ack_ok(index: u64, commit_index: u64) -> bool {
    commit_index >= index
}

/// AS-IS F10: whole log is committed.
#[must_use]
pub fn recover_commit_as_is(_loaded_commit: u64, log_last: u64) -> u64 {
    log_last
}

/// AS-IS F23: majority alone commits a previous-term index.
#[must_use]
pub fn may_commit_at_as_is(_index_term: u64, _current_term: u64, has_majority: bool) -> bool {
    has_majority
}

/// AS-IS F11: ACK as soon as the entry is appended.
#[must_use]
pub fn propose_ack_ok_as_is(_index: u64, _commit_index: u64) -> bool {
    true
}

/// Majority of `n` voters (`⌊n/2⌋+1`). Empty set never grants.
#[must_use]
pub fn majority_of(n: u64) -> u64 {
    if n == 0 {
        1
    } else {
        n / 2 + 1
    }
}

/// Raft §6: majority(C-old) ∧ majority(C-new) when joint is in flight.
#[must_use]
pub fn joint_election_ok(old_yes: u64, old_n: u64, new_yes: Option<(u64, u64)>) -> bool {
    if old_yes < majority_of(old_n) {
        return false;
    }
    match new_yes {
        None => true,
        Some((yes, n)) => yes >= majority_of(n),
    }
}

/// AS-IS: ignore C-new (elects on C-old during joint add).
#[must_use]
pub fn joint_election_ok_as_is(old_yes: u64, old_n: u64, _new_yes: Option<(u64, u64)>) -> bool {
    old_yes >= majority_of(old_n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_does_not_promote_suffix() {
        assert_eq!(recover_commit(1, 5), 1);
        assert_eq!(recover_commit_as_is(1, 5), 5);
    }

    #[test]
    fn prev_term_needs_current_term_index() {
        assert!(!may_commit_at(1, 2, true));
        assert!(may_commit_at_as_is(1, 2, true));
    }

    #[test]
    fn propose_ack_requires_commit() {
        assert!(!propose_ack_ok(3, 2));
        assert!(propose_ack_ok_as_is(3, 2));
    }

    #[test]
    fn joint_election_old_majority_is_not_enough_during_add() {
        // C-old=3 (maj 2), C-new=4 (maj 3): two old votes elect AS-IS, not joint.
        assert!(!joint_election_ok(2, 3, Some((2, 4))));
        assert!(joint_election_ok_as_is(2, 3, Some((2, 4))));
        assert!(joint_election_ok(2, 3, Some((3, 4))));
        assert!(joint_election_ok(2, 3, None));
        assert!(!joint_election_ok(1, 3, None));
        assert!(!joint_election_ok(2, 3, Some((0, 0))));
    }
}
