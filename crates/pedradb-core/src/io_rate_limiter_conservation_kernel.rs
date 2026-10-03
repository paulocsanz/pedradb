//! RFC-0291 Fronteira 7: Cálculo de Redes e Conservação de Quota no I/O Rate Limiter.
//!
//! Garante matematicamente através de Network Calculus determinístico que a taxa
//! de serviço do Flush e da compactação de emergência se expanda monotonicamente
//! sob saturação de memória suja, impedindo starvation e congelamento da base.

#![forbid(unsafe_code)]

/// Configuração determinística do I/O Rate Limiter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimiterConfig {
    /// Quota base configurada de I/O em bytes/segundo.
    pub base_rate_bytes_per_sec: u64,
    /// Capacidade máxima do token bucket para acomodar rajadas $\sigma$.
    pub burst_capacity_bytes: u64,
    /// Limiar de memória suja onde se inicia a expansão dinâmica (ex: 70% do budget de RAM).
    pub dirty_expansion_threshold_ratio: f64,
    /// Capacidade máxima absoluta do canal de disco físico.
    pub max_hardware_bandwidth_bytes_per_sec: u64,
    /// Expoente de aceleração não-linear $\alpha$ para amortecimento de saturação.
    pub acceleration_exponent: f64,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            base_rate_bytes_per_sec: 100_000_000, // 100 MB/s
            burst_capacity_bytes: 32_000_000,     // 32 MB burst
            dirty_expansion_threshold_ratio: 0.70, // Inicia em 70%
            max_hardware_bandwidth_bytes_per_sec: 1_000_000_000, // 1 GB/s NVMe
            acceleration_exponent: 2.0,
        }
    }
}

impl RateLimiterConfig {
    /// Safely validates and constructs a RateLimiterConfig, rejecting zero quotas and invalid ratios.
    pub fn try_new(
        base_rate_bytes_per_sec: u64,
        burst_capacity_bytes: u64,
        dirty_expansion_threshold_ratio: f64,
        max_hardware_bandwidth_bytes_per_sec: u64,
        acceleration_exponent: f64,
    ) -> Result<Self, RateLimiterConservationViolation> {
        if base_rate_bytes_per_sec == 0 {
            return Err(RateLimiterConservationViolation::ZeroBaseRate);
        }
        if burst_capacity_bytes == 0 {
            return Err(RateLimiterConservationViolation::ZeroBurstCapacity);
        }
        if max_hardware_bandwidth_bytes_per_sec == 0 {
            return Err(RateLimiterConservationViolation::ZeroMaxHardwareBandwidth);
        }
        if max_hardware_bandwidth_bytes_per_sec < base_rate_bytes_per_sec {
            return Err(RateLimiterConservationViolation::MaxBandwidthBelowBaseRate {
                max_bw: max_hardware_bandwidth_bytes_per_sec,
                base_rate: base_rate_bytes_per_sec,
            });
        }
        if !dirty_expansion_threshold_ratio.is_finite()
            || dirty_expansion_threshold_ratio < 0.0
            || dirty_expansion_threshold_ratio >= 1.0
        {
            return Err(RateLimiterConservationViolation::InvalidThresholdRatio(
                dirty_expansion_threshold_ratio,
            ));
        }
        if !acceleration_exponent.is_finite() || acceleration_exponent <= 0.0 {
            return Err(RateLimiterConservationViolation::InvalidAccelerationExponent(
                acceleration_exponent,
            ));
        }

        Ok(Self {
            base_rate_bytes_per_sec,
            burst_capacity_bytes,
            dirty_expansion_threshold_ratio,
            max_hardware_bandwidth_bytes_per_sec,
            acceleration_exponent,
        })
    }
}

/// Estado dinâmico do limitador de taxa.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimiterState {
    pub available_tokens: f64,
    pub last_refill_timestamp_ns: u64,
    pub active_service_rate: u64,
    pub total_bytes_served: u64,
}

/// Resultado do pedido de admissão de I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionResult {
    /// Admissão imediata sem espera.
    AdmittedImmediately { bytes_granted: u64 },
    /// Admissão condicionada a atraso temporal bounded.
    DelayedWithWait {
        bytes_granted: u64,
        wait_duration_micros: u64,
    },
}

/// Violação da Lei de Conservação de Quota e Starvation.
#[derive(Debug, Clone, PartialEq)]
pub enum RateLimiterConservationViolation {
    StarvationDetected {
        dirty_ratio: u64,
        allocated_rate: u64,
        required_rate: u64,
    },
    DeadlockFreezeDetected {
        mem_stall_ms: u64,
    },
    ZeroBaseRate,
    ZeroBurstCapacity,
    ZeroMaxHardwareBandwidth,
    InvalidThresholdRatio(f64),
    InvalidAccelerationExponent(f64),
    MaxBandwidthBelowBaseRate {
        max_bw: u64,
        base_rate: u64,
    },
}

impl std::fmt::Display for RateLimiterConservationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StarvationDetected { dirty_ratio, allocated_rate, required_rate } => write!(
                f,
                "Starvation detected at {dirty_ratio}% dirty memory: allocated rate {allocated_rate} < required {required_rate}"
            ),
            Self::DeadlockFreezeDetected { mem_stall_ms } => {
                write!(f, "Deadlock freeze detected after {mem_stall_ms}ms memory stall")
            }
            Self::ZeroBaseRate => write!(f, "Base rate cannot be zero"),
            Self::ZeroBurstCapacity => write!(f, "Burst capacity cannot be zero"),
            Self::ZeroMaxHardwareBandwidth => write!(f, "Max hardware bandwidth cannot be zero"),
            Self::InvalidThresholdRatio(ratio) => write!(f, "Invalid threshold ratio: {ratio} (must be in [0.0, 1.0))"),
            Self::InvalidAccelerationExponent(exp) => write!(f, "Invalid acceleration exponent: {exp} (must be finite > 0.0)"),
            Self::MaxBandwidthBelowBaseRate { max_bw, base_rate } => write!(
                f,
                "Max hardware bandwidth ({max_bw}) cannot be less than base rate ({base_rate})"
            ),
        }
    }
}

impl std::error::Error for RateLimiterConservationViolation {}

/// Controlador com auto-expansão e prova de conservação de I/O.
pub struct ConservationRateLimiter {
    config: RateLimiterConfig,
    state: RateLimiterState,
}

impl ConservationRateLimiter {
    pub fn new(mut config: RateLimiterConfig, now_ns: u64) -> Self {
        // Sanitize config
        if !config.dirty_expansion_threshold_ratio.is_finite() || config.dirty_expansion_threshold_ratio < 0.0 {
            config.dirty_expansion_threshold_ratio = 0.0;
        } else if config.dirty_expansion_threshold_ratio >= 1.0 {
            // Clamp strictly below 1.0 to guarantee dynamic expansion capacity before saturation
            config.dirty_expansion_threshold_ratio = 0.95;
        }
        if !config.acceleration_exponent.is_finite() || config.acceleration_exponent <= 0.0 {
            config.acceleration_exponent = 1.0;
        }
        if config.max_hardware_bandwidth_bytes_per_sec < config.base_rate_bytes_per_sec {
            config.max_hardware_bandwidth_bytes_per_sec = config.base_rate_bytes_per_sec;
        }

        let initial_tokens = config.burst_capacity_bytes as f64;
        let initial_rate = config.base_rate_bytes_per_sec;
        Self {
            config,
            state: RateLimiterState {
                available_tokens: initial_tokens,
                last_refill_timestamp_ns: now_ns,
                active_service_rate: initial_rate,
                total_bytes_served: 0,
            },
        }
    }

    /// Safely constructs a ConservationRateLimiter validating config.
    pub fn try_new(config: RateLimiterConfig, now_ns: u64) -> Result<Self, RateLimiterConservationViolation> {
        RateLimiterConfig::try_new(
            config.base_rate_bytes_per_sec,
            config.burst_capacity_bytes,
            config.dirty_expansion_threshold_ratio,
            config.max_hardware_bandwidth_bytes_per_sec,
            config.acceleration_exponent,
        )?;
        Ok(Self::new(config, now_ns))
    }

    /// Retorna os tokens atualmente disponíveis.
    #[must_use]
    pub fn available_tokens(&self) -> f64 {
        self.state.available_tokens
    }

    /// Retorna o total de bytes servidos.
    #[must_use]
    pub fn total_bytes_served(&self) -> u64 {
        self.state.total_bytes_served
    }

    /// Retorna o timestamp do último refill em nanosegundos.
    #[must_use]
    pub fn last_refill_timestamp_ns(&self) -> u64 {
        self.state.last_refill_timestamp_ns
    }

    /// Calcula a taxa de serviço adaptativa $\mu_{\text{flush}}$ com base na pressão de memória suja:
    /// $\rho_{\text{dirty}} = \frac{\text{dirty\_bytes}}{\text{mem\_budget}}$.
    #[must_use]
    pub fn compute_dynamic_rate(&self, dirty_bytes: u64, mem_budget: u64) -> u64 {
        if mem_budget == 0 {
            return self.config.base_rate_bytes_per_sec;
        }

        let ratio = (dirty_bytes as f64 / mem_budget as f64).clamp(0.0, 1.0);
        let threshold = self.config.dirty_expansion_threshold_ratio.clamp(0.0, 0.9999);
        if ratio <= threshold {
            self.config.base_rate_bytes_per_sec
        } else {
            // Expansão suave não-linear:
            // excesso normalizado em [0, 1] a partir do limiar
            let denom = 1.0 - threshold;
            let excess = if denom > f64::EPSILON {
                ((ratio - threshold) / denom).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let factor = excess.powf(self.config.acceleration_exponent);

            let headroom = self.config.max_hardware_bandwidth_bytes_per_sec
                .saturating_sub(self.config.base_rate_bytes_per_sec);
            let boost = (headroom as f64 * factor) as u64;

            (self.config.base_rate_bytes_per_sec + boost)
                .min(self.config.max_hardware_bandwidth_bytes_per_sec)
        }
    }

    /// Reabastece tokens de acordo com o tempo decorrido e a taxa dinâmica ativa.
    pub fn refill(&mut self, now_ns: u64, dirty_bytes: u64, mem_budget: u64) {
        if now_ns <= self.state.last_refill_timestamp_ns {
            // Monotonic time invariant: ignore backwards time steps or duplicate timestamps
            return;
        }

        let elapsed_ns = now_ns - self.state.last_refill_timestamp_ns;
        let new_rate = self.compute_dynamic_rate(dirty_bytes, mem_budget);
        self.state.active_service_rate = new_rate;

        let tokens_to_add = (elapsed_ns as f64 / 1_000_000_000.0) * (new_rate as f64);
        let max_capacity = self.config.burst_capacity_bytes as f64;

        self.state.available_tokens = (self.state.available_tokens + tokens_to_add).min(max_capacity);
        self.state.last_refill_timestamp_ns = now_ns;
    }

    /// Requisita permissão de transmissão para `bytes_requested`.
    pub fn request_admission(
        &mut self,
        bytes_requested: u64,
        now_ns: u64,
        dirty_bytes: u64,
        mem_budget: u64,
    ) -> Result<AdmissionResult, RateLimiterConservationViolation> {
        self.refill(now_ns, dirty_bytes, mem_budget);

        if bytes_requested == 0 {
            return Ok(AdmissionResult::AdmittedImmediately { bytes_granted: 0 });
        }

        let dirty_ratio = if mem_budget > 0 {
            let ratio_f = (dirty_bytes as f64 / mem_budget as f64) * 100.0;
            ratio_f.clamp(0.0, 100.0) as u64
        } else {
            0
        };

        // Auditoria de Starvation: se a memória suja está em >= 95%, a taxa de serviço DEVE ser
        // superior a 80% da banda máxima do hardware
        if dirty_ratio >= 95 {
            let threshold_rate = (self.config.max_hardware_bandwidth_bytes_per_sec * 80) / 100;
            if self.state.active_service_rate < threshold_rate {
                return Err(RateLimiterConservationViolation::StarvationDetected {
                    dirty_ratio,
                    allocated_rate: self.state.active_service_rate,
                    required_rate: threshold_rate,
                });
            }
        }

        let req = bytes_requested as f64;
        if self.state.available_tokens >= req {
            self.state.available_tokens = (self.state.available_tokens - req).max(0.0);
            self.state.total_bytes_served = self.state.total_bytes_served.saturating_add(bytes_requested);
            Ok(AdmissionResult::AdmittedImmediately {
                bytes_granted: bytes_requested,
            })
        } else {
            let deficit = req - self.state.available_tokens;
            let effective_rate = self.state.active_service_rate.max(1) as f64;
            let wait_secs = deficit / effective_rate;
            let wait_micros = (wait_secs * 1_000_000.0).ceil() as u64;

            self.state.available_tokens = 0.0;
            self.state.total_bytes_served = self.state.total_bytes_served.saturating_add(bytes_requested);

            Ok(AdmissionResult::DelayedWithWait {
                bytes_granted: bytes_requested,
                wait_duration_micros: wait_micros,
            })
        }
    }

    #[must_use]
    pub fn current_service_rate(&self) -> u64 {
        self.state.active_service_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_try_new_validations_red_to_green() {
        assert_eq!(
            RateLimiterConfig::try_new(0, 1000, 0.7, 10000, 2.0),
            Err(RateLimiterConservationViolation::ZeroBaseRate)
        );
        assert_eq!(
            RateLimiterConfig::try_new(100, 0, 0.7, 10000, 2.0),
            Err(RateLimiterConservationViolation::ZeroBurstCapacity)
        );
        assert_eq!(
            RateLimiterConfig::try_new(100, 1000, 0.7, 0, 2.0),
            Err(RateLimiterConservationViolation::ZeroMaxHardwareBandwidth)
        );
        assert_eq!(
            RateLimiterConfig::try_new(1000, 1000, 0.7, 500, 2.0),
            Err(RateLimiterConservationViolation::MaxBandwidthBelowBaseRate {
                max_bw: 500,
                base_rate: 1000
            })
        );
        assert_eq!(
            RateLimiterConfig::try_new(100, 1000, 1.5, 1000, 2.0),
            Err(RateLimiterConservationViolation::InvalidThresholdRatio(1.5))
        );
        assert_eq!(
            RateLimiterConfig::try_new(100, 1000, 0.7, 1000, -1.0),
            Err(RateLimiterConservationViolation::InvalidAccelerationExponent(-1.0))
        );
    }

    #[test]
    fn test_overflow_protection_in_dirty_bytes_calculation() {
        let config = RateLimiterConfig::default();
        let mut limiter = ConservationRateLimiter::new(config, 0);

        // dirty_bytes close to u64::MAX should not panic with overflow in multiplication
        let res = limiter.request_admission(1000, 10, u64::MAX / 2, u64::MAX);
        assert!(res.is_ok());
    }

    #[test]
    fn test_token_depletion_and_refill_precision() {
        let config = RateLimiterConfig {
            base_rate_bytes_per_sec: 10_000,
            burst_capacity_bytes: 1_000,
            dirty_expansion_threshold_ratio: 0.7,
            max_hardware_bandwidth_bytes_per_sec: 50_000,
            acceleration_exponent: 2.0,
        };
        let mut limiter = ConservationRateLimiter::new(config, 100);
        assert_eq!(limiter.available_tokens(), 1_000.0);

        let adm = limiter.request_admission(500, 100, 0, 100).unwrap();
        assert_eq!(adm, AdmissionResult::AdmittedImmediately { bytes_granted: 500 });
        assert_eq!(limiter.available_tokens(), 500.0);
        assert_eq!(limiter.total_bytes_served(), 500);
    }
}

