//! RFC-0315: Deterministic Decay Model & Complexity Scaling Oracle Kernel
//!
//! Fits cost(n) = c * n^alpha via log-log ordinary least squares regression
//! and enforces mathematical bounds on operational scaling complexity.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PhaseKind {
    ApplyBatchUs,
    MissOutNs,
    MissInNs,
    HitWarmUs,
}

impl PhaseKind {
    #[inline]
    pub fn alpha_ceiling(&self) -> f64 {
        match self {
            PhaseKind::ApplyBatchUs => 0.15,
            PhaseKind::MissOutNs => 0.20,
            PhaseKind::MissInNs => 0.20,
            PhaseKind::HitWarmUs => 0.20,
        }
    }

    #[inline]
    pub fn max_allocs_per_op(&self) -> f64 {
        match self {
            PhaseKind::ApplyBatchUs => 0.50,
            PhaseKind::MissOutNs => 0.00,
            PhaseKind::MissInNs => 0.50,
            PhaseKind::HitWarmUs => 2.00,
        }
    }
}

/// Errors occurring during decay measurement validation or regression fitting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecayModelError {
    /// Scale N must be strictly greater than zero.
    ZeroScaleN,
    /// Samples vector cannot be empty.
    EmptySamples,
    /// Sample value must be finite and non-negative.
    InvalidSample,
    /// Allocs per op must be finite and non-negative.
    InvalidAllocs,
    /// Insufficient data points for ordinary least squares regression.
    InsufficientPoints,
    /// Zero variance in scale points.
    DegenerateScaleVariance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecayMeasurement {
    pub scale_n: u64,
    pub samples: Vec<f64>,
    pub allocs_per_op: f64,
}

impl DecayMeasurement {
    /// Attempts to create a new validated decay measurement descriptor.
    pub fn try_new(
        scale_n: u64,
        samples: Vec<f64>,
        allocs_per_op: f64,
    ) -> Result<Self, DecayModelError> {
        if scale_n == 0 {
            return Err(DecayModelError::ZeroScaleN);
        }
        if samples.is_empty() {
            return Err(DecayModelError::EmptySamples);
        }
        for &s in &samples {
            if s.is_nan() || !s.is_finite() || s < 0.0 {
                return Err(DecayModelError::InvalidSample);
            }
        }
        if allocs_per_op.is_nan() || !allocs_per_op.is_finite() || allocs_per_op < 0.0 {
            return Err(DecayModelError::InvalidAllocs);
        }
        Ok(Self {
            scale_n,
            samples,
            allocs_per_op,
        })
    }

    pub fn new(scale_n: u64, samples: Vec<f64>, allocs_per_op: f64) -> Self {
        Self::try_new(scale_n, samples, allocs_per_op).expect("valid decay measurement")
    }

    #[inline]
    pub fn mean(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f64>() / (self.samples.len() as f64)
    }

    /// Spread check: (max - min) / mean. Returns true if spread > 0.15 (contaminated).
    pub fn is_contaminated(&self, max_spread: f64) -> bool {
        if self.samples.len() < 2 {
            return false;
        }
        let mut min_val = f64::MAX;
        let mut max_val = f64::MIN;
        for &s in &self.samples {
            if s < min_val {
                min_val = s;
            }
            if s > max_val {
                max_val = s;
            }
        }
        let mean = self.mean();
        if mean <= 1e-9 {
            return false;
        }
        (max_val - min_val) / mean > max_spread
    }
}

/// Fitted decay model: `cost(n) = c * n^alpha`.
#[derive(Clone, Debug, PartialEq)]
pub struct DecayModel {
    /// Scaling constant `c`.
    pub c: f64,
    /// Scaling exponent `alpha`.
    pub alpha: f64,
    /// Coefficient of determination `R^2` in log-log space.
    pub r_squared: f64,
}

impl DecayModel {
    /// Predicts expected operational cost for scale `n`.
    #[inline]
    pub fn predict_cost(&self, n: u64) -> f64 {
        if n == 0 {
            return 0.0;
        }
        self.c * (n as f64).powf(self.alpha)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DecayVerdict {
    Pass { alpha: f64, max_allocs: f64 },
    FailAlpha { alpha: f64, ceiling: f64 },
    FailAllocs { allocs: f64, ceiling: f64 },
    Contaminated { scale_n: u64, spread: f64 },
    InsufficientData,
}

#[derive(Clone, Debug, Default)]
pub struct DecayModelEvaluator;

impl DecayModelEvaluator {
    /// Fits alpha from measurements (n_i, mean_cost_i) using ordinary least squares in log-log space.
    pub fn fit_alpha(measurements: &[DecayMeasurement]) -> Option<f64> {
        Self::fit_model(measurements).ok().map(|m| m.alpha)
    }

    /// Fits complete decay model (c, alpha, R^2) from measurements in log-log space.
    pub fn fit_model(measurements: &[DecayMeasurement]) -> Result<DecayModel, DecayModelError> {
        if measurements.len() < 2 {
            return Err(DecayModelError::InsufficientPoints);
        }

        let mut pts = Vec::with_capacity(measurements.len());
        for m in measurements {
            let mean = m.mean();
            if m.scale_n == 0 {
                return Err(DecayModelError::ZeroScaleN);
            }
            if mean <= 0.0 || mean.is_nan() {
                return Err(DecayModelError::InvalidSample);
            }
            pts.push(((m.scale_n as f64).ln(), mean.ln()));
        }

        let n = pts.len() as f64;
        let mean_x = pts.iter().map(|(x, _)| *x).sum::<f64>() / n;
        let mean_y = pts.iter().map(|(_, y)| *y).sum::<f64>() / n;

        let mut ss_xy = 0.0;
        let mut ss_xx = 0.0;
        let mut ss_yy = 0.0;
        for (x, y) in &pts {
            let dx = x - mean_x;
            let dy = y - mean_y;
            ss_xy += dx * dy;
            ss_xx += dx * dx;
            ss_yy += dy * dy;
        }

        if ss_xx.abs() <= 1e-12 {
            return Err(DecayModelError::DegenerateScaleVariance);
        }

        let alpha = ss_xy / ss_xx;
        let ln_c = mean_y - alpha * mean_x;
        let c = ln_c.exp();

        let r_squared = if ss_yy > 1e-12 {
            (ss_xy * ss_xy) / (ss_xx * ss_yy)
        } else {
            1.0
        };

        Ok(DecayModel {
            c,
            alpha,
            r_squared,
        })
    }

    /// Evaluates whether a set of phase measurements complies with RFC-0315 complexity bounds.
    pub fn evaluate_phase(
        phase: PhaseKind,
        measurements: &[DecayMeasurement],
        max_allowed_spread: f64,
    ) -> DecayVerdict {
        if measurements.len() < 2 {
            return DecayVerdict::InsufficientData;
        }

        // Check for measurement contamination (noise/contention)
        for m in measurements {
            if m.is_contaminated(max_allowed_spread) {
                let mean = m.mean();
                let min_val = m.samples.iter().cloned().fold(f64::INFINITY, f64::min);
                let max_val = m.samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let spread = if mean > 1e-9 {
                    (max_val - min_val) / mean
                } else {
                    0.0
                };
                return DecayVerdict::Contaminated {
                    scale_n: m.scale_n,
                    spread,
                };
            }
        }

        // Check allocations per op against budget
        let max_allocs = measurements
            .iter()
            .map(|m| m.allocs_per_op)
            .fold(0.0, f64::max);

        if max_allocs > phase.max_allocs_per_op() {
            return DecayVerdict::FailAllocs {
                allocs: max_allocs,
                ceiling: phase.max_allocs_per_op(),
            };
        }

        // Fit alpha
        match Self::fit_alpha(measurements) {
            Some(alpha) => {
                if alpha > phase.alpha_ceiling() {
                    DecayVerdict::FailAlpha {
                        alpha,
                        ceiling: phase.alpha_ceiling(),
                    }
                } else {
                    DecayVerdict::Pass {
                        alpha,
                        max_allocs,
                    }
                }
            }
            None => DecayVerdict::InsufficientData,
        }
    }
}
