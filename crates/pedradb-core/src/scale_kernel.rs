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
/// LSM level fanout \(F\) (RFC-0176). Not [`SCALE_INDEX_FANOUT`] (SST index).
pub const SCALE_LEVEL_FANOUT: u64 = 10;
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
/// 1c overwrite WAL+fd band (RFC-0183 Darwin wal ≈ 2.46 µs of 3.3 µs p50).
pub const SCALE_TAU_FD_NS: u64 = 2_500;
/// WAL encode aside from the barrier (order-of-magnitude; not a retune of τ_ram).
pub const SCALE_TAU_WAL_ENCODE_NS: u64 = 200;
/// Async pwrite without `fdatasync` (same-class peer). Not a retune of τ_fd.
pub const SCALE_TAU_WAL_WRITE_NS: u64 = 2_000;
/// Serial write-lock CS leftover after Adaptive grouping (order-of-magnitude).
/// Measured `lock_wait` at mc4 is the real number; this names the cut.
pub const SCALE_TAU_LOCK_HOLD_NS: u64 = 500;
/// Adaptive merge window (RFC-0178 P0.12): n=2–8 merge, else 1.
pub const SCALE_GROUP_ADAPTIVE_MAX: u64 = 8;
/// Grouping is paid when `avg_group ≥ expected − 0.15` (1_500 bps of 10_000).
pub const SCALE_GROUPING_PAID_SLACK_BPS: u64 = 1_500;
/// Mixed shape: p50 is the get, not the put (`read_pct ≥ 40`).
pub const SCALE_GET_PATH_READ_PCT: u64 = 40;
/// Adaptive-off convoy (RFC-0178): n≥16, not a merge bug.
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
    scale_forecast_with(keys, ram_bytes, SCALE_BYTES_PER_ENTRY)
}

/// Same as [`scale_forecast`] with an explicit on-disk byte/entry
/// (YCSB 100 B payload is not the scale-shape 245 B).
#[must_use]
pub fn scale_forecast_with(keys: u64, ram_bytes: u64, bytes_per_entry: u64) -> ScaleForecast {
    let bpe = bytes_per_entry.max(1);
    let store_bytes = keys.saturating_mul(bpe);
    let levels = u64::from(level_count(store_bytes, SCALE_L1_BYTES));
    let p_best = point_get_probes(levels, SCALE_L0_BEST);
    let p_worst = probes_worst(levels, SCALE_L0_WORST);
    let n_files = file_count(store_bytes);
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
    scale_forecast_as_is_with(keys, ram_bytes, SCALE_BYTES_PER_ENTRY)
}

/// [`scale_forecast_as_is`] with explicit byte/entry.
#[must_use]
pub fn scale_forecast_as_is_with(keys: u64, ram_bytes: u64, bytes_per_entry: u64) -> ScaleForecast {
    let store_bytes = keys.saturating_mul(bytes_per_entry.max(1));
    let n_files = file_count(store_bytes);
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

/// Walk-all is a *different physics* from legal \(P\) iff more than \(2P_{\mathrm{best}}\) files.
/// Below that, \(N_{\mathrm{files}}\approx P\) and wall-clock cannot prove a probe bug
/// (1M scale-shape is one L1 file).
#[must_use]
pub fn walk_distinguishable(n_files: u64, p_best: u64) -> bool {
    p_best > 0 && n_files > p_best.saturating_mul(2)
}

/// Clustered prefix/scan: covering SST count for `prefix_keys` (not the whole store).
#[must_use]
pub fn prefix_covering_files(prefix_keys: u64, bytes_per_entry: u64) -> u64 {
    file_count(prefix_keys.saturating_mul(bytes_per_entry.max(1))).max(1)
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

/// Static write cut. Deeper than `grouping` vs `fd_ceiling` (RFC-0184 P2.37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStaticCut {
    /// n=2–8 and Adaptive merge is unpaid (avg unknown or &lt; expected−0.15).
    Grouping,
    /// n=2–8 grouping paid, or lock_wait at n&lt;16: leader CS, not acquire spin.
    LockHold,
    /// n≥16 Adaptive-off (named ceiling; do not "win" by turning merge on).
    LockConvoy,
    /// 1c `--sync 1` (G1): one `fdatasync` per op.
    FdCeiling,
    /// 1c same-class async: WAL pwrite, not the fd column.
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

/// Inputs for [`predict_write_mix`]. Defaults: async, pure write, avg unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePredictIn {
    /// Client count.
    pub clients: u64,
    /// YCSB-style read percent (`0` = pure write).
    pub read_pct: u64,
    /// G1 (`true`) vs same-class async (`false`, product peer).
    pub sync: bool,
    /// `avg_group * SCALE_BPS`; `0` = unknown (static default).
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

/// Static write clock. `cut` is the unpaid lever; `next` is what remains after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteForecast {
    /// Client count.
    pub clients: u64,
    /// Expected `avg_group` under Adaptive merge.
    pub expected_group: u64,
    /// Encode + `barrier / expected_group`.
    pub best_ns: u64,
    /// Encode + one barrier per op (grouping dead).
    pub as_is_ns: u64,
    /// `clients` in 2–8 (merge window exists; cut may already be `lock_hold`).
    pub distinguishable: bool,
    /// Unpaid lever without a bench.
    pub cut: WriteStaticCut,
    /// After `cut` is paid.
    pub next: WriteStaticCut,
    /// Serial lock CS leftover (not τ_fd).
    pub lock_hold_ns: u64,
    /// Mix that produced `cut`.
    pub read_pct: u64,
    /// G1 vs async.
    pub sync: bool,
    /// How QPS scales with `clients` (or get mix): not always ~n.
    pub growth: WriteGrowth,
}

/// Non-linear QPS vs client count / mix (RFC-0184 P2.38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteGrowth {
    /// n=1: one barrier (or pwrite) per op. QPS does not grow with n.
    OneBarrier,
    /// n=2–8, grouping unpaid: QPS should scale ~n if merge lives.
    Amortize,
    /// n=2–8, grouping paid: QPS ≈ 1/τ_cs, **not** ~n (serial lock CS).
    SerialCs,
    /// n≥16 Adaptive-off: QPS falls as n grows. Named ceiling.
    ConvoyCollapse,
    /// `read_pct ≥ 40`: QPS bound by get tail, not put. p50 ≠ QPS.
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

/// Growth token for [`WriteForecast`].
#[must_use]
pub fn write_forecast_growth(w: WriteForecast) -> &'static str {
    w.growth.token()
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
    // n=9–15: Adaptive-off, not yet the n≥16 named ceiling.
    (WriteStaticCut::LockHold, WriteStaticCut::LockConvoy)
}

fn file_count(store_bytes: u64) -> u64 {
    if SCALE_L1_BYTES == 0 || store_bytes == 0 {
        0
    } else {
        store_bytes.div_ceil(SCALE_L1_BYTES)
    }
}

fn level_count(store_bytes: u64, l1_target: u64) -> u32 {
    if store_bytes == 0 || l1_target == 0 {
        return 0;
    }
    let mut target = l1_target;
    let mut level = 1u32;
    while target < store_bytes && level < 19 {
        target = target.saturating_mul(SCALE_LEVEL_FANOUT);
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

    #[test]
    fn walk_distinguishable_by_probes_not_wall_clock() {
        let ram = 64u64 << 30;
        let f1 = scale_forecast(1_000_000, ram);
        let a1 = scale_forecast_as_is(1_000_000, ram);
        assert_eq!(a1.n_files, 1);
        assert!(!walk_distinguishable(a1.n_files, f1.p_best));
        let f10 = scale_forecast(10_000_000, ram);
        let a10 = scale_forecast_as_is(10_000_000, ram);
        assert_eq!(a10.n_files, 10);
        assert!(walk_distinguishable(a10.n_files, f10.p_best));
        let f1b = scale_forecast(1_000_000_000, ram);
        let a1b = scale_forecast_as_is(1_000_000_000, ram);
        assert!(walk_distinguishable(a1b.n_files, f1b.p_best));
    }

    #[test]
    fn scale_forecast_with_smaller_entry_has_fewer_files() {
        let ram = 64u64 << 30;
        let scale = scale_forecast_with(100_000_000, ram, SCALE_BYTES_PER_ENTRY);
        let ycsb = scale_forecast_with(100_000_000, ram, 100);
        assert!(
            ycsb.n_files < scale.n_files,
            "{} vs {}",
            ycsb.n_files,
            scale.n_files
        );
        assert_eq!(scale.n_files, 92);
    }

    #[test]
    fn prefix_covering_clustered_is_one_file() {
        assert_eq!(prefix_covering_files(1_000, SCALE_BYTES_PER_ENTRY), 1);
        assert!(prefix_covering_files(2_000_000, SCALE_BYTES_PER_ENTRY) > 1);
    }

    #[test]
    fn predict_write_mc4_grouping_is_the_gap() {
        let w1 = predict_write(1);
        assert!(!w1.distinguishable);
        assert_eq!(w1.expected_group, 1);
        assert_eq!(w1.best_ns, w1.as_is_ns);
        assert_eq!(write_forecast_cut(w1), "async_wal");
        assert_eq!(write_forecast_next(w1), "fd_ceiling");
        let w4 = predict_write(4);
        assert!(w4.distinguishable);
        assert_eq!(w4.expected_group, 4);
        assert!(w4.best_ns < w4.as_is_ns);
        assert_eq!(w4.as_is_ns, w1.as_is_ns);
        assert_eq!(write_forecast_cut(w4), "grouping");
        assert_eq!(write_forecast_next(w4), "lock_hold");
        let w50 = predict_write(50);
        assert!(!w50.distinguishable);
        assert_eq!(write_forecast_cut(w50), "lock_convoy");
    }

    #[test]
    fn predict_write_mix_names_the_four_cells() {
        // overwrite_mc4: n=4 unpaid grouping; paid avg_group → lock_hold.
        let over = predict_write(4);
        assert_eq!(over.cut, WriteStaticCut::Grouping);
        assert_eq!(over.next, WriteStaticCut::LockHold);
        let over_paid = predict_write_mix(WritePredictIn {
            clients: 4,
            read_pct: 0,
            sync: false,
            avg_group_bps: 38_500, // 3.85 ≥ 4 − 0.15
        });
        assert!(grouping_paid(4, 38_500));
        assert!(!grouping_paid(4, 0));
        assert_eq!(over_paid.cut, WriteStaticCut::LockHold);
        assert_eq!(over_paid.next, WriteStaticCut::AsyncWal);
        // ycsb_b_mc4 95% get; yugabyte_docdb_rmw 70% get.
        let ycsb_b = predict_write_mix(WritePredictIn {
            clients: 4,
            read_pct: 95,
            sync: false,
            avg_group_bps: 0,
        });
        assert_eq!(ycsb_b.cut, WriteStaticCut::GetPath);
        let yugabyte = predict_write_mix(WritePredictIn {
            clients: 1,
            read_pct: 70,
            sync: false,
            avg_group_bps: 0,
        });
        assert_eq!(yugabyte.cut, WriteStaticCut::GetPath);
        // rockset_hybrid 1c ~11% get: WAL, not get_path (threshold 40).
        let rockset = predict_write_mix(WritePredictIn {
            clients: 1,
            read_pct: 11,
            sync: false,
            avg_group_bps: 0,
        });
        assert_eq!(rockset.cut, WriteStaticCut::AsyncWal);
        // G1 1c is the fd column, not a same-class win.
        let g1 = predict_write_mix(WritePredictIn {
            clients: 1,
            read_pct: 0,
            sync: true,
            avg_group_bps: 0,
        });
        assert_eq!(g1.cut, WriteStaticCut::FdCeiling);
        assert_eq!(g1.best_ns, SCALE_TAU_WAL_ENCODE_NS + SCALE_TAU_FD_NS);
        // Non-linear QPS vs n / mix (RFC-0184 P2.38).
        assert_eq!(over.growth, WriteGrowth::Amortize);
        assert_eq!(over_paid.growth, WriteGrowth::SerialCs);
        assert_eq!(ycsb_b.growth, WriteGrowth::GetBound);
        assert_eq!(rockset.growth, WriteGrowth::OneBarrier);
        assert_eq!(predict_write(50).growth, WriteGrowth::ConvoyCollapse);
        assert_eq!(g1.growth, WriteGrowth::OneBarrier);
    }

    #[test]
    fn predict_write_growth_is_not_linear_in_n() {
        // Grouping unpaid: QPS ~ n. Paid: QPS stuck at 1/CS.
        assert_eq!(write_forecast_growth(predict_write(4)), "amortize");
        let paid = predict_write_mix(WritePredictIn {
            clients: 4,
            read_pct: 0,
            sync: false,
            avg_group_bps: 38_500,
        });
        assert_eq!(write_forecast_growth(paid), "serial_cs");
        assert_eq!(write_forecast_growth(predict_write(50)), "convoy_collapse");
        assert_eq!(write_forecast_growth(predict_write(1)), "one_barrier");
    }

    #[test]
    fn level_count_uses_named_fanout() {
        assert_eq!(SCALE_LEVEL_FANOUT, 10);
        assert_eq!(level_count(SCALE_L1_BYTES, SCALE_L1_BYTES), 1);
        assert_eq!(
            level_count(SCALE_L1_BYTES.saturating_mul(10), SCALE_L1_BYTES),
            2
        );
    }
}
