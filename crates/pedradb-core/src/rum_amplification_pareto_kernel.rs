//! RUM Conjecture Pareto Boundary and IOPS Budget Kernel (RFC-0286 Fronteira 3).
//!
//! Enforces mathematical bounds on Read, Write, and Space amplification (RUM Conjecture)
//! and guarantees reserved read IOPS headroom under saturation write bursts.
//!
//! Guarantees:
//! 1. Read amplification upper bound: `WorstCaseReadAmp <= 1 + sum(FPR_l)`.
//! 2. Write amplification upper bound: `WorstCaseWriteAmp <= LevelRatio * NumLevels`.
//! 3. Fair-share I/O bandwidth reservation: Read IOPS floor is protected from write monopolization.

#![forbid(unsafe_code)]

/// Configuration governing LSM amplification and hardware budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumBudgetConfig {
    /// Number of levels in the LSM tree (L_max).
    pub num_levels: u32,
    /// Size amplification ratio between consecutive levels (T).
    pub level_ratio: u32,
    /// Bloom filter false positive rate expressed in permille (e.g. 10 = 1%).
    pub bloom_fpr_permille: u32,
    /// Total device IOPS budget capacity (e.g. 100_000 IOPS).
    pub total_device_iops: u64,
    /// Minimum reserved percentage of IOPS dedicated to client reads (e.g. 30%).
    pub reserved_read_iops_percent: u32,
}

impl Default for RumBudgetConfig {
    fn default() -> Self {
        Self {
            num_levels: 7,
            level_ratio: 10,
            bloom_fpr_permille: 10, // 1%
            total_device_iops: 100_000,
            reserved_read_iops_percent: 30, // 30,000 IOPS guaranteed for reads
        }
    }
}

/// Calculated theoretical amplification bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TheoreticalAmplificationBounds {
    /// Maximum worst-case write amplification factor.
    pub max_write_amp: u64,
    /// Maximum expected read amplification (disk reads per point lookup) in permille.
    pub max_read_amp_permille: u64,
    /// Minimum guaranteed read IOPS floor.
    pub reserved_read_iops: u64,
    /// Maximum allowable background compaction IOPS ceiling.
    pub max_compaction_iops: u64,
}

/// Evaluator and runtime regulator for RUM bounds.
pub struct RumParetoEvaluator {
    config: RumBudgetConfig,
}

impl RumParetoEvaluator {
    /// Creates a new evaluator with given configuration.
    pub fn new(config: RumBudgetConfig) -> Self {
        Self { config }
    }

    /// Computes the theoretical upper bounds on read and write amplifications.
    pub fn compute_bounds(&self) -> TheoreticalAmplificationBounds {
        // Write Amp <= T * L
        let max_write_amp = (self.config.level_ratio as u64) * (self.config.num_levels as u64);

        // Read Amp <= 1 (L_max data block) + sum(FPR across levels)
        let total_fpr_permille = (self.config.bloom_fpr_permille as u64) * (self.config.num_levels as u64);
        let max_read_amp_permille = 1000 + total_fpr_permille; // 1000 = 1.0 base read

        // IOPS split
        let reserved_read_iops = (self.config.total_device_iops * self.config.reserved_read_iops_percent as u64) / 100;
        let max_compaction_iops = self.config.total_device_iops.saturating_sub(reserved_read_iops);

        TheoreticalAmplificationBounds {
            max_write_amp,
            max_read_amp_permille,
            reserved_read_iops,
            max_compaction_iops,
        }
    }

    /// Evaluates whether an in-flight background compaction burst is within the allowable hardware budget.
    pub fn is_compaction_within_budget(&self, active_compaction_iops: u64) -> bool {
        let bounds = self.compute_bounds();
        active_compaction_iops <= bounds.max_compaction_iops
    }
}
