//! RFC-0018 / C0.4: multi-seed buggify matrix — **measured** silent_wrong from Trace.
//!
//! ```text
//! cargo run --release --bin world_buggify_matrix -- [n_seeds] [start_seed] [steps]
//! ```

use pedradb_world::coverage::{l2_required_bits_from_unit_trials, CoverageMask, SEAM_IDS};
use pedradb_world::{temp_parent, World, WorldConfig};

fn main() {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let start: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let steps: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    let parent_root = temp_parent("buggify-matrix");
    let mut union = CoverageMask::new();
    let mut fail = 0u32;
    let mut silent = 0u64;
    let mut dual_open = 0u64;
    let mut false_maj = 0u64;
    let mut ok_n = 0u32;
    let mut hashes = std::collections::HashSet::new();

    println!("world_buggify_matrix n={n} start={start} steps={steps}");

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
            net_drop_ppm: 10_000,
            net_max_delay: 2,
            ..Default::default()
        };
        match World::new(seed, cfg).run() {
            Ok(t) => {
                ok_n += 1;
                silent += t.silent_wrong;
                dual_open += t.dual_leader_fail_open;
                false_maj += t.false_majority;
                hashes.insert(t.trace_hash);
                let mut m = CoverageMask::new();
                for (idx, id) in SEAM_IDS.iter().enumerate() {
                    if t.coverage_mask & (1u64 << idx) != 0 {
                        m.hit(id);
                    }
                }
                union.merge(&m);
                println!(
                    "ok seed={seed} hash={:016x} mask_pop={} arms={} puts={}/{} rpc={} silent={} dual={} false_maj={}",
                    t.trace_hash,
                    m.popcount(),
                    t.arms.len(),
                    t.puts_ok,
                    t.puts_err,
                    t.rpc_applied,
                    t.silent_wrong,
                    t.dual_leader_fail_open,
                    t.false_majority
                );
            }
            Err(e) => {
                fail += 1;
                eprintln!("fail-stop seed={seed}: {e}");
            }
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    let unit = l2_required_bits_from_unit_trials();
    let mut l2 = unit;
    l2.merge(&union);
    let unique = hashes.len() as u64;
    // After SeedRng fix: consecutive seeds must not collapse. Allow tiny
    // collisions from schedule grammar (not 50% even/odd collapse).
    let min_unique = ((ok_n as f64) * 0.9).ceil() as u64;

    println!("== summary ==");
    println!(
        "ok={ok_n} fail_stop={fail} silent_wrong={silent} dual_leader_fail_open={dual_open} false_majority={false_maj} unique_hashes={unique} min_unique={min_unique}"
    );
    println!(
        "union_mask_pop={} ratio={:.2} l2_pop={} l2_ratio={:.2}",
        union.popcount(),
        union.ratio(),
        l2.popcount(),
        l2.ratio()
    );
    println!("hit_ids={:?}", union.hit_ids());

    if silent > 0 || dual_open > 0 || false_maj > 0 {
        eprintln!(
            "BUG: silent_wrong={silent} dual_leader_fail_open={dual_open} false_majority={false_maj}"
        );
        std::process::exit(2);
    }
    if (l2.popcount() as usize) < SEAM_IDS.len() {
        eprintln!(
            "BUG: L2 incomplete pop={} need={}",
            l2.popcount(),
            SEAM_IDS.len()
        );
        std::process::exit(3);
    }
    if ok_n > 0 && unique < min_unique {
        eprintln!(
            "BUG: unique_hashes={unique} < 90% of ok_runs={ok_n} (SeedRng even/odd collapse?)"
        );
        std::process::exit(4);
    }
    println!("buggify_matrix_ok n={n} l2=100% silent_wrong=0 unique_hashes={unique}/{ok_n}");
}
