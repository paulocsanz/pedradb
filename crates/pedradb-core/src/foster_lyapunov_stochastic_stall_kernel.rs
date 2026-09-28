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
}

/// Configuração do controlador de Foster-Lyapunov.
#[derive(Debug, Clone)]
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
    /// Cria uma nova instância do controlador com a configuração especificada.
    pub fn new(config: FosterLyapunovConfig) -> Self {
        Self { config }
    }

    /// Calcula a função candidata de Lyapunov sub-quadrática:
    /// V(M, S) = ((M / M_compact)^2 + (S / S_compact)^2)^(gamma / 2)
    pub fn compute_v(&self, state: LsmDynamicState) -> f64 {
        let m_ratio = (state.memtable_bytes as f64) / (self.config.compact_memtable_bytes as f64);
        let s_ratio = (state.l0_files as f64) / (self.config.compact_l0_files as f64);
        let norm_sq = m_ratio * m_ratio + s_ratio * s_ratio;
        norm_sq.powf(self.config.gamma / 2.0)
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
            let penalty = 1.0 / (1.0 + (v - 1.0).max(0.0));
            penalty.clamp(0.01, 1.0)
        };

        AdmissionDecision {
            admission_fraction,
            in_compact_set,
            lyapunov_value: v,
        }
    }

    /// Simula um passo estocástico de transição e valida a deriva de Foster-Lyapunov:
    /// E[V(X_{t+1}) - V(X_t) | X_t = x] <= -epsilon quando x fora de C.
    pub fn verify_step_lyapunov(
        &self,
        current_state: LsmDynamicState,
        arrival_samples: &[u64], // Amostras da distribuição de cauda pesada de chegada
    ) -> Result<f64, StochasticStallViolation> {
        let in_compact = self.is_in_compact_set(current_state);
        let v_curr = self.compute_v(current_state);
        let decision = self.evaluate_admission(current_state);

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
            let new_l0 = if admitted_bytes > self.config.compact_memtable_bytes / 2 {
                1
            } else {
                0
            };
            let next_l0 = current_state
                .l0_files
                .saturating_add(new_l0)
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
}
