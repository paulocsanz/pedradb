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

/// Violações de limites e invariantes de amplificação RUM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RumBudgetViolation {
    /// Número de níveis da árvore LSM deve ser pelo menos 1.
    InvalidNumLevels,
    /// Razão de crescimento entre níveis consecutivos (T) deve ser pelo menos 2.
    InvalidLevelRatio,
    /// Porcentagem reservada para leitura não pode ultrapassar 100%.
    ReservedPercentageExceeds100 { percent: u32 },
    /// Capacidade total de IOPS do dispositivo deve ser estritamente positiva.
    ZeroTotalDeviceIops,
    /// Taxa de falso positivo do Bloom filter não pode exceder 1000 por mil (100%).
    InvalidBloomFprPermille { permille: u32 },
    /// Requisição de admissão de IOPS de compactação não pode ser zero.
    ZeroRequestedCompactionIops,
    /// Overflow aritmético nos cálculos de limites teóricos.
    ArithmeticOverflow,
}

impl std::fmt::Display for RumBudgetViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidNumLevels => write!(f, "LSM tree level count must be at least 1"),
            Self::InvalidLevelRatio => write!(f, "LSM level growth ratio must be at least 2"),
            Self::ReservedPercentageExceeds100 { percent } => {
                write!(f, "reserved read percentage {percent}% exceeds 100%")
            }
            Self::ZeroTotalDeviceIops => write!(f, "total device IOPS budget must be strictly positive"),
            Self::InvalidBloomFprPermille { permille } => {
                write!(f, "bloom false positive rate {permille}‰ exceeds maximum of 1000‰")
            }
            Self::ZeroRequestedCompactionIops => {
                write!(f, "requested compaction IOPS admission cannot be zero")
            }
            Self::ArithmeticOverflow => write!(f, "arithmetic overflow in RUM bound calculations"),
        }
    }
}

impl std::error::Error for RumBudgetViolation {}

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

impl RumBudgetConfig {
    /// Cria uma nova configuração RUM validada.
    pub fn try_new(
        num_levels: u32,
        level_ratio: u32,
        bloom_fpr_permille: u32,
        total_device_iops: u64,
        reserved_read_iops_percent: u32,
    ) -> Result<Self, RumBudgetViolation> {
        let config = Self {
            num_levels,
            level_ratio,
            bloom_fpr_permille,
            total_device_iops,
            reserved_read_iops_percent,
        };
        config.validate()?;
        Ok(config)
    }

    /// Valida as restrições matemáticas e físicas da configuração RUM.
    pub fn validate(&self) -> Result<(), RumBudgetViolation> {
        if self.num_levels == 0 {
            return Err(RumBudgetViolation::InvalidNumLevels);
        }
        if self.level_ratio < 2 {
            return Err(RumBudgetViolation::InvalidLevelRatio);
        }
        if self.reserved_read_iops_percent > 100 {
            return Err(RumBudgetViolation::ReservedPercentageExceeds100 {
                percent: self.reserved_read_iops_percent,
            });
        }
        if self.total_device_iops == 0 {
            return Err(RumBudgetViolation::ZeroTotalDeviceIops);
        }
        if self.bloom_fpr_permille > 1000 {
            return Err(RumBudgetViolation::InvalidBloomFprPermille {
                permille: self.bloom_fpr_permille,
            });
        }
        Ok(())
    }
}

/// Calculated theoretical amplification bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TheoreticalAmplificationBounds {
    /// Maximum worst-case write amplification factor.
    pub max_write_amp: u64,
    /// Maximum expected read amplification (disk reads per point lookup) in permille.
    pub max_read_amp_permille: u64,
    /// Maximum theoretical space amplification factor in permille (1000 = 1.0x).
    pub max_space_amp_permille: u64,
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
    /// Creates a new evaluator with given pre-validated configuration.
    pub fn try_new(config: RumBudgetConfig) -> Result<Self, RumBudgetViolation> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Creates a new evaluator with given configuration.
    pub fn new(config: RumBudgetConfig) -> Self {
        Self { config }
    }

    /// Computa com segurança os limites teóricos de amplificação RUM.
    pub fn checked_compute_bounds(&self) -> Result<TheoreticalAmplificationBounds, RumBudgetViolation> {
        self.config.validate()?;

        let max_write_amp = (self.config.level_ratio as u64)
            .checked_mul(self.config.num_levels as u64)
            .ok_or(RumBudgetViolation::ArithmeticOverflow)?;

        let total_fpr = (self.config.bloom_fpr_permille as u64)
            .checked_mul(self.config.num_levels as u64)
            .ok_or(RumBudgetViolation::ArithmeticOverflow)?;

        let max_read_amp_permille = 1000u64
            .checked_add(total_fpr)
            .ok_or(RumBudgetViolation::ArithmeticOverflow)?;

        let max_space_amp_permille = if self.config.level_ratio > 1 {
            1000u64.saturating_add(1000u64 / (self.config.level_ratio as u64 - 1))
        } else {
            2000u64
        };

        let reserved_read_iops = self
            .config
            .total_device_iops
            .checked_mul(self.config.reserved_read_iops_percent as u64)
            .map(|prod| prod / 100)
            .unwrap_or_else(|| {
                (self.config.total_device_iops / 100).saturating_mul(self.config.reserved_read_iops_percent as u64)
            });

        let max_compaction_iops = self.config.total_device_iops.saturating_sub(reserved_read_iops);

        Ok(TheoreticalAmplificationBounds {
            max_write_amp,
            max_read_amp_permille,
            max_space_amp_permille,
            reserved_read_iops,
            max_compaction_iops,
        })
    }

    /// Computes the theoretical upper bounds on read and write amplifications.
    pub fn compute_bounds(&self) -> TheoreticalAmplificationBounds {
        self.checked_compute_bounds().unwrap_or_else(|_| {
            let max_write_amp = (self.config.level_ratio as u64).saturating_mul(self.config.num_levels as u64);
            let total_fpr_permille = (self.config.bloom_fpr_permille as u64).saturating_mul(self.config.num_levels as u64);
            let max_read_amp_permille = 1000u64.saturating_add(total_fpr_permille);
            let max_space_amp_permille = if self.config.level_ratio > 1 {
                1000u64.saturating_add(1000u64 / (self.config.level_ratio as u64 - 1))
            } else {
                2000u64
            };
            let reserved_read_iops = (self.config.total_device_iops / 100).saturating_mul(self.config.reserved_read_iops_percent as u64);
            let max_compaction_iops = self.config.total_device_iops.saturating_sub(reserved_read_iops);
            TheoreticalAmplificationBounds {
                max_write_amp,
                max_read_amp_permille,
                max_space_amp_permille,
                reserved_read_iops,
                max_compaction_iops,
            }
        })
    }

    /// Regula a admissão de requisições de IOPS de compactação de background, garantindo o piso de leitura.
    pub fn admit_compaction_iops(&self, requested_compaction_iops: u64) -> Result<u64, RumBudgetViolation> {
        if requested_compaction_iops == 0 {
            return Err(RumBudgetViolation::ZeroRequestedCompactionIops);
        }
        let bounds = self.checked_compute_bounds()?;
        Ok(std::cmp::min(requested_compaction_iops, bounds.max_compaction_iops))
    }

    /// Evaluates whether an in-flight background compaction burst is within the allowable hardware budget.
    pub fn is_compaction_within_budget(&self, active_compaction_iops: u64) -> bool {
        let bounds = self.compute_bounds();
        active_compaction_iops <= bounds.max_compaction_iops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rum_pareto_structural_invariants_red_to_green() {
        // 1. Default config is strictly valid
        let default_config = RumBudgetConfig::default();
        let evaluator = RumParetoEvaluator::try_new(default_config).expect("default config must be valid");
        let bounds = evaluator.checked_compute_bounds().expect("bounds compute successfully");
        assert_eq!(bounds.max_write_amp, 70); // 7 levels * ratio 10
        assert_eq!(bounds.reserved_read_iops, 30_000);
        assert_eq!(bounds.max_compaction_iops, 70_000);

        // 2. Reject zero levels
        let err_levels = RumBudgetConfig::try_new(0, 10, 10, 100_000, 30);
        assert_eq!(err_levels, Err(RumBudgetViolation::InvalidNumLevels));

        // 3. Reject level ratio < 2
        let err_ratio = RumBudgetConfig::try_new(7, 1, 10, 100_000, 30);
        assert_eq!(err_ratio, Err(RumBudgetViolation::InvalidLevelRatio));

        // 4. Reject reserved percent > 100
        let err_percent = RumBudgetConfig::try_new(7, 10, 10, 100_000, 101);
        assert_eq!(err_percent, Err(RumBudgetViolation::ReservedPercentageExceeds100 { percent: 101 }));

        // 5. Reject zero total IOPS
        let err_zero_iops = RumBudgetConfig::try_new(7, 10, 10, 0, 30);
        assert_eq!(err_zero_iops, Err(RumBudgetViolation::ZeroTotalDeviceIops));

        // 6. Reject bloom FPR > 1000 permille
        let err_bloom = RumBudgetConfig::try_new(7, 10, 1001, 100_000, 30);
        assert_eq!(err_bloom, Err(RumBudgetViolation::InvalidBloomFprPermille { permille: 1001 }));

        // 7. Compaction admission regulation
        assert_eq!(evaluator.admit_compaction_iops(50_000), Ok(50_000));
        assert_eq!(evaluator.admit_compaction_iops(100_000), Ok(70_000));
        assert_eq!(evaluator.admit_compaction_iops(0), Err(RumBudgetViolation::ZeroRequestedCompactionIops));

        // 8. Display & Error implementations
        let d = format!("{}", RumBudgetViolation::InvalidBloomFprPermille { permille: 1200 });
        assert!(d.contains("1200‰ exceeds maximum of 1000‰"));
    }
}
