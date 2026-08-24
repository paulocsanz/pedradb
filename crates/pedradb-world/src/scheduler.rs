//! RFC 0003: ready-queue order via the in-tree [`crate::pct`] seam
//! (minimal PCT copied from the sibling `dst_core`; RFC-0050 P0).
//!
//! Does **not** change [`crate::World::run`] traces (those stay seed→Action).
//! This is the hook for “who ticks / who delivers AE” under PCT.

pub use crate::pct::{run_with_scheduler, schedule_hash, PctScheduler};

/// Seed → peer index sequence (`ops` steps per peer).
pub fn pct_ready_queue(seed: u64, n_peers: usize, ops: usize) -> Vec<usize> {
    let n = n_peers.max(1);
    let ops = ops.max(1);
    let k = n.saturating_mul(ops);
    let mut sched = PctScheduler::from_seed(seed, n, 2, k);
    run_with_scheduler(n, ops, &mut sched)
        .into_iter()
        .map(|s| s.worker)
        .collect()
}

/// Bit-stable hash of [`pct_ready_queue`].
pub fn pct_ready_queue_hash(seed: u64, n_peers: usize, ops: usize) -> u64 {
    let n = n_peers.max(1);
    let ops = ops.max(1);
    let k = n.saturating_mul(ops);
    let mut sched = PctScheduler::from_seed(seed, n, 2, k);
    schedule_hash(&run_with_scheduler(n, ops, &mut sched))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_ready_queue() {
        let a = pct_ready_queue(42, 3, 4);
        let b = pct_ready_queue(42, 3, 4);
        assert_eq!(a, b);
        assert_eq!(pct_ready_queue_hash(42, 3, 4), pct_ready_queue_hash(42, 3, 4));
        assert_ne!(pct_ready_queue_hash(42, 3, 4), pct_ready_queue_hash(43, 3, 4));
        assert_eq!(a.len(), 12);
    }
}
