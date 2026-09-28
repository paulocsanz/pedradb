//! RFC-0287: Non-Zenonian Monotonic Clock and Anti-Resurrection TTL Kernel.
//!
//! Provides mathematically robust monotonic causality for MVCC snapshots and TTL
//! expiration, guaranteeing that wall-clock backward steps (NTP adjustments, VM migrations)
//! can never cause deleted or expired records to resurrect.

use std::cmp::Ordering as CmpOrdering;
use std::sync::atomic::{AtomicU64, Ordering};

/// A composite, strictly non-decreasing logical timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NonZenoTimestamp {
    /// Monotonically incrementing engine epoch (bumped on restarts/reconfigurations).
    pub epoch: u64,
    /// Transaction sequence number assigned by write pipeline.
    pub logical_seq: u64,
    /// Monotonicized physical time tick in nanoseconds.
    pub monotonic_tick_ns: u64,
}

impl PartialOrd for NonZenoTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for NonZenoTimestamp {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.epoch
            .cmp(&other.epoch)
            .then_with(|| self.logical_seq.cmp(&other.logical_seq))
            .then_with(|| self.monotonic_tick_ns.cmp(&other.monotonic_tick_ns))
    }
}

/// Thread-safe generator of non-zenonian monotonic timestamps.
pub struct NonZenoLogicalClock {
    /// Current engine epoch.
    epoch: u64,
    /// Strictly increasing logical sequence counter.
    next_logical_seq: AtomicU64,
    /// Highest observed monotonic tick in nanoseconds.
    last_monotonic_tick_ns: AtomicU64,
}

impl NonZenoLogicalClock {
    /// Creates a new logical clock anchored at a specific epoch and initial tick.
    #[must_use]
    pub fn new(epoch: u64, initial_tick_ns: u64) -> Self {
        Self {
            epoch,
            next_logical_seq: AtomicU64::new(1),
            last_monotonic_tick_ns: AtomicU64::new(initial_tick_ns),
        }
    }

    /// Advances the clock observing a proposed wall-clock reading in nanoseconds.
    /// Even if `wall_clock_ns` is in the past (backward clock step), the generated
    /// timestamp is guaranteed to be strictly greater than any previously issued timestamp.
    pub fn tick(&self, observed_wall_clock_ns: u64) -> NonZenoTimestamp {
        let seq = self.next_logical_seq.fetch_add(1, Ordering::AcqRel);

        loop {
            let last = self.last_monotonic_tick_ns.load(Ordering::Acquire);
            // Non-Zenonian advance: at least last + 1, or wall clock if ahead
            let next_tick = observed_wall_clock_ns.max(last.saturating_add(1));

            if self
                .last_monotonic_tick_ns
                .compare_exchange_weak(last, next_tick, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return NonZenoTimestamp {
                    epoch: self.epoch,
                    logical_seq: seq,
                    monotonic_tick_ns: next_tick,
                };
            }
        }
    }

    /// Returns the current epoch.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// Visibility and TTL purge oracle.
pub struct TtlVisibilityOracle;

impl TtlVisibilityOracle {
    /// Verifies if a record with timestamp `record_ts` is visible under snapshot `snapshot_ts`.
    /// Invariant: Record is visible iff `record_ts <= snapshot_ts`.
    #[must_use]
    pub fn is_visible(record_ts: NonZenoTimestamp, snapshot_ts: NonZenoTimestamp) -> bool {
        record_ts <= snapshot_ts
    }

    /// Computes whether a record has expired given its timestamp, TTL duration in nanoseconds,
    /// and current clock reading.
    ///
    /// Non-resurrection Invariant:
    /// If `is_expired(r, now, ttl)` is true, then for all `future >= now`,
    /// `is_expired(r, future, ttl)` remains strictly true.
    #[must_use]
    pub fn is_expired(
        record_ts: NonZenoTimestamp,
        current_clock: NonZenoTimestamp,
        ttl_duration_ns: u64,
    ) -> bool {
        if current_clock.epoch > record_ts.epoch {
            // New epoch: time advances across restarts
            return true;
        }
        if current_clock.epoch < record_ts.epoch {
            return false;
        }

        let expiration_deadline = record_ts.monotonic_tick_ns.saturating_add(ttl_duration_ns);
        current_clock.monotonic_tick_ns >= expiration_deadline
    }
}
