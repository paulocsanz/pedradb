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

/// Finite PCT depth never covers ∀ OS interleavings (RFC-0070 / R-pct).
/// A campaign of depth `pct_depth` (including d=2) is not a ∀π theorem.
#[must_use]
pub fn forall_schedules_admitted(_pct_depth: u64) -> bool {
    false
}

/// AS-IS: d≥2 is rounded to forall (the 0070 hole — PCT CLEAN as a theorem).
#[must_use]
pub fn forall_schedules_admitted_as_is(pct_depth: u64) -> bool {
    pct_depth >= 2
}

/// RFC-0070 P2.2: campaign default PCT depth. d>2 stays RFC-0051
/// (`planted_depth3_three_teeth`); this RFC does not raise it.
#[must_use]
pub fn pct_campaign_default_depth() -> u64 {
    2
}

/// RFC-0070 P2.2: admit a “0070 raised the default PCT depth” claim.
/// Always false.
#[must_use]
pub fn default_pct_depth_raised() -> bool {
    false
}

/// AS-IS: 0070 P2 is rounded to “we now run d>2 by default”.
#[must_use]
pub fn default_pct_depth_raised_as_is() -> bool {
    true
}

/// Visibility publish after group (or lone) WAL I/O (RFC-0071 / R-group-glue).
/// The group becomes visible only when off-lock / lone WAL I/O succeeded.
#[must_use]
pub fn may_publish_group(wal_io_ok: bool) -> bool {
    wal_io_ok
}

/// AS-IS: publish even if WAL I/O failed (the 0071 hole — Ok with a lie).
#[must_use]
pub fn may_publish_group_as_is(_wal_io_ok: bool) -> bool {
    true
}

/// RFC-0071 P2.2: lock / OS-scheduler interleavings around the publish
/// gate are not a ∀π theorem. Always refuse.
#[must_use]
pub fn lock_interleavings_admitted() -> bool {
    false
}

/// AS-IS: a green publish gate is rounded to ∀ lock schedules.
#[must_use]
pub fn lock_interleavings_admitted_as_is() -> bool {
    true
}

/// RFC-0078 / R-fsync-lie: promote pending bytes only when the OS (or Env)
/// is honest. A lying `fsync` Ok must not make the write crash-durable.
#[must_use]
pub fn fsync_promotes_pending(os_honest: bool) -> bool {
    os_honest
}

/// AS-IS: fsync Ok always promotes (the 0078 hole — Lying recovers).
#[must_use]
pub fn fsync_promotes_pending_as_is(_os_honest: bool) -> bool {
    true
}

/// `fdatasync` rc==0 is not a proof the drive stored the bytes (R-fsync-lie).
#[must_use]
pub fn media_durable_admitted(_fsync_ok: bool) -> bool {
    false
}

/// AS-IS: rc==0 is rounded to a media theorem (the 0078 hole).
#[must_use]
pub fn media_durable_admitted_as_is(fsync_ok: bool) -> bool {
    fsync_ok
}

/// RFC-0078 P1.2 / RFC-0052: `RecordingEnv::Lying` and det_io PRELOAD
/// are two fsync-liar boxes. Stacking them in one process is not a
/// campaign. Always refuse.
#[must_use]
pub fn stacked_fsync_liars_admitted(_lying: bool, _det_io: bool) -> bool {
    false
}

/// AS-IS: AND both liar boxes in one run (the 0052 hole).
#[must_use]
pub fn stacked_fsync_liars_admitted_as_is(lying: bool, det_io: bool) -> bool {
    lying && det_io
}

/// RFC-0078 P2.2: closing the lying-fsync model does not invent a TCG
/// guest (`R-tcg-guest` stays 0079). Always false.
#[must_use]
pub fn fsync_lie_closes_tcg_guest() -> bool {
    false
}

/// AS-IS: 0078 is rounded to TCG guest coverage (the hole).
#[must_use]
pub fn fsync_lie_closes_tcg_guest_as_is() -> bool {
    true
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

    #[test]
    fn pct_depth_is_not_forall_schedules() {
        assert!(!forall_schedules_admitted(0));
        assert!(!forall_schedules_admitted(2));
        assert!(!forall_schedules_admitted(3));
        assert!(!forall_schedules_admitted_as_is(0));
        assert!(forall_schedules_admitted_as_is(2));
        assert!(forall_schedules_admitted_as_is(3));
        assert_eq!(pct_campaign_default_depth(), 2);
        assert!(!default_pct_depth_raised());
        assert!(
            default_pct_depth_raised_as_is(),
            "AS-IS dente: 0070 would claim it raised default depth"
        );
    }

    #[test]
    fn fsync_ok_is_not_media_proof() {
        assert!(fsync_promotes_pending(true));
        assert!(!fsync_promotes_pending(false));
        assert!(
            fsync_promotes_pending_as_is(false),
            "AS-IS dente: promote on a lying fsync"
        );
        assert!(!media_durable_admitted(true));
        assert!(!media_durable_admitted(false));
        assert!(
            media_durable_admitted_as_is(true),
            "AS-IS dente: fsync Ok proves the drive"
        );
        assert!(!media_durable_admitted_as_is(false));
        assert!(!stacked_fsync_liars_admitted(true, true));
        assert!(!stacked_fsync_liars_admitted(true, false));
        assert!(
            stacked_fsync_liars_admitted_as_is(true, true),
            "AS-IS dente: AND Lying × det_io in one run"
        );
        assert!(!stacked_fsync_liars_admitted_as_is(true, false));
        assert!(!fsync_lie_closes_tcg_guest());
        assert!(
            fsync_lie_closes_tcg_guest_as_is(),
            "AS-IS dente: 0078 would invent a TCG guest"
        );
    }

    #[test]
    fn publish_only_when_wal_io_ok() {
        assert!(may_publish_group(true));
        assert!(!may_publish_group(false));
        assert!(may_publish_group_as_is(false));
        assert!(may_publish_group_as_is(true));
    }

    #[test]
    fn lock_interleavings_are_not_a_theorem() {
        assert!(!lock_interleavings_admitted());
        assert!(
            lock_interleavings_admitted_as_is(),
            "AS-IS dente: admit ∀ lock schedules"
        );
    }
}
