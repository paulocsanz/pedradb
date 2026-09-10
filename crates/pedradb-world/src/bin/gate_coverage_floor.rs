//! RFC-0187 P1.1 — interleaving coverage floor (blocking, self-verifying).
//!
//! Runs the REAL `World` (buggify site×kind arms, the soak
//! configuration) over a pinned seed list, unions every trial's
//! `CoverageMask`, and requires the union to meet the versioned floor
//! `scripts/ratchet/coverage_floor.tsv`:
//!
//! - every `site 1` line is REQUIRED (a seam site the campaign used to
//!   exercise but no longer reaches = coverage regression = red);
//! - `floor_pop N` pins the minimum union popcount.
//!
//! `--selftest` proves redness without editing any file: a phantom
//! required site and an unreachable popcount must each be caught, the
//! honest union must pass (an always-red floor is also a bug), and
//! every pinned seed is load-bearing (S4: drop any one → red).
//! `--measure` prints the live union for re-freezing after an
//! INTENTIONAL campaign change (same commit as the TSV move).
//!
//! Piso: the floor is over THIS pinned seed set's union — it is a
//! regression tripwire, not a claim that the campaign explores the whole
//! seam space (the adaptive soak and nightly hunts own discovery).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pedradb_world::buggify::buggify_schedule_from_seed;
use pedradb_world::coverage::{CoverageMask, SEAM_IDS};
use pedradb_world::{World, WorldConfig};

/// Pinned seed set (fixed order; deterministic union).
/// RFC-0188 P2.2: irredundant 3-seed cover of all 15 inventory sites
/// (measured 2026-09-10). Each seed is load-bearing — removing any one
/// drops a required site (`--selftest` S4). The four soak-owned sites
/// land on: `0x1` → `W.crash` (+ `E.create_open`/`E.meta`); `0x15` →
/// `E.remove`; `0xc8` → `D.bitrot` (and `E.meta` with `0x1`).
const SEEDS: &[u64] = &[
    0x0000_0001,
    0x0000_0015,
    0x0000_00C8,
];

/// One pinned-seed World trial (soak config). `Some(mask)` on Ok, `None`
/// on a fail-stop trial (its coverage does not count).
fn run_trial(parent_root: &Path, seed: u64) -> Option<CoverageMask> {
    let parent = parent_root.join(format!("t-s{seed:016x}"));
    let _ = std::fs::create_dir_all(&parent);
    let cfg = WorldConfig {
        n_nodes: 3,
        n_ranges: 1,
        schedule_steps: 12,
        parent: parent.clone(),
        exchange_rounds: 48,
        buggify: true,
        buggify_widen_sites: true,
        net_reorder_window: 2,
        ..Default::default()
    };
    let mask = World::new(seed, cfg).run().ok().map(|t| {
        let mut m = CoverageMask::new();
        for (i, site) in SEAM_IDS.iter().enumerate() {
            if t.coverage_mask & (1u64 << i) != 0 {
                m.hit(site);
            }
        }
        m
    });
    let _ = std::fs::remove_dir_all(&parent);
    mask
}

fn measure_union(seeds: &[u64]) -> (CoverageMask, Vec<CoverageMask>, usize, usize) {
    let parent_root = std::env::temp_dir().join(format!("pedra-gate-cov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent_root);
    let mut union = CoverageMask::new();
    let mut per_seed = Vec::with_capacity(seeds.len());
    let mut ok = 0usize;
    let mut errored = 0usize;
    for &seed in seeds {
        match run_trial(&parent_root, seed) {
            Some(m) => {
                union.merge(&m);
                per_seed.push(m);
                ok += 1;
            }
            None => {
                per_seed.push(CoverageMask::new());
                errored += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&parent_root);
    (union, per_seed, ok, errored)
}

struct Floor {
    floor_pop: usize,
    required: Vec<String>,
}

fn parse_floor(text: &str) -> Result<Floor, String> {
    let mut floor_pop = None;
    let mut required = Vec::new();
    for (no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("floor_pop ") {
            floor_pop = Some(
                rest.trim()
                    .parse::<usize>()
                    .map_err(|e| format!("line {}: bad floor_pop: {e}", no + 1))?,
            );
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 2 || cols[1].trim() != "1" {
            return Err(format!("line {}: need `site<TAB>1`: {line}", no + 1));
        }
        let site = cols[0].trim();
        if !SEAM_IDS.contains(&site) {
            return Err(format!("line {}: unknown seam site {site}", no + 1));
        }
        required.push(site.to_string());
    }
    Ok(Floor { floor_pop: floor_pop.ok_or("missing `floor_pop N` line")?, required })
}

/// Pure verdict: live union vs floor. The selftest tampers the FLOOR and
/// calls THIS.
fn check_floor(floor: &Floor, union: &CoverageMask) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    let pop = union.popcount() as usize;
    if pop < floor.floor_pop {
        errs.push(format!(
            "union popcount {pop} < floor {} — interleaving coverage regressed",
            floor.floor_pop
        ));
    }
    for site in &floor.required {
        if !union.has(site) {
            errs.push(format!("required seam site {site} no longer exercised by the pinned campaign"));
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn repo_floor_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/ratchet/coverage_floor.tsv")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // RFC-0188 P2.2 — seed hunt for floor growth: per-seed site report
    // over `start..start+n` (same soak config as the pinned campaign).
    // Prints one line per trial; Ok trials carry their covered sites.
    // Plan-only hunt: print sites the seed-derived buggify schedule
    // *arms* (no World run). Use this to find candidates for the 4
    // soak sites, then confirm with `--sweep` (real World coverage).
    if let Some(pos) = args.iter().position(|a| a == "--sweep-plan") {
        let spec = args.get(pos + 1).cloned().unwrap_or_default();
        let parts: Vec<&str> = spec.split(':').collect();
        if let (Ok(start), Ok(n)) = (
            parts.first().copied().unwrap_or("").parse::<u64>(),
            parts.get(1).copied().unwrap_or("").parse::<u64>(),
        ) {
            for seed in start..start + n {
                let plan = buggify_schedule_from_seed(seed, 3, 12, true);
                let mut sites: Vec<&str> = plan
                    .arms
                    .iter()
                    .map(|a| a.site.as_str())
                    .collect();
                sites.sort();
                sites.dedup();
                println!("PLAN seed={seed:016x} sites={}", sites.join(","));
            }
            return;
        }
        eprintln!("GATE coverage: FAIL — --sweep-plan needs START:N");
        std::process::exit(1);
    }

    if let Some(pos) = args.iter().position(|a| a == "--sweep") {
        let spec = args.get(pos + 1).cloned().unwrap_or_default();
        let parts: Vec<&str> = spec.split(':').collect();
        if let (Ok(start), Ok(n)) = (
            parts.first().copied().unwrap_or("").parse::<u64>(),
            parts.get(1).copied().unwrap_or("").parse::<u64>(),
        ) {
                let parent_root =
                    std::env::temp_dir().join(format!("pedra-gate-sweep-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&parent_root);
                for seed in start..start + n {
                    match run_trial(&parent_root, seed) {
                        Some(m) => println!(
                            "SWEEP seed={seed:016x} ok=1 sites={}",
                            m.hit_ids().join(",")
                        ),
                        None => println!("SWEEP seed={seed:016x} ok=0 sites=-"),
                    }
                }
                let _ = std::fs::remove_dir_all(&parent_root);
                return;
        }
        eprintln!("GATE coverage: FAIL — --sweep needs START:N (e.g. --sweep 1:200)");
        std::process::exit(1);
    }

    let (union, per_seed, ok, errored) = measure_union(SEEDS);
    let sites: Vec<&str> = union.hit_ids();

    if args.iter().any(|a| a == "--measure") {
        println!(
            "MEASURE coverage: trials={} ok={ok} errored={errored} union_pop={} sites={}",
            SEEDS.len(),
            sites.len(),
            sites.join(",")
        );
        return;
    }

    let text = match std::fs::read_to_string(repo_floor_path()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("GATE coverage: FAIL — cannot read {}: {e}", repo_floor_path().display());
            std::process::exit(1);
        }
    };
    let floor = match parse_floor(&text) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("GATE coverage: FAIL — {e}");
            std::process::exit(1);
        }
    };

    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest(&floor, &union, &per_seed));
    }

    println!(
        "COVERAGE trials={} ok={ok} errored={errored} union_pop={}/{} sites={}",
        SEEDS.len(),
        sites.len(),
        floor.floor_pop,
        sites.join(",")
    );
    match check_floor(&floor, &union) {
        Ok(()) => println!(
            "GATE coverage: GREEN — union popcount {} >= floor {}, all {} required sites exercised",
            sites.len(),
            floor.floor_pop,
            floor.required.len()
        ),
        Err(errs) => {
            for e in errs {
                eprintln!("GATE coverage: FAIL — {e}");
            }
            eprintln!("GATE coverage: RED");
            std::process::exit(1);
        }
    }
}

/// Prove the floor can fail (and pass) without touching any file.
/// S4 (RFC-0188 P2.2): every pinned seed is load-bearing — removing any
/// one must drop a required site (or the popcount) and go red.
fn selftest(floor: &Floor, union: &CoverageMask, per_seed: &[CoverageMask]) -> i32 {
    let mut caught = 0;
    let mut total = 0;

    // S1: phantom required site must be caught. Pick dynamically: any
    // seam site NOT in the live union (a covered site would never flag).
    total += 1;
    match SEAM_IDS.iter().find(|s| !union.has(s)) {
        Some(uncovered) => {
            let mut phantom = Floor { floor_pop: floor.floor_pop, required: floor.required.clone() };
            phantom.required.push((*uncovered).to_string());
            match check_floor(&phantom, union) {
                Err(errs) if errs.iter().any(|e| e.contains("no longer exercised")) => {
                    println!("SELFTEST coverage: caught=phantom-required-site ({uncovered})");
                    caught += 1;
                }
                other => {
                    eprintln!("SELFTEST coverage: phantom site {uncovered} not flagged as missing: {other:?}")
                }
            }
        }
        None => {
            // Full-coverage future: popcount sabotage (S2) still proves
            // the floor can fail; no uncovered site exists to phantom.
            println!("SELFTEST coverage: skipped=phantom-required-site (union already covers every seam site)");
            caught += 1;
        }
    }

    // S2: unreachable popcount floor must be caught.
    total += 1;
    let sky = Floor {
        floor_pop: union.popcount() as usize + 1,
        required: floor.required.clone(),
    };
    match check_floor(&sky, union) {
        Err(errs) if errs.iter().any(|e| e.contains("popcount")) => {
            println!("SELFTEST coverage: caught=unreachable-popcount");
            caught += 1;
        }
        other => eprintln!("SELFTEST coverage: popcount floor not flagged: {other:?}"),
    }

    // S3: the honest floor must PASS (oracle not always-red).
    total += 1;
    let seen: HashSet<&str> = union.hit_ids().into_iter().collect();
    let honest = Floor {
        floor_pop: union.popcount() as usize,
        required: seen.iter().map(|s| s.to_string()).collect(),
    };
    if check_floor(&honest, union).is_ok() {
        println!("SELFTEST coverage: passed=honest-floor (oracle not always-red)");
        caught += 1;
    } else {
        eprintln!("SELFTEST coverage: honest floor REJECTED — oracle always-red");
    }

    // S4: every pinned seed is load-bearing — the union minus any ONE
    // seed must fail the floor (a redundant seed would mean the pinned
    // campaign is looser than the floor claims).
    total += 1;
    let mut load_bearing = 0usize;
    for i in 0..per_seed.len() {
        let mut without = CoverageMask::new();
        for (j, m) in per_seed.iter().enumerate() {
            if j != i {
                without.merge(m);
            }
        }
        if check_floor(floor, &without).is_err() {
            load_bearing += 1;
        }
    }
    if per_seed.is_empty() {
        eprintln!("SELFTEST coverage: MISSED seed-removal (no pinned seeds)");
    } else if load_bearing == per_seed.len() {
        println!(
            "SELFTEST coverage: caught=seed-removal-red ({load_bearing}/{} seeds load-bearing)",
            per_seed.len()
        );
        caught += 1;
    } else {
        eprintln!(
            "SELFTEST coverage: MISSED seed-removal ({load_bearing}/{} seeds load-bearing — pin an irredundant set)",
            per_seed.len()
        );
    }

    println!("SELFTEST coverage: {caught}/{total} checks caught");
    if caught == total { 0 } else { 1 }
}
