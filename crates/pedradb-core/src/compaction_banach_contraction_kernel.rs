//! RFC-0289: Compaction Banach Contraction and Wave Stability Kernel.
//!
//! Models inter-level compaction scheduling as a strict Banach contraction mapping (gamma < 1.0).
//! Mathematically proves asymptotic exponential convergence of compaction debts to a stable fixed point,
//! eliminating resonance write stalls and chaotic debt amplification.

/// Errors occurring in Banach compaction scheduling and debt vector calculations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BanachCompactionError {
    /// Contraction factor gamma must be strictly in (0.0, 1.0) and finite.
    InvalidGamma(f64),
    /// Epsilon for convergence must be strictly greater than 0.0 and finite.
    InvalidEpsilon(f64),
    /// Compaction debt or distance must be non-negative and finite.
    InvalidDebt(f64),
    /// Compaction debt vector must contain at least one level.
    EmptyDebtVector,
    /// Dimension mismatch between debt levels and write load vector.
    DimensionMismatch {
        /// Expected levels.
        expected: usize,
        /// Provided write load entries.
        provided: usize,
    },
}

impl std::fmt::Display for BanachCompactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGamma(g) => write!(f, "Invalid gamma {g}; must be strictly in (0.0, 1.0)"),
            Self::InvalidEpsilon(e) => write!(f, "Invalid epsilon {e}; must be > 0.0 and finite"),
            Self::InvalidDebt(d) => write!(f, "Invalid debt {d}; must be non-negative and finite"),
            Self::EmptyDebtVector => write!(f, "Compaction debt vector cannot be empty"),
            Self::DimensionMismatch { expected, provided } => {
                write!(f, "Dimension mismatch: expected {expected} levels, provided {provided}")
            }
        }
    }
}

impl std::error::Error for BanachCompactionError {}

/// State vector of compaction debts across LSM levels (L0, L1, ..., L_m).
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionDebtVector {
    /// Normalized debt scores per level (1.0 = target size reached, > 1.0 = compaction required).
    pub level_debts: Vec<f64>,
}

impl CompactionDebtVector {
    /// Attempts to create a new debt vector, validating that all components are non-negative, finite, and non-empty.
    pub fn try_new(level_debts: Vec<f64>) -> Result<Self, BanachCompactionError> {
        if level_debts.is_empty() {
            return Err(BanachCompactionError::EmptyDebtVector);
        }
        for &debt in &level_debts {
            if debt.is_nan() || !debt.is_finite() || debt < 0.0 {
                return Err(BanachCompactionError::InvalidDebt(debt));
            }
        }
        Ok(Self { level_debts })
    }

    /// Creates a new debt vector.
    ///
    /// # Panics
    /// Panics if any debt component is NaN, infinite, or negative.
    #[must_use]
    pub fn new(level_debts: Vec<f64>) -> Self {
        Self::try_new(level_debts).expect("valid debt components")
    }

    /// Computes infinity norm (maximum absolute debt): ||D||_inf.
    #[must_use]
    pub fn norm_inf(&self) -> f64 {
        self.level_debts
            .iter()
            .copied()
            .fold(0.0f64, |acc, val| if val.is_nan() { acc } else { acc.max(val.abs()) })
    }

    /// Computes infinity distance to another debt vector: ||D1 - D2||_inf.
    #[must_use]
    pub fn distance_inf(&self, other: &Self) -> f64 {
        let max_len = self.level_debts.len().max(other.level_debts.len());
        let mut max_diff = 0.0f64;
        for i in 0..max_len {
            let v1 = self.level_debts.get(i).copied().unwrap_or(0.0);
            let v2 = other.level_debts.get(i).copied().unwrap_or(0.0);
            max_diff = max_diff.max((v1 - v2).abs());
        }
        max_diff
    }
}

/// Contraction mapping scheduler enforcing stable convergence.
pub struct BanachCompactionScheduler {
    /// Contraction factor gamma in (0.0, 1.0).
    gamma: f64,
}

impl BanachCompactionScheduler {
    /// Attempts to create a new scheduler with a verified contraction factor gamma in (0.0, 1.0).
    pub fn try_new(gamma: f64) -> Result<Self, BanachCompactionError> {
        if gamma.is_nan() || !gamma.is_finite() || gamma <= 0.0 || gamma >= 1.0 {
            return Err(BanachCompactionError::InvalidGamma(gamma));
        }
        Ok(Self { gamma })
    }

    /// Creates a new scheduler with a proven contraction factor gamma < 1.0.
    ///
    /// # Panics
    /// Panics if gamma is not in (0.0, 1.0).
    #[must_use]
    pub fn new(gamma: f64) -> Self {
        Self::try_new(gamma).expect("contraction factor gamma must be strictly in (0.0, 1.0)")
    }

    /// Returns the contraction factor gamma.
    #[must_use]
    pub fn gamma(&self) -> f64 {
        self.gamma
    }

    /// Computes the exact theoretical fixed point D* = W / (1 - gamma) under constant write load W.
    pub fn compute_fixed_point(&self, write_load: &[f64]) -> Result<CompactionDebtVector, BanachCompactionError> {
        if write_load.is_empty() {
            return Err(BanachCompactionError::EmptyDebtVector);
        }
        let one_minus_gamma = 1.0 - self.gamma;
        let mut fixed_debts = Vec::with_capacity(write_load.len());
        for &w in write_load {
            if w.is_nan() || !w.is_finite() || w < 0.0 {
                return Err(BanachCompactionError::InvalidDebt(w));
            }
            fixed_debts.push(w / one_minus_gamma);
        }
        CompactionDebtVector::try_new(fixed_debts)
    }

    /// Safely applies one step of the compaction operator with dimension and value checking.
    pub fn try_transition_step(
        &self,
        current: &CompactionDebtVector,
        injected_write_load: &[f64],
    ) -> Result<CompactionDebtVector, BanachCompactionError> {
        let n = current.level_debts.len();
        if injected_write_load.len() != n {
            return Err(BanachCompactionError::DimensionMismatch {
                expected: n,
                provided: injected_write_load.len(),
            });
        }
        let mut next = Vec::with_capacity(n);
        for i in 0..n {
            let cur_debt = current.level_debts[i];
            let injection = injected_write_load[i];
            if injection.is_nan() || !injection.is_finite() || injection < 0.0 {
                return Err(BanachCompactionError::InvalidDebt(injection));
            }
            let new_debt = (self.gamma * cur_debt + injection).max(0.0);
            next.push(new_debt);
        }
        CompactionDebtVector::try_new(next)
    }

    /// Applies one step of the compaction dynamic operator T(D) with external write injection.
    #[must_use]
    pub fn transition_step(
        &self,
        current: &CompactionDebtVector,
        injected_write_load: &[f64],
    ) -> CompactionDebtVector {
        let n = current.level_debts.len();
        let mut next = Vec::with_capacity(n);

        for i in 0..n {
            let cur_debt = current.level_debts[i];
            let injection = injected_write_load.get(i).copied().unwrap_or(0.0);

            // T(D)_i = gamma * cur_debt + injection
            // Because gamma < 1.0, this operator contracts distances: ||T(D1) - T(D2)|| <= gamma * ||D1 - D2||
            let new_debt = (self.gamma * cur_debt + injection).max(0.0);
            next.push(new_debt);
        }

        CompactionDebtVector { level_debts: next }
    }

    /// Verifies the Banach Contraction Invariant:
    /// `||T(D1) - T(D2)||_inf <= gamma * ||D1 - D2||_inf`.
    #[must_use]
    pub fn verify_contraction(
        &self,
        d1: &CompactionDebtVector,
        d2: &CompactionDebtVector,
        injection: &[f64],
    ) -> bool {
        let dist_before = d1.distance_inf(d2);
        let t_d1 = self.transition_step(d1, injection);
        let t_d2 = self.transition_step(d2, injection);
        let dist_after = t_d1.distance_inf(&t_d2);

        // Allow tiny epsilon for f64 floating point rounding
        dist_after <= (self.gamma * dist_before) + 1e-12
    }

    /// Attempts to calculate theoretical maximum steps to converge within `epsilon` of fixed point.
    pub fn try_steps_to_convergence(&self, initial_distance: f64, epsilon: f64) -> Result<usize, BanachCompactionError> {
        if initial_distance.is_nan() || !initial_distance.is_finite() || initial_distance < 0.0 {
            return Err(BanachCompactionError::InvalidDebt(initial_distance));
        }
        if epsilon.is_nan() || !epsilon.is_finite() || epsilon <= 0.0 {
            return Err(BanachCompactionError::InvalidEpsilon(epsilon));
        }
        if initial_distance <= epsilon {
            return Ok(0);
        }
        // gamma^k * d0 <= eps => k * ln(gamma) <= ln(eps / d0) => k >= ln(eps / d0) / ln(gamma)
        let k = (epsilon / initial_distance).ln() / self.gamma.ln();
        let ceil_k = k.ceil();
        if ceil_k < 0.0 || ceil_k.is_nan() {
            Ok(0)
        } else {
            Ok(ceil_k as usize)
        }
    }

    /// Calculates theoretical maximum steps to converge within `epsilon` of fixed point.
    #[must_use]
    pub fn steps_to_convergence(&self, initial_distance: f64, epsilon: f64) -> usize {
        self.try_steps_to_convergence(initial_distance, epsilon).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_debt_vector_rejected() {
        let err = CompactionDebtVector::try_new(vec![]).unwrap_err();
        assert_eq!(err, BanachCompactionError::EmptyDebtVector);
    }

    #[test]
    fn test_empty_write_load_fixed_point_rejected() {
        let scheduler = BanachCompactionScheduler::new(0.5);
        let err = scheduler.compute_fixed_point(&[]).unwrap_err();
        assert_eq!(err, BanachCompactionError::EmptyDebtVector);
    }

    #[test]
    fn test_dimension_mismatch_transition_step_rejected() {
        let scheduler = BanachCompactionScheduler::new(0.5);
        let debts = CompactionDebtVector::new(vec![1.0, 2.0, 3.0]);
        let err = scheduler.try_transition_step(&debts, &[0.5]).unwrap_err();
        assert_eq!(
            err,
            BanachCompactionError::DimensionMismatch {
                expected: 3,
                provided: 1,
            }
        );
    }

    #[test]
    fn test_nan_or_negative_injection_rejected() {
        let scheduler = BanachCompactionScheduler::new(0.5);
        let debts = CompactionDebtVector::new(vec![1.0, 2.0]);
        let err_nan = scheduler.try_transition_step(&debts, &[0.5, f64::NAN]).unwrap_err();
        assert!(matches!(err_nan, BanachCompactionError::InvalidDebt(_)));

        let err_neg = scheduler.try_transition_step(&debts, &[0.5, -1.0]).unwrap_err();
        assert_eq!(err_neg, BanachCompactionError::InvalidDebt(-1.0));
    }
}

