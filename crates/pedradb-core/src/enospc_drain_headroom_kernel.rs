//! RFC-0287: ENOSPC Drain Headroom and Acyclic Deallocation Liveness Kernel.
//!
//! Enforces strict priority-based allocation on disk space exhaustion.
//! Guarantees that maintenance tasks (compaction, VLog GC) never deadlock on
//! temporary metadata allocation while client writes are backpressured.

use std::sync::atomic::{AtomicU64, Ordering};

/// Erros de validação da configuração do governador de espaço.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnospcConfigError {
    /// Reserva de emergência excede a capacidade total do disco.
    ReserveExceedsCapacity { reserve: u64, capacity: u64 },
    /// Limite de stall de clientes é inferior à reserva de emergência.
    StallThresholdBelowReserve { threshold: u64, reserve: u64 },
    /// Limite de stall de clientes excede a capacidade total do disco.
    StallThresholdExceedsCapacity { threshold: u64, capacity: u64 },
    /// Capacidade total não pode ser zero.
    ZeroCapacity,
    /// Espaço disponível inicial excede a capacidade total.
    AvailableExceedsCapacity { available: u64, capacity: u64 },
}

impl std::fmt::Display for EnospcConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReserveExceedsCapacity { reserve, capacity } => {
                write!(f, "reserve {reserve} exceeds capacity {capacity}")
            }
            Self::StallThresholdBelowReserve { threshold, reserve } => {
                write!(f, "stall threshold {threshold} is below reserve {reserve}")
            }
            Self::StallThresholdExceedsCapacity { threshold, capacity } => {
                write!(f, "stall threshold {threshold} exceeds capacity {capacity}")
            }
            Self::ZeroCapacity => {
                write!(f, "total capacity cannot be zero")
            }
            Self::AvailableExceedsCapacity { available, capacity } => {
                write!(f, "initial available bytes {available} exceeds capacity {capacity}")
            }
        }
    }
}

impl std::error::Error for EnospcConfigError {}

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

/// Guard RAII que protege o espaço alocado contra vazamento definitivo em caso de abort/falha/pânico.
pub struct SpaceLease<'a> {
    governor: &'a DrainHeadroomGovernor,
    ticket_id: u64,
    granted_bytes: u64,
    committed: bool,
}

impl<'a> SpaceLease<'a> {
    /// ID do ticket de reserva.
    #[must_use]
    pub fn ticket_id(&self) -> u64 {
        self.ticket_id
    }

    /// Bytes reservados pelo lease.
    #[must_use]
    pub fn granted_bytes(&self) -> u64 {
        self.granted_bytes
    }

    /// Comita a operação de recuperação/reclaim, liberando o espaço de volta com ganho líquido.
    pub fn commit_reclaim(mut self, freed_net_bytes: u64) -> u64 {
        self.committed = true;
        self.governor.release_and_reclaim(self.granted_bytes, freed_net_bytes)
    }
}

impl<'a> Drop for SpaceLease<'a> {
    fn drop(&mut self) {
        if !self.committed && self.granted_bytes > 0 {
            // Rollback automático: devolve os bytes reservados à piscina disponível
            self.governor.release_and_reclaim(self.granted_bytes, 0);
        }
    }
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
    /// Cria um novo governador de espaço validando os invariantes de forma segura (sem pânico).
    pub fn try_new(
        total_capacity_bytes: u64,
        initial_available_bytes: u64,
        reserve_headroom_bytes: u64,
        client_stall_threshold_bytes: u64,
    ) -> Result<Self, EnospcConfigError> {
        if total_capacity_bytes == 0 {
            return Err(EnospcConfigError::ZeroCapacity);
        }
        if initial_available_bytes > total_capacity_bytes {
            return Err(EnospcConfigError::AvailableExceedsCapacity {
                available: initial_available_bytes,
                capacity: total_capacity_bytes,
            });
        }
        if reserve_headroom_bytes > total_capacity_bytes {
            return Err(EnospcConfigError::ReserveExceedsCapacity {
                reserve: reserve_headroom_bytes,
                capacity: total_capacity_bytes,
            });
        }
        if client_stall_threshold_bytes < reserve_headroom_bytes {
            return Err(EnospcConfigError::StallThresholdBelowReserve {
                threshold: client_stall_threshold_bytes,
                reserve: reserve_headroom_bytes,
            });
        }
        if client_stall_threshold_bytes > total_capacity_bytes {
            return Err(EnospcConfigError::StallThresholdExceedsCapacity {
                threshold: client_stall_threshold_bytes,
                capacity: total_capacity_bytes,
            });
        }

        Ok(Self {
            total_capacity_bytes,
            available_bytes: AtomicU64::new(initial_available_bytes.min(total_capacity_bytes)),
            reserve_headroom_bytes,
            client_stall_threshold_bytes,
            next_ticket_id: AtomicU64::new(1),
        })
    }

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
        Self::try_new(
            total_capacity_bytes,
            initial_available_bytes,
            reserve_headroom_bytes,
            client_stall_threshold_bytes,
        )
        .expect("invalid drain headroom governor configuration")
    }

    /// Aloca um lease RAII garantindo que, em caso de erro ou panic, o espaço reservado seja devolvido.
    pub fn allocate_lease(
        &self,
        priority: AllocationPriority,
        requested_bytes: u64,
    ) -> Result<SpaceLease<'_>, AllocationOutcome> {
        match self.request_allocation(priority, requested_bytes) {
            AllocationOutcome::Granted {
                granted_bytes,
                ticket_id,
            } => Ok(SpaceLease {
                governor: self,
                ticket_id,
                granted_bytes,
                committed: false,
            }),
            other => Err(other),
        }
    }

    /// Attempts to allocate space for a given priority class.
    #[must_use]
    pub fn request_allocation(&self, priority: AllocationPriority, requested_bytes: u64) -> AllocationOutcome {
        if requested_bytes == 0 {
            return AllocationOutcome::Granted {
                granted_bytes: 0,
                ticket_id: 0,
            };
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enospc_governor_bounds() {
        assert_eq!(
            DrainHeadroomGovernor::try_new(0, 0, 0, 0).err(),
            Some(EnospcConfigError::ZeroCapacity)
        );

        assert_eq!(
            DrainHeadroomGovernor::try_new(100, 150, 10, 20).err(),
            Some(EnospcConfigError::AvailableExceedsCapacity {
                available: 150,
                capacity: 100,
            })
        );

        let gov = DrainHeadroomGovernor::try_new(100, 80, 10, 20).expect("valid governor");
        assert_eq!(gov.available_bytes(), 80);
        assert_eq!(gov.reserve_headroom_bytes(), 10);
    }

    #[test]
    fn test_enospc_config_error_display() {
        let err = EnospcConfigError::ZeroCapacity;
        assert_eq!(format!("{err}"), "total capacity cannot be zero");

        let err2 = EnospcConfigError::AvailableExceedsCapacity {
            available: 200,
            capacity: 100,
        };
        assert_eq!(
            format!("{err2}"),
            "initial available bytes 200 exceeds capacity 100"
        );
    }
}

