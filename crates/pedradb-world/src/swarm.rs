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
                    "{{\"seed\":{},\"ok\":{},\"hash\":\"{:016x}\",\"puts_ok\":{},\"silent_wrong\":{},\"row_half\":{},\"consistency\":{},\"err\":{}}}",
                    s.seed,
                    s.ok,
                    s.trace_hash,
                    s.puts_ok,
                    s.silent_wrong,
                    s.row_half_indexed,
                    s.consistency_violations,
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
    t.silent_wrong == 0 && t.row_half_indexed == 0 && t.consistency_violations == 0
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
    /// stale snapshots and replies success at its own commit.
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
