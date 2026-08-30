//! RFC-0051 P0 — PCT over real concurrent code (feature `pct`).
//!
//! Drives real OS threads through the cooperative turnstile
//! (`pedradb_core::pct_hooks`): exactly one worker runs between yield
//! points, and a [`PiPolicy`] decides who runs next. `Sequential` and
//! [`PiPolicy::RoundRobin`] model the coarse π (op-atomic, no intra-op
//! preemption); [`PiPolicy::Pct`] is depth-d PCT (Burckhardt et al. Fig. 7,
//! [`crate::pct`]) which preempts **inside** ops at the engine's yield
//! sites. Planted bugs live in test code only — the dente is finding the
//! plant, the engine is what runs.

use std::sync::Arc;

use pedradb_core::pct_hooks::Turnstile;

use crate::pct::{PctScheduler, Scheduler};

/// Scheduling policy for a trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiPolicy {
    /// Run-to-completion: the granted task keeps the CPU (each of its
    /// yields is immediately re-granted) until it finishes; the next task
    /// is the lowest-index ready one. A late-entering lower index never
    /// preempts a task mid-op — without the stickiness, thread spawn
    /// races would let the "sequential" band express intra-op preemption.
    Sequential,
    /// Fixed rotation over ready tasks at yield granularity (coarse π:
    /// with op-atomic plants this never preempts inside an op).
    RoundRobin,
    /// Depth-d PCT from `seed`.
    Pct {
        /// Preemption-bound depth (2 = at most 2 nested preemption points).
        depth: usize,
    },
}

impl PiPolicy {
    /// RFC-0070 P2.2: campaign default PCT depth (2). d>2 remains RFC-0051.
    #[must_use]
    pub fn pct_campaign_default() -> Self {
        PiPolicy::Pct {
            depth: pedradb_core::group_commit_kernel::pct_campaign_default_depth() as usize,
        }
    }
}

struct SequentialSched {
    // Sticky CPU holder: stays Some(task) until the task is no longer
    // enabled (i.e. it exited — with the CPU free, every live task is
    // parked and thus enabled).
    current: Option<usize>,
}

impl Scheduler for SequentialSched {
    fn next(&mut self, enabled: &[usize]) -> Option<usize> {
        if let Some(c) = self.current {
            if enabled.contains(&c) {
                return Some(c);
            }
            self.current = None;
        }
        let first = enabled.first().copied();
        self.current = first;
        first
    }
}

struct RoundRobinSched {
    next_up: usize,
    n: usize,
}

impl Scheduler for RoundRobinSched {
    fn next(&mut self, enabled: &[usize]) -> Option<usize> {
        if enabled.is_empty() {
            return None;
        }
        for _ in 0..self.n {
            let t = self.next_up % self.n;
            self.next_up += 1;
            if enabled.contains(&t) {
                return Some(t);
            }
        }
        enabled.first().copied()
    }
}

/// One scheduler grant (worker + the yield site it resumed from).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStep {
    /// Worker granted the CPU.
    pub worker: usize,
    /// Yield site the grant resumed (`"enter"` for the first grant).
    pub site: &'static str,
}

/// Result of one trial.
#[derive(Clone, Debug)]
pub struct RunReport {
    /// Grant sequence, in order.
    pub steps: Vec<RunStep>,
    /// Bit-stable hash of the grant sequence.
    pub schedule_hash: u64,
    /// Per-group returned-seq ranges of every atomic group commit in this
    /// run (group-aware oracle forensics; per-run, so parallel trials in
    /// one process never interleave).
    pub group_ranges: Vec<(u64, u64)>,
    /// RFC-0070 P1.1: this PCT run is not ∀ OS schedules.
    pub forall_schedules: bool,
}

impl RunReport {
    /// RFC-0070 P1.1: admit ∀π after this PCT run. Always false.
    #[must_use]
    pub fn claim_forall_schedules(&self) -> bool {
        self.forall_schedules
    }
}

/// PCT depth encoded in `policy` (0 for sequential / round-robin).
#[must_use]
pub fn policy_pct_depth(policy: PiPolicy) -> u64 {
    match policy {
        PiPolicy::Pct { depth } => depth as u64,
        PiPolicy::Sequential | PiPolicy::RoundRobin => 0,
    }
}

/// Bit-stable hash over grant sequence (worker + site).
#[must_use]
pub fn run_steps_hash(steps: &[RunStep]) -> u64 {
    let mut h = 0x0051_DC70_0000_0001u64;
    for s in steps {
        h ^= s.worker as u64;
        h = h.wrapping_mul(0x1000_0000_01B3);
        for &b in s.site.as_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x1000_0000_01B3);
        }
        h ^= h >> 33;
    }
    h ^ (steps.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Worker-side yielder handed to each task closure.
pub struct Yielder {
    ts: Arc<Turnstile>,
    task: usize,
}

impl Yielder {
    /// Explicit yield at `site` (test/model code).
    pub fn at(&self, site: &'static str) {
        self.ts.yield_point(self.task, site);
    }
}

fn policy_scheduler(seed: u64, n: usize, k: usize, policy: PiPolicy) -> Box<dyn Scheduler> {
    match policy {
        PiPolicy::Sequential => Box::new(SequentialSched { current: None }),
        PiPolicy::RoundRobin => Box::new(RoundRobinSched { next_up: 0, n }),
        PiPolicy::Pct { depth } => Box::new(PctScheduler::from_seed(seed, n, depth, k)),
    }
}

/// Run `mk(task)` on `n` real threads under `policy`; returns the grant
/// sequence and its hash. The runner registers each thread as a PCT worker
/// so engine `maybe_yield` hooks (feature `pct`) preempt inside real ops.
pub fn run_pcts<F>(seed: u64, n: usize, policy: PiPolicy, mk: impl Fn(usize) -> F) -> RunReport
where
    F: FnOnce(&Yielder) + Send + 'static,
{
    let k = 16 * n; // generous step bound for change-point placement
    let mut sched = policy_scheduler(seed, n, k, policy);
    let tasks: Vec<Box<dyn FnOnce(&Yielder) + Send + 'static>> = (0..n)
        .map(|task| {
            let f = mk(task);
            Box::new(f) as Box<dyn FnOnce(&Yielder) + Send + 'static>
        })
        .collect();
    let mut rep = run_with_sched(n, tasks, sched.as_mut());
    rep.forall_schedules =
        pedradb_core::group_commit_kernel::forall_schedules_admitted(policy_pct_depth(policy));
    rep
}

/// `run_pcts` with an arbitrary scheduler and pre-built task closures
/// (shared by the policy runner and the exhaustive enumerator).
fn run_with_sched(
    n: usize,
    tasks: Vec<Box<dyn FnOnce(&Yielder) + Send + 'static>>,
    sched: &mut dyn Scheduler,
) -> RunReport {
    let ts = Arc::new(Turnstile::new(n));
    let mut joins = Vec::with_capacity(n);
    for (task, f) in tasks.into_iter().enumerate() {
        let ts = Arc::clone(&ts);
        joins.push(std::thread::spawn(move || {
            pedradb_core::pct_hooks::install_worker(Arc::clone(&ts), task);
            struct ExitGuard<'a>(&'a Turnstile, usize);
            impl Drop for ExitGuard<'_> {
                fn drop(&mut self) {
                    pedradb_core::pct_hooks::clear_worker();
                    self.0.worker_exit(self.1);
                }
            }
            let _guard = ExitGuard(&ts, task);
            ts.worker_enter(task);
            let y = Yielder {
                ts: Arc::clone(&ts),
                task,
            };
            f(&y);
        }));
    }

    // Scheduler: one grant at a time, driven by the policy. The entry
    // barrier pins the enabled-set sequence to the grant history, so the
    // schedule is a pure function of (seed, policy, n).
    ts.wait_all_entered();
    let mut steps = Vec::new();
    loop {
        ts.wait_ready();
        let enabled = ts.ready_tasks();
        if enabled.is_empty() {
            break;
        }
        let Some(t) = sched.next(&enabled) else {
            break;
        };
        let site = ts.grant(t);
        steps.push(RunStep { worker: t, site });
        if ts.finished() == n && ts.ready_tasks().is_empty() {
            break;
        }
    }
    for j in joins {
        j.join().expect("pct worker panicked");
    }
    let schedule_hash = run_steps_hash(&steps);
    let group_ranges = ts.take_group_ranges();
    RunReport {
        forall_schedules: false,
        steps,
        schedule_hash,
        group_ranges,
    }
}

// -------------------------------------------------------------------------
// RFC-0157 P1.3 — exhaustive small-space enumeration (no sampling).
// -------------------------------------------------------------------------

/// Replays a recorded grant prefix; on exhaustion records the enabled set
/// (the node's branches) and completes on a sticky-first tail. Reports
/// `diverged` if the run ever deviates from the prefix (a determinism
/// break — the entry barrier pins enabled-sets to grant history).
struct PrefixSched {
    prefix: Vec<usize>,
    pos: usize,
    branch: Option<Vec<usize>>,
    recorded: bool,
    diverged: bool,
}

impl Scheduler for PrefixSched {
    fn next(&mut self, enabled: &[usize]) -> Option<usize> {
        if enabled.is_empty() {
            return None;
        }
        if self.pos < self.prefix.len() {
            let t = self.prefix[self.pos];
            self.pos += 1;
            if enabled.contains(&t) {
                Some(t)
            } else {
                self.diverged = true;
                enabled.first().copied()
            }
        } else {
            if !self.recorded {
                self.recorded = true;
                self.branch = Some(enabled.to_vec());
            }
            // Deterministic completion: lowest-index enabled task hogs the
            // CPU until it exits; deeper choices are enumerated via the
            // branch children, not this tail.
            enabled.first().copied()
        }
    }
}

/// Fresh per-run scenario state for [`run_exhaustive`]: task closures
/// sharing one new scenario instance, plus a violation oracle probed
/// after the run joins.
pub struct ExhaustiveSetup {
    /// One closure per task (len must equal the `n` passed to the runner).
    pub tasks: Vec<Box<dyn FnOnce(&Yielder) + Send + 'static>>,
    /// `Some(reason)` when the completed run violates the invariant.
    pub probe: Box<dyn FnOnce() -> Option<String>>,
}

/// Coverage report of one exhaustive enumeration.
#[derive(Clone, Debug, Default)]
pub struct ExhaustiveReport {
    /// Tree nodes visited (one real run each).
    pub nodes: usize,
    /// Complete schedules enumerated (leaves).
    pub leaves: usize,
    /// Distinct leaf schedule hashes (sanity: == leaves).
    pub distinct_leaves: usize,
    /// Violating leaves: (grant sequence, reason).
    pub violators: Vec<(Vec<usize>, String)>,
    /// True when `cap_nodes` truncated the DFS — bounded coverage, NOT
    /// exhaustive; must be reported as such.
    pub hit_cap: bool,
    /// True when some run deviated from its recorded prefix (determinism
    /// break in the harness itself).
    pub diverged: bool,
    /// How many nodes deviated from their recorded prefix (per-node count
    /// behind `diverged`). On non-deterministic scenarios (live engine
    /// under real threads) the enumerated space is a lower bound.
    pub diverged_nodes: usize,
}

/// Enumerate **every** turnstile grant sequence for `n` tasks: DFS over
/// the schedule tree via prefix replay (no sampling, no seed). Each node
/// runs the real scenario under a recorded grant prefix; the enabled set
/// at prefix exhaustion are that node's branches; a prefix that carries
/// the run to completion is a leaf (one complete schedule). `cap_nodes`
/// bounds the work (`None` = unbounded); a capped run reports
/// `hit_cap` and is bounded coverage, never "exhaustive".
///
/// Piso que isto NÃO derruba: exhaustive over this harness's grant space
/// for THIS scenario is not ∀ OS interleavings (R-pct / R-glue seguem);
/// `forall_schedules_admitted` stays false.
pub fn run_exhaustive(
    n: usize,
    cap_nodes: Option<usize>,
    setup: impl Fn() -> ExhaustiveSetup,
) -> ExhaustiveReport {
    let mut report = ExhaustiveReport::default();
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut stack: Vec<Vec<usize>> = vec![Vec::new()];
    while let Some(prefix) = stack.pop() {
        if let Some(cap) = cap_nodes {
            if report.nodes >= cap {
                report.hit_cap = true;
                break;
            }
        }
        let st = setup();
        let mut sched = PrefixSched {
            prefix: prefix.clone(),
            pos: 0,
            branch: None,
            recorded: false,
            diverged: false,
        };
        let rep = run_with_sched(n, st.tasks, &mut sched);
        report.nodes += 1;
        if sched.diverged {
            report.diverged = true;
            report.diverged_nodes += 1;
        }
        match sched.branch {
            Some(enabled) => {
                // Internal node: queue each branch (pushed ascending so the
                // DFS pops the highest index first — preemption-heavy
                // prefixes surface early under a cap).
                for &e in &enabled {
                    let mut child = prefix.clone();
                    child.push(e);
                    stack.push(child);
                }
            }
            None => {
                // Leaf: the prefix carried the run to completion.
                report.leaves += 1;
                if seen.insert(rep.schedule_hash) {
                    report.distinct_leaves += 1;
                }
                if let Some(reason) = (st.probe)() {
                    let grants = rep.steps.iter().map(|s| s.worker).collect();
                    report.violators.push((grants, reason));
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ---------------------------------------------------------------------
    // Planted depth-2 bug (TEST CODE ONLY — never in the engine).
    //
    // `withdraw_all`: check-then-act on a shared balance with a yield
    // **inside** the op (between check and act) in the fine (AS-IS) mode.
    // Coarse mode yields only between ops (op-atomic π) — the model the
    // coarse schedulers can express. Invariant: balance >= 0.
    // ---------------------------------------------------------------------
    struct Plant {
        balance: Mutex<i64>,
        op_atomic: bool,
    }

    impl Plant {
        fn withdraw_all(&self, y: &Yielder) {
            let trace = std::env::var_os("PCT_TRACE").is_some();
            {
                let mut b = self.balance.lock().unwrap();
                if *b < 100 {
                    if trace {
                        eprintln!("TRACE plant task={} check-fail bal={}", y.task, *b);
                    }
                    return; // nothing to take
                }
                if !self.op_atomic {
                    // AS-IS: preemption window between check and act.
                    if trace {
                        eprintln!("TRACE plant task={} check-ok bal={}", y.task, *b);
                    }
                    drop(b);
                    y.at("check_act");
                    b = self.balance.lock().unwrap();
                    if trace {
                        eprintln!("TRACE plant task={} act bal={}", y.task, *b);
                    }
                }
                *b -= 100; // act without re-check (the planted bug)
            }
            if self.op_atomic {
                y.at("op_done");
            }
        }

        fn violation(&self) -> Option<String> {
            let b = *self.balance.lock().unwrap();
            (b < 0).then(|| format!("double_spend: balance={b}"))
        }
    }

    fn plant_run(seed: u64, n: usize, ops: usize, policy: PiPolicy, op_atomic: bool) -> RunReport {
        let plant = Arc::new(Plant {
            balance: Mutex::new(100),
            op_atomic,
        });
        run_pcts(seed, n, policy, |_task| {
            let plant = Arc::clone(&plant);
            move |y: &Yielder| {
                for _ in 0..ops {
                    plant.withdraw_all(y);
                }
            }
        })
    }

    fn plant_violation(seed: u64, n: usize, ops: usize, policy: PiPolicy) -> Option<String> {
        let plant = Arc::new(Plant {
            balance: Mutex::new(100),
            op_atomic: false,
        });
        run_pcts(seed, n, policy, |_task| {
            let plant = Arc::clone(&plant);
            move |y: &Yielder| {
                for _ in 0..ops {
                    plant.withdraw_all(y);
                }
            }
        });
        plant.violation()
    }

    #[test]
    fn planted_depth2_three_teeth() {
        const N: usize = 3;
        const OPS: usize = 3;
        const SEEDS: u64 = 256;

        // (i) sequential, fine plant: 0/256 — one task drains, others no-op.
        let seq_violators: Vec<u64> = (0..SEEDS)
            .filter(|&s| plant_violation(s, N, OPS, PiPolicy::Sequential).is_some())
            .collect();
        if std::env::var_os("PCT_TRACE").is_some() {
            eprintln!("TRACE seq violators: {seq_violators:?}");
        }
        let seq_hits = seq_violators.len();
        assert_eq!(seq_hits, 0, "sequential must be CLEAN on the fine plant");

        // (ii) coarse round-robin on the op-atomic plant: 0/256 — the π
        // grosso cannot express intra-op preemption.
        let rr_hits = (0..SEEDS)
            .filter(|&s| {
                let plant = Arc::new(Plant {
                    balance: Mutex::new(100),
                    op_atomic: true,
                });
                run_pcts(s, N, PiPolicy::RoundRobin, |_task| {
                    let plant = Arc::clone(&plant);
                    move |y: &Yielder| {
                        for _ in 0..OPS {
                            plant.withdraw_all(y);
                        }
                    }
                });
                plant.violation().is_some()
            })
            .count();
        assert_eq!(rr_hits, 0, "coarse round-robin must be CLEAN (π grosso)");

        // (iii) PCT d=2 on the fine plant: >=1/256.
        let violators: Vec<(u64, String)> = (0..SEEDS)
            .filter_map(|s| plant_violation(s, N, OPS, PiPolicy::Pct { depth: 2 }).map(|v| (s, v)))
            .collect();
        let pct_hits = violators.len();
        assert!(
            pct_hits >= 1,
            "PCT d=2 must find the planted depth-2 bug in 0..255 (got {pct_hits}/256)"
        );
        let (vs, vv) = violators[0].clone();

        // Replay the violating seed 8x: same violation, and the schedule
        // hash is bit-stable.
        let baseline = plant_run(vs, N, OPS, PiPolicy::Pct { depth: 2 }, false);
        for _ in 0..8 {
            let r = plant_run(vs, N, OPS, PiPolicy::Pct { depth: 2 }, false);
            assert_eq!(
                r.schedule_hash, baseline.schedule_hash,
                "replay must be bit-stable"
            );
            assert_eq!(
                plant_violation(vs, N, OPS, PiPolicy::Pct { depth: 2 }),
                Some(vv.clone()),
                "same seed must reproduce the same violation"
            );
        }

        // Telemetry: hit rate over the band (evidence, not a product claim).
        eprintln!("pct_concurrent: planted d=2 hits {pct_hits}/256 (first seed {vs}: {vv})");
    }

    /// RFC-0064 / 0063 P2.2: d>2 is the same runner, deeper change points.
    /// Seq/RR stay CLEAN; PCT d=3 finds the plant and replays.
    #[test]
    fn planted_depth3_three_teeth() {
        const N: usize = 3;
        const OPS: usize = 4;
        const SEEDS: u64 = 64;
        let seq_hits = (0..SEEDS)
            .filter(|&s| plant_violation(s, N, OPS, PiPolicy::Sequential).is_some())
            .count();
        assert_eq!(seq_hits, 0, "sequential must stay CLEAN at d=3 band");
        let violators: Vec<(u64, String)> = (0..SEEDS)
            .filter_map(|s| plant_violation(s, N, OPS, PiPolicy::Pct { depth: 3 }).map(|v| (s, v)))
            .collect();
        assert!(
            !violators.is_empty(),
            "PCT d=3 must find the planted bug in 0..63"
        );
        let (vs, vv) = violators[0].clone();
        let baseline = plant_run(vs, N, OPS, PiPolicy::Pct { depth: 3 }, false);
        for _ in 0..8 {
            let r = plant_run(vs, N, OPS, PiPolicy::Pct { depth: 3 }, false);
            assert_eq!(r.schedule_hash, baseline.schedule_hash);
            assert_eq!(
                plant_violation(vs, N, OPS, PiPolicy::Pct { depth: 3 }),
                Some(vv.clone())
            );
        }
        eprintln!(
            "pct_concurrent: planted d=3 hits {}/{} (first seed {vs}: {vv})",
            violators.len(),
            SEEDS
        );
    }

    /// RFC-0157 P1.3 — exhaustive enumeration (no sampling, no seed) of the
    /// turnstile grant space for the planted scenarios: depth-2 plant
    /// (N=3, OPS=3) and the chain-3 plant (N=3, OPS=4). The full space is
    /// enumerated; the violations PCT only samples are counted leaves of a
    /// complete space. Piso: exhaustive over this harness's grant space is
    /// still not ∀ OS interleavings (R-pct / R-glue seguem).
    #[test]
    fn rfc0157_exhaustive_plants() {
        const N: usize = 3;
        for (ops, tag) in [(3usize, "d2"), (4usize, "chain3")] {
            let report = run_exhaustive(N, None, || {
                let plant = Arc::new(Plant {
                    balance: Mutex::new(100),
                    op_atomic: false,
                });
                let mut tasks = Vec::with_capacity(N);
                for _ in 0..N {
                    let plant = Arc::clone(&plant);
                    tasks.push(Box::new(move |y: &Yielder| {
                        for _ in 0..ops {
                            plant.withdraw_all(y);
                        }
                    }) as Box<dyn FnOnce(&Yielder) + Send + 'static>);
                }
                let probe_plant = Arc::clone(&plant);
                ExhaustiveSetup {
                    tasks,
                    probe: Box::new(move || probe_plant.violation()),
                }
            });
            assert!(!report.diverged, "prefix replay diverged (determinism break)");
            assert!(!report.hit_cap, "{tag}: unbounded enumeration must complete");
            assert_eq!(
                report.leaves, report.distinct_leaves,
                "{tag}: duplicate leaf schedules"
            );
            assert!(
                !report.violators.is_empty(),
                "{tag}: exhaustive enumeration must contain the planted violation"
            );
            eprintln!(
                "rfc0157_exhaustive_plants {tag}: nodes={} |space|={} violators={} first={:?}",
                report.nodes,
                report.leaves,
                report.violators.len(),
                report.violators[0]
            );
        }
    }

    /// RFC-0157 P1.3 — the group-commit publish path under the exhaustive
    /// runner: the disk-fence scenario (single off-lock EIO fencing 2+
    /// members — the `pct_disk_fence_three_teeth` shape) enumerated under
    /// a node cap. A capped run is bounded coverage, never "exhaustive";
    /// `hit_cap` is reported as-is. Piso idem R-pct.
    #[test]
    fn rfc0157_exhaustive_disk_fence_bounded() {
        use pedradb_core::{ConcurrentDb, CoreError, OpenOptions};
        use pedradb_sim::{FailingEnvArc, FaultKind};
        use std::sync::atomic::{AtomicUsize, Ordering};

        const N: usize = 3;
        const COMMITS: usize = 3;
        const AFTER_FS: u64 = 5;
        const CAP: usize = 3000;
        static NODE: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!("pedra-xh-fence-{}", std::process::id()));

        let report = run_exhaustive(N, Some(CAP), || {
            let dir = base.join(format!("x-{}", NODE.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&dir);
            let env = FailingEnvArc::passing();
            let opts = OpenOptions {
                sync: true,
                ..OpenOptions::default()
            };
            let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = Arc::new(db);
            // Arm AFTER open: failures only touch this node-run's fsyncs.
            env.arm_with_kind(AFTER_FS, true, FaultKind::SyncFail);
            let outcomes: std::sync::Arc<Mutex<Vec<(usize, &'static str)>>> =
                std::sync::Arc::new(Mutex::new(Vec::new()));
            let mut tasks = Vec::with_capacity(N);
            for task in 0..N {
                let db = Arc::clone(&db);
                let outcomes = Arc::clone(&outcomes);
                tasks.push(Box::new(move |_y: &Yielder| {
                    for i in 0..COMMITS {
                        let k = format!("fence/{task}/{i}");
                        let mut tx = db.begin_occ();
                        tx.put(k.as_bytes(), b"v").unwrap();
                        let tag = match tx.commit() {
                            Ok(()) => "ok",
                            Err(CoreError::Internal(m))
                                if m.starts_with("group wal write/sync failed") =>
                            {
                                "group_fence"
                            }
                            Err(CoreError::DurabilityFenced) => "refused",
                            Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                        };
                        outcomes.lock().unwrap().push((task * COMMITS + i, tag));
                    }
                }) as Box<dyn FnOnce(&Yielder) + Send + 'static>);
            }
            let probe_outcomes = Arc::clone(&outcomes);
            ExhaustiveSetup {
                tasks,
                probe: Box::new(move || {
                    let members = probe_outcomes
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(_, t)| *t == "group_fence")
                        .count();
                    let _ = std::fs::remove_dir_all(&dir);
                    (members >= 2).then(|| format!("members={members}"))
                }),
            }
        });
        // No hard `!diverged` assert here: the live ConcurrentDb path is
        // not prefix-deterministic (that nondeterminism is exactly the
        // R-glue/R-pct floor); diverged nodes are counted and reported.
        assert!(
            !report.violators.is_empty(),
            "a fencing schedule (members>=2) must appear within {CAP} nodes"
        );
        eprintln!(
            "rfc0157_exhaustive_disk_fence: nodes={} |space|={} violators={} hit_cap={} diverged={} (diverged>0: live ConcurrentDb is not prefix-deterministic — R-glue/R-pct floor; the enumerated space is a lower bound)",
            report.nodes,
            report.leaves,
            report.violators.len(),
            report.hit_cap,
            report.diverged_nodes
        );
    }

    /// P1.1 (RFC-0051): π × disk. `FailingEnvArc` armed with a one-shot
    /// `SyncFail` after 5 successful group fsyncs — the 6th off-lock
    /// `fdatasync` (`finish_group_off_lock`) fails once and fences the
    /// whole group. Commits are **OCC** (`submit_occ`): the lone-writer
    /// fast path is skipped unconditionally, so group membership is a
    /// pure function of the grant sequence (no MULTI_HOLD wall clock in
    /// the oracle). The dente: only a preempting π can park the leader
    /// mid-commit (`lead_write_lock`) long enough for a follower to
    /// enqueue, so the single EIO fences **2+ members at once**.
    /// Run-to-completion (no preemption, incl. at the fsync) never forms
    /// a multi-member group: same workload, same arm point, misses.
    #[test]
    fn pct_disk_fence_three_teeth() {
        use pedradb_core::{ConcurrentDb, CoreError, OpenOptions, StdEnv};
        use pedradb_sim::{FailingEnvArc, FaultKind};
        use std::sync::Mutex;

        const N: usize = 3;
        const COMMITS: usize = 3;
        const AFTER_FS: u64 = 5; // EIO on the 6th group fsync
        const SEEDS: u64 = 256;
        let base = std::env::temp_dir().join(format!("pedra-pct-fence-{}", std::process::id()));

        // One trial: fresh dir/db, `COMMITS` OCC puts per task. Returns
        // (members of the EIO-fenced group, oks, refused, other, tripped,
        // hash, outcomes).
        #[allow(clippy::type_complexity)]
        let trial = |tag: &str, seed: u64, policy: PiPolicy| {
            let dir = base.join(format!("{tag}-{seed:03}"));
            let _ = std::fs::remove_dir_all(&dir);
            let env = FailingEnvArc::passing();
            let opts = OpenOptions {
                sync: true,
                ..OpenOptions::default()
            };
            let db = ConcurrentDb::open_with_env(&dir, opts.clone(), env.clone()).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = std::sync::Arc::new(db);
            // Arm AFTER open: failures only touch the trial's group fsyncs.
            env.arm_with_kind(AFTER_FS, true, FaultKind::SyncFail);

            let outcomes: std::sync::Arc<Mutex<Vec<(usize, &'static str)>>> =
                std::sync::Arc::new(Mutex::new(Vec::new()));
            let report = run_pcts(seed, N, policy, |task| {
                let db = std::sync::Arc::clone(&db);
                let outcomes = std::sync::Arc::clone(&outcomes);
                move |_y: &Yielder| {
                    for i in 0..COMMITS {
                        let k = format!("fence/{task}/{i}");
                        let mut tx = db.begin_occ();
                        tx.put(k.as_bytes(), b"v").unwrap();
                        let tag = match tx.commit() {
                            Ok(()) => "ok",
                            Err(CoreError::Internal(m))
                                if m.starts_with("group wal write/sync failed") =>
                            {
                                "group_fence"
                            }
                            Err(CoreError::DurabilityFenced) => "refused",
                            Err(e) => {
                                // unexpected class — surface it in the assert
                                Box::leak(format!("{e}").into_boxed_str())
                            }
                        };
                        outcomes.lock().unwrap().push((task * COMMITS + i, tag));
                    }
                }
            });
            let mut got = outcomes.lock().unwrap().clone();
            got.sort_unstable();
            let members = got.iter().filter(|(_, t)| *t == "group_fence").count();
            let oks = got.iter().filter(|(_, t)| *t == "ok").count();
            let refused = got.iter().filter(|(_, t)| *t == "refused").count();
            let other = got.len() - members - oks - refused;
            let tripped = env.tripped();
            drop(db);

            // silent_wrong oracle: every Ok commit fsynced before the EIO,
            // so all its keys must survive a reopen.
            let re = ConcurrentDb::open_with_env(&dir, opts, StdEnv).unwrap();
            for &(idx, tag) in &got {
                if tag == "ok" {
                    let (t, i) = (idx / COMMITS, idx % COMMITS);
                    let k = format!("fence/{t}/{i}");
                    assert!(
                        re.get(k.as_bytes()).is_some(),
                        "silent wrong: ok commit {k} vanished on reopen"
                    );
                }
            }
            drop(re);
            let _ = std::fs::remove_dir_all(&dir);
            (
                members,
                oks,
                refused,
                other,
                tripped,
                report.schedule_hash,
                got,
            )
        };

        // (i) grosso: run-to-completion (no preemption anywhere, incl. the
        // off-lock fsync) — same workload, same arm point: 0 hits in 0..255.
        let seq_hits = (0..SEEDS)
            .filter(|&s| {
                let (members, oks, refused, other, tripped, _h, got) =
                    trial("seq", s, PiPolicy::Sequential);
                assert_eq!(
                    other, 0,
                    "sequential trial saw an unexpected error class: {got:?}"
                );
                assert!(tripped, "SyncFail must fire in every sequential trial");
                assert_eq!(members + refused + oks, N * COMMITS);
                members >= 2
            })
            .count();
        assert_eq!(
            seq_hits, 0,
            "run-to-completion must miss the multi-member fence"
        );

        // (ii) AS-IS: PCT d=2 preempts the leader mid-commit; some seed
        // fences 2+ members with the single off-lock EIO.
        let violators: Vec<(u64, usize)> = (0..SEEDS)
            .filter_map(|s| {
                let (members, oks, refused, other, tripped, _h, got) =
                    trial("pct", s, PiPolicy::Pct { depth: 2 });
                assert_eq!(other, 0, "pct trial saw an unexpected error class: {got:?}");
                assert_eq!(members + refused + oks, N * COMMITS);
                if tripped {
                    (members >= 2).then_some((s, members))
                } else {
                    // heavy merging can keep the fsync count <= AFTER_FS;
                    // no fault fired, nothing to fence — not a hit.
                    assert_eq!(oks, N * COMMITS, "no EIO but not all Ok: {got:?}");
                    None
                }
            })
            .collect();
        let pct_hits = violators.len();
        assert!(
            pct_hits >= 1,
            "PCT d=2 must fence a multi-member group in 0..255 (got {pct_hits}/256)"
        );
        let (vs, vm) = violators[0];

        // (iii) replay the hitting seed 8x: same membership, same outcomes,
        // bit-stable schedule hash.
        let (_, _, _, _, _, baseline, base_got) = trial("pct", vs, PiPolicy::Pct { depth: 2 });
        for _ in 0..8 {
            let (members, oks, refused, other, _tripped, h, got) =
                trial("pct", vs, PiPolicy::Pct { depth: 2 });
            assert_eq!(other, 0);
            assert_eq!(
                (members, oks, refused),
                (
                    vm,
                    base_got.iter().filter(|(_, t)| *t == "ok").count(),
                    base_got.iter().filter(|(_, t)| *t == "refused").count()
                )
            );
            assert_eq!(got, base_got, "same seed must reproduce the same outcomes");
            assert_eq!(h, baseline, "replay must be bit-stable");
        }

        let _ = std::fs::remove_dir_all(&base);
        eprintln!(
            "pct_concurrent: pi x disk fence dente: seq {seq_hits}/256 (miss), pct {pct_hits}/256 (first seed {vs}: {vm} members fenced at the off-lock EIO)"
        );
    }

    /// RFC-0071 P1.2: yield after off-lock fd (`after_wal_sync`); failed
    /// sync must not publish. Sequential stays on the lone path (never
    /// parks at that site). PCT d=2 forms a group, hits the site, still
    /// unpublished; replay 8× bit-stable. AS-IS kernel would publish.
    #[test]
    fn pct_after_failed_fd_does_not_publish() {
        use pedradb_core::{ConcurrentDb, CoreError, OpenOptions, StdEnv};
        use pedradb_sim::{FailingEnvArc, FaultKind};
        use std::sync::Mutex;

        const N: usize = 2;
        const COMMITS: usize = 2;
        const SEEDS: u64 = 64;
        let base = std::env::temp_dir().join(format!("pedra-pct-after-fd-{}", std::process::id()));

        let trial = |tag: &str, seed: u64, policy: PiPolicy| {
            let dir = base.join(format!("{tag}-{seed:03}"));
            let _ = std::fs::remove_dir_all(&dir);
            let env = FailingEnvArc::passing();
            let opts = OpenOptions {
                sync: true,
                ..OpenOptions::default()
            };
            let db = ConcurrentDb::open_with_env(&dir, opts.clone(), env.clone()).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = std::sync::Arc::new(db);
            env.arm_with_kind(0, true, FaultKind::SyncFail);

            let outcomes: std::sync::Arc<Mutex<Vec<(usize, &'static str)>>> =
                std::sync::Arc::new(Mutex::new(Vec::new()));
            let report = run_pcts(seed, N, policy, |task| {
                let db = std::sync::Arc::clone(&db);
                let outcomes = std::sync::Arc::clone(&outcomes);
                move |_y: &Yielder| {
                    for i in 0..COMMITS {
                        let k = format!("afterfd/{task}/{i}");
                        let tag = match db.put(k.as_bytes(), b"v") {
                            Ok(()) => "ok",
                            Err(CoreError::Internal(m))
                                if m.starts_with("group wal write/sync failed") =>
                            {
                                "group_fence"
                            }
                            Err(CoreError::DurabilityFenced) => "refused",
                            Err(_) => "err",
                        };
                        outcomes.lock().unwrap().push((task * COMMITS + i, tag));
                    }
                }
            });
            let mut got = outcomes.lock().unwrap().clone();
            got.sort_unstable();
            let after_fd = report
                .steps
                .iter()
                .any(|s| s.site == "after_wal_sync");
            // Live unpublished: a failed put must not be visible before reopen.
            let mut silent_wrong = 0usize;
            for &(idx, tag) in &got {
                let (t, i) = (idx / COMMITS, idx % COMMITS);
                let k = format!("afterfd/{t}/{i}");
                let live = db.get(k.as_bytes()).is_some();
                if tag != "ok" && live {
                    silent_wrong += 1;
                }
            }
            drop(db);
            let re = ConcurrentDb::open_with_env(&dir, opts, StdEnv).unwrap();
            for &(idx, tag) in &got {
                let (t, i) = (idx / COMMITS, idx % COMMITS);
                let k = format!("afterfd/{t}/{i}");
                if tag == "ok" && re.get(k.as_bytes()).is_none() {
                    silent_wrong += 1;
                }
            }
            drop(re);
            let _ = std::fs::remove_dir_all(&dir);
            (after_fd, silent_wrong, report.schedule_hash, got)
        };

        let seq_site = (0..SEEDS)
            .filter(|&s| {
                let (after_fd, sw, _h, got) = trial("seq", s, PiPolicy::Sequential);
                assert_eq!(sw, 0, "sequential silent_wrong: {got:?}");
                after_fd
            })
            .count();
        assert_eq!(
            seq_site, 0,
            "sequential must miss after_wal_sync (lone path)"
        );

        let violators: Vec<u64> = (0..SEEDS)
            .filter(|&s| {
                let (after_fd, sw, _h, got) = trial("pct", s, PiPolicy::Pct { depth: 2 });
                assert_eq!(sw, 0, "pct silent_wrong: {got:?}");
                after_fd
            })
            .collect();
        assert!(
            !violators.is_empty(),
            "PCT d=2 must park at after_wal_sync in 0..{SEEDS}"
        );
        let vs = violators[0];
        let (_, _, baseline, base_got) = trial("pct", vs, PiPolicy::Pct { depth: 2 });
        for _ in 0..8 {
            let (after_fd, sw, h, got) = trial("pct", vs, PiPolicy::Pct { depth: 2 });
            assert!(after_fd);
            assert_eq!(sw, 0);
            assert_eq!(got, base_got);
            assert_eq!(h, baseline, "replay must be bit-stable");
        }
        assert!(
            pedradb_core::group_commit_kernel::may_publish_group_as_is(false),
            "AS-IS dente: publish after failed WAL I/O"
        );
        assert!(!pedradb_core::group_commit_kernel::may_publish_group(false));
        let _ = std::fs::remove_dir_all(&base);
        eprintln!(
            "pct_concurrent: after_wal_sync dente: seq {seq_site}/{SEEDS} (miss), pct {}/{SEEDS} (first seed {vs})",
            violators.len()
        );
    }

    /// P2.1 (RFC-0058): PCT over the **verified profile with the merge
    /// back** — same π×disk teeth, but the DB runs the proved group-commit
    /// kernel: sync puts and OCC txs may share a leader and one fsync,
    /// async `no_sync` puts keep the un-merged bypass. No matter how π
    /// preempts: outcomes are only ok / fenced / refused / ok_async, every
    /// sync-Ok commit survives the reopen (`silent_wrong = 0`), async
    /// survivors are never wrong, and across the seed set the merge
    /// actually engages under preemption (`queued > 0` somewhere — the
    /// comparison is not vacuous). A fence may hit several members of one
    /// group: with group atomicity proven, that is the shape, not a bug.
    #[test]
    fn pct_verified_merges_under_preemption() {
        use pedradb_core::{ConcurrentDb, CoreError, OpenOptions, StdEnv, WriteOptions};
        use pedradb_sim::{FailingEnvArc, FaultKind};
        use std::sync::Mutex;

        const N: usize = 3;
        const COMMITS: usize = 3;
        const AFTER_FS: u64 = 5; // one-shot EIO on the 6th group fsync
        const SEEDS: u64 = 64;
        let base = std::env::temp_dir().join(format!("pedra-pct-verified-{}", std::process::id()));

        #[allow(clippy::type_complexity)]
        let trial = |tag: &str, seed: u64, policy: PiPolicy| {
            let dir = base.join(format!("{tag}-{seed:03}"));
            let _ = std::fs::remove_dir_all(&dir);
            let env = FailingEnvArc::passing();
            let opts = OpenOptions::verified();
            let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).unwrap();
            db.pin_verified();
            assert!(db.is_verified());
            env.arm_with_kind(AFTER_FS, true, FaultKind::SyncFail);
            let db = std::sync::Arc::new(db);

            let outcomes: std::sync::Arc<Mutex<Vec<(usize, &'static str)>>> =
                std::sync::Arc::new(Mutex::new(Vec::new()));
            let report = run_pcts(seed, N, policy, |task| {
                let db = std::sync::Arc::clone(&db);
                let outcomes = std::sync::Arc::clone(&outcomes);
                move |_y: &Yielder| {
                    for i in 0..COMMITS {
                        let k = format!("vrf/{task}/{i}");
                        // Three shapes: sync put / OCC tx / async no_sync put.
                        let tag = match task % 3 {
                            0 => match db.put(k.as_bytes(), b"v") {
                                Ok(()) => "ok",
                                Err(CoreError::Internal(m))
                                    if m.starts_with("group wal write/sync failed") =>
                                {
                                    "fence"
                                }
                                Err(CoreError::DurabilityFenced) => "refused",
                                Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                            },
                            1 => {
                                let mut tx = db.begin_occ();
                                tx.put(k.as_bytes(), b"v").unwrap();
                                match tx.commit() {
                                    Ok(()) => "ok",
                                    Err(CoreError::Internal(m))
                                        if m.starts_with("group wal write/sync failed") =>
                                    {
                                        "fence"
                                    }
                                    Err(CoreError::DurabilityFenced) => "refused",
                                    Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                                }
                            }
                            _ => match db.put_with(k.as_bytes(), b"v", WriteOptions::no_sync()) {
                                // Separate tag: async Ok promises no
                                // durability before a barrier — the oracle
                                // below only checks survivors' values.
                                Ok(()) => "ok_async",
                                Err(CoreError::DurabilityFenced) => "refused",
                                Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                            },
                        };
                        outcomes.lock().unwrap().push((task * COMMITS + i, tag));
                    }
                }
            });
            let mut got = outcomes.lock().unwrap().clone();
            got.sort_unstable();
            let fenced = got.iter().filter(|(_, t)| *t == "fence").count();
            let oks = got.iter().filter(|(_, t)| *t == "ok").count();
            let async_oks = got.iter().filter(|(_, t)| *t == "ok_async").count();
            let refused = got.iter().filter(|(_, t)| *t == "refused").count();
            let other = got.len() - fenced - oks - async_oks - refused;
            let stats = db.write_group_stats();
            let tripped = env.tripped();
            drop(db);

            // silent_wrong oracle: every sync/OCC Ok commit fsynced before
            // any EIO fence, so all its keys must survive the reopen. Async
            // Oks promise no durability before a barrier (the DB is dropped,
            // not closed) — survivors must simply never be wrong.
            let re = ConcurrentDb::open_with_env(&dir, opts, StdEnv).unwrap();
            for &(idx, tag) in &got {
                let (t, i) = (idx / COMMITS, idx % COMMITS);
                let k = format!("vrf/{t}/{i}");
                match tag {
                    "ok" => assert!(
                        re.get(k.as_bytes()).is_some(),
                        "silent wrong: ok commit {k} vanished on reopen"
                    ),
                    "ok_async" => {
                        if let Some(v) = re.get(k.as_bytes()) {
                            assert_eq!(
                                v.as_ref(),
                                &b"v"[..],
                                "async survivor {k} has a wrong value"
                            );
                        }
                    }
                    _ => {}
                }
            }
            drop(re);
            let _ = std::fs::remove_dir_all(&dir);
            (
                fenced,
                oks,
                async_oks,
                refused,
                other,
                tripped,
                stats,
                report.schedule_hash,
            )
        };

        for policy in [PiPolicy::Sequential, PiPolicy::Pct { depth: 2 }] {
            let mut fenced_total = 0usize;
            let mut queued_total = 0u64;
            for s in 0..SEEDS {
                let (fenced, oks, async_oks, refused, other, tripped, stats, _h) = trial(
                    if matches!(policy, PiPolicy::Sequential) {
                        "seq"
                    } else {
                        "pct"
                    },
                    s,
                    policy,
                );
                assert_eq!(other, 0, "unexpected error class in verified trial {s}");
                assert_eq!(fenced + refused + oks + async_oks, N * COMMITS);
                if !tripped {
                    // Merging keeps the group fsync count <= AFTER_FS: no
                    // fault fired, nothing fenced — everything must be Ok.
                    assert_eq!(
                        oks + async_oks,
                        N * COMMITS,
                        "no EIO yet not all Ok (trial {s})"
                    );
                }
                let (submits, queued, batches, batch_ops) = stats;
                assert_eq!(submits, (N * COMMITS) as u64);
                assert!(batches <= submits, "trial {s}: batches {batches} > submits");
                assert_eq!(batch_ops, (N * COMMITS) as u64);
                queued_total += queued;
                fenced_total += fenced;
            }
            assert!(
                fenced_total >= 1,
                "the EIO must fence someone across {SEEDS} seeds"
            );
            if matches!(policy, PiPolicy::Pct { .. }) {
                assert!(
                    queued_total > 0,
                    "verified mode never merged across {SEEDS} seeds — the merge is not exercised"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// RFC-0058 P1.3→P2.1: semantics derivation — the verified profile
    /// must keep the **same safety oracles** as the full mode while both
    /// run the (now shared) proved group-commit kernel. Same seeds, same
    /// PCT schedules, same three write shapes (sync put / OCC tx / async
    /// `no_sync` put), same one-shot EIO on the 6th group fsync. The
    /// safety oracles are **identical** in both modes: every sync/OCC Ok
    /// survives the reopen (`silent_wrong == 0`), every async survivor
    /// holds the right value, the reopen invents nothing (ghost-free,
    /// value-exact scan over the written range), and the only error
    /// classes are fence/refused. Since RFC-0058 P2.1 the group
    /// semantics are the same kernel in both modes (the full side pins
    /// the same ZERO catch-up window the verified pin forces) — so the
    /// derivation asserts both modes actually merge under PCT
    /// preemption (each side non-vacuous), and a fence may hit several
    /// members of one group in either mode; what differs is the
    /// declared composition (verified forces sync, fail-closed, the
    /// un-merged async bypass), not the safety outcome.
    #[test]
    fn verified_vs_full_same_oracles() {
        use pedradb_core::{ConcurrentDb, CoreError, OpenOptions, StdEnv, WriteOptions};
        use pedradb_sim::{FailingEnvArc, FaultKind};
        use std::collections::HashSet;
        use std::ops::Bound;
        use std::sync::Mutex;

        const N: usize = 3;
        const COMMITS: usize = 3;
        const AFTER_FS: u64 = 5; // one-shot EIO on the 6th group fsync
        const SEEDS: u64 = 64;
        let base = std::env::temp_dir().join(format!("pedra-pct-derive-{}", std::process::id()));

        #[allow(clippy::type_complexity)]
        let trial = |tag: &str, seed: u64, policy: PiPolicy, verified: bool| {
            let dir = base.join(format!("{tag}-{seed:03}"));
            let _ = std::fs::remove_dir_all(&dir);
            let env = FailingEnvArc::passing();
            let opts = if verified {
                OpenOptions::verified()
            } else {
                OpenOptions {
                    sync: true,
                    ..OpenOptions::default()
                }
            };
            let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).unwrap();
            if verified {
                db.pin_verified();
            } else {
                // Same merge-friendly window the fence test uses (the
                // verified pin already implies ZERO).
                db.set_write_group_catchup_window(std::time::Duration::ZERO);
            }
            env.arm_with_kind(AFTER_FS, true, FaultKind::SyncFail);
            let db = std::sync::Arc::new(db);

            let outcomes: std::sync::Arc<Mutex<Vec<(usize, &'static str)>>> =
                std::sync::Arc::new(Mutex::new(Vec::new()));
            let _report = run_pcts(seed, N, policy, |task| {
                let db = std::sync::Arc::clone(&db);
                let outcomes = std::sync::Arc::clone(&outcomes);
                move |_y: &Yielder| {
                    for i in 0..COMMITS {
                        let k = format!("drv/{task}/{i}");
                        let tag = match task % 3 {
                            0 => match db.put(k.as_bytes(), b"v") {
                                Ok(()) => "ok",
                                Err(CoreError::Internal(m))
                                    if m.starts_with("group wal write/sync failed") =>
                                {
                                    "fence"
                                }
                                Err(CoreError::DurabilityFenced) => "refused",
                                Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                            },
                            1 => {
                                let mut tx = db.begin_occ();
                                tx.put(k.as_bytes(), b"v").unwrap();
                                match tx.commit() {
                                    Ok(()) => "ok",
                                    Err(CoreError::Internal(m))
                                        if m.starts_with("group wal write/sync failed") =>
                                    {
                                        "fence"
                                    }
                                    Err(CoreError::DurabilityFenced) => "refused",
                                    Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                                }
                            }
                            _ => match db.put_with(k.as_bytes(), b"v", WriteOptions::no_sync()) {
                                Ok(()) => "ok_async",
                                Err(CoreError::DurabilityFenced) => "refused",
                                Err(e) => Box::leak(format!("{e}").into_boxed_str()),
                            },
                        };
                        outcomes.lock().unwrap().push((task * COMMITS + i, tag));
                    }
                }
            });
            let mut got = outcomes.lock().unwrap().clone();
            got.sort_unstable();
            let fenced = got.iter().filter(|(_, t)| *t == "fence").count();
            let oks = got.iter().filter(|(_, t)| *t == "ok").count();
            let async_oks = got.iter().filter(|(_, t)| *t == "ok_async").count();
            let refused = got.iter().filter(|(_, t)| *t == "refused").count();
            let other = got.len() - fenced - oks - async_oks - refused;
            let stats = db.write_group_stats();
            let tripped = env.tripped();
            drop(db);

            // Shared safety oracle (must be identical across modes): Ok
            // sync/OCC ⇒ present after reopen; async survivor ⇒ right
            // value; reopen scan over the written range is ghost-free and
            // value-exact.
            let mut silent_wrong = 0usize;
            let mut wrong_async = 0usize;
            let re = ConcurrentDb::open_with_env(&dir, opts, StdEnv).unwrap();
            for &(idx, tag) in &got {
                let (t, i) = (idx / COMMITS, idx % COMMITS);
                let k = format!("drv/{t}/{i}");
                match tag {
                    "ok" => {
                        if re.get(k.as_bytes()).is_none() {
                            silent_wrong += 1;
                        }
                    }
                    "ok_async" => {
                        if let Some(v) = re.get(k.as_bytes()) {
                            if v.as_ref() != &b"v"[..] {
                                wrong_async += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
            let expected: HashSet<Vec<u8>> = (0..N)
                .flat_map(|t| (0..COMMITS).map(move |i| format!("drv/{t}/{i}").into_bytes()))
                .collect();
            let ghost = re
                .scan_collect(Bound::Included(&b"drv/"[..]), Bound::Excluded(&b"drv0"[..]))
                .iter()
                .filter(|(k, v)| !expected.contains(k.as_ref()) || v.as_ref() != &b"v"[..])
                .count();
            drop(re);
            let _ = std::fs::remove_dir_all(&dir);
            (
                oks,
                async_oks,
                fenced,
                refused,
                other,
                silent_wrong,
                wrong_async,
                ghost,
                tripped,
                stats,
            )
        };

        for policy in [PiPolicy::Sequential, PiPolicy::Pct { depth: 2 }] {
            let tag = if matches!(policy, PiPolicy::Sequential) {
                "seq"
            } else {
                "pct"
            };
            let mut full_queued_total = 0u64;
            let mut ver_queued_total = 0u64;
            for s in 0..SEEDS {
                let full = trial(&format!("{tag}-full"), s, policy, false);
                let ver = trial(&format!("{tag}-ver"), s, policy, true);

                // (a) identical safety oracles in both modes.
                for (mode, r) in [("full", &full), ("verified", &ver)] {
                    let (
                        oks,
                        async_oks,
                        fenced,
                        refused,
                        other,
                        silent_wrong,
                        wrong_async,
                        ghost,
                        tripped,
                        _stats,
                    ) = *r;
                    assert_eq!(other, 0, "{mode} trial {s}: unexpected error class");
                    assert_eq!(
                        oks + async_oks + fenced + refused,
                        N * COMMITS,
                        "{mode} trial {s}: outcome total"
                    );
                    assert_eq!(
                        silent_wrong, 0,
                        "{mode} trial {s}: Ok commit vanished on reopen"
                    );
                    assert_eq!(
                        wrong_async, 0,
                        "{mode} trial {s}: async survivor holds a wrong value"
                    );
                    assert_eq!(
                        ghost, 0,
                        "{mode} trial {s}: reopen invented or wrong-valued key"
                    );
                    if !tripped {
                        // Heavy merging (full mode) can keep the fsync count
                        // <= AFTER_FS: no fault fired, nothing fenced.
                        assert_eq!(
                            oks + async_oks,
                            N * COMMITS,
                            "{mode} trial {s}: no EIO yet not all Ok"
                        );
                    }
                }

                // (b) same kernel, same group semantics: both modes may
                // merge (and both must, somewhere across the seeds, or
                // the pairing is vacuous); a fence may hit several
                // members of one group in either mode. What differs is
                // the declared composition, not the safety outcome.
                let (_, _, ver_fenced, _, _, _, _, _, ver_tripped, ver_stats) = ver;
                let (submits, queued, batches, batch_ops) = ver_stats;
                assert_eq!(submits, (N * COMMITS) as u64, "verified trial {s}");
                assert!(
                    queued <= submits,
                    "verified trial {s}: queued {queued} > submits"
                );
                assert!(
                    batches <= submits,
                    "verified trial {s}: batches {batches} > submits"
                );
                assert_eq!(batch_ops, (N * COMMITS) as u64, "verified trial {s}");
                if !ver_tripped && ver_fenced > 0 {
                    panic!("verified trial {s}: fenced without a tripped EIO");
                }
                ver_queued_total += queued;
                // Full mode: same kernel, same shape.
                let (_, _, _, _, _, _, _, _, _, full_stats) = full;
                full_queued_total += full_stats.1;
            }
            if matches!(policy, PiPolicy::Pct { .. }) {
                assert!(
                    full_queued_total > 0,
                    "{tag}: full mode never merged across {SEEDS} seeds — vacuous comparison"
                );
                assert!(
                    ver_queued_total > 0,
                    "{tag}: verified mode never merged across {SEEDS} seeds — vacuous comparison"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// P1.3 (RFC-0051): OCC under the same runner, three-teeth ritual.
    /// Task 0 is the X-writer; tasks 1–2 read X and write a disjoint Z.
    /// Planted depth-2 bug (TEST CODE ONLY): the reader reads X raw
    /// (`db.get`, not the OCC read set), so an X-write that commits inside
    /// the reader's (snapshot, commit) window goes unnoticed — a
    /// non-serializable Ok. Correct mode records X in the read set: the
    /// real engine validation must turn exactly those windows into
    /// `TransactionConflict` (the REAL invariant under preemption).
    ///
    /// Group atomicity: members of one group commit commit at the same
    /// atomic instant (one write-lock hold, one WAL fsync), so seq order
    /// between them is NOT serialization order. A reader whose window
    /// contains only same-group writer seqs serializes before that writer
    /// (its raw read predates the group) — a valid Ok. The oracle therefore
    /// classifies each window via the run's recorded group seq-ranges
    /// (`RunReport::group_ranges`): only cross-group windows are violations.
    #[test]
    fn occ_three_teeth() {
        use pedradb_core::{BatchOp, ConcurrentDb, CoreError, OpenOptions, StdEnv};
        use std::sync::Mutex;

        const N: usize = 3;
        const ROUNDS: usize = 3;
        const SEEDS: u64 = 256;
        let base = std::env::temp_dir().join(format!("pedra-pct-occ-{}", std::process::id()));

        #[derive(Clone)]
        struct Rec {
            writer: bool,
            snapshot: u64,
            ok_seq: Option<u64>,
            conflict: bool,
        }

        /// One trial: (violation, conflicts, schedule_hash, simultaneous
        /// same-group windows observed — forensics, not a pass/fail input).
        fn trial(
            base: &std::path::Path,
            tag: &str,
            seed: u64,
            policy: PiPolicy,
            planted: bool,
        ) -> (Option<String>, usize, u64, usize) {
            use bytes::Bytes;
            let dir = base.join(format!("{tag}-{seed:03}"));
            let _ = std::fs::remove_dir_all(&dir);
            let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), StdEnv).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = std::sync::Arc::new(db);
            let recs: std::sync::Arc<Mutex<Vec<Rec>>> = std::sync::Arc::new(Mutex::new(Vec::new()));
            let report = run_pcts(seed, N, policy, |task| {
                let db = std::sync::Arc::clone(&db);
                let recs = std::sync::Arc::clone(&recs);
                move |y: &Yielder| {
                    for r in 0..ROUNDS {
                        let snap = db.begin_occ().snapshot();
                        let (writer, res) = if task == 0 {
                            let ops = vec![BatchOp::put(b"occ/x", format!("w{r}").as_bytes())];
                            (true, db.apply_batch_occ(snap, Vec::<Bytes>::new(), ops))
                        } else {
                            let z = db
                                .get(b"occ/x")
                                .unwrap_or_else(|| Bytes::from_static(b"init"));
                            // Planted window: between the raw read and the
                            // commit an X-write may land unseen.
                            y.at("rmw_window");
                            let zkey = format!("occ/z{task}");
                            let read_set: Vec<Bytes> = if planted {
                                Vec::new()
                            } else {
                                vec![Bytes::from_static(b"occ/x")]
                            };
                            let ops = vec![BatchOp::put(zkey.as_bytes(), &z)];
                            (false, db.apply_batch_occ(snap, read_set, ops))
                        };
                        let rec = match res {
                            Ok(seq) => Rec {
                                writer,
                                snapshot: snap,
                                ok_seq: Some(seq),
                                conflict: false,
                            },
                            Err(CoreError::TransactionConflict) => Rec {
                                writer,
                                snapshot: snap,
                                ok_seq: None,
                                conflict: true,
                            },
                            Err(e) => panic!("occ trial: unexpected error {e}"),
                        };
                        recs.lock().unwrap().push(rec);
                    }
                }
            });
            let rs = recs.lock().unwrap().clone();
            let ranges = report.group_ranges.clone();
            let same_group = |a: u64, b: u64| {
                ranges
                    .iter()
                    .any(|(lo, hi)| *lo <= a && a <= *hi && *lo <= b && b <= *hi)
            };
            let writer_seqs: Vec<u64> = rs
                .iter()
                .filter(|r| r.writer)
                .filter_map(|r| r.ok_seq)
                .collect();
            let mut violation = None;
            let mut simultaneous = 0usize;
            for r in &rs {
                if r.writer || r.conflict {
                    continue;
                }
                if let Some(seq) = r.ok_seq {
                    for &w in writer_seqs.iter().filter(|w| r.snapshot < **w && **w < seq) {
                        if same_group(w, seq) {
                            simultaneous += 1;
                        } else {
                            violation = Some(format!(
                                "non_serializable_ok snap={} w={w} seq={seq}",
                                r.snapshot
                            ));
                        }
                    }
                }
            }
            let conflicts = rs.iter().filter(|r| r.conflict).count();
            drop(db);
            let _ = std::fs::remove_dir_all(&dir);
            (violation, conflicts, report.schedule_hash, simultaneous)
        }

        // (i) grosso: run-to-completion on the planted reader — serialized
        // rounds never overlap a window: 0 violations in 0..255.
        let seq_hits = (0..SEEDS)
            .filter(|&s| {
                trial(&base, "seq", s, PiPolicy::Sequential, true)
                    .0
                    .is_some()
            })
            .count();
        assert_eq!(seq_hits, 0, "sequential must miss the planted stale read");

        // (ii) AS-IS: PCT d=2 preempts inside the window; the stale reader
        // still commits Ok (empty read set) — non-serializable.
        let violators: Vec<(u64, String)> = (0..SEEDS)
            .filter_map(|s| {
                trial(&base, "pct", s, PiPolicy::Pct { depth: 2 }, true)
                    .0
                    .map(|v| (s, v))
            })
            .collect();
        let pct_hits = violators.len();
        assert!(
            pct_hits >= 1,
            "PCT d=2 must find the planted non-serializable Ok (got {pct_hits}/256)"
        );
        let (vs, vv) = violators[0].clone();

        // (iii) replay the violating seed 8x: same violation, bit-stable.
        let (_, _, baseline, _) = trial(&base, "pct", vs, PiPolicy::Pct { depth: 2 }, true);
        for _ in 0..8 {
            let (v, _c, h, _) = trial(&base, "pct", vs, PiPolicy::Pct { depth: 2 }, true);
            assert_eq!(v.as_deref(), Some(vv.as_str()), "same seed, same violation");
            assert_eq!(h, baseline, "replay must be bit-stable");
        }

        // (iv) REAL invariant: correct read set — PCT preempts the same
        // windows, but engine validation converts every cross-group one
        // into a conflict: 0 violations, and the band must actually
        // conflict somewhere (otherwise (iv) is vacuous). Same-group
        // windows are simultaneous atomic commits (valid Oks) and are
        // counted as forensics, not violations.
        let mut correct_violations = 0usize;
        let mut correct_conflict_seeds = 0usize;
        let mut simultaneous_windows = 0usize;
        for s in 0..SEEDS {
            let (v, c, _h, sim) = trial(&base, "ok", s, PiPolicy::Pct { depth: 2 }, false);
            if v.is_some() {
                correct_violations += 1;
            }
            if c > 0 {
                correct_conflict_seeds += 1;
            }
            simultaneous_windows += sim;
        }
        assert_eq!(
            correct_violations, 0,
            "real OCC validation must hold under PCT preemption (cross-group windows)"
        );
        assert!(
            correct_conflict_seeds >= 1,
            "correct band must engage validation (conflicts > 0 somewhere)"
        );

        let _ = std::fs::remove_dir_all(&base);
        eprintln!(
            "pct_concurrent: occ dente: seq {seq_hits}/256 (miss), pct planted {pct_hits}/256 (first seed {vs}: {vv}), pct correct 0/256 with conflicts in {correct_conflict_seeds}/256 seeds ({simultaneous_windows} same-group windows, simultaneous by design)"
        );
    }

    /// P0.1: the runner drives the **real** `ConcurrentDb` (production
    /// group-commit / WAL-lock path) under PCT via the engine `maybe_yield`
    /// hooks — same seed replays the same grant sequence, all writes land,
    /// durable on reopen.
    #[test]
    fn pct_runner_drives_real_concurrentdb() {
        use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};

        let dir = std::env::temp_dir().join(format!("pedra-pct-cdb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let opts = OpenOptions {
            sync: false,
            ..OpenOptions::default()
        };
        let db = ConcurrentDb::open_with_env(&dir, opts.clone(), StdEnv).unwrap();
        db.set_write_group_catchup_window(std::time::Duration::ZERO);
        let db = Arc::new(db);

        const N: usize = 3;
        const PUTS: usize = 12;

        let run_once = |seed: u64| -> (u64, usize) {
            let db = Arc::clone(&db);
            let report = run_pcts(seed, N, PiPolicy::Pct { depth: 2 }, |task| {
                let db = Arc::clone(&db);
                move |_y: &Yielder| {
                    for i in 0..PUTS {
                        let k = format!("pct/{task}/{i}");
                        db.put(k.as_bytes(), b"v").unwrap();
                    }
                }
            });
            let total = (0..N)
                .map(|t| {
                    (0..PUTS)
                        .filter(|i| db.get(format!("pct/{t}/{i}").as_bytes()).is_some())
                        .count()
                })
                .sum::<usize>();
            (report.schedule_hash, total)
        };

        let (h1, t1) = run_once(0x0051_5EED);
        let (h2, t2) = run_once(0x0051_5EED);
        assert_eq!(t1, N * PUTS, "every put must be visible");
        assert_eq!(t2, N * PUTS);
        assert_eq!(h1, h2, "same seed must replay the same grant sequence");

        let db = Arc::try_unwrap(db)
            .map_err(|_| "workers still hold the db")
            .unwrap()
            .close()
            .unwrap();
        let _ = db;
        let re =
            ConcurrentDb::open_with_env(&dir, opts, StdEnv).expect("reopen after close failed");
        assert!(re.get(b"pct/0/0").is_some(), "durable after reopen");
        assert!(re.get(b"pct/2/11").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0070 P1.1: a live `run_pcts` at d=2 refuses ∀π. AS-IS would admit.
    #[test]
    fn pct_runner_refuses_forall_schedules_at_depth2() {
        use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};

        let dir = std::env::temp_dir().join(format!("pedra-pct-forall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), StdEnv).unwrap();
        db.set_write_group_catchup_window(std::time::Duration::ZERO);
        let db = Arc::new(db);
        let report = run_pcts(0x0070_0C72, 2, PiPolicy::Pct { depth: 2 }, |task| {
            let db = Arc::clone(&db);
            move |_y: &Yielder| {
                db.put(format!("f70/{task}").as_bytes(), b"v").unwrap();
            }
        });
        assert!(
            !report.steps.is_empty(),
            "run_pcts must grant at least one step"
        );
        assert!(
            !report.claim_forall_schedules(),
            "PCT d=2 must not round to forall schedules"
        );
        assert!(!pedradb_core::group_commit_kernel::forall_schedules_admitted(
            policy_pct_depth(PiPolicy::Pct { depth: 2 })
        ));
        assert!(
            pedradb_core::group_commit_kernel::forall_schedules_admitted_as_is(2),
            "AS-IS dente: d>=2 would claim forall"
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0070 P2.2: live `run_pcts` at campaign default depth is 2;
    /// 0070 did not raise it. d>2 remains RFC-0051. AS-IS would admit.
    #[test]
    fn pct_runner_default_depth_not_raised() {
        use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};

        let dir = std::env::temp_dir().join(format!("pedra-pct-d2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), StdEnv).unwrap();
        db.set_write_group_catchup_window(std::time::Duration::ZERO);
        let db = Arc::new(db);
        let policy = PiPolicy::pct_campaign_default();
        assert_eq!(policy_pct_depth(policy), 2);
        let report = run_pcts(0x0070_0D22, 2, policy, |task| {
            let db = Arc::clone(&db);
            move |_y: &Yielder| {
                db.put(format!("d70/{task}").as_bytes(), b"v").unwrap();
            }
        });
        assert!(
            !report.steps.is_empty(),
            "run_pcts must grant at least one step"
        );
        assert!(
            !pedradb_core::group_commit_kernel::default_pct_depth_raised(),
            "0070 must not raise default PCT depth"
        );
        assert!(
            pedradb_core::group_commit_kernel::default_pct_depth_raised_as_is(),
            "AS-IS dente: 0070 P2 would claim d>2 is now default"
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0057 P1.2 target: the runner under TSan with **no PCT in the
    /// process** (0052 XOR rule — TSan's OS scheduling and logical π never
    /// nest). Sequential and RoundRobin still drive real threads through
    /// the turnstile and a real `ConcurrentDb`, so a sanitizer build of
    /// this one test exercises the runner's own synchronization (grant
    /// hand-offs, worker install/clear, group-range forensics) plus the
    /// engine publish path those threads drive. Natively it is a plain
    /// progress/replay check: Sequential is a pure function of n (same
    /// grant hash twice) and every put stays visible.
    #[test]
    fn pct_runner_without_pct_replays_and_covers_engine() {
        use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};

        let dir = std::env::temp_dir().join(format!("pedra-pct-nopct-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let opts = OpenOptions {
            sync: false,
            ..OpenOptions::default()
        };
        let db = ConcurrentDb::open_with_env(&dir, opts, StdEnv).unwrap();
        db.set_write_group_catchup_window(std::time::Duration::ZERO);
        let db = Arc::new(db);

        const N: usize = 4;
        const PUTS: usize = 8;

        let run_once = |policy: PiPolicy| {
            let db = Arc::clone(&db);
            run_pcts(0x0057_7EED, N, policy, |task| {
                let db = Arc::clone(&db);
                move |_y: &Yielder| {
                    for i in 0..PUTS {
                        let k = format!("nopct/{task}/{i}");
                        db.put(k.as_bytes(), b"v").unwrap();
                    }
                }
            })
        };

        let seq1 = run_once(PiPolicy::Sequential);
        let seq2 = run_once(PiPolicy::Sequential);
        let rr = run_once(PiPolicy::RoundRobin);

        assert!(!seq1.steps.is_empty());
        assert_eq!(
            seq1.schedule_hash, seq2.schedule_hash,
            "Sequential policy is a pure function of n — replay must match"
        );
        assert!(!rr.steps.is_empty());
        let covered: std::collections::HashSet<_> = rr.steps.iter().map(|s| s.worker).collect();
        assert_eq!(covered.len(), N, "round-robin must grant every worker");
        let total = (0..N)
            .map(|t| {
                (0..PUTS)
                    .filter(|i| db.get(format!("nopct/{t}/{i}").as_bytes()).is_some())
                    .count()
            })
            .sum::<usize>();
        assert_eq!(total, N * PUTS, "every put from all three runs visible");

        Arc::try_unwrap(db)
            .map_err(|_| "workers still hold the db")
            .unwrap()
            .close()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0051 P2.1: Linux trial — the runner over a real `ConcurrentDb`
    /// opened on `FailingEnvArc<IoUringEnv>` (io_uring path; Darwin's
    /// canonical path stays POSIX `StdEnv`). The schedule is a pure
    /// function of (seed, policy, n), so the same seed must produce the
    /// SAME grant hash as the StdEnv run, all writes land, and everything
    /// is durable on reopen. Non-Linux prints an explicit skip (documented
    /// residual — the io_uring path cannot be exercised here).
    #[test]
    fn pct_linux_iouring_env_trial() {
        #[cfg(target_os = "linux")]
        {
            use pedradb_core::{ConcurrentDb, Env, OpenOptions, StdEnv};
            use pedradb_io_uring::IoUringEnv;
            use pedradb_sim::FailingEnvArc;

            let base = std::env::temp_dir().join(format!("pedra-pct-uring-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            let opts = OpenOptions {
                sync: true,
                ..OpenOptions::default()
            };
            let mk_env = || FailingEnvArc::<IoUringEnv>::with_inner_passing(IoUringEnv::default());
            let db = ConcurrentDb::open_with_env(&base, opts.clone(), mk_env()).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = Arc::new(db);
            let std_db =
                Arc::new(ConcurrentDb::open_with_env(&base.join("std"), opts, StdEnv).unwrap());

            const N: usize = 3;
            const PUTS: usize = 8;
            fn body<E: Env>(db: Arc<ConcurrentDb<E>>) -> impl FnOnce(&Yielder) {
                move |_y: &Yielder| {
                    for i in 0..PUTS {
                        let k = format!("uring/{i}");
                        db.put(k.as_bytes(), b"v").unwrap();
                    }
                }
            }
            let r_uring = run_pcts(0x0051_0002, N, PiPolicy::Pct { depth: 2 }, |task| {
                let d = Arc::clone(&db);
                let _ = task;
                body(d)
            });
            let r_std = run_pcts(0x0051_0002, N, PiPolicy::Pct { depth: 2 }, |task| {
                let d = Arc::clone(&std_db);
                let _ = task;
                body(d)
            });
            // same seed ⇒ same schedule as the P0 (StdEnv) run
            assert_eq!(
                r_uring.schedule_hash, r_std.schedule_hash,
                "schedule must not depend on the Env"
            );
            for i in 0..PUTS {
                assert!(db.get(format!("uring/{i}").as_bytes()).is_some());
            }
            drop(r_uring);
            drop(r_std);
            let db = Arc::try_unwrap(db)
                .map_err(|_| "workers still hold the db")
                .unwrap()
                .close()
                .unwrap();
            let _ = db;
            let re = ConcurrentDb::open_with_env(&base, OpenOptions::default(), mk_env())
                .expect("reopen on FailingEnvArc<IoUringEnv>");
            assert!(re.get(b"uring/0").is_some(), "durable after reopen");
            let _ = std::fs::remove_dir_all(&base);
        }
        #[cfg(not(target_os = "linux"))]
        eprintln!(
            "SKIP (RFC-0051 P2.1): FailingEnvArc<IoUringEnv> trial is Linux-only; \
             Darwin/other run the POSIX StdEnv path (see pct_runner_drives_real_concurrentdb)"
        );
    }

    /// RFC-0057 P0: parallel trials in one process must not interfere —
    /// each `run_pcts` owns its turnstile and its forensics
    /// (`group_ranges` is per-run, not a global), so concurrent trials
    /// produce the same schedule hash and the same group forensics as a
    /// serial run of the same seeds.
    #[test]
    fn pct_parallel_trials_no_interference() {
        use pedradb_core::{ConcurrentDb, OpenOptions, StdEnv};

        let base = std::env::temp_dir().join(format!("pedra-pct-par-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        const SEEDS: [u64; 4] = [0xAAAA_0001, 0xAAAA_0002, 0xAAAA_0003, 0xAAAA_0004];
        const N: usize = 3;
        const PUTS: usize = 6;

        let run_one = |seed: u64, slot: u64| -> RunReport {
            let dir = base.join(format!("s{slot}"));
            let _ = std::fs::remove_dir_all(&dir);
            let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), StdEnv).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = Arc::new(db);
            let report = run_pcts(seed, N, PiPolicy::Pct { depth: 2 }, |task| {
                let db = Arc::clone(&db);
                move |_y: &Yielder| {
                    for i in 0..PUTS {
                        let k = format!("par/{task}/{i}");
                        db.put(k.as_bytes(), b"v").unwrap();
                    }
                }
            });
            let closed = Arc::try_unwrap(db)
                .map_err(|_| "workers still hold the db")
                .unwrap()
                .close()
                .unwrap();
            let _ = closed;
            let _ = std::fs::remove_dir_all(&dir);
            report
        };

        // Serial baseline.
        let baseline: Vec<RunReport> = SEEDS
            .iter()
            .enumerate()
            .map(|(i, &s)| run_one(s, i as u64))
            .collect();

        // Same seeds, trials running concurrently on separate threads.
        let parallel: Vec<RunReport> = {
            let mut out: Vec<Option<RunReport>> = vec![None; SEEDS.len()];
            std::thread::scope(|scope| {
                let handles: Vec<_> = SEEDS
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| {
                        scope.spawn(move || {
                            let r = run_one(s, i as u64 + 100);
                            (i, r)
                        })
                    })
                    .collect();
                for h in handles {
                    let (i, r) = h.join().expect("parallel trial panicked");
                    out[i] = Some(r);
                }
            });
            out.into_iter().map(|r| r.expect("slot filled")).collect()
        };

        for (i, (b, p)) in baseline.iter().zip(parallel.iter()).enumerate() {
            assert_eq!(
                b.schedule_hash, p.schedule_hash,
                "seed {} changed schedule under parallel trials",
                SEEDS[i]
            );
            assert_eq!(
                b.group_ranges, p.group_ranges,
                "seed {} group forensics changed under parallel trials",
                SEEDS[i]
            );
        }
        let _ = std::fs::remove_dir_all(&base);
        eprintln!(
            "pct_concurrent: parallel trials: {}/{} seeds schedule+forensics stable",
            SEEDS.len(),
            SEEDS.len()
        );
    }

    // ---------------------------------------------------------------------
    // Planted depth-3 bug (TEST CODE ONLY — never in the engine).
    // RFC-0156 P1.1: three tasks, one window each between check and act.
    // The violation signature is ALL THREE succeeding from balance=100
    // (balance = -200): that requires every check to land before any
    // act, i.e. the third task checking while two are parked — a
    // preemption chain of 3. Two-task overdraw (balance = 0, taken=2)
    // is the depth-2 territory the `Plant` above already covers and is
    // NOT a violation here.
    // ---------------------------------------------------------------------
    struct Plant3 {
        balance: Mutex<i64>,
        taken: std::sync::atomic::AtomicI64,
    }

    impl Plant3 {
        fn take100(&self, y: &Yielder) {
            {
                let b = self.balance.lock().unwrap();
                if *b < 100 {
                    return; // nothing to take
                }
                drop(b);
                y.at("p3_read");
                // act without re-check (the planted bug)
                *self.balance.lock().unwrap() -= 100;
            }
            self.taken
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        fn violation(&self) -> Option<String> {
            let b = *self.balance.lock().unwrap();
            let t = self.taken.load(std::sync::atomic::Ordering::SeqCst);
            (t >= 3 && b <= -200).then(|| format!("triple_take: balance={b} taken={t}"))
        }
    }

    fn plant3_violation(seed: u64, n: usize, policy: PiPolicy) -> Option<String> {
        let plant = Arc::new(Plant3 {
            balance: Mutex::new(100),
            taken: std::sync::atomic::AtomicI64::new(0),
        });
        run_pcts(seed, n, policy, |_task| {
            let plant = Arc::clone(&plant);
            move |y: &Yielder| {
                plant.take100(y);
            }
        });
        plant.violation()
    }

    /// RFC-0156 P1.1 (R-group-glue): the chain-3 signature is found by
    /// PCT d=3 and NOT by d=2 in the measured sweep — evidence that d=3
    /// expresses interleavings d=2 cannot reach (the 0064 d=3 test runs
    /// the depth-2 plant deeper; this one plants a signature only a
    /// chain of 3 can produce). This does not raise the campaign default
    /// (still 2) and `forall_schedules_admitted(3)` stays false: d=3 is
    /// still not ∀ lock interleavings of the OS.
    #[test]
    fn planted_chain3_found_by_pct_d3() {
        const N: usize = 3;
        const SEEDS: u64 = 256;

        // (i) sequential: one task drains (100-100=0), others no-op. Clean.
        let seq_hits = (0..SEEDS)
            .filter(|&s| plant3_violation(s, N, PiPolicy::Sequential).is_some())
            .count();
        assert_eq!(seq_hits, 0, "sequential must be CLEAN on the depth-3 plant");

        // (ii) PCT d=2 sweep: structural — depth 2 has ONE change point,
        // so at most one task is ever demoted below another while parked;
        // the new top then runs to completion (check+act) before the
        // parked task acts. taken <= 2, balance >= -100: the chain-3
        // signature (taken=3, balance=-200) cannot appear. Measured 0.
        let d2_hits = (0..SEEDS)
            .filter(|&s| plant3_violation(s, N, PiPolicy::Pct { depth: 2 }).is_some())
            .count();
        eprintln!("planted_chain3_found_by_pct_d3: d=2 sweep found {d2_hits}/{SEEDS}");

        // (iii) PCT d=3 sweep: two change points at consecutive selection
        // steps demote the top two tasks — the third checks while both
        // are parked, and all three acts land. k = 16n positions makes
        // the alignment rare per seed (~1e-3 with permutation slack), so
        // the sweep is 16384 deterministic seeds, not 256.
        let d3_seeds: u64 = 16384;
        let d3_violators: Vec<(u64, String)> = (0..d3_seeds)
            .filter_map(|s| {
                plant3_violation(s, N, PiPolicy::Pct { depth: 3 }).map(|v| (s, v))
            })
            .collect();
        let d3_hits = d3_violators.len();
        eprintln!("planted_chain3_found_by_pct_d3: d=3 sweep found {d3_hits}/{d3_seeds}");

        // The three teeth: d=2 misses what d=3 finds (when d2_hits == 0),
        // d=3 finds it, and the refusal travels with the campaign.
        assert_eq!(
            d2_hits, 0,
            "PCT d=2 must NOT reach the chain-3 signature (got {d2_hits}/{SEEDS})"
        );
        assert!(
            d3_hits >= 1,
            "PCT d=3 must find the chain-3 signature in 0..16383 (got {d3_hits}/{d3_seeds})"
        );

        // Default stays 2; d=3 is not ∀.
        assert_eq!(
            pedradb_core::group_commit_kernel::pct_campaign_default_depth(),
            2,
            "campaign default PCT depth stays 2"
        );
        assert!(
            !pedradb_core::group_commit_kernel::forall_schedules_admitted(3),
            "d=3 campaign is not forall lock interleavings"
        );
    }
}
