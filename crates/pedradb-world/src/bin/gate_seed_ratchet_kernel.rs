//! RFC-0187 P0.2 — versioned PCT seed ratchet (blocking, self-verifying).
//!
//! Replays EVERY entry of `scripts/ratchet/pct_seeds.txt` through the real
//! entry points (`run_pcts` over the real `ConcurrentDb`, and the
//! RFC-0156 chain-3 plant) and fails if:
//!
//! - the file SHRANK (line count below the `floor N` pin — coverage
//!   regression; an intentional change moves entries AND floor together);
//! - any pinned seed stops reproducing its pinned outcome (`clean` /
//!   `violator`) — a scheduler or oracle regression;
//! - any pinned `schedule_hash` changes — the grant sequence itself
//!   changed (plant3 hashes only: pure logic, bit-stable; the engine
//!   scenario checks determinism as same-process replay h1==h2, the
//!   exact assertion of the shipped `pct_runner_drives_real_concurrentdb`
//!   test, because real-I/O grouping is timing-shaped);
//! - the engine oracle regresses (not all 36 puts visible / not durable
//!   on reopen).
//!
//! `--selftest` proves redness without touching the file: floor shrink,
//! expectation flip, and hash tamper must each be caught.
//! `--discover` prints live numbers (engine hash/steps replayed twice,
//! first d=3 violator seeds) for re-pinning after an intentional change.

use std::sync::{Arc, Mutex};

use pedradb_world::pct_concurrent::{run_pcts, PiPolicy, RunReport, Yielder};

// ---------------------------------------------------------------------------
// Scenarios (mirror the shipped tests; test-code plants, never the engine).
// ---------------------------------------------------------------------------

const ENGINE_N: usize = 3;
const ENGINE_PUTS: usize = 12;

/// Real `ConcurrentDb` under PCT (mirror of
/// `pct_runner_drives_real_concurrentdb`): returns (hash, visible_count,
/// hash_of_second_replay). Durable-reopen is checked once per gate run on
/// the first engine entry, not per seed.
fn engine_run(seed: u64, policy: PiPolicy, dir: &std::path::Path) -> (u64, usize, u64) {
    use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};
    let _ = std::fs::remove_dir_all(dir);
    let opts = OpenOptions { sync: false, ..OpenOptions::default() };
    let db = ConcurrentDb::open_with_env(dir, opts.clone(), StdEnv).unwrap();
    db.set_write_group_catchup_window(std::time::Duration::ZERO);
    let db = Arc::new(db);

    let run_once = |db: &Arc<ConcurrentDb>| -> (u64, usize) {
        let report: RunReport = run_pcts(seed, ENGINE_N, policy, |task| {
            let db = Arc::clone(db);
            move |_y: &Yielder| {
                for i in 0..ENGINE_PUTS {
                    let k = format!("pct/{task}/{i}");
                    db.put(k.as_bytes(), b"v").unwrap();
                }
            }
        });
        let total = (0..ENGINE_N)
            .map(|t| {
                (0..ENGINE_PUTS)
                    .filter(|i| db.get(format!("pct/{t}/{i}").as_bytes()).is_some())
                    .count()
            })
            .sum::<usize>();
        (report.schedule_hash, total)
    };
    let (h1, t1) = run_once(&db);
    let (h2, _) = run_once(&db);
    assert_eq!(t1, ENGINE_N * ENGINE_PUTS, "every put must be visible");

    let db = Arc::try_unwrap(db).map_err(|_| "workers still hold the db").unwrap().close().unwrap();
    let _ = db;
    (h1, t1, h2)
}

fn engine_reopen_durable(dir: &std::path::Path) -> Result<(), String> {
    use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};
    let re = ConcurrentDb::open_with_env(dir, OpenOptions::default(), StdEnv)
        .map_err(|e| format!("reopen: {e}"))?;
    for (t, i) in [(0usize, 0usize), (ENGINE_N - 1, ENGINE_PUTS - 1)] {
        if re.get(format!("pct/{t}/{i}").as_bytes()).is_none() {
            return Err(format!("pct/{t}/{i} missing after reopen — not durable"));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// RFC-0156 chain-3 plant (mirror of `Plant3`): `take100` parks between
/// check and act; violation = taken >= 3 && balance <= -200.
struct Plant3 {
    balance: Mutex<i64>,
    taken: std::sync::atomic::AtomicI64,
}

impl Plant3 {
    fn take100(&self, y: &Yielder) {
        {
            let b = self.balance.lock().unwrap();
            if *b < 100 {
                return;
            }
            drop(b);
            y.at("p3_read");
            *self.balance.lock().unwrap() -= 100;
        }
        self.taken.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn violation(&self) -> Option<String> {
        let b = *self.balance.lock().unwrap();
        let t = self.taken.load(std::sync::atomic::Ordering::SeqCst);
        (t >= 3 && b <= -200).then(|| format!("triple_take: balance={b} taken={t}"))
    }
}

fn plant3_run(seed: u64, n: usize, policy: PiPolicy) -> (Option<String>, u64) {
    let plant = Arc::new(Plant3 {
        balance: Mutex::new(100),
        taken: std::sync::atomic::AtomicI64::new(0),
    });
    let report = run_pcts(seed, n, policy, |_task| {
        let plant = Arc::clone(&plant);
        move |y: &Yielder| {
            plant.take100(y);
        }
    });
    (plant.violation(), report.schedule_hash)
}

// ---------------------------------------------------------------------------
// Ratchet file (plain text; the crate has no serde by design).
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Entry {
    seed: u64,
    scenario: String, // engine12 | plant3
    policy: String,   // seq | d2 | d3 | d4
    expect: String,   // clean | violator
    hash: Option<u64>,
}

struct Ratchet {
    floor: usize,
    entries: Vec<Entry>,
    parse_errors: Vec<String>,
}

fn parse_ratchet(text: &str) -> Ratchet {
    let mut floor = 0usize;
    let mut entries = Vec::new();
    let mut errs = Vec::new();
    for (no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("floor ") {
            match rest.trim().parse::<usize>() {
                Ok(f) => floor = f,
                Err(_) => errs.push(format!("line {}: bad floor: {line}", no + 1)),
            }
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 5 {
            errs.push(format!("line {}: need 5 tab columns: {line}", no + 1));
            continue;
        }
        let seed = match u64::from_str_radix(cols[0].trim_start_matches("0x"), 16) {
            Ok(s) => s,
            Err(_) => {
                errs.push(format!("line {}: bad seed: {}", no + 1, cols[0]));
                continue;
            }
        };
        entries.push(Entry {
            seed,
            scenario: cols[1].trim().to_string(),
            policy: cols[2].trim().to_string(),
            expect: cols[3].trim().to_string(),
            hash: if cols[4].trim() == "-" {
                None
            } else {
                match u64::from_str_radix(cols[4].trim().trim_start_matches("0x"), 16) {
                    Ok(h) => Some(h),
                    Err(_) => {
                        errs.push(format!("line {}: bad hash: {}", no + 1, cols[4]));
                        None
                    }
                }
            },
        });
    }
    Ratchet { floor, entries, parse_errors: errs }
}

fn policy_of(tag: &str) -> Result<PiPolicy, String> {
    match tag {
        "seq" => Ok(PiPolicy::Sequential),
        "rr" => Ok(PiPolicy::RoundRobin),
        "d2" => Ok(PiPolicy::Pct { depth: 2 }),
        "d3" => Ok(PiPolicy::Pct { depth: 3 }),
        "d4" => Ok(PiPolicy::Pct { depth: 4 }),
        other => Err(format!("unknown policy {other}")),
    }
}

fn repo_ratchet_path() -> std::path::PathBuf {
    // target/debug from crates/pedradb-world => repo root three up.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/ratchet/pct_seeds.txt")
}

/// Replay ONE entry. `workdir` is a scratch dir for engine scenarios.
/// Returns Err(reason) on any mismatch with the pinned expectation.
fn replay_entry(e: &Entry, workdir: &std::path::Path) -> Result<String, String> {
    let policy = policy_of(&e.policy)?;
    match e.scenario.as_str() {
        "engine12" => {
            let dir = workdir.join(format!("engine-{:x}", e.seed));
            let (h1, visible, h2) = engine_run(e.seed, policy, &dir);
            if h1 != h2 {
                return Err(format!(
                    "seed {:x}: same seed replayed different schedules ({h1:x} vs {h2:x})",
                    e.seed
                ));
            }
            if visible != ENGINE_N * ENGINE_PUTS {
                return Err(format!("seed {:x}: only {visible}/{} puts visible", e.seed, ENGINE_N * ENGINE_PUTS));
            }
            if e.expect != "clean" {
                return Err(format!("seed {:x}: replayed clean, pinned {other}", e.seed, other = e.expect));
            }
            if let Some(want) = e.hash {
                if h1 != want {
                    return Err(format!("seed {:x}: schedule hash {h1:x} != pinned {want:x}", e.seed));
                }
            }
            Ok(format!("engine12 {:x} {} clean hash={h1:x}", e.seed, e.policy))
        }
        "plant3" => {
            let (viol, hash) = plant3_run(e.seed, 3, policy);
            let got = if viol.is_some() { "violator" } else { "clean" };
            if got != e.expect {
                return Err(format!(
                    "seed {:x}: replayed {got}, pinned {} ({})",
                    e.seed,
                    e.expect,
                    viol.unwrap_or_default()
                ));
            }
            if let Some(want) = e.hash {
                if hash != want {
                    return Err(format!("seed {:x}: schedule hash {hash:x} != pinned {want:x}", e.seed));
                }
            }
            Ok(format!("plant3 {:x} {} {got} hash={hash:x}", e.seed, e.policy))
        }
        other => Err(format!("unknown scenario {other}")),
    }
}

/// The gate verdict over a parsed ratchet: pure over (ratchet, replay
/// results); the selftest tampers in memory and calls THIS.
fn check_ratchet(r: &Ratchet, results: &[Result<String, String>]) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    errs.extend(r.parse_errors.iter().cloned());
    if r.entries.len() < r.floor {
        errs.push(format!(
            "ratchet SHRANK: {} entries < floor {} (coverage regression; intentional change moves entries AND floor in one commit)",
            r.entries.len(),
            r.floor
        ));
    }
    for (i, res) in results.iter().enumerate() {
        if let Err(e) = res {
            errs.push(format!("entry {}: {e}", i + 1));
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn run_gate() -> i32 {
    let text = match std::fs::read_to_string(repo_ratchet_path()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("GATE seed-ratchet: FAIL — cannot read {}: {e}", repo_ratchet_path().display());
            return 1;
        }
    };
    let ratchet = parse_ratchet(&text);
    let workdir = std::env::temp_dir().join(format!("pedra-gate-ratchet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).unwrap();

    let mut results = Vec::new();
    let mut first_engine_done = false;
    for e in &ratchet.entries {
        let res = replay_entry(e, &workdir);
        // Durable-reopen oracle on the FIRST engine entry (same shape as
        // the shipped test: one reopen after close).
        if e.scenario == "engine12" && !first_engine_done {
            first_engine_done = true;
            if res.is_ok() {
                let dir = workdir.join(format!("engine-{:x}", e.seed));
                if let Err(err) = engine_reopen_durable(&dir) {
                    results.push(Err(format!("seed {:x}: reopen: {err}", e.seed)));
                    continue;
                }
            }
        }
        match &res {
            Ok(line) => println!("REPLAY {line}"),
            Err(e) => eprintln!("REPLAY FAIL {e}"),
        }
        results.push(res);
    }

    match check_ratchet(&ratchet, &results) {
        Ok(()) => {
            println!(
                "GATE seed-ratchet: GREEN — {}/{} seeds replayed (floor {}), all pinned outcomes/hashes reproduced",
                results.iter().filter(|r| r.is_ok()).count(),
                ratchet.entries.len(),
                ratchet.floor
            );
            0
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("GATE seed-ratchet: FAIL — {e}");
            }
            eprintln!("GATE seed-ratchet: RED");
            1
        }
    }
}

fn selftest() -> i32 {
    let mut caught = 0;
    let mut total = 0;

    let text = std::fs::read_to_string(repo_ratchet_path()).expect("ratchet file");
    let base = parse_ratchet(&text);
    assert!(base.parse_errors.is_empty(), "ratchet file must parse");

    // S1: shrink — one entry removed must fail the floor.
    total += 1;
    let mut shrunk = parse_ratchet(&text);
    shrunk.entries.pop();
    let ok_results: Vec<_> = shrunk.entries.iter().map(|_| Ok(String::new())).collect();
    if check_ratchet(&shrunk, &ok_results).is_err() {
        println!("SELFTEST seed-ratchet: caught=shrink ({} < floor {})", shrunk.entries.len(), shrunk.floor);
        caught += 1;
    } else {
        eprintln!("SELFTEST seed-ratchet: MISSED shrink");
    }

    // S2: expectation flip — a pinned CLEAN entry replayed with a live
    // mismatch must be flagged. Use the real plant3 replay with a flipped
    // pin (violator seed expecting clean is also fine; flip a clean pin).
    total += 1;
    let workdir = std::env::temp_dir().join(format!("pedra-gate-ratchet-st-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).unwrap();
    let clean_entry = base
        .entries
        .iter()
        .find(|e| e.scenario == "plant3" && e.expect == "clean")
        .expect("ratchet has a clean plant3 entry")
        .clone();
    let mut flipped = clean_entry.clone();
    flipped.expect = "violator".to_string();
    match replay_entry(&flipped, &workdir) {
        Err(e) => {
            println!("SELFTEST seed-ratchet: caught=expectation-flip — {e}");
            caught += 1;
        }
        Ok(_) => eprintln!("SELFTEST seed-ratchet: MISSED expectation flip"),
    }

    // S3: violator-blind oracle — a pinned VIOLATOR seed replayed with a
    // clean pin must be flagged (regression-of-caught-bug property).
    total += 1;
    let viol_entry = base
        .entries
        .iter()
        .find(|e| e.scenario == "plant3" && e.expect == "violator")
        .expect("ratchet has a violator plant3 entry")
        .clone();
    let mut blinded = viol_entry;
    blinded.expect = "clean".to_string();
    match replay_entry(&blinded, &workdir) {
        Err(e) => {
            println!("SELFTEST seed-ratchet: caught=violator-blind — {e}");
            caught += 1;
        }
        Ok(_) => eprintln!("SELFTEST seed-ratchet: MISSED violator-blind"),
    }

    // S4: hash tamper — a wrong pinned hash must be flagged.
    total += 1;
    let hashed = base
        .entries
        .iter()
        .find(|e| e.hash.is_some())
        .expect("ratchet has a hash-pinned entry")
        .clone();
    let mut tampered = hashed;
    tampered.hash = Some(tampered.hash.unwrap() ^ 1);
    match replay_entry(&tampered, &workdir) {
        Err(e) => {
            println!("SELFTEST seed-ratchet: caught=hash-tamper — {e}");
            caught += 1;
        }
        Ok(_) => eprintln!("SELFTEST seed-ratchet: MISSED hash tamper"),
    }

    println!("SELFTEST seed-ratchet: {caught}/{total} sabotages caught");
    if caught == total { 0 } else { 1 }
}

fn discover() {
    let workdir = std::env::temp_dir().join(format!("pedra-gate-ratchet-disc-{}", std::process::id()));
    println!("DISCOVER engine12 d2 seed 0x00515EED (replay x2):");
    let (h1, visible, h2) =
        engine_run(0x0051_5EED, PiPolicy::Pct { depth: 2 }, &workdir.join("d0"));
    println!("  hash1={h1:x} hash2={h2:x} same={} visible={visible}", h1 == h2);
    let _ = engine_reopen_durable(&workdir.join("d0"));

    println!("DISCOVER plant3 d3 violators in 0..16384 (first 8):");
    let mut found = 0;
    for s in 0u64..16384 {
        if plant3_run(s, 3, PiPolicy::Pct { depth: 3 }).0.is_some() {
            let (_, hash) = plant3_run(s, 3, PiPolicy::Pct { depth: 3 });
            println!("  seed {s}\tviolator\thash={hash:x}");
            found += 1;
            if found == 8 {
                break;
            }
        }
    }
    println!("DISCOVER plant3 reference hashes (seq/d2, seeds 0-2):");
    for s in 0u64..3 {
        let (v, h) = plant3_run(s, 3, PiPolicy::Sequential);
        println!("  seed {s}\tseq\t{}\thash={h:x}", if v.is_some() { "violator" } else { "clean" });
        let (v, h) = plant3_run(s, 3, PiPolicy::Pct { depth: 2 });
        println!("  seed {s}\td2\t{}\thash={h:x}", if v.is_some() { "violator" } else { "clean" });
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }
    if args.iter().any(|a| a == "--discover") {
        discover();
        return;
    }
    std::process::exit(run_gate());
}
