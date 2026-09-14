//! RFC-0187 P0.1 — exhaustive N≤3 concurrency gate (blocking, self-verifying).
//!
//! Runs `run_exhaustive` (RFC-0157 P1.3, production runner) over two
//! scenario families, N=3, and asserts the COMPLETE grant space of each
//! is enumerated with the frozen shape:
//!
//! - **buggy** (planted depth-2 bug, as-is): the 66-schedule space where
//!   the bug lives. Asserts nodes/leaves/violators EXACTLY — an
//!   enumerator regression (truncation, schedule loss) or a re-blinded
//!   oracle (violators < frozen) goes red.
//! - **clean** (the re-check fix, same yield structure): every explored
//!   schedule keeps `balance >= 0`. Asserts violators == 0 — any
//!   violation on ANY explored schedule goes red.
//!
//! This is the only ∀ the CI can state: every schedule in the enumerated
//! grant space (not ∀ OS interleavings — R-pct / R-glue stand, RFC-0070).
//!
//! `--selftest` proves the gate can go red: four sabotages, exit 0 only
//! if EVERY one is caught (violation detected, truncated enumeration
//! detected, coverage shrink detected, liar oracle detected). No
//! production code is edited.
//!
//! `--measure` prints the live shape of every scenario (no assertions)
//! for re-freezing after an INTENTIONAL runner change; the frozen
//! constants below then move in the same commit as that change.

use std::sync::{Arc, Mutex};

use pedradb_world::pct_concurrent::{
    run_exhaustive, ExhaustiveReport, ExhaustiveSetup, Yielder,
};

const N: usize = 3;

/// Frozen space shape: measured 2026-09-10 on `run_exhaustive` from this
/// gate. `violators` = expected violating leaves (the planted bug is
/// FOUND on the buggy shape; a re-blinded oracle changes the count and
/// goes red).
struct Frozen {
    nodes: usize,
    leaves: usize,
    violators: usize,
}

const FROZEN_BUGGY_D2: Frozen = Frozen { nodes: 181, leaves: 66, violators: 60 };
const FROZEN_BUGGY_CHAIN3: Frozen = Frozen { nodes: 181, leaves: 66, violators: 60 };
const FROZEN_CLEAN_D2: Frozen = Frozen { nodes: 181, leaves: 66, violators: 0 };
const FROZEN_CLEAN_CHAIN3: Frozen = Frozen { nodes: 181, leaves: 66, violators: 0 };

/// Clean variant of the RFC-0157 `Plant`: identical check/yield
/// structure, but the act RE-CHECKS the balance — the fix. Invariant
/// `balance >= 0` must hold on every explored schedule.
struct CleanPlant {
    balance: Mutex<i64>,
}

impl CleanPlant {
    fn withdraw_all(&self, y: &Yielder) {
        {
            let b = self.balance.lock().unwrap();
            if *b < 100 {
                return; // nothing to take
            }
            drop(b);
            y.at("check_act"); // same preemption window as the planted bug
            let mut b = self.balance.lock().unwrap();
            if *b >= 100 {
                *b -= 100; // RE-CHECK: the fix
            }
        }
    }

    fn violation(&self) -> Option<String> {
        let b = *self.balance.lock().unwrap();
        (b < 0).then(|| format!("double_spend: balance={b}"))
    }
}

/// Planted (as-is) variant — the RFC-0157 bug class: act without
/// re-check after the yield.
struct BuggyPlant {
    balance: Mutex<i64>,
}

impl BuggyPlant {
    fn withdraw_all(&self, y: &Yielder) {
        {
            let b = self.balance.lock().unwrap();
            if *b < 100 {
                return;
            }
            drop(b);
            y.at("check_act");
            let mut b = self.balance.lock().unwrap();
            *b -= 100; // act without re-check (the planted bug)
        }
    }

    fn violation(&self) -> Option<String> {
        let b = *self.balance.lock().unwrap();
        (b < 0).then(|| format!("double_spend: balance={b}"))
    }
}

trait Plant: Send + Sync {
    fn withdraw_all(&self, y: &Yielder);
    fn violation(&self) -> Option<String>;
}

impl Plant for CleanPlant {
    fn withdraw_all(&self, y: &Yielder) {
        CleanPlant::withdraw_all(self, y)
    }
    fn violation(&self) -> Option<String> {
        CleanPlant::violation(self)
    }
}

impl Plant for BuggyPlant {
    fn withdraw_all(&self, y: &Yielder) {
        BuggyPlant::withdraw_all(self, y)
    }
    fn violation(&self) -> Option<String> {
        BuggyPlant::violation(self)
    }
}

fn setup<P: Plant + Default + 'static>(ops: usize) -> impl Fn() -> ExhaustiveSetup {
    move || {
        let plant = Arc::new(P::default());
        let mut tasks = Vec::with_capacity(N);
        for _ in 0..N {
            let plant = Arc::clone(&plant);
            tasks.push(
                Box::new(move |y: &Yielder| {
                    for _ in 0..ops {
                        plant.withdraw_all(y);
                    }
                }) as Box<dyn FnOnce(&Yielder) + Send + 'static>,
            );
        }
        let probe_plant = Arc::clone(&plant);
        ExhaustiveSetup {
            tasks,
            probe: Box::new(move || probe_plant.violation()),
        }
    }
}

impl Default for CleanPlant {
    fn default() -> Self {
        CleanPlant { balance: Mutex::new(100) }
    }
}

impl Default for BuggyPlant {
    fn default() -> Self {
        BuggyPlant { balance: Mutex::new(100) }
    }
}

/// The gate verdict for one exhaustive run against its frozen shape.
/// Pure: the live gate, `--measure` re-freeze checks, and every
/// selftest sabotage call THIS function.
fn check_run(tag: &str, report: &ExhaustiveReport, frozen: &Frozen) -> Result<(), String> {
    if report.diverged {
        return Err(format!("{tag}: prefix replay diverged (determinism break)"));
    }
    if report.hit_cap {
        return Err(format!(
            "{tag}: enumeration truncated by cap — NOT exhaustive ({} leaves)",
            report.leaves
        ));
    }
    if report.leaves != frozen.leaves {
        return Err(format!(
            "{tag}: explored {} schedules, frozen space is {} — enumeration coverage changed (shrink=red; intentional runner change => re-freeze in the same commit)",
            report.leaves, frozen.leaves
        ));
    }
    if report.distinct_leaves != report.leaves {
        return Err(format!(
            "{tag}: duplicate leaf schedules ({} distinct / {} leaves)",
            report.distinct_leaves, report.leaves
        ));
    }
    if report.nodes != frozen.nodes {
        return Err(format!(
            "{tag}: {} tree nodes, frozen is {} — space shape changed",
            report.nodes, frozen.nodes
        ));
    }
    if report.violators.len() != frozen.violators {
        return Err(format!(
            "{tag}: {} violating schedules (frozen {}); first: {:?}",
            report.violators.len(),
            frozen.violators,
            report.violators.first()
        ));
    }
    Ok(())
}

fn scenarios() -> Vec<(&'static str, usize, Frozen, bool)> {
    // (tag, ops, frozen, buggy)
    vec![
        ("buggy/d2", 3, FROZEN_BUGGY_D2, true),
        ("buggy/chain3", 4, FROZEN_BUGGY_CHAIN3, true),
        ("clean/d2", 3, FROZEN_CLEAN_D2, false),
        ("clean/chain3", 4, FROZEN_CLEAN_CHAIN3, false),
    ]
}

fn run_scenario(ops: usize, buggy: bool) -> ExhaustiveReport {
    if buggy {
        run_exhaustive(N, None, setup::<BuggyPlant>(ops))
    } else {
        run_exhaustive(N, None, setup::<CleanPlant>(ops))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }

    if args.iter().any(|a| a == "--measure") {
        for (tag, ops, _, buggy) in scenarios() {
            let r = run_scenario(ops, buggy);
            println!(
                "MEASURE {tag}: nodes={} leaves={} distinct={} violators={}",
                r.nodes, r.leaves, r.distinct_leaves, r.violators.len()
            );
        }
        return;
    }

    let mut failed = false;
    for (tag, ops, frozen, buggy) in scenarios() {
        let report = run_scenario(ops, buggy);
        println!(
            "SCENARIO {tag}: schedules={}/{} nodes={}/{} violators={}/{}",
            report.leaves,
            frozen.leaves,
            report.nodes,
            frozen.nodes,
            report.violators.len(),
            frozen.violators
        );
        if let Err(e) = check_run(tag, &report, &frozen) {
            eprintln!("GATE exhaustive: FAIL — {e}");
            failed = true;
        }
    }
    if failed {
        eprintln!("GATE exhaustive: RED");
        std::process::exit(1);
    }
    println!("GATE exhaustive: GREEN — 66/66 schedules enumerated per scenario (buggy shape: bug found on 60; clean shape: 0 violations), counts asserted");
}

/// Prove the gate can fail. Exit 0 iff every sabotage IS caught.
fn selftest() -> i32 {
    let mut caught = 0;
    let mut total = 0;
    macro_rules! sabotage {
        ($name:expr, $res:expr) => {
            total += 1;
            match $res {
                Err(e) => {
                    println!("SELFTEST exhaustive: caught={} — {}", $name, e);
                    caught += 1;
                }
                Ok(()) => {
                    eprintln!("SELFTEST exhaustive: MISSED {} — gate is blind", $name);
                }
            }
        };
    }

    // S1: violation on an explored schedule — the planted bug fed to the
    // CLEAN expectation must be flagged.
    let buggy = run_exhaustive(N, None, setup::<BuggyPlant>(3));
    sabotage!(
        "violation-on-explored-schedule",
        check_run("s1", &buggy, &FROZEN_CLEAN_D2)
    );

    // S2: truncated enumeration (cap 10 << full space) — the count
    // assertion must fire, never a green capped run.
    let truncated = run_exhaustive(N, Some(10), setup::<CleanPlant>(3));
    sabotage!("truncated-enumeration", check_run("s2", &truncated, &FROZEN_CLEAN_D2));

    // S3: coverage shrink — the full clean space checked against a
    // floor one leaf ABOVE the real space must be flagged.
    let clean = run_exhaustive(N, None, setup::<CleanPlant>(3));
    let shrunk = Frozen {
        nodes: FROZEN_CLEAN_D2.nodes,
        leaves: FROZEN_CLEAN_D2.leaves + 1,
        violators: 0,
    };
    sabotage!("coverage-shrink", check_run("s3", &clean, &shrunk));

    // S4: liar oracle (probe always fires) must fail the clean gate.
    let liar = run_exhaustive(N, None, || {
        let plant = Arc::new(CleanPlant::default());
        let mut tasks = Vec::with_capacity(N);
        for _ in 0..N {
            let plant = Arc::clone(&plant);
            tasks.push(
                Box::new(move |y: &Yielder| {
                    for _ in 0..3 {
                        plant.withdraw_all(y);
                    }
                }) as Box<dyn FnOnce(&Yielder) + Send + 'static>,
            );
        }
        ExhaustiveSetup {
            tasks,
            probe: Box::new(|| Some("liar oracle".to_string())),
        }
    });
    sabotage!("liar-oracle", check_run("s4", &liar, &FROZEN_CLEAN_D2));

    println!("SELFTEST exhaustive: {caught}/{total} sabotages caught");
    if caught == total {
        0
    } else {
        1
    }
}
