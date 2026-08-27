//! RFC-0063: hunt *valuable* World seeds (UCB1), not random volume.
//!
//! Reward = membership / trajectory / disk trip / silent_wrong (the last
//! is a finding). Prints JSONL; exit 1 if silent_wrong > 0.
//!
//! ```text
//! cargo run -p pedradb-world --release --bin world_hunt -- [n_seeds] [start]
//! ```

use std::collections::HashMap;
use std::process::ExitCode;

use pedradb_world::bandit::{pick_seed, Ucb1};
use pedradb_world::{fdb_class_campaign, temp_parent, World};

fn main() -> ExitCode {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let start: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0063_1000);
    let arms = ["buggify", "membership", "net", "disk"];
    let mut bandit = Ucb1::new(&arms);
    let mut cursors = HashMap::new();
    let mut used = std::collections::HashSet::new();
    let mut silent = 0u64;
    let mut interesting = 0u64;
    for _ in 0..n {
        let (seed, wanted) = pick_seed(&bandit, &mut cursors, &mut used, start, |s, arm| {
            match arm {
                "membership" => s % 5 == 0,
                "net" => s % 3 == 1,
                "disk" => s % 3 == 2,
                _ => true,
            }
        });
        let parent = temp_parent("hunt");
        let mut cfg = fdb_class_campaign(parent.clone());
        cfg.membership_upgrade = wanted == "membership";
        cfg.consistency_check = true;
        cfg.trajectory_check = wanted == "membership";
        let res = World::new(seed, cfg).run();
        let _ = std::fs::remove_dir_all(&parent);
        match res {
            Ok(t) => {
                let mut reward = 0.05_f64;
                reward += t.membership_events as f64 * 0.1;
                reward += t.trajectory_violations as f64;
                reward += t.disk_tripped_nodes as f64 * 0.2;
                reward += f64::from(t.coverage_mask.count_ones()) * 0.02;
                if t.silent_wrong > 0 {
                    reward += 10.0;
                    silent += 1;
                    eprintln!(
                        "FINDING silent_wrong seed={seed} hash={:x} sw={}",
                        t.trace_hash, t.silent_wrong
                    );
                }
                if t.membership_events > 0 || t.disk_tripped_nodes > 0 {
                    interesting += 1;
                }
                bandit.update(&wanted, reward);
                println!(
                    "hunt seed={seed} arm={wanted} hash={:x} reward={reward:.3} memb={} trip={} silent_wrong={} traj={}",
                    t.trace_hash,
                    t.membership_events,
                    t.disk_tripped_nodes,
                    t.silent_wrong,
                    t.trajectory_violations
                );
            }
            Err(e) => {
                bandit.update(&wanted, 0.0);
                eprintln!("hunt seed={seed} arm={wanted} err={e}");
            }
        }
    }
    println!(
        "world_hunt_ok n={n} interesting={interesting} silent_wrong_seeds={silent}"
    );
    if silent > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
