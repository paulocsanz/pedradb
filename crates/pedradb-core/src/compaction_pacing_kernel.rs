//! Compaction Debt Pacing and Write-Stall Prevention Kernel (RFC-0284 Pilar 3).
//!
//! Replaces catastrophic "stop-the-world" write stalls with smooth, bounded
//! backpressure pacing based on pending compaction debt.
//!
//! Guarantees:
//! 1. Write delay is strictly bounded: `Delay <= MaxDelayLimit` (e.g. <= 10 ms).
//! 2. Smooth exponential/linear ramp from 0 to max delay across `[SoftDebt, HardDebt]`.
//! 3. Deterministic pacing decision without arbitrary thread sleeps inside the storage kernel.

#![forbid(unsafe_code)]

/// Configuration parameters for compaction debt pacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacingConfig {
    /// Pending compaction bytes threshold where pacing begins.
    pub soft_debt_bytes: u64,
    /// Pending compaction bytes threshold where maximum pacing is reached.
    pub hard_debt_bytes: u64,
    /// Maximum allowed write delay in microseconds (e.g. 10_000 µs = 10 ms).
    pub max_delay_micros: u64,
    /// Base delay step in microseconds when pacing is initiated.
    pub base_delay_micros: u64,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            soft_debt_bytes: 64 * 1024 * 1024,      // 64 MiB
            hard_debt_bytes: 512 * 1024 * 1024,     // 512 MiB
            max_delay_micros: 10_000,               // 10 ms maximum pause
            base_delay_micros: 100,                 // 100 µs minimum backpressure
        }
    }
}

/// Action determined by the pacing kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacingDecision {
    /// No backpressure needed. Write proceeds with zero delay.
    NoDelay,
    /// Moderate backpressure: write delayed smoothly by `delay_micros`.
    SoftPacing {
        /// Microseconds to throttle.
        delay_micros: u64,
        /// Current pending debt in bytes.
        debt_bytes: u64,
    },
    /// Severe debt: maximum backpressure reached.
    HardPacing {
        /// Maximum allowed delay enforced.
        delay_micros: u64,
        /// Current pending debt in bytes.
        debt_bytes: u64,
    },
}

/// Engine that computes pacing decisions from live LSM compaction metrics.
pub struct CompactionPacer {
    config: PacingConfig,
}

impl CompactionPacer {
    /// Creates a new pacer with given config.
    pub fn new(config: PacingConfig) -> Self {
        assert!(
            config.soft_debt_bytes < config.hard_debt_bytes,
            "Soft debt limit must be strictly less than hard debt limit"
        );
        Self { config }
    }

    /// Evaluates write backpressure given current pending compaction debt in bytes.
    pub fn evaluate_debt(&self, pending_debt_bytes: u64) -> PacingDecision {
        if pending_debt_bytes <= self.config.soft_debt_bytes {
            return PacingDecision::NoDelay;
        }

        if pending_debt_bytes >= self.config.hard_debt_bytes {
            return PacingDecision::HardPacing {
                delay_micros: self.config.max_delay_micros,
                debt_bytes: pending_debt_bytes,
            };
        }

        // Linear interpolation between [soft_debt, hard_debt] into [base_delay, max_delay]
        let debt_range = self.config.hard_debt_bytes - self.config.soft_debt_bytes;
        let excess = pending_debt_bytes - self.config.soft_debt_bytes;

        let delay_range = self.config.max_delay_micros.saturating_sub(self.config.base_delay_micros);
        let scaled_delay = (excess as u128 * delay_range as u128) / (debt_range as u128);

        let delay_micros = (self.config.base_delay_micros as u128 + scaled_delay) as u64;
        let final_delay = delay_micros.min(self.config.max_delay_micros);

        PacingDecision::SoftPacing {
            delay_micros: final_delay,
            debt_bytes: pending_debt_bytes,
        }
    }
}
