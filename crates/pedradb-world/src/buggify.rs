//! RFC-0018 BuggifySchedule: seed → multi-site arm list (deterministic).

use pedradb_core::{Rng, SeedRng};
use pedradb_sim::{FaultKind, OpClass};

use crate::coverage::CoverageMask;

/// One deterministic fault arm derived from a World seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuggifyArm {
    /// Inventory site id (e.g. `E.write`, `N.part`).
    pub site: String,
    /// When in the schedule (step index hint).
    pub at_step: u32,
    /// Opaque kind tag for logs.
    pub kind: String,
    /// Peer id when applicable (1-based), else 0.
    pub node: u64,
    /// Extra param (delay ticks, after_ops, ppm, …).
    pub param: u64,
}

/// Seed-derived multi-fault plan.
#[derive(Debug, Clone)]
pub struct BuggifySchedule {
    /// World seed.
    pub seed: u64,
    /// Ordered arms.
    pub arms: Vec<BuggifyArm>,
}

impl BuggifySchedule {
    /// Sites touched by this schedule (mask).
    #[must_use]
    pub fn coverage_mask(&self) -> CoverageMask {
        let mut m = CoverageMask::new();
        for a in &self.arms {
            m.hit(&a.site);
        }
        m
    }
}

/// Build K arms from seed (1..=kmax). Fully deterministic.
#[must_use]
pub fn buggify_schedule_from_seed(
    seed: u64,
    n_nodes: u64,
    schedule_steps: usize,
) -> BuggifySchedule {
    let rng = SeedRng::new(seed ^ 0xB006_1F1E);
    let kmax = 6u64;
    let k = 1 + rng.gen_range(kmax);
    let n_nodes = n_nodes.max(1);
    let steps = schedule_steps.max(1) as u64;
    let mut arms = Vec::with_capacity(k as usize);
    for i in 0..k {
        let site_roll = rng.gen_range(10);
        let at_step = (rng.gen_range(steps) as u32).min(schedule_steps.saturating_sub(1) as u32);
        let node = 1 + rng.gen_range(n_nodes);
        let (site, kind, param) = match site_roll {
            0 => ("E.write", "io", rng.gen_range(4)),
            1 => ("E.sync", "sync_fail", 0),
            2 => ("E.rename", "io", 0),
            3 => ("N.send", "drop_delay", 50_000 + rng.gen_range(200_000)),
            4 => ("N.part", "partition", node),
            5 => ("N.corrupt", "bitflip_ppm", 1000 + rng.gen_range(50_000)),
            6 => ("C.tick", "jump", 5 + rng.gen_range(40)),
            7 => ("D.bitrot", "xor1", rng.gen_range(64)),
            8 => ("H.open", "fail_nth", 1 + rng.gen_range(8)),
            _ => ("B.buggify", "compose", i),
        };
        arms.push(BuggifyArm {
            site: site.to_string(),
            at_step,
            kind: kind.to_string(),
            node,
            param,
        });
    }
    // Always tag buggify registry site.
    if !arms.iter().any(|a| a.site == "B.buggify") {
        arms.push(BuggifyArm {
            site: "B.buggify".into(),
            at_step: 0,
            kind: "registry".into(),
            node: 0,
            param: k,
        });
    }
    BuggifySchedule { seed, arms }
}

/// Map arm kind to FailingEnv settings when applying disk arms.
#[must_use]
pub fn arm_to_disk_kind(arm: &BuggifyArm) -> Option<(OpClass, FaultKind, u64, bool)> {
    match arm.site.as_str() {
        "E.write" => Some((
            OpClass::Write,
            if arm.param % 3 == 0 {
                FaultKind::ShortWrite
            } else if arm.param % 3 == 1 {
                FaultKind::StorageFull
            } else {
                FaultKind::IoError
            },
            arm.param.min(8),
            true,
        )),
        "E.sync" => Some((OpClass::Sync, FaultKind::SyncFail, 0, true)),
        "E.rename" => Some((OpClass::Rename, FaultKind::IoError, 0, true)),
        "E.create_open" | "H.open" => Some((
            OpClass::CreateOpen,
            FaultKind::IoError,
            arm.param.min(8),
            true,
        )),
        "E.remove" => Some((OpClass::Remove, FaultKind::IoError, 0, true)),
        "E.meta" => Some((OpClass::Meta, FaultKind::IoError, 0, true)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buggify_schedule_replayable() {
        let a = buggify_schedule_from_seed(42, 3, 16);
        let b = buggify_schedule_from_seed(42, 3, 16);
        assert_eq!(a.arms, b.arms);
        assert!(!a.arms.is_empty());
        let m = a.coverage_mask();
        assert!(m.has("B.buggify") || m.popcount() > 0);
        // Sweep until we find a seed with a different plan (deterministic).
        let mut found_diff = false;
        for s in 43..200u64 {
            let c = buggify_schedule_from_seed(s, 3, 16);
            if c.arms != a.arms {
                found_diff = true;
                break;
            }
        }
        assert!(found_diff, "expected some seed != 42 to differ");
    }
}
