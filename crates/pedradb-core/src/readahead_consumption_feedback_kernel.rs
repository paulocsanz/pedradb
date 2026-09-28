//! RFC-0290: Readahead Window Contraction and Consumption Feedback Invariant Kernel.
//!
//! Models adaptive prefetching as a feedback-controlled stochastic automaton:
//! rho = BytesConsumed / BytesPrefetched.
//! If rho < rho_contract => W_{n+1} = max(W_min, floor(W_n / 2)).
//! If rho >= rho_expand  => W_{n+1} = min(W_max, 2 * W_n).
//! Mathematically guarantees asymptotic O(1) read amplification on short/aborted scans.

#![forbid(unsafe_code)]

/// Configuration parameters for adaptive readahead control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadaheadFeedbackPolicy {
    /// Minimum prefetch window size in bytes (e.g. 4 KiB).
    pub min_window_bytes: usize,
    /// Maximum prefetch window size in bytes (e.g. 2 MiB).
    pub max_window_bytes: usize,
    /// Consumption ratio threshold triggering geometric expansion (e.g. 0.85 = 85%).
    pub expand_threshold: f64,
    /// Consumption ratio threshold triggering geometric contraction (e.g. 0.30 = 30%).
    pub contract_threshold: f64,
}

impl Default for ReadaheadFeedbackPolicy {
    fn default() -> Self {
        Self {
            min_window_bytes: 4096,           // 4 KiB
            max_window_bytes: 2 * 1024 * 1024, // 2 MiB
            expand_threshold: 0.85,
            contract_threshold: 0.30,
        }
    }
}

/// Adaptive readahead controller tracking consumption efficiency.
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptiveReadaheadController {
    policy: ReadaheadFeedbackPolicy,
    current_window_bytes: usize,
    epoch_prefetched_bytes: usize,
    epoch_consumed_bytes: usize,
}

impl AdaptiveReadaheadController {
    /// Creates a new readahead controller initialized at minimum window size.
    #[must_use]
    pub fn new(policy: ReadaheadFeedbackPolicy) -> Self {
        assert!(policy.min_window_bytes > 0);
        assert!(policy.min_window_bytes <= policy.max_window_bytes);
        assert!(policy.contract_threshold <= policy.expand_threshold);
        let current_window_bytes = policy.min_window_bytes;
        Self {
            policy,
            current_window_bytes,
            epoch_prefetched_bytes: 0,
            epoch_consumed_bytes: 0,
        }
    }

    /// Returns the currently recommended prefetch window size.
    #[must_use]
    pub fn current_window(&self) -> usize {
        self.current_window_bytes
    }

    /// Records that `bytes` were prefetched into the read buffer.
    pub fn record_prefetch(&mut self, bytes: usize) {
        self.epoch_prefetched_bytes = self.epoch_prefetched_bytes.saturating_add(bytes);
    }

    /// Records that `bytes` were actively consumed by the caller iterator.
    pub fn record_consumption(&mut self, bytes: usize) {
        self.epoch_consumed_bytes = self.epoch_consumed_bytes.saturating_add(bytes);
    }

    /// Evaluates consumption efficiency rho and adjusts the prefetch window for the next epoch.
    /// Resets the epoch counters and returns the new window size.
    pub fn evaluate_and_adapt(&mut self) -> usize {
        if self.epoch_prefetched_bytes == 0 {
            return self.current_window_bytes;
        }

        let rho = (self.epoch_consumed_bytes as f64) / (self.epoch_prefetched_bytes as f64);

        if rho >= self.policy.expand_threshold {
            // High utilization: expand window geometrically
            self.current_window_bytes = (self.current_window_bytes.saturating_mul(2))
                .min(self.policy.max_window_bytes);
        } else if rho < self.policy.contract_threshold {
            // Low utilization: contract window to prevent I/O bloat and cache pollution
            self.current_window_bytes = (self.current_window_bytes / 2)
                .max(self.policy.min_window_bytes);
        }

        // Reset epoch counters for next observation cycle
        self.epoch_prefetched_bytes = 0;
        self.epoch_consumed_bytes = 0;

        self.current_window_bytes
    }
}
