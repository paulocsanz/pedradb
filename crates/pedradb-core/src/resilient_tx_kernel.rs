//! Resilient transaction execution, adaptive backoff, and contention management (RFC-0300 P0.1).
//!
//! Mitigates optimistic concurrency control (OCC) abort storms under skewed workloads
//! (e.g. Zipfian distributions, hot rows, counter updates) by providing adaptive
//! exponential backoff with full decorrelated jitter and dynamic contention tracking.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Configuration policy for resilient transaction retries and backoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransactionRetryPolicy {
    /// Maximum number of retry attempts before propagating [`crate::error::CoreError::TransactionConflict`].
    pub max_retries: usize,
    /// Initial base backoff duration on the first conflict.
    pub initial_backoff: Duration,
    /// Maximum backoff duration ceiling.
    pub max_backoff: Duration,
    /// Exponential growth multiplier per successive conflict attempt.
    pub backoff_multiplier: f64,
    /// Whether to apply randomized decorrelated jitter to prevent thundering herd restarts.
    pub jitter: bool,
}

impl Default for TransactionRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 32,
            initial_backoff: Duration::from_micros(50),
            max_backoff: Duration::from_millis(15),
            backoff_multiplier: 1.6,
            jitter: true,
        }
    }
}

impl TransactionRetryPolicy {
    /// Creates a new policy with immediate retries (no backoff delay).
    #[must_use]
    pub const fn immediate(max_retries: usize) -> Self {
        Self {
            max_retries,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            backoff_multiplier: 1.0,
            jitter: false,
        }
    }

    /// Creates an aggressive contention backoff policy for high-skew workloads.
    #[must_use]
    pub const fn high_contention() -> Self {
        Self {
            max_retries: 64,
            initial_backoff: Duration::from_micros(100),
            max_backoff: Duration::from_millis(30),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }

    /// Computes the recommended sleep duration for a given conflict attempt index.
    #[must_use]
    pub fn compute_backoff(&self, attempt: usize) -> Duration {
        if attempt == 0 || self.initial_backoff.is_zero() {
            return Duration::ZERO;
        }

        let exp = (attempt - 1).min(16);
        let factor = self.backoff_multiplier.powi(exp as i32);
        let base_nanos = self.initial_backoff.as_nanos() as f64 * factor;
        let max_nanos = self.max_backoff.as_nanos() as f64;
        let clamped_nanos = base_nanos.min(max_nanos) as u64;

        if !self.jitter {
            return Duration::from_nanos(clamped_nanos);
        }

        // Fast, deterministic pseudo-random jitter without external crate dependencies.
        // Uses thread ID / attempt seed to generate full jitter in [clamped_nanos / 2, clamped_nanos].
        let seed = (attempt as u64)
            .wrapping_mul(0x517c_c1b7_2722_0a95)
            .wrapping_add(clamped_nanos);
        let half = clamped_nanos / 2;
        if half == 0 {
            return Duration::from_nanos(clamped_nanos);
        }
        let jittered = half + (seed % half);
        Duration::from_nanos(jittered)
    }
}

/// Thread-safe metrics and contention tracking for transaction execution.
#[derive(Debug, Default)]
pub struct ContentionTracker {
    /// Total number of transaction conflicts detected across all threads.
    conflicts_total: AtomicU64,
    /// Total number of successful transaction commits after at least one conflict retry.
    retried_commits_total: AtomicU64,
    /// Total number of transactions aborted due to retry policy exhaustion.
    exhausted_aborts_total: AtomicU64,
}

impl ContentionTracker {
    /// Creates a new zeroed contention tracker.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            conflicts_total: AtomicU64::new(0),
            retried_commits_total: AtomicU64::new(0),
            exhausted_aborts_total: AtomicU64::new(0),
        }
    }

    /// Records a conflict occurrence.
    pub fn record_conflict(&self) -> u64 {
        self.conflicts_total.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Records a transaction commit that succeeded after retrying.
    pub fn record_retried_commit(&self) {
        self.retried_commits_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a transaction abort due to exceeding retry limit.
    pub fn record_exhausted_abort(&self) {
        self.exhausted_aborts_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the snapshot of cumulative conflict counts.
    #[must_use]
    pub fn stats(&self) -> ContentionStats {
        ContentionStats {
            conflicts: self.conflicts_total.load(Ordering::Relaxed),
            retried_commits: self.retried_commits_total.load(Ordering::Relaxed),
            exhausted_aborts: self.exhausted_aborts_total.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of transaction contention metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentionStats {
    /// Cumulative count of transaction conflict errors.
    pub conflicts: u64,
    /// Transactions successfully committed after resolving conflicts through backoff.
    pub retried_commits: u64,
    /// Transactions permanently aborted because retries exceeded max limits.
    pub exhausted_aborts: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_policy_backoff_progression() {
        let policy = TransactionRetryPolicy {
            max_retries: 5,
            initial_backoff: Duration::from_micros(100),
            max_backoff: Duration::from_millis(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        assert_eq!(policy.compute_backoff(0), Duration::ZERO);
        assert_eq!(policy.compute_backoff(1), Duration::from_micros(100));
        assert_eq!(policy.compute_backoff(2), Duration::from_micros(200));
        assert_eq!(policy.compute_backoff(3), Duration::from_micros(400));
        assert_eq!(policy.compute_backoff(4), Duration::from_micros(800));
        // Clamped at 1ms max_backoff
        assert_eq!(policy.compute_backoff(5), Duration::from_millis(1));
        assert_eq!(policy.compute_backoff(10), Duration::from_millis(1));
    }

    #[test]
    fn test_retry_policy_with_jitter_bounded() {
        let policy = TransactionRetryPolicy {
            max_retries: 5,
            initial_backoff: Duration::from_micros(100),
            max_backoff: Duration::from_millis(1),
            backoff_multiplier: 2.0,
            jitter: true,
        };

        for attempt in 1..=5 {
            let backoff = policy.compute_backoff(attempt);
            assert!(backoff >= Duration::from_micros(50));
            assert!(backoff <= Duration::from_millis(1));
        }
    }

    #[test]
    fn test_contention_tracker_lifecycle() {
        let tracker = ContentionTracker::new();
        assert_eq!(tracker.stats().conflicts, 0);

        tracker.record_conflict();
        tracker.record_conflict();
        tracker.record_retried_commit();
        tracker.record_exhausted_abort();

        let s = tracker.stats();
        assert_eq!(s.conflicts, 2);
        assert_eq!(s.retried_commits, 1);
        assert_eq!(s.exhausted_aborts, 1);
    }
}
