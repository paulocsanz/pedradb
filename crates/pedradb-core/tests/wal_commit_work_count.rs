//! RFC-0199 P1.1 counting ladder — Rust twin of
//! `wal_commit_plan_at_most_one_fdatasync` (Lean:
//! `formal/aeneas/lean/WorkIo.lean`, count row
//! `catalog:wal_commit_plan`).
//!
//! The theorem's claim in Rust terms: a wal commit plan that APPLIES OK
//! (a committed group) executes at most one `fdatasync` — exactly one
//! when the group asked for sync, zero otherwise; the barrier is per
//! group, never per write in the batch. The REAL kernels are driven on
//! every shape: `wal_commit_plan` (the extracted plan) and
//! `fdatasync_rc_ok` (the extracted posix barrier primitive, rc = 0 is
//! the only ok). The Work mirror below is the Lean algebra's counting
//! interpretation done in-test; it counts constructors, it does not
//! re-implement any kernel decision.

use pedradb_core::write_admission_kernel::WalCommitPlan;
use pedradb_core::write_admission_kernel::wal_commit_plan;
use pedradb_posix::fdatasync_rc_ok;

/// In-test mirror of the Lean `Work.fdatasync_count` interpretation of
/// a plan (`wal_commit_work` in `WorkIo.lean`): AppendApplyOk commits
/// with no barrier; AppendSyncApplyOk pays exactly one; the fence paid
/// its (failed) barrier and refused.
fn work_fdatasync_count(p: &WalCommitPlan) -> usize {
    match p {
        WalCommitPlan::AppendApplyOk => 0,
        WalCommitPlan::AppendSyncApplyOk => 1,
        WalCommitPlan::AppendSyncFence => 1,
    }
}

/// The Lean `wal_commit_applies_ok`: the fence outcome is a refusal,
/// never a commit.
fn applies_ok(p: &WalCommitPlan) -> bool {
    !matches!(p, WalCommitPlan::AppendSyncFence)
}

/// The theorem's full input space is Bool × Bool — drive every shape of
/// the REAL plan and assert the counted bound and the exact decisions.
#[test]
fn committed_group_pays_at_most_one_fdatasync() {
    for need_sync in [false, true] {
        for sync_failed in [false, true] {
            let p = wal_commit_plan(need_sync, sync_failed);
            let syncs = work_fdatasync_count(&p);
            // the sharp count (Lean: wal_commit_plan_committed_sync_count)
            if applies_ok(&p) {
                assert_eq!(
                    syncs,
                    usize::from(need_sync),
                    "committed group: exactly one barrier iff it asked for sync"
                );
            }
            // the registered bound (≤1 for every plan, committed or fenced)
            assert!(
                syncs <= 1,
                "need_sync={need_sync} sync_failed={sync_failed}: {syncs} barriers"
            );
            // exact decisions per shape (the real kernel, not the mirror)
            match (need_sync, sync_failed) {
                (true, true) => {
                    assert!(matches!(p, WalCommitPlan::AppendSyncFence));
                }
                (true, false) => {
                    assert!(matches!(p, WalCommitPlan::AppendSyncApplyOk));
                }
                (false, _) => {
                    assert!(matches!(p, WalCommitPlan::AppendApplyOk));
                }
            }
        }
    }
}

/// The barrier primitive the constructor counts: `fdatasync_rc_ok`
/// admits exactly rc = 0 — every other return code is a failed barrier,
/// so a failed group fences instead of committing (never a second
/// barrier, never a silent skip).
#[test]
fn barrier_primitive_admits_only_rc_zero() {
    for rc in [0i32, -1, 1, 4, i32::MAX, i32::MIN] {
        assert_eq!(fdatasync_rc_ok(rc), rc == 0);
    }
    assert!(fdatasync_rc_ok(0));
    // the failed barrier routes the group to the fence arm of the plan
    let fenced = wal_commit_plan(true, !fdatasync_rc_ok(-1));
    assert!(matches!(fenced, WalCommitPlan::AppendSyncFence));
}

/// Composition sanity for the algebra's seq rule (Lean:
/// `Work.seq w₁ w₂` counts additively): committing two groups in
/// sequence pays their counts summed — still one barrier per group.
#[test]
fn two_groups_amortize_two_barriers_not_four() {
    let g1 = wal_commit_plan(true, false); // committed, asked sync: 1
    let g2 = wal_commit_plan(false, false); // committed, no sync: 0
    let total = work_fdatasync_count(&g1) + work_fdatasync_count(&g2);
    assert_eq!(total, 1);
    assert!(total <= 2);
}
