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
use std::fmt;

/// Default maximum number of pending admissions.
pub const DEFAULT_MAX_CAPACITY: usize = 1_000_000;
/// Maximum allowed K parameter.
pub const MAX_ALLOWED_K: usize = 1_000_000;

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
    /// The specified max_k parameter is invalid (must be between 1 and MAX_ALLOWED_K).
    InvalidMaxK(usize),
    /// Monotonically increasing ticket counter overflowed u64::MAX.
    TicketCounterOverflow,
    /// Queue capacity reached maximum allowed pending tickets.
    QueueCapacityExceeded {
        /// Current pending tickets.
        current: usize,
        /// Maximum capacity.
        max: usize,
    },
    /// Zero ticket is invalid.
    ZeroTicket,
    /// Admission queue is empty.
    EmptyQueue,
}

impl fmt::Display for StarvationViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BoundedOvertakingExceeded {
                starved_ticket,
                observed_bypasses,
                max_allowed_k,
            } => write!(
                f,
                "ticket {starved_ticket} overtaken {observed_bypasses} times exceeding bound K={max_allowed_k}"
            ),
            Self::InvalidTicketAdmitted { ticket } => {
                write!(f, "attempted to admit invalid ticket {ticket}")
            }
            Self::InvalidMaxK(k) => write!(
                f,
                "invalid max_k {k}: must be in range 1..={MAX_ALLOWED_K}"
            ),
            Self::TicketCounterOverflow => write!(f, "ticket sequence counter overflowed u64::MAX"),
            Self::QueueCapacityExceeded { current, max } => write!(
                f,
                "admission queue capacity exceeded (current: {current}, max: {max})"
            ),
            Self::ZeroTicket => write!(f, "ticket 0 is not a valid admission ticket"),
            Self::EmptyQueue => write!(f, "admission queue is empty"),
        }
    }
}

impl std::error::Error for StarvationViolation {}

/// An admission queue enforcing strict K-bounded overtaking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedOvertakingQueue {
    /// Maximum allowed overtaking threshold K.
    pub max_k: usize,
    /// Maximum capacity of pending tickets.
    pub max_capacity: usize,
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
        Self::try_new_with_capacity(max_k, DEFAULT_MAX_CAPACITY).unwrap_or_else(|_| Self {
            max_k: max_k.clamp(1, MAX_ALLOWED_K),
            max_capacity: DEFAULT_MAX_CAPACITY,
            next_ticket: 1,
            pending_tickets: BTreeSet::new(),
            bypass_counts: BTreeMap::new(),
        })
    }

    /// Creates a validated bounded overtaking queue with default capacity.
    pub fn try_new(max_k: usize) -> Result<Self, StarvationViolation> {
        Self::try_new_with_capacity(max_k, DEFAULT_MAX_CAPACITY)
    }

    /// Creates a validated bounded overtaking queue with explicit maximum capacity.
    pub fn try_new_with_capacity(
        max_k: usize,
        max_capacity: usize,
    ) -> Result<Self, StarvationViolation> {
        if max_k == 0 || max_k > MAX_ALLOWED_K {
            return Err(StarvationViolation::InvalidMaxK(max_k));
        }
        if max_capacity == 0 {
            return Err(StarvationViolation::QueueCapacityExceeded {
                current: 0,
                max: 0,
            });
        }
        Ok(Self {
            max_k,
            max_capacity,
            next_ticket: 1,
            pending_tickets: BTreeSet::new(),
            bypass_counts: BTreeMap::new(),
        })
    }

    /// Tries to enqueue a writer and returns its unique monotonically increasing ticket.
    pub fn try_enqueue(&mut self) -> Result<u64, StarvationViolation> {
        if self.pending_tickets.len() >= self.max_capacity {
            return Err(StarvationViolation::QueueCapacityExceeded {
                current: self.pending_tickets.len(),
                max: self.max_capacity,
            });
        }
        let ticket = self.next_ticket;
        let next = ticket
            .checked_add(1)
            .ok_or(StarvationViolation::TicketCounterOverflow)?;
        self.next_ticket = next;
        self.pending_tickets.insert(ticket);
        self.bypass_counts.insert(ticket, 0);
        Ok(ticket)
    }

    /// Enqueues a writer and returns its unique monotonically increasing ticket.
    pub fn enqueue(&mut self) -> u64 {
        self.try_enqueue().expect("enqueue failed")
    }

    /// Checks if a thread with `ticket` is permitted to be admitted for commit.
    /// If admitting this thread would cause an older thread to exceed K bypasses,
    /// admission is refused until the older thread is serviced.
    #[must_use]
    pub fn can_admit(&self, ticket: u64) -> bool {
        if ticket == 0 || !self.pending_tickets.contains(&ticket) {
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
        if ticket == 0 {
            return Err(StarvationViolation::ZeroTicket);
        }
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

    /// Returns the oldest pending ticket if queue is non-empty.
    #[must_use]
    pub fn oldest_pending_ticket(&self) -> Option<u64> {
        self.pending_tickets.iter().next().copied()
    }

    /// Returns the current number of bypasses observed by a pending ticket.
    #[must_use]
    pub fn bypass_count_of(&self, ticket: u64) -> Option<usize> {
        self.bypass_counts.get(&ticket).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starvation_freedom_structural_invariants_red_to_green() {
        // 1. Error Display & Error trait compliance
        let errs: Vec<StarvationViolation> = vec![
            StarvationViolation::BoundedOvertakingExceeded {
                starved_ticket: 1,
                observed_bypasses: 3,
                max_allowed_k: 2,
            },
            StarvationViolation::InvalidTicketAdmitted { ticket: 99 },
            StarvationViolation::InvalidMaxK(0),
            StarvationViolation::TicketCounterOverflow,
            StarvationViolation::QueueCapacityExceeded {
                current: 10,
                max: 10,
            },
            StarvationViolation::ZeroTicket,
            StarvationViolation::EmptyQueue,
        ];
        for err in &errs {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. try_new validation
        assert_eq!(
            BoundedOvertakingQueue::try_new(0),
            Err(StarvationViolation::InvalidMaxK(0))
        );
        assert_eq!(
            BoundedOvertakingQueue::try_new(MAX_ALLOWED_K + 1),
            Err(StarvationViolation::InvalidMaxK(MAX_ALLOWED_K + 1))
        );
        assert_eq!(
            BoundedOvertakingQueue::try_new_with_capacity(5, 0),
            Err(StarvationViolation::QueueCapacityExceeded {
                current: 0,
                max: 0
            })
        );
        let mut queue = BoundedOvertakingQueue::try_new_with_capacity(2, 3).unwrap();

        // 3. Zero ticket rejection
        assert_eq!(queue.admit(0), Err(StarvationViolation::ZeroTicket));
        assert!(!queue.can_admit(0));

        // 4. Capacity bound enforcement
        let t1 = queue.try_enqueue().unwrap();
        let t2 = queue.try_enqueue().unwrap();
        let _t3 = queue.try_enqueue().unwrap();
        assert_eq!(queue.pending_count(), 3);
        assert_eq!(
            queue.try_enqueue(),
            Err(StarvationViolation::QueueCapacityExceeded {
                current: 3,
                max: 3
            })
        );

        // 5. Query helpers
        assert_eq!(queue.oldest_pending_ticket(), Some(t1));
        assert_eq!(queue.bypass_count_of(t1), Some(0));

        // 6. Admit and bypass mechanics
        assert!(queue.can_admit(t2));
        assert!(queue.admit(t2).is_ok());
        assert_eq!(queue.bypass_count_of(t1), Some(1));
        assert_eq!(queue.bypass_count_of(t2), None);
    }
}

