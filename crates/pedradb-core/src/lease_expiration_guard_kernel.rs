//! Pilar 1: Oráculo de Validade de Leases sob Suspensão de Runtime (RFC-0285).
//!
//! Previne leituras stale e escritas por líderes defasados quando uma máquina
//! virtual sofre suspensão (VM freeze) ou quando o runtime assíncrono (Tokio)
//! sofre de inanição profunda de workers.

/// Erros de configuração e validação de lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseConfigError {
    /// Duração do lease não pode ser zero.
    ZeroDuration,
    /// Margem de segurança (slack + 2*drift) excede ou iguala a duração nominal.
    SafetyMarginExceedsDuration { safety_margin_ns: u64, duration_ns: u64 },
    /// Timestamp de concessão não pode ser zero.
    ZeroGrantedAt,
}

impl std::fmt::Display for LeaseConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDuration => write!(f, "lease duration cannot be zero"),
            Self::SafetyMarginExceedsDuration { safety_margin_ns, duration_ns } => {
                write!(f, "safety margin {safety_margin_ns}ns >= duration {duration_ns}ns")
            }
            Self::ZeroGrantedAt => write!(f, "granted_at timestamp cannot be zero"),
        }
    }
}

impl std::error::Error for LeaseConfigError {}

/// Parâmetros de segurança para avaliação de lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseConfig {
    /// Duração nominal do lease concedido pela malha (em nanossegundos).
    pub duration_ns: u64,
    /// Margem de folga para preempção do escalonador do SO (slack, em nanossegundos).
    pub slack_ns: u64,
    /// Tolerância máxima estimada para deriva de relógio (drift, em nanossegundos).
    pub max_drift_ns: u64,
}

impl LeaseConfig {
    /// Cria uma configuração de lease validando invariantes temporais.
    pub fn try_new(duration_ns: u64, slack_ns: u64, max_drift_ns: u64) -> Result<Self, LeaseConfigError> {
        if duration_ns == 0 {
            return Err(LeaseConfigError::ZeroDuration);
        }
        let safety_margin = slack_ns.saturating_add(max_drift_ns.saturating_mul(2));
        if safety_margin >= duration_ns {
            return Err(LeaseConfigError::SafetyMarginExceedsDuration {
                safety_margin_ns: safety_margin,
                duration_ns,
            });
        }
        Ok(Self {
            duration_ns,
            slack_ns,
            max_drift_ns,
        })
    }
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            duration_ns: 5_000_000_000,    // 5 segundos
            slack_ns: 500_000_000,        // 500 ms de folga de SO
            max_drift_ns: 50_000_000,     // 50 ms de drift
        }
    }
}

/// Estado do lease emitido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseGrant {
    /// Timestamp monotônico de início da concessão (em nanossegundos).
    pub granted_at_ns: u64,
    /// Configuração do lease.
    pub config: LeaseConfig,
}

impl LeaseGrant {
    /// Cria uma concessão validando timestamp e margem de segurança.
    pub fn try_new(granted_at_ns: u64, config: LeaseConfig) -> Result<Self, LeaseConfigError> {
        if granted_at_ns == 0 {
            return Err(LeaseConfigError::ZeroGrantedAt);
        }
        let safety_margin = config.slack_ns.saturating_add(config.max_drift_ns.saturating_mul(2));
        if safety_margin >= config.duration_ns {
            return Err(LeaseConfigError::SafetyMarginExceedsDuration {
                safety_margin_ns: safety_margin,
                duration_ns: config.duration_ns,
            });
        }
        Ok(Self {
            granted_at_ns,
            config,
        })
    }
}

/// Decisão de admissão sob lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseVerdict {
    /// O lease é estritamente válido dentro da janela segura.
    Valid {
        /// Tempo restante antes da margem de segurança expirar.
        remaining_safe_ns: u64,
    },
    /// O lease está na margem de risco (dentro de slack/drift); fail-closed.
    InGraceZone,
    /// O lease expirou completamente; qualquer operação deve ser abortada.
    Expired,
    /// Inversão temporal detectada (clock jump ou anomalia monotônica).
    TimeInversionDetected,
}

/// Avalia se uma operação de leitura/escrita pode ser despachada com segurança sob o lease atual.
#[must_use]
pub fn check_lease_validity(grant: &LeaseGrant, current_time_ns: u64) -> LeaseVerdict {
    if current_time_ns < grant.granted_at_ns {
        return LeaseVerdict::TimeInversionDetected;
    }
    let elapsed = current_time_ns.saturating_sub(grant.granted_at_ns);
    let total_safety_margin = grant.config.slack_ns.saturating_add(grant.config.max_drift_ns.saturating_mul(2));
    
    // Se o elapsed já ultrapassou a duração total, expirou categoricamente
    if elapsed >= grant.config.duration_ns {
        return LeaseVerdict::Expired;
    }

    // Limiar de segurança estrito: duração menos margem de folga
    let safe_threshold = grant.config.duration_ns.saturating_sub(total_safety_margin);

    if elapsed < safe_threshold {
        LeaseVerdict::Valid {
            remaining_safe_ns: safe_threshold - elapsed,
        }
    } else {
        LeaseVerdict::InGraceZone
    }
}

/// Mutante degenerado (AS-IS): ignora slack e drift, admitindo leases até o último nanossegundo.
#[must_use]
pub fn check_lease_validity_as_is(grant: &LeaseGrant, current_time_ns: u64) -> LeaseVerdict {
    let elapsed = current_time_ns.saturating_sub(grant.granted_at_ns);
    if elapsed < grant.config.duration_ns {
        LeaseVerdict::Valid {
            remaining_safe_ns: grant.config.duration_ns - elapsed,
        }
    } else {
        LeaseVerdict::Expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lease_config_bounds() {
        assert_eq!(
            LeaseConfig::try_new(0, 10, 5),
            Err(LeaseConfigError::ZeroDuration)
        );

        assert_eq!(
            LeaseConfig::try_new(100, 80, 15), // safety_margin = 80 + 30 = 110 >= 100
            Err(LeaseConfigError::SafetyMarginExceedsDuration {
                safety_margin_ns: 110,
                duration_ns: 100,
            })
        );

        let config = LeaseConfig::try_new(1000, 100, 50).expect("valid config");
        assert_eq!(
            LeaseGrant::try_new(0, config),
            Err(LeaseConfigError::ZeroGrantedAt)
        );

        let grant = LeaseGrant::try_new(100, config).expect("valid grant");
        assert_eq!(grant.granted_at_ns, 100);

        // Check lease validity
        assert!(matches!(
            check_lease_validity(&grant, 200),
            LeaseVerdict::Valid { .. }
        ));
        assert_eq!(
            check_lease_validity(&grant, 50),
            LeaseVerdict::TimeInversionDetected
        );
    }

    #[test]
    fn test_lease_error_display() {
        let err = LeaseConfigError::ZeroDuration;
        assert_eq!(format!("{err}"), "lease duration cannot be zero");

        let err2 = LeaseConfigError::ZeroGrantedAt;
        assert_eq!(format!("{err2}"), "granted_at timestamp cannot be zero");

        let err3 = LeaseConfigError::SafetyMarginExceedsDuration {
            safety_margin_ns: 200,
            duration_ns: 100,
        };
        assert_eq!(format!("{err3}"), "safety margin 200ns >= duration 100ns");
    }
}

