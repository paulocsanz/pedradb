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
//! required site and an unreachable popcount must each be caught, and
//! the honest union must pass (an always-red floor is also a bug).
//! `--measure` prints the live union for re-freezing after an
//! INTENTIONAL campaign change (same commit as the TSV move).
//!
//! Piso: the floor is over THIS pinned seed set's union — it is a
//! regression tripwire, not a claim that the campaign explores the whole
//! seam space (the adaptive soak and nightly hunts own discovery).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pedradb_world::coverage::{CoverageMask, SEAM_IDS};
use pedradb_world::{World, WorldConfig};

/// Pinned seed set (fixed order; deterministic union).
const SEEDS: &[u64] = &[
    0x00C0_FF01, 0x00C0_FF02, 0x00C0_FF03, 0x00C0_FF04, 0x00C0_FF05, 0x00C0_FF06, 0x00C0_FF07,
    0x00C0_FF08, 0x00C0_FF09, 0x00C0_FF0A, 0x00C0_FF0B, 0x00C0_FF0C, 0x00C0_FF0D, 0x00C0_FF0E,
    0x00C0_FF0F, 0x00C0_FF10, 0x1871_0001, 0x1871_0002, 0x1871_0003, 0x1871_0004, 0x1871_0005,
    0x1871_0006, 0x1871_0007, 0x1871_0008, 0x1871_0009, 0x1871_000A, 0x1871_000B, 0x1871_000C,
    0x1871_000D, 0x1871_000E, 0x1871_000F, 0x1871_0010,
];

fn measure_union() -> (CoverageMask, usize, usize) {
    let parent_root = std::env::temp_dir().join(format!("pedra-gate-cov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent_root);
    let mut union = CoverageMask::new();
    let mut ok = 0usize;
    let mut errored = 0usize;
    for (i, &seed) in SEEDS.iter().enumerate() {
        let parent = parent_root.join(format!("t{i:03}-s{seed:016x}"));
        let _ = std::fs::create_dir_all(&parent);
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.clone(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            ..Default::default()
        };
        match World::new(seed, cfg).run() {
            Ok(t) => {
                // TrialResult carries the raw mask bits (u64); fold into
                // the union via the public API.
                for (i, site) in SEAM_IDS.iter().enumerate() {
                    if t.coverage_mask & (1u64 << i) != 0 {
                        union.hit(site);
                    }
                }
                ok += 1;
            }
            Err(_) => errored += 1,
        }
        let _ = std::fs::remove_dir_all(&parent);
    }
    let _ = std::fs::remove_dir_all(&parent_root);
    (union, ok, errored)
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
    let (union, ok, errored) = measure_union();
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
        std::process::exit(selftest(&floor, &union));
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
fn selftest(floor: &Floor, union: &CoverageMask) -> i32 {
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

    println!("SELFTEST coverage: {caught}/{total} checks caught");
    if caught == total { 0 } else { 1 }
}
