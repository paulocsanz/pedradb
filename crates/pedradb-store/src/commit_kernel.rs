//! Pure commit / reopen watermarks (RFC-0002 P7 / F10 / F23).
//!
//! Same rules as `pedradb-raft::commit_kernel`. Production code must not
//! depend on `pedradb-raft` (it is a dev-dependency only); token identity
//! is frozen by `pedra_formal.py --ci` (check_clones) and same-function
//! agreement is pinned by the cross-crate twin test below, so a drift on
//! either side breaks `cargo test`, not just the lint.

#![forbid(unsafe_code)]

/// F10: durable commit only, never `log.len()`.
#[must_use]
pub fn recover_commit(loaded_commit: u64, log_last: u64) -> u64 {
    loaded_commit.min(log_last)
}

/// F10: always re-apply `1..=commit` after open (do not jump `last_applied`).
#[must_use]
pub fn recover_last_applied() -> u64 {
    0
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
        assert_eq!(recover_last_applied(), 0);
        assert_eq!(recover_last_applied_as_is(5), 5);
        assert!(recover_last_applied_as_is(5) > recover_last_applied());
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

    /// Clone twin (catalog `commit_raft_store`): both copies must implement
    /// the same function. Token identity (lint) catches one-sided drift;
    /// this cross-crate sweep also catches both-sides drift at `cargo test`
    /// time and pins boundary behavior (`u64::MAX`) tokens cannot express.
    #[test]
    fn twin_agrees_with_raft_commit_kernel_on_full_domain() {
        use pedradb_raft::commit_kernel as raft;
        let anchors = [0u64, 1, 2, 3, 5, 8, 61, u64::MAX - 1, u64::MAX];
        let mut checked = 0usize;
        assert_eq!(recover_last_applied(), raft::recover_last_applied(), "recover_last_applied()");
        checked += 1;
        for &x in &anchors {
            assert_eq!(
                recover_last_applied_as_is(x),
                raft::recover_last_applied_as_is(x),
                "recover_last_applied_as_is({x})"
            );
            checked += 1;
            for &y in &anchors {
                assert_eq!(recover_commit(x, y), raft::recover_commit(x, y), "recover_commit({x},{y})");
                assert_eq!(
                    recover_commit_as_is(x, y),
                    raft::recover_commit_as_is(x, y),
                    "recover_commit_as_is({x},{y})"
                );
                assert_eq!(propose_ack_ok(x, y), raft::propose_ack_ok(x, y), "propose_ack_ok({x},{y})");
                assert_eq!(
                    propose_ack_ok_as_is(x, y),
                    raft::propose_ack_ok_as_is(x, y),
                    "propose_ack_ok_as_is({x},{y})"
                );
                checked += 4;
            }
            for &t in &anchors {
                for maj in [false, true] {
                    assert_eq!(may_commit_at(x, t, maj), raft::may_commit_at(x, t, maj), "may_commit_at({x},{t},{maj})");
                    assert_eq!(
                        may_commit_at_as_is(x, t, maj),
                        raft::may_commit_at_as_is(x, t, maj),
                        "may_commit_at_as_is({x},{t},{maj})"
                    );
                    checked += 2;
                }
            }
        }
        // 1 + n as_is last_applied + n² pairs ×4 + n² terms ×2 majority ×2.
        let n = anchors.len();
        assert_eq!(checked, 1 + n + n * n * 4 + n * n * 2 * 2);
    }

    /// Mirror of the raft-side theorem on THIS copy (F10/F23/F11): without
    /// it, both sides could drift together and only the raft-side test
    /// would notice.
    #[test]
    fn theorem_commit_invariants_on_finite_domain() {
        const B: u64 = 5;
        for loaded in 0..B {
            for last in 0..B {
                let c = recover_commit(loaded, last);
                assert!(c <= loaded && c <= last);
                assert_eq!(c, loaded.min(last));
                if loaded < last {
                    assert!(recover_commit_as_is(loaded, last) > c);
                }
            }
        }
        for it in 0..B {
            for ct in 0..B {
                for maj in [false, true] {
                    assert_eq!(may_commit_at(it, ct, maj), maj && it == ct);
                    if maj && it != ct {
                        assert!(may_commit_at_as_is(it, ct, maj));
                        assert!(!may_commit_at(it, ct, maj));
                    }
                }
            }
        }
        for idx in 0..B {
            for commit in 0..B {
                assert_eq!(propose_ack_ok(idx, commit), commit >= idx);
                assert!(propose_ack_ok_as_is(idx, commit));
            }
        }
    }
}
