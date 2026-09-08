//! C0.7: greedy shrink of buggify arms for a seed (World arm toggles).
//!
//! ```text
//! cargo run --release --bin world_shrink -- <seed> [steps]
//! ```
//!
//! Drops arms one-by-one while `World::run` stays Ok with same success class.
//! Prints original vs minimal arm counts and mask bits.

use pedradb_world::buggify::buggify_schedule_from_seed;
use pedradb_world::{temp_parent, World, WorldConfig};

fn run_mask(seed: u64, steps: usize, arm_mask: Option<u64>) -> Result<(bool, u64, usize), String> {
    let parent = temp_parent("shrink");
    let cfg = WorldConfig {
        n_nodes: 3,
        n_ranges: 1,
        schedule_steps: steps,
        parent: parent.clone(),
        exchange_rounds: 32,
        buggify: true,
        buggify_arm_mask: arm_mask,
        net_reorder_window: 2,
        ..Default::default()
    };
    let res = World::new(seed, cfg).run();
    let _ = std::fs::remove_dir_all(&parent);
    match res {
        Ok(t) => Ok((true, t.trace_hash, t.arms.len())),
        Err(e) => Err(e.to_string()),
    }
}

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let steps: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let plan = buggify_schedule_from_seed(seed, 3, steps);
    let n = plan.arms.len().min(64);
    let full_mask = if n == 0 { 0u64 } else { (1u64 << n) - 1 };

    let baseline = run_mask(seed, steps, Some(full_mask));
    let (base_ok, base_hash, base_arms) = match &baseline {
        Ok(t) => *t,
        Err(e) => {
            // Fail-stop baseline: try to keep failure while dropping arms.
            eprintln!("baseline fail-stop seed={seed}: {e}");
            let mut mask = full_mask;
            for i in 0..n {
                let trial = mask & !(1u64 << i);
                if trial == 0 {
                    continue;
                }
                if run_mask(seed, steps, Some(trial)).is_err() {
                    mask = trial;
                }
            }
            let kept: Vec<_> = plan
                .arms
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1u64 << i) != 0)
                .map(|(_, a)| format!("{}:{}", a.site, a.kind))
                .collect();
            println!(
                "shrink_ok seed={seed} class=fail-stop original_arms={n} minimal_arms={} mask={mask:#x} arms={kept:?}",
                kept.len()
            );
            return;
        }
    };

    // Success baseline: drop any arm that keeps Ok (prefer fewer arms).
    let mut mask = full_mask;
    let mut irreducible = Vec::new();
    for i in 0..n {
        let trial = mask & !(1u64 << i);
        match run_mask(seed, steps, Some(trial)) {
            Ok((true, _, _)) => {
                mask = trial; // arm i was not necessary for success
            }
            Ok((false, _, _)) | Err(_) => {
                irreducible.push(i);
            }
        }
    }

    let kept: Vec<_> = plan
        .arms
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1u64 << i) != 0)
        .map(|(_, a)| format!("{}:{}@{}", a.site, a.kind, a.at_step))
        .collect();

    let reduced = kept.len() < base_arms || mask != full_mask;
    println!(
        "shrink_ok seed={seed} class=ok base_hash={base_hash:016x} original_arms={base_arms} minimal_arms={} reduced={reduced} mask={mask:#x} irreducible_idx={irreducible:?} arms={kept:?}",
        kept.len()
    );
    // C0.7 bar: either reduced arm count, empty (all optional), or documented irreducible set.
    if !reduced && irreducible.is_empty() && base_arms > 0 {
        // All arms dropped successfully → reduced to 0
        println!("note=all_arms_optional");
    }
    if base_ok && kept.len() < base_arms {
        println!("proof=reduced");
    } else if pedradb_core::write_admission_kernel::batch_is_empty(kept.len() as u64) {
        println!("proof=empty_optional");
    } else {
        println!("proof=irreducible_or_stable");
    }
}
