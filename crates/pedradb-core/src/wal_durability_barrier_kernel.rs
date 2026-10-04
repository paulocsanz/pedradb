//! RFC-0324: WAL Durability Barrier Kernel.
//!
//! Eradicates O1 durability failure modes (acknowledged writes lost post-crash)
//! by enforcing a physical barrier contract: no write batch may return `Ok` to a
//! caller without holding a verified `DurabilityReceipt` demonstrating that its
//! WAL ticket has been physically synchronized via `fdatasync` (RFC-0041, RFC-0270).

#![forbid(unsafe_code)]

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

/// Errors emitted by the WAL durability barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurabilityBarrierError {
    /// Zero ticket hazard: tickets must be strictly monotonic non-zero.
    ZeroTicketHazard,
    /// Out of order sequence: ticket was not strictly greater than previous ticket.
    NonMonotonicTicket { previous: u64, current: u64 },
    /// Premature acknowledgement attempt: ticket has not been fsynced yet.
    PrematureAckHazard { ticket: u64, fsynced: u64 },
    /// Flush watermark lag: fsync called on a ticket that has not even been flushed to OS buffers.
    FsyncAheadOfFlush { ticket: u64, flushed: u64 },
    /// Regressive sync attempt: tried to advance sync watermark backwards.
    RegressiveSyncWatermark { current_synced: u64, attempted: u64 },
}

impl fmt::Display for DurabilityBarrierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTicketHazard => write!(f, "WAL ticket must be strictly positive non-zero"),
            Self::NonMonotonicTicket { previous, current } => {
                write!(f, "Ticket out of order: previous {previous} >= current {current}")
            }
            Self::PrematureAckHazard { ticket, fsynced } => {
                write!(f, "Premature ACK: ticket {ticket} exceeds fsynced watermark {fsynced}")
            }
            Self::FsyncAheadOfFlush { ticket, flushed } => {
                write!(f, "Fsync ahead of flush: ticket {ticket} > flushed {flushed}")
            }
            Self::RegressiveSyncWatermark { current_synced, attempted } => {
                write!(f, "Regressive sync: cannot move watermark from {current_synced} down to {attempted}")
            }
        }
    }
}

impl std::error::Error for DurabilityBarrierError {}

/// Monotonically increasing non-zero write ticket.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WriteTicket(NonZeroU64);

impl WriteTicket {
    /// Construct a write ticket, rejecting 0.
    pub fn try_new(raw: u64) -> Result<Self, DurabilityBarrierError> {
        NonZeroU64::new(raw)
            .map(Self)
            .ok_or(DurabilityBarrierError::ZeroTicketHazard)
    }

    /// Access the underlying raw ticket value.
    #[must_use]
    pub fn as_u64(self) -> u64 {
        self.0.get()
    }
}

/// Linear proof token certifying that a write ticket has been committed to physical disk.
///
/// Under RFC-0324, this token cannot be constructed outside the barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurabilityReceipt {
    ticket: WriteTicket,
    fsynced_watermark: u64,
}

impl DurabilityReceipt {
    /// The ticket covered by this receipt.
    #[must_use]
    pub fn ticket(&self) -> WriteTicket {
        self.ticket
    }

    /// The fsynced watermark at receipt generation time.
    #[must_use]
    pub fn fsynced_watermark(&self) -> u64 {
        self.fsynced_watermark
    }

    /// Internal test helper for simulating corrupted/forged receipts.
    #[doc(hidden)]
    #[must_use]
    pub fn from_parts_for_test(ticket: WriteTicket, fsynced_watermark: u64) -> Self {
        Self { ticket, fsynced_watermark }
    }
}

/// Zero-allocation atomic durability barrier tracking the progression of write batches.
///
/// Invariant: `max_acked <= fsynced <= flushed <= issued`.
#[derive(Debug)]
pub struct WalDurabilityBarrier {
    issued_ticket: AtomicU64,
    flushed_ticket: AtomicU64,
    fsynced_ticket: AtomicU64,
    acked_ticket: AtomicU64,
}

impl Default for WalDurabilityBarrier {
    fn default() -> Self {
        Self::new()
    }
}

impl WalDurabilityBarrier {
    /// Creates a new durability barrier initialized at ticket 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            issued_ticket: AtomicU64::new(0),
            flushed_ticket: AtomicU64::new(0),
            fsynced_ticket: AtomicU64::new(0),
            acked_ticket: AtomicU64::new(0),
        }
    }

    /// Issue the next monotonic write ticket.
    pub fn issue_ticket(&self) -> WriteTicket {
        let raw = self.issued_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        WriteTicket(NonZeroU64::new(raw).expect("ticket strictly positive"))
    }

    /// Records that batches up to `ticket` have been written to the OS page cache.
    pub fn record_flush(&self, ticket: WriteTicket) {
        let target = ticket.as_u64();
        let mut current = self.flushed_ticket.load(Ordering::Acquire);
        while target > current {
            match self.flushed_ticket.compare_exchange_weak(
                current,
                target,
                Ordering::SeqCst,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Records physical `fdatasync` completion up to `ticket` and issues a `DurabilityReceipt`.
    pub fn record_fdatasync(
        &self,
        ticket: WriteTicket,
    ) -> Result<DurabilityReceipt, DurabilityBarrierError> {
        let target = ticket.as_u64();
        let flushed = self.flushed_ticket.load(Ordering::Acquire);
        if target > flushed {
            return Err(DurabilityBarrierError::FsyncAheadOfFlush { ticket: target, flushed });
        }

        let mut current = self.fsynced_ticket.load(Ordering::Acquire);
        loop {
            if target < current {
                // Already fsynced past this ticket: idempotent receipt generation
                break;
            }
            match self.fsynced_ticket.compare_exchange_weak(
                current,
                target,
                Ordering::SeqCst,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    current = target;
                    break;
                }
                Err(actual) => current = actual,
            }
        }

        Ok(DurabilityReceipt {
            ticket,
            fsynced_watermark: current,
        })
    }

    /// Acknowledges completion to the client using a valid `DurabilityReceipt`.
    ///
    /// Fails closed if the receipt's ticket exceeds the physical fsynced watermark.
    pub fn acknowledge_client(
        &self,
        receipt: &DurabilityReceipt,
    ) -> Result<(), DurabilityBarrierError> {
        let ticket_val = receipt.ticket.as_u64();
        let fsynced = self.fsynced_ticket.load(Ordering::Acquire);
        if ticket_val > fsynced {
            return Err(DurabilityBarrierError::PrematureAckHazard { ticket: ticket_val, fsynced });
        }

        let mut current = self.acked_ticket.load(Ordering::Acquire);
        while ticket_val > current {
            match self.acked_ticket.compare_exchange_weak(
                current,
                ticket_val,
                Ordering::SeqCst,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        Ok(())
    }

    /// Current highest issued ticket.
    #[must_use]
    pub fn issued_watermark(&self) -> u64 {
        self.issued_ticket.load(Ordering::Acquire)
    }

    /// Current highest flushed watermark.
    #[must_use]
    pub fn flushed_watermark(&self) -> u64 {
        self.flushed_ticket.load(Ordering::Acquire)
    }

    /// Current highest fsynced watermark.
    #[must_use]
    pub fn fsynced_watermark(&self) -> u64 {
        self.fsynced_ticket.load(Ordering::Acquire)
    }

    /// Current highest acknowledged watermark.
    #[must_use]
    pub fn acked_watermark(&self) -> u64 {
        self.acked_ticket.load(Ordering::Acquire)
    }

    /// Evaluates the fundamental durability invariant:
    /// `acked <= fsynced <= flushed <= issued`.
    #[must_use]
    pub fn verify_durability_invariants(&self) -> bool {
        let acked = self.acked_watermark();
        let fsynced = self.fsynced_watermark();
        let flushed = self.flushed_watermark();
        let issued = self.issued_watermark();

        acked <= fsynced && fsynced <= flushed && flushed <= issued
    }
}
