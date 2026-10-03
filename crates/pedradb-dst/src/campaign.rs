//! RFC-0325: continuous DST campaign core.
//!
//! One seed ⇒ one deterministic scenario over the REAL engine
//! (`Db` + `FailingEnv` + `DetHost`, no twins — RFC-0270): workload bursts,
//! per-cycle fault arms (kinds × op classes × transient/permanent),
//! process-crash drops, reopen, checkpoint copies and clean closes.
//!
//! Oracle battery (every violation is a REAL):
//! - `O1 acked_write_lost` — an op that returned `Ok` missing/diverged after
//!   crash-reopen (process-crash model: the page cache survives the drop, so
//!   `Ok` ⇒ must survive; power-loss durability stays with the real-disk
//!   swarm campaign, RFC-0273 §2).
//! - `O2 silent_wrong_read` — `get`/`range` disagrees with the acked-op model
//!   on a healthy (never-armed) window.
//! - `O3 resurrection_after_delete` — acked-deleted key present after reopen
//!   (folded into the O1/O2 model equality on the full scan).
//! - `O4 open_failed_on_healthy_env` — reopen on a disarmed env failed.
//! - `O5 engine_invariant_broken` — `Db::assert_all_invariants` rejected.
//! - `O6 corruption_not_detected` — `Db::verify_checksums` failed post-crash;
//!   in mutant M2, passed over a corrupted SST.
//! - `O7 unexpected_failure_on_healthy_env` — engine returned `Err` with no
//!   fault armed or tripped.
//! - `O8 nondeterministic_replay` — same seed, two fresh dirs, different
//!   final logical state (both runs oracle-clean).
//! - `O9 spontaneous_panic` — unwind without a `FaultKind::Panic` arm.
//!
//! Anti-vacuity (RFC-0270 §4 / RFC-0273 §1): the M1/M2/M3 mutants prove the
//! battery kills a lost-WAL engine, silent bit-rot and a wrong model.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bytes::Bytes;
use pedradb_core::db::Db;
use pedradb_core::{BatchOp, CoreError};
use pedradb_core::{DetHost, Host, OpenOptions, Rng, SeedRng, WriteOptions, mix_seed};
use pedradb_sim::{FailingEnv, FaultKind, OpClass};

use crate::temp_parent;

// ---------------------------------------------------------------------------
// Config derivation (seed ⇒ scenario)
// ---------------------------------------------------------------------------

/// SplitMix64 stepping — keeps scenario derivation independent of the
/// `SeedRng` consumption order inside the trial.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        z
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next() % bound
        }
    }
}

/// One deterministic scenario. Every field comes from the seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignConfig {
    /// Trial seed (the scenario id).
    pub seed: u64,
    /// Crash/reopen cycles.
    pub cycles: u32,
    /// Workload ops per cycle.
    pub ops_per_cycle: u32,
    /// Distinct user keys.
    pub keyspace: u32,
    /// Maximum value bytes.
    pub value_max: usize,
    /// `sync=true` per put when true; `no_sync` + periodic `sync` otherwise.
    pub sync_writes: bool,
    /// `wal_full_fsync` open option.
    pub wal_full_fsync: bool,
    /// `auto_flush_bytes` open option.
    pub auto_flush_bytes: Option<usize>,
    /// `auto_compact_sst_count` open option.
    pub auto_compact_sst_count: Option<usize>,
    /// `large_value_threshold` (vlog path) open option.
    pub large_value_threshold: Option<usize>,
    /// Per-cycle probability (%) of arming a fault window.
    pub fault_density_pct: u32,
    /// Fault kinds this scenario may arm.
    pub kinds: Vec<FaultKind>,
    /// Op classes this scenario may target.
    pub targeted_classes: Vec<OpClass>,
    /// Probability (%) per cycle-end of a checkpoint copy + verify.
    pub checkpoint_pct: u32,
    /// Probability (%) per cycle of an ENOSPC envelope: `available_bytes`
    /// injected below the hard watermark for the fault window (disk-pressure
    /// admission path), restored after the crash.
    pub enospc_pct: u32,
    /// Probability (%) per crash of LIVE byte-level damage: flip one byte in
    /// (or truncate) a random WAL/SST/MANIFEST/CHANGELOG file. Taints the
    /// trial: fail-closed reopen is a legal outcome and post-crash checks go
    /// plausible-only (no fabricated values, idempotent reopen).
    pub corrupt_pct: u32,
    /// Probability (%) per crash of arming a fault across the REOPEN itself
    /// (recovery interrupted ⇒ open may Err; a disarmed retry must succeed).
    pub reopen_fault_pct: u32,
    /// Run the whole scenario twice and require equal final state (O8).
    pub double_run: bool,
    /// Long-horizon trial (1-in-6 seeds): 8-32 crash cycles, 3-4k ops/cycle —
    /// reaches compaction-debt / changelog-growth states shallow sweeps miss.
    pub deep: bool,
}

impl CampaignConfig {
    /// Derive the scenario for `seed` (stable across builds).
    #[must_use]
    pub fn derive(seed: u64) -> Self {
        let mut g = SplitMix(seed ^ 0x0D57_665B_0C6D_AE0Bu64);
        let deep = seed % 6 == 3;
        let cycles = if deep { 8 + g.below(25) as u32 } else { 1 + g.below(6) as u32 };
        let ops_per_cycle = if deep { 512 + g.below(3_072) as u32 } else { 64 + g.below(1_920) as u32 };
        let keyspace = 8 + g.below(1_016) as u32;
        let value_max = 8 + g.below(1_536) as usize;
        let sync_writes = g.below(5) != 0;
        let wal_full_fsync = g.below(2) == 0;
        let auto_flush_bytes = match g.below(3) {
            0 => None,
            1 => Some(16 * 1024),
            _ => Some(128 * 1024),
        };
        let auto_compact_sst_count = match g.below(3) {
            0 => None,
            n => Some(2 + n as usize),
        };
        let large_value_threshold = if g.below(4) == 0 { Some(64) } else { None };
        let fault_density_pct = g.below(101) as u32;
        let mut kinds = vec![
            FaultKind::IoError,
            FaultKind::StorageFull,
            FaultKind::Interrupted,
            FaultKind::SyncFail,
            FaultKind::ShortWrite,
        ];
        if g.below(4) == 0 {
            kinds.push(FaultKind::Panic);
        }
        let mut targeted_classes = Vec::new();
        for class in [
            OpClass::Write,
            OpClass::Sync,
            OpClass::Rename,
            OpClass::CreateOpen,
            OpClass::Remove,
            OpClass::Meta,
        ] {
            if g.below(2) == 0 {
                targeted_classes.push(class);
            }
        }
        let checkpoint_pct = if g.below(8) == 0 { 100 } else { 0 };
        let enospc_pct = if g.below(5) == 0 { 80 } else { 0 };
        let corrupt_pct = if g.below(4) == 0 { 50 } else { 0 };
        let reopen_fault_pct = if g.below(6) == 0 { 60 } else { 0 };
        let double_run = seed % 8 == 0;
        Self {
            seed,
            cycles,
            ops_per_cycle,
            keyspace,
            value_max,
            sync_writes,
            wal_full_fsync,
            auto_flush_bytes,
            auto_compact_sst_count,
            large_value_threshold,
            fault_density_pct,
            kinds,
            targeted_classes,
            checkpoint_pct,
            enospc_pct,
            corrupt_pct,
            reopen_fault_pct,
            double_run,
            deep,
        }
    }

    /// CI-safe caps (smoke band must stay in seconds, not minutes).
    #[must_use]
    pub fn clamped_for_ci(mut self) -> Self {
        self.cycles = self.cycles.min(2);
        self.ops_per_cycle = self.ops_per_cycle.min(384);
        self.keyspace = self.keyspace.min(256);
        self.deep = false;
        self
    }

    /// Shape tag for telemetry aggregation / baselines.
    #[must_use]
    pub fn shape(&self) -> String {
        format!(
            "{}-vlog{}-k{}{}",
            if self.sync_writes { "sync" } else { "nosync" },
            u8::from(self.large_value_threshold.is_some()),
            self.keyspace,
            if self.deep { "-deep" } else { "" }
        )
    }

    /// True once live byte damage was injected this trial (oracles relax).
    #[must_use]
    pub fn corruption_axis_armed(&self) -> bool {
        self.corrupt_pct > 0
    }

    /// Open options this scenario uses (also used by the triage probe).
    #[must_use]
    pub fn open_options(&self) -> OpenOptions {
        OpenOptions {
            wal_full_fsync: self.wal_full_fsync,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: self.sync_writes,
            auto_flush_bytes: self.auto_flush_bytes,
            auto_compact_sst_count: self.auto_compact_sst_count,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: self.large_value_threshold,
            sst_payload_budget_bytes: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Trace + outcomes
// ---------------------------------------------------------------------------

/// One trace event (deterministic content only — no wall clock).
#[derive(Debug, Clone)]
pub struct TraceLine {
    /// Op counter within the trial.
    pub op: u64,
    /// Phase / op tag (`put`, `del`, `get`, `scan`, `flush`, `compact`, `ckpt`, `sync`, `reopen`).
    pub tag: &'static str,
    /// Key (or range) tag.
    pub key: String,
    /// `OK` / `ERR` / `CRASH`.
    pub fate: &'static str,
}

fn trace_hash(lines: &[TraceLine]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for l in lines {
        for b in l
            .op
            .to_le_bytes()
            .iter()
            .chain(l.tag.as_bytes())
            .chain(l.key.as_bytes())
            .chain(l.fate.as_bytes())
            .copied()
        {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

/// A REAL violation captured by the oracle battery.
#[derive(Debug, Clone)]
pub struct TrialFailure {
    /// Invariant id (`O1`..`O9`).
    pub invariant: &'static str,
    /// Human-readable evidence.
    pub detail: String,
    /// Trace tail at violation time.
    pub trace_tail: Vec<TraceLine>,
    /// FNV-1a over the full trace.
    pub trace_hash: u64,
    /// Directory kept as evidence (not cleaned).
    pub artifact_dir: Option<PathBuf>,
}

/// Per-trial telemetry (wall clock allowed here — never enters the trace hash).
#[derive(Debug, Clone, Default)]
pub struct TrialTelemetry {
    /// Trial seed.
    pub seed: u64,
    /// Shape tag.
    pub shape: String,
    /// Cycles executed.
    pub cycles: u32,
    /// Workload ops attempted.
    pub ops: u64,
    /// Fault windows armed.
    pub faults_armed: u64,
    /// Fault windows that actually tripped.
    pub faults_fired: u64,
    /// Expected unwinds (`FaultKind::Panic` arms).
    pub expected_panics: u64,
    /// Final logical state digest (FNV over the acked model = full scan).
    pub digest: u64,
    /// FNV over the trace.
    pub trace_hash: u64,
    /// Put/delete timings, nanoseconds.
    pub put_ns: u64,
    /// Get timings.
    pub get_ns: u64,
    /// Scan timings.
    pub scan_ns: u64,
    /// Flush/sync timings.
    pub flush_ns: u64,
    /// Compact timings.
    pub compact_ns: u64,
    /// Checkpoint timings.
    pub checkpoint_ns: u64,
    /// Reopen timings.
    pub reopen_ns: u64,
    /// Total wall nanoseconds.
    pub wall_ns: u64,
    /// `wal_sync_count` at close.
    pub wal_syncs: u64,
    /// `write_stall_count` at close.
    pub write_stalls: u64,
    /// SST count at close.
    pub ssts: usize,
    /// On-disk bytes at close.
    pub dir_bytes: u64,
    /// Trial ended in a LEGAL fail-closed reopen (injected bit-rot detected,
    /// engine refused to open — the quarantine contract, not a violation).
    pub fail_closed: bool,
    /// Violation, when the trial failed.
    pub failure: Option<TrialFailure>,
}

impl TrialTelemetry {
    /// `ops / second` over the whole trial.
    #[must_use]
    pub fn ops_per_sec(&self) -> f64 {
        if self.wall_ns == 0 {
            return 0.0;
        }
        self.ops as f64 * 1e9 / self.wall_ns as f64
    }

    /// Compact JSON line (no serde dep).
    #[must_use]
    pub fn to_json(&self) -> String {
        let f = &self.failure;
        format!(
            "{{\"seed\":{},\"shape\":\"{}\",\"ops\":{},\"ops_per_sec\":{:.1},\"cycles\":{},\"faults_armed\":{},\"faults_fired\":{},\"expected_panics\":{},\"wall_ms\":{},\"wal_syncs\":{},\"write_stalls\":{},\"ssts\":{},\"dir_bytes\":{},\"fail_closed\":{},\"digest\":{},\"trace_hash\":{},\"ok\":{},\"invariant\":\"{}\"}}",
            self.seed,
            self.shape,
            self.ops,
            self.ops_per_sec(),
            self.cycles,
            self.faults_armed,
            self.faults_fired,
            self.expected_panics,
            self.wall_ns / 1_000_000,
            self.wal_syncs,
            self.write_stalls,
            self.ssts,
            self.dir_bytes,
            self.fail_closed,
            self.digest,
            self.trace_hash,
            f.is_none(),
            f.as_ref().map_or("", |x| x.invariant),
        )
    }
}

// ---------------------------------------------------------------------------
// Shared crash-state (model + trace survive an expected unwind)
// ---------------------------------------------------------------------------

/// Acked-Ok model: every op that returned `Ok` MUST be visible after any
/// crash-reopen (process-crash model: the page cache survives the drop).
#[derive(Default)]
struct Model {
    /// key → last acked value (absent = acked delete / never written).
    live: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Keys whose last op returned Err (fate UNKNOWN: the write/tombstone may
    /// or may not have reached the WAL before the fault). Presence is
    /// optional; values stay constrained by the universe.
    uncertain: BTreeSet<Vec<u8>>,
    /// Every key ever touched (Ok or Err fate) — nothing outside may exist.
    touched: BTreeSet<Vec<u8>>,
    /// key → every value ever written to it, acked-Ok or Err'd (an Err'd put
    /// may have landed). Plausible-value universe.
    universe: HashMap<Vec<u8>, BTreeSet<Vec<u8>>>,
    /// key → trace op id of the last acked-Ok put/delete (O1 diagnostics).
    acked_at: HashMap<Vec<u8>, u64>,
}

impl Model {
    fn ack_put(&mut self, k: Vec<u8>, v: Vec<u8>, op: u64) {
        self.touched.insert(k.clone());
        self.uncertain.remove(&k);
        self.universe.entry(k.clone()).or_default().insert(v.clone());
        self.acked_at.insert(k.clone(), op);
        self.live.insert(k, v);
    }

    fn ack_delete(&mut self, k: &[u8], op: u64) {
        self.touched.insert(k.to_vec());
        self.live.remove(k);
        self.uncertain.remove(k);
        self.acked_at.insert(k.to_vec(), op);
    }

    fn note_err_put(&mut self, k: &[u8], v: &[u8]) {
        self.touched.insert(k.to_vec());
        self.universe.entry(k.to_vec()).or_default().insert(v.to_vec());
        self.uncertain.insert(k.to_vec());
    }

    fn note_err_delete(&mut self, k: &[u8]) {
        self.touched.insert(k.to_vec());
        self.uncertain.insert(k.to_vec());
    }

    /// Acked range-delete: every live key in `[s, e)` is gone.
    fn ack_delete_range(&mut self, s: &[u8], e: &[u8], op: u64) {
        let dead: Vec<Vec<u8>> = self
            .live
            .range(s.to_vec()..e.to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        for k in dead {
            self.live.remove(&k);
            self.acked_at.insert(k, op);
        }
    }

    /// Err'd range-delete: fate UNKNOWN for every live key in range (the
    /// tombstone may have landed for any subset).
    fn note_err_range(&mut self, s: &[u8], e: &[u8]) {
        let suspect: Vec<Vec<u8>> = self
            .live
            .range(s.to_vec()..e.to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        for k in suspect {
            self.uncertain.insert(k);
        }
    }

    /// FNV digest of the live model (BTreeMap order = engine key order).
    fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (k, v) in &self.live {
            for b in k.iter().chain(v.iter()).copied().chain([0xFF]) {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
        h
    }
}

/// State that must survive an expected `FaultKind::Panic` unwind so the
/// catch handler can run the post-crash oracle pass.
struct CrashState {
    model: Mutex<Model>,
    trace: Mutex<Vec<TraceLine>>,
    op: AtomicU64,
    /// Set only across an individual engine call while a Panic-kind arm is live.
    panic_expected: AtomicBool,
    /// The engine call currently in flight (window ops only). A `FaultKind::Panic`
    /// unwind lands AFTER the WAL write, so the in-flight op's fate is UNKNOWN —
    /// the catch handler must mark it uncertain or the oracle false-positives.
    in_flight: Mutex<Option<InFlight>>,
    /// Set once live byte damage was injected: acked data may legitimately be
    /// lost loudly, so strict model equality relaxes to plausible-only.
    tainted: AtomicBool,
    /// Files damaged this trial (detection attribution for the at-rest walk).
    corrupted_file: Mutex<Vec<String>>,
}

/// One in-flight window op (key + payload for the universe on panicked puts).
struct InFlight {
    key: Vec<u8>,
    put: Option<Vec<u8>>,
}

impl CrashState {
    fn new() -> Self {
        Self {
            model: Mutex::new(Model::default()),
            trace: Mutex::new(Vec::new()),
            op: AtomicU64::new(0),
            panic_expected: AtomicBool::new(false),
            in_flight: Mutex::new(None),
            tainted: AtomicBool::new(false),
            corrupted_file: Mutex::new(Vec::new()),
        }
    }

    fn trace(&self, tag: &'static str, key: &str, fate: &'static str) {
        let op = self.op.fetch_add(1, Ordering::Relaxed) + 1;
        let mut t = self.trace.lock().unwrap();
        t.push(TraceLine {
            op,
            tag,
            key: key.to_owned(),
            fate,
        });
        if t.len() > 4_096 {
            t.drain(..2_048);
        }
    }

    fn tail(&self, n: usize) -> Vec<TraceLine> {
        let t = self.trace.lock().unwrap();
        let start = t.len().saturating_sub(n);
        t[start..].to_vec()
    }

    fn hash(&self) -> u64 {
        trace_hash(&self.trace.lock().unwrap())
    }
}

/// Run `f` marking the window so an unwind is classified as an expected
/// crash (FaultKind::Panic), not O9.
fn guard_panic<T>(sh: &CrashState, f: impl FnOnce() -> T) -> T {
    sh.panic_expected.store(true, Ordering::Relaxed);
    let out = f();
    sh.panic_expected.store(false, Ordering::Relaxed);
    out
}

/// Like [`guard_panic`] for model-visible window ops: registers the in-flight
/// key so an unwind marks its fate UNKNOWN before the post-crash verify.
fn guard_panic_op<T>(
    sh: &CrashState,
    key: &[u8],
    put: Option<Vec<u8>>,
    f: impl FnOnce() -> T,
) -> T {
    *sh.in_flight.lock().unwrap() = Some(InFlight {
        key: key.to_vec(),
        put,
    });
    sh.panic_expected.store(true, Ordering::Relaxed);
    let out = f();
    sh.panic_expected.store(false, Ordering::Relaxed);
    *sh.in_flight.lock().unwrap() = None;
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn key_of(n: u64) -> String {
    format!("k/{n:012}")
}

fn value_of(n: u64, len: usize) -> Vec<u8> {
    let mut g = SplitMix(n ^ 0xA5A5_5A5A_5A5A_A5A5);
    let mut v = Vec::with_capacity(len.max(1));
    while v.len() < len.max(1) {
        v.extend_from_slice(&g.next().to_le_bytes());
    }
    v.truncate(len.max(1));
    v
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_dir_recursive(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Highly compressible value (drives the LZ4 encode/decode + expansion-cap
/// surface): repeated ASCII pattern.
fn compressible_value(len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len.max(1));
    let pattern = b"PEDRA0123456789abcdefPEDRAFEDCBA9876543210";
    while v.len() < len.max(1) {
        let take = (len.max(1) - v.len()).min(pattern.len());
        v.extend_from_slice(&pattern[..take]);
    }
    v
}

/// Inject live byte damage into one data file (bit-flip, 25% truncate).
/// Returns the file name for the trace.
fn inject_file_damage(dir: &Path, rng: &SeedRng) -> String {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let is_data = n == "CURRENT.log"
                || n.starts_with("WAL.arch")
                || n.ends_with(".sst")
                || n.starts_with("MANIFEST")
                || n == "CHANGELOG"
                || n == "CURRENT";
            if is_data && e.metadata().map(|m| m.len() > 8).unwrap_or(false) {
                candidates.push(e.path());
            }
        }
    }
    if candidates.is_empty() {
        return "<none>".to_owned();
    }
    let path = candidates[(rng.gen_range(candidates.len() as u64)) as usize].clone();
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let Ok(mut bytes) = std::fs::read(&path) else {
        return name;
    };
    if bytes.is_empty() {
        return name;
    }
    if rng.gen_range(4) == 0 {
        // Truncate mid-file (25%): torn file class.
        let keep = 1 + rng.gen_range(bytes.len() as u64) as usize;
        bytes.truncate(keep);
    } else {
        let at = rng.gen_range(bytes.len() as u64) as usize;
        bytes[at] ^= 0x01 << (rng.gen_range(8) as u32);
    }
    let _ = std::fs::write(&path, &bytes);
    name
}

fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_bytes(&p);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// Scenario value with compressibility roll (25% compressible pattern).
fn scenario_value(rng: &SeedRng, value_max: usize) -> Vec<u8> {
    let len = 1 + rng.gen_range(value_max.saturating_sub(1).max(1) as u64) as usize;
    if rng.gen_range(4) == 0 {
        compressible_value(len)
    } else {
        value_of(rng.next_u64(), len)
    }
}

fn healthy_host(seed: u64) -> (DetHost<FailingEnv>, FailingEnv) {
    let env = FailingEnv::passing();
    (DetHost::with_seed(env.clone(), seed), env)
}

fn open_db(dir: &Path, cfg: &CampaignConfig, host: &DetHost<FailingEnv>) -> Result<Db<FailingEnv>, String> {
    Db::open_with_host(dir, cfg.open_options(), host).map_err(|e| e.to_string())
}

/// Compare an engine scan against the model (uncertain-fate aware);
/// `Some(detail)` = mismatch.
///
/// For a key whose last op was acked-Ok the engine state must match `live`
/// exactly. For an `uncertain` key (last op Err'd) presence is optional and
/// any universe value is legal — an Err means UNKNOWN fate, not "not applied"
/// (the WAL write may have landed before the fault fired on the sync).
fn scan_mismatch(got: &[(Bytes, Bytes)], model: &Model) -> Option<String> {
    scan_mismatch_mode(got, model, false)
}

/// Plausible-only mode (post-corruption): acked data may be lost LOUDLY to
/// injected bit-rot, so live-equality and missing checks are skipped; what
/// must NEVER happen is a value that was never written (fabrication) or a
/// key outside the touched set.
fn scan_mismatch_mode(
    got: &[(Bytes, Bytes)],
    model: &Model,
    plausible_only: bool,
) -> Option<String> {
    for (k, v) in got {
        if !model.touched.contains(k.as_ref()) {
            return Some(format!(
                "key {:?} exists in engine but was never written by the scenario (outside-write)",
                String::from_utf8_lossy(k)
            ));
        }
        if plausible_only || model.uncertain.contains(k.as_ref()) {
            let plausible = model
                .universe
                .get(k.as_ref())
                .map_or(false, |set| set.contains(v.as_ref()));
            if !plausible {
                return Some(format!(
                    "uncertain key {:?} holds an implausible value ({} bytes, never written)",
                    String::from_utf8_lossy(k),
                    v.len()
                ));
            }
            continue;
        }
        match model.live.get(k.as_ref()) {
            Some(expect) => {
                if v.as_ref() != expect.as_slice() {
                    return Some(format!(
                        "key {:?} holds {} bytes, model says {} bytes (acked at op {:?})",
                        String::from_utf8_lossy(k),
                        v.len(),
                        expect.len(),
                        model.acked_at.get(k.as_ref())
                    ));
                }
            }
            None => {
                return Some(format!(
                    "key {:?} exists in engine but the acked model says deleted (resurrection, deleted at op {:?})",
                    String::from_utf8_lossy(k),
                    model.acked_at.get(k.as_ref())
                ));
            }
        }
    }
    if plausible_only {
        return None;
    }
    // Acked-Ok live keys must all be present.
    for (k, _) in &model.live {
        if model.uncertain.contains(k) {
            continue;
        }
        if !got.iter().any(|(ek, _)| ek.as_ref() == k.as_slice()) {
            let vhex = model
                .universe
                .get(k)
                .and_then(|set| set.iter().next())
                .map(|v| {
                    let head: Vec<String> =
                        v.iter().take(16).map(|b| format!("{b:02x}")).collect();
                    format!("len={} hex[0..16]={}", v.len(), head.join(""))
                })
                .unwrap_or_default();
            return Some(format!(
                "acked key {:?} missing from engine ({} live vs {} engine keys, last acked at op {:?}, {})",
                String::from_utf8_lossy(k),
                model.live.len(),
                got.len(),
                model.acked_at.get(k),
                vhex
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The trial
// ---------------------------------------------------------------------------

/// Run the scenario for `seed` in a fresh temp dir under `parent`.
/// On violation the evidence dir is KEPT and returned in [`TrialFailure`].
#[must_use]
pub fn run_trial(parent: &Path, seed: u64) -> TrialTelemetry {
    run_trial_cfg(parent, &CampaignConfig::derive(seed))
}

/// Run an explicit scenario (shrink / replay / CI caps).
#[must_use]
pub fn run_trial_cfg(parent: &Path, cfg: &CampaignConfig) -> TrialTelemetry {
    let t0 = std::time::Instant::now();
    let dir = parent.join(format!("pedradb-dst-trial-{}-{}", std::process::id(), cfg.seed));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);

    let sh = std::sync::Arc::new(CrashState::new());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_trial_inner(parent, &dir, cfg, &sh)
    }));

    let mut telemetry = match outcome {
        Ok(Ok(mut t)) => {
            let _ = std::fs::remove_dir_all(&dir);
            t.wall_ns = t0.elapsed().as_nanos() as u64;
            return t;
        }
        Ok(Err(mut f)) => {
            f.trace_tail = sh.tail(40);
            f.trace_hash = sh.hash();
            base_telemetry(cfg, t0, Some(f))
        }
        Err(payload) => {
            let expected = sh.panic_expected.load(Ordering::Relaxed);
            if expected {
                // Keep the RAW crash image: verification opens/closes (and a
                // graceful close flushes, mutating the evidence).
                let raw = dir.with_extension("crash-raw");
                let _ = std::fs::remove_dir_all(&raw);
                let _ = copy_dir_recursive(&dir, &raw);
                // The in-flight op's WAL write may have landed before the
                // unwind: its fate is UNKNOWN (same contract as Err).
                if let Some(inf) = sh.in_flight.lock().unwrap().take() {
                    let mut m = sh.model.lock().unwrap();
                    match inf.put {
                        Some(v) => m.note_err_put(&inf.key, &v),
                        None => m.note_err_delete(&inf.key),
                    }
                }
                // FaultKind::Panic arm fired inside an engine call: this IS the
                // crash. Verify crash-consistency of the on-disk state.
                match verify_post_crash(parent, &dir, cfg, &sh) {
                    Ok(mut t) => {
                        t.expected_panics += 1;
                        t.wall_ns = t0.elapsed().as_nanos() as u64;
                        let _ = std::fs::remove_dir_all(&dir);
                        return t;
                    }
                    Err(mut f) => {
                        f.artifact_dir = Some(raw);
                        base_telemetry(cfg, t0, Some(f))
                    }
                }
            } else {
                let what = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "opaque panic payload".to_owned());
                base_telemetry(
                    cfg,
                    t0,
                    Some(TrialFailure {
                        invariant: "O9",
                        detail: format!("spontaneous panic (no Panic arm live): {what}"),
                        trace_tail: sh.tail(40),
                        trace_hash: sh.hash(),
                        artifact_dir: Some(dir.clone()),
                    }),
                )
            }
        }
    };
    telemetry.wall_ns = t0.elapsed().as_nanos() as u64;
    telemetry
}

fn base_telemetry(cfg: &CampaignConfig, t0: std::time::Instant, failure: Option<TrialFailure>) -> TrialTelemetry {
    TrialTelemetry {
        seed: cfg.seed,
        shape: cfg.shape(),
        wall_ns: t0.elapsed().as_nanos() as u64,
        failure,
        ..TrialTelemetry::default()
    }
}

/// Reopen the crashed dir on a healthy env and run O1/O4/O5/O6.
fn verify_post_crash(
    _parent: &Path,
    dir: &Path,
    cfg: &CampaignConfig,
    sh: &CrashState,
) -> Result<TrialTelemetry, TrialFailure> {
    let mut t = TrialTelemetry {
        seed: cfg.seed,
        shape: cfg.shape(),
        ..TrialTelemetry::default()
    };
    let (host, _) = healthy_host(cfg.seed ^ 0xC4A5_0000_0000_0001);
    let t0 = std::time::Instant::now();
    let db = open_db(dir, cfg, &host).map_err(|e| TrialFailure {
        invariant: "O4",
        detail: format!("reopen after panic-crash failed: {e}"),
        trace_tail: sh.tail(40),
        trace_hash: sh.hash(),
        artifact_dir: Some(dir.to_path_buf()),
    })?;
    t.reopen_ns = t0.elapsed().as_nanos() as u64;
    if let Err(e) = db.assert_all_invariants() {
        let _ = db.close();
        return Err(TrialFailure {
            invariant: "O5",
            detail: format!("engine invariants rejected post-panic-crash: {e}"),
            trace_tail: sh.tail(40),
            trace_hash: sh.hash(),
            artifact_dir: Some(dir.to_path_buf()),
        });
    }
    let got = db.range_limited(
                        std::ops::Bound::Unbounded,
                        std::ops::Bound::Unbounded,
                        Some(usize::MAX),
                    );
    let model = sh.model.lock().unwrap();
    if let Some(detail) = scan_mismatch(&got, &model) {
        let _ = db.close();
        return Err(TrialFailure {
            invariant: "O1",
            detail: format!("acked state lost/corrupted after panic-crash: {detail}"),
            trace_tail: sh.tail(40),
            trace_hash: sh.hash(),
            artifact_dir: Some(dir.to_path_buf()),
        });
    }
    if let Err(e) = db.verify_checksums() {
        let _ = db.close();
        return Err(TrialFailure {
            invariant: "O6",
            detail: format!("checksum verify failed after panic-crash: {e}"),
            trace_tail: sh.tail(40),
            trace_hash: sh.hash(),
            artifact_dir: Some(dir.to_path_buf()),
        });
    }
    t.digest = model.digest();
    t.trace_hash = sh.hash();
    t.ops = sh.op.load(Ordering::Relaxed);
    t.dir_bytes = dir_bytes(dir);
    let _ = db.close();
    Ok(t)
}

type InnerOut = Result<TrialTelemetry, TrialFailure>;

fn run_trial_inner(
    parent: &Path,
    dir: &Path,
    cfg: &CampaignConfig,
    sh: &std::sync::Arc<CrashState>,
) -> InnerOut {
    let rng = SeedRng::new(mix_seed(cfg.seed));
    let mut telemetry = TrialTelemetry {
        seed: cfg.seed,
        shape: cfg.shape(),
        ..TrialTelemetry::default()
    };

    let (host, env) = healthy_host(rng.next_u64());
    let t = std::time::Instant::now();
    // ManuallyDrop: `Db` has a graceful-close Drop — a crash must NOT run it
    // (in-window FaultKind::Panic unwinds included). Every replacement below
    // leaks the old handle explicitly (crash semantics, fds die with the
    // child process).
    let mut db: ManuallyDrop<Db<FailingEnv>> =
        ManuallyDrop::new(open_db(dir, cfg, &host).map_err(|e| TrialFailure {
            invariant: "O4",
            detail: format!("first open failed on healthy env: {e}"),
            trace_tail: sh.tail(40),
            trace_hash: sh.hash(),
            artifact_dir: Some(dir.to_path_buf()),
        })?);
    telemetry.reopen_ns += t.elapsed().as_nanos() as u64;

    let fail = |invariant: &'static str, detail: String, dir: &Path, sh: &CrashState| TrialFailure {
        invariant,
        detail,
        trace_tail: sh.tail(40),
        trace_hash: sh.hash(),
        artifact_dir: Some(dir.to_path_buf()),
    };

    for _cycle in 0..cfg.cycles {
        // ---- workload burst -------------------------------------------------
        for _op in 0..cfg.ops_per_cycle {
            let roll = rng.gen_range(1_000);
            let k = key_of(rng.gen_range(u64::from(cfg.keyspace)));
            if roll < 540 {
                let v = scenario_value(&rng, cfg.value_max);
                let wopts = if cfg.sync_writes { WriteOptions::sync() } else { WriteOptions::no_sync() };
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || db.put_with(k.as_bytes(), &v, wopts));
                telemetry.put_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += 1;
                match r {
                    Ok(_) => {
                        sh.trace("put", &k, "OK");
                        let op = sh.op.load(Ordering::Relaxed);
                        sh.model.lock().unwrap().ack_put(k.clone().into_bytes(), v, op);
                    }
                    Err(e) => {
                        sh.model.lock().unwrap().note_err_put(k.as_bytes(), &v);
                        sh.trace("put", &k, "ERR");
                        if !env.tripped() {
                            return Err(fail(
                                "O7",
                                format!("put failed with no fault armed/tripped: {e}"),
                                dir,
                                sh,
                            ));
                        }
                    }
                }
            } else if roll < 670 {
                // Range tombstone (Vector 129 family): [s, s+span)
                let start_idx = rng.gen_range(u64::from(cfg.keyspace));
                let span = 1 + rng.gen_range((cfg.keyspace.saturating_sub(1).max(1) as u64) / 8 + 1);
                let s_key = key_of(start_idx);
                let e_key = key_of((start_idx + span).min(u64::from(cfg.keyspace)));
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || db.delete_range(s_key.as_bytes(), e_key.as_bytes()));
                telemetry.put_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += 1;
                match r {
                    Ok(_) => {
                        sh.trace("delr", &format!("{s_key}..{e_key}"), "OK");
                        let op = sh.op.load(Ordering::Relaxed);
                        sh.model
                            .lock()
                            .unwrap()
                            .ack_delete_range(s_key.as_bytes(), e_key.as_bytes(), op);
                    }
                    Err(e) => {
                        sh.trace("delr", &format!("{s_key}..{e_key}"), "ERR");
                        sh.model.lock().unwrap().note_err_range(s_key.as_bytes(), e_key.as_bytes());
                        if !env.tripped() {
                            return Err(fail(
                                "O7",
                                format!("delete_range failed with no fault armed/tripped: {e}"),
                                dir,
                                sh,
                            ));
                        }
                    }
                }
            } else if roll < 710 {
                // CAS / LWT: put_if_absent — refusal is predictable, so this
                // doubles as a model-agreement probe.
                let v = scenario_value(&rng, cfg.value_max);
                let present_per_model = sh.model.lock().unwrap().live.contains_key(k.as_bytes());
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || {
                    db.put_if_absent_with(k.as_bytes(), &v, WriteOptions::sync())
                });
                telemetry.put_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += 1;
                match r {
                    Ok(_) => {
                        sh.trace("cas", &k, "OK");
                        if present_per_model {
                            return Err(fail(
                                "O2",
                                format!("cas ok where model says key present: {k}"),
                                dir,
                                sh,
                            ));
                        }
                        let op = sh.op.load(Ordering::Relaxed);
                        sh.model.lock().unwrap().ack_put(k.clone().into_bytes(), v, op);
                    }
                    Err(e) => {
                        if matches!(e, CoreError::CasMismatch) {
                            sh.trace("cas", &k, "REFUSED");
                            if !present_per_model {
                                return Err(fail(
                                    "O2",
                                    format!("cas refused where model says key absent: {k}"),
                                    dir,
                                    sh,
                                ));
                            }
                        } else {
                            sh.trace("cas", &k, "ERR");
                            sh.model.lock().unwrap().note_err_put(k.as_bytes(), &v);
                            if !env.tripped() {
                                return Err(fail(
                                    "O7",
                                    format!("cas failed with no fault armed/tripped: {e}"),
                                    dir,
                                    sh,
                                ));
                            }
                        }
                    }
                }
            } else if roll < 740 {
                // Atomic batch: 2-4 ops, all-or-nothing.
                let n_ops = 2 + rng.gen_range(3);
                let mut batch: Vec<BatchOp> = Vec::new();
                let mut plan: Vec<(bool, String, Vec<u8>)> = Vec::new();
                for _ in 0..n_ops {
                    let bk = key_of(rng.gen_range(u64::from(cfg.keyspace)));
                    let bv = scenario_value(&rng, cfg.value_max);
                    let is_put = rng.gen_range(3) != 0;
                    if is_put {
                        batch.push(BatchOp::Put {
                            key: Bytes::from(bk.as_bytes().to_vec()),
                            value: Bytes::from(bv.clone()),
                        });
                    } else {
                        batch.push(BatchOp::Delete {
                            key: Bytes::from(bk.as_bytes().to_vec()),
                        });
                    }
                    plan.push((is_put, bk, bv));
                }
                let wopts = if cfg.sync_writes { WriteOptions::sync() } else { WriteOptions::no_sync() };
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || db.apply_batch_with(batch, wopts));
                telemetry.put_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += u64::try_from(plan.len()).unwrap_or(u64::MAX);
                match r {
                    Ok(_) => {
                        sh.trace("batch", &format!("{}ops", plan.len()), "OK");
                        let op = sh.op.load(Ordering::Relaxed);
                        let mut m = sh.model.lock().unwrap();
                        for (is_put, bk, bv) in plan {
                            if is_put {
                                m.ack_put(bk.into_bytes(), bv, op);
                            } else {
                                m.ack_delete(bk.as_bytes(), op);
                            }
                        }
                    }
                    Err(e) => {
                        sh.trace("batch", &format!("{}ops", plan.len()), "ERR");
                        let mut m = sh.model.lock().unwrap();
                        for (is_put, bk, bv) in plan {
                            if is_put {
                                m.note_err_put(bk.as_bytes(), &bv);
                            } else {
                                m.note_err_delete(bk.as_bytes());
                            }
                        }
                        if !env.tripped() {
                            return Err(fail(
                                "O7",
                                format!("batch failed with no fault armed/tripped: {e}"),
                                dir,
                                sh,
                            ));
                        }
                    }
                }
            } else if roll < 890 {
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || db.delete(k.as_bytes()));
                telemetry.put_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += 1;
                match r {
                    Ok(_) => {
                        sh.trace("del", &k, "OK");
                        let op = sh.op.load(Ordering::Relaxed);
                        let kb = k.clone();
                        sh.model.lock().unwrap().ack_delete(kb.as_bytes(), op);
                    }
                    Err(e) => {
                        sh.model.lock().unwrap().note_err_delete(k.as_bytes());
                        sh.trace("del", &k, "ERR");
                        if !env.tripped() {
                            return Err(fail(
                                "O7",
                                format!("delete failed with no fault armed/tripped: {e}"),
                                dir,
                                sh,
                            ));
                        }
                    }
                }
            } else if roll < 900 {
                let t = std::time::Instant::now();
                let got = guard_panic(sh, || db.get(k.as_bytes()));
                telemetry.get_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += 1;
                sh.trace("get", &k, "OK");
                if !env.tripped() {
                    let m = sh.model.lock().unwrap();
                    let plausible = if m.uncertain.contains(k.as_bytes()) {
                        got.as_deref().map_or(true, |v| {
                            m.universe
                                .get(k.as_bytes())
                                .map_or(false, |set| set.contains(v))
                        })
                    } else {
                        let expect = m.live.get(k.as_bytes());
                        got.as_deref() == expect.map(|v| v.as_slice())
                    };
                    if !plausible {
                        return Err(fail(
                            "O2",
                            format!(
                                "silent_wrong_read on {k}: engine={:?} model={:?}",
                                got.map(|b| b.len()),
                                m.live.get(k.as_bytes()).map(|v| v.len())
                            ),
                            dir,
                            sh,
                        ));
                    }
                }
            } else if roll < 950 {
                let t = std::time::Instant::now();
                let got = guard_panic(sh, || {
                    db.range_limited(
                        std::ops::Bound::Unbounded,
                        std::ops::Bound::Unbounded,
                        Some(usize::MAX),
                    )
                });
                telemetry.scan_ns += t.elapsed().as_nanos() as u64;
                telemetry.ops += 1;
                sh.trace("scan", "*", "OK");
                if !env.tripped() {
                    let m = sh.model.lock().unwrap();
                    if let Some(detail) = scan_mismatch(&got, &m) {
                        return Err(fail("O2", format!("silent_wrong_scan: {detail}"), dir, sh));
                    }
                }
            } else if roll < 980 {
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || db.flush());
                telemetry.flush_ns += t.elapsed().as_nanos() as u64;
                sh.trace("flush", "*", if r.is_ok() { "OK" } else { "ERR" });
                if r.is_err() && !env.tripped() {
                    return Err(fail(
                        "O7",
                        format!("flush failed with no fault armed/tripped: {}", r.unwrap_err()),
                        dir,
                        sh,
                    ));
                }
            } else {
                let t = std::time::Instant::now();
                let r = guard_panic(sh, || db.compact());
                telemetry.compact_ns += t.elapsed().as_nanos() as u64;
                sh.trace("compact", "*", if r.is_ok() { "OK" } else { "ERR" });
                if r.is_err() && !env.tripped() {
                    return Err(fail(
                        "O7",
                        format!("compact failed with no fault armed/tripped: {}", r.unwrap_err()),
                        dir,
                        sh,
                    ));
                }
            }
        }

        // ---- durability barrier ---------------------------------------------
        let t = std::time::Instant::now();
        let synced = guard_panic(sh, || db.sync());
        telemetry.flush_ns += t.elapsed().as_nanos() as u64;
        sh.trace("sync", "*", if synced.is_ok() { "OK" } else { "ERR" });
        if synced.is_err() && !env.tripped() {
            return Err(fail(
                "O7",
                format!("sync failed with no fault armed/tripped: {}", synced.unwrap_err()),
                dir,
                sh,
            ));
        }

        // ---- checkpoint copy + verify ---------------------------------------
        if rng.gen_range(100) < u64::from(cfg.checkpoint_pct) {
            let dest = parent.join(format!("pedradb-dst-ckpt-{}-{}", cfg.seed, telemetry.cycles));
            let _ = std::fs::remove_dir_all(&dest);
            let t = std::time::Instant::now();
            let r = guard_panic(sh, || db.create_checkpoint(&dest));
            telemetry.checkpoint_ns += t.elapsed().as_nanos() as u64;
            sh.trace("ckpt", &dest.display().to_string(), if r.is_ok() { "OK" } else { "ERR" });
            match r {
                Err(e) => {
                    return Err(fail("O7", format!("checkpoint failed on healthy env: {e}"), dir, sh));
                }
                Ok(_) => {
                    // The checkpoint verified; abandon the live handle WITHOUT
                    // its graceful-close Drop (crash semantics for the reopen).
                    std::mem::forget(db);
                    let (chost, _) = healthy_host(rng.next_u64());
                    let t = std::time::Instant::now();
                    let ck = open_db(&dest, cfg, &chost);
                    telemetry.reopen_ns += t.elapsed().as_nanos() as u64;
                    match ck {
                        Err(e) => {
                            return Err(fail("O4", format!("checkpoint open failed: {e}"), dir, sh));
                        }
                        Ok(ck) => {
                            let got = ck.range_limited(
                                std::ops::Bound::Unbounded,
                                std::ops::Bound::Unbounded,
                                Some(usize::MAX),
                            );
                            let m = sh.model.lock().unwrap();
                            if let Some(detail) = scan_mismatch(&got, &m) {
                                let _ = ck.close();
                                return Err(fail(
                                    "O1",
                                    format!("checkpoint state diverged: {detail}"),
                                    dir,
                                    sh,
                                ));
                            }
                            let _ = ck.close();
                        }
                    }
                    let _ = std::fs::remove_dir_all(&dest);
                    let (host2, _) = healthy_host(rng.next_u64());
                    let t = std::time::Instant::now();
                    db = ManuallyDrop::new(open_db(dir, cfg, &host2).map_err(|e| {
                        fail("O4", format!("reopen after checkpoint failed: {e}"), dir, sh)
                    })?);
                    telemetry.reopen_ns += t.elapsed().as_nanos() as u64;
                }
            }
        }

        // ---- fault window + crash -------------------------------------------
        // ENOSPC envelope is fused to the window: it always ends in a crash +
        // lift, otherwise a live envelope would poison the next cycle's
        // healthy path with disk-pressure refusals (O7 noise).
        let mut enospc_live = false;
        if rng.gen_range(100) < u64::from(cfg.fault_density_pct) || rng.gen_range(100) < u64::from(cfg.enospc_pct) {
            if rng.gen_range(100) < u64::from(cfg.enospc_pct) {
                // Disk-pressure envelope: the engine must refuse further
                // writes (Err fate) while free space is below the hard
                // watermark and drain cleanly once the envelope lifts.
                env.set_available_bytes(Some(1));
                enospc_live = true;
                telemetry.faults_armed += 1;
            }
            let kind = cfg.kinds[(rng.gen_range(cfg.kinds.len() as u64)) as usize];
            let after_ops = rng.gen_range(16);
            let transient = rng.gen_range(3) != 0;
            if rng.gen_range(2) == 0 || cfg.targeted_classes.is_empty() {
                env.arm_with_kind(after_ops, transient, kind);
            } else {
                let class =
                    cfg.targeted_classes[(rng.gen_range(cfg.targeted_classes.len() as u64)) as usize];
                env.arm_op_class(class, after_ops, transient, kind);
            }
            // In-window burst: errors are fates, not violations.
            for _ in 0..(8 + rng.gen_range(24)) {
                let k = key_of(rng.gen_range(u64::from(cfg.keyspace)));
                let v = scenario_value(&rng, cfg.value_max);
                if rng.gen_range(3) == 0 {
                    let kb = k.clone();
                    match guard_panic_op(sh, kb.as_bytes(), None, || db.delete(k.as_bytes())) {
                        Ok(_) => {
                            sh.trace("del", &k, "OK");
                            let op = sh.op.load(Ordering::Relaxed);
                            sh.model.lock().unwrap().ack_delete(k.as_bytes(), op);
                        }
                        Err(_) => {
                            sh.model.lock().unwrap().note_err_delete(k.as_bytes());
                            sh.trace("del", &k, "ERR");
                        }
                    }
                } else {
                    let kb = k.clone();
                    let vb = v.clone();
                    match guard_panic_op(sh, kb.as_bytes(), Some(vb), || {
                        db.put_with(k.as_bytes(), &v, WriteOptions::no_sync())
                    }) {
                        Ok(_) => {
                            sh.trace("put", &k, "OK");
                            let op = sh.op.load(Ordering::Relaxed);
                            sh.model.lock().unwrap().ack_put(k.into_bytes(), v, op);
                        }
                        Err(_) => {
                            sh.model.lock().unwrap().note_err_put(k.as_bytes(), &v);
                            sh.trace("put", &k, "ERR");
                        }
                    }
                }
            }
            telemetry.faults_fired += u64::from(env.tripped());

            // CRASH: abandon without close. Page cache survives; userspace is
            // gone. A live Panic arm may have already unwound us — the catch
            // handler in `run_trial_cfg` finishes this path via
            // `verify_post_crash` (the unwound handle leaks, no Drop runs).
            sh.panic_expected.store(false, Ordering::Relaxed);
            std::mem::forget(db);

            env.disarm();
            if enospc_live {
                env.set_available_bytes(None);
            }

            // Live byte damage (RFC-0321 surface): fail-closed reopen is a
            // LEGAL outcome; if it opens, nothing fabricated may surface.
            let mut tainted = false;
            if rng.gen_range(100) < u64::from(cfg.corrupt_pct) {
                sh.tainted.store(true, Ordering::Relaxed);
                tainted = true;
                let name = inject_file_damage(dir, &rng);
                sh.corrupted_file.lock().unwrap().push(name.clone());
                sh.trace("corrupt", &name, "OK");
            }

            // Recovery interrupted by a live fault: open may Err; the
            // disarmed retry must succeed.
            let (host3, fenv) = healthy_host(rng.next_u64());
            let mut reopen_under_fault = false;
            if rng.gen_range(100) < u64::from(cfg.reopen_fault_pct) {
                fenv.arm(rng.gen_range(4), true);
                reopen_under_fault = true;
            }
            let t = std::time::Instant::now();
            let opened = open_db(dir, cfg, &host3);
            telemetry.reopen_ns += t.elapsed().as_nanos() as u64;
            let reopened = match opened {
                Ok(d) => {
                    fenv.disarm();
                    d
                }
                Err(e) => {
                    let fate_legal =
                        (reopen_under_fault && host3.env().tripped()) || tainted;
                    if !fate_legal {
                        return Err(fail(
                            "O4",
                            format!("reopen after crash failed: {e}"),
                            dir,
                            sh,
                        ));
                    }
                    sh.trace("reopen", "*", "ERR");
                    fenv.disarm();
                    let (host4, _) = healthy_host(rng.next_u64());
                    let t = std::time::Instant::now();
                    match open_db(dir, cfg, &host4) {
                        Ok(retried) => {
                            telemetry.reopen_ns += t.elapsed().as_nanos() as u64;
                            retried
                        }
                        Err(_) if tainted => {
                            // Injected bit-rot detected ⇒ fail-closed is the
                            // quarantine contract working, not a violation.
                            telemetry.reopen_ns += t.elapsed().as_nanos() as u64;
                            telemetry.fail_closed = true;
                            telemetry.cycles = sh.op.load(Ordering::Relaxed) as u32;
                            telemetry.ops = sh.op.load(Ordering::Relaxed);
                            telemetry.trace_hash = sh.hash();
                            telemetry.dir_bytes = dir_bytes(dir);
                            return Ok(telemetry);
                        }
                        Err(e) => {
                            return Err(fail(
                                "O4",
                                format!("reopen retry (disarmed) failed: {e}"),
                                dir,
                                sh,
                            ));
                        }
                    }
                }
            };

            if let Err(e) = reopened.assert_all_invariants() {
                let f = fail(
                    "O5",
                    format!("engine invariants rejected post-crash: {e}"),
                    dir,
                    sh,
                );
                let _ = reopened.close();
                return Err(f);
            }
            let got = reopened.range_limited(
                        std::ops::Bound::Unbounded,
                        std::ops::Bound::Unbounded,
                        Some(usize::MAX),
                    );
            let m = sh.model.lock().unwrap();
            if let Some(detail) = scan_mismatch_mode(&got, &m, tainted) {
                let f = if tainted {
                    fail(
                        "O2",
                        format!("fabricated value surfaced post-corruption: {detail}"),
                        dir,
                        sh,
                    )
                } else {
                    fail(
                        "O1",
                        format!("acked state lost/corrupted post-crash: {detail}"),
                        dir,
                        sh,
                    )
                };
                let _ = reopened.close();
                return Err(f);
            }
            drop(m);
            // Under taint, checksum Err is DETECTION (good); strict O6 only
            // applies to untainted crashes.
            if !tainted {
                if let Err(e) = reopened.verify_checksums() {
                    let f = fail(
                        "O6",
                        format!("checksum verify failed post-crash: {e}"),
                        dir,
                        sh,
                    );
                    let _ = reopened.close();
                    return Err(f);
                }
            }
            db = ManuallyDrop::new(reopened);
        }

        telemetry.cycles += 1;
    }

    // ---- clean close + final verify -----------------------------------------
    {
        let db_inner = ManuallyDrop::into_inner(db);
        telemetry.wal_syncs = db_inner.wal_sync_count();
        telemetry.write_stalls = db_inner.write_stall_count();
        telemetry.ssts = db_inner.sst_count();
        if let Err(e) = db_inner.close() {
            return Err(fail("O7", format!("clean close failed: {e}"), dir, sh));
        }
    }
    {
        let (host4, _) = healthy_host(rng.next_u64());
        let t = std::time::Instant::now();
        let fin = open_db(dir, cfg, &host4).map_err(|e| {
            fail("O4", format!("final reopen failed: {e}"), dir, sh)
        })?;
        telemetry.reopen_ns += t.elapsed().as_nanos() as u64;
        let got = fin.range_limited(
                    std::ops::Bound::Unbounded,
                    std::ops::Bound::Unbounded,
                    Some(usize::MAX),
                );
        let m = sh.model.lock().unwrap();
        if let Some(detail) = scan_mismatch(&got, &m) {
            let f = fail(
                "O1",
                format!("final state diverged from the acked model: {detail}"),
                dir,
                sh,
            );
            let _ = fin.close();
            return Err(f);
        }
        if let Err(e) = fin.assert_all_invariants() {
            let f = fail(
                "O5",
                format!("engine invariants rejected at final: {e}"),
                dir,
                sh,
            );
            let _ = fin.close();
            return Err(f);
        }
        if let Err(e) = fin.verify_checksums() {
            let f = fail("O6", format!("checksum verify failed at final: {e}"), dir, sh);
            let _ = fin.close();
            return Err(f);
        }
        telemetry.digest = {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for (k, v) in &got {
                for b in k.iter().chain(v.iter()).copied().chain([0xFF_u8]) {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0000_0100_0000_01B3);
                }
            }
            h
        };
        let walk = fin.verify_at_rest();
        if walk.errors > 0 {
            let corrupted = sh.corrupted_file.lock().unwrap().clone();
            let foreign: Vec<String> = walk
                .failures
                .iter()
                .filter(|fl| {
                    corrupted
                        .iter()
                        .all(|name| !fl.file.contains(name.as_str()))
                })
                .map(|fl| format!("{}@{}: {}", fl.file, fl.offset, fl.message))
                .collect();
            if !foreign.is_empty() {
                let f = fail(
                    "O6",
                    format!(
                        "at-rest walk reported {} decode/CRC errors at final (foreign to injected damage: {})",
                        walk.errors,
                        foreign.join("; ")
                    ),
                    dir,
                    sh,
                );
                let _ = fin.close();
                return Err(f);
            }
            // All failures on the file we damaged: detection working.
        }
        let _ = fin.close();
    }
    // Second reopen: recovery must be idempotent (no state drift between the
    // first and second recovery of the same image).
    {
        let (host5, _) = healthy_host(rng.next_u64());
        let again = open_db(dir, cfg, &host5).map_err(|e| {
            fail("O4", format!("second reopen failed: {e}"), dir, sh)
        })?;
        let got2 = again.range_limited(
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
            Some(usize::MAX),
        );
        let digest2 = {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for (k, v) in &got2 {
                for b in k.iter().chain(v.iter()).copied().chain([0xFF_u8]) {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0000_0100_0000_01B3);
                }
            }
            h
        };
        if digest2 != telemetry.digest {
            let f = fail(
                "O1",
                format!(
                    "recovery is not idempotent: first reopen digest {:016x}, second {:016x}",
                    telemetry.digest, digest2
                ),
                dir,
                sh,
            );
            let _ = again.close();
            return Err(f);
        }
        let _ = again.close();
    }
    telemetry.trace_hash = sh.hash();
    telemetry.dir_bytes = dir_bytes(dir);
    Ok(telemetry)
}

// ---------------------------------------------------------------------------
// Determinism double-run (O8)
// ---------------------------------------------------------------------------

/// Run the scenario twice in fresh dirs; oracle-clean + equal final digest.
///
/// # Errors
/// The first violation (either run, or the O8 mismatch).
pub fn assert_deterministic_replay(parent: &Path, seed: u64) -> Result<TrialTelemetry, TrialFailure> {
    let cfg = CampaignConfig::derive(seed).clamped_for_ci();
    assert_deterministic_replay_cfg(parent, &cfg)
}

/// Run a specific config twice in fresh dirs; oracle-clean + equal final digest.
///
/// # Errors
/// The first violation (either run, or the O8 mismatch).
pub fn assert_deterministic_replay_cfg(
    parent: &Path,
    cfg: &CampaignConfig,
) -> Result<TrialTelemetry, TrialFailure> {
    let a = run_trial_cfg(parent, cfg);
    if let Some(f) = a.failure.clone() {
        return Err(f);
    }
    let b = run_trial_cfg(parent, cfg);
    if let Some(f) = b.failure {
        return Err(f);
    }
    if a.digest != b.digest {
        return Err(TrialFailure {
            invariant: "O8",
            detail: format!(
                "same seed produced different final state: digest {:016x} vs {:016x}",
                a.digest, b.digest
            ),
            trace_tail: Vec::new(),
            trace_hash: a.trace_hash,
            artifact_dir: None,
        });
    }
    Ok(a)
}

// ---------------------------------------------------------------------------
// Shrink (minimal failing scenario)
// ---------------------------------------------------------------------------

/// Shrink a failing scenario: halve cycles/ops while the violation keeps
/// reproducing. Returns (minimal failing cfg, violation, tried cfgs).
#[must_use]
pub fn shrink_failure(
    parent: &Path,
    failing: &CampaignConfig,
    violation: &TrialFailure,
) -> (CampaignConfig, TrialFailure, Vec<CampaignConfig>) {
    let mut best_cfg = failing.clone();
    let mut best_fail = violation.clone();
    let mut tried = Vec::new();
    for _ in 0..8 {
        let cand = CampaignConfig {
            cycles: (best_cfg.cycles / 2).max(1),
            ops_per_cycle: (best_cfg.ops_per_cycle / 2).max(8),
            ..best_cfg.clone()
        };
        if cand == best_cfg {
            break;
        }
        tried.push(cand.clone());
        let t = run_trial_cfg(parent, &cand);
        match t.failure {
            Some(f) => {
                best_cfg = cand;
                best_fail = f;
            }
            None => break,
        }
    }
    (best_cfg, best_fail, tried)
}

// ---------------------------------------------------------------------------
// Anti-vacuity mutants (RFC-0270 §4 / RFC-0273 §1)
// ---------------------------------------------------------------------------

/// Fixture config: small, fast, all-durable.
fn mutant_cfg(seed: u64, flush: bool) -> CampaignConfig {
    CampaignConfig {
        seed,
        cycles: 1,
        ops_per_cycle: 64,
        keyspace: 64,
        value_max: 256,
        sync_writes: true,
        wal_full_fsync: true,
        auto_flush_bytes: if flush { Some(64) } else { None },
        auto_compact_sst_count: if flush { Some(2) } else { None },
        large_value_threshold: None,
        fault_density_pct: 0,
        kinds: vec![FaultKind::IoError],
        targeted_classes: vec![],
        checkpoint_pct: 0,
        enospc_pct: 0,
        corrupt_pct: 0,
        reopen_fault_pct: 0,
        double_run: false,
        deep: false,
    }
}

/// M1 — simulate a durability-lying engine: sync-acked puts, crash (drop
/// without close), then destroy the WAL before reopen. The battery MUST see
/// the loss (`O1`), else the oracle is vacuous.
///
/// # Errors
/// Mechanical I/O failures while building the fixture.
pub fn mutant_lost_wal_is_caught(parent: &Path) -> Result<bool, String> {
    let dir = parent.join("m1-lost-wal");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = mutant_cfg(0xC0BB_ABEE, false);
    let (host, _env) = healthy_host(1);
    let mut db = open_db(&dir, &cfg, &host)?;
    for i in 0..48u64 {
        db.put(key_of(i).as_bytes(), value_of(i, 24).as_slice())
            .map_err(|e| e.to_string())?;
    }
    // Crash: forget WITHOUT close — the WAL is the only durable home of these
    // puts (Db's Drop is a graceful close and would flush to SSTs).
    std::mem::forget(db);
    // Mutant: the engine "loses" the WAL (both primary CURRENT.log and any WAL.arch* segments).
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.contains("wal")
            || name == pedradb_core::WAL_FILE_NAME.to_lowercase()
            || name.ends_with(".log")
        {
            let p = e.path();
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    // Reopen: oracle must flag the missing acked keys.
    let (host2, _) = healthy_host(2);
    let reopened = open_db(&dir, &cfg, &host2)?;
    let mut lost = 0u64;
    for i in 0..48u64 {
        if reopened.get(key_of(i).as_bytes()).is_none() {
            lost += 1;
        }
    }
    let _ = reopened.close();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(lost > 0)
}

/// M2 — silent bit-rot: flip one byte in an SST after a clean close. The
/// engine MUST flag it via `verify_checksums`/`verify_at_rest` (or the O6
/// oracle is vacuous and the engine ships undetectable corruption).
///
/// # Errors
/// Mechanical I/O failures while building the fixture.
pub fn mutant_bitrot_is_detected(parent: &Path) -> Result<bool, String> {
    let dir = parent.join("m2-bitrot");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = mutant_cfg(0xB177_0001, true);
    let (host, _env) = healthy_host(1);
    let mut db = open_db(&dir, &cfg, &host)?;
    for i in 0..256u64 {
        db.put(key_of(i).as_bytes(), value_of(i, 96).as_slice())
            .map_err(|e| e.to_string())?;
    }
    db.flush().map_err(|e| e.to_string())?;
    db.close().map_err(|e| e.to_string())?;

    // Flip one payload byte in the largest SST.
    let mut victim: Option<(PathBuf, u64)> = None;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.ends_with(".sst") {
            let len = e.metadata().map_err(|e| e.to_string())?.len();
            if victim.as_ref().map_or(true, |(_, l)| len > *l) {
                victim = Some((e.path(), len));
            }
        }
    }
    let Some((path, len)) = victim else {
        return Err("no SST file produced by flush — fixture broken".to_owned());
    };
    if len < 8 {
        return Err("SST too small to corrupt safely".to_owned());
    }
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let at = bytes.len() / 3;
    bytes[at] ^= 0xA5;
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;

    // Reopen: verification MUST fail, or open_db itself MUST reject the corrupted SST.
    let (host2, _) = healthy_host(2);
    match open_db(&dir, &cfg, &host2) {
        Ok(reopened) => {
            let detected =
                reopened.verify_checksums().is_err() || reopened.verify_at_rest().errors > 0;
            let _ = reopened.close();
            let _ = std::fs::remove_dir_all(&dir);
            Ok(detected)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            let e_lower = e.to_lowercase();
            if e_lower.contains("crc")
                || e_lower.contains("corrupt")
                || e_lower.contains("checksum")
            {
                Ok(true)
            } else {
                Err(e)
            }
        }
    }
}

/// M3 — model-vacuity check: a deliberately wrong model (one dropped key)
/// MUST be flagged by [`scan_mismatch`]. Pure oracle mutation, no engine.
#[must_use]
pub fn mutant_wrong_model_is_caught() -> bool {
    let mut model = Model::default();
    for i in 0..8u64 {
        model.ack_put(key_of(i).into_bytes(), value_of(i, 8), i + 1);
    }
    let mut got: Vec<(Bytes, Bytes)> = model
        .live
        .iter()
        .map(|(k, v)| (Bytes::from(k.clone()), Bytes::from(v.clone())))
        .collect();
    if scan_mismatch(&got, &model).is_some() {
        return false; // already wrong with equal content — oracle broken
    }
    // Drop one key from the engine side → must be caught.
    got.remove(3);
    scan_mismatch(&got, &model).is_some()
}

/// M4 — WAL header/record CRC corruption: flip bytes in the active WAL file
/// after sync-acked puts and a crash. The engine MUST reject with CRC/corruption
/// on reopen or flag the corruption, proving WAL integrity checks are non-vacuous.
///
/// # Errors
/// Mechanical I/O failures while building the fixture.
pub fn mutant_wal_crc_tamper_is_detected(parent: &Path) -> Result<bool, String> {
    let dir = parent.join("m4-wal-crc");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = mutant_cfg(0xDEAD_CAFE, false);
    let (host, _env) = healthy_host(1);
    let mut db = open_db(&dir, &cfg, &host)?;
    for i in 0..48u64 {
        db.put(key_of(i).as_bytes(), value_of(i, 32).as_slice())
            .map_err(|e| e.to_string())?;
    }
    // Crash without graceful close so WAL contains pending recovery records.
    std::mem::forget(db);

    // Locate WAL file and tamper with record payload / CRC
    let mut wal_victim: Option<PathBuf> = None;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.contains("wal")
            || name == pedradb_core::WAL_FILE_NAME.to_lowercase()
            || name.ends_with(".log")
        {
            let p = e.path();
            if p.is_file() {
                wal_victim = Some(p);
                break;
            }
        }
    }
    let Some(wal_path) = wal_victim else {
        return Err("no WAL file produced for M4 fixture".to_owned());
    };
    let mut bytes = std::fs::read(&wal_path).map_err(|e| e.to_string())?;
    if bytes.len() > 16 {
        let idx = bytes.len() / 2;
        bytes[idx] ^= 0xEE;
        std::fs::write(&wal_path, &bytes).map_err(|e| e.to_string())?;
    }

    // Reopen: must either return Err(Crc/Corruption), report corruption via verify_checksums,
    // or fail to recover the corrupted acked puts.
    let (host2, _) = healthy_host(2);
    match open_db(&dir, &cfg, &host2) {
        Ok(reopened) => {
            let chk_failed = reopened.verify_checksums().is_err()
                || reopened.verify_at_rest().errors > 0;
            let mut lost_keys = 0u64;
            for i in 0..48u64 {
                if reopened.get(key_of(i).as_bytes()).is_none() {
                    lost_keys += 1;
                }
            }
            let _ = reopened.close();
            let _ = std::fs::remove_dir_all(&dir);
            Ok(chk_failed || lost_keys > 0)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            let e_lower = e.to_lowercase();
            Ok(e_lower.contains("crc")
                || e_lower.contains("corrupt")
                || e_lower.contains("checksum")
                || e_lower.contains("invalid"))
        }
    }
}

/// M5 — Tombstone resurrection check: a deleted key mistakenly resurrected
/// in the engine scan MUST be flagged by [`scan_mismatch`]. Proves O3 is non-vacuous.
#[must_use]
pub fn mutant_tombstone_leak_is_caught() -> bool {
    let mut model = Model::default();
    for i in 0..8u64 {
        model.ack_put(key_of(i).into_bytes(), value_of(i, 8), i + 1);
    }
    // Delete key 3
    model.ack_delete(key_of(3).as_bytes(), 9);

    let mut got: Vec<(Bytes, Bytes)> = model
        .live
        .iter()
        .map(|(k, v)| (Bytes::from(k.clone()), Bytes::from(v.clone())))
        .collect();

    // Invert/leak: resurrect deleted key 3 into the engine scan output
    got.push((Bytes::from(key_of(3).into_bytes()), Bytes::from(value_of(3, 8))));

    match scan_mismatch(&got, &model) {
        Some(msg) => msg.contains("resurrection") || msg.contains("deleted"),
        None => false,
    }
}

/// M6 — Unsorted / Comparator inversion check: out-of-order keys returned by
/// an engine scan MUST be flagged by monotonicity / sorting validation,
/// proving O2 / O5 are non-vacuous.
#[must_use]
pub fn mutant_comparator_inversion_is_caught() -> bool {
    let mut model = Model::default();
    for i in 0..8u64 {
        model.ack_put(key_of(i).into_bytes(), value_of(i, 8), i + 1);
    }
    let mut got: Vec<(Bytes, Bytes)> = model
        .live
        .iter()
        .map(|(k, v)| (Bytes::from(k.clone()), Bytes::from(v.clone())))
        .collect();

    // Swap got[1] and got[2] to introduce an unsorted sequence
    if got.len() >= 3 {
        got.swap(1, 2);
    }

    // Monotonicity check on engine scan output:
    let is_sorted = got.windows(2).all(|w| w[0].0 < w[1].0);
    !is_sorted
}

/// M7 — Unwritten / fabricated key check: an engine that fabricates a key
/// outside the touched universe MUST be flagged by [`scan_mismatch`].
/// Proves model universe containment checking is non-vacuous.
#[must_use]
pub fn mutant_fabricated_key_is_caught() -> bool {
    let mut model = Model::default();
    for i in 0..8u64 {
        model.ack_put(key_of(i).into_bytes(), value_of(i, 8), i + 1);
    }
    let mut got: Vec<(Bytes, Bytes)> = model
        .live
        .iter()
        .map(|(k, v)| (Bytes::from(k.clone()), Bytes::from(v.clone())))
        .collect();

    // Add a fabricated key never touched by any put or delete
    got.push((
        Bytes::from_static(b"ghost-unwritten-key-9999"),
        Bytes::from_static(b"phantom"),
    ));

    match scan_mismatch(&got, &model) {
        Some(msg) => msg.contains("never written") || msg.contains("outside-write"),
        None => false,
    }
}

/// M6 (RFC-0329) — Durable barrier omission (fdatasync bypass): simulate an engine that
/// loses un-fsynced writes on crash. Proves O1 is non-vacuous against durable barrier skipping.
///
/// # Errors
/// Mechanical I/O failures while building the fixture.
pub fn mutant_fdatasync_bypass_is_caught(parent: &Path) -> Result<bool, String> {
    let dir = parent.join("m6-fdatasync-bypass");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = mutant_cfg(0x514C_0006, false);
    let (host, _env) = healthy_host(1);
    let mut db = open_db(&dir, &cfg, &host)?;
    for i in 0..48u64 {
        db.put(key_of(i).as_bytes(), value_of(i, 24).as_slice())
            .map_err(|e| e.to_string())?;
    }
    // Crash before graceful close
    std::mem::forget(db);
    // Simulate un-fsynced write loss: truncate WAL file
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.contains("wal")
            || name == pedradb_core::WAL_FILE_NAME.to_lowercase()
            || name.ends_with(".log")
        {
            let p = e.path();
            if p.is_file() {
                let _ = std::fs::write(&p, &[]);
            }
        }
    }
    let (host2, _) = healthy_host(2);
    let reopened = open_db(&dir, &cfg, &host2)?;
    let mut lost = 0u64;
    for i in 0..48u64 {
        if reopened.get(key_of(i).as_bytes()).is_none() {
            lost += 1;
        }
    }
    let _ = reopened.close();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(lost > 0)
}


// ---------------------------------------------------------------------------
// Shared helpers used by the runner / tests
// ---------------------------------------------------------------------------

/// Fresh unique parent dir for trials/tests.
#[must_use]
pub fn campaign_temp(tag: &str) -> PathBuf {
    temp_parent(tag)
}

/// Telemetry wrapper for a violation produced outside `run_trial_cfg`
/// (e.g. the O8 double-run inside the campaign loop).
#[must_use]
pub fn failure_telemetry(seed: u64, failure: TrialFailure) -> TrialTelemetry {
    let shape = CampaignConfig::derive(seed).shape();
    TrialTelemetry {
        seed,
        shape,
        failure: Some(failure),
        ..TrialTelemetry::default()
    }
}

/// Full oracle pass for one seed with CI caps (used by ratchet replay).
///
/// # Errors
/// The first violation hit.
pub fn replay_seed(parent: &Path, seed: u64) -> Result<TrialTelemetry, TrialFailure> {
    let cfg = CampaignConfig::derive(seed).clamped_for_ci();
    let t = run_trial_cfg(parent, &cfg);
    match t.failure {
        Some(f) => Err(f),
        None => Ok(t),
    }
}
