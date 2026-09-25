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

/// Thread-safe sequence allocator with hard horizon enforcement.
pub struct SequenceHorizonAllocator {
    current: AtomicU64,
}

impl SequenceHorizonAllocator {
    /// Creates allocator initialized to a specific starting sequence.
    pub fn new(initial_seq: u64) -> Self {
        Self {
            current: AtomicU64::new(initial_seq),
        }
    }

    /// Current allocated sequence number.
    pub fn current_seq(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }

    /// Atomically allocates `count` sequential sequence numbers.
    ///
    /// Returns `SequenceAllocation::Allocated(start_seq)` on success, or
    /// `SequenceAllocation::HorizonReached` if allocation would breach `SEQUENCE_SAFE_CEILING`.
    pub fn allocate_batch(&self, count: u64) -> SequenceAllocation {
        if count == 0 {
            return SequenceAllocation::Allocated(self.current_seq());
        }

        let mut curr = self.current.load(Ordering::Acquire);
        loop {
            // Check if allocation would breach ceiling or overflow
            if curr >= SEQUENCE_SAFE_CEILING || curr.checked_add(count).map_or(true, |next| next > SEQUENCE_SAFE_CEILING) {
                return SequenceAllocation::HorizonReached {
                    current_seq: curr,
                    safe_ceiling: SEQUENCE_SAFE_CEILING,
                };
            }

            let next = curr + count;
            match self.current.compare_exchange_weak(
                curr,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(assigned) => return SequenceAllocation::Allocated(assigned),
                Err(actual) => curr = actual,
            }
        }
    }

    /// Verifies strict monotonicity for a sequence of allocations.
    pub fn verify_monotonicity(allocations: &[u64]) -> bool {
        if allocations.len() <= 1 {
            return true;
        }
        for window in allocations.windows(2) {
            if window[0] >= window[1] {
                return false;
            }
        }
        true
    }
}
