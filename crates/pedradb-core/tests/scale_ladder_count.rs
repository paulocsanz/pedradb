//! RFC-0199 P0.3 counting ladder — Rust twin of
//! `point_get_probes_le_levels_l0_max` (Lean:
//! `formal/aeneas/lean/ProbeLadderCount.lean`, count row
//! `catalog:scale_predict`).
//!
//! The theorem's claim in Rust terms: under the L0-covering cap
//! invariant (`l0_covering <= l0_max`) and a `levels + l0_max` that fits
//! u64, the probes a point get pays never exceed `levels + l0_max` —
//! probes grow with the level count, never with the file count. The
//! REAL kernel (`scale_kernel::point_get_probes`, the extracted fn) is
//! driven on every row; the bounds are plain checked arithmetic in the
//! test, never a re-implementation of the kernel.

use pedradb_core::scale_kernel::{point_get_probes, SCALE_L0_BEST, SCALE_L0_WORST};

/// The theorem's hypothesis and bound, on rows where `levels + l0_max`
/// fits u64 (plain `+`, no saturation on the bound side).
#[test]
fn probes_le_levels_plus_l0_max_under_cap() {
    let rows: [(u64, u64, u64); 8] = [
        (0, 0, 4),
        (3, 1, 4),
        (7, 4, 4),
        (2, 0, 0),
        (1, 1, 1),
        (1_000, 999, 999),
        (u64::MAX - 4, 4, 4),
        (u64::MAX / 2, 0, 1),
    ];
    for (levels, l0_covering, l0_max) in rows {
        assert!(l0_covering <= l0_max, "row violates the cap invariant");
        assert!(
            levels + l0_max <= u64::MAX,
            "row violates the fits-u64 hypothesis"
        );
        let probes = point_get_probes(levels, l0_covering);
        assert_eq!(
            probes,
            levels + l0_covering,
            "under the cap the probes are the exact sum (no saturation)"
        );
        assert!(
            probes <= levels + l0_max,
            "probes must stay <= levels + l0_max (the level-ratio shape)"
        );
    }
}

/// The engine's own L0 bounds (`SCALE_L0_BEST`/`SCALE_L0_WORST`) satisfy
/// the cap invariant, so the worst settled shape is `levels + 4` — not
/// the walk-all file count.
#[test]
fn probes_worst_shape_is_levels_plus_l0_worst() {
    assert!(SCALE_L0_BEST <= SCALE_L0_WORST);
    for levels in [0u64, 1, 5, 33, 1 << 20] {
        let best = point_get_probes(levels, SCALE_L0_BEST);
        let worst = point_get_probes(levels, SCALE_L0_WORST);
        assert_eq!(best, levels + SCALE_L0_BEST);
        assert_eq!(worst, levels + SCALE_L0_WORST);
        // the as-is mutant walks every file — the theorem's point: the
        // covering ladder's probe count never scales with n_files.
        let n_files = levels.saturating_mul(12);
        let as_is = pedradb_core::scale_kernel::point_get_probes_as_is(n_files, levels, SCALE_L0_WORST);
        if n_files > worst + 1 {
            assert!(as_is > worst, "walk-all pays per file, not per level");
        }
    }
}

/// Saturation is the u64 ceiling, never a wraparound: the bound degrades
/// to MAX exactly when the sum would overflow.
#[test]
fn probes_saturate_at_u64_max() {
    assert_eq!(point_get_probes(u64::MAX, 1), u64::MAX);
    assert_eq!(point_get_probes(u64::MAX - 1, 2), u64::MAX);
    assert_eq!(point_get_probes(u64::MAX, u64::MAX), u64::MAX);
    // one below the ceiling: still the exact sum
    assert_eq!(point_get_probes(u64::MAX - 1, 1), u64::MAX);
    assert_eq!(point_get_probes(u64::MAX - 2, 2), u64::MAX);
    assert_eq!(point_get_probes(u64::MAX - 3, 3), u64::MAX);
}
