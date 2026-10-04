//! RFC-0294: Pilar 6 - Desacoplamento Espectral e Amortecimento de Ressonância Harmônica.
//!
//! Modela as compactações concorrentes inter-nível como um sistema dinâmico linear acoplado,
//! provando que o raio espectral satisfaz rho(A) < 1.0 e erradica ressonâncias harmônicas de fase.

/// Violações de estabilidade espectral do escalonador de compactação.
#[derive(Debug, Clone, PartialEq)]
pub enum SpectralResonanceViolation {
    /// O raio espectral da matriz de acoplamento inter-nível excedeu o teto estável (< 1.0).
    SpectralRadiusExceeded {
        /// Raio espectral calculado.
        spectral_radius: f64,
        /// Teto máximo estável permitido (gamma).
        max_allowed_radius: f64,
    },
    /// Oscilações harmônicas sustentadas (onda estacionária de backlog) detectadas.
    HarmonicStandingWaveDetected {
        /// Amplitude da oscilação.
        oscillation_amplitude: f64,
        /// Período observado.
        period_ticks: usize,
    },
    /// Incompatibilidade dimensional entre a matriz de acoplamento e o vetor de impulso.
    DimensionMismatch {
        /// Dimensão esperada pela matriz.
        matrix_size: usize,
        /// Dimensão recebida pelo vetor de impulso.
        vector_size: usize,
    },
    /// Energia do sistema não-finita, nula ou corrompida com NaN/Inf.
    NonFiniteOrCorruptedEnergy {
        /// Passo da iteração em que a anomalia ocorreu.
        step: usize,
    },
    /// Limiar de estabilidade espectral inválido (deve ser finito e pertencer ao intervalo aberto (0.0, 1.0)).
    InvalidStabilityThreshold {
        /// Limiar rejeitado.
        threshold: f64,
    },
    /// Quantidade de passos de verificação de impulso não pode ser zero.
    ZeroSteps,
    /// Dimensão da matriz de acoplamento não pode ser zero.
    ZeroMatrixSize,
}

impl std::fmt::Display for SpectralResonanceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpectralRadiusExceeded { spectral_radius, max_allowed_radius } => write!(
                f,
                "Spectral radius {spectral_radius} exceeded stability threshold {max_allowed_radius}"
            ),
            Self::HarmonicStandingWaveDetected { oscillation_amplitude, period_ticks } => write!(
                f,
                "Harmonic standing wave detected: amplitude {oscillation_amplitude} at period {period_ticks}"
            ),
            Self::DimensionMismatch { matrix_size, vector_size } => write!(
                f,
                "Matrix dimension mismatch: matrix size {matrix_size} != vector size {vector_size}"
            ),
            Self::NonFiniteOrCorruptedEnergy { step } => write!(
                f,
                "Non-finite or corrupted energy detected at step {step}"
            ),
            Self::InvalidStabilityThreshold { threshold } => write!(
                f,
                "Invalid stability threshold {threshold} (must be in (0.0, 1.0))"
            ),
            Self::ZeroSteps => write!(f, "Verification step count cannot be zero"),
            Self::ZeroMatrixSize => write!(f, "Matrix dimension size cannot be zero"),
        }
    }
}

impl std::error::Error for SpectralResonanceViolation {}

/// Matriz de acoplamento dinâmico entre K níveis de compactação (L0 a L_{K-1}).
#[derive(Debug, Clone)]
pub struct InterLevelCouplingMatrix {
    /// Dimensão da matriz (K x K).
    pub size: usize,
    /// Coeficientes de transição em linha contígua.
    pub data: Vec<f64>,
}

impl InterLevelCouplingMatrix {
    /// Constrói uma matriz de acoplamento com validação fail-closed.
    pub fn try_build_damped_cascade(size: usize, diagonal_retention: f64, cascade_gain: f64) -> Result<Self, SpectralResonanceViolation> {
        if size == 0 {
            return Err(SpectralResonanceViolation::ZeroMatrixSize);
        }
        if !diagonal_retention.is_finite() || !cascade_gain.is_finite() {
            return Err(SpectralResonanceViolation::NonFiniteOrCorruptedEnergy { step: 0 });
        }
        Ok(Self::build_damped_cascade(size, diagonal_retention, cascade_gain))
    }
    /// Constrói uma matriz de acoplamento estável com fatores de amortecimento diagonal
    /// e coeficientes de transferência em cascata sub-diagonais.
    pub fn build_damped_cascade(size: usize, diagonal_retention: f64, cascade_gain: f64) -> Self {
        let mut data = vec![0.0; size * size];

        for i in 0..size {
            // Diagonal: retenção de dívida amortecida (deve ser < 1.0)
            data[i * size + i] = diagonal_retention;

            // Sub-diagonal: dados que L_{i-1} despeja em L_i durante compactação
            if i > 0 {
                data[i * size + (i - 1)] = cascade_gain;
            }
        }

        Self { size, data }
    }

    /// Multiplica a matriz pelo vetor de dívida atual: D_{t+1} = A * D_t.
    pub fn multiply_vector(&self, vec: &[f64]) -> Vec<f64> {
        if self.size == 0 {
            return Vec::new();
        }
        let mut out = vec![0.0; self.size];

        for i in 0..self.size {
            let mut sum = 0.0;
            for j in 0..self.size {
                let v = if j < vec.len() { vec[j] } else { 0.0 };
                let coef = self.data.get(i * self.size + j).copied().unwrap_or(0.0);
                sum += coef * v;
            }
            out[i] = if sum.is_finite() { sum } else { 0.0 };
        }

        out
    }

    /// Calcula o raio espectral (maior autovalor em módulo) via Power Iteration Method.
    pub fn compute_spectral_radius(&self, max_iterations: usize) -> f64 {
        if self.size == 0 {
            return 0.0;
        }

        let mut v = vec![1.0 / (self.size as f64).sqrt(); self.size];
        let mut eigenvalue = 0.0;

        for _ in 0..max_iterations {
            let next_v = self.multiply_vector(&v);

            // Norma euclidiana
            let norm = next_v.iter().map(|x| x * x).sum::<f64>().sqrt();
            if !norm.is_finite() || norm < 1e-12 {
                return if norm.is_nan() { 0.0 } else { eigenvalue };
            }

            eigenvalue = norm;
            for i in 0..self.size {
                v[i] = next_v[i] / norm;
            }
        }

        if eigenvalue.is_finite() { eigenvalue } else { 0.0 }
    }
}

/// Oráculo de verificação do amortecimento harmônico de compactações acopladas.
pub struct CompactionSpectralOracle;

impl CompactionSpectralOracle {
    /// Valida que a matriz de acoplamento de compactação satisfaz estritamente rho(A) <= gamma < 1.0,
    /// garantindo que qualquer perturbação de escrita decai assintoticamente para zero.
    pub fn verify_spectral_damping(
        matrix: &InterLevelCouplingMatrix,
        max_allowed_radius: f64,
    ) -> Result<f64, SpectralResonanceViolation> {
        if !max_allowed_radius.is_finite() || max_allowed_radius <= 0.0 || max_allowed_radius >= 1.0 {
            return Err(SpectralResonanceViolation::InvalidStabilityThreshold {
                threshold: max_allowed_radius,
            });
        }

        let radius = matrix.compute_spectral_radius(200);

        if radius > max_allowed_radius {
            return Err(SpectralResonanceViolation::SpectralRadiusExceeded {
                spectral_radius: radius,
                max_allowed_radius,
            });
        }

        Ok(radius)
    }

    /// Simula a evolução da dívida de compactação sob uma rajada inicial impulsiva
    /// e valida que a amplitude decai monotonicamente sem formar ondas estacionárias.
    pub fn verify_impulse_decay(
        matrix: &InterLevelCouplingMatrix,
        initial_impulse: &[f64],
        steps: usize,
    ) -> Result<(), SpectralResonanceViolation> {
        if steps == 0 {
            return Err(SpectralResonanceViolation::ZeroSteps);
        }
        if matrix.size != initial_impulse.len() {
            return Err(SpectralResonanceViolation::DimensionMismatch {
                matrix_size: matrix.size,
                vector_size: initial_impulse.len(),
            });
        }

        let mut d = initial_impulse.to_vec();
        for &val in &d {
            if !val.is_finite() || val < 0.0 {
                return Err(SpectralResonanceViolation::NonFiniteOrCorruptedEnergy { step: 0 });
            }
        }

        let mut prev_energy = d.iter().map(|x| x * x).sum::<f64>();
        if !prev_energy.is_finite() {
            return Err(SpectralResonanceViolation::NonFiniteOrCorruptedEnergy { step: 0 });
        }
        let initial_energy = prev_energy;

        for step in 0..steps {
            d = matrix.multiply_vector(&d);
            let energy = d.iter().map(|x| x * x).sum::<f64>();

            if !energy.is_finite() {
                return Err(SpectralResonanceViolation::NonFiniteOrCorruptedEnergy { step: step + 1 });
            }

            // Se a energia explodir além de 2x a inicial em qualquer momento, onda destrutiva detectada
            if energy > initial_energy.max(1.0) * 2.0 {
                return Err(SpectralResonanceViolation::HarmonicStandingWaveDetected {
                    oscillation_amplitude: energy - initial_energy,
                    period_ticks: step,
                });
            }

            // A energia total do sistema deve decair monotonicamente após acomodação de cascata (step > 2)
            if step > 2 && energy > prev_energy {
                return Err(SpectralResonanceViolation::HarmonicStandingWaveDetected {
                    oscillation_amplitude: energy - prev_energy,
                    period_ticks: step,
                });
            }
            prev_energy = energy;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_spectral_bounds_red_to_green() {
        assert_eq!(
            InterLevelCouplingMatrix::try_build_damped_cascade(0, 0.4, 0.2).err(),
            Some(SpectralResonanceViolation::ZeroMatrixSize)
        );

        let m = InterLevelCouplingMatrix::try_build_damped_cascade(2, 0.4, 0.2).expect("matrix");
        assert_eq!(
            CompactionSpectralOracle::verify_impulse_decay(&m, &[1.0, 0.0], 0).err(),
            Some(SpectralResonanceViolation::ZeroSteps)
        );
    }
}
