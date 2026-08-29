// Verus proof of the flush-pipeline decision kernel
// (RFC-0056 P0.2 — crash dictionary on the memtable→SST→WAL-rotate path).
//
// Source of truth for production remains `src/flush_kernel.rs`
// (called by `Db::flush` / `Db::try_rotate_wal` /
// `Db::ensure_wal_rotated_for_gc`). This file is the machine-checked
// theorem: exec == spec, the mem tail is never dropped, the WAL is never
// truncated while any copy of acked keys depends on it, and the AS-IS
// mutants (lose-tail / ignore-pin) diverge exactly where the fixed
// kernel refuses.
//
//   ./scripts/verus_flush_decision.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

verus! {

/// Mirrors `FlushPlan` in flush_kernel.rs.
pub enum FlushPlan {
    FinishImmThenFlush,
    WriteSstThenRotate,
    RotateOnly,
}

/// Closed-form spec — same arms as `flush_kernel::flush_plan`.
pub open spec fn flush_plan_spec(mem_empty: bool, imm_present: bool) -> FlushPlan {
    if imm_present {
        FlushPlan::FinishImmThenFlush
    } else if !mem_empty {
        FlushPlan::WriteSstThenRotate
    } else {
        FlushPlan::RotateOnly
    }
}

/// AS-IS data loss: flush without writing the mem tail (WAL rotate then
/// truncates the only durable copy of acked mem keys).
pub open spec fn flush_plan_as_is(_mem_empty: bool, _imm_present: bool) -> FlushPlan {
    FlushPlan::RotateOnly
}

/// Executable decision — must match `pedradb_core::flush_kernel::
/// flush_plan` bit-for-bit.
#[verifier::when_used_as_spec(flush_plan_spec)]
pub fn flush_plan(mem_empty: bool, imm_present: bool) -> (p: FlushPlan)
    ensures
        p == flush_plan_spec(mem_empty, imm_present),
        (p == FlushPlan::RotateOnly) ==> (mem_empty && !imm_present),
        !mem_empty ==> p != FlushPlan::RotateOnly,
        imm_present ==> p == FlushPlan::FinishImmThenFlush,
{
    if imm_present {
        FlushPlan::FinishImmThenFlush
    } else if !mem_empty {
        FlushPlan::WriteSstThenRotate
    } else {
        FlushPlan::RotateOnly
    }
}

/// Mirrors `WalPinState` in flush_kernel.rs.
pub struct WalPinState {
    pub mem_empty: bool,
    pub imm_present: bool,
    pub pin_live: bool,
    pub parked_unflushed: bool,
    pub commit_inflight: bool,
}

/// Mirrors `WalRotateAction` in flush_kernel.rs.
pub enum WalRotateAction {
    RotateWal,
    KeepWal,
}

/// Closed-form spec — same arms as `flush_kernel::wal_rotate_decision`.
pub open spec fn wal_rotate_spec(s: WalPinState) -> WalRotateAction {
    if s.mem_empty
        && !s.imm_present
        && !s.pin_live
        && !s.parked_unflushed
        && !s.commit_inflight
    {
        WalRotateAction::RotateWal
    } else {
        WalRotateAction::KeepWal
    }
}

/// AS-IS pre-fix hole: decide the rotate ignoring the flush read pin.
pub open spec fn wal_rotate_as_is_ignore_pin(s: WalPinState) -> WalRotateAction {
    if s.mem_empty && !s.imm_present && !s.parked_unflushed && !s.commit_inflight {
        WalRotateAction::RotateWal
    } else {
        WalRotateAction::KeepWal
    }
}

/// Executable decision — must match `pedradb_core::flush_kernel::
/// wal_rotate_decision` bit-for-bit.
#[verifier::when_used_as_spec(wal_rotate_spec)]
pub fn wal_rotate_decision(s: WalPinState) -> (a: WalRotateAction)
    ensures
        a == wal_rotate_spec(s),
        (a == WalRotateAction::RotateWal)
            ==> (s.mem_empty && !s.imm_present && !s.pin_live && !s.parked_unflushed && !s.commit_inflight),
        (s.pin_live || s.commit_inflight || s.imm_present || s.parked_unflushed || !s.mem_empty)
            ==> a == WalRotateAction::KeepWal,
{
    if s.mem_empty
        && !s.imm_present
        && !s.pin_live
        && !s.parked_unflushed
        && !s.commit_inflight
    {
        WalRotateAction::RotateWal
    } else {
        WalRotateAction::KeepWal
    }
}

/// P0.2 named lemma (crash dictionary): the mem tail is never dropped —
/// a flush with a non-empty mem always writes an SST before any rotate.
proof fn lemma_tail_never_dropped(mem_empty: bool, imm_present: bool)
    requires
        !mem_empty,
    ensures
        flush_plan(mem_empty, imm_present) != FlushPlan::RotateOnly,
        flush_plan(mem_empty, imm_present) == FlushPlan::WriteSstThenRotate
            || flush_plan(mem_empty, imm_present) == FlushPlan::FinishImmThenFlush,
{
}

/// P0.2 named lemma (single-flight): a pending imm finishes first.
proof fn lemma_pending_imm_finishes_first(mem_empty: bool)
    requires
        true,
    ensures
        flush_plan(mem_empty, true) == FlushPlan::FinishImmThenFlush,
{
}

/// P0.2 named lemma (no false refusal): an empty, idle pipeline rotates.
proof fn lemma_clean_pipeline_rotates()
    ensures
        flush_plan(true, false) == FlushPlan::RotateOnly,
        wal_rotate_decision(WalPinState {
            mem_empty: true,
            imm_present: false,
            pin_live: false,
            parked_unflushed: false,
            commit_inflight: false,
        }) == WalRotateAction::RotateWal,
{
}

/// P0.2 named lemma (pin keeps the WAL): a live flush read pin holds the
/// only copy of acked keys — the WAL is never truncated under it (the
/// pre-fix hole `Db::rotate_wal_ignoring_pin` replays).
proof fn lemma_pin_keeps_wal(s: WalPinState)
    requires
        s.pin_live,
    ensures
        wal_rotate_decision(s) == WalRotateAction::KeepWal,
{
}

/// P0.2 named lemma (commit keeps the WAL): a commit inside the off-lock
/// fsync window owns WAL bytes (F2).
proof fn lemma_commit_inflight_keeps_wal(s: WalPinState)
    requires
        s.commit_inflight,
    ensures
        wal_rotate_decision(s) == WalRotateAction::KeepWal,
{
}

/// P0.2 named lemma (mem tail keeps the WAL): unflushed acked keys in mem
/// pin the WAL even when everything else is idle.
proof fn lemma_unflushed_mem_keeps_wal(s: WalPinState)
    requires
        !s.mem_empty,
    ensures
        wal_rotate_decision(s) == WalRotateAction::KeepWal,
{
}

/// Teeth: the AS-IS lose-tail mutant answers `RotateOnly` (rotates the
/// WAL without writing the SST) exactly where the fixed kernel writes.
proof fn lemma_mutant_loses_tail(mem_empty: bool, imm_present: bool)
    requires
        !mem_empty,
    ensures
        flush_plan(mem_empty, imm_present) != FlushPlan::RotateOnly,
        flush_plan_as_is(mem_empty, imm_present) == FlushPlan::RotateOnly,
{
}

/// Teeth: the AS-IS ignore-pin mutant truncates the WAL exactly when the
/// pin is live and everything else is idle — the acked-write loss the
/// fixed kernel refuses.
pub open spec fn may_publish_manifest_spec(sst_durable: bool) -> bool {
    sst_durable
}

pub open spec fn may_publish_manifest_as_is_spec(_sst_durable: bool) -> bool {
    true
}

pub fn may_publish_manifest(sst_durable: bool) -> (d: bool)
    ensures
        d == may_publish_manifest_spec(sst_durable),
{
    sst_durable
}

pub fn may_publish_manifest_as_is(_sst_durable: bool) -> (d: bool)
    ensures
        d == true,
{
    true
}

proof fn lemma_as_is_publishes_unsynced()
    ensures
        !may_publish_manifest_spec(false),
        may_publish_manifest_as_is_spec(false),
{
}

proof fn lemma_mutant_ignores_pin()
    ensures
        wal_rotate_decision(WalPinState {
            mem_empty: true,
            imm_present: false,
            pin_live: true,
            parked_unflushed: false,
            commit_inflight: false,
        }) == WalRotateAction::KeepWal,
        wal_rotate_as_is_ignore_pin(WalPinState {
            mem_empty: true,
            imm_present: false,
            pin_live: true,
            parked_unflushed: false,
            commit_inflight: false,
        }) == WalRotateAction::RotateWal,
{
}

} // verus!
