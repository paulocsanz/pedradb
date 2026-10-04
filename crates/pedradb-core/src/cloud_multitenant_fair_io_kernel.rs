//! kernel: cloud_multitenant_fair_io
//! Multi-tenant Deficit Round-Robin (DRR) I/O bandwidth scheduler and tail-latency defense kernel.
//!
//! Provides deterministic fair queuing across interactive client queries, WAL write barriers,
//! and background compaction I/O to guarantee strict p99.99 latency SLAs on shared cloud volumes.

/// I/O traffic classification for multi-tenant cloud storage operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IoTrafficClass {
    /// Class 1: Interactive client point gets and range scans (highest latency priority).
    InteractiveRead = 0,
    /// Class 2: Durable transaction WAL appends and manifest commits.
    TransactionWal = 1,
    /// Class 3: Background L0 flush and leveled compaction merge streams.
    BackgroundCompaction = 2,
}

/// Typed errors produced during multi-tenant I/O scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoScheduleError {
    /// Quantum bytes cannot be zero for any traffic class.
    ZeroQuantum,
    /// Requested I/O bytes exceed available deficit budget for this round.
    DeficitExceeded { requested: u32, available: i64 },
    /// Throttle percentage must be between 0 and 100.
    InvalidThrottlePercentage(u32),
}

impl core::fmt::Display for IoScheduleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroQuantum => write!(f, "I/O quantum must be greater than zero"),
            Self::DeficitExceeded { requested, available } => {
                write!(f, "I/O request {} bytes exceeds available deficit {}", requested, available)
            }
            Self::InvalidThrottlePercentage(p) => write!(f, "Throttle percentage {} must be <= 100", p),
        }
    }
}

/// Manages Deficit Round-Robin (DRR) bandwidth budgeting across I/O traffic classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudIoScheduler {
    /// Per-round quantum allocation in bytes for [InteractiveRead, TransactionWal, BackgroundCompaction].
    quantum_bytes: [u32; 3],
    /// Current accumulated deficit counter in bytes for each class.
    deficit_bytes: [i64; 3],
    /// Cumulative total bytes admitted across lifetime for each class.
    total_bytes_admitted: [u64; 3],
}

impl CloudIoScheduler {
    /// Constructs a new scheduler with explicit quanta for interactive reads, WAL, and compaction.
    pub fn new(
        interactive_quantum: u32,
        wal_quantum: u32,
        compaction_quantum: u32,
    ) -> Result<Self, IoScheduleError> {
        if interactive_quantum == 0 || wal_quantum == 0 || compaction_quantum == 0 {
            return Err(IoScheduleError::ZeroQuantum);
        }

        let quanta = [interactive_quantum, wal_quantum, compaction_quantum];
        let initial_deficit = [
            interactive_quantum as i64,
            wal_quantum as i64,
            compaction_quantum as i64,
        ];

        Ok(Self {
            quantum_bytes: quanta,
            deficit_bytes: initial_deficit,
            total_bytes_admitted: [0, 0, 0],
        })
    }

    /// Checks if a request of `bytes` length can be immediately admitted under current deficit.
    #[must_use]
    pub fn can_admit(&self, class: IoTrafficClass, bytes: u32) -> bool {
        let idx = class as usize;
        self.deficit_bytes[idx] >= (bytes as i64)
    }

    /// Admits an I/O request, deducting from the class's deficit counter.
    pub fn admit(&mut self, class: IoTrafficClass, bytes: u32) -> Result<(), IoScheduleError> {
        let idx = class as usize;
        let req = bytes as i64;
        if self.deficit_bytes[idx] < req {
            return Err(IoScheduleError::DeficitExceeded {
                requested: bytes,
                available: self.deficit_bytes[idx],
            });
        }

        self.deficit_bytes[idx] -= req;
        self.total_bytes_admitted[idx] = self.total_bytes_admitted[idx].saturating_add(bytes as u64);
        debug_assert!(self.verify_internal_invariants());
        Ok(())
    }

    /// Advances to the next round, replenishing quanta for all traffic classes.
    ///
    /// Caps deficit accumulation at 4x quantum to prevent burst runaway.
    pub fn replenish_round(&mut self) {
        for i in 0..3 {
            let max_accum = (self.quantum_bytes[i] as i64).saturating_mul(4);
            let next = self.deficit_bytes[i].saturating_add(self.quantum_bytes[i] as i64);
            self.deficit_bytes[i] = next.min(max_accum);
        }
        debug_assert!(self.verify_internal_invariants());
    }

    /// Throttles background compaction quantum by `reduction_pct` when cloud volume noisy neighbor
    /// throttle or EBS credit exhaustion is detected.
    pub fn throttle_compaction(&mut self, reduction_pct: u32) -> Result<(), IoScheduleError> {
        if reduction_pct > 100 {
            return Err(IoScheduleError::InvalidThrottlePercentage(reduction_pct));
        }

        let orig = self.quantum_bytes[IoTrafficClass::BackgroundCompaction as usize];
        let multiplier = 100u32.saturating_sub(reduction_pct);
        let new_quantum = ((orig as u64 * multiplier as u64) / 100).max(1) as u32;

        self.quantum_bytes[IoTrafficClass::BackgroundCompaction as usize] = new_quantum;
        debug_assert!(self.verify_internal_invariants());
        Ok(())
    }

    /// Cumulative bytes admitted for a specific class.
    #[must_use]
    pub fn total_bytes_admitted(&self, class: IoTrafficClass) -> u64 {
        self.total_bytes_admitted[class as usize]
    }

    /// Current deficit balance for a specific class.
    #[must_use]
    pub fn current_deficit(&self, class: IoTrafficClass) -> i64 {
        self.deficit_bytes[class as usize]
    }

    /// Verifies mathematical consistency of the scheduler state.
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        self.quantum_bytes.iter().all(|&q| q > 0)
    }
}
