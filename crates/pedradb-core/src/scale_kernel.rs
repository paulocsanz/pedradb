//! kernel: scale — mathematical model of one-process scale (RFC-0176).
//!
//! Work per point get after settle is [`point_get_probes`] = levels + L0
//! covering files, not the live file count. Clock is [`predict_get_ns`]
//! (hot fraction × noisy-neighbor tax). Single artifact (Aeneas-paid):
//! this file is what `rustc` links and what the Lean defs run over
//! (`scripts/aeneas_scale.sh`, `ScaleKernel.lean`). No Verus twin stands
//! in for them.

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
/// LSM level fanout (RFC-0176).
pub const SCALE_LEVEL_FANOUT: u64 = 10;
/// G1 fdatasync τ (write-forecast helper).
pub const SCALE_TAU_FD_NS: u64 = 2_500;
/// WAL encode τ (write-forecast helper).
pub const SCALE_TAU_WAL_ENCODE_NS: u64 = 200;
/// WAL pwrite τ (write-forecast helper).
pub const SCALE_TAU_WAL_WRITE_NS: u64 = 2_000;
/// Serial lock CS leftover (write-forecast helper).
pub const SCALE_TAU_LOCK_HOLD_NS: u64 = 500;
/// Adaptive merge window (n=2–8).
pub const SCALE_GROUP_ADAPTIVE_MAX: u64 = 8;
/// Slack (0.15 group) before grouping is paid, in basis points.
pub const SCALE_GROUPING_PAID_SLACK_BPS: u64 = 1_500;
/// Mix at which p50 is the get, not the put.
pub const SCALE_GET_PATH_READ_PCT: u64 = 40;
/// Client count at which Adaptive-off becomes the named convoy ceiling.
pub const SCALE_LOCK_CONVOY_CLIENTS: u64 = 16;

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
/// Same add as [`point_get_probes`] — the GPS of probes, not a second walk.
#[must_use]
pub fn probes_worst(levels: u64, l0_max: u64) -> u64 {
    point_get_probes(levels, l0_max)
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

/// Best-path clock: L0-best probes, 100% hot, η = 0. `scale_forecast` matches
/// this for `best_ns`.
#[must_use]
pub fn best_get_ns(levels: u64) -> u64 {
    predict_get_ns(
        point_get_probes(levels, SCALE_L0_BEST),
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        SCALE_BPS,
        0,
    )
}

/// AS-IS: walk every live file as a cold disk probe (η ignored).
#[must_use]
pub fn best_get_ns_as_is(n_files: u64, _levels: u64) -> u64 {
    predict_get_ns_as_is(n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, 0, 0)
}

/// Happy-path clock: L0-best probes, residual hot fraction, η = [`SCALE_HAPPY_NOISY_BPS`].
/// `scale_forecast` matches this for `happy_ns`.
#[must_use]
pub fn happy_get_ns(levels: u64, store_bytes: u64, ram_bytes: u64) -> u64 {
    predict_get_ns(
        point_get_probes(levels, SCALE_L0_BEST),
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        happy_hot_bps(store_bytes, ram_bytes),
        SCALE_HAPPY_NOISY_BPS,
    )
}

/// AS-IS: walk every live file as a cold disk probe (η ignored).
#[must_use]
pub fn happy_get_ns_as_is(n_files: u64, _levels: u64, _store_bytes: u64, _ram_bytes: u64) -> u64 {
    predict_get_ns_as_is(n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, 0, 0)
}

/// Worst-path clock: L0-trigger probes, all cold, η = [`SCALE_WORST_NOISY_BPS`].
/// `scale_forecast` matches this for `worst_ns`.
#[must_use]
pub fn worst_get_ns(levels: u64, l0_max: u64) -> u64 {
    predict_get_ns(
        probes_worst(levels, l0_max),
        SCALE_TAU_RAM_NS,
        SCALE_TAU_DISK_NS,
        0,
        SCALE_WORST_NOISY_BPS,
    )
}

/// AS-IS: walk every live file as a cold disk probe (η ignored).
#[must_use]
pub fn worst_get_ns_as_is(n_files: u64, _levels: u64, _l0_max: u64) -> u64 {
    predict_get_ns_as_is(n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, 0, 0)
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
    let best_ns = best_get_ns(levels);
    let happy_ns = happy_get_ns(levels, store_bytes, ram_bytes);
    let worst_ns = worst_get_ns(levels, SCALE_L0_WORST);
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

/// Same as [`scale_forecast`] with an explicit on-disk byte/entry
/// (YCSB 100 B payload is not the scale-shape 245 B). Bench helper, not
/// a catalog atom — the proved table is [`scale_forecast`].
#[must_use]
pub fn scale_forecast_with(keys: u64, ram_bytes: u64, bytes_per_entry: u64) -> ScaleForecast {
    let bpe = bytes_per_entry.max(1);
    let store_bytes = keys.saturating_mul(bpe);
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
        best_ns: best_get_ns(levels),
        happy_ns: happy_get_ns(levels, store_bytes, ram_bytes),
        worst_ns: worst_get_ns(levels, SCALE_L0_WORST),
    }
}

/// [`scale_forecast_as_is`] with explicit byte/entry.
#[must_use]
pub fn scale_forecast_as_is_with(keys: u64, ram_bytes: u64, bytes_per_entry: u64) -> ScaleForecast {
    let store_bytes = keys.saturating_mul(bytes_per_entry.max(1));
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

/// Walk-every-file is distinguishable from the covering-probe clock.
#[must_use]
pub fn walk_distinguishable(n_files: u64, p_best: u64) -> bool {
    p_best > 0 && n_files > p_best.saturating_mul(2)
}

/// Adaptive expected group size: n=2–8 → n, else 1 (bypass / 1c).
#[must_use]
pub fn expected_avg_group(clients: u64) -> u64 {
    if (2..=SCALE_GROUP_ADAPTIVE_MAX).contains(&clients) {
        clients
    } else {
        1
    }
}

/// `avg_group` has paid Adaptive merge (`expected − 0.15`). `0` bps = unknown.
#[must_use]
pub fn grouping_paid(clients: u64, avg_group_bps: u64) -> bool {
    if avg_group_bps == 0 {
        return false;
    }
    let expected = expected_avg_group(clients).max(1);
    let floor = expected
        .saturating_mul(SCALE_BPS)
        .saturating_sub(SCALE_GROUPING_PAID_SLACK_BPS);
    avg_group_bps >= floor
}

/// Static write cut (RFC-0184 P2.37). Bench helper, not a catalog atom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStaticCut {
    /// n=2–8 and Adaptive merge is unpaid.
    Grouping,
    /// n=2–8 grouping paid, or lock_wait at n&lt;16.
    LockHold,
    /// n≥16 Adaptive-off.
    LockConvoy,
    /// 1c `--sync 1` (G1): one `fdatasync` per op.
    FdCeiling,
    /// 1c same-class async: WAL pwrite.
    AsyncWal,
    /// `read_pct ≥ 40`: p50 is the get.
    GetPath,
}

impl WriteStaticCut {
    /// Stable token for CLI / JSON / mapa.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Grouping => "grouping",
            Self::LockHold => "lock_hold",
            Self::LockConvoy => "lock_convoy",
            Self::FdCeiling => "fd_ceiling",
            Self::AsyncWal => "async_wal",
            Self::GetPath => "get_path",
        }
    }
}

/// Inputs for [`predict_write_mix`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePredictIn {
    /// Client count.
    pub clients: u64,
    /// YCSB-style read percent (`0` = pure write).
    pub read_pct: u64,
    /// G1 (`true`) vs same-class async (`false`).
    pub sync: bool,
    /// `avg_group * SCALE_BPS`; `0` = unknown.
    pub avg_group_bps: u64,
}

impl WritePredictIn {
    /// Pure-write async (product peer).
    #[must_use]
    pub const fn clients(clients: u64) -> Self {
        Self {
            clients,
            read_pct: 0,
            sync: false,
            avg_group_bps: 0,
        }
    }
}

/// Non-linear QPS vs client count / mix (RFC-0184 P2.38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteGrowth {
    /// n=1: one barrier (or pwrite) per op.
    OneBarrier,
    /// n=2–8, grouping unpaid: QPS should scale ~n if merge lives.
    Amortize,
    /// n=2–8, grouping paid: QPS ≈ 1/τ_cs.
    SerialCs,
    /// n≥16 Adaptive-off: QPS falls as n grows.
    ConvoyCollapse,
    /// `read_pct ≥ 40`: QPS bound by get tail.
    GetBound,
}

impl WriteGrowth {
    /// Stable token for CLI / JSON.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::OneBarrier => "one_barrier",
            Self::Amortize => "amortize",
            Self::SerialCs => "serial_cs",
            Self::ConvoyCollapse => "convoy_collapse",
            Self::GetBound => "get_bound",
        }
    }
}

/// Static write clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteForecast {
    /// Client count.
    pub clients: u64,
    /// Expected `avg_group` under Adaptive merge.
    pub expected_group: u64,
    /// Encode + `barrier / expected_group`.
    pub best_ns: u64,
    /// Encode + one barrier per op.
    pub as_is_ns: u64,
    /// `clients` in 2–8.
    pub distinguishable: bool,
    /// Unpaid lever without a bench.
    pub cut: WriteStaticCut,
    /// After `cut` is paid.
    pub next: WriteStaticCut,
    /// Serial lock CS leftover.
    pub lock_hold_ns: u64,
    /// Mix that produced `cut`.
    pub read_pct: u64,
    /// G1 vs async.
    pub sync: bool,
    /// How QPS scales with `clients`.
    pub growth: WriteGrowth,
}

/// Cut token for [`WriteForecast`].
#[must_use]
pub fn write_forecast_cut(w: WriteForecast) -> &'static str {
    w.cut.token()
}

/// Next-cut token after [`write_forecast_cut`].
#[must_use]
pub fn write_forecast_next(w: WriteForecast) -> &'static str {
    w.next.token()
}

/// Growth token for [`WriteForecast`].
#[must_use]
pub fn write_forecast_growth(w: WriteForecast) -> &'static str {
    w.growth.token()
}

/// Predict write ns without a bench. Product peer = async (`sync=false`).
#[must_use]
pub fn predict_write(clients: u64) -> WriteForecast {
    predict_write_mix(WritePredictIn::clients(clients))
}

/// Predict write ns with mix / durability / measured `avg_group`.
#[must_use]
pub fn predict_write_mix(inp: WritePredictIn) -> WriteForecast {
    let g = expected_avg_group(inp.clients).max(1);
    let barrier = if inp.sync {
        SCALE_TAU_FD_NS
    } else {
        SCALE_TAU_WAL_WRITE_NS
    };
    let encode = SCALE_TAU_WAL_ENCODE_NS;
    let (cut, next) = static_write_cut(inp);
    WriteForecast {
        clients: inp.clients,
        expected_group: g,
        best_ns: encode.saturating_add(barrier / g),
        as_is_ns: encode.saturating_add(barrier),
        distinguishable: (2..=SCALE_GROUP_ADAPTIVE_MAX).contains(&inp.clients),
        cut,
        next,
        lock_hold_ns: SCALE_TAU_LOCK_HOLD_NS,
        read_pct: inp.read_pct,
        sync: inp.sync,
        growth: write_growth(cut),
    }
}

fn write_growth(cut: WriteStaticCut) -> WriteGrowth {
    match cut {
        WriteStaticCut::GetPath => WriteGrowth::GetBound,
        WriteStaticCut::LockConvoy => WriteGrowth::ConvoyCollapse,
        WriteStaticCut::FdCeiling | WriteStaticCut::AsyncWal => WriteGrowth::OneBarrier,
        WriteStaticCut::Grouping => WriteGrowth::Amortize,
        WriteStaticCut::LockHold => WriteGrowth::SerialCs,
    }
}

fn static_write_cut(inp: WritePredictIn) -> (WriteStaticCut, WriteStaticCut) {
    if inp.read_pct >= SCALE_GET_PATH_READ_PCT {
        return (WriteStaticCut::GetPath, WriteStaticCut::GetPath);
    }
    if inp.clients >= SCALE_LOCK_CONVOY_CLIENTS {
        return (WriteStaticCut::LockConvoy, WriteStaticCut::LockConvoy);
    }
    if inp.clients <= 1 {
        return if inp.sync {
            (WriteStaticCut::FdCeiling, WriteStaticCut::FdCeiling)
        } else {
            (WriteStaticCut::AsyncWal, WriteStaticCut::FdCeiling)
        };
    }
    if (2..=SCALE_GROUP_ADAPTIVE_MAX).contains(&inp.clients) {
        if grouping_paid(inp.clients, inp.avg_group_bps) {
            return (WriteStaticCut::LockHold, WriteStaticCut::AsyncWal);
        }
        return (WriteStaticCut::Grouping, WriteStaticCut::LockHold);
    }
    (WriteStaticCut::LockHold, WriteStaticCut::LockConvoy)
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
        assert_eq!(probes_worst(u64::from(l10), SCALE_L0_WORST), 9);
        assert_eq!(
            best_get_ns(u64::from(l10)),
            predict_get_ns(6, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, SCALE_BPS, 0,)
        );
    }

    #[test]
    fn scale_forecast_on_empty_store_is_not_ok() {
        let f = scale_forecast(0, 0);
        assert_eq!(f.levels, 0);
        assert_eq!(f.p_best, point_get_probes(0, SCALE_L0_BEST));
        assert_eq!(f.best_ns, best_get_ns(0));
        assert_eq!(f.happy_ns, happy_get_ns(0, 0, 0));
        assert_eq!(f.worst_ns, worst_get_ns(0, SCALE_L0_WORST));
        assert_eq!(
            scale_forecast_as_is(0, 0).p_best,
            0,
            "AS-IS dente: empty walk is 0 files not L0-best probes"
        );
        let src = include_str!("scale_kernel.rs");
        let forecast = src
            .split("pub fn scale_forecast(")
            .nth(1)
            .expect("scale_forecast");
        assert!(forecast.contains("best_get_ns("));
        assert!(forecast.contains("happy_get_ns("));
        assert!(forecast.contains("worst_get_ns("));
    }

    #[test]
    fn best_get_ns_on_l0_best_is_not_ok() {
        assert_eq!(
            best_get_ns(4),
            predict_get_ns(
                point_get_probes(4, SCALE_L0_BEST),
                SCALE_TAU_RAM_NS,
                SCALE_TAU_DISK_NS,
                SCALE_BPS,
                0
            )
        );
        assert!(
            best_get_ns_as_is(913, 4) > best_get_ns(4),
            "as-is walk is slower than the best clock"
        );
        let forecast = include_str!("scale_kernel.rs")
            .split("pub fn scale_forecast(")
            .nth(1)
            .expect("scale_forecast");
        assert!(
            forecast.contains("best_get_ns("),
            "scale_forecast must match best_get_ns"
        );
    }

    #[test]
    fn happy_get_ns_on_l0_best_is_not_ok() {
        let store = 1_000_000_000u64.saturating_mul(SCALE_BYTES_PER_ENTRY);
        let ram = 4u64 << 30;
        assert_eq!(
            happy_get_ns(4, store, ram),
            predict_get_ns(
                point_get_probes(4, SCALE_L0_BEST),
                SCALE_TAU_RAM_NS,
                SCALE_TAU_DISK_NS,
                happy_hot_bps(store, ram),
                SCALE_HAPPY_NOISY_BPS,
            )
        );
        assert!(
            happy_get_ns_as_is(913, 4, store, ram) > happy_get_ns(4, store, ram),
            "as-is walk is slower than the happy clock"
        );
        let forecast = include_str!("scale_kernel.rs")
            .split("pub fn scale_forecast(")
            .nth(1)
            .expect("scale_forecast");
        assert!(
            forecast.contains("happy_get_ns("),
            "scale_forecast must match happy_get_ns"
        );
    }

    #[test]
    fn worst_get_ns_on_l0_trigger_is_not_ok() {
        assert_eq!(
            worst_get_ns(4, SCALE_L0_WORST),
            predict_get_ns(
                8,
                SCALE_TAU_RAM_NS,
                SCALE_TAU_DISK_NS,
                0,
                SCALE_WORST_NOISY_BPS
            )
        );
        assert!(
            worst_get_ns_as_is(913, 4, SCALE_L0_WORST) > worst_get_ns(4, SCALE_L0_WORST) * 50,
            "as-is walk is >50× the L0-trigger clock"
        );
        let src = include_str!("scale_kernel.rs");
        let forecast = src
            .split("pub fn scale_forecast(")
            .nth(1)
            .expect("scale_forecast");
        assert!(
            forecast.contains("worst_get_ns("),
            "scale_forecast must match worst_get_ns"
        );
    }

    #[test]
    fn probes_worst_on_l0_trigger_is_not_ok() {
        assert_eq!(probes_worst(4, SCALE_L0_WORST), 8);
        assert_eq!(point_get_probes(4, SCALE_L0_WORST), 8);
        assert_eq!(probes_worst_as_is(913, 4, SCALE_L0_WORST), 913);
        let src = include_str!("scale_kernel.rs");
        let worst = src
            .split("pub fn probes_worst(")
            .nth(1)
            .expect("probes_worst");
        let body = worst
            .split("pub fn probes_worst_as_is")
            .next()
            .expect("body");
        assert!(
            body.contains("point_get_probes("),
            "probes_worst must call point_get_probes"
        );
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
