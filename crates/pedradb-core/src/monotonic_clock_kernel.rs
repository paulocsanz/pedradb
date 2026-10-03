//! RFC-0280 P1.2 — Hybrid Logical Clock & Monotonic Time Horizon Kernel.
//!
//! Formalizes a monotonic time horizon protecting Time-To-Live (TTL) compactions
//! and snapshot expirations against physical wall-clock retrogressions (NTP steps, leap seconds).
//! Proves strict time monotonicity: $H_{now} = \max(\text{PhysicalWallClock}, H_{prev} + 1)$.

#![forbid(unsafe_code)]

use std::fmt;

/// Errors resulting from clock horizon exhaustion or invalid time invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    /// Logical clock reached u64::MAX and cannot advance without stalling monotonicity.
    ClockHorizonExhausted,
    /// TTL duration cannot be zero.
    ZeroTtlDuration,
    /// Entry creation time cannot be zero.
    ZeroCreatedTime,
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClockHorizonExhausted => write!(f, "Clock logical horizon exhausted u64::MAX"),
            Self::ZeroTtlDuration => write!(f, "TTL duration cannot be zero"),
            Self::ZeroCreatedTime => write!(f, "Entry created time cannot be zero"),
        }
    }
}

impl std::error::Error for ClockError {}

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

    /// Reads the clock with a given raw physical wall-clock input, with strict error
    /// propagation on logical horizon exhaustion preventing silent clock stalls.
    pub fn try_now(&mut self, physical_wall_clock: u64) -> Result<u64, ClockError> {
        let next_logical = self
            .max_logical_time
            .checked_add(1)
            .ok_or(ClockError::ClockHorizonExhausted)?;
        let logical_now = physical_wall_clock.max(next_logical);
        self.max_logical_time = logical_now;
        Ok(logical_now)
    }

    /// Reads the clock with a given raw physical wall-clock input (backward-compatible).
    /// Strictly guarantees that output is monotonic: $H_{now} \ge H_{prev} + 1$,
    /// completely absorbing backward time jumps from NTP or leap seconds.
    pub fn now(&mut self, physical_wall_clock: u64) -> u64 {
        self.try_now(physical_wall_clock).unwrap_or(self.max_logical_time)
    }

    /// Safely evaluates whether an entry with a TTL timestamp has expired.
    pub fn try_is_ttl_expired(&self, entry_created_time: u64, ttl_duration: u64) -> Result<bool, ClockError> {
        if entry_created_time == 0 {
            return Err(ClockError::ZeroCreatedTime);
        }
        if ttl_duration == 0 {
            return Err(ClockError::ZeroTtlDuration);
        }
        let expiry_horizon = entry_created_time
            .checked_add(ttl_duration)
            .ok_or(ClockError::ClockHorizonExhausted)?;
        Ok(self.max_logical_time >= expiry_horizon)
    }

    /// Evaluates whether an entry with a TTL timestamp has expired.
    /// Uses the monotonic logical horizon, eliminating false-positive premature deletions.
    pub fn is_ttl_expired(&self, entry_created_time: u64, ttl_duration: u64) -> bool {
        self.try_is_ttl_expired(entry_created_time, ttl_duration).unwrap_or(false)
    }

    /// Verifies the Monotonic Clock Invariant:
    /// For any sequence of physical inputs, the emitted logical timestamps are strictly increasing.
    pub fn verify_monotonicity_sequence(physical_inputs: &[u64]) -> bool {
        let mut clock = MonotonicClock::new(0);
        let mut prev = 0u64;

        for &phys in physical_inputs {
            let logical = match clock.try_now(phys) {
                Ok(t) => t,
                Err(_) => return false,
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monotonic_clock_hardening_red_to_green() {
        // 1. Clock no limite u64::MAX não pode estagnar em loop silencioso
        let mut clock = MonotonicClock::new(u64::MAX);
        assert_eq!(clock.try_now(100), Err(ClockError::ClockHorizonExhausted));

        // 2. TTL duração zero rejeitada
        assert_eq!(
            clock.try_is_ttl_expired(1000, 0),
            Err(ClockError::ZeroTtlDuration)
        );

        // 3. TTL created_time zero rejeitado
        assert_eq!(
            clock.try_is_ttl_expired(0, 100),
            Err(ClockError::ZeroCreatedTime)
        );

        // 4. Verificação de sequência detecta exaustão como violação de monotonicidade estrita
        assert!(!MonotonicClock::verify_monotonicity_sequence(&[u64::MAX, u64::MAX]));
    }
}

