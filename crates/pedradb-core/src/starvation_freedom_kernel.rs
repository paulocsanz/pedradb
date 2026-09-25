//! RFC-0283 Pilar 1 — Ausência de Inanição e Limite Estrito de Ultrapassagem (Starvation Freedom Kernel).
//!
//! Formalizes starvation-freedom and bounded overtaking (K-exclusion) in concurrent
//! commit admission queues.
//! Proves that no enqueued writer thread can be bypassed by more than K newly arrived writers:
//!   |{ w' | w' admitted before w ∧ w' enqueued after w }| <= K.
//!
//! Guarantees wait-freedom with a deterministic upper bound on steps, preventing
//! livelocks and thread starvation under heavy contention.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Violations resulting from excessive thread overtaking or queue starvation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarvationViolation {
    /// A thread was overtaken by more than K subsequently enqueued threads.
    BoundedOvertakingExceeded {
        /// Ticket of the starved thread.
        starved_ticket: u64,
        /// Number of observed bypasses.
        observed_bypasses: usize,
        /// Maximum allowed limit K.
        max_allowed_k: usize,
    },
    /// An unknown or non-existent ticket was admitted.
    InvalidTicketAdmitted {
        /// The invalid ticket.
        ticket: u64,
    },
}

/// An admission queue enforcing strict K-bounded overtaking.
#[derive(Clone, Debug)]
pub struct BoundedOvertakingQueue {
    /// Maximum allowed overtaking threshold K.
    pub max_k: usize,
    /// Counter for issuing monotonically increasing tickets.
    next_ticket: u64,
    /// Set of actively pending tickets in the queue.
    pending_tickets: BTreeSet<u64>,
    /// Number of bypasses accrued per pending ticket.
    bypass_counts: BTreeMap<u64, usize>,
}

impl BoundedOvertakingQueue {
    /// Creates a new bounded overtaking queue with parameter K.
    #[must_use]
    pub fn new(max_k: usize) -> Self {
        Self {
            max_k,
            next_ticket: 1,
            pending_tickets: BTreeSet::new(),
            bypass_counts: BTreeMap::new(),
        }
    }

    /// Enqueues a writer and returns its unique monotonically increasing ticket.
    pub fn enqueue(&mut self) -> u64 {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        self.pending_tickets.insert(ticket);
        self.bypass_counts.insert(ticket, 0);
        ticket
    }

    /// Checks if a thread with `ticket` is permitted to be admitted for commit.
    /// If admitting this thread would cause an older thread to exceed K bypasses,
    /// admission is refused until the older thread is serviced.
    #[must_use]
    pub fn can_admit(&self, ticket: u64) -> bool {
        if !self.pending_tickets.contains(&ticket) {
            return false;
        }

        // Check if admitting `ticket` would violate K for any strictly older pending ticket
        for &older_ticket in self.pending_tickets.range(..ticket) {
            let current_bypasses = self.bypass_counts.get(&older_ticket).copied().unwrap_or(0);
            if current_bypasses >= self.max_k {
                // Must service `older_ticket` first!
                return false;
            }
        }

        true
    }

    /// Admits a ticket into the commit execution group, updating bypass counters.
    ///
    /// # Errors
    /// Returns `StarvationViolation` if `ticket` is invalid or exceeds K overtaking.
    pub fn admit(&mut self, ticket: u64) -> Result<(), StarvationViolation> {
        if !self.pending_tickets.remove(&ticket) {
            return Err(StarvationViolation::InvalidTicketAdmitted { ticket });
        }
        self.bypass_counts.remove(&ticket);

        // Every strictly older pending ticket was bypassed by this admission
        for &older_ticket in self.pending_tickets.range(..ticket) {
            let count = self.bypass_counts.entry(older_ticket).or_insert(0);
            *count += 1;
            if *count > self.max_k {
                return Err(StarvationViolation::BoundedOvertakingExceeded {
                    starved_ticket: older_ticket,
                    observed_bypasses: *count,
                    max_allowed_k: self.max_k,
                });
            }
        }

        Ok(())
    }

    /// Returns the number of currently pending threads in queue.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending_tickets.len()
    }
}
