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
    let ts = Arc::new(Turnstile::new(n));
    let mut joins = Vec::with_capacity(n);
    for task in 0..n {
        let ts = Arc::clone(&ts);
        let f = mk(task);
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
    let k = 16 * n; // generous step bound for change-point placement
    let mut sched = policy_scheduler(seed, n, k, policy);
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
        steps,
        schedule_hash,
        group_ranges,
    }
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
            .filter_map(|s| {
                plant_violation(s, N, OPS, PiPolicy::Pct { depth: 2 })
                    .map(|v| (s, v))
            })
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
            assert_eq!(r.schedule_hash, baseline.schedule_hash, "replay must be bit-stable");
            assert_eq!(
                plant_violation(vs, N, OPS, PiPolicy::Pct { depth: 2 }),
                Some(vv.clone()),
                "same seed must reproduce the same violation"
            );
        }

        // Telemetry: hit rate over the band (evidence, not a product claim).
        eprintln!("pct_concurrent: planted d=2 hits {pct_hits}/256 (first seed {vs}: {vv})");
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
                    assert!(re.get(k.as_bytes()).is_some(), "silent wrong: ok commit {k} vanished on reopen");
                }
            }
            drop(re);
            let _ = std::fs::remove_dir_all(&dir);
            (members, oks, refused, other, tripped, report.schedule_hash, got)
        };

        // (i) grosso: run-to-completion (no preemption anywhere, incl. the
        // off-lock fsync) — same workload, same arm point: 0 hits in 0..255.
        let seq_hits = (0..SEEDS)
            .filter(|&s| {
                let (members, oks, refused, other, tripped, _h, got) =
                    trial("seq", s, PiPolicy::Sequential);
                assert_eq!(other, 0, "sequential trial saw an unexpected error class: {got:?}");
                assert!(tripped, "SyncFail must fire in every sequential trial");
                assert_eq!(members + refused + oks, N * COMMITS);
                members >= 2
            })
            .count();
        assert_eq!(seq_hits, 0, "run-to-completion must miss the multi-member fence");

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
            assert_eq!((members, oks, refused), (vm, base_got.iter().filter(|(_, t)| *t == "ok").count(), base_got.iter().filter(|(_, t)| *t == "refused").count()));
            assert_eq!(got, base_got, "same seed must reproduce the same outcomes");
            assert_eq!(h, baseline, "replay must be bit-stable");
        }

        let _ = std::fs::remove_dir_all(&base);
        eprintln!(
            "pct_concurrent: pi x disk fence dente: seq {seq_hits}/256 (miss), pct {pct_hits}/256 (first seed {vs}: {vm} members fenced at the off-lock EIO)"
        );
    }

    /// P0.2 (RFC-0058): PCT over the **verified profile** — same π×disk
    /// teeth as `pct_disk_fence_three_teeth`, but the DB is pinned
    /// lone-commit-only, with all three write shapes in the mix: plain
    /// sync put, OCC tx, and `no_sync` async put. No matter how π
    /// preempts: no writer ever joins a group (`queued == 0`,
    /// `batches == submits`), the one-shot EIO fences **at most one**
    /// writer (the full mode proves 2+ members can share a fence — that
    /// shape is structurally absent here), outcomes are only ok / fenced /
    /// refused, every sync-Ok commit survives the reopen (`silent_wrong =
    /// 0`), and async survivors are never wrong (async Ok promises no
    /// durability before a barrier — the DB is dropped, not closed).
    #[test]
    fn pct_verified_lone_never_merges() {
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
            (fenced, oks, async_oks, refused, other, tripped, stats, report.schedule_hash)
        };

        for policy in [PiPolicy::Sequential, PiPolicy::Pct { depth: 2 }] {
            let mut fenced_total = 0usize;
            for s in 0..SEEDS {
                let (fenced, oks, async_oks, refused, other, tripped, stats, _h) =
                    trial(if matches!(policy, PiPolicy::Sequential) { "seq" } else { "pct" }, s, policy);
                assert_eq!(other, 0, "unexpected error class in verified trial {s}");
                assert!(tripped, "SyncFail must fire in every verified trial {s}");
                assert_eq!(fenced + refused + oks + async_oks, N * COMMITS);
                assert!(fenced <= 1, "lone-only pin yet {fenced} writers shared one fence");
                let (submits, queued, batches, batch_ops) = stats;
                assert_eq!(submits, (N * COMMITS) as u64);
                assert_eq!(queued, 0, "verified mode must never merge writers (trial {s})");
                assert_eq!(batches, submits, "every commit its own batch (trial {s})");
                assert_eq!(batch_ops, (N * COMMITS) as u64);
                fenced_total += fenced;
            }
            assert!(fenced_total >= 1, "the EIO must fence someone across {SEEDS} seeds");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// RFC-0058 P1.3: semantics derivation — the verified profile must be a
    /// **scheduling-only** transformation of the full mode. Same seeds, same
    /// PCT schedules, same three write shapes (sync put / OCC tx / async
    /// `no_sync` put), same one-shot EIO on the 6th group fsync. The safety
    /// oracles are **identical** in both modes: every sync/OCC Ok survives
    /// the reopen (`silent_wrong == 0`), every async survivor holds the
    /// right value, the reopen invents nothing (ghost-free, value-exact
    /// scan over the written range), and the only error classes are
    /// fence/refused. Differences are allowed **only** in scheduling and
    /// performance: the full mode (catchup window ZERO, group commit
    /// active) merges writers under PCT preemption — asserted to actually
    /// happen, else the pairing is vacuous — and one EIO may fence 2+
    /// members of one group; the verified mode never merges (`queued == 0`,
    /// `batches == submits`) and fences at most one writer per trial.
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
                .filter(|(k, v)| {
                    !expected.contains(k.as_ref()) || v.as_ref() != &b"v"[..]
                })
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
            for s in 0..SEEDS {
                let full = trial(&format!("{tag}-full"), s, policy, false);
                let ver = trial(&format!("{tag}-ver"), s, policy, true);

                // (a) identical safety oracles in both modes.
                for (mode, r) in [("full", &full), ("verified", &ver)] {
                    let (oks, async_oks, fenced, refused, other, silent_wrong, wrong_async, ghost, tripped, _stats) =
                        *r;
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

                // (b) differences allowed only in scheduling/performance.
                let (_, _, ver_fenced, _, _, _, _, _, ver_tripped, ver_stats) = ver;
                let (submits, queued, batches, batch_ops) = ver_stats;
                assert_eq!(submits, (N * COMMITS) as u64, "verified trial {s}");
                assert_eq!(queued, 0, "verified trial {s} merged writers");
                assert_eq!(batches, submits, "verified trial {s}: commit not lone");
                assert_eq!(batch_ops, (N * COMMITS) as u64, "verified trial {s}");
                assert!(
                    ver_tripped,
                    "verified trial {s}: SyncFail must fire (6 lone fsyncs > {AFTER_FS})"
                );
                assert!(
                    ver_fenced <= 1,
                    "verified trial {s}: lone-only yet {ver_fenced} writers shared one fence"
                );
                // Full mode: merging is *allowed* (the documented shape) —
                // and must actually happen under PCT preemption somewhere.
                let (_, _, _, _, _, _, _, _, _, full_stats) = full;
                full_queued_total += full_stats.1;
            }
            if matches!(policy, PiPolicy::Pct { .. }) {
                assert!(
                    full_queued_total > 0,
                    "{tag}: full mode never merged across {SEEDS} seeds — vacuous comparison"
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
            let recs: std::sync::Arc<Mutex<Vec<Rec>>> =
                std::sync::Arc::new(Mutex::new(Vec::new()));
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
                ranges.iter().any(|(lo, hi)| *lo <= a && a <= *hi && *lo <= b && b <= *hi)
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
                    for &w in writer_seqs
                        .iter()
                        .filter(|w| r.snapshot < **w && **w < seq)
                    {
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
            .filter(|&s| trial(&base, "seq", s, PiPolicy::Sequential, true).0.is_some())
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
        let re = ConcurrentDb::open_with_env(&dir, opts, StdEnv)
            .expect("reopen after close failed");
        assert!(re.get(b"pct/0/0").is_some(), "durable after reopen");
        assert!(re.get(b"pct/2/11").is_some());
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
            let mk_env =
                || FailingEnvArc::<IoUringEnv>::with_inner_passing(IoUringEnv::default());
            let db = ConcurrentDb::open_with_env(&base, opts.clone(), mk_env()).unwrap();
            db.set_write_group_catchup_window(std::time::Duration::ZERO);
            let db = Arc::new(db);
            let std_db = Arc::new(
                ConcurrentDb::open_with_env(&base.join("std"), opts, StdEnv).unwrap(),
            );

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
}
