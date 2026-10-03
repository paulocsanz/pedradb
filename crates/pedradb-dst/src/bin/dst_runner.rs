//! RFC-0325: continuous DST campaign runner (`dst_runner`).
//!
//! Mirrors the caixote-dst elastic campaign shape: time-boxed, worker-parallel,
//! date-rotated seeds; violations are shrunk, deduped and filed under
//! `--findings`; passing seeds append to the grow-only ratchet that CI replays;
//! per-shape throughput feeds a rolling baseline that flags bottlenecks.
//!
//! ```text
//! cargo run -p pedradb-dst --bin dst_runner -- \
//!   --seed-base 20261002 --forever --max-runtime-secs 1200 --workers 6 \
//!   --ratchet findings/dst/campaign/ratchet/seeds.jsonl \
//!   --findings findings/dst/campaign
//! ```
//!
//! Wedge defence (uninterruptible fsync waits survive `kill -9`): progress is
//! appended per trial, and `scripts/dst_campaign.sh` runs a supervisor that
//! kills the whole process group at budget + tolerance.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pedradb_dst::campaign::{
    assert_deterministic_replay, campaign_temp, failure_telemetry, mutant_bitrot_is_detected,
    mutant_comparator_inversion_is_caught, mutant_fabricated_key_is_caught,
    mutant_fdatasync_bypass_is_caught, mutant_lost_wal_is_caught, mutant_tombstone_leak_is_caught,
    mutant_wal_crc_tamper_is_detected, mutant_wrong_model_is_caught, run_trial_cfg, shrink_failure,
    CampaignConfig,
};

const SEED_STRIDE: u64 = 1; // ratchet seeds stay contiguous: base..base+n

struct Args {
    seed_base: u64,
    forever: bool,
    seeds: u64,
    max_runtime_secs: Option<u64>,
    workers: usize,
    ratchet: PathBuf,
    findings: PathBuf,
    selftest: bool,
    replay: Option<u64>,
    open_dir: Option<PathBuf>,
    open_seed: Option<u64>,
    open_key: Option<String>,
    cycles_override: Option<u32>,
    ops_override: Option<u32>,
    mutants: Option<usize>,
    baseline: Option<PathBuf>,
    quiet: bool,
}

fn arg_of(name: &str) -> Option<String> {
    std::env::args().position(|a| a == name).and_then(|i| std::env::args().nth(i + 1))
}

fn flag_of(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn parse_args() -> Args {
    let date_base: u64 = std::env::var("PEDRA_DST_SEED_BASE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() / 86_400)
                .unwrap_or(0)
        });
    Args {
        seed_base: arg_of("--seed-base").and_then(|s| s.parse().ok()).unwrap_or(date_base),
        forever: flag_of("--forever"),
        seeds: arg_of("--seeds").and_then(|s| s.parse().ok()).unwrap_or(64),
        max_runtime_secs: arg_of("--max-runtime-secs").and_then(|s| s.parse().ok()),
        workers: arg_of("--workers").and_then(|s| s.parse().ok()).unwrap_or(4).max(1),
        ratchet: arg_of("--ratchet").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from("findings/dst/campaign/ratchet/seeds.jsonl")
        }),
        findings: arg_of("--findings").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from("findings/dst/campaign")
        }),
        selftest: flag_of("--selftest"),
        replay: arg_of("--replay").and_then(|s| s.parse().ok()),
        open_dir: arg_of("--open-dir").map(PathBuf::from),
        open_seed: arg_of("--open-seed").and_then(|s| s.parse().ok()),
        open_key: arg_of("--open-key"),
        mutants: arg_of("--mutants").and_then(|s| s.parse().ok()),
        cycles_override: arg_of("--cycles").and_then(|s| s.parse().ok()),
        ops_override: arg_of("--ops").and_then(|s| s.parse().ok()),
        baseline: arg_of("--baseline").map(PathBuf::from),
        quiet: flag_of("--quiet"),
    }
}

fn log(q: bool, line: &str) {
    if !q {
        println!("{line}");
    }
}

// ---------------------------------------------------------------------------
// Baseline (gargalos): shape → ema ops/sec, sample count
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Baseline {
    map: HashMap<String, (f64, u64)>,
}

impl Baseline {
    fn load(path: Option<&Path>) -> Baseline {
        let mut map = HashMap::new();
        if let Some(p) = path {
            if let Ok(body) = std::fs::read_to_string(p) {
                for line in body.lines() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() == 3 {
                        if let (Ok(v), Ok(n)) = (parts[1].parse::<f64>(), parts[2].parse::<u64>()) {
                            map.insert(parts[0].to_owned(), (v, n));
                        }
                    }
                }
            }
        }
        Baseline { map }
    }

    fn note(&mut self, shape: &str, ops_per_sec: f64) {
        let e = self.map.entry(shape.to_owned()).or_insert((ops_per_sec, 0));
        e.1 += 1;
        e.0 = if e.1 == 1 {
            ops_per_sec
        } else {
            0.7 * e.0 + 0.3 * ops_per_sec
        };
    }

    fn slowdown_vs(&self, shape: &str, ops_per_sec: f64) -> Option<(f64, u64)> {
        let (ema, samples) = self.map.get(shape)?;
        if *samples < 3 || *ema <= 0.0 {
            return None;
        }
        if ops_per_sec < 0.7 * ema {
            Some((*ema, *samples))
        } else {
            None
        }
    }

    fn save(&self, path: Option<&Path>) {
        if let Some(p) = path {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut lines: Vec<String> = self
                .map
                .iter()
                .map(|(k, (v, n))| format!("{k}\t{v:.1}\t{n}"))
                .collect();
            lines.sort();
            let _ = std::fs::write(p, lines.join("\n") + "\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Findings + ratchet
// ---------------------------------------------------------------------------

struct Ledger {
    findings: PathBuf,
    signatures: Mutex<HashMap<String, (String, u64)>>, // sig → (file, hits)
    ratchet: PathBuf,
    ratchet_seen: Mutex<HashSet<u64>>,
    ratchet_io: Mutex<()>,
}

impl Ledger {
    fn load(findings: &Path, ratchet: &Path) -> Ledger {
        let mut seen = HashSet::new();
        if let Ok(body) = std::fs::read_to_string(ratchet) {
            for line in body.lines() {
                if let Some(seed) = parse_seed_line(line) {
                    seen.insert(seed);
                }
            }
        }
        let signatures = Mutex::new(load_signature_index(findings));
        Ledger {
            findings: findings.to_path_buf(),
            signatures,
            ratchet: ratchet.to_path_buf(),
            ratchet_seen: Mutex::new(seen),
            ratchet_io: Mutex::new(()),
        }
    }

    fn ratchet_pass(&self, seed: u64, shape: &str) {
        let _g = self.ratchet_io.lock().unwrap();
        if self.ratchet_seen.lock().unwrap().insert(seed) {
            if let Some(parent) = self.ratchet.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.ratchet)
            {
                let _ = writeln!(f, "{{\"seed\":{seed},\"shape\":\"{shape}\"}}");
            }
        }
    }

    fn file_violation(
        &self,
        seed: u64,
        cfg: &CampaignConfig,
        invariant: &str,
        detail: &str,
        trace_hash: u64,
        trace_tail: &[pedradb_dst::campaign::TraceLine],
        artifact_dir: Option<&Path>,
        shrink_note: Option<(u32, u32, usize)>,
    ) -> PathBuf {
        let _g = self.ratchet_io.lock().unwrap();
        let _ = std::fs::create_dir_all(&self.findings);
        let sig = format!("{invariant}/{trace_hash:016x}");
        let file = self.findings.join(format!(
            "FAIL-{}-s{seed}-{}.md",
            date_tag(),
            invariant.to_lowercase()
        ));
        {
            let mut idx = self.signatures.lock().unwrap();
            if let Some((existing, hits)) = idx.get_mut(&sig) {
                *hits += 1;
                let path = PathBuf::from(existing.as_str());
                let _ = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .map(|mut f| writeln!(f, "\n<!-- repeat hit #{hits}: seed {seed} -->"));
                return path;
            }
        }
        let mut body = String::new();
        body.push_str(&format!("# FAIL {invariant} — seed {seed}\n\n"));
        body.push_str(&format!("- **Data:** {}\n", iso_date()));
        body.push_str(&format!("- **Shape:** {}\n", cfg.shape()));
        body.push_str(&format!("- **trace_hash:** `{trace_hash:016x}`\n"));
        body.push_str(&format!(
            "- **Repro:** `cargo run -p pedradb-dst --bin dst_runner -- --replay {seed}`\n"
        ));
        if let Some(d) = artifact_dir {
            body.push_str(&format!("- **Evidência (dir mantido):** `{}`\n", d.display()));
        }
        body.push_str(&format!(
            "\n## Config\n\n```rust\n{cfg:#?}\n```\n\n## Violação\n\n{detail}\n"
        ));
        if let Some((cycles, ops, tried)) = shrink_note {
            body.push_str(&format!(
                "\n## Shrink\n\nReproduz com `cycles={cycles}` `ops_per_cycle={ops}` (shrink tried {tried} steps; menor cenário que ainda falha).\n"
            ));
        }
        if !trace_tail.is_empty() {
            body.push_str("\n## Trace tail\n\n```text\n");
            for l in trace_tail {
                body.push_str(&format!("{:>5} {:<8} {:<16} {}\n", l.op, l.tag, l.key, l.fate));
            }
            body.push_str("```\n");
        }
        let _ = std::fs::write(&file, body);
        self.signatures
            .lock()
            .unwrap()
            .insert(sig, (file.display().to_string(), 1));
        file
    }

    fn file_bottleneck(&self, shape: &str, ops_per_sec: f64, ema: f64, samples: u64) -> PathBuf {
        let _g = self.ratchet_io.lock().unwrap();
        let _ = std::fs::create_dir_all(&self.findings);
        let file = self.findings.join(format!("BOTTLENECK-{}-{}.md", date_tag(), shape));
        let body = format!(
            "# BOTTLENECK {shape}\n\n- **Data:** {iso}\n- Medição atual: **{now:.1} ops/s**\n- Baseline (EMA de {samples} amostras): **{ema:.1} ops/s**\n- Queda > 30% contra o baseline de forma idêntica → investigar regressão de gargalo antes de Shell.\n",
            iso = iso_date(),
            now = ops_per_sec,
        );
        let _ = std::fs::write(&file, body);
        file
    }
}

fn load_signature_index(findings: &Path) -> HashMap<String, (String, u64)> {
    // The index lives inside the findings dir; rebuilt from FAIL headers when absent.
    let mut map = HashMap::new();
    let idx = findings.join("index.tsv");
    if let Ok(body) = std::fs::read_to_string(&idx) {
        for line in body.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() == 3 {
                if let Ok(hits) = parts[2].parse::<u64>() {
                    map.insert(parts[0].to_owned(), (parts[1].to_owned(), hits));
                }
            }
        }
        return map;
    }
    if let Ok(rd) = std::fs::read_dir(findings) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("FAIL-") || !name.ends_with(".md") {
                continue;
            }
            if let Ok(body) = std::fs::read_to_string(e.path()) {
                let sig_line = body.lines().find(|l| l.starts_with("- **Assinatura:**"));
                let inv_line = body.lines().find(|l| l.starts_with("# FAIL "));
                if let (Some(sig), Some(inv)) = (sig_line, inv_line) {
                    let sig = sig.trim_start_matches("- **Assinatura:** `").trim_end_matches('`');
                    let inv = inv.trim_start_matches("# FAIL ").split(' ').next().unwrap_or("?");
                    map.insert(sig.to_owned(), (e.path().display().to_string(), 1));
                    let _ = inv;
                }
            }
        }
    }
    map
}

fn parse_seed_line(line: &str) -> Option<u64> {
    let at = line.find("\"seed\":")? + 7;
    let rest = &line[at..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn date_tag() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    format!("d{days}")
}

fn iso_date() -> String {
    // Seconds-since-epoch ISO-ish tag (no chrono dep; campaign is date-rotated).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch-{secs}")
}

// ---------------------------------------------------------------------------
// Campaign aggregation
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Aggregate {
    shapes: Mutex<HashMap<String, ShapeAgg>>,
    ok: AtomicU64,
    bad: AtomicU64,
    violations: Mutex<Vec<(u64, String, String)>>, // seed, invariant, file
    bottlenecks: Mutex<Vec<String>>,
}

#[derive(Default)]
struct ShapeAgg {
    trials: u64,
    ops: f64,
    wall_ns: f64,
    ops_per_sec: Vec<f64>,
    faults_fired: u64,
    panics: u64,
}

impl Aggregate {
    fn note(&self, t: &pedradb_dst::campaign::TrialTelemetry) {
        let mut shapes = self.shapes.lock().unwrap();
        let e = shapes.entry(t.shape.clone()).or_default();
        e.trials += 1;
        e.ops += t.ops as f64;
        e.wall_ns += t.wall_ns as f64;
        e.ops_per_sec.push(t.ops_per_sec());
        e.faults_fired += t.faults_fired;
        e.panics += t.expected_panics;
        if t.failure.is_some() {
            self.bad.fetch_add(1, Ordering::Relaxed);
        } else {
            self.ok.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn report(&self) -> String {
        let mut out = String::from("## Por forma (shape)\n\n| shape | trials | ops | ops/s mediana | ops/s média | falhas injetadas | panics esperados |\n|---|---|---|---|---|---|---|\n");
        let shapes = self.shapes.lock().unwrap();
        let mut keys: Vec<&String> = shapes.keys().collect();
        keys.sort();
        for k in keys {
            let e = &shapes[k];
            let mut rates = e.ops_per_sec.clone();
            rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = rates.get(rates.len() / 2).copied().unwrap_or(0.0);
            let mean = if e.wall_ns > 0.0 { e.ops * 1e9 / e.wall_ns } else { 0.0 };
            out.push_str(&format!(
                "| {k} | {} | {:.0} | {median:.0} | {mean:.0} | {} | {} |\n",
                e.trials, e.ops, e.faults_fired, e.panics
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

fn main() -> std::process::ExitCode {
    silence_expected_panics();
    let args = parse_args();

    if args.selftest {
        return selftest(&args);
    }
    if let Some(seed) = args.replay {
        return replay(&args, seed);
    }
    if let Some(dir) = args.open_dir.clone() {
        return open_dir_probe(&args, &dir);
    }
    if let Some(seeds_per) = args.mutants {
        return mutants_mode(&args, seeds_per.max(1));
    }

    campaign(&args)
}

/// Suppress default panic output for FaultKind::Panic injections (they are
/// crash simulations handled by the campaign, not test failures).
fn silence_expected_panics() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if !msg.contains("injected panic fault") {
            eprintln!("panic: {msg}");
        }
    }));
}

fn selftest(args: &Args) -> std::process::ExitCode {
    let parent = campaign_temp("selftest");
    let mut failed = false;
    log(args.quiet, "== RFC-0325 selftest (anti-vacuity, RFC-0270 §4) ==");

    match mutant_lost_wal_is_caught(&parent) {
        Ok(true) => log(args.quiet, "  M1 lost-WAL caught .......... PASS"),
        Ok(false) => {
            log(args.quiet, "  M1 lost-WAL caught .......... VACUOUS (oracle did NOT see the loss)");
            failed = true;
        }
        Err(e) => {
            log(args.quiet, &format!("  M1 lost-WAL caught .......... ERROR: {e}"));
            failed = true;
        }
    }
    match mutant_bitrot_is_detected(&parent) {
        Ok(true) => log(args.quiet, "  M2 bit-rot detected ......... PASS"),
        Ok(false) => {
            log(args.quiet, "  M2 bit-rot detected ......... UNDETECTED (checksum oracle did NOT see corruption)");
            failed = true;
        }
        Err(e) => {
            log(args.quiet, &format!("  M2 bit-rot detected ......... ERROR: {e}"));
            failed = true;
        }
    }
    if mutant_wrong_model_is_caught() {
        log(args.quiet, "  M3 wrong-model caught ....... PASS");
    } else {
        log(args.quiet, "  M3 wrong-model caught ....... VACUOUS");
        failed = true;
    }
    match mutant_wal_crc_tamper_is_detected(&parent) {
        Ok(true) => log(args.quiet, "  M4 WAL-CRC tamper caught .... PASS"),
        Ok(false) => {
            log(args.quiet, "  M4 WAL-CRC tamper caught .... VACUOUS (CRC tampering was NOT detected)");
            failed = true;
        }
        Err(e) => {
            log(args.quiet, &format!("  M4 WAL-CRC tamper caught .... ERROR: {e}"));
            failed = true;
        }
    }
    if mutant_tombstone_leak_is_caught() {
        log(args.quiet, "  M5 tombstone leak caught .... PASS");
    } else {
        log(args.quiet, "  M5 tombstone leak caught .... VACUOUS");
        failed = true;
    }
    match mutant_fdatasync_bypass_is_caught(&parent) {
        Ok(true) => log(args.quiet, "  M6 fdatasync bypass caught .. PASS"),
        Ok(false) => {
            log(args.quiet, "  M6 fdatasync bypass caught .. VACUOUS (fdatasync bypass was NOT detected)");
            failed = true;
        }
        Err(e) => {
            log(args.quiet, &format!("  M6 fdatasync bypass caught .. ERROR: {e}"));
            failed = true;
        }
    }
    if mutant_comparator_inversion_is_caught() {
        log(args.quiet, "  M6 comparator inversion ..... PASS");
    } else {
        log(args.quiet, "  M6 comparator inversion ..... VACUOUS");
        failed = true;
    }
    if mutant_fabricated_key_is_caught() {
        log(args.quiet, "  M7 fabricated key caught .... PASS");
    } else {
        log(args.quiet, "  M7 fabricated key caught .... VACUOUS");
        failed = true;
    }
    match assert_deterministic_replay(&parent, 7) {
        Ok(t) => log(
            args.quiet,
            &format!("  O8 determinism seed 7 ....... PASS (digest {:016x}, {} ops/s)", t.digest, t.ops_per_sec() as u64),
        ),
        Err(f) => {
            log(args.quiet, &format!("  O8 determinism seed 7 ....... FAIL [{}]: {}", f.invariant, f.detail));
            failed = true;
        }
    }
    let _ = std::fs::remove_dir_all(&parent);
    if failed {
        log(args.quiet, "selftest: RED");
        std::process::ExitCode::from(2)
    } else {
        log(args.quiet, "selftest: GREEN");
        std::process::ExitCode::SUCCESS
    }
}

/// Triage probe: reopen an evidence dir with `--open-seed`'s open options and
/// dump observable state (key lookup, live count, engine invariants, checksums).
/// Mutation-scored DST (RFC-0270 §4): run a seed band with each engine
/// mutant active (zero-recompile `mutation_switch_kernel`); the oracle
/// battery MUST kill it. Survivors are filed — either an oracle gap or an
/// inert-for-this-surface mutant; both are triage input.
fn mutants_mode(args: &Args, seeds_per: usize) -> std::process::ExitCode {
    use pedradb_core::mutation_switch_kernel::{all_mutants, mutant_name, MutantGuard};

    let parent = campaign_temp("mutants");
    let seeds: Vec<u64> = (1..=seeds_per as u64).collect();
    let mut killed = Vec::new();
    let mut survivors: Vec<(u32, &'static str)> = Vec::new();

    for &id in all_mutants() {
        let _guard = MutantGuard::activate(id);
        let mut hit = false;
        for &seed in &seeds {
            let cfg = CampaignConfig::derive(seed).clamped_for_ci();
            let t = run_trial_cfg(&parent, &cfg);
            if t.failure.is_some() {
                log(
                    args.quiet,
                    &format!(
                        "  mutant {} ({id}) KILLED by seed {seed}: [{}]",
                        mutant_name(id),
                        t.failure.as_ref().map_or("?", |f| f.invariant),
                    ),
                );
                hit = true;
                break;
            }
        }
        // Second pass for stubborn survivors: compaction-forcing, long,
        // compressible-heavy shapes (drop_manifest_edit / fabricate_orphan_key
        // / decompression_bomb_bypass live on those surfaces).
        if !hit {
            for &seed in &[11u64, 12, 13, 14] {
                let mut cfg = CampaignConfig::derive(seed).clamped_for_ci();
                cfg.auto_compact_sst_count = Some(1);
                cfg.auto_flush_bytes = Some(8 * 1024);
                cfg.cycles = cfg.cycles.min(4).max(3);
                cfg.ops_per_cycle = 512;
                let t = run_trial_cfg(&parent, &cfg);
                if t.failure.is_some() {
                    log(
                        args.quiet,
                        &format!(
                            "  mutant {} ({id}) KILLED by targeted seed {seed}: [{}]",
                            mutant_name(id),
                            t.failure.as_ref().map_or("?", |f| f.invariant),
                        ),
                    );
                    hit = true;
                    break;
                }
            }
        }
        if hit {
            killed.push(id);
        } else {
            survivors.push((id, mutant_name(id)));
            log(args.quiet, &format!("  mutant {} ({}) SURVIVED the band", id, mutant_name(id)));
        }
    }
    let _ = std::fs::remove_dir_all(&parent);

    // Mutants whose effect is only observable under power-loss semantics
    // (page-cache crash model cannot see a skipped fdatasync): documented
    // inert here — the real-disk lane (scripts/swarm_physical_disk.sh) owns
    // them.
    const POWER_LOSS_LANE: &[u32] = &[1001];
    let real_gaps: Vec<_> = survivors
        .iter()
        .copied()
        .filter(|(id, _)| !POWER_LOSS_LANE.contains(id))
        .collect();
    let _ = survivors_len_setter(&mut survivors);
    if real_gaps.is_empty() {
        log(args.quiet, &format!("mutation-scored DST: {} killed · survivors all documented-inert (power-loss lane)", killed.len()));
        std::process::ExitCode::SUCCESS
    } else {
        let names: Vec<String> = real_gaps
            .iter()
            .map(|(id, name)| format!("{id}:{name}"))
            .collect();
        let body = format!(
            "# MUTANT SURVIVORS — DST oracle gaps\n\n- **Data:** {}\n- Band: {} seeds clamped por mutante\n- Sobreviventes: {}\n\nTriar: gap de oráculo vs mutante inerte nesta superfície (ex.: bypass_wal_sync só visível em power-loss ⇒ lane real-disk).\n",
            iso_date(),
            seeds_per,
            names.join(", ")
        );
        let _ = std::fs::create_dir_all(&args.findings);
        let path = args.findings.join(format!("MUTANT-SURVIVORS-{}.md", date_tag()));
        let _ = std::fs::write(&path, body);
        log(args.quiet, &format!("mutation-scored DST: {} SURVIVORS → {}", survivors.len(), path.display()));
        std::process::ExitCode::from(2)
    }
}

#[allow(dead_code)]
fn survivors_len_setter(_: &mut Vec<(u32, &'static str)>) {}

fn open_dir_probe(args: &Args, dir: &Path) -> std::process::ExitCode {
    use pedradb_core::{db::Db, DetHost};
    use pedradb_sim::FailingEnv;

    let seed = args.open_seed.unwrap_or(0);
    let cfg = CampaignConfig::derive(seed);
    let host = DetHost::with_seed(FailingEnv::passing(), seed);
    match Db::open_with_host(dir, cfg.open_options(), &host) {
        Err(e) => {
            log(args.quiet, &format!("open: ERR {e}"));
            std::process::ExitCode::FAILURE
        }
        Ok(db) => {
            if let Some(k) = &args.open_key {
                log(
                    args.quiet,
                    &format!("get {k}: {:?} bytes", db.get(k.as_bytes()).map(|b| b.len())),
                );
            }
            let all = db.range_limited(
                std::ops::Bound::Unbounded,
                std::ops::Bound::Unbounded,
                Some(usize::MAX),
            );
            log(args.quiet, &format!("live keys: {}", all.len()));
            log(
                args.quiet,
                &format!("assert_all_invariants: {:?}", db.assert_all_invariants().map(|_| "OK")),
            );
            log(args.quiet, &format!("verify_checksums: {:?}", db.verify_checksums().map(|_| "OK")));
            log(args.quiet, &format!("last_sequence: {}", u64::from(db.last_sequence())));
            let _ = db.close();
            std::process::ExitCode::SUCCESS
        }
    }
}

fn replay(args: &Args, seed: u64) -> std::process::ExitCode {
    let parent = campaign_temp("replay");
    let mut cfg = CampaignConfig::derive(seed);
    if let Some(c) = args.cycles_override {
        cfg.cycles = c;
    }
    if let Some(o) = args.ops_override {
        cfg.ops_per_cycle = o;
    }
    log(args.quiet, &format!("replay seed {seed}: {}", cfg.shape()));
    log(args.quiet, &format!("config: {cfg:?}"));
    let t = run_trial_cfg(&parent, &cfg);
    log(args.quiet, &t.to_json());
    if let Some(f) = &t.failure {
        log(args.quiet, &format!("VIOLATION [{}]: {}", f.invariant, f.detail));
        if !f.trace_tail.is_empty() {
            log(args.quiet, "trace tail:");
            for l in &f.trace_tail {
                log(args.quiet, &format!("{:>5} {:<8} {:<16} {}", l.op, l.tag, l.key, l.fate));
            }
        }
        if let Some(d) = &f.artifact_dir {
            log(args.quiet, &format!("evidence kept at {}", d.display()));
        }
        return std::process::ExitCode::FAILURE;
    }
    log(args.quiet, "PASS");
    let _ = std::fs::remove_dir_all(&parent);
    std::process::ExitCode::SUCCESS
}

fn campaign(args: &Args) -> std::process::ExitCode {
    let started = Instant::now();
    let deadline = args
        .max_runtime_secs
        .map(|s| started + Duration::from_secs(s));
    let seed_base = args.seed_base;

    let baseline = Mutex::new(Baseline::load(args.baseline.as_deref()));
    let ledger = Ledger::load(&args.findings, &args.ratchet);
    let agg = Aggregate::default();
    let next_i = AtomicU64::new(0);
    let progress = args.findings.join(format!("progress-{}.jsonl", date_tag()));
    let _ = std::fs::create_dir_all(&args.findings);

    log(
        args.quiet,
        &format!(
            "[dst-campaign] seed base {seed_base} · {} · workers {} · findings {}",
            if args.forever {
                format!("forever (budget {:?})", args.max_runtime_secs)
            } else {
                format!("{} seeds", args.seeds)
            },
            args.workers,
            args.findings.display(),
        ),
    );

    let total_planned = if args.forever { u64::MAX } else { args.seeds };
    // Circuit breaker: if the environment dies (temp volume unmounted, dir
    // removed), every trial fails instantly and the loop would burn millions
    // of fake violations. 20 consecutive sub-100ms O4 failures ⇒ abort.
    let fast_fails = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    std::thread::scope(|scope| {
        for _w in 0..args.workers {
            scope.spawn(|| loop {
                if let Some(dl) = deadline {
                    if Instant::now() >= dl {
                        break;
                    }
                }
                let i = next_i.fetch_add(1, Ordering::Relaxed);
                if i >= total_planned {
                    break;
                }
                let seed = seed_base.wrapping_add(i.wrapping_mul(SEED_STRIDE));
                let cfg = CampaignConfig::derive(seed);
                // Pre-flight: a nearly-full volume turns disk-pressure
                // refusals into O7 noise (and cascades via kept evidence
                // dirs). Wait for headroom instead of filing garbage.
                for _ in 0..12 {
                    let free = pedradb_core::probe_available_bytes(
                        &pedradb_core::StdEnv,
                        std::env::temp_dir().as_path(),
                    )
                    .unwrap_or(u64::MAX);
                    if free > 512 * 1024 * 1024 {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(5));
                }
                let parent = campaign_temp("camp");
                // O8 in the wild: every 8th seed runs the scenario twice and
                // must land on the same final logical state.
                let t = if cfg.double_run {
                    match assert_deterministic_replay(&parent, seed) {
                        Ok(t) => t,
                        Err(f) => failure_telemetry(seed, f),
                    }
                } else {
                    run_trial_cfg(&parent, &cfg)
                };
                use std::sync::atomic::Ordering as Ao;
                if t.failure.as_ref().map_or(false, |f| f.invariant == "O4")
                    && t.wall_ns < 100_000_000
                {
                    let n = fast_fails.fetch_add(1, Ao::Relaxed) + 1;
                    if n >= 20 {
                        eprintln!(
                            "[dst-campaign] ENVIRONMENT LOST: {n} consecutive instant open failures — aborting (volume unmounted?)"
                        );
                        std::process::exit(3);
                    }
                } else {
                    fast_fails.store(0, Ao::Relaxed);
                }
                let label = match &t.failure {
                    None => "OK".to_owned(),
                    Some(f) => format!("VIOLATION[{}]", f.invariant),
                };
                log(
                    args.quiet,
                    &format!(
                        "seed={seed} {} cycles={} ops={} faults={}/{} {} {:.2} ops/s{}",
                        t.shape,
                        t.cycles,
                        t.ops,
                        t.faults_fired,
                        t.faults_armed,
                        label,
                        t.ops_per_sec(),
                        t.failure
                            .as_ref()
                            .map(|f| format!(" :: {}", f.detail))
                            .unwrap_or_default(),
                    ),
                );
                agg.note(&t);
                {
                    if let Ok(mut pf) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&progress)
                    {
                        let _ = writeln!(pf, "{}", t.to_json());
                    }
                }
                if let Some(f) = &t.failure {
                    // Shrink once per signature to keep campaign time bounded.
                    let (min_cfg, min_fail, tried) = shrink_failure(&parent, &cfg, f);
                    let note = Some((min_cfg.cycles, min_cfg.ops_per_cycle, tried.len()));
                    let file = ledger.file_violation(
                        seed,
                        &cfg,
                        f.invariant,
                        &f.detail,
                        f.trace_hash,
                        &min_fail.trace_tail,
                        f.artifact_dir.as_deref(),
                        note,
                    );
                    agg.violations
                        .lock()
                        .unwrap()
                        .push((seed, f.invariant.to_owned(), file.display().to_string()));
                } else {
                    ledger.ratchet_pass(seed, &t.shape);
                    baseline.lock().unwrap().note(&t.shape, t.ops_per_sec());
                }
                let _ = std::fs::remove_dir_all(&parent);
            });
        }
    });

    let baseline = baseline.into_inner().unwrap();
    baseline.save(args.baseline.as_deref());

    // Bottleneck flags against the pre-campaign baseline.
    {
        let shapes = agg.shapes.lock().unwrap();
        for (shape, e) in shapes.iter() {
            let mut rates = e.ops_per_sec.clone();
            rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = rates.get(rates.len() / 2).copied().unwrap_or(0.0);
            if let Some((ema, samples)) = baseline.slowdown_vs(shape, median) {
                let file = ledger.file_bottleneck(shape, median, ema, samples);
                agg.bottlenecks
                    .lock()
                    .unwrap()
                    .push(file.display().to_string());
            }
        }
    }

    // Report.
    let report = args.findings.join(format!("REPORT-{}.md", date_tag()));
    let mut body = String::new();
    body.push_str(&format!("# DST campaign — {}\n\n", date_tag()));
    body.push_str(&format!(
        "- seeds OK: {} · seeds com violação: {} · elapsed: {:.1}s\n",
        agg.ok.load(Ordering::Relaxed),
        agg.bad.load(Ordering::Relaxed),
        started.elapsed().as_secs_f32()
    ));
    body.push_str(&format!(
        "- ratchet: {} seeds pinned ({})\n",
        ledger.ratchet_seen.lock().unwrap().len(),
        args.ratchet.display()
    ));
    body.push('\n');
    body.push_str(&agg.report());
    let vios = agg.violations.lock().unwrap();
    if !vios.is_empty() {
        body.push_str("\n## Violações\n\n");
        for (seed, inv, file) in vios.iter() {
            body.push_str(&format!("- seed {seed} [{inv}] → {file}\n"));
        }
    }
    let btns = agg.bottlenecks.lock().unwrap();
    if !btns.is_empty() {
        body.push_str("\n## Gargalos\n\n");
        for b in btns.iter() {
            body.push_str(&format!("- {b}\n"));
        }
    }
    let _ = std::fs::write(&report, body);
    log(args.quiet, &format!("[dst-campaign] report: {}", report.display()));

    if agg.bad.load(Ordering::Relaxed) > 0 {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
