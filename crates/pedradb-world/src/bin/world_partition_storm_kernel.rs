//! C2.6: partition+heal and/or remove+add (snapshot catch-up) stress.
//! Requires (Partition∧Heal) ∨ (RemoveMember∧AddMember). Measures real Trace safety.

use pedradb_world::schedule::{schedule_from_seed, Action};
use pedradb_world::{temp_parent, World, WorldConfig};

fn schedule_has_partition_storm(seed: u64, steps: usize) -> bool {
    let sch = schedule_from_seed(seed, 3, steps);
    let mut part = false;
    let mut heal = false;
    let mut rm = false;
    let mut add = false;
    for a in &sch {
        match a {
            Action::Partition { .. } => part = true,
            Action::Heal { .. } => heal = true,
            Action::RemoveMember { .. } => rm = true,
            Action::AddMember { .. } => add = true,
            _ => {}
        }
    }
    (part && heal) || (rm && add)
}

fn main() {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(48);
    let steps: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);

    let mut ran = 0u64;
    let mut fail_stop = 0u64;
    let mut dual_leader_open = 0u64;
    let mut false_majority = 0u64;
    let mut silent_wrong = 0u64;
    let mut max_claims = 0u64;
    let mut scanned = 0u64;
    let parent_root = temp_parent("part-storm");

    println!("world_partition_storm target_runs={n} steps={steps}");
    let mut seed = 1u64;
    while ran < n && scanned < n * 80 {
        scanned += 1;
        seed += 1;
        if !schedule_has_partition_storm(seed, steps) {
            continue;
        }
        let parent = parent_root.join(format!("s{seed:016x}"));
        let _ = std::fs::create_dir_all(&parent);
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: steps,
            parent: parent.clone(),
            exchange_rounds: 64,
            buggify: false,
            ..Default::default()
        };
        match World::new(seed, cfg).run() {
            Ok(t) => {
                ran += 1;
                dual_leader_open += t.dual_leader_fail_open;
                false_majority += t.false_majority;
                silent_wrong += t.silent_wrong;
                if t.max_leader_claims > max_claims {
                    max_claims = t.max_leader_claims;
                }
                if t.dual_leader_fail_open > 0 || t.false_majority > 0 || t.silent_wrong > 0 {
                    eprintln!(
                        "FAIL seed={seed} dual={} false_maj={} silent={} max_claims={}",
                        t.dual_leader_fail_open,
                        t.false_majority,
                        t.silent_wrong,
                        t.max_leader_claims
                    );
                    std::process::exit(2);
                }
                println!(
                    "ok seed={seed} hash={:016x} puts={}/{} rpc={} max_claims={}",
                    t.trace_hash, t.puts_ok, t.puts_err, t.rpc_applied, t.max_leader_claims
                );
            }
            Err(e) => {
                fail_stop += 1;
                ran += 1;
                println!("fail-stop seed={seed}: {e}");
            }
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    println!(
        "partition_storm_ok runs={ran} fail_stop={fail_stop} dual_leader_fail_open={dual_leader_open} false_majority={false_majority} silent_wrong={silent_wrong} max_leader_claims={max_claims}"
    );
    if dual_leader_open != 0 || false_majority != 0 || silent_wrong != 0 {
        eprintln!("FAIL: safety counters non-zero");
        std::process::exit(1);
    }
    if ran < n {
        eprintln!("WARN: only found {ran}/{n} storm seeds in scan={scanned}");
    }
}
