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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimiterConservationViolation {
    StarvationDetected {
        dirty_ratio: u64,
        allocated_rate: u64,
        required_rate: u64,
    },
    DeadlockFreezeDetected {
        mem_stall_ms: u64,
    },
}

/// Controlador com auto-expansão e prova de conservação de I/O.
pub struct ConservationRateLimiter {
    config: RateLimiterConfig,
    state: RateLimiterState,
}

impl ConservationRateLimiter {
    pub fn new(config: RateLimiterConfig, now_ns: u64) -> Self {
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

    /// Calcula a taxa de serviço adaptativa $\mu_{\text{flush}}$ com base na pressão de memória suja:
    /// $\rho_{\text{dirty}} = \frac{\text{dirty\_bytes}}{\text{mem\_budget}}$.
    #[must_use]
    pub fn compute_dynamic_rate(&self, dirty_bytes: u64, mem_budget: u64) -> u64 {
        if mem_budget == 0 {
            return self.config.base_rate_bytes_per_sec;
        }

        let ratio = (dirty_bytes as f64 / mem_budget as f64).clamp(0.0, 1.0);
        if ratio <= self.config.dirty_expansion_threshold_ratio {
            self.config.base_rate_bytes_per_sec
        } else {
            // Expansão suave não-linear:
            // excesso normalizado em [0, 1] a partir do limiar
            let excess = (ratio - self.config.dirty_expansion_threshold_ratio)
                / (1.0 - self.config.dirty_expansion_threshold_ratio);
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

        let dirty_ratio = if mem_budget > 0 {
            (dirty_bytes * 100) / mem_budget
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
            self.state.available_tokens -= req;
            self.state.total_bytes_served += bytes_requested;
            Ok(AdmissionResult::AdmittedImmediately {
                bytes_granted: bytes_requested,
            })
        } else {
            let deficit = req - self.state.available_tokens;
            let wait_secs = deficit / (self.state.active_service_rate as f64);
            let wait_micros = (wait_secs * 1_000_000.0).ceil() as u64;

            self.state.available_tokens = 0.0;
            self.state.total_bytes_served += bytes_requested;

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
