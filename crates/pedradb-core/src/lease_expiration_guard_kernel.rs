//! Pilar 1: Oráculo de Validade de Leases sob Suspensão de Runtime (RFC-0285).
//!
//! Previne leituras stale e escritas por líderes defasados quando uma máquina
//! virtual sofre suspensão (VM freeze) ou quando o runtime assíncrono (Tokio)
//! sofre de inanição profunda de workers.

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
