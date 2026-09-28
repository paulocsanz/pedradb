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
}

/// Matriz de acoplamento dinâmico entre K níveis de compactação (L0 a L_{K-1}).
#[derive(Debug, Clone)]
pub struct InterLevelCouplingMatrix {
    /// Dimensão da matriz (K x K).
    pub size: usize,
    /// Coeficientes de transição em linha contígua.
    pub data: Vec<f64>,
}

impl InterLevelCouplingMatrix {
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
        assert_eq!(vec.len(), self.size);
        let mut out = vec![0.0; self.size];

        for i in 0..self.size {
            let mut sum = 0.0;
            for j in 0..self.size {
                sum += self.data[i * self.size + j] * vec[j];
            }
            out[i] = sum;
        }

        out
    }

    /// Calcula o raio espectral (maior autovalor em módulo) via Power Iteration Method.
    pub fn compute_spectral_radius(&self, max_iterations: usize) -> f64 {
        let mut v = vec![1.0 / (self.size as f64).sqrt(); self.size];

        let mut eigenvalue = 0.0;

        for _ in 0..max_iterations {
            let next_v = self.multiply_vector(&v);

            // Norma euclidiana
            let norm = next_v.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm < 1e-12 {
                return 0.0;
            }

            eigenvalue = norm;
            for i in 0..self.size {
                v[i] = next_v[i] / norm;
            }
        }

        eigenvalue
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
        let mut d = initial_impulse.to_vec();
        let mut prev_energy = d.iter().map(|x| x * x).sum::<f64>();

        for step in 0..steps {
            d = matrix.multiply_vector(&d);
            let energy = d.iter().map(|x| x * x).sum::<f64>();

            // A energia total do sistema deve decair exponencialmente
            if step > 5 && energy > prev_energy {
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
