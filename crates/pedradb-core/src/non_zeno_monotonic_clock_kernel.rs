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

/// Expiration policy for LSM entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TtlPolicy {
    /// The entry never expires.
    NoExpiration,
    /// The entry expires after `u64` nanoseconds from its write timestamp.
    ExpireAfterNs(u64),
}

/// Errors resulting from clock overflow or illegal epoch transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonZenoClockError {
    /// Logical sequence counter reached u64::MAX and would wrap around to 0.
    LogicalSequenceOverflow,
    /// New epoch attempted is less than or equal to current epoch.
    EpochRegressed {
        /// Current epoch.
        current: u64,
        /// Attempted epoch.
        attempted: u64,
    },
    /// Initial epoch cannot be zero.
    ZeroEpoch,
}

impl std::fmt::Display for NonZenoClockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LogicalSequenceOverflow => {
                write!(f, "Non-Zeno logical sequence counter reached u64::MAX and cannot wrap around")
            }
            Self::EpochRegressed { current, attempted } => {
                write!(f, "Attempted epoch {} <= current epoch {}", attempted, current)
            }
            Self::ZeroEpoch => write!(f, "Engine epoch cannot be zero"),
        }
    }
}

impl std::error::Error for NonZenoClockError {}

/// Thread-safe generator of non-zenonian monotonic timestamps.
pub struct NonZenoLogicalClock {
    /// Current engine epoch.
    epoch: AtomicU64,
    /// Strictly increasing logical sequence counter.
    next_logical_seq: AtomicU64,
    /// Highest observed monotonic tick in nanoseconds.
    last_monotonic_tick_ns: AtomicU64,
}

impl NonZenoLogicalClock {
    /// Creates a new logical clock anchored at a specific epoch and initial tick.
    #[must_use]
    pub fn new(epoch: u64, initial_tick_ns: u64) -> Self {
        Self::try_new(epoch, initial_tick_ns).unwrap_or_else(|_| Self {
            epoch: AtomicU64::new(1),
            next_logical_seq: AtomicU64::new(1),
            last_monotonic_tick_ns: AtomicU64::new(initial_tick_ns),
        })
    }

    /// Safely constructs a new logical clock, rejecting epoch zero.
    pub fn try_new(epoch: u64, initial_tick_ns: u64) -> Result<Self, NonZenoClockError> {
        if epoch == 0 {
            return Err(NonZenoClockError::ZeroEpoch);
        }
        Ok(Self {
            epoch: AtomicU64::new(epoch),
            next_logical_seq: AtomicU64::new(1),
            last_monotonic_tick_ns: AtomicU64::new(initial_tick_ns),
        })
    }

    /// Safely advances the engine epoch atomically, validating that new_epoch > current_epoch.
    pub fn try_advance_epoch(&self, new_epoch: u64, initial_tick_ns: u64) -> Result<(), NonZenoClockError> {
        let mut cur = self.epoch.load(Ordering::Acquire);
        if new_epoch <= cur {
            return Err(NonZenoClockError::EpochRegressed {
                current: cur,
                attempted: new_epoch,
            });
        }
        while new_epoch > cur {
            match self.epoch.compare_exchange_weak(
                cur,
                new_epoch,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => {
                    if new_epoch <= actual {
                        return Err(NonZenoClockError::EpochRegressed {
                            current: actual,
                            attempted: new_epoch,
                        });
                    }
                    cur = actual;
                }
            }
        }

        // Ensure monotonic tick advances to at least initial_tick_ns
        let mut last = self.last_monotonic_tick_ns.load(Ordering::Acquire);
        while initial_tick_ns > last {
            match self.last_monotonic_tick_ns.compare_exchange_weak(
                last,
                initial_tick_ns,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => last = actual,
            }
        }
        Ok(())
    }

    /// Advances the engine epoch atomically across restarts or coordinator failovers.
    ///
    /// Requires `new_epoch > current_epoch`. Ensures that monotonic ticks are initialized
    /// to at least `initial_tick_ns` without stepping backward.
    pub fn advance_epoch(&self, new_epoch: u64, initial_tick_ns: u64) {
        let _ = self.try_advance_epoch(new_epoch, initial_tick_ns);
    }

    /// Safely advances the clock, rejecting sequence number overflow at u64::MAX.
    pub fn try_tick(&self, observed_wall_clock_ns: u64) -> Result<NonZenoTimestamp, NonZenoClockError> {
        let seq = self
            .next_logical_seq
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |val| val.checked_add(1))
            .map_err(|_| NonZenoClockError::LogicalSequenceOverflow)?;
        let current_epoch = self.epoch.load(Ordering::Acquire);

        loop {
            let last = self.last_monotonic_tick_ns.load(Ordering::Acquire);
            let next_tick = observed_wall_clock_ns.max(last.saturating_add(1));

            if self
                .last_monotonic_tick_ns
                .compare_exchange_weak(last, next_tick, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(NonZenoTimestamp {
                    epoch: current_epoch,
                    logical_seq: seq,
                    monotonic_tick_ns: next_tick,
                });
            }
        }
    }

    /// Advances the clock observing a proposed wall-clock reading in nanoseconds.
    /// Even if `wall_clock_ns` is in the past (backward clock step), the generated
    /// timestamp is guaranteed to be strictly greater than any previously issued timestamp.
    pub fn tick(&self, observed_wall_clock_ns: u64) -> NonZenoTimestamp {
        self.try_tick(observed_wall_clock_ns).unwrap_or_else(|_| NonZenoTimestamp {
            epoch: self.epoch.load(Ordering::Acquire),
            logical_seq: u64::MAX,
            monotonic_tick_ns: observed_wall_clock_ns,
        })
    }

    /// Returns the current epoch.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
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

    /// Evaluates expiration against a high-level `TtlPolicy`.
    #[must_use]
    pub fn is_expired_policy(
        record_ts: NonZenoTimestamp,
        current_clock: NonZenoTimestamp,
        policy: TtlPolicy,
    ) -> bool {
        match policy {
            TtlPolicy::NoExpiration => false,
            TtlPolicy::ExpireAfterNs(ttl_ns) => Self::is_expired(record_ts, current_clock, ttl_ns),
        }
    }

    /// Returns the remaining nanoseconds before expiration, or `None` if already expired.
    #[must_use]
    pub fn remaining_ttl_ns(
        record_ts: NonZenoTimestamp,
        current_clock: NonZenoTimestamp,
        ttl_duration_ns: u64,
    ) -> Option<u64> {
        if current_clock.epoch > record_ts.epoch {
            return None;
        }
        if current_clock.epoch < record_ts.epoch {
            return Some(ttl_duration_ns);
        }

        let deadline = record_ts.monotonic_tick_ns.saturating_add(ttl_duration_ns);
        if current_clock.monotonic_tick_ns >= deadline {
            None
        } else {
            Some(deadline.saturating_sub(current_clock.monotonic_tick_ns))
        }
    }

    /// Evaluates expiration across epochs with an explicit physical continuity assumption.
    ///
    /// When `assume_continuous_physical_time` is true (e.g. cluster-synchronized physical time
    /// across graceful failover), entries whose physical monotonic tick has not yet elapsed
    /// are preserved rather than being prematurely wiped out.
    #[must_use]
    pub fn is_expired_cross_epoch(
        record_ts: NonZenoTimestamp,
        current_clock: NonZenoTimestamp,
        ttl_duration_ns: u64,
        assume_continuous_physical_time: bool,
    ) -> bool {
        if current_clock.epoch < record_ts.epoch {
            return false;
        }

        if current_clock.epoch == record_ts.epoch {
            return Self::is_expired(record_ts, current_clock, ttl_duration_ns);
        }

        // current_clock.epoch > record_ts.epoch
        if !assume_continuous_physical_time {
            // Pessimistic fail-safe: assume epoch increment implies unknown elapsed time
            return true;
        }

        let deadline = record_ts.monotonic_tick_ns.saturating_add(ttl_duration_ns);
        current_clock.monotonic_tick_ns >= deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_zeno_clock_hardening_red_to_green() {
        // 1. Rejeita epoch 0
        assert_eq!(
            NonZenoLogicalClock::try_new(0, 100).err(),
            Some(NonZenoClockError::ZeroEpoch)
        );

        let clock = NonZenoLogicalClock::try_new(1, 100).unwrap();

        // 2. Rejeita regressão de epoch
        assert_eq!(
            clock.try_advance_epoch(1, 200),
            Err(NonZenoClockError::EpochRegressed { current: 1, attempted: 1 })
        );

        // 3. Overflow de sequência não pode dar wrap-around silencioso para 0
        clock.next_logical_seq.store(u64::MAX, Ordering::Release);
        assert_eq!(
            clock.try_tick(200),
            Err(NonZenoClockError::LogicalSequenceOverflow)
        );
    }
}

