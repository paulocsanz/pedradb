//! Minimal PCT scheduler, in-tree (RFC-0050 P0 / RFC-0051 P0).
//!
//! Copied verbatim-semantics from the sibling `rust-dst/dst_core` scheduler
//! (Burckhardt et al., ASPLOS 2010, Fig. 7) so the workspace needs no path
//! outside itself. Same seed ⇒ same π — the property `World::run` and the
//! ConcurrentDb PCT runner both replay on.

/// Chooses the next enabled task. Same seed ⇒ same π.
pub trait Scheduler {
    /// Next enabled task, or `None` when idle.
    fn next(&mut self, enabled: &[usize]) -> Option<usize>;
}

/// PCT (Burckhardt et al., ASPLOS 2010 Fig. 7): depth-`d` priority backtracking.
pub struct PctScheduler {
    pri: Vec<i32>,
    change_points: Vec<(usize, i32)>,
    steps: usize,
}

impl PctScheduler {
    /// Seed → priorities + change points over `k` steps.
    #[must_use]
    pub fn from_seed(seed: u64, n: usize, depth: usize, k: usize) -> Self {
        assert!(n > 0 && depth >= 1);
        let k = k.max(1);
        let mut state = seed ^ 0xDC70_DC70_DC70_DC70;
        let mut next = || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let mut pi: Vec<i32> = (1..=n as i32).collect();
        for i in (1..n).rev() {
            let j = (next() as usize) % (i + 1);
            pi.swap(i, j);
        }
        let d = depth as i32;
        let pri: Vec<i32> = pi.iter().map(|&p| d + p - 1).collect();
        let mut change_points = Vec::new();
        if depth >= 2 {
            for i in 1..depth {
                let ki = (next() as usize) % k + 1;
                change_points.push((ki, d - i as i32));
            }
        }
        Self {
            pri,
            change_points,
            steps: 0,
        }
    }
}

impl Scheduler for PctScheduler {
    fn next(&mut self, enabled: &[usize]) -> Option<usize> {
        if enabled.is_empty() {
            return None;
        }
        let t = *enabled
            .iter()
            .max_by_key(|&&i| (self.pri.get(i).copied().unwrap_or(0), -(i as i32)))?;
        self.steps += 1;
        for &(ki, p) in &self.change_points {
            if ki == self.steps {
                if let Some(slot) = self.pri.get_mut(t) {
                    *slot = p;
                }
            }
        }
        Some(t)
    }
}

/// One scheduled step (worker index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Worker that ran.
    pub worker: usize,
}

/// Drive `n` tasks that each have `ops` remaining steps through `sched`.
pub fn run_with_scheduler(n: usize, ops: usize, sched: &mut impl Scheduler) -> Vec<Step> {
    let mut rem = vec![ops; n];
    let mut out = Vec::new();
    loop {
        let enabled: Vec<usize> = (0..n).filter(|&i| rem[i] > 0).collect();
        match sched.next(&enabled) {
            None => break,
            Some(t) => {
                rem[t] -= 1;
                out.push(Step { worker: t });
            }
        }
    }
    out
}

/// Bit-stable hash of a schedule (same schedule ⇒ same hash).
#[must_use]
pub fn schedule_hash(steps: &[Step]) -> u64 {
    let mut h = 0xC0FF_EE00_DC70_0001u64;
    for s in steps {
        h ^= s.worker as u64;
        h = h.wrapping_mul(0x1000_0000_01B3);
        h ^= h >> 33;
    }
    h ^ (steps.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_schedule() {
        let mut a = PctScheduler::from_seed(42, 3, 2, 8);
        let mut b = PctScheduler::from_seed(42, 3, 2, 8);
        let sa = run_with_scheduler(3, 4, &mut a);
        let sb = run_with_scheduler(3, 4, &mut b);
        assert_eq!(sa, sb);
        assert_eq!(schedule_hash(&sa), schedule_hash(&sb));
        let mut c = PctScheduler::from_seed(43, 3, 2, 8);
        let sc = run_with_scheduler(3, 4, &mut c);
        assert_ne!(schedule_hash(&sa), schedule_hash(&sc));
        assert_eq!(sa.len(), 12);
    }

    #[test]
    fn pct_can_reorder_vs_round_robin() {
        // Depth-2 PCT over 4 workers must sometimes deviate from 0,1,2,3
        // cycling (that deviation is exactly the preemption it buys).
        let mut deviations = 0;
        for seed in 0..64u64 {
            let mut sched = PctScheduler::from_seed(seed, 4, 2, 16);
            let steps = run_with_scheduler(4, 4, &mut sched);
            let rr: Vec<usize> = (0..16usize).map(|i| i % 4).collect();
            let got: Vec<usize> = steps.iter().map(|s| s.worker).collect();
            if got != rr {
                deviations += 1;
            }
        }
        assert!(deviations > 0, "PCT must interleave, got none in 64 seeds");
    }
}
