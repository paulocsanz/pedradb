//! kernel: scale — mathematical model of one-process scale (RFC-0176).
//!
//! Work per point get after settle is [`point_get_probes`] = levels + L0
//! covering files, not the live file count. Clock is [`predict_get_ns`]
//! (hot fraction × noisy-neighbor tax). Twin: `verus/scale.rs`.

#![forbid(unsafe_code)]

/// WARM floor (10M ~2.4 GiB still fits a 4 GiB box).
pub const WARM_FLOOR_BYTES: u64 = 3 * (1 << 30);
/// Reserve vs cgroup / physical RAM (RFC-0173 P2.2).
pub const WARM_RESERVE_BYTES: u64 = 1 << 30;
/// On-disk bytes per scale-shape entry (241 framed + ~4 meta).
pub const SCALE_BYTES_PER_ENTRY: u64 = 245;
/// Compact / L1 target (same as `COMPACT_TARGET_FILE_BYTES`).
pub const SCALE_L1_BYTES: u64 = 256 * 1024 * 1024;
/// SST data-block target.
pub const SCALE_BLOCK_BYTES: u64 = 4_096;
/// Two-level index fanout.
pub const SCALE_INDEX_FANOUT: u64 = 32;
/// Sample row: `u64` p8 + `u32` file-rel.
pub const SCALE_INDEX_SAMPLE_BYTES: u64 = 12;
/// L0 covering files after settle (best / happy).
pub const SCALE_L0_BEST: u64 = 1;
/// L0 files at the compaction trigger (worst production).
pub const SCALE_L0_WORST: u64 = 4;
/// Basis-point denominator (100% = 10_000).
pub const SCALE_BPS: u64 = 10_000;
/// Happy-path residual hits when the store does not fit in RAM.
pub const SCALE_HAPPY_COLD_HOT_BPS: u64 = 2_000;
/// Happy-path noisy-neighbor tax (10% → 1.1×).
pub const SCALE_HAPPY_NOISY_BPS: u64 = 1_000;
/// Worst-path noisy-neighbor tax (50% → 1.5×).
pub const SCALE_WORST_NOISY_BPS: u64 = 5_000;
/// Per-probe RAM time (calibrated; retune from `scale_spectrum` if off).
pub const SCALE_TAU_RAM_NS: u64 = 1_100;
/// Per-probe disk time (calibrated; SSD pread).
pub const SCALE_TAU_DISK_NS: u64 = 13_500;

/// After settle: one covering SST per disjoint L1+ level plus `l0_covering`.
#[must_use]
pub fn point_get_probes(levels: u64, l0_covering: u64) -> u64 {
    levels.saturating_add(l0_covering)
}

/// AS-IS: walk every live SST.
#[must_use]
pub fn point_get_probes_as_is(n_files: u64, _levels: u64, _l0_covering: u64) -> u64 {
    n_files
}

/// Default WARM cap from a RAM ceiling (`0` = unknown → floor only).
#[must_use]
pub fn warm_cap_bytes(ram_ceiling: u64) -> u64 {
    if ram_ceiling == 0 {
        return WARM_FLOOR_BYTES;
    }
    let share = ram_ceiling.saturating_mul(3) / 4;
    let cap = if WARM_FLOOR_BYTES >= share {
        WARM_FLOOR_BYTES
    } else {
        share
    };
    let reserved = ram_ceiling.saturating_sub(WARM_RESERVE_BYTES);
    if cap <= reserved {
        cap
    } else {
        reserved
    }
}

/// AS-IS: ignore the ceiling.
#[must_use]
pub fn warm_cap_bytes_as_is(_ram_ceiling: u64) -> u64 {
    u64::MAX
}

/// Worst production probes: every L1+ level plus a full L0 trigger stack.
#[must_use]
pub fn probes_worst(levels: u64, l0_max: u64) -> u64 {
    levels.saturating_add(l0_max)
}

/// AS-IS: still walk every live file.
#[must_use]
pub fn probes_worst_as_is(n_files: u64, _levels: u64, _l0_max: u64) -> u64 {
    n_files
}

/// Clock: `P * (h·τ_ram + (1-h)·τ_disk) * (1 + η)`. Fractions in basis points.
/// `noisy_bps` is the noisy-neighbor variable (CPU/IO/page-cache steal).
#[must_use]
pub fn predict_get_ns(
    probes: u64,
    tau_ram_ns: u64,
    tau_disk_ns: u64,
    hot_bps: u64,
    noisy_bps: u64,
) -> u64 {
    let hot = hot_bps.min(SCALE_BPS);
    let noisy = noisy_bps.min(9_000);
    let mix = hot
        .saturating_mul(tau_ram_ns)
        .saturating_add((SCALE_BPS - hot).saturating_mul(tau_disk_ns));
    let per = (u128::from(probes)).saturating_mul(u128::from(mix)) / u128::from(SCALE_BPS);
    let taxed =
        per.saturating_mul(u128::from(SCALE_BPS) + u128::from(noisy)) / u128::from(SCALE_BPS);
    u64::try_from(taxed).unwrap_or(u64::MAX)
}

/// AS-IS: every live file is a cold disk probe; η is ignored.
#[must_use]
pub fn predict_get_ns_as_is(
    n_files: u64,
    _tau_ram_ns: u64,
    tau_disk_ns: u64,
    _hot_bps: u64,
    _noisy_bps: u64,
) -> u64 {
    n_files.saturating_mul(tau_disk_ns)
}

/// Happy-path hot fraction: 100% if the store fits the WARM cap, else residual.
#[must_use]
pub fn happy_hot_bps(store_bytes: u64, ram_bytes: u64) -> u64 {
    if store_bytes <= warm_cap_bytes(ram_bytes) {
        SCALE_BPS
    } else {
        SCALE_HAPPY_COLD_HOT_BPS
    }
}

/// AS-IS: always claim 100% hot (the 3 TiB lie).
#[must_use]
pub fn happy_hot_bps_as_is(_store_bytes: u64, _ram_bytes: u64) -> u64 {
    SCALE_BPS
}

/// One-process scale table (RFC-0176). CLI and tests call this; they do
/// not re-derive \(P\) or \(\mathrm{cap}(R)\).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleForecast {
    /// User keys.
    pub keys: u64,
    /// Host RAM budget (bytes).
    pub ram_bytes: u64,
    /// On-disk bytes \(S = n\cdot b\).
    pub store_bytes: u64,
    /// LSM levels \(L\).
    pub levels: u64,
    /// Best-path probes \(L+1\).
    pub p_best: u64,
    /// Worst production probes \(L+4\).
    pub p_worst: u64,
    /// File count if every SST were L1-sized (as-is walk).
    pub n_files: u64,
    /// WARM cap for this RAM.
    pub warm_cap: u64,
    /// Whether the store fits the WARM cap.
    pub hot: bool,
    /// Happy-path hot fraction (basis points).
    pub happy_hot_bps: u64,
    /// Predicted get ns: best / happy / worst.
    pub best_ns: u64,
    /// Happy-path predicted get ns (cold residual + η).
    pub happy_ns: u64,
    /// Worst-path predicted get ns.
    pub worst_ns: u64,
}

/// Compose the RFC-0176 table from the atomic kernel fns.
#[must_use]
pub fn scale_forecast(keys: u64, ram_bytes: u64) -> ScaleForecast {
    let store_bytes = keys.saturating_mul(SCALE_BYTES_PER_ENTRY);
    let levels = u64::from(level_count(store_bytes, SCALE_L1_BYTES));
    let p_best = point_get_probes(levels, SCALE_L0_BEST);
    let p_worst = probes_worst(levels, SCALE_L0_WORST);
    let n_files = if SCALE_L1_BYTES == 0 {
        0
    } else {
        store_bytes.div_ceil(SCALE_L1_BYTES)
    };
    let warm_cap = warm_cap_bytes(ram_bytes);
    let hot = store_bytes <= warm_cap;
    let happy_hot = happy_hot_bps(store_bytes, ram_bytes);
    let best_ns = predict_get_ns(p_best, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0);
    let happy_ns = predict_get_ns(
        p_best,
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        happy_hot,
        SCALE_HAPPY_NOISY_BPS,
    );
    let worst_ns = predict_get_ns(
        p_worst,
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        0,
        SCALE_WORST_NOISY_BPS,
    );
    ScaleForecast {
        keys,
        ram_bytes,
        store_bytes,
        levels,
        p_best,
        p_worst,
        n_files,
        warm_cap,
        hot,
        happy_hot_bps: happy_hot,
        best_ns,
        happy_ns,
        worst_ns,
    }
}

/// AS-IS: walk every file and claim the store is always hot.
#[must_use]
pub fn scale_forecast_as_is(keys: u64, ram_bytes: u64) -> ScaleForecast {
    let store_bytes = keys.saturating_mul(SCALE_BYTES_PER_ENTRY);
    let n_files = if SCALE_L1_BYTES == 0 {
        0
    } else {
        store_bytes.div_ceil(SCALE_L1_BYTES)
    };
    ScaleForecast {
        keys,
        ram_bytes,
        store_bytes,
        levels: n_files,
        p_best: n_files,
        p_worst: n_files,
        n_files,
        warm_cap: u64::MAX,
        hot: true,
        happy_hot_bps: SCALE_BPS,
        best_ns: predict_get_ns_as_is(n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0),
        happy_ns: predict_get_ns_as_is(n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0),
        worst_ns: predict_get_ns_as_is(n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0),
    }
}

fn level_count(store_bytes: u64, l1_target: u64) -> u32 {
    if store_bytes == 0 || l1_target == 0 {
        return 0;
    }
    let mut target = l1_target;
    let mut level = 1u32;
    while target < store_bytes && level < 19 {
        target = target.saturating_mul(10);
        level = level.saturating_add(1);
    }
    level
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(n: u64) -> u64 {
        n.saturating_mul(SCALE_BYTES_PER_ENTRY)
    }

    #[test]
    fn point_get_probes_on_all_files_walk_is_not_ok() {
        let n: u64 = 1_000_000_000;
        let s = store(n);
        let n_files = s.div_ceil(SCALE_L1_BYTES);
        let levels = u64::from(level_count(s, SCALE_L1_BYTES));
        let probes = point_get_probes(levels, 1);
        let as_is = point_get_probes_as_is(n_files, levels, 1);
        assert_eq!(levels, 4);
        assert_eq!(probes, 5);
        assert!(n_files >= 900 && n_files <= 930, "n_files={n_files}");
        assert!(as_is > probes * 100);
        assert_eq!(as_is, n_files);
    }

    #[test]
    fn warm_cap_bytes_on_unbounded_warm_is_not_ok() {
        assert_eq!(warm_cap_bytes(4 << 30), 3 << 30);
        assert_eq!(warm_cap_bytes_as_is(4 << 30), u64::MAX);
        assert_eq!(warm_cap_bytes(0), WARM_FLOOR_BYTES);
        assert_eq!(warm_cap_bytes(8 << 30), 6 << 30);
        assert_eq!(warm_cap_bytes(2 << 30), 1 << 30);
    }

    #[test]
    fn rfc0176_one_and_ten_billion_stay_log_n() {
        let s1 = store(1_000_000_000);
        let s10 = store(10_000_000_000);
        let l1 = level_count(s1, SCALE_L1_BYTES);
        let l10 = level_count(s10, SCALE_L1_BYTES);
        assert_eq!(l1, 4);
        assert_eq!(l10, 5);
        assert_eq!(point_get_probes(u64::from(l1), 1), 5);
        assert_eq!(point_get_probes(u64::from(l10), 1), 6);
    }

    #[test]
    fn probes_worst_on_all_files_walk_is_not_ok() {
        let levels = 4u64;
        let worst = probes_worst(levels, SCALE_L0_WORST);
        assert_eq!(worst, 8);
        assert!(point_get_probes(levels, SCALE_L0_BEST) < worst);
        let as_is = probes_worst_as_is(913, levels, SCALE_L0_WORST);
        assert!(as_is > worst * 100);
    }

    #[test]
    fn predict_get_ns_on_ignoring_noisy_is_not_ok() {
        let p = 5u64;
        let best = predict_get_ns(p, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0);
        let happy = predict_get_ns(
            p,
            SCALE_TAU_RAM_NS,
            SCALE_TAU_DISK_NS,
            SCALE_HAPPY_COLD_HOT_BPS,
            SCALE_HAPPY_NOISY_BPS,
        );
        let worst = predict_get_ns(
            probes_worst(4, SCALE_L0_WORST),
            SCALE_TAU_RAM_NS,
            SCALE_TAU_DISK_NS,
            0,
            SCALE_WORST_NOISY_BPS,
        );
        assert!(
            best > 0 && best < happy && happy < worst,
            "{best} {happy} {worst}"
        );
        let noisy = predict_get_ns(
            p,
            SCALE_TAU_RAM_NS,
            SCALE_TAU_DISK_NS,
            0,
            SCALE_WORST_NOISY_BPS,
        );
        let quiet = predict_get_ns(p, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, 0, 0);
        assert!(noisy > quiet);
        let as_is = predict_get_ns_as_is(913, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0);
        assert!(as_is > worst * 50);
    }

    #[test]
    fn happy_hot_bps_on_always_hot_is_not_ok() {
        let store_10b = 10_000_000_000u64.saturating_mul(SCALE_BYTES_PER_ENTRY);
        let ram = 64u64 << 30;
        assert_eq!(happy_hot_bps(store_10b, ram), SCALE_HAPPY_COLD_HOT_BPS);
        assert_eq!(happy_hot_bps_as_is(store_10b, ram), SCALE_BPS);
        let tiny = 1u64 << 20;
        assert_eq!(happy_hot_bps(tiny, ram), SCALE_BPS);
    }

    #[test]
    fn scale_forecast_on_always_hot_walk_is_not_ok() {
        let ram = 64u64 << 30;
        let f1 = scale_forecast(1_000_000_000, ram);
        let f10 = scale_forecast(10_000_000_000, ram);
        assert_eq!(f1.p_best, 5);
        assert_eq!(f10.p_best, 6);
        assert!(!f1.hot, "1B @ 64 GiB is bounded-cache");
        assert!(!f10.hot, "10B @ 64 GiB is bounded-cache");
        assert!(f1.best_ns < f1.happy_ns && f1.happy_ns < f1.worst_ns);
        assert!(f10.best_ns < f10.happy_ns && f10.happy_ns < f10.worst_ns);
        let as_is = scale_forecast_as_is(1_000_000_000, ram);
        assert!(as_is.hot);
        assert!(as_is.p_best > f1.p_best * 100);
        assert_eq!(as_is.happy_hot_bps, SCALE_BPS);
    }
}
