//! RFC-0057 P0.3 campaign bin — parallel World swarm at hardware scale.
//!
//! ```text
//! cargo run --release -p pedradb-world --bin world_swarm -- [n_seeds] [start_seed] [workers] [n_nodes] [steps]
//! ```
//!
//! Env: `PEDRA_SWARM_LOG=dir` writes per-seed JSONL + a summary line.
//! Exit 1 if any seed fails an oracle. Throughput (`seeds/s`) is the
//! honest scale metric — machine-years of exploration are cores ×
//! wall-clock × budget; no CPU-hours-vs-FDB claim.

use pedradb_world::swarm::run_swarm;
use pedradb_world::WorldConfig;

fn main() {
    let arg =
        |i: usize, d: &str| -> String { std::env::args().nth(i).unwrap_or_else(|| d.to_string()) };
    let n_seeds: u64 = arg(1, "256").parse().unwrap_or(256);
    let start_seed: u64 = arg(2, "1").parse().unwrap_or(1);
    let workers: usize = arg(3, "0").parse().unwrap_or(0);
    let n_nodes: u64 = arg(4, "3").parse().unwrap_or(3);
    let steps: usize = arg(5, "16").parse().unwrap_or(16);
    let n_ranges: u64 = std::env::var("PEDRA_SWARM_RANGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let buggify = std::env::var("PEDRA_SWARM_BUGGIFY")
        .map(|v| v != "0")
        .unwrap_or(true);
    let consistency = std::env::var("PEDRA_SWARM_CONSISTENCY")
        .map(|v| v != "0")
        .unwrap_or(true);
    // RFC-0059 P0.1: campaign default = in-memory node storage (same
    // engine paths and fault seams, no host-I/O serialization).
    // PEDRA_SWARM_DISK=1 keeps the real-FS backend.
    let mem_storage = std::env::var("PEDRA_SWARM_DISK")
        .map(|v| v == "0")
        .unwrap_or(true);
    // RFC-0059 P2: membership upgrade/rollback windows + trajectory
    // invariants in the campaign (default off keeps the base stream).
    let membership_upgrade = std::env::var("PEDRA_SWARM_UPGRADE")
        .map(|v| v != "0")
        .unwrap_or(false);
    let trajectory_check = std::env::var("PEDRA_SWARM_TRAJECTORY")
        .map(|v| v != "0")
        .unwrap_or(false);
    // Single-seed diagnostic mode: run one seed inline with the campaign
    // config and print the event trace (consistency/silent-wrong lines
    // included). Exit 1 if the seed fails an oracle.
    if let Ok(dump_seed) = std::env::var("PEDRA_SWARM_DUMP") {
        let dump_seed: u64 = dump_seed.parse().expect("PEDRA_SWARM_DUMP=<u64 seed>");
        let parent = std::env::temp_dir().join(format!("world-swarm-dump-{dump_seed:016x}"));
        let _ = std::fs::remove_dir_all(&parent);
        let cfg = WorldConfig {
            n_nodes,
            n_ranges,
            schedule_steps: steps,
            parent: parent.clone(),
            exchange_rounds: 48,
            buggify,
            net_reorder_window: if buggify { 2 } else { 0 },
            consistency_check: consistency,
            mem_storage,
            membership_upgrade,
            trajectory_check,
            ..Default::default()
        };
        let t = pedradb_world::World::new(dump_seed, cfg)
            .run()
            .unwrap_or_else(|e| panic!("seed {dump_seed}: {e}"));
        let full_trace = std::env::var("PEDRA_SWARM_TRACE")
            .map(|v| v != "0")
            .unwrap_or(false);
        for ev in &t.events {
            if full_trace
                || ev.kind.contains("consistency")
                || ev.kind.contains("silent")
                || ev.kind.contains("half")
                || ev.kind.contains("false_majority")
                || ev.kind.contains("dual_leader")
                || ev.kind.contains("wrong")
                || ev.kind.contains("trajectory")
            {
                println!("seed={dump_seed} {ev:?}");
            }
        }
        let ok = t.silent_wrong == 0
            && t.row_half_indexed == 0
            && t.consistency_violations == 0
            && t.false_majority == 0
            && t.trajectory_violations == 0;
        println!(
            "dump seed={dump_seed} ok={ok} hash={:016x} silent_wrong={} row_half={} consistency={} false_majority={} dual_leader_fail_open={} traj={}",
            t.trace_hash,
            t.silent_wrong,
            t.row_half_indexed,
            t.consistency_violations,
            t.false_majority,
            t.dual_leader_fail_open,
            t.trajectory_violations
        );
        let _ = std::fs::remove_dir_all(&parent);
        std::process::exit(i32::from(!ok));
    }
    let workers = if pedradb_core::write_admission_kernel::batch_is_empty(workers as u64) {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        workers
    };

    println!(
        "world_swarm seeds={n_seeds} start={start_seed} workers={workers} n_nodes={n_nodes} steps={steps} ranges={n_ranges} buggify={buggify} consistency={consistency} mem={mem_storage} upgrade={membership_upgrade} trajectory={trajectory_check}"
    );

    let mk = move |_seed: u64, parent: &std::path::Path| WorldConfig {
        n_nodes,
        n_ranges,
        schedule_steps: steps,
        parent: parent.to_path_buf(),
        exchange_rounds: 48,
        buggify,
        net_reorder_window: if buggify { 2 } else { 0 },
        consistency_check: consistency,
        mem_storage,
        membership_upgrade,
        trajectory_check,
        ..Default::default()
    };

    let report = run_swarm(start_seed, n_seeds, workers, mk);
    for s in &report.seeds {
        if !s.ok {
            eprintln!(
                "FAIL seed={} hash={:016x} silent_wrong={} row_half={} consistency={} traj={} err={:?}",
                s.seed,
                s.trace_hash,
                s.silent_wrong,
                s.row_half_indexed,
                s.consistency_violations,
                s.trajectory_violations,
                s.err
            );
        }
    }
    println!(
        "summary seeds={} workers={} wall={:.2}s seeds_per_s={:.2} failures={}",
        report.seeds.len(),
        report.workers,
        report.wall_s,
        report.seeds_per_s,
        report.failures
    );

    if let Ok(dir) = std::env::var("PEDRA_SWARM_LOG") {
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("swarm.jsonl"), report.jsonl() + "\n");
        let summary = format!(
            "{{\"seeds\":{},\"workers\":{},\"wall_s\":{:.3},\"seeds_per_s\":{:.3},\"failures\":{},\"n_nodes\":{n_nodes},\"steps\":{steps}}}\n",
            report.seeds.len(),
            report.workers,
            report.wall_s,
            report.seeds_per_s,
            report.failures
        );
        let _ = std::fs::write(dir.join("swarm_summary.json"), summary);
        println!("wrote {}", dir.display());
    }

    if !pedradb_core::write_admission_kernel::batch_is_empty(report.failures as u64) {
        std::process::exit(1);
    }
}
