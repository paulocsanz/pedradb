//! RFC-0184 P2.35 — compositional get clock: [`GetWork`] × [`MachineSpec`].
//!
//! Discrete work (probes, bloom `k`, `⌈log₂ n_blocks⌉`, FNV bytes, CRC 4 KiB)
//! is exact from the algorithm and the YCSB key layout (`ycsb/{i:06}`).
//! Cache **capacity** miss is exact given a named spec (working set vs
//! L1/L2/L3/DRAM). Wall-clock of a compiled get is **not**: OOO, turbo,
//! preemption, NVMe queue, and η are host state. This kernel is the spec
//! lower bound; RFC-0176 `P·τ` stays the calibrated envelope.
//!
//! Twin teeth: [`cache_level_as_is`] always returns DRAM (the single-τ lie).

#![forbid(unsafe_code)]

use crate::bloom::DEFAULT_BITS_PER_KEY;
use crate::scale_kernel::{
    scale_forecast_as_is_with, scale_forecast_with, SCALE_BLOCK_BYTES, SCALE_INDEX_SAMPLE_BYTES,
    SCALE_L1_BYTES, SCALE_TAU_DISK_NS,
};

/// Intel 4 GHz server-class spec (SDM / Agner Fog Skylake-family latencies).
///
/// Cycles → ns at 4.0 GHz. L2 = 1 MiB (server), L3 = 16 MiB **process share**
/// (not the whole die). `pread_4k_ns` is [`SCALE_TAU_DISK_NS`] (SSD class),
/// not an SDM number. FNV-1a is a dependent `xor+imul r64` chain (~4 c/B).
pub const INTEL_SERVER_4GHZ: MachineSpec = MachineSpec {
    name: "intel_server_4ghz",
    l1_bytes: 32 * 1024,
    l2_bytes: 1024 * 1024,
    l3_bytes: 16 * 1024 * 1024,
    l1_ns: 1,
    l2_ns: 3,
    l3_ns: 10,
    dram_ns: 80,
    pread_4k_ns: SCALE_TAU_DISK_NS,
    fnv_ns_per_byte: 1,
    cmp_ns: 1,
    crc32c_ns_per_4k: 200,
    l1_bytes_per_ns: 256,
    l2_bytes_per_ns: 64,
    l3_bytes_per_ns: 32,
    dram_bytes_per_ns: 25,
};

/// Bloom `k = (bpk × 69) / 100` for [`DEFAULT_BITS_PER_KEY`] = 10 → 6.
const BLOOM_NEG_BIT_TESTS: u64 = 2;
/// `(1 − e^{−k/bpk})^k` at bpk=10, k=6, × 1e6.
const BLOOM_FPR_E6: u64 = 8_400;

/// Named cache / ALU / IO latencies. Fields are ns or bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineSpec {
    /// Stable token (`intel_server_4ghz`).
    pub name: &'static str,
    /// L1d size (bytes).
    pub l1_bytes: u64,
    /// L2 size (bytes).
    pub l2_bytes: u64,
    /// L3 process-share (bytes).
    pub l3_bytes: u64,
    /// L1 hit (ns).
    pub l1_ns: u64,
    /// L2 hit (ns).
    pub l2_ns: u64,
    /// L3 hit (ns).
    pub l3_ns: u64,
    /// DRAM open-page (ns).
    pub dram_ns: u64,
    /// 4 KiB positioned `pread` (ns).
    pub pread_4k_ns: u64,
    /// FNV-1a dependent imul chain (ns / key byte).
    pub fnv_ns_per_byte: u64,
    /// Short-key ALU compare, cache charged separately (ns).
    pub cmp_ns: u64,
    /// `crc32c` of one 4 KiB block (ns).
    pub crc32c_ns_per_4k: u64,
    /// L1 sequential fill bandwidth (bytes / ns).
    pub l1_bytes_per_ns: u64,
    /// L2 sequential fill bandwidth (bytes / ns).
    pub l2_bytes_per_ns: u64,
    /// L3 sequential fill bandwidth (bytes / ns).
    pub l3_bytes_per_ns: u64,
    /// DRAM sequential fill bandwidth (bytes / ns).
    pub dram_bytes_per_ns: u64,
}

impl MachineSpec {
    /// Hit latency of `lvl`. Disk uses `pread_4k_ns`.
    #[must_use]
    pub const fn hit_ns(self, lvl: CacheLevel) -> u64 {
        match lvl {
            CacheLevel::L1 => self.l1_ns,
            CacheLevel::L2 => self.l2_ns,
            CacheLevel::L3 => self.l3_ns,
            CacheLevel::Dram => self.dram_ns,
            CacheLevel::Disk => self.pread_4k_ns,
        }
    }

    fn fill_ns(self, lvl: CacheLevel, bytes: u64) -> u64 {
        let bpn = match lvl {
            CacheLevel::L1 => self.l1_bytes_per_ns,
            CacheLevel::L2 => self.l2_bytes_per_ns,
            CacheLevel::L3 => self.l3_bytes_per_ns,
            CacheLevel::Dram => self.dram_bytes_per_ns,
            CacheLevel::Disk => 1,
        };
        bytes.div_ceil(bpn.max(1))
    }
}

/// Where a working set sits (capacity, not conflict).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLevel {
    /// ≤ L1d.
    L1,
    /// ≤ L2.
    L2,
    /// ≤ L3 process share.
    L3,
    /// Fits RAM, misses LLC.
    Dram,
    /// Store misses the WARM cap — 4 KiB `pread`.
    Disk,
}

impl CacheLevel {
    /// Stable token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::L1 => "l1",
            Self::L2 => "l2",
            Self::L3 => "l3",
            Self::Dram => "dram",
            Self::Disk => "disk",
        }
    }
}

/// Block-cache case. Blooms/index still follow working-set vs spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheCase {
    /// Repeated get / zipf last-get: the data block is L1.
    Happy,
    /// Capacity: block is DRAM if the store fits WARM, else Disk.
    Capacity,
    /// Data via 4 KiB `pread` (cold miss).
    Cold,
}

impl CacheCase {
    /// Stable token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Happy => "happy",
            Self::Capacity => "capacity",
            Self::Cold => "cold",
        }
    }

    /// Parse CLI `--cache`. `None` if unknown.
    #[must_use]
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "happy" => Self::Happy,
            "capacity" => Self::Capacity,
            "cold" => Self::Cold,
            _ => return None,
        })
    }
}

/// Dominant composed stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetStage {
    /// Bloom hashes + bit tests + bounds.
    Bloom,
    /// SST index `partition_point`.
    Index,
    /// CRC + entry walk of the data block.
    Block,
    /// 4 KiB `pread`.
    Pread,
}

impl GetStage {
    /// Stable token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Bloom => "bloom",
            Self::Index => "index",
            Self::Block => "block",
            Self::Pread => "pread",
        }
    }
}

/// Discrete work of **one** point get (key exists, memtable empty after settle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetWork {
    /// Files bloom-gated (legal \(P\) or as-is \(N_{\mathrm{files}}\)).
    pub probes: u64,
    /// YCSB / scale user-key bytes.
    pub key_bytes: u64,
    /// Bloom hash probes (`k`).
    pub bloom_k: u64,
    /// Filter bytes per L1-sized file.
    pub bloom_bytes_per_file: u64,
    /// `⌈log₂ n_blocks⌉` index compares on a bloom hit.
    pub index_cmps: u64,
    /// Sparse-index bytes per L1-sized file.
    pub index_bytes_per_file: u64,
    /// Data blocks in an L1 file.
    pub n_blocks: u64,
    /// `⌈log₂ entries_per_block⌉`.
    pub block_cmps: u64,
    /// Data-block bytes ([`SCALE_BLOCK_BYTES`]).
    pub block_bytes: u64,
    /// Entries in one data block.
    pub entries_per_block: u64,
}

/// Spec-ns of one get, split by stage. Glue (snapshot, version-set, η) is **not**
/// included — this is the lower bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetCost {
    /// Bounds + FNV + bit tests over `probes` files.
    pub bloom_ns: u64,
    /// Index binary search on bloom hits.
    pub index_ns: u64,
    /// CRC + walk of the data block (no `pread`).
    pub block_ns: u64,
    /// 4 KiB `pread` (0 unless the block is Disk).
    pub pread_ns: u64,
    /// Sum of the four stages.
    pub total_ns: u64,
    /// Bloom-array capacity level.
    pub bloom_level: CacheLevel,
    /// Index-array capacity level.
    pub index_level: CacheLevel,
    /// Data-block level.
    pub block_level: CacheLevel,
    /// Largest stage.
    pub dominant: GetStage,
}

/// Legal \(P\) and as-is \(N_{\mathrm{files}}\) composed clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposedGet {
    /// Work at legal \(P_{\mathrm{best}}\).
    pub work_legal: GetWork,
    /// Work walking every L1-sized file.
    pub work_as_is: GetWork,
    /// Spec-ns at legal \(P\).
    pub cost_legal: GetCost,
    /// Spec-ns at \(N_{\mathrm{files}}\).
    pub cost_as_is: GetCost,
    /// Spec token.
    pub spec_name: &'static str,
    /// Block-cache case.
    pub cache_case: CacheCase,
}

impl ComposedGet {
    /// One log line.
    #[must_use]
    pub fn line(self) -> String {
        format!(
            "composed spec={} cache={} legal={} as_is={} bloom={} index={} block={} pread={} bloom_lvl={} block_lvl={} dominant={}",
            self.spec_name,
            self.cache_case.token(),
            self.cost_legal.total_ns,
            self.cost_as_is.total_ns,
            self.cost_legal.bloom_ns,
            self.cost_legal.index_ns,
            self.cost_legal.block_ns,
            self.cost_legal.pread_ns,
            self.cost_legal.bloom_level.token(),
            self.cost_legal.block_level.token(),
            self.cost_legal.dominant.token()
        )
    }
}

/// First cache level whose size ≥ `working_set` (capacity miss). `0` → L1.
#[must_use]
pub fn cache_level(working_set: u64, spec: MachineSpec) -> CacheLevel {
    if working_set <= spec.l1_bytes {
        CacheLevel::L1
    } else if working_set <= spec.l2_bytes {
        CacheLevel::L2
    } else if working_set <= spec.l3_bytes {
        CacheLevel::L3
    } else {
        CacheLevel::Dram
    }
}

/// AS-IS: ignore the spec, always DRAM (the single-τ lie).
#[must_use]
pub fn cache_level_as_is(_working_set: u64, _spec: MachineSpec) -> CacheLevel {
    CacheLevel::Dram
}

/// YCSB user-key bytes: `ycsb/{i:06}` (`:06` pad, then extra digits).
#[must_use]
pub fn ycsb_key_bytes(keys: u64) -> u64 {
    let n = keys.saturating_sub(1).max(1);
    5u64.saturating_add(decimal_digits(n).max(6))
}

/// Discrete work for `probes` covering (or walked) files.
#[must_use]
pub fn get_work(probes: u64, bytes_per_entry: u64, key_bytes: u64) -> GetWork {
    let bpe = bytes_per_entry.max(1);
    let n_blocks = if SCALE_BLOCK_BYTES == 0 {
        0
    } else {
        SCALE_L1_BYTES / SCALE_BLOCK_BYTES
    };
    let entries = SCALE_BLOCK_BYTES / bpe;
    let keys_per_file = SCALE_L1_BYTES / bpe;
    let bpk = u64::try_from(DEFAULT_BITS_PER_KEY).unwrap_or(10);
    GetWork {
        probes,
        key_bytes: key_bytes.max(1),
        bloom_k: bloom_k(bpk),
        bloom_bytes_per_file: keys_per_file.saturating_mul(bpk) / 8,
        index_cmps: log2_ceil(n_blocks),
        index_bytes_per_file: n_blocks.saturating_mul(SCALE_INDEX_SAMPLE_BYTES),
        n_blocks,
        block_cmps: log2_ceil(entries.max(1)),
        block_bytes: SCALE_BLOCK_BYTES,
        entries_per_block: entries,
    }
}

/// Spec-ns of `work` on `spec` under `case`. `store_hot` = store fits WARM.
#[must_use]
pub fn cost_of(work: GetWork, spec: MachineSpec, case: CacheCase, store_hot: bool) -> GetCost {
    let bloom_ws = work.probes.saturating_mul(work.bloom_bytes_per_file);
    let index_ws = work.probes.saturating_mul(work.index_bytes_per_file);
    let bloom_level = cache_level(bloom_ws, spec);
    let index_level = cache_level(index_ws, spec);
    let block_level = match case {
        CacheCase::Happy => CacheLevel::L1,
        CacheCase::Capacity => {
            if store_hot {
                CacheLevel::Dram
            } else {
                CacheLevel::Disk
            }
        }
        CacheCase::Cold => CacheLevel::Disk,
    };
    let bloom_hit = spec.hit_ns(bloom_level);
    let index_hit = spec.hit_ns(index_level);
    let rejects = work.probes.saturating_sub(1);

    // Bounds keys live on the `SstTable` object — L1 after the first touch.
    let bounds_ns = work
        .probes
        .saturating_mul(2)
        .saturating_mul(spec.cmp_ns.saturating_add(spec.l1_ns));
    // Two FNV-1a hashes over the user key, per file (ALU; key is L1).
    let hash_ns = work
        .probes
        .saturating_mul(2)
        .saturating_mul(work.key_bytes)
        .saturating_mul(spec.fnv_ns_per_byte);
    // Random bit tests in the filter array.
    let neg_bits_ns = rejects
        .saturating_mul(BLOOM_NEG_BIT_TESTS)
        .saturating_mul(bloom_hit);
    let pos_bits_ns = work.bloom_k.saturating_mul(bloom_hit);
    let bloom_ns = bounds_ns
        .saturating_add(hash_ns)
        .saturating_add(neg_bits_ns)
        .saturating_add(pos_bits_ns);

    let fp_e6 = BLOOM_FPR_E6.saturating_mul(rejects);
    let index_one = work
        .index_cmps
        .saturating_mul(spec.cmp_ns.saturating_add(index_hit));
    let index_ns = index_one.saturating_add(index_one.saturating_mul(fp_e6) / 1_000_000);

    let crc = spec.crc32c_ns_per_4k;
    let (block_mem, pread_ns) = match block_level {
        CacheLevel::Disk => (crc, spec.pread_4k_ns),
        lvl => {
            let fill = spec
                .hit_ns(lvl)
                .saturating_add(spec.fill_ns(lvl, work.block_bytes));
            (crc.max(fill), 0)
        }
    };
    let walk = work
        .block_cmps
        .saturating_mul(spec.cmp_ns.saturating_add(spec.l1_ns));
    let block_one = block_mem.saturating_add(walk);
    let block_ns = block_one.saturating_add(block_one.saturating_mul(fp_e6) / 1_000_000);

    let total = bloom_ns
        .saturating_add(index_ns)
        .saturating_add(block_ns)
        .saturating_add(pread_ns);
    let dominant = dominant_stage(bloom_ns, index_ns, block_ns, pread_ns);
    GetCost {
        bloom_ns,
        index_ns,
        block_ns,
        pread_ns,
        total_ns: total,
        bloom_level,
        index_level,
        block_level,
        dominant,
    }
}

/// Compose legal \(P\) and as-is \(N_{\mathrm{files}}\) clocks without a get.
#[must_use]
pub fn predict_get_composed(
    keys: u64,
    ram: u64,
    bytes_per_entry: u64,
    spec: MachineSpec,
    case: CacheCase,
) -> ComposedGet {
    let bpe = bytes_per_entry.max(1);
    let legal = scale_forecast_with(keys, ram, bpe);
    let as_is = scale_forecast_as_is_with(keys, ram, bpe);
    let key_bytes = ycsb_key_bytes(keys);
    let work_legal = get_work(legal.p_best, bpe, key_bytes);
    let work_as_is = get_work(as_is.n_files, bpe, key_bytes);
    ComposedGet {
        work_legal,
        work_as_is,
        cost_legal: cost_of(work_legal, spec, case, legal.hot),
        cost_as_is: cost_of(work_as_is, spec, case, legal.hot),
        spec_name: spec.name,
        cache_case: case,
    }
}

/// YCSB mix: `read_pct` gets + the rest writes. Harness schedule is
/// xorshift/`0x5EED_0001` — the mix is exact, not sampled.
#[must_use]
pub fn mix_ns(read_pct: u64, get_ns: u64, write_ns: u64) -> u64 {
    let r = read_pct.min(100);
    let g = u128::from(get_ns).saturating_mul(u128::from(r));
    let w = u128::from(write_ns).saturating_mul(u128::from(100u64.saturating_sub(r)));
    u64::try_from((g.saturating_add(w)) / 100).unwrap_or(u64::MAX)
}

fn bloom_k(bpk: u64) -> u64 {
    let raw = bpk.saturating_mul(69) / 100;
    if raw < 1 {
        1
    } else if raw > 30 {
        30
    } else {
        raw
    }
}

fn log2_ceil(n: u64) -> u64 {
    if n <= 1 {
        0
    } else {
        u64::from(64 - (n - 1).leading_zeros())
    }
}

fn decimal_digits(mut n: u64) -> u64 {
    let mut d = 1u64;
    while n >= 10 {
        n /= 10;
        d = d.saturating_add(1);
    }
    d
}

fn dominant_stage(bloom: u64, index: u64, block: u64, pread: u64) -> GetStage {
    let mut best = GetStage::Bloom;
    let mut ns = bloom;
    if index > ns {
        ns = index;
        best = GetStage::Index;
    }
    if block > ns {
        ns = block;
        best = GetStage::Block;
    }
    if pread > ns {
        best = GetStage::Pread;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale_kernel::{scale_forecast, SCALE_BYTES_PER_ENTRY, SCALE_TAU_RAM_NS};

    const RAM_64GI: u64 = 64 << 30;

    #[test]
    fn get_work_counts_bloom_k_and_index_log() {
        let w = get_work(4, SCALE_BYTES_PER_ENTRY, 13);
        assert_eq!(w.probes, 4);
        assert_eq!(w.bloom_k, 6, "bpk=10 → k=(10*69)/100=6");
        assert_eq!(w.n_blocks, 65_536);
        assert_eq!(w.index_cmps, 16);
        assert_eq!(w.block_bytes, 4_096);
        assert_eq!(w.entries_per_block, 4_096 / SCALE_BYTES_PER_ENTRY);
        assert_eq!(w.block_cmps, 4);
        assert!(w.bloom_bytes_per_file > 1_000_000 && w.bloom_bytes_per_file < 2_000_000);
    }

    #[test]
    fn ycsb_key_bytes_padded_six() {
        assert_eq!(ycsb_key_bytes(1), 11);
        assert_eq!(ycsb_key_bytes(1_000_000), 11);
        assert_eq!(ycsb_key_bytes(1_000_001), 12);
        assert_eq!(ycsb_key_bytes(1_000_000_000), 14); // i=999_999_999
        assert_eq!(ycsb_key_bytes(1_000_000_001), 15);
    }

    #[test]
    fn cache_level_on_always_dram_is_not_ok() {
        let s = INTEL_SERVER_4GHZ;
        assert_eq!(cache_level(16 * 1024, s), CacheLevel::L1);
        assert_eq!(cache_level(512 * 1024, s), CacheLevel::L2);
        assert_eq!(cache_level(4 * 1024 * 1024, s), CacheLevel::L3);
        assert_eq!(cache_level(64 * 1024 * 1024, s), CacheLevel::Dram);
        assert_eq!(cache_level_as_is(16 * 1024, s), CacheLevel::Dram);
        assert_ne!(cache_level(16 * 1024, s), cache_level_as_is(16 * 1024, s));
    }

    #[test]
    fn happy_is_faster_than_capacity_is_faster_than_cold() {
        let h = predict_get_composed(
            50_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Happy,
        );
        let c = predict_get_composed(
            50_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Capacity,
        );
        let d = predict_get_composed(
            50_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Cold,
        );
        assert_eq!(h.cost_legal.block_level, CacheLevel::L1);
        assert_eq!(c.cost_legal.block_level, CacheLevel::Dram);
        assert_eq!(d.cost_legal.block_level, CacheLevel::Disk);
        assert!(
            h.cost_legal.total_ns <= c.cost_legal.total_ns
                && c.cost_legal.total_ns < d.cost_legal.total_ns,
            "happy={} cap={} cold={}",
            h.cost_legal.total_ns,
            c.cost_legal.total_ns,
            d.cost_legal.total_ns
        );
        assert_eq!(d.cost_legal.pread_ns, SCALE_TAU_DISK_NS);
        assert_eq!(h.cost_legal.pread_ns, 0);
        assert_eq!(c.cost_legal.pread_ns, 0);
    }

    #[test]
    fn composed_does_not_charge_disk_on_bloom_reject() {
        let d = predict_get_composed(
            1_000_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Capacity,
        );
        assert!(
            !scale_forecast(1_000_000_000, RAM_64GI).hot,
            "1B @ 64 GiB is bounded-cache"
        );
        assert_eq!(d.cost_legal.block_level, CacheLevel::Disk);
        assert_eq!(
            d.cost_legal.pread_ns, SCALE_TAU_DISK_NS,
            "one hit block, not P preads"
        );
        assert!(
            d.cost_legal.pread_ns < SCALE_TAU_DISK_NS.saturating_mul(d.work_legal.probes),
            "0176 charged τ_disk on every probe"
        );
        assert_eq!(d.cost_legal.dominant, GetStage::Pread);
    }

    #[test]
    fn as_is_walk_is_bloom_bound_when_hot() {
        let c = predict_get_composed(
            50_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Capacity,
        );
        assert!(c.work_as_is.probes > c.work_legal.probes.saturating_mul(2));
        assert!(c.cost_as_is.total_ns > c.cost_legal.total_ns);
        assert_eq!(c.cost_as_is.dominant, GetStage::Bloom);
        assert_eq!(c.cost_as_is.bloom_level, CacheLevel::Dram);
        assert_eq!(c.cost_legal.bloom_level, CacheLevel::L3);
    }

    #[test]
    fn composed_50m_is_below_0176_envelope() {
        let c = predict_get_composed(
            50_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Capacity,
        );
        let envelope = crate::scale_kernel::predict_get_ns(
            c.work_legal.probes,
            SCALE_TAU_RAM_NS,
            SCALE_TAU_DISK_NS,
            crate::scale_kernel::SCALE_BPS,
            0,
        );
        assert_eq!(c.work_legal.probes, 4);
        assert!(
            c.cost_legal.total_ns < envelope,
            "composed {} vs 0176 {}",
            c.cost_legal.total_ns,
            envelope
        );
        assert!(
            c.cost_legal.total_ns > 200,
            "not a zero-work stub: {}",
            c.cost_legal.total_ns
        );
        // 50M get_hit calibration 4.3 µs lives in the 0176 envelope, not in
        // the spec lower bound (glue / η / OOO are not SDM numbers).
        assert_eq!(envelope, 4_400);
    }

    #[test]
    fn mix_ns_ycsb_a_is_half() {
        assert_eq!(mix_ns(50, 1_000, 3_000), 2_000);
        assert_eq!(mix_ns(100, 1_000, 3_000), 1_000);
        assert_eq!(mix_ns(0, 1_000, 3_000), 3_000);
    }

    #[test]
    fn composed_1b_as_is_is_bloom_not_one_pread() {
        let c = predict_get_composed(
            1_000_000_000,
            RAM_64GI,
            SCALE_BYTES_PER_ENTRY,
            INTEL_SERVER_4GHZ,
            CacheCase::Capacity,
        );
        assert!(c.work_as_is.probes >= 900);
        assert_eq!(c.cost_as_is.dominant, GetStage::Bloom);
        assert!(
            c.cost_as_is.bloom_ns > c.cost_as_is.pread_ns,
            "walk-all is bloom DRAM, not one disk"
        );
    }
}
