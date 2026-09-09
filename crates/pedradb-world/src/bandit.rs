//! UCB1 bandit for World seed exploration (P3.3-lite).
//!
//! Outcome-novelty guided (reward from Trace interestingness), not LLVM branch
//! coverage — same honesty note as `dst-envelope/tesoura/bandit.py`.

use std::collections::HashMap;

/// UCB1 over named arms.
#[derive(Debug, Clone)]
pub struct Ucb1 {
    arms: Vec<String>,
    pulls: HashMap<String, u64>,
    total_reward: HashMap<String, f64>,
    n: u64,
}

impl Ucb1 {
    /// Build bandit with fixed arm list (order is init priority for never-pulled).
    #[must_use]
    pub fn new(arms: &[&str]) -> Self {
        let mut pulls = HashMap::new();
        let mut total_reward = HashMap::new();
        for a in arms {
            pulls.insert((*a).to_string(), 0);
            total_reward.insert((*a).to_string(), 0.0);
        }
        Self {
            arms: arms.iter().map(|s| (*s).to_string()).collect(),
            pulls,
            total_reward,
            n: 0,
        }
    }

    /// Select next arm (UCB1).
    #[must_use]
    pub fn select(&self) -> &str {
        for a in &self.arms {
            if pedradb_core::write_admission_kernel::batch_is_empty(self.pulls.get(a).copied().unwrap_or(0) as u64) {
                return a.as_str();
            }
        }
        let mut best = self.arms[0].as_str();
        let mut best_score = f64::NEG_INFINITY;
        let n = self.n.max(1) as f64;
        for a in &self.arms {
            let pulls = self.pulls[a] as f64;
            let mean = self.total_reward[a] / pulls;
            let bonus = (2.0 * n.ln() / pulls).sqrt();
            let score = mean + bonus;
            if score > best_score {
                best_score = score;
                best = a.as_str();
            }
        }
        best
    }

    /// Record reward in \[0, 1+] for an arm.
    pub fn update(&mut self, arm: &str, reward: f64) {
        let p = self.pulls.entry(arm.to_string()).or_insert(0);
        *p += 1;
        *self.total_reward.entry(arm.to_string()).or_insert(0.0) += reward;
        self.n += 1;
    }

    /// Pull counts and means for logging.
    #[must_use]
    pub fn stats(&self) -> Vec<(String, u64, f64)> {
        self.arms
            .iter()
            .map(|a| {
                let n = self.pulls.get(a).copied().unwrap_or(0);
                let r = self.total_reward.get(a).copied().unwrap_or(0.0);
                let mean = if n == 0 { 0.0 } else { r / n as f64 };
                (a.clone(), n, mean)
            })
            .collect()
    }
}

/// Online pick: `(seed, wanted_arm)`.
///
/// Finds an unused seed for which `has_arm(seed, wanted)` is true (multi-label).
/// Falls back to any unused seed if the class is absent in the scan window.
pub fn pick_seed(
    bandit: &Ucb1,
    cursors: &mut HashMap<String, u64>,
    used: &mut std::collections::HashSet<u64>,
    start_seed: u64,
    has_arm: impl Fn(u64, &str) -> bool,
) -> (u64, String) {
    let want = bandit.select().to_string();
    let mut seed = *cursors.get(&want).unwrap_or(&start_seed);
    for _ in 0..100_000u32 {
        if !used.contains(&seed) && has_arm(seed, want.as_str()) {
            cursors.insert(want.clone(), seed.saturating_add(1));
            used.insert(seed);
            return (seed, want);
        }
        seed = seed.saturating_add(1);
    }
    let mut s = start_seed;
    while used.contains(&s) {
        s = s.saturating_add(1);
    }
    used.insert(s);
    cursors.insert(want.clone(), s.saturating_add(1));
    (s, want)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{arm_for_seed, WORLD_ARMS};

    #[test]
    fn ucb1_prefers_rewarded_arm() {
        let mut b = Ucb1::new(&["a", "b"]);
        assert_eq!(b.select(), "a"); // never pulled first
        b.update("a", 0.0);
        assert_eq!(b.select(), "b");
        b.update("b", 1.0);
        // After both pulled, high mean wins with exploration.
        for _ in 0..20 {
            let arm = b.select().to_string();
            if arm == "b" {
                b.update("b", 1.0);
            } else {
                b.update("a", 0.0);
            }
        }
        let stats = b.stats();
        let mean_b = stats.iter().find(|(n, _, _)| n == "b").unwrap().2;
        let mean_a = stats.iter().find(|(n, _, _)| n == "a").unwrap().2;
        assert!(mean_b > mean_a);
    }

    #[test]
    fn arm_for_seed_is_stable() {
        let a = arm_for_seed(99, 3, 16);
        let b = arm_for_seed(99, 3, 16);
        assert_eq!(a, b);
        assert!(WORLD_ARMS.contains(&a));
    }
}
