//! P3 soak: multi-seed World runs with UCB1 arm selection over schedule classes.
//!
//! ```text
//! cargo run --release --bin world_soak -- [n_trials] [start_seed] [steps]
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use pedradb_world::bandit::{pick_seed, Ucb1};
use pedradb_world::buggify::buggify_schedule_from_seed;
use pedradb_world::schedule::{seed_has_arm, ScheduleCoverage, WORLD_ARMS};
use pedradb_world::{temp_parent, World, WorldConfig};

/// Site×kind arms for RFC-0018 P1.2 (mask novelty over buggify inventory sites).
const SITE_KIND_ARMS: &[&str] = &[
    "E.write",
    "E.sync",
    "N.send",
    "N.part",
    "N.corrupt",
    "C.tick",
    "B.buggify",
    "disk",
    "membership",
    "partition",
];

fn interest(t: &pedradb_world::Trace, cov: ScheduleCoverage) -> f64 {
    let mut r = 0.0;
    // Errors / faults are "interesting" for exploration (outcome novelty).
    r += f64::from(t.puts_err.min(3)) * 0.15;
    r += f64::from(t.dcs_err.min(3)) * 0.1;
    r += f64::from(t.disk_arms.min(4)) * 0.12;
    r += if !pedradb_core::write_admission_kernel::batch_is_empty(t.rpc_applied as u64) {
        0.2
    } else {
        0.0
    };
    r += if !pedradb_core::write_admission_kernel::batch_is_empty(t.puts_ok as u64) {
        0.15
    } else {
        0.0
    };
    r += if cov.membership { 0.1 } else { 0.0 };
    r += if cov.dcs_ttl { 0.1 } else { 0.0 };
    r += if cov.disk { 0.15 } else { 0.0 };
    // Mask popcount novelty (site×kind coverage).
    r += (t.coverage_mask.count_ones() as f64) * 0.03;
    r.min(2.0)
}

fn seed_has_site(seed: u64, site: &str) -> bool {
    if WORLD_ARMS.contains(&site) {
        return seed_has_arm(seed, 3, 16, site);
    }
    let plan = buggify_schedule_from_seed(seed, 3, 16);
    plan.arms.iter().any(|a| a.site == site)
}

fn main() {
    let n_trials: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);
    let start_seed: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let steps: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    let parent_root = temp_parent("soak");
    // Prefer site×kind arms when BUGGIFY_SOAK=1 (default on).
    let use_site_kind = std::env::var("PEDRA_SOAK_SITE_KIND")
        .map(|v| v != "0")
        .unwrap_or(true);
    let arms: &[&str] = if use_site_kind {
        SITE_KIND_ARMS
    } else {
        WORLD_ARMS
    };
    let mut bandit = Ucb1::new(arms);
    let mut cursors: HashMap<String, u64> = HashMap::new();
    for a in arms {
        cursors.insert((*a).to_string(), start_seed);
    }
    let mut used = HashSet::new();
    let mut seen_masks = HashSet::new();
    let mut seen_cov_bits = HashSet::new();
    let mut fail = 0u32;
    let mut rows = Vec::new();

    println!(
        "world_soak trials={n_trials} start_seed={start_seed} steps={steps} site_kind={use_site_kind}"
    );

    for i in 0..n_trials {
        let (seed, arm) = pick_seed(&bandit, &mut cursors, &mut used, start_seed, |s, a| {
            if use_site_kind {
                seed_has_site(s, a)
            } else {
                seed_has_arm(s, 3, steps, a)
            }
        });
        let sch = pedradb_world::schedule::schedule_from_seed(seed, 3, steps);
        let cov = ScheduleCoverage::from_actions(&sch);
        let novel = seen_masks.insert(cov.mask());
        let matched = if use_site_kind {
            seed_has_site(seed, &arm)
        } else {
            seed_has_arm(seed, 3, steps, &arm)
        };

        let parent = parent_root.join(format!("t{i:04}-s{seed:016x}"));
        let _ = std::fs::create_dir_all(&parent);
        let cfg = WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: steps,
            parent: parent.clone(),
            exchange_rounds: 48,
            buggify: use_site_kind,
            net_reorder_window: if use_site_kind { 2 } else { 0 },
            ..Default::default()
        };
        let result = World::new(seed, cfg).run();
        let _ = std::fs::remove_dir_all(&parent);

        match result {
            Ok(t) => {
                let mut reward = interest(&t, cov);
                if novel {
                    reward += 0.25;
                }
                let mask_novel = seen_cov_bits.insert(t.coverage_mask);
                if mask_novel {
                    reward += 0.2;
                }
                if !matched {
                    reward *= 0.5; // missed class — still explore
                }
                bandit.update(&arm, reward);
                println!(
                    "ok i={i} seed={seed} arm={arm} hit={matched} novel={novel} mask_novel={mask_novel} reward={reward:.3} hash={:016x} cov_pop={} puts={}/{} dcs={}/{} disk_arms={} rpc={}",
                    t.trace_hash,
                    t.coverage_mask.count_ones(),
                    t.puts_ok,
                    t.puts_err,
                    t.dcs_ok,
                    t.dcs_err,
                    t.disk_arms,
                    t.rpc_applied
                );
                rows.push(format!(
                    "{{\"i\":{i},\"seed\":{seed},\"arm\":\"{arm}\",\"hit\":{matched},\"ok\":true,\"reward\":{reward:.4},\"novel\":{novel},\"mask\":\"{:016x}\",\"hash\":\"{:016x}\"}}",
                    t.coverage_mask, t.trace_hash
                ));
            }
            Err(e) => {
                fail += 1;
                bandit.update(&arm, 1.5);
                eprintln!("FAIL i={i} seed={seed} arm={arm}: {e}");
                rows.push(format!(
                    "{{\"i\":{i},\"seed\":{seed},\"arm\":\"{arm}\",\"ok\":false,\"err\":\"{e}\"}}"
                ));
            }
        }
    }

    println!("== bandit stats ==");
    for (arm, pulls, mean) in bandit.stats() {
        println!("  arm={arm:12} pulls={pulls:3} mean={mean:.3}");
    }
    println!(
        "summary fail={fail}/{n_trials} unique_cov_masks={}",
        seen_masks.len()
    );

    // Optional JSONL dump under findings if PEDRA_SOAK_LOG set.
    if let Ok(dir) = std::env::var("PEDRA_SOAK_LOG") {
        let p = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&p);
        let path = p.join("soak.jsonl");
        let body = rows.join("\n") + "\n";
        let _ = std::fs::write(&path, body);
        println!("wrote {}", path.display());
    }

    let _ = std::fs::remove_dir_all(&parent_root);
    if !pedradb_core::write_admission_kernel::batch_is_empty(fail as u64) {
        std::process::exit(1);
    }
}
