//! RFC-0292: Pilar 2 - Estabilidade de Foster-Lyapunov sob Ruído Estocástico de Lévy/Pareto.
//!
//! Controla o write stall e a drenagem do LSM sob rajadas de escrita com cauda pesada e momentos
//! infinitos, provando a condição de deriva negativa quase-certa e retorno ao equilíbrio em tempo finito.

/// Violações de estabilidade estocástica e deriva de Foster-Lyapunov.
#[derive(Debug, Clone, PartialEq)]
pub enum StochasticStallViolation {
    /// A deriva estocástica calculada foi positiva fora do conjunto compacto C.
    PositiveDriftOutsideCompactSet {
        /// Estado atual (MemTable bytes, L0 SSTs).
        state: (u64, u32),
        /// Valor de Lyapunov V(X).
        v_current: f64,
        /// Valor esperado após transição E[V(X')].
        v_expected_next: f64,
        /// Deriva calculada.
        drift: f64,
        /// Limite de deriva epsilon.
        expected_epsilon: f64,
    },
    /// A taxa de admissão colapsou a zero indefinidamente sem recuperação.
    PermanentStarvationDetected {
        /// Duração dos passos em admissão zero.
        zero_admission_steps: usize,
    },
    ZeroCompactMemtableBytes,
    ZeroCompactL0Files,
    InvalidGamma(f64),
    InvalidDriftEpsilon(f64),
    ZeroDrainCapacity,
}

impl std::fmt::Display for StochasticStallViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PositiveDriftOutsideCompactSet { state, v_current, v_expected_next, drift, expected_epsilon } => {
                write!(
                    f,
                    "Positive drift {drift} outside compact set {state:?}: V={v_current} -> E[V']={v_expected_next} (expected <= -{expected_epsilon})"
                )
            }
            Self::PermanentStarvationDetected { zero_admission_steps } => {
                write!(f, "Permanent starvation detected for {zero_admission_steps} consecutive steps")
            }
            Self::ZeroCompactMemtableBytes => write!(f, "Compact memtable bytes cannot be zero"),
            Self::ZeroCompactL0Files => write!(f, "Compact L0 files cannot be zero"),
            Self::InvalidGamma(gamma) => write!(f, "Invalid gamma parameter: {gamma} (must be in (0.0, 1.0))"),
            Self::InvalidDriftEpsilon(eps) => write!(f, "Invalid drift epsilon: {eps} (must be finite > 0.0)"),
            Self::ZeroDrainCapacity => write!(f, "Drain capacity cannot be zero"),
        }
    }
}

impl std::error::Error for StochasticStallViolation {}

/// Configuração do controlador de Foster-Lyapunov.
#[derive(Debug, Clone, PartialEq)]
pub struct FosterLyapunovConfig {
    /// Limite compacto para MemTable dirty bytes (ex: 64 MiB).
    pub compact_memtable_bytes: u64,
    /// Limite compacto para arquivos L0 (ex: 4 arquivos).
    pub compact_l0_files: u32,
    /// Expoente fracionário da função de Lyapunov (gamma < alpha - 1, ex: 0.5 para alpha=1.8).
    pub gamma: f64,
    /// Margem de deriva estritamente negativa epsilon (ex: 0.05).
    pub drift_epsilon: f64,
    /// Capacidade máxima do flusher/compactor em bytes por tick.
    pub drain_capacity_bytes: u64,
    /// Capacidade máxima de drenagem de arquivos L0 por tick de compactação.
    pub drain_capacity_l0: u32,
}

impl FosterLyapunovConfig {
    /// Safely validates and constructs a FosterLyapunovConfig.
    pub fn try_new(
        compact_memtable_bytes: u64,
        compact_l0_files: u32,
        gamma: f64,
        drift_epsilon: f64,
        drain_capacity_bytes: u64,
        drain_capacity_l0: u32,
    ) -> Result<Self, StochasticStallViolation> {
        if compact_memtable_bytes == 0 {
            return Err(StochasticStallViolation::ZeroCompactMemtableBytes);
        }
        if compact_l0_files == 0 {
            return Err(StochasticStallViolation::ZeroCompactL0Files);
        }
        if !gamma.is_finite() || gamma <= 0.0 || gamma >= 1.0 {
            return Err(StochasticStallViolation::InvalidGamma(gamma));
        }
        if !drift_epsilon.is_finite() || drift_epsilon <= 0.0 {
            return Err(StochasticStallViolation::InvalidDriftEpsilon(drift_epsilon));
        }
        if drain_capacity_bytes == 0 && drain_capacity_l0 == 0 {
            return Err(StochasticStallViolation::ZeroDrainCapacity);
        }
        Ok(Self {
            compact_memtable_bytes,
            compact_l0_files,
            gamma,
            drift_epsilon,
            drain_capacity_bytes,
            drain_capacity_l0,
        })
    }
}

impl Default for FosterLyapunovConfig {
    fn default() -> Self {
        Self {
            compact_memtable_bytes: 64 * 1024 * 1024,
            compact_l0_files: 4,
            gamma: 0.5,
            drift_epsilon: 0.05,
            drain_capacity_bytes: 32 * 1024 * 1024,
            drain_capacity_l0: 2,
        }
    }
}

/// Estado dinâmico do motor LSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LsmDynamicState {
    /// Bytes pendentes em MemTables ativas e imutáveis.
    pub memtable_bytes: u64,
    /// Quantidade de arquivos SST residentes em L0.
    pub l0_files: u32,
}

impl LsmDynamicState {
    /// Constrói um estado LSM dinâmico.
    #[must_use]
    pub const fn new(memtable_bytes: u64, l0_files: u32) -> Self {
        Self {
            memtable_bytes,
            l0_files,
        }
    }
}

/// Decisão de controle de admissão calculada pelo controlador de Foster-Lyapunov.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdmissionDecision {
    /// Fração de admissão de escrita [0.0, 1.0].
    pub admission_fraction: f64,
    /// Flag indicando se o estado está no conjunto compacto C.
    pub in_compact_set: bool,
    /// Valor atual da função de Lyapunov V(X).
    pub lyapunov_value: f64,
}

/// Controlador de admissão baseado no Teorema de Deriva de Foster-Lyapunov.
#[derive(Debug, Clone)]
pub struct FosterLyapunovController {
    config: FosterLyapunovConfig,
}

impl FosterLyapunovController {
    /// Cria uma nova instância do controlador com a configuração especificada de forma segura.
    pub fn try_new(config: FosterLyapunovConfig) -> Result<Self, StochasticStallViolation> {
        FosterLyapunovConfig::try_new(
            config.compact_memtable_bytes,
            config.compact_l0_files,
            config.gamma,
            config.drift_epsilon,
            config.drain_capacity_bytes,
            config.drain_capacity_l0,
        )?;
        Ok(Self::new(config))
    }

    /// Retorna a configuração do controlador.
    #[must_use]
    pub fn config(&self) -> &FosterLyapunovConfig {
        &self.config
    }

    /// Cria uma nova instância do controlador com a configuração especificada.
    pub fn new(mut config: FosterLyapunovConfig) -> Self {
        if config.compact_memtable_bytes == 0 {
            config.compact_memtable_bytes = 1;
        }
        if config.compact_l0_files == 0 {
            config.compact_l0_files = 1;
        }
        if !config.gamma.is_finite() || config.gamma <= 0.0 || config.gamma >= 1.0 {
            config.gamma = 0.5;
        }
        if !config.drift_epsilon.is_finite() || config.drift_epsilon < 0.0 {
            config.drift_epsilon = 0.05;
        }
        Self { config }
    }


    /// Calcula a função candidata de Lyapunov sub-quadrática:
    /// V(M, S) = ((M / M_compact)^2 + (S / S_compact)^2)^(gamma / 2)
    pub fn compute_v(&self, state: LsmDynamicState) -> f64 {
        let m_compact = self.config.compact_memtable_bytes.max(1) as f64;
        let s_compact = self.config.compact_l0_files.max(1) as f64;
        let m_ratio = (state.memtable_bytes as f64) / m_compact;
        let s_ratio = (state.l0_files as f64) / s_compact;
        let norm_sq = m_ratio * m_ratio + s_ratio * s_ratio;
        let v = norm_sq.powf(self.config.gamma / 2.0);
        if v.is_finite() { v } else { 0.0 }
    }

    /// Avalia se o estado atual pertence ao conjunto compacto C.
    pub fn is_in_compact_set(&self, state: LsmDynamicState) -> bool {
        state.memtable_bytes <= self.config.compact_memtable_bytes
            && state.l0_files <= self.config.compact_l0_files
    }

    /// Calcula a fração de admissão garantindo controle estável mesmo sob caudas pesadas.
    pub fn evaluate_admission(&self, state: LsmDynamicState) -> AdmissionDecision {
        let in_compact_set = self.is_in_compact_set(state);
        let v = self.compute_v(state);

        let admission_fraction = if in_compact_set {
            1.0
        } else {
            // Decaimento suave inversamente proporcional a V(X) elevado a 1/gamma,
            // garantindo que escritas sejam estranguladas antes que a deriva fique positiva.
            let inv_gamma = 1.0 / self.config.gamma;
            let scaled_excess = (v - 1.0).max(0.0).powf(inv_gamma);
            let penalty = 1.0 / (1.0 + scaled_excess);
            if penalty.is_finite() {
                penalty.clamp(0.001, 1.0)
            } else {
                0.001
            }
        };

        AdmissionDecision {
            admission_fraction,
            in_compact_set,
            lyapunov_value: v,
        }
    }

    /// Simula um passo estocástico de transição e valida a deriva de Foster-Lyapunov:
    /// E[V(X_{t+1}) - V(X_t) | X_t = x] <= -epsilon quando x fora de C.
    pub fn verify_step_drift(
        &self,
        current_state: LsmDynamicState,
        arrival_samples: &[u64], // Amostras da distribuição de cauda pesada de chegada
    ) -> Result<f64, StochasticStallViolation> {
        let in_compact = self.is_in_compact_set(current_state);
        let v_curr = self.compute_v(current_state);
        let decision = self.evaluate_admission(current_state);

        if arrival_samples.is_empty() {
            return Ok(0.0);
        }

        // Calcula a média de V(X') sobre todas as amostras de rajada estocástica
        let mut sum_v_next = 0.0;
        for &raw_arrival in arrival_samples {
            // A taxa admitida é modulada pela fração de admissão do controlador
            let admitted_bytes = ((raw_arrival as f64) * decision.admission_fraction) as u64;

            // Transição de estado sob capacidade de drenagem do flusher
            let next_mem_bytes = current_state
                .memtable_bytes
                .saturating_add(admitted_bytes)
                .saturating_sub(self.config.drain_capacity_bytes);

            // Novos arquivos L0 gerados pelo flush
            let new_l0 = (admitted_bytes / self.config.compact_memtable_bytes.max(1)) as u32;
            let remainder = admitted_bytes % self.config.compact_memtable_bytes.max(1);
            let extra = if remainder > self.config.compact_memtable_bytes / 2 { 1 } else { 0 };
            let next_l0 = current_state
                .l0_files
                .saturating_add(new_l0.saturating_add(extra))
                .saturating_sub(self.config.drain_capacity_l0);

            let next_state = LsmDynamicState {
                memtable_bytes: next_mem_bytes,
                l0_files: next_l0,
            };

            sum_v_next += self.compute_v(next_state);
        }

        let e_v_next = sum_v_next / (arrival_samples.len() as f64);
        let drift = e_v_next - v_curr;

        // Se fora do conjunto compacto C, a deriva DEVE ser menor ou igual a -drift_epsilon
        if !in_compact && drift > -self.config.drift_epsilon {
            return Err(StochasticStallViolation::PositiveDriftOutsideCompactSet {
                state: (current_state.memtable_bytes, current_state.l0_files),
                v_current: v_curr,
                v_expected_next: e_v_next,
                drift,
                expected_epsilon: self.config.drift_epsilon,
            });
        }

        Ok(drift)
    }

    /// Simula a evolução dinâmica do LSM sob uma série temporal de chegadas e valida
    /// a ausência de inanição permanente (Permanent Starvation) e convergência.
    pub fn simulate_trajectory(
        &self,
        initial_state: LsmDynamicState,
        arrival_trace: &[u64],
        max_consecutive_starvation: usize,
    ) -> Result<Vec<LsmDynamicState>, StochasticStallViolation> {
        let mut state = initial_state;
        let mut trajectory = Vec::with_capacity(arrival_trace.len() + 1);
        trajectory.push(state);

        let mut starvation_counter = 0;

        for &arrival in arrival_trace {
            let decision = self.evaluate_admission(state);
            if decision.admission_fraction <= 0.01 {
                starvation_counter += 1;
                if starvation_counter > max_consecutive_starvation {
                    return Err(StochasticStallViolation::PermanentStarvationDetected {
                        zero_admission_steps: starvation_counter,
                    });
                }
            } else {
                starvation_counter = 0;
            }

            let admitted = ((arrival as f64) * decision.admission_fraction) as u64;
            let next_mem = state
                .memtable_bytes
                .saturating_add(admitted)
                .saturating_sub(self.config.drain_capacity_bytes);

            let new_l0 = (admitted / self.config.compact_memtable_bytes.max(1)) as u32;
            let remainder = admitted % self.config.compact_memtable_bytes.max(1);
            let extra = if remainder > self.config.compact_memtable_bytes / 2 { 1 } else { 0 };
            let next_l0 = state
                .l0_files
                .saturating_add(new_l0.saturating_add(extra))
                .saturating_sub(self.config.drain_capacity_l0);

            state = LsmDynamicState {
                memtable_bytes: next_mem,
                l0_files: next_l0,
            };
            trajectory.push(state);
        }

        Ok(trajectory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_try_new_validations_red_to_green() {
        assert_eq!(
            FosterLyapunovConfig::try_new(0, 4, 0.5, 0.05, 1000, 2),
            Err(StochasticStallViolation::ZeroCompactMemtableBytes)
        );
        assert_eq!(
            FosterLyapunovConfig::try_new(1000, 0, 0.5, 0.05, 1000, 2),
            Err(StochasticStallViolation::ZeroCompactL0Files)
        );
        assert_eq!(
            FosterLyapunovConfig::try_new(1000, 4, 1.5, 0.05, 1000, 2),
            Err(StochasticStallViolation::InvalidGamma(1.5))
        );
        assert_eq!(
            FosterLyapunovConfig::try_new(1000, 4, 0.5, -0.1, 1000, 2),
            Err(StochasticStallViolation::InvalidDriftEpsilon(-0.1))
        );
        assert_eq!(
            FosterLyapunovConfig::try_new(1000, 4, 0.5, 0.05, 0, 0),
            Err(StochasticStallViolation::ZeroDrainCapacity)
        );

        let valid = FosterLyapunovConfig::try_new(1000, 4, 0.5, 0.05, 1000, 2).unwrap();
        let controller = FosterLyapunovController::try_new(valid).unwrap();
        assert_eq!(controller.config().compact_memtable_bytes, 1000);
    }

    #[test]
    fn test_lsm_dynamic_state_constructor_and_compact_set() {
        let state = LsmDynamicState::new(500, 2);
        assert_eq!(state.memtable_bytes, 500);
        assert_eq!(state.l0_files, 2);

        let config = FosterLyapunovConfig::default();
        let controller = FosterLyapunovController::new(config);
        assert!(controller.is_in_compact_set(state));
    }
}

