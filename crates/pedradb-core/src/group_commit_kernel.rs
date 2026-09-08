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
//! The Verus twin is `crates/pedradb-core/verus/group_commit.rs` except
//! [`rwlock_client_may_mutate`]: that fn is **single artifact** — this
//! file is what `rustc` links *and* what Verus proves
//! (`cfg(verus_keep_ghost)`). `./scripts/verus_group_commit_kernel.sh`
//!
//! The Aeneas extract is `formal/aeneas/lean/GroupCommitKernel.lean` with
//! theorems in `GroupCommit.lean` (second machine).
//!
//! `occ_conflict_as_is_serialized` is TEST-ONLY teeth (the serialized
//! mutant the theorems diverge from); production never calls it.

macro_rules! rwlock_client_may_mutate_body {
    ($holding_write:expr) => {
        $holding_write
    };
}

macro_rules! rwlock_client_may_mutate_as_is_body {
    ($holding_write:expr) => {{
        let _ = $holding_write;
        true
    }};
}

macro_rules! occ_member_fate_body {
    ($too_old:expr, $conflict:expr) => {
        if $too_old {
            OccMemberFate::TooOld
        } else if $conflict {
            OccMemberFate::Conflict
        } else {
            OccMemberFate::Ok
        }
    };
}

macro_rules! occ_member_fate_as_is_body {
    ($too_old:expr, $conflict:expr) => {{
        let _ = ($too_old, $conflict);
        OccMemberFate::Ok
    }};
}

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
#[cfg(not(verus_keep_ghost))]
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

/// Fate of one OCC member after `group_validate` (and snapshot TooOld).
/// `validate_occ_batch` matches this — TooOld wins over Conflict.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccMemberFate {
    /// Apply with the group.
    Ok,
    /// Snapshot unreadable — abort TooOld.
    TooOld,
    /// OCC conflict — abort TransactionConflict.
    Conflict,
}

/// Caller of `group_validate`: too-old or conflict ⇒ abort that member.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_member_fate(too_old: bool, conflict: bool) -> OccMemberFate {
    occ_member_fate_body!(too_old, conflict)
}

/// AS-IS: never abort (lagging member commits).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_member_fate_as_is(_too_old: bool, _conflict: bool) -> OccMemberFate {
    occ_member_fate_as_is_body!(_too_old, _conflict)
}

/// ConcurrentDb `validate_occ_batch` / `lone_commit` plan: TooOld wins
/// over Conflict over Ok, against one `last_seq`. Glue collects
/// (`too_old`, `OccRead`); this fn is the order rustc links.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_batch_plan(
    too_old: &[bool],
    reads: &[OccRead],
    last_seq: u64,
) -> Vec<OccMemberFate> {
    let n = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let conflict = occ_conflict(
            reads[i].snap,
            last_seq,
            reads[i].touched_key_written_after,
        );
        out.push(occ_member_fate(too_old[i], conflict));
        i += 1;
    }
    out
}

/// AS-IS: every member Ok (lagging / too-old still commit).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn occ_batch_plan_as_is(
    too_old: &[bool],
    reads: &[OccRead],
    _last_seq: u64,
) -> Vec<OccMemberFate> {
    let n = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let _ = (too_old[i], reads[i]);
        out.push(OccMemberFate::Ok);
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

/// Data-race token (CapybaraKV RW-lock *client*, not `parking_lot`):
/// exclusive mutate of `Db` only while the write guard is held. Off-lock
/// fd (`drop(guard)` then `sync_data`) must pass `false`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn rwlock_client_may_mutate(holding_write: bool) -> bool {
    rwlock_client_may_mutate_body!(holding_write)
}

/// AS-IS: mutate even after dropping the write lock (data-race lie).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn rwlock_client_may_mutate_as_is(_holding_write: bool) -> bool {
    rwlock_client_may_mutate_as_is_body!(_holding_write)
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

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn rwlock_client_may_mutate_spec(holding_write: bool) -> bool {
    holding_write
}

pub fn rwlock_client_may_mutate(holding_write: bool) -> (ok: bool)
    ensures
        ok == rwlock_client_may_mutate_spec(holding_write),
        holding_write ==> ok,
        !holding_write ==> !ok,
{
    rwlock_client_may_mutate_body!(holding_write)
}

pub fn rwlock_client_may_mutate_as_is(_holding_write: bool) -> (ok: bool)
    ensures
        ok == true,
{
    rwlock_client_may_mutate_as_is_body!(_holding_write)
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum OccMemberFate {
    Ok,
    TooOld,
    Conflict,
}

pub open spec fn occ_member_fate_spec(too_old: bool, conflict: bool) -> OccMemberFate {
    if too_old {
        OccMemberFate::TooOld
    } else if conflict {
        OccMemberFate::Conflict
    } else {
        OccMemberFate::Ok
    }
}

pub fn occ_member_fate(too_old: bool, conflict: bool) -> (d: OccMemberFate)
    ensures
        d == occ_member_fate_spec(too_old, conflict),
{
    occ_member_fate_body!(too_old, conflict)
}

pub fn occ_member_fate_as_is(_too_old: bool, _conflict: bool) -> (d: OccMemberFate)
    ensures
        d == OccMemberFate::Ok,
{
    occ_member_fate_as_is_body!(_too_old, _conflict)
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct OccRead {
    pub snap: u64,
    pub touched_key_written_after: bool,
}

pub open spec fn occ_conflict_spec(
    snap: u64,
    last_seq: u64,
    touched_key_written_after: bool,
) -> bool {
    last_seq > snap && touched_key_written_after
}

pub open spec fn occ_batch_plan_spec(
    too_old: &[bool],
    reads: &[OccRead],
    last_seq: u64,
) -> Seq<OccMemberFate> {
    let n = if too_old@.len() <= reads@.len() {
        too_old@.len()
    } else {
        reads@.len()
    };
    Seq::new(
        n,
        |i: int|
            if 0 <= i < too_old@.len() && i < reads@.len() {
                occ_member_fate_spec(
                    too_old[i],
                    occ_conflict_spec(
                        reads[i].snap,
                        last_seq,
                        reads[i].touched_key_written_after,
                    ),
                )
            } else {
                OccMemberFate::Ok
            },
    )
}

pub fn occ_batch_plan(
    too_old: &[bool],
    reads: &[OccRead],
    last_seq: u64,
) -> (out: Vec<OccMemberFate>)
    ensures
        out@ == occ_batch_plan_spec(too_old, reads, last_seq),
{
    let n: usize = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out: Vec<OccMemberFate> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            0 <= i <= n,
            n <= too_old.len(),
            n <= reads.len(),
            n == (if too_old@.len() <= reads@.len() {
                too_old@.len()
            } else {
                reads@.len()
            }),
            out.len() == i,
            forall|j: int|
                0 <= j < i ==> out[j] == occ_member_fate_spec(
                    too_old[j],
                    occ_conflict_spec(
                        reads[j].snap,
                        last_seq,
                        reads[j].touched_key_written_after,
                    ),
                ),
        decreases n - i,
    {
        let conflict = last_seq > reads[i].snap && reads[i].touched_key_written_after;
        out.push(occ_member_fate(too_old[i], conflict));
        i += 1;
    }
    proof {
        assert(out@ == occ_batch_plan_spec(too_old, reads, last_seq));
    }
    out
}

pub fn occ_batch_plan_as_is(
    too_old: &[bool],
    reads: &[OccRead],
    _last_seq: u64,
) -> (out: Vec<OccMemberFate>)
    ensures
        out.len() == (if too_old.len() <= reads.len() {
            too_old.len()
        } else {
            reads.len()
        }),
        forall|j: int| 0 <= j < out.len() ==> out[j] == OccMemberFate::Ok,
{
    let n: usize = if too_old.len() <= reads.len() {
        too_old.len()
    } else {
        reads.len()
    };
    let mut out: Vec<OccMemberFate> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            0 <= i <= n,
            n <= too_old.len(),
            n <= reads.len(),
            out.len() == i,
            forall|j: int| 0 <= j < i ==> out[j] == OccMemberFate::Ok,
        decreases n - i,
    {
        let _ = (too_old[i], reads[i]);
        out.push(OccMemberFate::Ok);
        i += 1;
    }
    out
}

} // verus!

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
        let n3 = [
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 7,
                touched_key_written_after: true,
            },
        ];
        assert_eq!(
            group_validate(&n3, 10),
            vec![false, false, true],
            "N-way: only the lagging member conflicts"
        );
    }

    #[test]
    fn occ_member_fate_on_live_conflict_is_not_ok() {
        assert_eq!(
            occ_member_fate(false, true),
            OccMemberFate::Conflict
        );
        assert_eq!(occ_member_fate(true, true), OccMemberFate::TooOld);
        assert_eq!(occ_member_fate(false, false), OccMemberFate::Ok);
        assert_eq!(
            occ_member_fate_as_is(true, true),
            OccMemberFate::Ok,
            "AS-IS dente: lagging member still Ok"
        );
        let src = include_str!("concurrent.rs");
        assert!(
            src.contains("occ_batch_plan("),
            "validate_occ_batch must match occ_batch_plan"
        );
        let lone = src
            .split("fn lone_commit")
            .nth(1)
            .expect("lone_commit");
        assert!(
            lone.contains("occ_batch_plan("),
            "lone_commit must match occ_batch_plan"
        );
    }

    #[test]
    fn occ_batch_plan_on_live_lagging_is_not_ok() {
        let too_old = [false, true];
        let reads = [
            OccRead {
                snap: 10,
                touched_key_written_after: true,
            },
            OccRead {
                snap: 7,
                touched_key_written_after: true,
            },
        ];
        assert_eq!(
            occ_batch_plan(&too_old, &reads, 10),
            vec![OccMemberFate::Ok, OccMemberFate::TooOld]
        );
        let lag = [false];
        let lag_read = [OccRead {
            snap: 7,
            touched_key_written_after: true,
        }];
        assert_eq!(
            occ_batch_plan(&lag, &lag_read, 10),
            vec![OccMemberFate::Conflict]
        );
        assert_eq!(
            occ_batch_plan_as_is(&lag, &lag_read, 10),
            vec![OccMemberFate::Ok],
            "AS-IS dente: lagging member still Ok"
        );
        let src = include_str!("concurrent.rs");
        let validate = src
            .split("fn validate_occ_batch")
            .nth(1)
            .expect("validate_occ_batch");
        assert!(
            validate.contains("occ_batch_plan("),
            "validate_occ_batch must match occ_batch_plan"
        );
        let lone = src.split("fn lone_commit").nth(1).expect("lone_commit");
        assert!(
            lone.contains("occ_batch_plan("),
            "lone_commit must match occ_batch_plan"
        );
    }

    #[test]
    fn rwlock_client_may_mutate_on_live_off_lock_is_not_ok() {
        assert!(rwlock_client_may_mutate(true));
        assert!(!rwlock_client_may_mutate(false));
        assert!(
            rwlock_client_may_mutate_as_is(false),
            "AS-IS dente: mutate after dropping the write lock"
        );
        let src = include_str!("concurrent.rs");
        let off = src
            .split("fn finish_group_off_lock")
            .nth(1)
            .expect("finish_group_off_lock");
        assert!(
            off.contains("drop(guard)"),
            "off-lock fd drops the write guard"
        );
        assert!(
            off.contains("rwlock_client_may_mutate("),
            "finish_group_off_lock must match the data-race token"
        );
        let after_drop = off.split("drop(guard)").nth(1).expect("after drop");
        let until_reacquire = after_drop.split("db.write()").next().expect("until write");
        assert!(
            !until_reacquire.contains("group_apply("),
            "must not apply mem while the write lock is dropped"
        );
        assert!(
            !until_reacquire.contains("publish_sequence("),
            "must not publish while the write lock is dropped"
        );
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
