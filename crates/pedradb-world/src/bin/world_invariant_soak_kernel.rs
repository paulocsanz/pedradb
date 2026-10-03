//! C1.6: scalable World soak with **real** dual-leader / false-majority counters
//! from [`Trace`] (populated via StoreCluster::leader_claim_count + Strong policy).
//!
//! ```text
//! cargo run --release --bin world_invariant_soak -- [n_seeds] [start] [steps]
//! PEDRA_INVARIANT_SEEDS=1000 for roadmap bar.
//! ```

use pedradb_world::{temp_parent, World, WorldConfig};

fn main() {
    let n: u64 = std::env::var("PEDRA_INVARIANT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(|| std::env::args().nth(1).and_then(|s| s.parse().ok()))
        .unwrap_or(32);
    let start: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let steps: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    let mut dual_leader_open = 0u64;
    let mut false_majority = 0u64;
    let mut silent_wrong = 0u64;
    let mut max_claims = 0u64;
    let mut fail_stop = 0u64;
    let mut ok_n = 0u64;
    let parent_root = temp_parent("inv-soak");

    println!("world_invariant_soak n={n} start={start} steps={steps}");
    for i in 0..n {
        let seed = start + i;
        let parent = parent_root.join(format!("s{seed:016x}"));
        let _ = std::fs::create_dir_all(&parent);
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: steps,
            parent: parent.clone(),
            exchange_rounds: 48,
            buggify: true,
            net_reorder_window: 2,
            ..Default::default()
        };
        match World::new(seed, cfg).run() {
            Ok(t) => {
                ok_n += 1;
                dual_leader_open += t.dual_leader_fail_open;
                false_majority += t.false_majority;
                silent_wrong += t.silent_wrong;
                if t.max_leader_claims > max_claims {
                    max_claims = t.max_leader_claims;
                }
            }
            Err(_) => {
                fail_stop += 1;
            }
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    println!(
        "invariant_soak_ok n={n} ok_runs={ok_n} fail_stop={fail_stop} dual_leader_fail_open={dual_leader_open} false_majority={false_majority} silent_wrong={silent_wrong} max_leader_claims={max_claims}"
    );
    if dual_leader_open != 0 || false_majority != 0 || silent_wrong != 0 {
        eprintln!("FAIL: safety invariant violated");
        std::process::exit(1);
    }
}
