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

/// AS-IS RFC-0057: fence is the first member's seq — later members stay
/// unpublished at the watermark.
#[must_use]
pub fn fence_publish_seq_as_is(member_seqs: &[u64]) -> u64 {
    if member_seqs.is_empty() {
        0
    } else {
        member_seqs[0]
    }
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

    /// RFC-0157 P1.2 — property sweep over the pure group-commit kernel
    /// family (deterministic seeded trials; the recorded trial IS the
    /// shrunk counterexample). Pins: `occ_conflict` == non-empty window
    /// AND touched; `group_validate` is position-independent
    /// (simultaneity); `fence_publish_seq` == max member seq; the
    /// `may_publish_group` AS-IS mutant diverges exactly when WAL I/O
    /// failed (publish without durability).
    #[test]
    fn rfc0157_property_sweep_group_commit_kernel() {
        use crate::{Rng, SeedRng};
        // Exhaustive boolean cases first.
        assert_eq!(may_publish_group(true), true);
        assert_ne!(
            may_publish_group(false),
            may_publish_group_as_is(false),
            "AS-IS publish mutant must diverge at wal_io_ok=false"
        );
        assert_eq!(
            may_publish_group(true),
            may_publish_group_as_is(true),
            "both publish when WAL I/O succeeded"
        );
        // The RFC-0051 plant shape stays reachable in the pure kernel:
        // serialized scheduling conflicts where the group does not.
        assert!(
            occ_conflict_as_is_serialized(10, 10, 1, true) && !occ_conflict(10, 10, true),
            "AS-IS serialized mutant must keep the intra-group tooth"
        );

        let mut viol: Option<String> = None;
        'trials: for trial in 0..20_000u64 {
            let rng = SeedRng::new(0x0157_5712 ^ trial);
            let last_seq = rng.gen_range(64);
            let n = 1 + (rng.gen_range(6) as usize);
            let mut reads = Vec::with_capacity(n);
            for _ in 0..n {
                reads.push(OccRead {
                    snap: rng.gen_range(64),
                    touched_key_written_after: rng.gen_range(2) == 0,
                });
            }
            let mut seqs = Vec::with_capacity(n);
            for _ in 0..n {
                seqs.push(rng.gen_range(64));
            }
            for r in &reads {
                let expect = last_seq > r.snap && r.touched_key_written_after;
                if occ_conflict(r.snap, last_seq, r.touched_key_written_after) != expect {
                    viol = Some(format!(
                        "trial={trial} occ_conflict(snap={}, last_seq={}, touched={})",
                        r.snap, last_seq, r.touched_key_written_after
                    ));
                    break 'trials;
                }
            }
            let flags = group_validate(&reads, last_seq);
            for i in 0..n {
                let alone = occ_conflict(
                    reads[i].snap,
                    last_seq,
                    reads[i].touched_key_written_after,
                );
                if flags[i] != alone {
                    viol = Some(format!(
                        "trial={trial} member {i} flag {} != alone {alone} (group not simultaneous)",
                        flags[i]
                    ));
                    break 'trials;
                }
            }
            let fold_max = seqs.iter().copied().fold(0u64, u64::max);
            if fence_publish_seq(&seqs) != fold_max {
                viol = Some(format!("trial={trial} fence != max of {seqs:?}"));
                break 'trials;
            }
            let wal_io_ok = rng.gen_range(2) == 0;
            if may_publish_group(wal_io_ok) != wal_io_ok {
                viol = Some(format!("trial={trial} may_publish_group({wal_io_ok})"));
                break 'trials;
            }
        }
        assert_eq!(viol, None, "rfc0157 sweep counterexample: {viol:?}");
    }

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

    /// Catalog three-teeth plant. Direct `group_members_are_simultaneous` is **not** this tooth.
    #[test]
    fn occ_conflict_on_live_group_is_not_ok() {
        assert!(!occ_conflict(10, 10, true));
        assert!(
            occ_conflict_as_is_serialized(10, 10, 1, true),
            "AS-IS dente: serialized scheduler aborts the second intra-group member"
        );
        let dir = std::env::temp_dir().join(format!(
            "group-commit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = crate::ConcurrentDb::open_with(
            &dir,
            crate::OpenOptions {
                exclusive: true,
                ..crate::OpenOptions::default()
            },
        )
        .unwrap();
        db.put(b"k", b"v0").unwrap();
        let mut tx1 = db.begin_occ();
        let mut tx2 = db.begin_occ();
        assert_eq!(tx1.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        assert_eq!(tx2.get(b"k").unwrap().as_deref(), Some(b"v0".as_ref()));
        tx1.put(b"k", b"from1").unwrap();
        tx2.put(b"k", b"from2").unwrap();
        tx1.commit().unwrap();
        let err = tx2.commit().unwrap_err();
        assert!(
            matches!(err, crate::CoreError::TransactionConflict),
            "live ConcurrentDb first-committer-wins must conflict the lagging OCC commit, got {err:?}"
        );
        assert_eq!(
            db.get(b"k").as_deref(),
            Some(b"from1".as_ref()),
            "live lone/group OCC path keeps the first committer"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fence_is_max_member_seq() {
        assert_eq!(fence_publish_seq(&[]), 0);
        assert_eq!(fence_publish_seq(&[3]), 3);
        assert_eq!(fence_publish_seq(&[5, 2, 9, 4]), 9);
        assert_eq!(fence_publish_seq(&[0, 0]), 0);
    }

    /// Catalog three-teeth plant. Direct `fence_is_max_member_seq` is **not** this tooth.
    #[test]
    fn fence_publish_seq_on_live_group_is_not_ok() {
        assert_eq!(fence_publish_seq(&[5, 2, 9, 4]), 9);
        assert_eq!(
            fence_publish_seq_as_is(&[5, 2, 9, 4]),
            5,
            "AS-IS dente: fence is the first member, later seqs stay unpublished"
        );
        let dir = std::env::temp_dir().join(format!(
            "group-fence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = std::sync::Arc::new(
            crate::ConcurrentDb::open_with(
                &dir,
                crate::OpenOptions {
                    exclusive: true,
                    ..crate::OpenOptions::default()
                },
            )
            .unwrap(),
        );
        db.set_write_group_catchup_window(std::time::Duration::from_millis(20));
        db.put(b"warm", b"1").unwrap();
        let n = 8usize;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let db = std::sync::Arc::clone(&db);
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let k = [b'k', u8::try_from(i).expect("n fits u8")];
                db.put(&k, b"v")
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            results.iter().all(|r| r.is_ok()),
            "every group member must Ok: {results:?}"
        );
        let (submits, _queued, groups, group_ops) = db.write_group_stats();
        assert_eq!(submits, n as u64 + 1, "warm + {n} grouped puts");
        assert!(
            groups < n as u64 && group_ops >= 2,
            "must have taken max_appended_seq group path groups={groups} ops={group_ops}"
        );
        assert_eq!(
            db.visible_sequence(),
            db.last_sequence(),
            "live fence must publish the max member seq, not the first"
        );
        for i in 0..n {
            let k = [b'k', u8::try_from(i).expect("n fits u8")];
            assert_eq!(
                db.get(&k).as_deref(),
                Some(b"v".as_ref()),
                "live get after group Ok must see member {i}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
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
