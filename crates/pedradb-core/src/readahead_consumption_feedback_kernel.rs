//! RFC-0290: Readahead Window Contraction and Consumption Feedback Invariant Kernel.
//!
//! Models adaptive prefetching as a feedback-controlled stochastic automaton:
//! rho = BytesConsumed / BytesPrefetched.
//! If rho < rho_contract => W_{n+1} = max(W_min, floor(W_n / 2)).
//! If rho >= rho_expand  => W_{n+1} = min(W_max, 2 * W_n).
//! Mathematically guarantees asymptotic O(1) read amplification on short/aborted scans.

#![forbid(unsafe_code)]

/// Errors resulting from invalid adaptive readahead policies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReadaheadPolicyError {
    /// Minimum prefetch window cannot be zero bytes.
    ZeroMinWindow,
    /// Minimum prefetch window cannot exceed maximum prefetch window.
    MinExceedsMaxWindow {
        /// Configured minimum window.
        min: usize,
        /// Configured maximum window.
        max: usize,
    },
    /// Thresholds must be finite and within [0.0, 1.0].
    InvalidThresholds {
        /// Contract threshold.
        contract: f64,
        /// Expand threshold.
        expand: f64,
    },
    /// Contract threshold cannot exceed expand threshold.
    ContractExceedsExpand {
        /// Contract threshold.
        contract: f64,
        /// Expand threshold.
        expand: f64,
    },
}

impl std::fmt::Display for ReadaheadPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroMinWindow => write!(f, "min_window_bytes must be > 0"),
            Self::MinExceedsMaxWindow { min, max } => {
                write!(f, "min_window_bytes ({min}) cannot exceed max_window_bytes ({max})")
            }
            Self::InvalidThresholds { contract, expand } => {
                write!(f, "Thresholds must be finite in [0.0, 1.0]: contract={contract}, expand={expand}")
            }
            Self::ContractExceedsExpand { contract, expand } => {
                write!(f, "contract_threshold ({contract}) cannot exceed expand_threshold ({expand})")
            }
        }
    }
}

impl std::error::Error for ReadaheadPolicyError {}

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

impl ReadaheadFeedbackPolicy {
    /// Validates and constructs an adaptive readahead policy.
    pub fn try_new(
        min_window_bytes: usize,
        max_window_bytes: usize,
        expand_threshold: f64,
        contract_threshold: f64,
    ) -> Result<Self, ReadaheadPolicyError> {
        if min_window_bytes == 0 {
            return Err(ReadaheadPolicyError::ZeroMinWindow);
        }
        if min_window_bytes > max_window_bytes {
            return Err(ReadaheadPolicyError::MinExceedsMaxWindow {
                min: min_window_bytes,
                max: max_window_bytes,
            });
        }
        if contract_threshold.is_nan()
            || !contract_threshold.is_finite()
            || !(0.0..=1.0).contains(&contract_threshold)
            || expand_threshold.is_nan()
            || !expand_threshold.is_finite()
            || !(0.0..=1.0).contains(&expand_threshold)
        {
            return Err(ReadaheadPolicyError::InvalidThresholds {
                contract: contract_threshold,
                expand: expand_threshold,
            });
        }
        if contract_threshold > expand_threshold {
            return Err(ReadaheadPolicyError::ContractExceedsExpand {
                contract: contract_threshold,
                expand: expand_threshold,
            });
        }
        Ok(Self {
            min_window_bytes,
            max_window_bytes,
            expand_threshold,
            contract_threshold,
        })
    }
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
    /// Safely creates a new readahead controller, validating policy parameters.
    pub fn try_new(policy: ReadaheadFeedbackPolicy) -> Result<Self, ReadaheadPolicyError> {
        ReadaheadFeedbackPolicy::try_new(
            policy.min_window_bytes,
            policy.max_window_bytes,
            policy.expand_threshold,
            policy.contract_threshold,
        )?;
        let current_window_bytes = policy.min_window_bytes;
        Ok(Self {
            policy,
            current_window_bytes,
            epoch_prefetched_bytes: 0,
            epoch_consumed_bytes: 0,
        })
    }

    /// Creates a new readahead controller initialized at minimum window size.
    #[must_use]
    pub fn new(policy: ReadaheadFeedbackPolicy) -> Self {
        Self::try_new(policy).expect("valid readahead policy")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_min_window_rejected() {
        let err = ReadaheadFeedbackPolicy::try_new(0, 1024, 0.8, 0.2).unwrap_err();
        assert_eq!(err, ReadaheadPolicyError::ZeroMinWindow);
    }

    #[test]
    fn test_min_exceeds_max_window_rejected() {
        let err = ReadaheadFeedbackPolicy::try_new(2048, 1024, 0.8, 0.2).unwrap_err();
        assert_eq!(
            err,
            ReadaheadPolicyError::MinExceedsMaxWindow {
                min: 2048,
                max: 1024,
            }
        );
    }

    #[test]
    fn test_nan_or_out_of_range_thresholds_rejected() {
        let err_nan = ReadaheadFeedbackPolicy::try_new(1024, 4096, f64::NAN, 0.2).unwrap_err();
        assert!(matches!(err_nan, ReadaheadPolicyError::InvalidThresholds { .. }));

        let err_oor = ReadaheadFeedbackPolicy::try_new(1024, 4096, 1.5, 0.2).unwrap_err();
        assert!(matches!(err_oor, ReadaheadPolicyError::InvalidThresholds { .. }));
    }

    #[test]
    fn test_contract_exceeds_expand_rejected() {
        let err = ReadaheadFeedbackPolicy::try_new(1024, 4096, 0.2, 0.8).unwrap_err();
        assert_eq!(
            err,
            ReadaheadPolicyError::ContractExceedsExpand {
                contract: 0.8,
                expand: 0.2,
            }
        );
    }
}

