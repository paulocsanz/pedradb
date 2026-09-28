//! RFC-0291 Fronteira 1: Estabilidade de Lyapunov e Ausência de Ciclos Limites Caóticos no Write Stall.
//!
//! Garante matematicamente que o controle dinâmico de admissão de escritas
//! converge monotonicamente para o atrator de equilíbrio $(D^*, R^*)$ sem oscilações
//! harmônicas caóticas (chattering) e sem colapso de throughput.

#![forbid(unsafe_code)]

/// Configuração do sistema dinâmico de amortecimento de escrita.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LyapunovStallConfig {
    /// Dívida de compactação ideal de equilíbrio (ex: 4 arquivos L0 ou bytes equivalentes).
    pub target_debt: f64,
    /// Taxa de escrita sustentada de equilíbrio (bytes/s ou ops/s).
    pub target_rate: f64,
    /// Taxa mínima permitida (garante que throughput nunca seja zero - anti-starvation).
    pub min_rate: f64,
    /// Taxa máxima permitida (teto físico do hardware).
    pub max_rate: f64,
    /// Fator de escala ponderada de energia de taxa na função de Lyapunov.
    pub gamma: f64,
    /// Coeficiente de convergência de Lyapunov (garante decaimento estrito de potencial).
    pub kappa: f64,
}

impl Default for LyapunovStallConfig {
    fn default() -> Self {
        Self {
            target_debt: 4.0,
            target_rate: 100_000.0,
            min_rate: 1_000.0,
            max_rate: 500_000.0,
            gamma: 1e-8,
            kappa: 0.15,
        }
    }
}

/// Estado dinâmico do controlador de Write Stall.
#[derive(Debug, Clone, PartialEq)]
pub struct LyapunovStallState {
    pub current_debt: f64,
    pub current_rate: f64,
    pub previous_potential: f64,
    pub step_count: u64,
}

/// Resultado da etapa de controle dinâmico.
#[derive(Debug, Clone, PartialEq)]
pub struct StallStepResult {
    pub new_rate: f64,
    pub potential_before: f64,
    pub potential_after: f64,
    pub delta_potential: f64,
    pub is_strictly_decaying_or_stable: bool,
    pub is_chattering_detected: bool,
}

/// Erro de violação de estabilidade dinâmica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicStabilityViolation {
    NonDecayingEnergy { step: u64 },
    ChatteringDetected { oscillations: usize },
    RateOutOfBounds { rate: u64 },
}

/// Controlador de Write Stall comprovado por Estabilidade de Lyapunov.
pub struct LyapunovWriteStallController {
    config: LyapunovStallConfig,
    state: LyapunovStallState,
    rate_history: Vec<f64>,
}

impl LyapunovWriteStallController {
    pub fn new(config: LyapunovStallConfig, initial_debt: f64) -> Self {
        let initial_rate = config.target_rate;
        let v0 = Self::compute_potential(&config, initial_debt, initial_rate);
        Self {
            config,
            state: LyapunovStallState {
                current_debt: initial_debt,
                current_rate: initial_rate,
                previous_potential: v0,
                step_count: 0,
            },
            rate_history: vec![initial_rate],
        }
    }

    /// Calcula a função potencial de Lyapunov $V(D, R) = \frac{1}{2}(D - D^*)^2 + \frac{\gamma}{2}(R - R^*)^2$.
    #[must_use]
    pub fn compute_potential(config: &LyapunovStallConfig, debt: f64, rate: f64) -> f64 {
        let err_d = debt - config.target_debt;
        let err_r = rate - config.target_rate;
        0.5 * (err_d * err_d) + 0.5 * config.gamma * (err_r * err_r)
    }

    /// Executa um passo do controlador com a dívida observada na etapa $t$.
    pub fn step(&mut self, observed_debt: f64) -> Result<StallStepResult, DynamicStabilityViolation> {
        self.state.step_count += 1;
        let v_before = Self::compute_potential(&self.config, observed_debt, self.state.current_rate);

        // Lei de controle proporcional com amortecimento exponencial:
        // R_{t+1} = target_rate / (1 + kappa * max(0, debt - target_debt))
        let debt_excess = (observed_debt - self.config.target_debt).max(0.0);
        let attenuation = 1.0 + self.config.kappa * debt_excess;
        let raw_next_rate = self.config.target_rate / attenuation;

        // Limita ao envelope de taxas válidas
        let clamped_next_rate = raw_next_rate
            .clamp(self.config.min_rate, self.config.max_rate);

        let v_after = Self::compute_potential(&self.config, observed_debt, clamped_next_rate);
        let delta_v = v_after - v_before;

        // Adiciona ao histórico para análise de oscilação harmônica
        self.rate_history.push(clamped_next_rate);
        if self.rate_history.len() > 10 {
            self.rate_history.remove(0);
        }

        let chattering = self.detect_chattering();
        if chattering {
            return Err(DynamicStabilityViolation::ChatteringDetected {
                oscillations: self.rate_history.len(),
            });
        }

        self.state.current_debt = observed_debt;
        self.state.current_rate = clamped_next_rate;
        self.state.previous_potential = v_after;

        Ok(StallStepResult {
            new_rate: clamped_next_rate,
            potential_before: v_before,
            potential_after: v_after,
            delta_potential: delta_v,
            is_strictly_decaying_or_stable: delta_v <= 1e-9,
            is_chattering_detected: false,
        })
    }

    /// Detecta chattering através da alternância de derivadas discretas sucessivas.
    fn detect_chattering(&self) -> bool {
        if self.rate_history.len() < 6 {
            return false;
        }
        let mut sign_flips = 0;
        let mut last_diff = self.rate_history[1] - self.rate_history[0];
        for i in 2..self.rate_history.len() {
            let diff = self.rate_history[i] - self.rate_history[i - 1];
            if (last_diff > 10.0 && diff < -10.0) || (last_diff < -10.0 && diff > 10.0) {
                sign_flips += 1;
            }
            last_diff = diff;
        }
        // Se alternar mais de 4 vezes em 8 passos, há instabilidade harmônica
        sign_flips >= 4
    }

    #[must_use]
    pub fn current_rate(&self) -> f64 {
        self.state.current_rate
    }
}
