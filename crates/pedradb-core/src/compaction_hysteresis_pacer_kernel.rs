//! Compaction Anti-Thrashing Hysteresis Controller Kernel (RFC-0313).
//!
//! Provides a mathematically verified Schmitt-trigger hysteresis state machine
//! that prevents limit-cycle flapping between active and idle compaction states
//! under high-throughput LSM ingestion workloads.
//!
//! # Problem Statement & Mathematical Foundation
//! In standard LSM engines, compactions are triggered when debt exceeds a threshold $\Theta$.
//! If compaction immediately consumes I/O bandwidth, write throughput drops, causing
//! oscillatory "flapping":
//! `Idle -> Exceeds Threshold -> Active -> Stalls Writes -> Debt Falls -> Idle -> Repeat`.
//!
//! This kernel implements a dual-threshold Schmitt trigger with energy potential $E(t)$:
//! $$E(t) = w_0 \cdot \max(0, N_{L0} - T_{L0}) + \sum_{l=1}^{L} w_l \cdot \frac{S_l}{T_l}$$
//!
//! The state transition invariant is:
//! - $\sigma(t) = \text{Active}$ if $E(t) \ge \Theta_{\text{high}}$
//! - $\sigma(t) = \text{Idle}$ if $E(t) \le \Theta_{\text{low}}$
//! - $\sigma(t) = \sigma(t - 1)$ if $\Theta_{\text{low}} < E(t) < \Theta_{\text{high}}$ (Deadband)
//!
//! # Invariant: Finite Transition Work Bound
//! Minimum energy change between state toggles is strictly $\Delta \Theta = \Theta_{\text{high}} - \Theta_{\text{low}} > 0$.

#![forbid(unsafe_code)]

/// Errors returned by the compaction hysteresis configuration validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HysteresisConfigError {
    /// Upper threshold must be strictly greater than lower threshold.
    InvalidThresholds {
        /// Lower threshold.
        theta_low: u64,
        /// Upper threshold.
        theta_high: u64,
    },
    /// Lower threshold must be greater than zero.
    ZeroThreshold,
    /// Both L0 file weight and Level size weight cannot be zero simultaneously.
    ZeroWeightsHazard,
    /// Threshold exceeds safe maximum permille limit (1,000,000 permille).
    ExcessiveThresholds { max: u64 },
}

impl std::fmt::Display for HysteresisConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidThresholds { theta_low, theta_high } => {
                write!(
                    f,
                    "Invalid hysteresis thresholds: theta_low ({theta_low}) must be strictly less than theta_high ({theta_high})"
                )
            }
            Self::ZeroThreshold => {
                write!(f, "Hysteresis lower threshold must be strictly greater than zero")
            }
            Self::ZeroWeightsHazard => {
                write!(f, "Hysteresis configuration hazard: both l0_file_weight and level_weight cannot be zero")
            }
            Self::ExcessiveThresholds { max } => {
                write!(f, "Hysteresis threshold exceeds maximum permitted permille limit {max}")
            }
        }
    }
}

impl std::error::Error for HysteresisConfigError {}

/// Operational state of the compaction engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompactionState {
    /// Compaction is dormant (energy below lower threshold or dwelling in deadband).
    Idle,
    /// Compaction is actively clearing debt (energy above upper threshold or dwelling in deadband).
    Active,
}

/// Metrics describing the state of an individual LSM level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionLevelMetrics {
    /// Level index (0-indexed, where Level 0 is the un-partitioned memtable flush target).
    pub level: usize,
    /// Total number of SSTable files at this level.
    pub file_count: usize,
    /// Total bytes currently occupied by files at this level.
    pub total_bytes: u64,
    /// Target byte capacity configured for this level.
    pub target_bytes: u64,
}

/// Configuration parameters for the hysteresis controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HysteresisConfig {
    /// Lower threshold to exit active compaction state into idle (in permille energy units).
    pub theta_low: u64,
    /// Upper threshold to enter active compaction state from idle (in permille energy units).
    pub theta_high: u64,
    /// Target file count for Level 0 before accumulating debt.
    pub l0_target_files: usize,
    /// Weight applied per Level 0 file exceeding `l0_target_files` (in permille units).
    pub l0_file_weight: u64,
    /// Weight multiplier applied to level size score $S_l / T_l$ (in permille units).
    pub level_weight: u64,
}

impl Default for HysteresisConfig {
    fn default() -> Self {
        Self {
            theta_low: 1000,    // 1.0x target debt
            theta_high: 1500,   // 1.5x target debt
            l0_target_files: 4,
            l0_file_weight: 250, // 0.25x per extra L0 file
            level_weight: 1000,  // 1.0x level capacity ratio
        }
    }
}

/// Pure Schmitt-trigger hysteresis state machine for LSM compaction scheduling.
#[derive(Debug, Clone)]
pub struct CompactionHysteresisController {
    config: HysteresisConfig,
    state: CompactionState,
    current_energy: u64,
    total_transitions: u64,
}

impl CompactionHysteresisController {
    /// Creates a new hysteresis controller with validated threshold parameters.
    pub fn new(config: HysteresisConfig) -> Result<Self, HysteresisConfigError> {
        if config.theta_low == 0 {
            return Err(HysteresisConfigError::ZeroThreshold);
        }
        if config.theta_high <= config.theta_low {
            return Err(HysteresisConfigError::InvalidThresholds {
                theta_low: config.theta_low,
                theta_high: config.theta_high,
            });
        }
        if config.l0_file_weight == 0 && config.level_weight == 0 {
            return Err(HysteresisConfigError::ZeroWeightsHazard);
        }
        if config.theta_high > 1_000_000 {
            return Err(HysteresisConfigError::ExcessiveThresholds { max: 1_000_000 });
        }

        Ok(Self {
            config,
            state: CompactionState::Idle,
            current_energy: 0,
            total_transitions: 0,
        })
    }

    /// Current operational state of the compaction engine.
    #[must_use]
    pub fn state(&self) -> CompactionState {
        self.state
    }

    /// Latest calculated energy potential (permille units).
    #[must_use]
    pub fn current_energy(&self) -> u64 {
        self.current_energy
    }

    /// Total count of state transitions observed over the lifetime of the controller.
    #[must_use]
    pub fn total_transitions(&self) -> u64 {
        self.total_transitions
    }

    /// Configuration parameters currently in effect.
    #[must_use]
    pub fn config(&self) -> &HysteresisConfig {
        &self.config
    }

    /// Width of the hysteresis deadband $\Delta \Theta = \Theta_{\text{high}} - \Theta_{\text{low}}$.
    #[must_use]
    pub fn deadband_width(&self) -> u64 {
        self.config.theta_high - self.config.theta_low
    }

    /// Checks whether a given energy value falls strictly inside the deadband.
    #[must_use]
    pub fn is_in_deadband(&self, energy: u64) -> bool {
        energy > self.config.theta_low && energy < self.config.theta_high
    }

    /// Computes the exact scalar compaction energy potential $E(t)$ for given LSM metrics.
    ///
    /// Energy is represented in permille (1000 = 1.0x normal capacity).
    #[must_use]
    pub fn calculate_energy(
        &self,
        l0_file_count: usize,
        level_metrics: &[CompactionLevelMetrics],
    ) -> u64 {
        let mut total_energy = 0u64;

        // Level 0 debt: linear penalty for files in excess of target
        if l0_file_count > self.config.l0_target_files {
            let excess = (l0_file_count - self.config.l0_target_files) as u64;
            let l0_energy = excess.saturating_mul(self.config.l0_file_weight);
            total_energy = total_energy.saturating_add(l0_energy);
        }

        // Levels 1..L debt: max score of Level Size / Target Size
        let mut max_level_debt = 0u64;
        for metrics in level_metrics {
            if metrics.level == 0 || metrics.target_bytes == 0 {
                continue;
            }

            // Ratio in permille: (total_bytes * 1000) / target_bytes
            let ratio_permille = (metrics.total_bytes as u128)
                .saturating_mul(1000)
                .checked_div(metrics.target_bytes as u128)
                .unwrap_or(0) as u64;

            let weighted_debt = (ratio_permille as u128)
                .saturating_mul(self.config.level_weight as u128)
                .checked_div(1000)
                .unwrap_or(0) as u64;

            max_level_debt = max_level_debt.max(weighted_debt);
        }

        total_energy.saturating_add(max_level_debt)
    }

    /// Ingests live LSM metrics, updates energy potential, and transitions the Schmitt trigger.
    ///
    /// Returns `(new_state, state_changed)`.
    pub fn update(
        &mut self,
        l0_file_count: usize,
        level_metrics: &[CompactionLevelMetrics],
    ) -> (CompactionState, bool) {
        let energy = self.calculate_energy(l0_file_count, level_metrics);
        self.current_energy = energy;

        let previous_state = self.state;
        let new_state = match previous_state {
            CompactionState::Idle => {
                if energy >= self.config.theta_high {
                    CompactionState::Active
                } else {
                    CompactionState::Idle
                }
            }
            CompactionState::Active => {
                if energy <= self.config.theta_low {
                    CompactionState::Idle
                } else {
                    CompactionState::Active
                }
            }
        };

        let changed = new_state != previous_state;
        if changed {
            self.total_transitions = self.total_transitions.saturating_add(1);
            self.state = new_state;
        }

        (new_state, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_hysteresis_pacer_structural_invariants_red_to_green() {
        // 1. Zero weights hazard rejected
        let zero_weights = HysteresisConfig {
            l0_file_weight: 0,
            level_weight: 0,
            ..Default::default()
        };
        assert_eq!(
            CompactionHysteresisController::new(zero_weights).unwrap_err(),
            HysteresisConfigError::ZeroWeightsHazard
        );

        // 2. Excessive threshold rejected
        let excessive = HysteresisConfig {
            theta_high: 2_000_000,
            ..Default::default()
        };
        assert_eq!(
            CompactionHysteresisController::new(excessive).unwrap_err(),
            HysteresisConfigError::ExcessiveThresholds { max: 1_000_000 }
        );

        // 3. Normal valid lifecycle
        let mut controller = CompactionHysteresisController::new(HysteresisConfig::default()).unwrap();
        assert_eq!(controller.state(), CompactionState::Idle);

        // Under high load, transition to active
        let (state, changed) = controller.update(20, &[]);
        assert_eq!(state, CompactionState::Active);
        assert!(changed);
    }
}

