//! Pure flush-pipeline decisions (RFC-0056 P0.2 / RFC-0174 P0.3).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_flush_decision.sh
//!
//! Production [`crate::Db::flush`] and [`crate::Db::try_rotate_wal`] route
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
            "AS-IS dente: MANIFEST names unsynced SST"
        );
        assert!(may_publish_manifest(true));
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
            "AS-IS dente: rotate while pin live"
        );
    }

    #[test]
    fn auto_flush_due_on_live_over_limit_is_not_ok() {
        assert!(auto_flush_due(100, true, 50));
        assert!(!auto_flush_due_as_is(100, true, 50), "AS-IS dente: never fires");
        assert!(!auto_flush_due(10, true, 50));
        assert!(!auto_flush_due(100, false, 50), "unarmed never fires");
    }

    #[test]
    fn skip_auto_flush_on_live_both_under_is_not_ok() {
        assert!(skip_auto_flush(true, true));
        assert!(!skip_auto_flush_as_is(true, true));
        assert!(!skip_auto_flush(true, false));
        assert!(!skip_auto_flush(false, true));
    }
}
