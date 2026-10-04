//! kernel: io_ring_completion_order
//! Monotonic asynchronous I/O completion queue ring order verifier and watermark tracker.
//!
//! Enforces strictly non-regressing commit watermarks and duplicate-rejection for
//! out-of-order completions from multi-queue NVMe devices and io_uring completion rings.

/// Typed errors produced during asynchronous completion tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoRingCompletionError {
    /// Window size must be greater than zero and within reasonable bounds.
    InvalidWindowSize(usize),
    /// Completed ticket is smaller than the current durable watermark (already retired).
    StaleTicket { ticket: u64, watermark: u64 },
    /// Ticket was already marked completed; duplicate CQE received.
    DuplicateCompletion(u64),
    /// Ticket exceeds the allowed sliding window capacity ahead of watermark.
    TicketExceedsWindow { ticket: u64, max_allowed: u64 },
    /// Watermark advancement overflowed u64 representation.
    WatermarkOverflow,
}

impl core::fmt::Display for IoRingCompletionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidWindowSize(w) => write!(f, "Invalid sliding window size: {}", w),
            Self::StaleTicket { ticket, watermark } => {
                write!(f, "Ticket {} is stale (watermark is at {})", ticket, watermark)
            }
            Self::DuplicateCompletion(t) => write!(f, "Duplicate completion for ticket {}", t),
            Self::TicketExceedsWindow { ticket, max_allowed } => {
                write!(f, "Ticket {} exceeds max window bound {}", ticket, max_allowed)
            }
            Self::WatermarkOverflow => write!(f, "Watermark advancement caused integer overflow"),
        }
    }
}

impl std::error::Error for IoRingCompletionError {}

/// Tracks asynchronous completion tickets and maintains a monotonically advancing contiguous watermark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoRingCompletionTracker {
    /// Highest contiguous completed ticket. All tickets < watermark are durably complete.
    watermark: u64,
    /// Maximum allowed distance between watermark and an in-flight ticket.
    max_window_size: usize,
    /// Sliding window tracking completion status of tickets in [watermark, watermark + max_window_size).
    slots: Vec<bool>,
}

impl IoRingCompletionTracker {
    /// Constructs a new completion tracker starting at `initial_watermark` with bounded window capacity.
    pub fn new(initial_watermark: u64, max_window_size: usize) -> Result<Self, IoRingCompletionError> {
        if max_window_size == 0 || max_window_size > 1_000_000 {
            return Err(IoRingCompletionError::InvalidWindowSize(max_window_size));
        }

        Ok(Self {
            watermark: initial_watermark,
            max_window_size,
            slots: vec![false; max_window_size],
        })
    }

    /// Records completion of `ticket`, advancing the contiguous durable watermark if possible.
    ///
    /// Returns the new (possibly advanced) durable watermark.
    pub fn complete_ticket(&mut self, ticket: u64) -> Result<u64, IoRingCompletionError> {
        if ticket < self.watermark {
            return Err(IoRingCompletionError::StaleTicket {
                ticket,
                watermark: self.watermark,
            });
        }

        let diff = ticket - self.watermark;
        if diff >= self.max_window_size as u64 {
            let max_allowed = self.watermark.saturating_add(self.max_window_size as u64).saturating_sub(1);
            return Err(IoRingCompletionError::TicketExceedsWindow { ticket, max_allowed });
        }

        let offset = diff as usize;
        if self.slots[offset] {
            return Err(IoRingCompletionError::DuplicateCompletion(ticket));
        }

        self.slots[offset] = true;

        // Drain contiguous completed tickets from the front of the sliding window
        let mut advance_count = 0usize;
        for &is_done in &self.slots {
            if is_done {
                advance_count += 1;
            } else {
                break;
            }
        }

        if advance_count > 0 {
            self.watermark = self
                .watermark
                .checked_add(advance_count as u64)
                .ok_or(IoRingCompletionError::WatermarkOverflow)?;
            self.slots.drain(0..advance_count);
            self.slots.resize(self.max_window_size, false);
        }

        debug_assert!(self.verify_internal_invariants());
        Ok(self.watermark)
    }

    /// Records completion of a batch of tickets atomically. If any ticket fails,
    /// tracker state is rolled back cleanly.
    pub fn complete_batch(&mut self, tickets: &[u64]) -> Result<u64, IoRingCompletionError> {
        let mut clone = self.clone();
        for &t in tickets {
            clone.complete_ticket(t)?;
        }
        *self = clone;
        Ok(self.watermark)
    }

    /// Current durable contiguous watermark.
    #[must_use]
    pub fn watermark(&self) -> u64 {
        self.watermark
    }

    /// Maximum window capacity.
    #[must_use]
    pub fn max_window_size(&self) -> usize {
        self.max_window_size
    }

    /// Returns `true` if `ticket` has been durably retired (< watermark) or marked complete in window.
    #[must_use]
    pub fn is_ticket_completed(&self, ticket: u64) -> bool {
        if ticket < self.watermark {
            return true;
        }
        let diff = ticket - self.watermark;
        if diff < self.max_window_size as u64 {
            self.slots[diff as usize]
        } else {
            false
        }
    }

    /// Returns the number of out-of-order tickets completed in the window that have not yet formed
    /// a contiguous run from the watermark.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.slots.iter().filter(|&&done| done).count()
    }

    /// Mathematical invariant validation:
    /// 1. Window size matches max_window_size.
    /// 2. slots[0] is always false after drainage (otherwise watermark should have advanced).
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        if self.slots.len() != self.max_window_size {
            return false;
        }
        if !self.slots.is_empty() && self.slots[0] {
            return false;
        }
        true
    }
}
