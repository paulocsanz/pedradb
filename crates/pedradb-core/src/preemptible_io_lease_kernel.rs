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
}

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

/// Decisão tomada na borda da chamada de sistema de I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreSyscallGuardDecision {
    /// Lease válido: prossegue com a submissão física ao kernel POSIX/io_uring.
    ProceedWithIo,
    /// Lease expirou por preempção: cancela o I/O e renegocia a cota com o rate limiter.
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
        let elapsed = current_tick.saturating_sub(token.issued_at_tick);

        if elapsed <= token.max_duration_ticks {
            PreSyscallGuardDecision::ProceedWithIo
        } else {
            PreSyscallGuardDecision::AbortAndRenegotiate {
                overrun_ticks: elapsed - token.max_duration_ticks,
            }
        }
    }

    /// Valida que nenhuma thread executa I/O sob lease expirado.
    pub fn verify_execution_safety(
        token: &IoLeaseToken,
        execution_tick: u64,
    ) -> Result<(), PreemptedIoLeaseViolation> {
        let elapsed = execution_tick.saturating_sub(token.issued_at_tick);

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
}
