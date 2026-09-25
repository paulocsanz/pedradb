//! RFC-0280 P1.2 — Hybrid Logical Clock & Monotonic Time Horizon Kernel.
//!
//! Formalizes a monotonic time horizon protecting Time-To-Live (TTL) compactions
//! and snapshot expirations against physical wall-clock retrogressions (NTP steps, leap seconds).
//! Proves strict time monotonicity: $H_{now} = \max(\text{PhysicalWallClock}, H_{prev} + 1)$.

#![forbid(unsafe_code)]

/// Monotonic hybrid clock tracking the logical time horizon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonotonicClock {
    /// Highest logical timestamp ever emitted (in nanoseconds or epoch units).
    pub max_logical_time: u64,
}

impl MonotonicClock {
    /// Initializes a monotonic clock at a given baseline.
    pub fn new(initial_time: u64) -> Self {
        Self {
            max_logical_time: initial_time,
        }
    }

    /// Reads the clock with a given raw physical wall-clock input.
    /// Strictly guarantees that output is monotonic: $H_{now} \ge H_{prev} + 1$,
    /// completely absorbing backward time jumps from NTP or leap seconds.
    pub fn now(&mut self, physical_wall_clock: u64) -> u64 {
        let logical_now = physical_wall_clock.max(self.max_logical_time.saturating_add(1));
        self.max_logical_time = logical_now;
        logical_now
    }

    /// Evaluates whether an entry with a TTL timestamp has expired.
    /// Uses the monotonic logical horizon, eliminating false-positive premature deletions.
    pub fn is_ttl_expired(&self, entry_created_time: u64, ttl_duration: u64) -> bool {
        let expiry_horizon = entry_created_time.saturating_add(ttl_duration);
        self.max_logical_time >= expiry_horizon
    }

    /// Verifies the Monotonic Clock Invariant:
    /// For any sequence of physical inputs, the emitted logical timestamps are strictly increasing.
    pub fn verify_monotonicity_sequence(physical_inputs: &[u64]) -> bool {
        let mut clock = MonotonicClock::new(0);
        let mut prev = 0u64;

        for &phys in physical_inputs {
            let logical = clock.now(phys);
            if logical <= prev {
                return false; // Time retrogressed or stalled!
            }
            prev = logical;
        }

        true
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new(0)
    }
}
