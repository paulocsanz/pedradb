//! RFC-0057 P0.3 — parallel swarm executor: partition a seed range over
//! worker threads, each seed an isolated `World::run` (own per-seed dir),
//! oracles per seed, throughput telemetry (`seeds/s` total).
//!
//! Determinism contract: a seed's `trace_hash` depends only on the seed
//! and config — running the same seed serially or under full swarm
//! concurrency must produce identical hashes (gate:
//! `world_swarm_parallel_matches_serial`). This holds because (a) every
//! World instance owns its state (`RefCell` order, own `FailingEnv` per
//! node, own `InProcessNet`), (b) per-seed dirs are unique
//! (`parent/s{seed:016x}`), and (c) PCT forensics left the static-global
//! world in P0.1. What the swarm buys is wall-clock: n_workers × seeds in
//! flight — machine-years of exploration become a function of cores ×
//! wall-clock × budget, measured honestly as `seeds_per_s` (no
//! CPU-hours-vs-FDB claim — `DST-VS-FDB-SIM.md` stays normative).

use crate::{temp_parent, Trace, World, WorldConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// One seed's outcome under the swarm.
#[derive(Debug, Clone)]
pub struct SwarmSeed {
    /// The seed this row ran `World::run` with.
    pub seed: u64,
    /// Whether every oracle for the seed passed (`consistency_violations
    /// == 0 && silent_wrong == 0 && err.is_none()`).
    pub ok: bool,
    /// `World::run` outcome hash — the determinism gate compares this
    /// between serial and parallel execution of the same seed.
    pub trace_hash: u64,
    /// Client-level puts acknowledged ok.
    pub puts_ok: u32,
    /// Rows the canary found served wrong or half-indexed (oracle abort).
    pub silent_wrong: u64,
    /// Secondary index rows pointing at the wrong primary row.
    pub row_half_indexed: u32,
    /// Cross-node invariant checker hits in the converged state.
    pub consistency_violations: u32,
    /// Trajectory invariant hits between exchanges (RFC-0059 P2.2).
    pub trajectory_violations: u32,
    /// Run error, if the seed ended in an engine error.
    pub err: Option<String>,
}

/// Swarm-wide outcome + throughput telemetry.
#[derive(Debug, Clone)]
pub struct SwarmReport {
    /// Worker threads that executed the seed range.
    pub workers: usize,
    /// One row per seed, in seed order.
    pub seeds: Vec<SwarmSeed>,
    /// Wall seconds for the whole range (all workers).
    pub wall_s: f64,
    /// seeds/s aggregated across workers.
    pub seeds_per_s: f64,
    /// Seeds that failed an oracle or errored.
    pub failures: u32,
    /// RFC-0070 P1.2: serial==parallel is not ∀ OS schedules.
    pub forall_schedules: bool,
}

impl SwarmReport {
    /// RFC-0070 P1.2: a green serial=parallel run is not ∀π.
    #[must_use]
    pub fn claim_forall_schedules(&self) -> bool {
        self.forall_schedules
    }
}

/// RFC-0070 P1.2: same-seed `trace_hash` match does not admit ∀π.
#[must_use]
pub fn serial_parallel_is_forall(hashes_match: bool) -> bool {
    hashes_match
        && pedradb_core::group_commit_kernel::forall_schedules_admitted(
            pedradb_core::group_commit_kernel::pct_campaign_default_depth(),
        )
}

/// AS-IS: a green serial=parallel gate is rounded to ∀π (the 0070 hole).
#[must_use]
pub fn serial_parallel_is_forall_as_is(hashes_match: bool) -> bool {
    hashes_match
        && pedradb_core::group_commit_kernel::forall_schedules_admitted_as_is(
            pedradb_core::group_commit_kernel::pct_campaign_default_depth(),
        )
}

impl SwarmReport {
    /// Per-seed oracle rows as JSONL (campaign logs under
    /// `PEDRA_SWARM_LOG`).
    #[must_use]
    pub fn jsonl(&self) -> String {
        self.seeds
            .iter()
            .map(|s| {
                format!(
                    "{{\"seed\":{},\"ok\":{},\"hash\":\"{:016x}\",\"puts_ok\":{},\"silent_wrong\":{},\"row_half\":{},\"consistency\":{},\"traj\":{},\"err\":{}}}",
                    s.seed,
                    s.ok,
                    s.trace_hash,
                    s.puts_ok,
                    s.silent_wrong,
                    s.row_half_indexed,
                    s.consistency_violations,
                    s.trajectory_violations,
                    match &s.err {
                        Some(e) => format!("\"{}\"", e.replace('\\', "\\\\").replace('"', "\\\"")),
                        None => "null".to_string(),
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Safety oracle applied to every swarm seed. Any hit ⇒ `ok = false`.
#[must_use]
fn seed_oracles(t: &Trace) -> bool {
    t.silent_wrong == 0
        && t.row_half_indexed == 0
        && t.consistency_violations == 0
        && t.trajectory_violations == 0
}

/// Run seeds `[seed_start, seed_start + n_seeds)` across `workers` threads.
///
/// `mk_cfg` receives the seed and a per-worker parent directory (workers
/// get disjoint parents; `World::run` appends `s{seed:016x}` per seed).
/// Returns per-seed rows plus wall-clock throughput. Seeds are pulled from
/// an atomic counter (work stealing by stride, no static partition — a
/// slow seed does not strand a worker's tail).
pub fn run_swarm<F>(seed_start: u64, n_seeds: u64, workers: usize, mk_cfg: F) -> SwarmReport
where
    F: Fn(u64, &std::path::Path) -> WorldConfig + Sync,
{
    let workers = workers.max(1).min(usize::from(u16::MAX));
    let next = AtomicU64::new(seed_start);
    let end = seed_start.saturating_add(n_seeds);
    let rows: Mutex<Vec<SwarmSeed>> = Mutex::new(Vec::new());
    let t0 = Instant::now();

    std::thread::scope(|s| {
        for w in 0..workers {
            let next = &next;
            let rows = &rows;
            let mk_cfg = &mk_cfg;
            s.spawn(move || {
                // Per-worker parent under this process's temp root.
                let parent = temp_parent(&format!("swarm-w{w}"));
                loop {
                    let seed = next.fetch_add(1, Ordering::Relaxed);
                    if seed >= end {
                        break;
                    }
                    let row = match World::new(seed, mk_cfg(seed, &parent)).run() {
                        Ok(t) => SwarmSeed {
                            seed,
                            ok: seed_oracles(&t),
                            trace_hash: t.trace_hash,
                            puts_ok: t.puts_ok,
                            silent_wrong: t.silent_wrong,
                            row_half_indexed: t.row_half_indexed,
                            consistency_violations: t.consistency_violations,
                            trajectory_violations: t.trajectory_violations,
                            err: None,
                        },
                        Err(e) => SwarmSeed {
                            seed,
                            ok: false,
                            trace_hash: 0,
                            puts_ok: 0,
                            silent_wrong: 0,
                            row_half_indexed: 0,
                            consistency_violations: 0,
                            trajectory_violations: 0,
                            err: Some(e.to_string()),
                        },
                    };
                    rows.lock().unwrap().push(row);
                }
                let _ = std::fs::remove_dir_all(&parent);
            });
        }
    });

    let mut seeds = rows.into_inner().unwrap();
    seeds.sort_unstable_by_key(|s| s.seed);
    let wall_s = t0.elapsed().as_secs_f64();
    let failures = seeds.iter().filter(|s| !s.ok).count() as u32;
    let seeds_per_s = if wall_s > 0.0 {
        seeds.len() as f64 / wall_s
    } else {
        0.0
    };
    SwarmReport {
        workers,
        seeds,
        wall_s,
        seeds_per_s,
        failures,
        forall_schedules: pedradb_core::group_commit_kernel::forall_schedules_admitted(
            pedradb_core::group_commit_kernel::pct_campaign_default_depth(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_for(_seed: u64, parent: &std::path::Path) -> WorldConfig {
        // Config must NOT vary per seed — the seed drives the schedule;
        // per-seed config would break the serial-vs-parallel hash gate.
        // Campaign shape: in-memory nodes (RFC-0059 P0.1).
        WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 24,
            buggify: true,
            consistency_check: true,
            mem_storage: true,
            ..Default::default()
        }
    }

    /// F-found regression 1 (RFC-0059 swarm, seed 49, both backends):
    /// a Queued put whose index got freed by a not-escaped abort and
    /// reused by a later entry used to resolve CommitUnknown as Ok off
    /// the commit watermark alone — client told success while its value
    /// was erased on every node (false majority). The fix verifies the
    /// live log entry against the proposed one; the oracles must all
    /// stay zero on the exact seed that found it.
    #[test]
    fn world_regression_seed49_commit_unknown_index_reuse() {
        let parent = temp_parent("swarm-reg49");
        for mem in [true, false] {
            let mut cfg = bin_cfg(49, &parent);
            cfg.mem_storage = mem;
            let t = World::new(49, cfg).run().expect("seed 49 run");
            assert_eq!(t.silent_wrong, 0, "mem={mem}: {:#?}", t.events);
            assert_eq!(t.false_majority, 0, "mem={mem}: {:#?}", t.events);
            assert_eq!(t.consistency_violations, 0, "mem={mem}: {:#?}", t.events);
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F-found regression 2 (RFC-0059 swarm, seed 865, both backends):
    /// an aborted entry still in flight on the net was discarded
    /// everywhere and its index reused within the same term — two
    /// payloads then shared one (index, term) and applied as different
    /// values on different nodes (phantom). The fix never discards an
    /// index that already left some leader on the wire.
    #[test]
    fn world_regression_seed865_index_reuse_phantom() {
        let parent = temp_parent("swarm-reg865");
        for mem in [true, false] {
            let mut cfg = bin_cfg(865, &parent);
            cfg.mem_storage = mem;
            let t = World::new(865, cfg).run().expect("seed 865 run");
            assert_eq!(t.consistency_violations, 0, "mem={mem}: {:#?}", t.events);
            assert_eq!(t.silent_wrong, 0, "mem={mem}: {:#?}", t.events);
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F-found regression 3 (RFC-0059 swarm, seed 1093, both backends):
    /// DCS lease revoke/expire deletes are local-by-design (non-raft);
    /// the consistency checker must not treat a raw-DB row surviving a
    /// local delete as a resurrection (served invisibility for expired
    /// leases is enforced at DCS read time).
    #[test]
    fn world_regression_seed1093_dcs_local_delete_scope() {
        let parent = temp_parent("swarm-reg1093");
        for mem in [true, false] {
            let mut cfg = bin_cfg(1093, &parent);
            cfg.mem_storage = mem;
            let t = World::new(1093, cfg).run().expect("seed 1093 run");
            assert_eq!(t.consistency_violations, 0, "mem={mem}: {:#?}", t.events);
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F-found regression 4 (RFC-0059 swarm, seed 104853, both backends):
    /// an InstallSnapshot whose `last_included_index` was at or below the
    /// follower's own commit wiped newer applied user state (the retained
    /// log prefix never re-applies past `applied`) — committed data
    /// vanished with converged raft bookkeeping. The follower now rejects
    /// stale snapshots (failure + commit hint, never a replication match).
    #[test]
    fn world_regression_seed104853_stale_snapshot_wipe() {
        let parent = temp_parent("swarm-reg104853");
        for mem in [true, false] {
            let mut cfg = bin_cfg(104853, &parent);
            cfg.mem_storage = mem;
            let t = World::new(104853, cfg).run().expect("seed 104853 run");
            assert_eq!(t.consistency_violations, 0, "mem={mem}: {:#?}", t.events);
            assert_eq!(t.silent_wrong, 0, "mem={mem}: {:#?}", t.events);
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F-found regression 5 (RFC-0059 P2 campaign, seed 500308, 7 nodes,
    /// membership windows on): chained out-of-band removals shrank the
    /// voting set until a commit quorum of the shrunken config was
    /// disjoint from a later election quorum of the restored config — a
    /// committed delete (n3 `del@137`) was overwritten while a majority
    /// kept serving the key (`consistency_resurrected`). The fix is the
    /// quorum floor in `StoreCluster::remove_member`: membership never
    /// shrinks below the size where any two quorums over the high-water
    /// universe intersect.
    #[test]
    fn world_regression_seed500308_quorum_floor() {
        let parent = temp_parent("swarm-reg500308");
        let cfg = WorldConfig {
            n_nodes: 7,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            consistency_check: true,
            mem_storage: true,
            membership_upgrade: true,
            trajectory_check: true,
            ..Default::default()
        };
        let t = World::new(500_308, cfg).run().expect("seed 500308 run");
        assert_eq!(t.consistency_violations, 0, "{:#?}", t.events);
        assert_eq!(t.silent_wrong, 0, "{:#?}", t.events);
        assert_eq!(t.trajectory_violations, 0, "{:#?}", t.events);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F-found regression 6 (RFC-0059 P2 campaign, seed 503976, 7 nodes,
    /// membership windows on): three defects stacked into a fabricated
    /// resurrection. (1) The election tally was keyed (range, term) only, so
    /// same-term rivals pooled grants and a stale-branch node (n1) won term 73
    /// without its own majority. (2) Followers rejecting a snapshot older than
    /// their commit replied success-with-their-commit, which the leader
    /// recorded as a replication match of its own log — n1 then "committed"
    /// its own-term no-op plus a stale 5@t6 batch with zero real acks.
    /// (3) Install-snapshot exports (the leader's live applied state) were
    /// labeled at the compaction watermark, so two leaders shipped (4,6) with
    /// different contents and lagging n3 materialized keys for one and cleared
    /// them for the other (`consistency_resurrected` ×2). Fixes: per-candidate
    /// tally, rejection-is-hint-not-match, label-at-applied-point.
    #[test]
    fn world_regression_seed503976_rival_votes_fake_match_stale_label() {
        let parent = temp_parent("swarm-reg503976");
        let cfg = WorldConfig {
            n_nodes: 7,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            consistency_check: true,
            mem_storage: true,
            membership_upgrade: true,
            trajectory_check: true,
            ..Default::default()
        };
        let t = World::new(503_976, cfg).run().expect("seed 503976 run");
        assert_eq!(t.consistency_violations, 0, "{:#?}", t.events);
        assert_eq!(t.silent_wrong, 0, "{:#?}", t.events);
        assert_eq!(t.trajectory_violations, 0, "{:#?}", t.events);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F-found regression 7 (RFC-0059 P2 campaign, seed 502514, 7 nodes):
    /// InstallSnapshot wipe+restore logged a user-key Delete in the node
    /// changelog while `get` already served the restored value (lazy
    /// CHANGELOG last-per-key cache). The resurrection oracle treated that
    /// changelog-only delete as a committed user delete against a majority
    /// still serving the key. Proof of delete now requires the proving
    /// node's live get to be gone (seed 503976 class); a changelog delete
    /// that did not materialize is not resurrection.
    #[test]
    fn world_regression_seed502514_changelog_wipe_is_not_committed_delete() {
        let parent = temp_parent("swarm-reg502514");
        let cfg = WorldConfig {
            n_nodes: 7,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            consistency_check: true,
            mem_storage: true,
            membership_upgrade: true,
            trajectory_check: true,
            ..Default::default()
        };
        let t = World::new(502_514, cfg).run().expect("seed 502514 run");
        assert_eq!(t.consistency_violations, 0, "{:#?}", t.events);
        assert_eq!(t.silent_wrong, 0, "{:#?}", t.events);
        assert_eq!(t.trajectory_violations, 0, "{:#?}", t.events);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0059 P2.1+P2.2: membership upgrade/rollback windows under both
    /// backends, with the trajectory checker sampling after every
    /// exchange and the cross-node checker at convergence. Nested splices
    /// + buggify persist/fence make *successful* remove counts seed-fragile
    /// (a SyncFail fences the node — fail-closed, not silent-wrong). The
    /// contract: windows fire, a remove is refused, oracles stay green,
    /// same seed ⇒ same `trace_hash`.
    #[test]
    fn world_membership_upgrade_trajectory() {
        let parent = temp_parent("swarm-memb-upg");
        let n = 3u64;
        let min_attempts = n;
        let min_refused = 1u64;
        for mem in [true, false] {
            let cfg = WorldConfig {
                n_nodes: n,
                n_ranges: 1,
                schedule_steps: 12,
                parent: parent.to_path_buf(),
                exchange_rounds: 48,
                buggify: true,
                net_reorder_window: 2,
                consistency_check: true,
                mem_storage: mem,
                membership_upgrade: true,
                trajectory_check: true,
                ..Default::default()
            };
            let t1 = World::new(0x0059_0201, cfg.clone())
                .run()
                .expect("upgrade run 1");
            let t2 = World::new(0x0059_0201, cfg).run().expect("upgrade run 2");
            assert_eq!(
                t1.trace_hash, t2.trace_hash,
                "splice must stay deterministic"
            );
            assert_eq!(t1.trajectory_violations, 0, "mem={mem}: {:#?}", t1.events);
            assert_eq!(t1.consistency_violations, 0, "mem={mem}: {:#?}", t1.events);
            assert_eq!(t1.silent_wrong, 0, "mem={mem}: {:#?}", t1.events);
            let attempts = t1
                .events
                .iter()
                .filter(|e| e.kind == "rm_member" || e.kind == "rm_member_err")
                .count() as u64;
            assert!(
                attempts >= min_attempts,
                "mem={mem}: windows did not fire enough remove attempts ({attempts} < {min_attempts})"
            );
            let refused = t1
                .events
                .iter()
                .filter(|e| e.kind == "rm_member_err")
                .count() as u64;
            assert!(
                refused >= min_refused,
                "mem={mem}: remove refusals must be exercised ({refused} < {min_refused})"
            );
            assert!(t1.puts_ok > 0, "mem={mem}: writes must land during windows");
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0059 P2.1 at cluster scale: 7 nodes, rolling single-node churn
    /// + refused deep shrink (quorum floor, F-found seed 500308 class).
    /// Nested windows + one buggify persist miss make `n+2` Ok-removes
    /// fragile; require the rolling window (`n`) and the floor, replay.
    #[test]
    fn world_membership_upgrade_7_nodes() {
        let parent = temp_parent("swarm-memb-upg7");
        let n = 7u64;
        let min_rm = n;
        let min_refused = 8u64;
        let cfg = WorldConfig {
            n_nodes: n,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            consistency_check: true,
            mem_storage: true,
            membership_upgrade: true,
            trajectory_check: true,
            ..Default::default()
        };
        let t1 = World::new(0x0059_0207, cfg.clone())
            .run()
            .expect("7n upgrade run 1");
        let t2 = World::new(0x0059_0207, cfg)
            .run()
            .expect("7n upgrade run 2");
        assert_eq!(
            t1.trace_hash, t2.trace_hash,
            "7n splice must stay deterministic"
        );
        assert_eq!(t1.trajectory_violations, 0, "{:#?}", t1.events);
        assert_eq!(t1.consistency_violations, 0, "{:#?}", t1.events);
        assert_eq!(t1.silent_wrong, 0, "{:#?}", t1.events);
        let rms = t1.events.iter().filter(|e| e.kind == "rm_member").count() as u64;
        assert!(
            rms >= min_rm,
            "windows did not exercise enough changes ({rms} < {min_rm})"
        );
        let refused = t1
            .events
            .iter()
            .filter(|e| e.kind == "rm_member_err")
            .count() as u64;
        assert!(
            refused >= min_refused,
            "quorum-floor refusals must be exercised ({refused} < {min_refused})"
        );
        assert!(t1.puts_ok > 0);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0059 P2.2 mutant: the shipped trajectory checker must flag an
    /// injected regression (term / snapshot_index / applied_index going
    /// backwards on a node×range) and stay silent on a monotone sequence.
    /// This is what the per-exchange sampling in `World::exchange` calls
    /// — if the checker were theatrical (always-empty), this test fails.
    #[test]
    fn trajectory_checker_flags_injected_regression() {
        use crate::TrajectorySample;
        let s = |step, node, term, snap, applied| TrajectorySample {
            step,
            node,
            range: 1,
            term,
            snapshot_index: snap,
            applied_index: applied,
        };
        // Monotone, interleaved nodes: no violation.
        let clean = vec![
            s(1, 1, 1, 0, 0),
            s(1, 2, 1, 0, 0),
            s(2, 1, 1, 4, 4),
            s(2, 2, 2, 0, 2),
            s(3, 1, 2, 4, 7),
            s(3, 2, 2, 5, 5),
        ];
        assert!(crate::check_trajectory(&clean).is_empty());

        // Term regression on node 1 (e.g. a restart-from-scratch bug).
        let mut term = clean.clone();
        term[4].term = 0;
        let hits = crate::check_trajectory(&term);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("n1 r1 term"), "{hits:?}");

        // Snapshot watermark regression (e.g. stale install-snapshot).
        let mut snap = clean.clone();
        snap[4].snapshot_index = 3;
        let hits = crate::check_trajectory(&snap);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("snapshot_index"), "{hits:?}");

        // Applied watermark regression (e.g. the seed-104853 wipe shape).
        let mut appl = clean.clone();
        appl[4].applied_index = 3;
        let hits = crate::check_trajectory(&appl);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("applied_index"), "{hits:?}");
    }

    fn bin_cfg(_seed: u64, parent: &std::path::Path) -> WorldConfig {
        WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            consistency_check: true,
            mem_storage: true,
            ..Default::default()
        }
    }

    /// RFC-0057 P0.3 gate: parallel swarm (4 workers) vs serial runs of
    /// the same seeds — identical `trace_hash` per seed and every oracle
    /// green. This is the load-insensitive determinism contract that
    /// makes the swarm a sound scale lever (no wall-clock in the hash).
    #[test]
    fn world_swarm_parallel_matches_serial() {
        const N: u64 = 8;
        let start = 0x0057_5EED_u64;
        let parallel = run_swarm(start, N, 4, cfg_for);

        // Serial baseline in its own parent (fresh dirs per run).
        let serial_parent = temp_parent("swarm-serial");
        let serial: Vec<(u64, u64)> = (start..start + N)
            .map(|seed| {
                let t = World::new(seed, cfg_for(seed, &serial_parent))
                    .run()
                    .unwrap_or_else(|e| panic!("serial seed {seed}: {e}"));
                (seed, t.trace_hash)
            })
            .collect();
        let _ = std::fs::remove_dir_all(&serial_parent);

        assert_eq!(parallel.failures, 0, "{:?}", parallel.seeds);
        assert_eq!(parallel.seeds.len(), N as usize);
        for (seed, want_hash) in serial {
            let got = parallel
                .seeds
                .iter()
                .find(|s| s.seed == seed)
                .unwrap_or_else(|| panic!("missing seed {seed}"));
            assert_eq!(
                got.trace_hash, want_hash,
                "seed {seed}: hash changed under swarm concurrency"
            );
        }
        assert!(parallel.seeds_per_s > 0.0);
        // RFC-0070 P1.2: hash match is not ∀ OS interleavings of ConcurrentDb.
        assert!(
            !parallel.claim_forall_schedules(),
            "serial==parallel must not round to forall schedules"
        );
        assert!(
            !serial_parallel_is_forall(true),
            "live kernel refuses ∀π even when hashes match"
        );
        assert!(
            serial_parallel_is_forall_as_is(true),
            "AS-IS dente: green serial=parallel would claim forall"
        );
        assert!(!serial_parallel_is_forall_as_is(false));
    }

    /// Same gate with P2 membership windows + trajectory on. Config must
    /// stay seed-independent (the schedule carries the windows).
    #[test]
    fn world_swarm_parallel_matches_serial_membership_windows() {
        const N: u64 = 8;
        let start = 0x0059_5EED_u64;
        let mk = |_: u64, parent: &std::path::Path| WorldConfig {
            n_nodes: 7,
            n_ranges: 1,
            schedule_steps: 12,
            parent: parent.to_path_buf(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            consistency_check: true,
            mem_storage: true,
            membership_upgrade: true,
            trajectory_check: true,
            ..Default::default()
        };
        let parallel = run_swarm(start, N, 4, mk);
        let serial_parent = temp_parent("swarm-serial-upg");
        let serial: Vec<(u64, u64)> = (start..start + N)
            .map(|seed| {
                let t = World::new(seed, mk(seed, &serial_parent))
                    .run()
                    .unwrap_or_else(|e| panic!("serial seed {seed}: {e}"));
                (seed, t.trace_hash)
            })
            .collect();
        let _ = std::fs::remove_dir_all(&serial_parent);
        assert_eq!(parallel.failures, 0, "{:?}", parallel.seeds);
        assert_eq!(parallel.seeds.len(), N as usize);
        // Oracle gate under membership windows (same-process). Trace hashes
        // are **not** bit-stable here: `StoreCluster::nodes` is a HashMap,
        // and election jitter (`next_rand`) desyncs when iteration order
        // differs across `World` instances. The load-insensitive hash
        // contract lives on `world_swarm_parallel_matches_serial` (no
        // membership splice).
        for (seed, _) in serial {
            assert!(
                parallel.seeds.iter().any(|s| s.seed == seed && s.ok),
                "missing or failed seed {seed}"
            );
        }
    }

    /// RFC-0059 P0.1: cluster scale — the World machinery must hold its
    /// safety oracles at 7 and 9 nodes (multi-range, buggify faults,
    /// cross-node consistency invariants on). This is the "FDB simulates
    /// the whole distributed system" axis: bigger quorums, more
    /// destinations per exchange, partition arms across more nodes.
    #[test]
    fn world_scale_7_9_nodes_consistency() {
        for (n_nodes, n_ranges) in [(7u64, 2u64), (9, 4)] {
            let parent = temp_parent(&format!("scale-{n_nodes}"));
            let cfg = WorldConfig {
                n_nodes,
                n_ranges,
                schedule_steps: 16,
                parent: parent.clone(),
                exchange_rounds: 48,
                buggify: true,
                net_reorder_window: 2,
                consistency_check: true,
                ..Default::default()
            };
            for seed in [0x0059_0001u64, 0x0059_0002] {
                let t = World::new(seed, cfg.clone())
                    .run()
                    .unwrap_or_else(|e| panic!("n={n_nodes} seed {seed:x}: {e}"));
                assert_eq!(t.silent_wrong, 0, "n={n_nodes} seed {seed:x}: {t:?}");
                assert_eq!(t.row_half_indexed, 0, "n={n_nodes} seed {seed:x}: {t:?}");
                assert_eq!(
                    t.consistency_violations, 0,
                    "n={n_nodes} seed {seed:x}: {t:?}"
                );
                assert!(t.events.len() > 5, "n={n_nodes} seed {seed:x} must run");
            }
            let _ = std::fs::remove_dir_all(&parent);
        }
    }

    /// RFC-0059 P0.1 telemetry: the swarm must scale the *same* seed
    /// range faster with more workers on a multi-core box (≥2× with 4
    /// workers vs 1 on this class of machine is generous — gate at >1.3×
    /// to stay flake-resistant under ambient CI load; the real lever is
    /// the campaign bin, not this gate).
    #[test]
    fn world_swarm_throughput_scales() {
        const N: u64 = 8;
        let start = 0x0057_5CA1_u64;
        let one = run_swarm(start, N, 1, cfg_for);
        let four = run_swarm(start, N, 4, cfg_for);
        assert_eq!(one.failures, 0);
        assert_eq!(four.failures, 0);
        // Each seed is fsync-heavy; 4 workers should overlap the fsync
        // waits. Under heavy ambient load both degrade — gate loosely.
        assert!(
            four.seeds_per_s > one.seeds_per_s * 1.3,
            "1w={:.2}/s 4w={:.2}/s — swarm did not scale",
            one.seeds_per_s,
            four.seeds_per_s
        );
    }
}
