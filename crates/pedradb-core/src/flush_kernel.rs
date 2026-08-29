//! Pure flush-pipeline decisions (RFC-0056 P0.2 — crash dictionary on the
//! memtable → SST → WAL-rotate path).
//!
//! Production [`crate::Db::flush`] and [`crate::Db::try_rotate_wal`] route
//! their decisions through this kernel: which flush step runs (finish the
//! pending imm / write the mem tail to an SST / rotate only), and whether
//! the WAL may be truncated at all. The invariant that makes it
//! data-fate: **the WAL may only be rotated once every copy of acked keys
//! lives in an installed SST** — mem, imm, the off-lock flush read pin,
//! parked-unflushed tables, and in-flight commits all pin the WAL.
//!
//! Named decisions (the ones that were `SilentWrong` when inverted):
//! - **Mem tail is never dropped** — `flush` with a non-empty mem always
//!   writes the SST before any WAL rotate; rotating instead loses every
//!   acked key that lived only in mem (crash ⇒ data loss).
//! - **Pin live ⇒ WAL kept** — after `prepare_flush_imm` the only copy of
//!   acked keys may be the flush read pin (and an in-flight SST);
//!   truncating the WAL there is the pre-fix hole
//!   (`Db::rotate_wal_ignoring_pin` replays it).
//! - **Commit in flight ⇒ WAL kept** — a writer parked in the off-lock
//!   fsync window still owns WAL bytes (F2).
//! - **Pending imm finishes first** — single-flight: a previous flush's
//!   imm is completed before mem is staged.
//!
//! Verus twin: `crates/pedradb-core/verus/flush_decision.rs`.
//! Spec page: `docs/formal/crash-dictionary.md` (flush section).

#![forbid(unsafe_code)]

/// One step of the flush pipeline (RFC-0056 P0.2).
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

/// Pure rule for which flush step runs.
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   (plan == RotateOnly)          ==> mem_empty && !imm_present
///   !mem_empty                    ==> plan != RotateOnly   // tail never dropped
///   imm_present                   ==> plan == FinishImmThenFlush
/// ```
///
/// Finite-domain check: [`tests::theorem_flush_plan_on_finite_domain`].
#[must_use]
pub fn flush_plan(mem_empty: bool, imm_present: bool) -> FlushPlan {
    match (imm_present, mem_empty) {
        (true, _) => FlushPlan::FinishImmThenFlush,
        (false, false) => FlushPlan::WriteSstThenRotate,
        (false, true) => FlushPlan::RotateOnly,
    }
}

/// AS-IS data loss: flush "succeeds" without writing the mem tail — the
/// WAL rotate then truncates the only durable copy of acked keys that
/// lived in mem (crash ⇒ every unflushed acked write is gone). Mutant
/// must fail every theorem above.
#[must_use]
pub fn flush_plan_as_is_lose_tail(_mem_empty: bool, _imm_present: bool) -> FlushPlan {
    FlushPlan::RotateOnly
}

/// MANIFEST / CURRENT may name an SST only after that file is durable.
#[must_use]
pub fn may_publish_manifest(sst_durable: bool) -> bool {
    sst_durable
}

/// AS-IS: publish MANIFEST while the SST is still unsynced (crash → CURRENT
/// points at a torn/missing file).
#[must_use]
pub fn may_publish_manifest_as_is(_sst_durable: bool) -> bool {
    true
}

/// Every way acked keys can still depend on the WAL.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WalRotateAction {
    /// Every copy of acked keys lives in an installed SST: the WAL may be
    /// recreated empty.
    RotateWal,
    /// Something still depends on the WAL: keep it, retry later.
    KeepWal,
}

/// Pure rule for the WAL rotate (G1 tail of the flush pipeline).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   (a == RotateWal) <==> (mem_empty && !imm_present && !pin_live
///        && !parked_unflushed && !commit_inflight)
///   pin_live || commit_inflight || imm_present || parked_unflushed
///        || !mem_empty  ==> a == KeepWal   // never truncate a live WAL
/// ```
///
/// Finite-domain check: [`tests::theorem_wal_rotate_on_finite_domain`].
#[must_use]
pub fn wal_rotate_decision(s: WalPinState) -> WalRotateAction {
    if s.mem_empty && !s.imm_present && !s.pin_live && !s.parked_unflushed && !s.commit_inflight {
        WalRotateAction::RotateWal
    } else {
        WalRotateAction::KeepWal
    }
}

/// AS-IS pre-fix hole: decide the rotate ignoring the flush read pin —
/// truncates the WAL while the pin (and an in-flight SST) holds the only
/// copy of acked keys (`Db::rotate_wal_ignoring_pin` replays this).
#[must_use]
pub fn wal_rotate_decision_as_is_ignore_pin(s: WalPinState) -> WalRotateAction {
    if s.mem_empty && !s.imm_present && !s.parked_unflushed && !s.commit_inflight {
        WalRotateAction::RotateWal
    } else {
        WalRotateAction::KeepWal
    }
}

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
}
