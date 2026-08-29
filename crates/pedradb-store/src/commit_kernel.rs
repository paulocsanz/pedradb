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
}
