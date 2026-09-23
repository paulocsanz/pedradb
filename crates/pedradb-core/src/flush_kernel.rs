//! Pure flush-pipeline decisions (RFC-0056 P0.2 / RFC-0174 P0.3).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin copy.
//!
//!   ./scripts/verus_flush_decision.sh
//!
//! Production [`crate::db::Db::flush`] and [`crate::db::Db::try_rotate_wal`] route
//! their decisions through this kernel. Data-fate: the WAL may only be
//! rotated once every copy of acked keys lives in an installed SST.
//!
//! Spec page: `docs/formal/crash-dictionary.md` (flush section).

#![forbid(unsafe_code)]

macro_rules! flush_plan_body {
    ($mem_empty:expr, $imm_present:expr) => {
        if $imm_present {
            FlushPlan::FinishImmThenFlush
        } else if !$mem_empty {
            FlushPlan::WriteSstThenRotate
        } else {
            FlushPlan::RotateOnly
        }
    };
}

macro_rules! flush_plan_as_is_body {
    ($mem_empty:expr, $imm_present:expr) => {{
        let _ = ($mem_empty, $imm_present);
        FlushPlan::RotateOnly
    }};
}

macro_rules! may_publish_body {
    ($sst_durable:expr) => {
        $sst_durable
    };
}

macro_rules! wal_rotate_body {
    ($s:expr) => {
        if $s.mem_empty
            && !$s.imm_present
            && !$s.pin_live
            && !$s.parked_unflushed
            && !$s.commit_inflight
        {
            WalRotateAction::RotateWal
        } else {
            WalRotateAction::KeepWal
        }
    };
}

macro_rules! wal_rotate_as_is_body {
    ($s:expr) => {
        if $s.mem_empty && !$s.imm_present && !$s.parked_unflushed && !$s.commit_inflight {
            WalRotateAction::RotateWal
        } else {
            WalRotateAction::KeepWal
        }
    };
}

macro_rules! auto_flush_due_body {
    ($mem_bytes:expr, $armed:expr, $limit:expr) => {
        $armed && $mem_bytes >= $limit
    };
}

/// L0 is due for compact EXACTLY at/above the trigger (RFC-0234 P0.1).
macro_rules! l0_compact_due_body {
    ($l0_files:expr, $trigger:expr) => {
        $l0_files >= $trigger
    };
}

/// Empty current WAL segment: rotate would only rewrite MANIFEST (idle poll).
macro_rules! wal_segment_is_empty_body {
    ($pos:expr) => {
        $pos == 0u64
    };
}

/// OCC snap uses published seq while a commit owns the WAL (lock-order).
macro_rules! occ_snap_uses_published_body {
    ($inflight:expr) => {
        $inflight
    };
}

macro_rules! occ_snap_uses_published_as_is_body {
    ($inflight:expr) => {{
        let _ = $inflight;
        false
    }};
}

/// Write-lock client: published snap if the read lock is not held (writer
/// exclusive) **or** a commit owns the WAL. Always calls the inflight
/// callee so Lean can unfold both.
macro_rules! occ_snap_lock_order_body {
    ($read_held:expr, $inflight:expr) => {{
        let inflight_pub = occ_snap_uses_published($inflight);
        !$read_held || inflight_pub
    }};
}

macro_rules! occ_snap_lock_order_as_is_body {
    ($read_held:expr, $inflight:expr) => {{
        let _ = ($read_held, $inflight);
        false
    }};
}

macro_rules! skip_auto_flush_body {
    ($global_under:expr, $cf_under:expr) => {
        $global_under && $cf_under
    };
}

/// One step of the flush pipeline (RFC-0056 P0.2).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlushPlan {
    /// A previous flush's imm is pending — finish it first (single-flight),
    /// then flush mem too if non-empty.
    FinishImmThenFlush,
    /// Mem holds the unflushed tail: switch mem → imm, write the SST,
    /// install it, then rotate the WAL if the kernel says it is safe.
    WriteSstThenRotate,
    /// Nothing to persist — rotate the WAL only if it is safe.
    RotateOnly,
}

#[cfg(not(verus_keep_ghost))]
/// Pure rule for which flush step runs.
#[must_use]
pub fn flush_plan(mem_empty: bool, imm_present: bool) -> FlushPlan {
    flush_plan_body!(mem_empty, imm_present)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS data loss: flush "succeeds" without writing the mem tail.
#[must_use]
pub fn flush_plan_as_is_lose_tail(mem_empty: bool, imm_present: bool) -> FlushPlan {
    flush_plan_as_is_body!(mem_empty, imm_present)
}

#[cfg(not(verus_keep_ghost))]
/// MANIFEST / CURRENT may name an SST only after that file is durable.
#[must_use]
pub fn may_publish_manifest(sst_durable: bool) -> bool {
    may_publish_body!(sst_durable)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: publish MANIFEST while the SST is still unsynced.
#[must_use]
pub fn may_publish_manifest_as_is(_sst_durable: bool) -> bool {
    true
}

#[cfg(not(verus_keep_ghost))]
/// Fate of the MANIFEST/CURRENT publish after the SST sync pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestPublishPlan {
    /// Every listed SST is durable — write MANIFEST + CURRENT.
    PublishManifest,
    /// Some SST is not durable — fail closed, hold the publish.
    HoldUnsyncedFailClosed,
}

#[cfg(not(verus_keep_ghost))]
/// Publish EXACTLY when every listed SST is durable
/// (may_publish_manifest stays live and proved in the body).
#[must_use]
pub fn manifest_publish_plan(sst_durable: bool) -> ManifestPublishPlan {
    if may_publish_manifest(sst_durable) {
        ManifestPublishPlan::PublishManifest
    } else {
        ManifestPublishPlan::HoldUnsyncedFailClosed
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: publishes while an SST is still unsynced (CURRENT names a
/// torn file after crash — tooth).
#[must_use]
pub fn manifest_publish_plan_as_is(_sst_durable: bool) -> ManifestPublishPlan {
    ManifestPublishPlan::PublishManifest
}

#[cfg(not(verus_keep_ghost))]
/// Fate of one column family inside the auto-flush scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfFlushPlan {
    /// CF armed and at/over its limit — flush this family now.
    FlushCfNow,
    /// CF not due — skip to the next family.
    CfNotDueSkip,
}

#[cfg(not(verus_keep_ghost))]
/// Flush the family EXACTLY when armed and at/over its limit
/// (auto_flush_due stays live and proved in the body; the scan reached
/// the family, so the axis is armed).
#[must_use]
pub fn cf_flush_plan(mem_bytes: u64, limit: u64) -> CfFlushPlan {
    if auto_flush_due(mem_bytes, true, limit) {
        CfFlushPlan::FlushCfNow
    } else {
        CfFlushPlan::CfNotDueSkip
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: skips every family (armed CFs over the limit keep growing —
/// tooth).
#[must_use]
pub fn cf_flush_plan_as_is(_mem_bytes: u64, _limit: u64) -> CfFlushPlan {
    CfFlushPlan::CfNotDueSkip
}

#[cfg(not(verus_keep_ghost))]
/// RFC-0223 P1.1: how a due family leaves the commit path under
/// `defer_auto_compact`. The O(n) per-family partition
/// (`MemTable::take_family`) under the write lock was the whole
/// in-commit "flush work" @10M (split `7f2758d4`: work 99,85%, gate
/// 58ns/commit) — staging the whole table is the O(1) swap the global
/// branch already uses, and `write_imm_l0_files` splits parked tables
/// per family at materialize time anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilyFlushMode {
    /// The due family dominates the memtable: swap the whole table to
    /// `imm` (O(1) in-commit); the host worker parks and materializes
    /// per-family L0 files.
    StageWholeMem,
    /// Small family (e.g. `lock`): partition it out in-commit; the rest
    /// of the table keeps accumulating.
    PartitionFamily,
}

/// `fam_bytes/total_bytes >= 3/4` — the dominant-family bar. Below it a
/// whole-mem stage would flush too many not-due bytes to be worth the
/// earlier commit return.
pub const DOMINANT_FAMILY_NUM: u64 = 3;
pub const DOMINANT_FAMILY_DEN: u64 = 4;

#[cfg(not(verus_keep_ghost))]
/// Stage the WHOLE memtable EXACTLY when the due family dominates it
/// (≥ 3/4 of usage): in-commit cost becomes the O(1) swap, the O(n)
/// partition moves to the worker's materialize pass.
#[must_use]
pub fn dominant_family_stage_plan(fam_bytes: u64, total_bytes: u64) -> FamilyFlushMode {
    if fam_bytes * DOMINANT_FAMILY_DEN >= total_bytes * DOMINANT_FAMILY_NUM {
        FamilyFlushMode::StageWholeMem
    } else {
        FamilyFlushMode::PartitionFamily
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: always partitions the family out in-commit (the O(n)
/// `take_family` under the write lock — tooth).
#[must_use]
pub fn dominant_family_stage_plan_as_is(_fam_bytes: u64, _total_bytes: u64) -> FamilyFlushMode {
    FamilyFlushMode::PartitionFamily
}

#[cfg(not(verus_keep_ghost))]
/// RFC-0219 P2.2: which flush-pipeline regime a submit/park/assist
/// decision is in — decided by whether a host flush worker is attached.
/// The five concurrent.rs trampoline gates (`await_flush_debt`,
/// `await_l0_park`, `submit_one`, `submit_inner`, `assist_flush_debt`)
/// match this plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlusherGate {
    /// No worker: parking could only hang — workerless paths keep the
    /// honest admission error / lone fast path.
    Workerless,
    /// Worker attached: park/retry/assist flows are bounded by the drain.
    WorkerDrains,
}

#[cfg(not(verus_keep_ghost))]
/// Park/assist EXACTLY when a host flush worker is attached to drain
/// the debt/stall the writer would sleep on.
#[must_use]
pub fn flusher_gate_plan(attached: bool) -> FlusherGate {
    if attached {
        FlusherGate::WorkerDrains
    } else {
        FlusherGate::Workerless
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: treats a workerless Db as drained — writers park on debt/stall
/// with nobody to drain them (unbounded sleep — tooth plantado).
#[must_use]
pub fn flusher_gate_plan_as_is(_attached: bool) -> FlusherGate {
    FlusherGate::WorkerDrains
}

#[cfg(not(verus_keep_ghost))]
/// RFC-0219 P2.2: whether parked-unflushed bytes are real debt —
/// at/above one table's worth (`flush_debt_cap`) a writer must
/// throttle. The trampolines `concurrent.rs await_flush_debt` and
/// `assist_flush_debt` match this plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkedDebtPlan {
    /// Parked bytes at/above the cap — park (bounded) / assist
    /// (materialize one table inline).
    DebtAtCap,
    /// Below the cap — no debt to drain; proceed with the submit.
    NoDebtBelowCap,
}

#[cfg(not(verus_keep_ghost))]
/// Debt EXACTLY when parked-unflushed bytes reach one table's worth.
#[must_use]
pub fn parked_debt_plan(parked: u64, cap: u64) -> ParkedDebtPlan {
    if parked < cap {
        ParkedDebtPlan::NoDebtBelowCap
    } else {
        ParkedDebtPlan::DebtAtCap
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never throttles — a lone fast writer parks tables faster than
/// the worker materializes them and the mem layer grows without bound
/// (the 25M slipstream OOM, v11–v15 — tooth plantado).
#[must_use]
pub fn parked_debt_plan_as_is(_parked: u64, _cap: u64) -> ParkedDebtPlan {
    ParkedDebtPlan::NoDebtBelowCap
}

/// Every way acked keys can still depend on the WAL.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WalPinState {
    /// Active memtable empty (empty ⇒ nothing depends on the WAL).
    pub mem_empty: bool,
    /// An imm is still being flushed (its SST is not installed yet).
    pub imm_present: bool,
    /// Off-lock flush read pin live — may hold the only copy of acked keys.
    pub pin_live: bool,
    /// Parked-unflushed tables (host pipeline backlog).
    pub parked_unflushed: bool,
    /// A commit is inside the off-lock fsync window (owns WAL bytes, F2).
    pub commit_inflight: bool,
}

/// Whether the flush pipeline may truncate the WAL.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WalRotateAction {
    /// Every copy of acked keys lives in an installed SST: the WAL may be
    /// recreated empty.
    RotateWal,
    /// Something still depends on the WAL: keep it, retry later.
    KeepWal,
}

#[cfg(not(verus_keep_ghost))]
/// Pure rule for the WAL rotate (G1 tail of the flush pipeline).
#[must_use]
pub fn wal_rotate_decision(s: WalPinState) -> WalRotateAction {
    wal_rotate_body!(s)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS pre-fix hole: decide the rotate ignoring the flush read pin.
#[must_use]
pub fn wal_rotate_decision_as_is_ignore_pin(s: WalPinState) -> WalRotateAction {
    wal_rotate_as_is_body!(s)
}

#[cfg(not(verus_keep_ghost))]
/// Lock-order client: OCC snapshot must not take `last_sequence` while a
/// group is in the off-lock fd window (`commit_inflight`). That seq is
/// unapplied; a snap equal to it misses the write and skips OCC conflict.
#[must_use]
pub fn occ_snap_uses_published(commit_inflight: bool) -> bool {
    occ_snap_uses_published_body!(commit_inflight)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: always last_sequence (TOCTOU vs unapplied group).
#[must_use]
pub fn occ_snap_uses_published_as_is(_commit_inflight: bool) -> bool {
    occ_snap_uses_published_as_is_body!(_commit_inflight)
}

#[cfg(not(verus_keep_ghost))]
/// Write-lock client protocol: OCC snap uses published seq when the read
/// lock is not held (writer exclusive) **or** `commit_inflight`.
/// `ConcurrentDb::occ_snapshot` matches this — not an inline nest of
/// `try_read` / inflight. Calls [`occ_snap_uses_published`].
#[must_use]
pub fn occ_snap_lock_order(read_held: bool, inflight: bool) -> bool {
    occ_snap_lock_order_body!(read_held, inflight)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: last_sequence even when the write lock is held (TOCTOU).
#[must_use]
pub fn occ_snap_lock_order_as_is(_read_held: bool, _inflight: bool) -> bool {
    occ_snap_lock_order_as_is_body!(_read_held, _inflight)
}

#[cfg(not(verus_keep_ghost))]
/// Fire auto-flush when the armed byte limit is reached (RFC-0170 P2.4).
#[must_use]
pub fn auto_flush_due(mem_bytes: u64, armed: bool, limit: u64) -> bool {
    auto_flush_due_body!(mem_bytes, armed, limit)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never auto-flush.
#[must_use]
pub fn auto_flush_due_as_is(_mem_bytes: u64, _armed: bool, _limit: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Compact L0 EXACTLY when the live file count is at/above the trigger
/// (RFC-0234 P0.1). The trampolines `maybe_auto_compact` and
/// `maybe_compact_l0_at_trigger` match this — not a raw `>=`.
#[must_use]
pub fn l0_compact_due(l0_files: u64, trigger: u64) -> bool {
    l0_compact_due_body!(l0_files, trigger)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never compact L0 (the park of today — seed piles 54–773 files).
#[must_use]
pub fn l0_compact_due_as_is(_l0_files: u64, _trigger: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Current WAL segment has no framed payload — rotate is a no-op rewrite.
#[must_use]
pub fn wal_segment_is_empty(pos: u64) -> bool {
    wal_segment_is_empty_body!(pos)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never skip (idle poll rotates empty, two fdatasyncs per tick).
#[must_use]
pub fn wal_segment_is_empty_as_is(_pos: u64) -> bool {
    false
}

#[cfg(not(verus_keep_ghost))]
/// Both mem axes under their limits ⇒ skip auto-flush (no SST write).
#[must_use]
pub fn skip_auto_flush(global_under: bool, cf_under: bool) -> bool {
    skip_auto_flush_body!(global_under, cf_under)
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never skip — would flush even when both axes are under.
#[must_use]
pub fn skip_auto_flush_as_is(_global_under: bool, _cf_under: bool) -> bool {
    false
}

/// RFC-0219 P1.3: whether the parked-unflushed queue can hand out its
/// two oldest tables as a fold pair (F174: the pair is validated again
/// at swap time). The trampoline `db.rs parked_oldest_pair_arcs`
/// matches this plan.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParkedPairPlan {
    /// Fewer than two parked tables — nothing to fold yet.
    WaitForPair,
    /// Two or more parked — hand out the two oldest as the fold pair.
    HandOutOldestPair,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn parked_pair_plan(parked_len: u64) -> ParkedPairPlan {
    if parked_len < 2 {
        ParkedPairPlan::WaitForPair
    } else {
        ParkedPairPlan::HandOutOldestPair
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: hand out regardless — a queue shorter than the pair loses or
/// mangles the single parked table (parked-pipeline data-loss tooth).
#[must_use]
pub fn parked_pair_plan_as_is(_parked_len: u64) -> ParkedPairPlan {
    ParkedPairPlan::HandOutOldestPair
}

/// RFC-0219 P1.3: whether `maybe_auto_flush` scans the column families
/// at all. Both mem axes under their limits ⇒ nothing due anywhere —
/// skip the scan; anything over ⇒ scan (the per-CF due gate applies).
/// Calls [`skip_auto_flush`].
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutoFlushGate {
    /// Both axes under — nothing due anywhere, skip the whole scan.
    SkipAllNotDue,
    /// At least one axis over — scan each CF for its own due gate.
    ScanColumnFamilies,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn auto_flush_gate(global_under: bool, cf_under: bool) -> AutoFlushGate {
    if skip_auto_flush(global_under, cf_under) {
        AutoFlushGate::SkipAllNotDue
    } else {
        AutoFlushGate::ScanColumnFamilies
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never skip the scan — SST writes fire even when both axes are
/// under (pointless flush churn tooth).
#[must_use]
pub fn auto_flush_gate_as_is(_global_under: bool, _cf_under: bool) -> AutoFlushGate {
    AutoFlushGate::ScanColumnFamilies
}

/// RFC-0219 P1.3: whether the mem-level auto-flush fires now. Armed and
/// at/over the limit ⇒ flush (or stage `imm` for the host worker); else
/// keep accumulating. Calls [`auto_flush_due`].
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemAutoFlushPlan {
    /// Armed and at/over the armed limit — flush the memtable now.
    FlushMemNow,
    /// Not due (unarmed or under the limit) — keep accumulating.
    NotDueKeepMem,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn mem_auto_flush_plan(mem_bytes: u64, armed: bool, limit: u64) -> MemAutoFlushPlan {
    if auto_flush_due(mem_bytes, armed, limit) {
        MemAutoFlushPlan::FlushMemNow
    } else {
        MemAutoFlushPlan::NotDueKeepMem
    }
}

#[cfg(not(verus_keep_ghost))]
/// AS-IS: never flush — the armed limit is ignored and the memtable
/// grows until the host stalls (unbounded-mem tooth).
#[must_use]
pub fn mem_auto_flush_plan_as_is(_mem_bytes: u64, _armed: bool, _limit: u64) -> MemAutoFlushPlan {
    MemAutoFlushPlan::NotDueKeepMem
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub enum FlushPlan {
    FinishImmThenFlush,
    WriteSstThenRotate,
    RotateOnly,
}

pub open spec fn flush_plan_spec(mem_empty: bool, imm_present: bool) -> FlushPlan {
    if imm_present {
        FlushPlan::FinishImmThenFlush
    } else if !mem_empty {
        FlushPlan::WriteSstThenRotate
    } else {
        FlushPlan::RotateOnly
    }
}

pub open spec fn flush_plan_as_is(_mem_empty: bool, _imm_present: bool) -> FlushPlan {
    FlushPlan::RotateOnly
}

#[verifier::when_used_as_spec(flush_plan_spec)]
pub fn flush_plan(mem_empty: bool, imm_present: bool) -> (p: FlushPlan)
    ensures
        p == flush_plan_spec(mem_empty, imm_present),
        (p == FlushPlan::RotateOnly) ==> (mem_empty && !imm_present),
        !mem_empty ==> p != FlushPlan::RotateOnly,
        imm_present ==> p == FlushPlan::FinishImmThenFlush,
{
    flush_plan_body!(mem_empty, imm_present)
}

pub fn flush_plan_as_is_lose_tail(mem_empty: bool, imm_present: bool) -> (p: FlushPlan)
    ensures
        p == flush_plan_as_is(mem_empty, imm_present),
{
    flush_plan_as_is_body!(mem_empty, imm_present)
}

pub struct WalPinState {
    pub mem_empty: bool,
    pub imm_present: bool,
    pub pin_live: bool,
    pub parked_unflushed: bool,
    pub commit_inflight: bool,
}

pub enum WalRotateAction {
    RotateWal,
    KeepWal,
}

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

pub open spec fn wal_rotate_as_is_ignore_pin(s: WalPinState) -> WalRotateAction {
    if s.mem_empty && !s.imm_present && !s.parked_unflushed && !s.commit_inflight {
        WalRotateAction::RotateWal
    } else {
        WalRotateAction::KeepWal
    }
}

#[verifier::when_used_as_spec(wal_rotate_spec)]
pub fn wal_rotate_decision(s: WalPinState) -> (a: WalRotateAction)
    ensures
        a == wal_rotate_spec(s),
        (a == WalRotateAction::RotateWal)
            ==> (s.mem_empty && !s.imm_present && !s.pin_live && !s.parked_unflushed && !s.commit_inflight),
        (s.pin_live || s.commit_inflight || s.imm_present || s.parked_unflushed || !s.mem_empty)
            ==> a == WalRotateAction::KeepWal,
{
    wal_rotate_body!(s)
}

pub fn wal_rotate_decision_as_is_ignore_pin(s: WalPinState) -> (a: WalRotateAction)
    ensures
        a == wal_rotate_as_is_ignore_pin(s),
{
    wal_rotate_as_is_body!(s)
}

pub open spec fn occ_snap_uses_published_spec(commit_inflight: bool) -> bool {
    commit_inflight
}

pub fn occ_snap_uses_published(commit_inflight: bool) -> (d: bool)
    ensures
        d == occ_snap_uses_published_spec(commit_inflight),
        d == commit_inflight,
{
    occ_snap_uses_published_body!(commit_inflight)
}

pub fn occ_snap_uses_published_as_is(commit_inflight: bool) -> (d: bool)
    ensures
        d == false,
{
    occ_snap_uses_published_as_is_body!(commit_inflight)
}

pub open spec fn occ_snap_lock_order_spec(read_held: bool, inflight: bool) -> bool {
    !read_held || occ_snap_uses_published_spec(inflight)
}

pub fn occ_snap_lock_order(read_held: bool, inflight: bool) -> (d: bool)
    ensures
        d == occ_snap_lock_order_spec(read_held, inflight),
        !read_held ==> d,
        read_held ==> d == occ_snap_uses_published_spec(inflight),
{
    occ_snap_lock_order_body!(read_held, inflight)
}

pub fn occ_snap_lock_order_as_is(_read_held: bool, _inflight: bool) -> (d: bool)
    ensures
        d == false,
{
    occ_snap_lock_order_as_is_body!(_read_held, _inflight)
}

proof fn lemma_write_held_snap_published(inflight: bool)
    ensures
        occ_snap_lock_order_spec(false, inflight),
{
}

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
    may_publish_body!(sst_durable)
}

pub fn may_publish_manifest_as_is(_sst_durable: bool) -> (d: bool)
    ensures
        d == true,
{
    true
}

pub open spec fn auto_flush_due_spec(mem_bytes: u64, armed: bool, limit: u64) -> bool {
    armed && mem_bytes >= limit
}

pub fn auto_flush_due(mem_bytes: u64, armed: bool, limit: u64) -> (d: bool)
    ensures
        d == auto_flush_due_spec(mem_bytes, armed, limit),
{
    auto_flush_due_body!(mem_bytes, armed, limit)
}

pub open spec fn auto_flush_due_as_is_spec(_mem_bytes: u64, _armed: bool, _limit: u64) -> bool {
    false
}

pub fn auto_flush_due_as_is(mem_bytes: u64, armed: bool, limit: u64) -> (d: bool)
    ensures
        d == auto_flush_due_as_is_spec(mem_bytes, armed, limit),
        d == false,
{
    let _ = (mem_bytes, armed, limit);
    false
}

pub open spec fn l0_compact_due_spec(l0_files: u64, trigger: u64) -> bool {
    l0_files >= trigger
}

pub fn l0_compact_due(l0_files: u64, trigger: u64) -> (d: bool)
    ensures
        d == l0_compact_due_spec(l0_files, trigger),
{
    l0_compact_due_body!(l0_files, trigger)
}

pub open spec fn l0_compact_due_as_is_spec(_l0_files: u64, _trigger: u64) -> bool {
    false
}

pub fn l0_compact_due_as_is(l0_files: u64, trigger: u64) -> (d: bool)
    ensures
        d == l0_compact_due_as_is_spec(l0_files, trigger),
        d == false,
{
    let _ = (l0_files, trigger);
    false
}

pub open spec fn wal_segment_is_empty_spec(pos: u64) -> bool {
    pos == 0
}

pub fn wal_segment_is_empty(pos: u64) -> (d: bool)
    ensures
        d == wal_segment_is_empty_spec(pos),
{
    wal_segment_is_empty_body!(pos)
}

pub fn wal_segment_is_empty_as_is(pos: u64) -> (d: bool)
    ensures
        d == false,
{
    let _ = pos;
    false
}

pub open spec fn skip_auto_flush_spec(global_under: bool, cf_under: bool) -> bool {
    global_under && cf_under
}

pub fn skip_auto_flush(global_under: bool, cf_under: bool) -> (d: bool)
    ensures
        d == skip_auto_flush_spec(global_under, cf_under),
{
    skip_auto_flush_body!(global_under, cf_under)
}

pub open spec fn skip_auto_flush_as_is_spec(_global_under: bool, _cf_under: bool) -> bool {
    false
}

pub fn skip_auto_flush_as_is(global_under: bool, cf_under: bool) -> (d: bool)
    ensures
        d == skip_auto_flush_as_is_spec(global_under, cf_under),
        d == false,
{
    let _ = (global_under, cf_under);
    false
}

proof fn lemma_tail_never_dropped(mem_empty: bool, imm_present: bool)
    requires
        !mem_empty,
    ensures
        flush_plan(mem_empty, imm_present) != FlushPlan::RotateOnly,
        flush_plan(mem_empty, imm_present) == FlushPlan::WriteSstThenRotate
            || flush_plan(mem_empty, imm_present) == FlushPlan::FinishImmThenFlush,
{
}

proof fn lemma_pending_imm_finishes_first(mem_empty: bool)
    requires
        true,
    ensures
        flush_plan(mem_empty, true) == FlushPlan::FinishImmThenFlush,
{
}

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

proof fn lemma_pin_keeps_wal(s: WalPinState)
    requires
        s.pin_live,
    ensures
        wal_rotate_decision(s) == WalRotateAction::KeepWal,
{
}

proof fn lemma_commit_inflight_keeps_wal(s: WalPinState)
    requires
        s.commit_inflight,
    ensures
        wal_rotate_decision(s) == WalRotateAction::KeepWal,
{
}

proof fn lemma_unflushed_mem_keeps_wal(s: WalPinState)
    requires
        !s.mem_empty,
    ensures
        wal_rotate_decision(s) == WalRotateAction::KeepWal,
{
}

proof fn lemma_mutant_loses_tail(mem_empty: bool, imm_present: bool)
    requires
        !mem_empty,
    ensures
        flush_plan(mem_empty, imm_present) != FlushPlan::RotateOnly,
        flush_plan_as_is(mem_empty, imm_present) == FlushPlan::RotateOnly,
{
}

proof fn lemma_as_is_publishes_unsynced()
    ensures
        !may_publish_manifest_spec(false),
        may_publish_manifest_as_is_spec(false),
{
}

proof fn lemma_as_is_never_auto_flushes()
    ensures
        auto_flush_due_spec(100, true, 50),
        !auto_flush_due_as_is_spec(100, true, 50),
{
}

proof fn lemma_as_is_never_compacts_l0()
    ensures
        l0_compact_due_spec(4, 4),
        !l0_compact_due_as_is_spec(4, 4),
{
}

proof fn lemma_as_is_never_skips_auto_flush()
    ensures
        skip_auto_flush_spec(true, true),
        !skip_auto_flush_as_is_spec(true, true),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_always_written() {
        assert_eq!(flush_plan(false, false), FlushPlan::WriteSstThenRotate);
        assert_eq!(flush_plan(false, true), FlushPlan::FinishImmThenFlush);
    }

    #[test]
    fn empty_flush_rotates_only() {
        assert_eq!(flush_plan(true, false), FlushPlan::RotateOnly);
        assert_eq!(flush_plan(true, true), FlushPlan::FinishImmThenFlush);
    }

    #[test]
    fn clean_pipeline_rotates_wal() {
        let a = wal_rotate_decision(WalPinState {
            mem_empty: true,
            imm_present: false,
            pin_live: false,
            parked_unflushed: false,
            commit_inflight: false,
        });
        assert_eq!(a, WalRotateAction::RotateWal);
    }

    #[test]
    fn every_live_dependency_keeps_wal() {
        for field in 0..5 {
            let mut s = WalPinState {
                mem_empty: true,
                imm_present: false,
                pin_live: false,
                parked_unflushed: false,
                commit_inflight: false,
            };
            match field {
                0 => s.mem_empty = false,
                1 => s.imm_present = true,
                2 => s.pin_live = true,
                3 => s.parked_unflushed = true,
                _ => s.commit_inflight = true,
            }
            assert_eq!(
                wal_rotate_decision(s),
                WalRotateAction::KeepWal,
                "field {field} live must keep the WAL"
            );
        }
    }

    /// Finite-domain theorem (2×2): the mem tail is never dropped, the
    /// pending imm finishes first, and the AS-IS lose-tail mutant answers
    /// `RotateOnly` exactly where the fixed kernel writes an SST.
    #[test]
    fn theorem_flush_plan_on_finite_domain() {
        for mem_empty in [false, true] {
            for imm_present in [false, true] {
                let p = flush_plan(mem_empty, imm_present);
                if p == FlushPlan::RotateOnly {
                    assert!(mem_empty && !imm_present, "rotate-only needs empty state");
                }
                if !mem_empty {
                    assert_ne!(p, FlushPlan::RotateOnly, "tail never dropped");
                    let m = flush_plan_as_is_lose_tail(mem_empty, imm_present);
                    assert_eq!(m, FlushPlan::RotateOnly, "AS-IS must drop the tail");
                    assert_ne!(m, p, "mutant must differ from fixed on non-empty mem");
                }
                if imm_present {
                    assert_eq!(p, FlushPlan::FinishImmThenFlush, "single-flight");
                }
            }
        }
    }

    /// Finite-domain theorem (2⁵): the WAL rotates exactly when nothing
    /// depends on it; the AS-IS ignore-pin mutant truncates exactly when
    /// the pin is live (the pre-fix hole).
    #[test]
    fn theorem_wal_rotate_on_finite_domain() {
        for bits in 0u8..32 {
            let s = WalPinState {
                mem_empty: bits & 1 == 0,
                imm_present: bits & 2 != 0,
                pin_live: bits & 4 != 0,
                parked_unflushed: bits & 8 != 0,
                commit_inflight: bits & 16 != 0,
            };
            let a = wal_rotate_decision(s);
            let clean = s.mem_empty
                && !s.imm_present
                && !s.pin_live
                && !s.parked_unflushed
                && !s.commit_inflight;
            assert_eq!(a == WalRotateAction::RotateWal, clean, "rotate iff clean");
            let m = wal_rotate_decision_as_is_ignore_pin(s);
            if s.pin_live
                && s.mem_empty
                && !s.imm_present
                && !s.parked_unflushed
                && !s.commit_inflight
            {
                assert_eq!(
                    m,
                    WalRotateAction::RotateWal,
                    "AS-IS must truncate with the pin live"
                );
                assert_eq!(a, WalRotateAction::KeepWal);
                assert_ne!(m, a, "mutant must differ from fixed when pin is live");
            }
        }
    }

    #[test]
    fn may_publish_manifest_on_live_unsynced_sst_is_not_ok() {
        assert!(!may_publish_manifest(false));
        assert!(
            may_publish_manifest_as_is(false),
            "AS-IS tooth: MANIFEST names unsynced SST"
        );
        assert!(may_publish_manifest(true));
    }

    #[test]
    fn manifest_publish_plan_on_live_unsynced_sst_holds() {
        // RFC-0219 P1.4: unsynced SST holds the MANIFEST publish
        // fail-closed; AS-IS publishes (CURRENT names a torn file).
        assert_eq!(
            manifest_publish_plan(true),
            ManifestPublishPlan::PublishManifest
        );
        assert_eq!(
            manifest_publish_plan(false),
            ManifestPublishPlan::HoldUnsyncedFailClosed
        );
        assert_eq!(
            manifest_publish_plan_as_is(false),
            ManifestPublishPlan::PublishManifest,
            "AS-IS tooth: publishes with unsynced SST"
        );
        let pm = named_fn_src(include_str!("db_kernel.rs"), "persist_manifest")
            .expect("persist_manifest");
        assert!(
            pm.contains("match crate::flush_kernel::manifest_publish_plan("),
            "persist_manifest must match manifest_publish_plan"
        );
        assert!(
            !pm.contains("may_publish_manifest("),
            "the raw publish gate left the trampoline"
        );
    }

    #[test]
    fn trampoline_drains_match_kernel_plans() {
        // RFC-0219 P2.1 drains: the already-paired kernel decisions
        // leave the `if` shape — the trampoline matches the kernel.
        let trw =
            named_fn_src(include_str!("db_kernel.rs"), "try_rotate_wal").expect("try_rotate_wal");
        assert!(
            trw.contains("match crate::flush_kernel::wal_rotate_decision("),
            "try_rotate_wal matches wal_rotate_decision"
        );
        assert!(
            trw.contains("match crate::flush_kernel::wal_segment_is_empty("),
            "try_rotate_wal matches wal_segment_is_empty"
        );
        let ewr = named_fn_src(include_str!("db_kernel.rs"), "ensure_wal_rotated_for_gc")
            .expect("ensure_wal_rotated_for_gc");
        assert!(
            ewr.contains("match crate::flush_kernel::wal_rotate_decision("),
            "ensure_wal_rotated_for_gc matches wal_rotate_decision"
        );
        let cv =
            named_fn_src(include_str!("db_kernel.rs"), "count_visible").expect("count_visible");
        assert_eq!(
            cv.matches("match visible").count(),
            2,
            "both scan-count sites match the visible_at result"
        );
    }

    #[test]
    fn flusher_gate_plan_on_live_workerless_parks_nowhere() {
        // RFC-0219 P2.2: park/assist/workerless-submit regimes are decided
        // by flusher_gate_plan — a workerless Db never sleeps on a drain
        // nobody runs. AS-IS says WorkerDrains always (workerless writers
        // park forever — tooth).
        assert_eq!(flusher_gate_plan(true), FlusherGate::WorkerDrains);
        assert_eq!(flusher_gate_plan(false), FlusherGate::Workerless);
        assert_eq!(
            flusher_gate_plan_as_is(false),
            FlusherGate::WorkerDrains,
            "AS-IS tooth: workerless Db parks on a drain nobody runs"
        );
        let cc = include_str!("concurrent_kernel.rs");
        for (name, field) in [
            ("await_flush_debt", "self"),
            ("await_l0_park", "self"),
            ("submit_one", "self"),
            ("submit_inner", "self"),
            ("assist_flush_debt", "self.writes"),
        ] {
            let body = named_fn_src(cc, name).unwrap_or_else(|| panic!("{name}"));
            assert!(
                body.contains("match crate::flush_kernel::flusher_gate_plan("),
                "{name} must match flusher_gate_plan"
            );
            assert!(
                !body.contains(&format!("if !{field}.flusher_attached.load")),
                "{name}: the raw worker gate left the trampoline"
            );
        }
    }

    #[test]
    fn parked_debt_plan_on_live_at_cap_parks() {
        // RFC-0219 P2.2: debt is real EXACTLY when parked-unflushed bytes
        // reach one table's worth — the writer throttles (park/assist).
        // AS-IS never throttles (mem layer grows without bound — the 25M
        // slipstream OOM — tooth).
        assert_eq!(parked_debt_plan(255, 256), ParkedDebtPlan::NoDebtBelowCap);
        assert_eq!(parked_debt_plan(256, 256), ParkedDebtPlan::DebtAtCap);
        assert_eq!(parked_debt_plan(1 << 30, 256), ParkedDebtPlan::DebtAtCap);
        assert_eq!(
            parked_debt_plan_as_is(1 << 30, 256),
            ParkedDebtPlan::NoDebtBelowCap,
            "AS-IS tooth: a table's worth of parked debt never throttles"
        );
        let cc = include_str!("concurrent_kernel.rs");
        let afd = named_fn_src(cc, "await_flush_debt").expect("await_flush_debt");
        assert!(
            afd.contains("match crate::flush_kernel::parked_debt_plan("),
            "await_flush_debt must match parked_debt_plan"
        );
        let assist = named_fn_src(cc, "assist_flush_debt").expect("assist_flush_debt");
        assert!(
            assist.contains("match crate::flush_kernel::parked_debt_plan("),
            "assist_flush_debt must match parked_debt_plan"
        );
        assert!(
            !assist.contains("parked_unflushed_bytes() < cap"),
            "the raw debt compare left the trampoline"
        );
    }

    #[test]
    fn trampoline_drains_p22_match_kernel_plans() {
        // RFC-0219 P2.2 drains: the already-paired kernel decisions
        // leave the `if` shape in concurrent.rs — the trampoline matches
        // the kernel (or its plan).
        let cc = include_str!("concurrent_kernel.rs");
        let fate_drains = [
            ("submit_after_begin", "lone/async 3-way"),
            ("lead", "catchup bound"),
            ("finish_group_off_lock", "wal-sync note + ledger barrier"),
        ];
        for (name, what) in fate_drains {
            let body = named_fn_src(cc, name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(
                body.matches("match crate::changelog_kernel::changelog_durable_commit_fate(")
                    .count(),
                if name == "finish_group_off_lock" {
                    2
                } else {
                    1
                },
                "{name} ({what}) must match changelog_durable_commit_fate"
            );
        }
        let occ = named_fn_src(cc, "occ_snapshot").expect("occ_snapshot");
        assert!(
            occ.contains("match crate::flush_kernel::occ_snap_lock_order("),
            "occ_snapshot matches occ_snap_lock_order"
        );
        let idle = named_fn_src(cc, "writes_idle_for").expect("writes_idle_for");
        assert!(
            idle.contains("match crate::flush_kernel::occ_snap_uses_published("),
            "writes_idle_for matches occ_snap_uses_published"
        );
        let rec = named_fn_src(cc, "recover_from_fence").expect("recover_from_fence");
        assert!(
            rec.contains("match crate::write_admission_kernel::fence_admission_plan("),
            "recover_from_fence matches fence_admission_plan"
        );
        for name in ["persist_unsynced_l0s_off_lock", "install_prepared_one"] {
            let body = named_fn_src(cc, name).unwrap_or_else(|| panic!("{name}"));
            assert!(
                body.contains("match crate::flush_kernel::manifest_publish_plan("),
                "{name} matches manifest_publish_plan"
            );
            assert!(
                !body.contains("may_publish_manifest("),
                "{name}: the raw publish gate left the trampoline"
            );
        }
    }

    #[test]
    fn cf_flush_plan_on_live_over_limit_flushes() {
        // RFC-0219 P2.1: inside the armed scan, a family at/over its
        // limit flushes now; below the limit skips. AS-IS skips every
        // family (armed CFs keep growing — tooth).
        assert_eq!(cf_flush_plan(10, 10), CfFlushPlan::FlushCfNow);
        assert_eq!(cf_flush_plan(11, 10), CfFlushPlan::FlushCfNow);
        assert_eq!(cf_flush_plan(9, 10), CfFlushPlan::CfNotDueSkip);
        assert_eq!(
            cf_flush_plan_as_is(10, 10),
            CfFlushPlan::CfNotDueSkip,
            "AS-IS tooth: armed family over the limit never flushes"
        );
        let maf = named_fn_src(include_str!("db_kernel.rs"), "maybe_auto_flush")
            .expect("maybe_auto_flush");
        assert!(
            maf.contains("match crate::flush_kernel::cf_flush_plan("),
            "maybe_auto_flush must match cf_flush_plan"
        );
        assert_eq!(
            maf.matches("auto_flush_due(").count(),
            2,
            "only the two axis probes remain (global_under/cf_under feeding auto_flush_gate); the per-CF gate is the kernel match"
        );
    }

    #[test]
    fn occ_snap_uses_published_on_live_inflight_is_not_ok() {
        assert!(occ_snap_uses_published(true));
        assert!(!occ_snap_uses_published(false));
        assert!(
            !occ_snap_uses_published_as_is(true),
            "AS-IS tooth: last_seq while inflight"
        );
        let src = include_str!("concurrent_kernel.rs");
        assert!(
            src.contains("occ_snap_uses_published("),
            "occ_snapshot must match occ_snap_uses_published"
        );
        assert!(
            src.split("fn writes_idle_for")
                .nth(1)
                .expect("writes_idle_for")
                .contains("occ_snap_uses_published("),
            "writes_idle_for must not treat inflight as idle"
        );
    }

    #[test]
    fn occ_snap_lock_order_on_write_held_is_not_ok() {
        assert!(
            occ_snap_lock_order(false, false),
            "write lock held, idle pipeline ⇒ published"
        );
        assert!(occ_snap_lock_order(false, true));
        assert!(occ_snap_lock_order(true, true));
        assert!(
            !occ_snap_lock_order(true, false),
            "read lock held, idle ⇒ last_seq"
        );
        assert!(
            !occ_snap_lock_order_as_is(false, true),
            "AS-IS tooth: last_seq while write lock held"
        );
        let snap = include_str!("concurrent_kernel.rs")
            .split("fn occ_snapshot(")
            .nth(1)
            .expect("occ_snapshot");
        assert!(
            snap.contains("occ_snap_lock_order("),
            "occ_snapshot must match occ_snap_lock_order"
        );
    }

    #[test]
    fn wal_rotate_decision_on_live_pin_is_not_ok() {
        let s = WalPinState {
            mem_empty: true,
            imm_present: false,
            pin_live: true,
            parked_unflushed: false,
            commit_inflight: false,
        };
        assert_eq!(wal_rotate_decision(s), WalRotateAction::KeepWal);
        assert_eq!(
            wal_rotate_decision_as_is_ignore_pin(s),
            WalRotateAction::RotateWal,
            "AS-IS tooth: rotate while pin live"
        );
    }

    #[test]
    fn wal_segment_is_empty_on_live_zero_is_not_ok() {
        assert!(wal_segment_is_empty(0));
        assert!(
            !wal_segment_is_empty_as_is(0),
            "AS-IS tooth: rotate empty segment"
        );
        assert!(!wal_segment_is_empty(1));
        let rot = include_str!("db_kernel.rs")
            .split("fn try_rotate_wal(&mut self)")
            .nth(1)
            .and_then(|s| s.split("fn wal_pin_state").next())
            .expect("try_rotate_wal");
        assert!(
            rot.contains("wal_segment_is_empty("),
            "try_rotate_wal must match wal_segment_is_empty"
        );
        assert!(
            rot.contains("wal_rotate_decision("),
            "try_rotate_wal must match wal_rotate_decision"
        );
        let pin = include_str!("db_kernel.rs")
            .split("fn wal_pin_state(")
            .nth(1)
            .and_then(|s| s.split("fn ensure_wal_rotated_for_gc").next())
            .expect("wal_pin_state");
        assert!(
            pin.contains("commit_inflight:"),
            "wal_pin_state feeds commit_inflight into wal_rotate_decision"
        );
        let flush_th = include_str!("../../../formal/aeneas/lean/Flush.lean");
        assert!(
            flush_th.contains("unfold wal_rotate_decision")
                && flush_th.contains("unfold wal_segment_is_empty"),
            "Flush.lean must dual-unfold plan and wal_segment_is_empty"
        );
    }

    #[test]
    fn auto_flush_due_on_live_over_limit_is_not_ok() {
        assert!(auto_flush_due(100, true, 50));
        assert!(
            !auto_flush_due_as_is(100, true, 50),
            "AS-IS tooth: never fires"
        );
        assert!(!auto_flush_due(10, true, 50));
        assert!(!auto_flush_due(100, false, 50), "unarmed never fires");
    }

    #[test]
    fn l0_compact_due_on_live_at_trigger_is_not_ok() {
        // RFC-0234 P0.1: L0 compact is due EXACTLY at/above the trigger.
        // AS-IS never fires (the park that left 54–773 L0 after a 10M seed).
        assert!(l0_compact_due(4, 4));
        assert!(l0_compact_due(5, 4));
        assert!(!l0_compact_due(3, 4));
        assert!(
            !l0_compact_due_as_is(4, 4),
            "AS-IS tooth: never compacta L0"
        );
        assert!(!l0_compact_due_as_is(773, 4));
        let mac = named_fn_src(include_str!("db_kernel.rs"), "maybe_auto_compact")
            .expect("maybe_auto_compact");
        assert!(
            mac.contains("crate::flush_kernel::l0_compact_due("),
            "maybe_auto_compact must match l0_compact_due"
        );
        let cc = include_str!("concurrent_kernel.rs");
        let mct =
            named_fn_src(cc, "maybe_compact_l0_at_trigger").expect("maybe_compact_l0_at_trigger");
        assert!(
            mct.contains("crate::flush_kernel::l0_compact_due("),
            "maybe_compact_l0_at_trigger must match l0_compact_due"
        );
        let drain = named_fn_src(cc, "drain_l0_below_trigger").expect("drain_l0_below_trigger");
        assert!(
            drain.contains("crate::flush_kernel::l0_compact_due("),
            "drain_l0_below_trigger must match l0_compact_due"
        );
        let compat = include_str!("../../../crates/rocksdb-compat/src/lib_kernel.rs");
        let spawn = named_fn_src(compat, "spawn_compact_worker").expect("spawn_compact_worker");
        assert!(
            spawn.contains("l0_compact_due("),
            "compat compact worker must match l0_compact_due"
        );
    }

    #[test]
    fn skip_auto_flush_on_live_both_under_is_not_ok() {
        assert!(skip_auto_flush(true, true));
        assert!(!skip_auto_flush_as_is(true, true));
        assert!(!skip_auto_flush(true, false));
        assert!(!skip_auto_flush(false, true));
    }

    /// Balanced-brace slice of one `fn` from a source file (plant lens).
    fn named_fn_src(src: &str, name: &str) -> Option<String> {
        // plain fns (`fn name(`) and generic fns (`fn name<E: Env>(`) alike
        let start = src
            .find(&format!("fn {name}("))
            .or_else(|| src.find(&format!("fn {name}<")))?;
        let rest = &src[start..];
        let bytes = rest.as_bytes();
        let brace = bytes.iter().position(|&b| b == b'{')?;
        let mut depth = 0i32;
        for (i, &b) in bytes[brace..].iter().enumerate() {
            if b == b'{' {
                depth += 1;
            } else if b == b'}' {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[brace..=brace + i].to_string());
                }
            }
        }
        None
    }

    #[test]
    fn parked_pair_plan_on_live_short_queue_waits() {
        // RFC-0219 P1.3: fewer than two parked tables wait; two or more
        // hand out the oldest pair (F174 revalidates at swap). AS-IS
        // hands out regardless (short queue loses a parked table).
        assert_eq!(parked_pair_plan(0), ParkedPairPlan::WaitForPair);
        assert_eq!(parked_pair_plan(1), ParkedPairPlan::WaitForPair);
        assert_eq!(parked_pair_plan(2), ParkedPairPlan::HandOutOldestPair);
        assert_eq!(
            parked_pair_plan_as_is(1),
            ParkedPairPlan::HandOutOldestPair,
            "AS-IS tooth: pair handed out of a short queue"
        );
        let popa = named_fn_src(include_str!("db_kernel.rs"), "parked_oldest_pair_arcs")
            .expect("parked_oldest_pair_arcs");
        assert!(
            popa.contains("match crate::flush_kernel::parked_pair_plan("),
            "parked_oldest_pair_arcs must match parked_pair_plan"
        );
        assert!(
            !popa.contains("parked_unflushed.len() < 2"),
            "the raw queue-length gate left the trampoline"
        );
    }

    #[test]
    fn auto_flush_gate_on_live_both_under_skips_scan() {
        // RFC-0219 P1.3: both axes under ⇒ skip the whole auto-flush
        // scan; anything over ⇒ scan. AS-IS always scans (flush churn
        // with nothing due).
        assert_eq!(auto_flush_gate(true, true), AutoFlushGate::SkipAllNotDue);
        assert_eq!(
            auto_flush_gate(true, false),
            AutoFlushGate::ScanColumnFamilies
        );
        assert_eq!(
            auto_flush_gate(false, true),
            AutoFlushGate::ScanColumnFamilies
        );
        assert_eq!(
            auto_flush_gate_as_is(true, true),
            AutoFlushGate::ScanColumnFamilies,
            "AS-IS tooth: scan even when both axes are under"
        );
        let maf = named_fn_src(include_str!("db_kernel.rs"), "maybe_auto_flush")
            .expect("maybe_auto_flush");
        assert!(
            maf.contains("match crate::flush_kernel::auto_flush_gate("),
            "maybe_auto_flush must match auto_flush_gate"
        );
        assert!(
            !maf.contains("skip_auto_flush(global_under, cf_under)"),
            "the raw both-under gate left the trampoline"
        );
    }

    #[test]
    fn dominant_family_stage_plan_on_live_dominant_stages() {
        // RFC-0223 P1.1: fam >= 3/4 of total stages the WHOLE mem
        // (O(1) swap in-commit; the worker splits per family at
        // materialize). Below the bar the family partitions out.
        // AS-IS always partitions (O(n) take_family under the write
        // lock — the whole in-commit flush work @10M, split 7f2758d4).
        assert_eq!(
            dominant_family_stage_plan(80, 100),
            FamilyFlushMode::StageWholeMem
        );
        assert_eq!(
            dominant_family_stage_plan(75, 100),
            FamilyFlushMode::StageWholeMem
        );
        assert_eq!(
            dominant_family_stage_plan(74, 100),
            FamilyFlushMode::PartitionFamily
        );
        assert_eq!(
            dominant_family_stage_plan_as_is(99, 100),
            FamilyFlushMode::PartitionFamily,
            "AS-IS tooth: always partitions in-commit"
        );
        let maf = named_fn_src(include_str!("db_kernel.rs"), "maybe_auto_flush")
            .expect("maybe_auto_flush");
        assert!(
            maf.contains("dominant_family_stage_plan("),
            "maybe_auto_flush defer branch must route through dominant_family_stage_plan"
        );
    }

    #[test]
    fn mem_auto_flush_plan_on_live_armed_over_limit_flushes() {
        // RFC-0219 P1.3: armed and at/over the limit flushes the mem
        // now; unarmed or under keeps accumulating. AS-IS never flushes
        // (unbounded mem until the host stalls).
        assert_eq!(
            mem_auto_flush_plan(100, true, 50),
            MemAutoFlushPlan::FlushMemNow
        );
        assert_eq!(
            mem_auto_flush_plan(10, true, 50),
            MemAutoFlushPlan::NotDueKeepMem
        );
        assert_eq!(
            mem_auto_flush_plan(100, false, 50),
            MemAutoFlushPlan::NotDueKeepMem,
            "unarmed never fires"
        );
        assert_eq!(
            mem_auto_flush_plan_as_is(100, true, 50),
            MemAutoFlushPlan::NotDueKeepMem,
            "AS-IS tooth: armed limit ignored, mem grows unbounded"
        );
        let maf = named_fn_src(include_str!("db_kernel.rs"), "maybe_auto_flush")
            .expect("maybe_auto_flush");
        assert!(
            maf.contains("match crate::flush_kernel::mem_auto_flush_plan("),
            "maybe_auto_flush must match mem_auto_flush_plan on the mem gate"
        );
    }
}
