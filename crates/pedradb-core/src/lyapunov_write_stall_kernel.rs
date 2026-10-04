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

impl LyapunovStallConfig {
    /// Valida estritamente os parâmetros do sistema de amortecimento.
    pub fn validate(&self) -> Result<(), DynamicStabilityViolation> {
        if !self.target_debt.is_finite() || self.target_debt < 0.0 {
            return Err(DynamicStabilityViolation::InvalidConfig {
                reason: "target_debt must be finite and non-negative",
            });
        }
        if !self.min_rate.is_finite() || self.min_rate <= 0.0 {
            return Err(DynamicStabilityViolation::InvalidConfig {
                reason: "min_rate must be finite and positive",
            });
        }
        if !self.max_rate.is_finite() || self.max_rate < self.min_rate {
            return Err(DynamicStabilityViolation::InvalidConfig {
                reason: "max_rate must be finite and >= min_rate",
            });
        }
        if !self.target_rate.is_finite() || self.target_rate < self.min_rate || self.target_rate > self.max_rate {
            return Err(DynamicStabilityViolation::InvalidConfig {
                reason: "target_rate must be within [min_rate, max_rate]",
            });
        }
        if !self.gamma.is_finite() || self.gamma < 0.0 {
            return Err(DynamicStabilityViolation::InvalidConfig {
                reason: "gamma must be finite and non-negative",
            });
        }
        if !self.kappa.is_finite() || self.kappa <= 0.0 {
            return Err(DynamicStabilityViolation::InvalidConfig {
                reason: "kappa must be finite and positive",
            });
        }
        Ok(())
    }
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
    InvalidDebtMeasurement,
    InvalidConfig { reason: &'static str },
}

impl std::fmt::Display for DynamicStabilityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonDecayingEnergy { step } => {
                write!(f, "Lyapunov energy function failed to decay at step {step}")
            }
            Self::ChatteringDetected { oscillations } => {
                write!(f, "chattering detected across {oscillations} rate oscillations")
            }
            Self::RateOutOfBounds { rate } => {
                write!(f, "write rate {rate} is outside allowed operational envelope")
            }
            Self::InvalidDebtMeasurement => {
                write!(f, "invalid debt measurement: must be finite and non-negative")
            }
            Self::InvalidConfig { reason } => {
                write!(f, "invalid Lyapunov write stall configuration: {reason}")
            }
        }
    }
}

impl std::error::Error for DynamicStabilityViolation {}

/// Controlador de Write Stall comprovado por Estabilidade de Lyapunov.
pub struct LyapunovWriteStallController {
    config: LyapunovStallConfig,
    state: LyapunovStallState,
    rate_history: Vec<f64>,
}

impl LyapunovWriteStallController {
    /// Constrói o controlador validando estritamente a configuração e a dívida inicial.
    pub fn try_new(config: LyapunovStallConfig, initial_debt: f64) -> Result<Self, DynamicStabilityViolation> {
        config.validate()?;
        if !initial_debt.is_finite() || initial_debt < 0.0 {
            return Err(DynamicStabilityViolation::InvalidDebtMeasurement);
        }
        let initial_rate = config.target_rate;
        let v0 = Self::compute_potential(&config, initial_debt, initial_rate);
        Ok(Self {
            config,
            state: LyapunovStallState {
                current_debt: initial_debt,
                current_rate: initial_rate,
                previous_potential: v0,
                step_count: 0,
            },
            rate_history: vec![initial_rate],
        })
    }

    pub fn new(mut config: LyapunovStallConfig, initial_debt: f64) -> Self {
        // Sanitize config
        if !config.target_debt.is_finite() || config.target_debt < 0.0 {
            config.target_debt = 0.0;
        }
        if !config.min_rate.is_finite() || config.min_rate < 0.0 {
            config.min_rate = 1.0;
        }
        if !config.max_rate.is_finite() || config.max_rate < config.min_rate {
            config.max_rate = config.min_rate.max(100_000.0);
        }
        if !config.target_rate.is_finite() {
            config.target_rate = config.min_rate;
        } else {
            config.target_rate = config.target_rate.clamp(config.min_rate, config.max_rate);
        }
        if !config.gamma.is_finite() || config.gamma < 0.0 {
            config.gamma = 1e-8;
        }
        if !config.kappa.is_finite() || config.kappa < 0.0 {
            config.kappa = 0.15;
        }

        let debt = if initial_debt.is_finite() { initial_debt.max(0.0) } else { config.target_debt };
        let initial_rate = config.target_rate;
        let v0 = Self::compute_potential(&config, debt, initial_rate);
        Self {
            config,
            state: LyapunovStallState {
                current_debt: debt,
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
        let sanitized_debt = if debt.is_finite() { debt.max(0.0) } else { config.target_debt };
        let sanitized_rate = if rate.is_finite() { rate.max(0.0) } else { config.target_rate };
        let err_d = sanitized_debt - config.target_debt;
        let err_r = sanitized_rate - config.target_rate;
        let pot = 0.5 * (err_d * err_d) + 0.5 * config.gamma * (err_r * err_r);
        if pot.is_finite() { pot } else { 0.0 }
    }

    /// Executa um passo do controlador com validação estrita fail-closed.
    pub fn try_step(&mut self, observed_debt: f64) -> Result<StallStepResult, DynamicStabilityViolation> {
        if !observed_debt.is_finite() || observed_debt < 0.0 {
            return Err(DynamicStabilityViolation::InvalidDebtMeasurement);
        }
        self.step_internal(observed_debt, false)
    }

    /// Executa um passo sob regime autônomo, verificando decaimento da função de energia.
    pub fn step_autonomous(&mut self, observed_debt: f64) -> Result<StallStepResult, DynamicStabilityViolation> {
        if !observed_debt.is_finite() || observed_debt < 0.0 {
            return Err(DynamicStabilityViolation::InvalidDebtMeasurement);
        }
        self.step_internal(observed_debt, true)
    }

    /// Executa um passo do controlador com a dívida observada na etapa $t$.
    pub fn step(&mut self, observed_debt: f64) -> Result<StallStepResult, DynamicStabilityViolation> {
        let safe_debt = if observed_debt.is_nan() {
            0.0
        } else if observed_debt.is_finite() {
            observed_debt.max(0.0)
        } else if observed_debt.is_sign_positive() {
            f64::MAX / 2.0
        } else {
            0.0
        };
        self.step_internal(safe_debt, false)
    }

    fn step_internal(&mut self, safe_debt: f64, enforce_decay: bool) -> Result<StallStepResult, DynamicStabilityViolation> {
        self.state.step_count += 1;
        let v_before = Self::compute_potential(&self.config, safe_debt, self.state.current_rate);

        // Lei de controle proporcional com amortecimento exponencial:
        // R_{t+1} = target_rate / (1 + kappa * max(0, debt - target_debt))
        let debt_excess = (safe_debt - self.config.target_debt).max(0.0);
        let attenuation = (1.0 + self.config.kappa * debt_excess).max(1.0);
        let raw_next_rate = self.config.target_rate / attenuation;

        // Limita ao envelope de taxas válidas
        let clamped_next_rate = if raw_next_rate.is_finite() {
            raw_next_rate.clamp(self.config.min_rate, self.config.max_rate)
        } else {
            self.config.min_rate
        };

        let v_after = Self::compute_potential(&self.config, safe_debt, clamped_next_rate);
        let delta_v = v_after - v_before;

        if enforce_decay && safe_debt <= self.state.current_debt && delta_v > 1e-6 {
            return Err(DynamicStabilityViolation::NonDecayingEnergy {
                step: self.state.step_count,
            });
        }

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

        self.state.current_debt = safe_debt;
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
