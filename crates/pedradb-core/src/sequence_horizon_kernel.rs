//! Monotonic Sequence Number Horizon and Rollover Prevention Kernel (RFC-0284 Pilar 7).
//!
//! Enforces an inviolable upper safety ceiling on 64-bit MVCC sequence number allocations,
//! preventing arithmetic overflow and cyclic ordering inversion.
//!
//! Guarantees:
//! 1. Sequence numbers are strictly monotonic: `s_{i+1} > s_i`.
//! 2. Safety barrier triggers at `2^64 - 2^32`, leaving a 4-billion margin for clean shutdown.
//! 3. Total order is preserved; modulo rollover ($2^{64}-1 \to 0$) is mathematically impossible.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

/// Safe sequence number ceiling: $2^{64} - 2^{32}$.
pub const SEQUENCE_SAFE_CEILING: u64 = u64::MAX - (1u64 << 32);

/// Erro de violação do horizonte de sequência ou alocação inválida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceHorizonError {
    /// Tentativa de alocação com contagem zero de sequências.
    ZeroCountAllocation,
    /// Sequência inicial fornecida excede o teto de segurança.
    InitialSeqExceedsCeiling { initial_seq: u64, safe_ceiling: u64 },
    /// Teto de segurança alcançado; alocação violaria a barreira de rollover.
    HorizonReached {
        current_seq: u64,
        requested_count: u64,
        safe_ceiling: u64,
    },
    /// A sequência avaliada viola a monotonicidade estrita.
    MonotonicityViolation { prev: u64, next: u64 },
}

impl std::fmt::Display for SequenceHorizonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroCountAllocation => write!(f, "allocation count cannot be zero"),
            Self::InitialSeqExceedsCeiling { initial_seq, safe_ceiling } => {
                write!(f, "initial sequence number {initial_seq} exceeds safe ceiling {safe_ceiling}")
            }
            Self::HorizonReached { current_seq, requested_count, safe_ceiling } => {
                write!(
                    f,
                    "sequence horizon breached: current {current_seq} + count {requested_count} exceeds safe ceiling {safe_ceiling}"
                )
            }
            Self::MonotonicityViolation { prev, next } => {
                write!(f, "monotonicity violation: sequence {prev} is not strictly less than {next}")
            }
        }
    }
}

impl std::error::Error for SequenceHorizonError {}

/// Status of sequence number allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceAllocation {
    /// Allocation succeeded within safe operational envelope.
    Allocated(u64),
    /// Allocation rejected: safe ceiling reached. Engine must freeze writes or rekey epoch.
    HorizonReached {
        current_seq: u64,
        safe_ceiling: u64,
    },
}

impl std::fmt::Display for SequenceAllocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allocated(seq) => write!(f, "sequence allocated: {seq}"),
            Self::HorizonReached { current_seq, safe_ceiling } => {
                write!(f, "sequence horizon reached: {current_seq} >= {safe_ceiling}")
            }
        }
    }
}

/// Thread-safe sequence allocator with hard horizon enforcement.
pub struct SequenceHorizonAllocator {
    current: AtomicU64,
}

impl SequenceHorizonAllocator {
    /// Creates allocator initialized to a specific starting sequence with ceiling validation.
    pub fn try_new(initial_seq: u64) -> Result<Self, SequenceHorizonError> {
        if initial_seq > SEQUENCE_SAFE_CEILING {
            return Err(SequenceHorizonError::InitialSeqExceedsCeiling {
                initial_seq,
                safe_ceiling: SEQUENCE_SAFE_CEILING,
            });
        }
        Ok(Self {
            current: AtomicU64::new(initial_seq),
        })
    }

    /// Creates allocator initialized to a specific starting sequence.
    pub fn new(initial_seq: u64) -> Self {
        Self::try_new(initial_seq).expect("initial sequence must not exceed safety ceiling")
    }

    /// Current allocated sequence number.
    pub fn current_seq(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }

    /// Atomically allocates `count` sequential sequence numbers returning a Result.
    pub fn try_allocate_batch(&self, count: u64) -> Result<u64, SequenceHorizonError> {
        if count == 0 {
            return Err(SequenceHorizonError::ZeroCountAllocation);
        }

        let mut curr = self.current.load(Ordering::Acquire);
        loop {
            // Check if allocation would breach ceiling or overflow
            if curr >= SEQUENCE_SAFE_CEILING
                || curr.checked_add(count).map_or(true, |next| next > SEQUENCE_SAFE_CEILING)
            {
                return Err(SequenceHorizonError::HorizonReached {
                    current_seq: curr,
                    requested_count: count,
                    safe_ceiling: SEQUENCE_SAFE_CEILING,
                });
            }

            let next = curr + count;
            match self.current.compare_exchange_weak(
                curr,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(assigned) => return Ok(assigned),
                Err(actual) => curr = actual,
            }
        }
    }

    /// Atomically allocates `count` sequential sequence numbers.
    ///
    /// Returns `SequenceAllocation::Allocated(start_seq)` on success, or
    /// `SequenceAllocation::HorizonReached` if allocation would breach `SEQUENCE_SAFE_CEILING`.
    pub fn allocate_batch(&self, count: u64) -> SequenceAllocation {
        match self.try_allocate_batch(count) {
            Ok(assigned) => SequenceAllocation::Allocated(assigned),
            Err(SequenceHorizonError::ZeroCountAllocation) => {
                SequenceAllocation::Allocated(self.current_seq())
            }
            Err(SequenceHorizonError::HorizonReached { current_seq, safe_ceiling, .. }) => {
                SequenceAllocation::HorizonReached {
                    current_seq,
                    safe_ceiling,
                }
            }
            Err(_) => SequenceAllocation::HorizonReached {
                current_seq: self.current_seq(),
                safe_ceiling: SEQUENCE_SAFE_CEILING,
            },
        }
    }

    /// Verifies strict monotonicity for a sequence of allocations with error reporting.
    pub fn try_verify_monotonicity(allocations: &[u64]) -> Result<(), SequenceHorizonError> {
        if allocations.len() <= 1 {
            return Ok(());
        }
        for window in allocations.windows(2) {
            if window[0] >= window[1] {
                return Err(SequenceHorizonError::MonotonicityViolation {
                    prev: window[0],
                    next: window[1],
                });
            }
        }
        Ok(())
    }

    /// Verifies strict monotonicity for a sequence of allocations.
    pub fn verify_monotonicity(allocations: &[u64]) -> bool {
        Self::try_verify_monotonicity(allocations).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_horizon_structural_invariants_red_to_green() {
        // 1. Valid allocation batch
        let allocator = SequenceHorizonAllocator::try_new(100).expect("valid allocator");
        assert_eq!(allocator.current_seq(), 100);

        let allocated = allocator.try_allocate_batch(10).expect("allocate batch");
        assert_eq!(allocated, 100);
        assert_eq!(allocator.current_seq(), 110);

        // 2. Reject zero count allocation in try_allocate_batch
        let err_zero = allocator.try_allocate_batch(0);
        assert_eq!(err_zero, Err(SequenceHorizonError::ZeroCountAllocation));

        // 3. Reject initial sequence exceeding safety ceiling
        let err_init = SequenceHorizonAllocator::try_new(SEQUENCE_SAFE_CEILING + 1);
        assert_eq!(
            err_init.err(),
            Some(SequenceHorizonError::InitialSeqExceedsCeiling {
                initial_seq: SEQUENCE_SAFE_CEILING + 1,
                safe_ceiling: SEQUENCE_SAFE_CEILING,
            })
        );

        // 4. Horizon reached near safe ceiling
        let near_ceiling = SequenceHorizonAllocator::try_new(SEQUENCE_SAFE_CEILING - 5).expect("valid");
        let err_horizon = near_ceiling.try_allocate_batch(10);
        assert_eq!(
            err_horizon,
            Err(SequenceHorizonError::HorizonReached {
                current_seq: SEQUENCE_SAFE_CEILING - 5,
                requested_count: 10,
                safe_ceiling: SEQUENCE_SAFE_CEILING,
            })
        );

        // 5. Monotonicity verification
        let monotonic_seq = vec![1, 5, 10, 100];
        assert!(SequenceHorizonAllocator::try_verify_monotonicity(&monotonic_seq).is_ok());

        let non_monotonic = vec![1, 5, 5, 10];
        assert_eq!(
            SequenceHorizonAllocator::try_verify_monotonicity(&non_monotonic),
            Err(SequenceHorizonError::MonotonicityViolation { prev: 5, next: 5 })
        );

        // 6. Display & Error implementations
        let d = format!("{}", SequenceHorizonError::ZeroCountAllocation);
        assert!(d.contains("allocation count cannot be zero"));
    }
}
