//! RFC-0184 — static gap attribution for a bench cell.
//!
//! Same inputs → same cut. No profiler, no host. WRITEPHASE ns and the
//! RFC-0176 get clock are ranked; the lever is a named next cut, not a
//! skiplist. Twin teeth: [`dominant_phase_as_is`] always blames memtable.

#![forbid(unsafe_code)]

/// Basis-point denominator (100% = 10_000).
pub const GAP_BPS: u64 = 10_000;
/// RFC-0055 / 0183: despark memtable-off-lock iff mem ≥ 15% of the *gap*
/// **and** there is a second writer. 1c never pays concurrent memtable.
pub const MEM_DESPARK_GAP_BPS: u64 = 1_500;
/// Unattributed (p50 − timed phases) above this share of p50 → read/client.
pub const UNATTRIBUTED_READ_BPS: u64 = 5_000;
/// Adaptive merge is off at n≥16; avg_group below 1.20 is bypass.
pub const GROUPING_DEAD_BPS: u64 = 12_000;

/// Per-commit (or per-op — caller picks one domain) WRITEPHASE timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WritePhases {
    /// `prepare_write_ops`.
    pub prepare_ns: u64,
    /// WAL encode + `write()`.
    pub wal_ns: u64,
    /// Memtable apply.
    pub mem_ns: u64,
    /// Publish + cache invalidate.
    pub publish_ns: u64,
    /// `maybe_auto_flush_best_effort`.
    pub flush_check_ns: u64,
    /// Time blocked on the Db write lock.
    pub lock_wait_ns: u64,
}

impl WritePhases {
    /// Sum of timed phases.
    #[must_use]
    pub fn timed_ns(self) -> u64 {
        self.prepare_ns
            .saturating_add(self.wal_ns)
            .saturating_add(self.mem_ns)
            .saturating_add(self.publish_ns)
            .saturating_add(self.flush_check_ns)
            .saturating_add(self.lock_wait_ns)
    }

    /// One phase by name.
    #[must_use]
    pub fn ns(self, p: WritePhase) -> u64 {
        match p {
            WritePhase::Prepare => self.prepare_ns,
            WritePhase::Wal => self.wal_ns,
            WritePhase::Mem => self.mem_ns,
            WritePhase::Publish => self.publish_ns,
            WritePhase::FlushCheck => self.flush_check_ns,
            WritePhase::LockWait => self.lock_wait_ns,
        }
    }
}

/// WRITEPHASE name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePhase {
    /// Prepare.
    Prepare,
    /// WAL.
    Wal,
    /// Memtable apply.
    Mem,
    /// Publish.
    Publish,
    /// Auto-flush check.
    FlushCheck,
    /// Write-lock wait.
    LockWait,
}

impl WritePhase {
    /// Stable token for logs / CLI.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Wal => "wal",
            Self::Mem => "mem",
            Self::Publish => "publish",
            Self::FlushCheck => "flush_check",
            Self::LockWait => "lock_wait",
        }
    }

    /// All phases, largest-first scan order (fixed, not sorted).
    pub const ALL: [Self; 6] = [
        Self::Prepare,
        Self::Wal,
        Self::Mem,
        Self::Publish,
        Self::FlushCheck,
        Self::LockWait,
    ];
}

/// Surgical next cut — what to change, not a slogan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteLever {
    /// 1c / grouped WAL encode or `write()`. Not skiplist.
    WalEncodeOrWrite,
    /// RFC-0055 P1.1: insert off the write lock (mc only, mem/gap ≥15%).
    MemtableOffLock,
    /// Auto-flush during the timed window (apply_mc4 Darwin).
    FlushCheck,
    /// Adaptive off / bypass convoy (n≥16, avg_group ~1).
    LockConvoy,
    /// Cache invalidation / publish.
    PublishInvalidate,
    /// Seq alloc / spill prepare.
    Prepare,
    /// p50 not explained by WRITEPHASE and read mix unknown.
    ReadOrClient,
    /// Mixed shape (`read_pct ≥ 40`) whose p50 is the get, not the put.
    GetPath,
    /// Concurrent writers not grouping (avg_group ~1 at mc4).
    Grouping,
}

impl WriteLever {
    /// Stable token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::WalEncodeOrWrite => "wal_encode_or_write",
            Self::MemtableOffLock => "memtable_off_lock",
            Self::FlushCheck => "flush_check",
            Self::LockConvoy => "lock_convoy",
            Self::PublishInvalidate => "publish_invalidate",
            Self::Prepare => "prepare",
            Self::ReadOrClient => "read_or_client",
            Self::GetPath => "get_path",
            Self::Grouping => "grouping",
        }
    }

    /// Parse a token from CLI / mapa. `None` if unknown.
    #[must_use]
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "wal_encode_or_write" => Self::WalEncodeOrWrite,
            "memtable_off_lock" => Self::MemtableOffLock,
            "flush_check" => Self::FlushCheck,
            "lock_convoy" => Self::LockConvoy,
            "publish_invalidate" => Self::PublishInvalidate,
            "prepare" => Self::Prepare,
            "read_or_client" => Self::ReadOrClient,
            "get_path" => Self::GetPath,
            "grouping" => Self::Grouping,
            _ => return None,
        })
    }
}

/// Inputs for [`diagnose_write`]. All ns in **one** domain (per-op or
/// per-commit). `rocks_ns = 0` skips gap shares. `avg_group_bps = 0`
/// skips the grouping lever (`10_000` = avg_group 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteGapInput {
    /// Pedra p50 (or wall/n).
    pub pedra_ns: u64,
    /// Rocks p50 in the same domain; `0` = unknown.
    pub rocks_ns: u64,
    /// Client count (1 = lone path; ≥2 can despark).
    pub clients: u64,
    /// `avg_group * GAP_BPS`; `0` = unknown.
    pub avg_group_bps: u64,
    /// YCSB-style read percent (`0` = unknown / pure write; `50` = A/F).
    pub read_pct: u64,
    /// Timed phases.
    pub phases: WritePhases,
}

/// Result of [`diagnose_write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteDiagnosis {
    /// `pedra − rocks` if pedra is slower, else 0.
    pub gap_ns: u64,
    /// Sum of WRITEPHASE timers.
    pub timed_ns: u64,
    /// `pedra − timed` (saturating).
    pub unattributed_ns: u64,
    /// Largest timed phase.
    pub dominant: WritePhase,
    /// That phase / timed, basis points.
    pub of_timed_bps: u64,
    /// That phase / gap, basis points (`0` if no gap).
    pub of_gap_bps: u64,
    /// Mem / gap, basis points.
    pub mem_of_gap_bps: u64,
    /// RFC-0055 iff: mc **and** mem/gap ≥15%.
    pub despark_memtable: bool,
    /// Named cut.
    pub lever: WriteLever,
}

impl WriteDiagnosis {
    /// One log line (`diagnose dominant=… lever=…`).
    #[must_use]
    pub fn line(self) -> String {
        format!(
            "dominant={} of_timed_bps={} of_gap_bps={} mem_gap_bps={} despark={} lever={}",
            self.dominant.token(),
            self.of_timed_bps,
            self.of_gap_bps,
            self.mem_of_gap_bps,
            u8::from(self.despark_memtable),
            self.lever.token()
        )
    }

    /// Compact JSON object for bench / compare files (RFC-0184 P1.2 / P2.31).
    #[must_use]
    pub fn json_object(self) -> String {
        format!(
            r#"{{"lever":"{}","dominant":"{}","despark":{},"mem_gap_bps":{},"gap_ns":{},"timed_ns":{},"unattributed_ns":{}}}"#,
            self.lever.token(),
            self.dominant.token(),
            u8::from(self.despark_memtable),
            self.mem_of_gap_bps,
            self.gap_ns,
            self.timed_ns,
            self.unattributed_ns
        )
    }
}

/// Largest timed phase (ties: first in [`WritePhase::ALL`]).
#[must_use]
pub fn dominant_phase(p: WritePhases) -> WritePhase {
    let mut best = WritePhase::Prepare;
    let mut ns = 0u64;
    for ph in WritePhase::ALL {
        let v = p.ns(ph);
        if v > ns {
            ns = v;
            best = ph;
        }
    }
    best
}

/// AS-IS: always blame memtable (the skiplist trap).
#[must_use]
pub fn dominant_phase_as_is(_p: WritePhases) -> WritePhase {
    WritePhase::Mem
}

fn bps(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        return 0;
    }
    let v = u128::from(part).saturating_mul(u128::from(GAP_BPS)) / u128::from(whole);
    u64::try_from(v).unwrap_or(u64::MAX)
}

/// Attribute a write cell. Pure.
#[must_use]
pub fn diagnose_write(inp: WriteGapInput) -> WriteDiagnosis {
    let timed = inp.phases.timed_ns();
    let gap = inp.pedra_ns.saturating_sub(inp.rocks_ns);
    let unattr = inp.pedra_ns.saturating_sub(timed);
    let dominant = dominant_phase(inp.phases);
    let of_timed = bps(inp.phases.ns(dominant), timed);
    let of_gap = bps(inp.phases.ns(dominant), gap);
    let mem_of_gap = bps(inp.phases.mem_ns, gap);
    let despark = inp.clients >= 2 && mem_of_gap >= MEM_DESPARK_GAP_BPS;
    let lever = write_lever(inp, timed, unattr, dominant, despark);
    WriteDiagnosis {
        gap_ns: gap,
        timed_ns: timed,
        unattributed_ns: unattr,
        dominant,
        of_timed_bps: of_timed,
        of_gap_bps: of_gap,
        mem_of_gap_bps: mem_of_gap,
        despark_memtable: despark,
        lever,
    }
}

fn write_lever(
    inp: WriteGapInput,
    timed: u64,
    unattr: u64,
    dominant: WritePhase,
    despark: bool,
) -> WriteLever {
    // Adaptive merge is 2–8. Dead grouping there is a merge bug.
    // n≥16 is Adaptive-off by policy (RFC-0178) — lock convoy, not "turn merge on".
    if (2..=8).contains(&inp.clients)
        && inp.avg_group_bps > 0
        && inp.avg_group_bps < GROUPING_DEAD_BPS
    {
        return WriteLever::Grouping;
    }
    if inp.clients >= 16
        && (dominant == WritePhase::LockWait
            || (inp.avg_group_bps > 0 && inp.avg_group_bps < GROUPING_DEAD_BPS))
    {
        return WriteLever::LockConvoy;
    }
    // Get-only / mixed: p50 is the get. Timed=0 (no write in the window)
    // used to fall through to `prepare` (ycsb_c, qs_neg). Unattributed
    // ≥50% of p50 is the same hole when a rare write inflates WRITEPHASE.
    if inp.read_pct >= 40 && (timed == 0 || bps(unattr, inp.pedra_ns) >= UNATTRIBUTED_READ_BPS) {
        return WriteLever::GetPath;
    }
    if timed > 0 && bps(unattr, inp.pedra_ns) >= UNATTRIBUTED_READ_BPS {
        return WriteLever::ReadOrClient;
    }
    if dominant == WritePhase::FlushCheck {
        return WriteLever::FlushCheck;
    }
    if despark {
        return WriteLever::MemtableOffLock;
    }
    match dominant {
        WritePhase::Wal => WriteLever::WalEncodeOrWrite,
        WritePhase::Mem => WriteLever::MemtableOffLock,
        WritePhase::LockWait => WriteLever::LockConvoy,
        WritePhase::Publish => WriteLever::PublishInvalidate,
        WritePhase::Prepare => WriteLever::Prepare,
        WritePhase::FlushCheck => WriteLever::FlushCheck,
    }
}

/// RFC-0176 get clock vs a measured cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetClass {
    /// At or under best (hot RAM, P = L+1).
    Best,
    /// Between best and happy (bounded-cache residual + η).
    Happy,
    /// Between happy and worst (disk + noisy neighbor).
    Worst,
    /// Above 2× worst: walk-all / P_as_is, not the legal P.
    AsIsWalk,
    /// Faster than the best-path clock (model slack or cache hit).
    FasterThanModel,
    /// \(N_{\mathrm{files}}\le 2P_{\mathrm{best}}\): walk-all is the same physics as legal P.
    Indistinguishable,
}

impl GetClass {
    /// Stable token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Happy => "happy",
            Self::Worst => "worst",
            Self::AsIsWalk => "as_is_walk",
            Self::FasterThanModel => "faster_than_model",
            Self::Indistinguishable => "indistinguishable",
        }
    }
}

/// Classify a measured point-get against the RFC-0176 spectrum.
///
/// `as_is_ns` is `n_files * τ_disk` (walk every SST). A cell that lands
/// there is the forbidden as-is, not "scale is disk-bound".
#[must_use]
pub fn classify_get(
    measured_ns: u64,
    best_ns: u64,
    happy_ns: u64,
    worst_ns: u64,
    as_is_ns: u64,
) -> GetClass {
    if measured_ns == 0 {
        return GetClass::FasterThanModel;
    }
    if best_ns > 0 && measured_ns + best_ns / 10 < best_ns {
        return GetClass::FasterThanModel;
    }
    if as_is_ns > 0
        && worst_ns > 0
        && measured_ns > worst_ns.saturating_mul(2)
        && measured_ns >= as_is_ns / 2
    {
        return GetClass::AsIsWalk;
    }
    if worst_ns > 0 && measured_ns > happy_ns.saturating_add(happy_ns / 5) {
        return GetClass::Worst;
    }
    if happy_ns > 0 && measured_ns > best_ns.saturating_add(best_ns / 10) {
        return GetClass::Happy;
    }
    GetClass::Best
}

/// RFC-0182 same-boot set. Any engine cut must diagnose these before
/// claiming a win. New use-case → add a name here (one home).
pub const BALANCE_SHAPES: [&str; 6] = [
    "deps_cache_overwrite_mc4",
    "ycsb_a_mc4",
    "ycsb_b_mc4",
    "ycsb_f_mc4",
    "deps_apply_batch_mc4",
    "deps_cache_overwrite",
];

/// One cell in a multi-shape board (RFC-0182 / /otimizar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BalanceCell {
    /// Ranked Linux 3/3 named loss (today: overwrite_mc4 caixa).
    pub linux_named_loss: bool,
    /// Darwin DIAG / host-load — not the cartaz.
    pub diag_only: bool,
    /// Diagnose lever for this cell.
    pub lever: WriteLever,
}

/// Engine cut is admitted iff it is the Linux named-loss cell, or it
/// hits ≥2 **cartaz** cells. A lever that only shows up on DIAG is
/// refused (0180-style single-shape overfit).
#[must_use]
pub fn balance_admits(cut: WriteLever, cells: &[BalanceCell]) -> bool {
    let mut hits = 0u32;
    let mut cartaz = 0u32;
    let mut named = false;
    for c in cells {
        if c.lever != cut {
            continue;
        }
        hits = hits.saturating_add(1);
        if c.linux_named_loss {
            named = true;
        }
        if !c.diag_only {
            cartaz = cartaz.saturating_add(1);
        }
    }
    if hits == 0 {
        return false;
    }
    named || cartaz >= 2
}

/// AS-IS: every cut is admitted (the overfit trap).
#[must_use]
pub fn balance_admits_as_is(_cut: WriteLever, _cells: &[BalanceCell]) -> bool {
    true
}

/// Cost-trace probes/get vs RFC-0176 \(P_{\mathrm{best}}\).
/// More than 2× legal probes ⇒ walk-all, not "scale is disk".
#[must_use]
pub fn classify_probes(probes_per_get: u64, p_best: u64) -> GetClass {
    if p_best == 0 {
        return GetClass::Best;
    }
    if probes_per_get > p_best.saturating_mul(2) {
        return GetClass::AsIsWalk;
    }
    if probes_per_get <= p_best.saturating_add(1) {
        return GetClass::Best;
    }
    GetClass::Happy
}

/// AS-IS: every probe count is best-path.
#[must_use]
pub fn classify_probes_as_is(_probes_per_get: u64, _p_best: u64) -> GetClass {
    GetClass::Best
}

/// Static GET bottleneck: legal clock vs walk-all, decided by **probes**
/// not wall-clock (time-class lies below ~25M where \(N_{\mathrm{files}}\approx P\)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleBottleneck {
    /// Legal RFC-0176 forecast.
    pub legal: crate::scale_kernel::ScaleForecast,
    /// Walk-every-L1-file forecast.
    pub as_is: crate::scale_kernel::ScaleForecast,
    /// Probe-based class (`as_is_walk` or `indistinguishable`).
    pub walk_class: GetClass,
    /// `n_files > 2 P_best`.
    pub distinguishable: bool,
    /// Byte/entry used for \(S\).
    pub bytes_per_entry: u64,
}

impl ScaleBottleneck {
    /// `probe_path` if walk-all is a different physics; else `indistinguishable`.
    #[must_use]
    pub fn cut_token(&self) -> &'static str {
        if self.distinguishable {
            "probe_path"
        } else {
            "indistinguishable"
        }
    }
}

/// RFC-0176 bottleneck at `keys` **without running a get**.
///
/// Probe rule (not µs): `n_files > 2 P_best` ⇒ `AsIsWalk` (cut probe path).
/// Otherwise `Indistinguishable` — do not claim walk-all at 1M.
#[must_use]
pub fn scale_bottleneck(keys: u64, ram: u64, bytes_per_entry: u64) -> ScaleBottleneck {
    let bpe = bytes_per_entry.max(1);
    let legal = crate::scale_kernel::scale_forecast_with(keys, ram, bpe);
    let as_is = crate::scale_kernel::scale_forecast_as_is_with(keys, ram, bpe);
    let distinguishable = crate::scale_kernel::walk_distinguishable(as_is.n_files, legal.p_best);
    let walk_class = if distinguishable {
        classify_probes(as_is.n_files, legal.p_best)
    } else {
        GetClass::Indistinguishable
    };
    ScaleBottleneck {
        legal,
        as_is,
        walk_class,
        distinguishable,
        bytes_per_entry: bpe,
    }
}

/// Scale-shape wrapper (`b=245`).
#[must_use]
pub fn predict_get_bottleneck(keys: u64, ram: u64) -> GetClass {
    scale_bottleneck(keys, ram, crate::scale_kernel::SCALE_BYTES_PER_ENTRY).walk_class
}

/// Same proof for probes: walk-all file count vs \(P_{\mathrm{best}}\).
/// At small n this is `Best` (not `Indistinguishable`) — one file is legal P.
#[must_use]
pub fn predict_probes_bottleneck(keys: u64, ram: u64) -> GetClass {
    let f = crate::scale_kernel::scale_forecast(keys, ram);
    let as_is = crate::scale_kernel::scale_forecast_as_is(keys, ram);
    if crate::scale_kernel::walk_distinguishable(as_is.n_files, f.p_best) {
        classify_probes(as_is.n_files, f.p_best)
    } else {
        GetClass::Indistinguishable
    }
}

/// AS-IS: every measured get is "best" (hides walk-all).
#[must_use]
pub fn classify_get_as_is(
    _measured_ns: u64,
    _best_ns: u64,
    _happy_ns: u64,
    _worst_ns: u64,
    _as_is_ns: u64,
) -> GetClass {
    GetClass::Best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale_kernel::{
        predict_get_ns_as_is, scale_forecast, scale_forecast_as_is, SCALE_TAU_DISK_NS,
        SCALE_TAU_RAM_NS,
    };

    /// RFC-0183 1c overwrite Darwin phasesΔ (per commit = per op).
    fn rfc0183_1c() -> WriteGapInput {
        WriteGapInput {
            pedra_ns: 3_300,
            rocks_ns: 2_600,
            clients: 1,
            avg_group_bps: 0,
            read_pct: 0,
            phases: WritePhases {
                prepare_ns: 30,
                wal_ns: 2_460,
                mem_ns: 140,
                publish_ns: 70,
                flush_check_ns: 30,
                lock_wait_ns: 0,
            },
        }
    }

    /// RFC-0183 apply_mc4: phases per commit; gap per op from 1/qps.
    /// mem per op = 10_950 * 105_835 / 400_000 ≈ 2_896 ns; gap ≈ 106_007.
    fn rfc0183_apply_mc4() -> WriteGapInput {
        WriteGapInput {
            pedra_ns: 203_170,
            rocks_ns: 97_163,
            clients: 4,
            avg_group_bps: 71_300,
            read_pct: 0,
            phases: WritePhases {
                prepare_ns: 640,
                wal_ns: 10_310,
                mem_ns: 2_896,
                publish_ns: 140,
                flush_check_ns: 148_310,
                lock_wait_ns: 560,
            },
        }
    }

    #[test]
    fn rfc0183_1c_blames_wal_not_memtable() {
        let d = diagnose_write(rfc0183_1c());
        assert_eq!(d.dominant, WritePhase::Wal);
        assert_eq!(d.lever, WriteLever::WalEncodeOrWrite);
        assert!(!d.despark_memtable, "1c never desparks 0055");
        assert!(
            d.of_timed_bps >= 8_000,
            "wal owns timed: {}",
            d.of_timed_bps
        );
        assert!(d.mem_of_gap_bps >= 1_500 && d.mem_of_gap_bps < 3_000);
        assert_eq!(dominant_phase_as_is(rfc0183_1c().phases), WritePhase::Mem);
        assert_ne!(d.dominant, dominant_phase_as_is(rfc0183_1c().phases));
    }

    #[test]
    fn rfc0184_diagnosis_json_has_lever() {
        let j = diagnose_write(rfc0183_1c()).json_object();
        assert!(
            j.contains("\"lever\":\"wal_encode_or_write\""),
            "compare JSON needs diagnose.lever: {j}"
        );
        assert!(j.contains("\"dominant\":\"wal\""), "{j}");
        assert!(j.contains("\"despark\":0"), "{j}");
        assert!(j.contains("\"gap_ns\":"), "RFC-0184 P2.31 clock: {j}");
        assert!(j.contains("\"timed_ns\":"), "RFC-0184 P2.31 clock: {j}");
        assert!(
            j.contains("\"unattributed_ns\":"),
            "RFC-0184 P2.31 clock: {j}"
        );
    }

    #[test]
    fn rfc0183_apply_mc4_blames_flush_check() {
        let d = diagnose_write(rfc0183_apply_mc4());
        assert_eq!(d.dominant, WritePhase::FlushCheck);
        assert_eq!(d.lever, WriteLever::FlushCheck);
        assert!(!d.despark_memtable, "mem/gap ~2.7% < 15%");
        assert!(
            d.mem_of_gap_bps < MEM_DESPARK_GAP_BPS,
            "mem_gap_bps={}",
            d.mem_of_gap_bps
        );
        assert!(d.of_timed_bps >= 8_000);
    }

    #[test]
    fn mc50_bypass_is_lock_convoy() {
        let d = diagnose_write(WriteGapInput {
            pedra_ns: 10_000,
            rocks_ns: 2_000,
            clients: 50,
            avg_group_bps: 10_000,
            read_pct: 0,
            phases: WritePhases {
                lock_wait_ns: 8_000,
                wal_ns: 1_000,
                mem_ns: 200,
                ..WritePhases::default()
            },
        });
        assert_eq!(d.lever, WriteLever::LockConvoy);
        assert_eq!(d.dominant, WritePhase::LockWait);
    }

    #[test]
    fn mixed_unattributed_is_read_or_client() {
        let d = diagnose_write(WriteGapInput {
            pedra_ns: 7_000,
            rocks_ns: 2_800,
            clients: 4,
            avg_group_bps: 25_000,
            read_pct: 0,
            phases: WritePhases {
                wal_ns: 400,
                mem_ns: 50,
                ..WritePhases::default()
            },
        });
        assert_eq!(d.lever, WriteLever::ReadOrClient);
    }

    #[test]
    fn grouping_dead_beats_wal() {
        let d = diagnose_write(WriteGapInput {
            pedra_ns: 5_000,
            rocks_ns: 2_000,
            clients: 4,
            avg_group_bps: 10_000,
            read_pct: 0,
            phases: WritePhases {
                wal_ns: 3_000,
                mem_ns: 200,
                ..WritePhases::default()
            },
        });
        assert_eq!(d.lever, WriteLever::Grouping);
    }

    #[test]
    fn classify_get_on_as_is_walk_is_not_ok() {
        let ram = 64u64 << 30;
        let f = scale_forecast(1_000_000_000, ram);
        let as_is = scale_forecast_as_is(1_000_000_000, ram);
        assert_eq!(
            classify_get(f.best_ns, f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns),
            GetClass::Best
        );
        assert_eq!(
            classify_get(f.happy_ns, f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns),
            GetClass::Happy
        );
        assert_eq!(
            classify_get(f.worst_ns, f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns),
            GetClass::Worst
        );
        assert_eq!(
            classify_get(
                as_is.best_ns,
                f.best_ns,
                f.happy_ns,
                f.worst_ns,
                as_is.best_ns
            ),
            GetClass::AsIsWalk
        );
        assert_eq!(
            classify_get_as_is(
                as_is.best_ns,
                f.best_ns,
                f.happy_ns,
                f.worst_ns,
                as_is.best_ns
            ),
            GetClass::Best
        );
        let walk = predict_get_ns_as_is(f.n_files, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, 0, 0);
        assert_eq!(walk, as_is.best_ns);
    }

    /// 1B @ 64 GiB: the as-is walk is the bottleneck without running a get.
    #[test]
    fn predict_get_bottleneck_1b_is_walk_without_runtime() {
        let ram = 64u64 << 30;
        assert_eq!(
            predict_get_bottleneck(1_000_000_000, ram),
            GetClass::AsIsWalk
        );
        assert_eq!(
            predict_probes_bottleneck(1_000_000_000, ram),
            GetClass::AsIsWalk
        );
        let f = scale_forecast(1_000_000_000, ram);
        assert_eq!(f.p_best, 5, "1B legal probes stay L+1, not n_files");
    }

    #[test]
    fn predict_get_bottleneck_uses_probes_not_wall_clock() {
        let ram = 64u64 << 30;
        assert_eq!(
            predict_get_bottleneck(1_000_000, ram),
            GetClass::Indistinguishable,
            "1M is one L1 file — time-class used to say worst"
        );
        assert_eq!(
            predict_get_bottleneck(10_000_000, ram),
            GetClass::AsIsWalk,
            "10 files vs P=3 is walk-all; time-class used to say worst"
        );
        let b = scale_bottleneck(10_000_000, ram, crate::scale_kernel::SCALE_BYTES_PER_ENTRY);
        assert!(b.distinguishable);
        assert_eq!(b.cut_token(), "probe_path");
        let small = scale_bottleneck(1_000_000, ram, crate::scale_kernel::SCALE_BYTES_PER_ENTRY);
        assert!(!small.distinguishable);
        assert_eq!(small.cut_token(), "indistinguishable");
    }

    #[test]
    fn rfc0176_calibration_cells_are_best_or_happy() {
        // 50M WARM get_hit ~4.3 µs vs best 4.4 µs (P≈4, hot).
        let best_50m = crate::scale_kernel::predict_get_ns(
            4,
            SCALE_TAU_RAM_NS,
            SCALE_TAU_DISK_NS,
            crate::scale_kernel::SCALE_BPS,
            0,
        );
        assert_eq!(
            classify_get(
                4_300,
                best_50m,
                best_50m.saturating_mul(2),
                50_000,
                5_000_000
            ),
            GetClass::Best
        );
        // 100M get_loop/100 ~53.9 µs vs cold P=4 η=0.
        let cold =
            crate::scale_kernel::predict_get_ns(4, SCALE_TAU_RAM_NS, SCALE_TAU_DISK_NS, 0, 0);
        assert_eq!(
            classify_get(53_900, best_50m, cold, cold.saturating_mul(2), 5_000_000),
            GetClass::Happy
        );
    }

    #[test]
    fn ycsb_a_mixed_is_get_path_not_wal() {
        let mut inp = rfc0183_1c();
        inp.pedra_ns = 7_000;
        inp.rocks_ns = 2_800;
        inp.clients = 4;
        inp.avg_group_bps = 25_000;
        inp.read_pct = 50;
        inp.phases = WritePhases {
            wal_ns: 400,
            mem_ns: 50,
            ..WritePhases::default()
        };
        let d = diagnose_write(inp);
        assert_eq!(d.lever, WriteLever::GetPath);
        let mut unknown = inp;
        unknown.read_pct = 0;
        assert_eq!(diagnose_write(unknown).lever, WriteLever::ReadOrClient);
    }

    #[test]
    fn ycsb_c_all_reads_timed_zero_is_get_path() {
        let mut inp = rfc0183_1c();
        inp.pedra_ns = 4_400;
        inp.rocks_ns = 0;
        inp.clients = 1;
        inp.read_pct = 100;
        inp.phases = WritePhases::default();
        assert_eq!(diagnose_write(inp).lever, WriteLever::GetPath);
        inp.read_pct = 0;
        assert_eq!(diagnose_write(inp).lever, WriteLever::Prepare);
    }

    #[test]
    fn rfc0182_darwin_board_refuses_single_diag_cut() {
        let board = [
            BalanceCell {
                linux_named_loss: false,
                diag_only: true,
                lever: WriteLever::GetPath,
            },
            BalanceCell {
                linux_named_loss: false,
                diag_only: true,
                lever: WriteLever::GetPath,
            },
            BalanceCell {
                linux_named_loss: false,
                diag_only: true,
                lever: WriteLever::WalEncodeOrWrite,
            },
            BalanceCell {
                linux_named_loss: false,
                diag_only: true,
                lever: WriteLever::FlushCheck,
            },
        ];
        assert!(
            !balance_admits(WriteLever::WalEncodeOrWrite, &board),
            "one Darwin 1c WAL is not an engine cut"
        );
        assert!(!balance_admits(WriteLever::FlushCheck, &board));
        assert!(
            !balance_admits(WriteLever::GetPath, &board),
            "two DIAG mixed cells still DIAG"
        );
        assert!(balance_admits_as_is(WriteLever::WalEncodeOrWrite, &board));
    }

    #[test]
    fn linux_named_loss_admits_its_lever() {
        let board = [BalanceCell {
            linux_named_loss: true,
            diag_only: false,
            lever: WriteLever::WalEncodeOrWrite,
        }];
        assert!(balance_admits(WriteLever::WalEncodeOrWrite, &board));
        assert!(!balance_admits(WriteLever::FlushCheck, &board));
    }

    #[test]
    fn two_cartaz_cells_admit_shared_lever() {
        let board = [
            BalanceCell {
                linux_named_loss: false,
                diag_only: false,
                lever: WriteLever::GetPath,
            },
            BalanceCell {
                linux_named_loss: false,
                diag_only: false,
                lever: WriteLever::GetPath,
            },
        ];
        assert!(balance_admits(WriteLever::GetPath, &board));
    }

    #[test]
    fn classify_probes_on_walk_all_is_not_ok() {
        assert_eq!(classify_probes(5, 5), GetClass::Best);
        assert_eq!(classify_probes(900, 5), GetClass::AsIsWalk);
        assert_eq!(classify_probes_as_is(900, 5), GetClass::Best);
        assert_eq!(BALANCE_SHAPES.len(), 6);
        assert!(BALANCE_SHAPES.contains(&"ycsb_b_mc4"));
    }
}
