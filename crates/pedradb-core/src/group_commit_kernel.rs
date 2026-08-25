//! RFC-0057 P2.1 / RFC-0058 P2.1: the group-commit kernel — the pure
//! decision core of the `ConcurrentDb` write group. The semantics
//! documented in RFC-0051 P1.3 (and enforced there by an empirical
//! oracle) become a theorem target here:
//!
//! - **first-committer-wins** ([`occ_conflict`]): a member that read
//!   snapshot `snap` conflicts iff some key it touched was written in
//!   `(snap, last_seq]`. Called by `WriteGroup::validate_occ_batch` and
//!   `WriteGroup::lone_commit` (`concurrent.rs`).
//! - **group atomicity** ([`group_validate`]): every member of a group
//!   is validated against the **same** `last_seq`, before any of the
//!   group's own sequences exist — members of the same group are
//!   simultaneous, with no serialization order between them (intra-group
//!   writes never conflict). Called by `WriteGroup::validate_occ_batch`
//!   after collecting each member's read state.
//! - **fence** ([`fence_publish_seq`]): the group becomes visible at one
//!   publish watermark — the max appended member sequence — after WAL
//!   durability. Called by `GroupInFlight::max_appended_seq` (`db.rs`).
//!
//! The Verus twin is `crates/pedradb-core/verus/group_commit.rs`; the
//! Aeneas extract is `formal/aeneas/lean/GroupCommitKernel.lean` with
//! theorems in `GroupCommit.lean` (second machine).
//!
//! `occ_conflict_as_is_serialized` is TEST-ONLY teeth (the serialized
//! mutant the theorems diverge from); production never calls it.

/// First-committer-wins predicate (OCC): a transaction that read
/// snapshot `snap` against current `last_seq` conflicts iff the window
/// `(snap, last_seq]` is non-empty **and** some key it touched was
/// written inside it. `last_seq > snap` (not `!=`) is the faithful
/// window: with `last_seq <= snap` the window is empty and no key can
/// be in it.
#[must_use]
pub fn occ_conflict(snap: u64, last_seq: u64, touched_key_written_after: bool) -> bool {
    last_seq > snap && touched_key_written_after
}

/// One member's OCC read of the pre-group state (collected under the
/// write lock, before any group sequence is assigned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccRead {
    /// Snapshot the member read at.
    pub snap: u64,
    /// Whether any key the member touched (read set ∪ write set) was
    /// written in `(snap, last_seq]`.
    pub touched_key_written_after: bool,
}

/// Group validation — the pure form of `validate_occ_batch`: every
/// member is decided against the same `last_seq`, so a member's outcome
/// never depends on another member (simultaneity). Position-for-position
/// conflict flags.
#[must_use]
pub fn group_validate(reads: &[OccRead], last_seq: u64) -> Vec<bool> {
    let mut out = Vec::with_capacity(reads.len());
    let mut i = 0;
    while i < reads.len() {
        out.push(occ_conflict(
            reads[i].snap,
            last_seq,
            reads[i].touched_key_written_after,
        ));
        i += 1;
    }
    out
}

/// The fence watermark: one publish sequence for the whole group — the
/// max appended member sequence (0 for an empty group).
#[must_use]
pub fn fence_publish_seq(member_seqs: &[u64]) -> u64 {
    let mut best = 0;
    let mut i = 0;
    while i < member_seqs.len() {
        if member_seqs[i] > best {
            best = member_seqs[i];
        }
        i += 1;
    }
    best
}

/// TEST-ONLY mutant (never called in production): the serialized
/// scheduler — members commit one at a time, so member `writes_before`
/// later members validate against `last_seq + writes_before`. With an
/// intra-group write to a shared key, the serialized form conflicts
/// where the group form does not: that divergence is exactly the
/// RFC-0051 P1.3 planted-bug shape the theorems pin.
#[must_use]
pub fn occ_conflict_as_is_serialized(
    snap: u64,
    last_seq: u64,
    writes_before: u64,
    touched_key_written_after: bool,
) -> bool {
    last_seq + writes_before > snap && touched_key_written_after
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_path_same_seq_never_conflicts() {
        assert!(!occ_conflict(7, 7, true));
        assert!(!occ_conflict(0, 0, true));
    }

    #[test]
    fn conflict_needs_window_and_touched_write() {
        assert!(occ_conflict(7, 9, true));
        assert!(!occ_conflict(7, 9, false));
        // Empty window (last_seq <= snap): nothing can be inside it.
        assert!(!occ_conflict(9, 7, true));
    }

    #[test]
    fn group_members_are_simultaneous() {
        // Two members of one group both touched the same key with
        // snapshots equal to last_seq: no conflict either way (the
        // group's own writes do not exist at validation time).
        let reads = [
            OccRead {
                snap: 10,
                touched_key_written_after: false,
            },
            OccRead {
                snap: 10,
                touched_key_written_after: false,
            },
        ];
        assert_eq!(group_validate(&reads, 10), vec![false, false]);
        // The serialized mutant aborts the second member.
        assert!(occ_conflict_as_is_serialized(10, 10, 1, true));
    }

    #[test]
    fn fence_is_max_member_seq() {
        assert_eq!(fence_publish_seq(&[]), 0);
        assert_eq!(fence_publish_seq(&[3]), 3);
        assert_eq!(fence_publish_seq(&[5, 2, 9, 4]), 9);
        assert_eq!(fence_publish_seq(&[0, 0]), 0);
    }
}
