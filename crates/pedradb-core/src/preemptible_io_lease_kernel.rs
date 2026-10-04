//! RFC-0294: Pilar 10 - Leases Temporais Atômicos de I/O com Revogação Pré-Syscall.
//!
//! Evita rajadas de I/O descontroladas provocadas por preempção de kernel ou roubo de vCPU,
//! cancelando atômica e imediatamente créditos temporais expirados na borda da chamada de sistema.

/// Violações do contrato de lease temporal de I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreemptedIoLeaseViolation {
    /// Uma operação de I/O foi executada com um lease temporal já expirado (Rajada Fantasma).
    ExpiredIoLeaseExecuted {
        /// ID do token de lease.
        token_id: u64,
        /// Duração máxima do lease permitida.
        max_duration_ticks: u64,
        /// Duração real observada na borda da syscall.
        elapsed_ticks: u64,
        /// Bytes que seriam emitidos ilegalmente.
        bytes: u64,
    },
    /// A cota concedida excedeu o teto máximo de emissão por chamada.
    QuotaExceededPerIo {
        /// Bytes requisitados.
        requested_bytes: u64,
        /// Teto máximo permitido.
        max_allowed_bytes: u64,
    },
    /// Regressão de relógio ou clock skew detectado na avaliação do lease.
    ClockSkewDetected {
        token_id: u64,
        issued_at_tick: u64,
        current_tick: u64,
    },
    ZeroTokenId,
    ZeroBytesAllowed,
    ZeroDurationTicks,
    ZeroRequestedBytes,
}

impl std::fmt::Display for PreemptedIoLeaseViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExpiredIoLeaseExecuted { token_id, max_duration_ticks, elapsed_ticks, bytes } => {
                write!(f, "Expired IO lease executed: token {token_id}, max {max_duration_ticks}, elapsed {elapsed_ticks}, bytes {bytes}")
            }
            Self::QuotaExceededPerIo { requested_bytes, max_allowed_bytes } => {
                write!(f, "Quota exceeded: requested {requested_bytes} > allowed {max_allowed_bytes}")
            }
            Self::ClockSkewDetected { token_id, issued_at_tick, current_tick } => {
                write!(f, "Clock skew detected: token {token_id} issued at {issued_at_tick} > current {current_tick}")
            }
            Self::ZeroTokenId => write!(f, "Token ID cannot be 0"),
            Self::ZeroBytesAllowed => write!(f, "Bytes allowed cannot be 0"),
            Self::ZeroDurationTicks => write!(f, "Duration ticks cannot be 0"),
            Self::ZeroRequestedBytes => write!(f, "Requested bytes cannot be 0"),
        }
    }
}

impl std::error::Error for PreemptedIoLeaseViolation {}

/// Token de lease temporal de I/O concedido pelo rate limiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoLeaseToken {
    /// ID unívoco do token.
    pub token_id: u64,
    /// Quantidade de bytes de I/O autorizados.
    pub bytes_allowed: u64,
    /// Timestamp (tick) de emissão do lease.
    pub issued_at_tick: u64,
    /// Duração máxima de validade do lease em ticks (ex: 50 ticks = 50ms).
    pub max_duration_ticks: u64,
}

impl IoLeaseToken {
    pub fn try_new(
        token_id: u64,
        bytes_allowed: u64,
        issued_at_tick: u64,
        max_duration_ticks: u64,
    ) -> Result<Self, PreemptedIoLeaseViolation> {
        if token_id == 0 {
            return Err(PreemptedIoLeaseViolation::ZeroTokenId);
        }
        if bytes_allowed == 0 {
            return Err(PreemptedIoLeaseViolation::ZeroBytesAllowed);
        }
        if max_duration_ticks == 0 {
            return Err(PreemptedIoLeaseViolation::ZeroDurationTicks);
        }
        Ok(Self {
            token_id,
            bytes_allowed,
            issued_at_tick,
            max_duration_ticks,
        })
    }
}

/// Decisão tomada na borda da chamada de sistema de I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreSyscallGuardDecision {
    /// Lease válido: prossegue com a submissão física ao kernel POSIX/io_uring.
    ProceedWithIo,
    /// Lease expirou por preempção ou clock skew: cancela o I/O e renegocia a cota com o rate limiter.
    AbortAndRenegotiate {
        /// Ticks transcorridos além do limite.
        overrun_ticks: u64,
    },
}

/// Barreira de segurança pré-syscall para validação atômica de leases temporais.
pub struct PreSyscallIoGuard;

impl PreSyscallIoGuard {
    /// Avalia a validade temporal do lease imediatamente antes de invocar a syscall física.
    pub fn evaluate_guard(
        token: &IoLeaseToken,
        current_tick: u64,
    ) -> PreSyscallGuardDecision {
        if current_tick < token.issued_at_tick {
            return PreSyscallGuardDecision::AbortAndRenegotiate { overrun_ticks: 0 };
        }

        let elapsed = current_tick - token.issued_at_tick;

        if elapsed <= token.max_duration_ticks {
            PreSyscallGuardDecision::ProceedWithIo
        } else {
            PreSyscallGuardDecision::AbortAndRenegotiate {
                overrun_ticks: elapsed - token.max_duration_ticks,
            }
        }
    }

    /// Valida que nenhuma thread executa I/O sob lease expirado ou relógio corrompido.
    pub fn verify_execution_safety(
        token: &IoLeaseToken,
        execution_tick: u64,
    ) -> Result<(), PreemptedIoLeaseViolation> {
        if execution_tick < token.issued_at_tick {
            return Err(PreemptedIoLeaseViolation::ClockSkewDetected {
                token_id: token.token_id,
                issued_at_tick: token.issued_at_tick,
                current_tick: execution_tick,
            });
        }

        let elapsed = execution_tick - token.issued_at_tick;

        if elapsed > token.max_duration_ticks {
            return Err(PreemptedIoLeaseViolation::ExpiredIoLeaseExecuted {
                token_id: token.token_id,
                max_duration_ticks: token.max_duration_ticks,
                elapsed_ticks: elapsed,
                bytes: token.bytes_allowed,
            });
        }

        Ok(())
    }

    /// Valida que a quantidade requisitada de bytes respeita a cota do lease.
    pub fn verify_quota_safety(
        token: &IoLeaseToken,
        requested_bytes: u64,
    ) -> Result<(), PreemptedIoLeaseViolation> {
        if requested_bytes == 0 {
            return Err(PreemptedIoLeaseViolation::ZeroRequestedBytes);
        }
        if requested_bytes > token.bytes_allowed {
            return Err(PreemptedIoLeaseViolation::QuotaExceededPerIo {
                requested_bytes,
                max_allowed_bytes: token.bytes_allowed,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preemptible_io_lease_structural_invariants_red_to_green() {
        assert_eq!(
            IoLeaseToken::try_new(0, 1000, 10, 50),
            Err(PreemptedIoLeaseViolation::ZeroTokenId)
        );

        assert_eq!(
            IoLeaseToken::try_new(1, 0, 10, 50),
            Err(PreemptedIoLeaseViolation::ZeroBytesAllowed)
        );

        assert_eq!(
            IoLeaseToken::try_new(1, 1000, 10, 0),
            Err(PreemptedIoLeaseViolation::ZeroDurationTicks)
        );

        let token = IoLeaseToken::try_new(1, 1000, 10, 50).unwrap();
        assert_eq!(
            PreSyscallIoGuard::verify_quota_safety(&token, 0),
            Err(PreemptedIoLeaseViolation::ZeroRequestedBytes)
        );

        let disp = format!("{}", PreemptedIoLeaseViolation::ZeroTokenId);
        assert!(!disp.is_empty());
    }
}
