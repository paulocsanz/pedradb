//! RFC-0287: ENOSPC Drain Headroom and Acyclic Deallocation Liveness Kernel.
//!
//! Enforces strict priority-based allocation on disk space exhaustion.
//! Guarantees that maintenance tasks (compaction, VLog GC) never deadlock on
//! temporary metadata allocation while client writes are backpressured.

use std::sync::atomic::{AtomicU64, Ordering};

/// Allocation priority distinguishing client traffic from space reclamation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationPriority {
    /// Standard client write or ingestion traffic.
    ClientIngest,
    /// High-priority space recovery task (Compaction finish, VLog truncation, Manifest checkpoint).
    EmergencyReclaimDrain,
}

/// Outcome of a space allocation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationOutcome {
    /// Space granted under valid lease.
    Granted {
        /// Bytes successfully reserved.
        granted_bytes: u64,
        /// Token generation ID.
        ticket_id: u64,
    },
    /// Client write stalled because available space is below headroom threshold.
    StalledClient {
        /// Currently available disk bytes.
        available_bytes: u64,
        /// Requested bytes.
        requested_bytes: u64,
        /// Required minimum buffer headroom above reserve.
        required_buffer: u64,
    },
    /// Emergency reserve exhausted (fatal disk condition).
    OutOfEmergencySpace {
        /// Currently available disk bytes.
        available_bytes: u64,
        /// Requested bytes.
        requested_bytes: u64,
    },
}

/// Governor managing space reservation and preventing space-deadlocks.
pub struct DrainHeadroomGovernor {
    /// Total filesystem capacity in bytes.
    total_capacity_bytes: u64,
    /// Currently observable free bytes on filesystem.
    available_bytes: AtomicU64,
    /// Untouchable emergency reserve dedicated strictly to drain/reclaim tasks.
    reserve_headroom_bytes: u64,
    /// Safety buffer threshold above reserve where client writes are stalled.
    client_stall_threshold_bytes: u64,
    /// Monotonic ticket counter.
    next_ticket_id: AtomicU64,
}

impl DrainHeadroomGovernor {
    /// Creates a new space governor.
    ///
    /// # Panics
    /// Panics if reserve headroom is larger than total capacity or stall threshold is smaller than reserve.
    #[must_use]
    pub fn new(
        total_capacity_bytes: u64,
        initial_available_bytes: u64,
        reserve_headroom_bytes: u64,
        client_stall_threshold_bytes: u64,
    ) -> Self {
        assert!(
            reserve_headroom_bytes <= total_capacity_bytes,
            "reserve headroom cannot exceed total capacity"
        );
        assert!(
            client_stall_threshold_bytes >= reserve_headroom_bytes,
            "client stall threshold must be at least reserve headroom"
        );

        Self {
            total_capacity_bytes,
            available_bytes: AtomicU64::new(initial_available_bytes.min(total_capacity_bytes)),
            reserve_headroom_bytes,
            client_stall_threshold_bytes,
            next_ticket_id: AtomicU64::new(1),
        }
    }

    /// Attempts to allocate space for a given priority class.
    #[must_use]
    pub fn request_allocation(&self, priority: AllocationPriority, requested_bytes: u64) -> AllocationOutcome {
        loop {
            let current = self.available_bytes.load(Ordering::Acquire);

            match priority {
                AllocationPriority::ClientIngest => {
                    // Client write requires current - requested >= client_stall_threshold_bytes
                    if current < self.client_stall_threshold_bytes || current < requested_bytes {
                        return AllocationOutcome::StalledClient {
                            available_bytes: current,
                            requested_bytes,
                            required_buffer: self.client_stall_threshold_bytes,
                        };
                    }
                    let remaining = current - requested_bytes;
                    if remaining < self.client_stall_threshold_bytes {
                        return AllocationOutcome::StalledClient {
                            available_bytes: current,
                            requested_bytes,
                            required_buffer: self.client_stall_threshold_bytes,
                        };
                    }

                    if self
                        .available_bytes
                        .compare_exchange_weak(current, remaining, Ordering::Release, Ordering::Relaxed)
                        .is_ok()
                    {
                        let ticket_id = self.next_ticket_id.fetch_add(1, Ordering::Relaxed);
                        return AllocationOutcome::Granted {
                            granted_bytes: requested_bytes,
                            ticket_id,
                        };
                    }
                }
                AllocationPriority::EmergencyReclaimDrain => {
                    // Drain task is permitted to dip directly into the reserve headroom
                    if current < requested_bytes {
                        return AllocationOutcome::OutOfEmergencySpace {
                            available_bytes: current,
                            requested_bytes,
                        };
                    }
                    let remaining = current - requested_bytes;

                    if self
                        .available_bytes
                        .compare_exchange_weak(current, remaining, Ordering::Release, Ordering::Relaxed)
                        .is_ok()
                    {
                        let ticket_id = self.next_ticket_id.fetch_add(1, Ordering::Relaxed);
                        return AllocationOutcome::Granted {
                            granted_bytes: requested_bytes,
                            ticket_id,
                        };
                    }
                }
            }
        }
    }

    /// Releases a ticket and returns newly freed net bytes back to available space.
    pub fn release_and_reclaim(&self, granted_bytes: u64, freed_net_bytes: u64) -> u64 {
        let total_returned = granted_bytes.saturating_add(freed_net_bytes);
        loop {
            let current = self.available_bytes.load(Ordering::Acquire);
            let updated = (current.saturating_add(total_returned)).min(self.total_capacity_bytes);
            if self
                .available_bytes
                .compare_exchange_weak(current, updated, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return updated;
            }
        }
    }

    /// Returns currently available disk bytes.
    #[must_use]
    pub fn available_bytes(&self) -> u64 {
        self.available_bytes.load(Ordering::Acquire)
    }

    /// Checks whether clients can currently execute writes without being stalled.
    #[must_use]
    pub fn can_client_write(&self) -> bool {
        self.available_bytes.load(Ordering::Acquire) > self.client_stall_threshold_bytes
    }

    /// Returns the reserve headroom size in bytes.
    #[must_use]
    pub fn reserve_headroom_bytes(&self) -> u64 {
        self.reserve_headroom_bytes
    }
}
