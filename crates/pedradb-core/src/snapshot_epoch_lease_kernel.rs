//! RFC-0283 Pilar 7 — Leases de Época para Snapshots (Snapshot Epoch Lease Kernel).
//!
//! Formalizes bounded-lifetime snapshot leases to prevent catastrophic disk space exhaustion (ENOSPC).
//! Abandoned or long-lived snapshots freeze tombstone purging and VLog garbage collection indefinitely.
//! Under the Epoch Lease contract:
//!   - Every snapshot is granted a bounded lifetime in epochs: lease_epochs;
//!   - If current_epoch > created_epoch + lease_epochs, the lease is revoked;
//!   - GC and Compaction oracles advance min_active_snapshot beyond revoked sequences;
//!   - Further reads on revoked snapshots fail closed with `SnapshotLeaseExpired`.
//!
//! Mathematically guarantees bounded disk retention even under client disconnects or leaks.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Violations resulting from expired snapshot access or invalid lease allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotLeaseViolation {
    /// Read attempted through an expired snapshot lease.
    SnapshotLeaseExpired {
        /// The expired snapshot ID.
        snapshot_id: u64,
        /// Current engine epoch.
        current_epoch: u64,
        /// Epoch when lease expired.
        expired_at_epoch: u64,
    },
    /// Snapshot lease not found in the active manager.
    UnknownSnapshotId {
        /// ID requested.
        snapshot_id: u64,
    },
    /// Snapshot ID already exists and cannot be re-allocated.
    DuplicateSnapshotId(u64),
    /// Snapshot ID cannot be zero.
    ZeroSnapshotId,
    /// Lease duration in epochs cannot be zero.
    ZeroLeaseEpochs,
    /// Engine epoch counter overflowed u64::MAX.
    EpochCounterOverflow,
}

impl std::fmt::Display for SnapshotLeaseViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SnapshotLeaseExpired {
                snapshot_id,
                current_epoch,
                expired_at_epoch,
            } => write!(
                f,
                "Snapshot {snapshot_id} expired at epoch {expired_at_epoch} (current {current_epoch})"
            ),
            Self::UnknownSnapshotId { snapshot_id } => {
                write!(f, "Snapshot ID {snapshot_id} not found in lease manager")
            }
            Self::DuplicateSnapshotId(id) => write!(f, "Snapshot ID {id} is already actively leased"),
            Self::ZeroSnapshotId => write!(f, "Snapshot ID cannot be zero"),
            Self::ZeroLeaseEpochs => write!(f, "Snapshot lease duration in epochs cannot be zero"),
            Self::EpochCounterOverflow => write!(f, "Snapshot lease epoch counter overflowed u64::MAX"),
        }
    }
}

impl std::error::Error for SnapshotLeaseViolation {}

/// Metadata governing an active snapshot lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotLease {
    /// Unique snapshot ID.
    pub snapshot_id: u64,
    /// Point-in-time sequence cutoff.
    pub seq: u64,
    /// Epoch when snapshot was created.
    pub created_epoch: u64,
    /// Maximum allowed epoch duration.
    pub lease_epochs: u64,
}

impl SnapshotLease {
    /// Returns the epoch after which this lease is considered dead.
    #[must_use]
    pub fn expiration_epoch(&self) -> u64 {
        self.created_epoch.saturating_add(self.lease_epochs)
    }

    /// Checks if this lease is still valid at `current_epoch`.
    #[must_use]
    pub fn is_valid_at(&self, current_epoch: u64) -> bool {
        current_epoch <= self.expiration_epoch()
    }
}

/// Manager tracking active snapshot leases and computing the safe purge horizon.
#[derive(Clone, Debug, Default)]
pub struct SnapshotLeaseManager {
    /// Current engine epoch counter.
    pub current_epoch: u64,
    /// Active snapshots: snapshot_id -> SnapshotLease
    pub active_leases: BTreeMap<u64, SnapshotLease>,
}

impl SnapshotLeaseManager {
    /// Creates a new lease manager.
    #[must_use]
    pub fn new(initial_epoch: u64) -> Self {
        Self {
            current_epoch: initial_epoch,
            active_leases: BTreeMap::new(),
        }
    }

    /// Safely advances the engine epoch, failing closed on overflow.
    pub fn try_advance_epoch(&mut self) -> Result<u64, SnapshotLeaseViolation> {
        let next = self
            .current_epoch
            .checked_add(1)
            .ok_or(SnapshotLeaseViolation::EpochCounterOverflow)?;
        self.current_epoch = next;
        Ok(next)
    }

    /// Advances the engine epoch (saturating at `u64::MAX` rather than overflowing).
    pub fn advance_epoch(&mut self) -> u64 {
        self.current_epoch = self.current_epoch.saturating_add(1);
        self.current_epoch
    }

    /// Safely allocates a new snapshot lease, preventing duplicate IDs or zero duration.
    pub fn try_acquire_snapshot(
        &mut self,
        snapshot_id: u64,
        seq: u64,
        lease_epochs: u64,
    ) -> Result<SnapshotLease, SnapshotLeaseViolation> {
        if snapshot_id == 0 {
            return Err(SnapshotLeaseViolation::ZeroSnapshotId);
        }
        if lease_epochs == 0 {
            return Err(SnapshotLeaseViolation::ZeroLeaseEpochs);
        }
        if self.active_leases.contains_key(&snapshot_id) {
            return Err(SnapshotLeaseViolation::DuplicateSnapshotId(snapshot_id));
        }

        let lease = SnapshotLease {
            snapshot_id,
            seq,
            created_epoch: self.current_epoch,
            lease_epochs,
        };
        self.active_leases.insert(snapshot_id, lease);
        Ok(lease)
    }

    /// Allocates a new snapshot lease, overwriting if existing (deprecated, use `try_acquire_snapshot`).
    pub fn acquire_snapshot(
        &mut self,
        snapshot_id: u64,
        seq: u64,
        lease_epochs: u64,
    ) -> SnapshotLease {
        let lease = SnapshotLease {
            snapshot_id,
            seq,
            created_epoch: self.current_epoch,
            lease_epochs,
        };
        self.active_leases.insert(snapshot_id, lease);
        lease
    }

    /// Releases a snapshot when explicitly closed by the client.
    pub fn release_snapshot(&mut self, snapshot_id: u64) {
        self.active_leases.remove(&snapshot_id);
    }

    /// Prunes expired snapshot leases to prevent memory bloat.
    /// Returns the number of pruned leases.
    pub fn prune_expired_leases(&mut self) -> usize {
        let initial_count = self.active_leases.len();
        self.active_leases
            .retain(|_, lease| lease.is_valid_at(self.current_epoch));
        initial_count.saturating_sub(self.active_leases.len())
    }

    /// Authorizes a read on a snapshot, verifying lease validity.
    ///
    /// # Errors
    /// Returns `SnapshotLeaseViolation::SnapshotLeaseExpired` if epoch horizon has passed.
    pub fn verify_read_access(&self, snapshot_id: u64) -> Result<u64, SnapshotLeaseViolation> {
        let lease = self
            .active_leases
            .get(&snapshot_id)
            .ok_or(SnapshotLeaseViolation::UnknownSnapshotId { snapshot_id })?;

        if !lease.is_valid_at(self.current_epoch) {
            return Err(SnapshotLeaseViolation::SnapshotLeaseExpired {
                snapshot_id,
                current_epoch: self.current_epoch,
                expired_at_epoch: lease.expiration_epoch(),
            });
        }

        Ok(lease.seq)
    }

    /// Calculates the safe minimum active sequence for GC and Tombstone Purging.
    /// Expired snapshots are strictly ignored, unblocking space reclamation.
    #[must_use]
    pub fn min_active_unexpired_seq(&self) -> Option<u64> {
        self.active_leases
            .values()
            .filter(|lease| lease.is_valid_at(self.current_epoch))
            .map(|lease| lease.seq)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_snapshot_id_rejected() {
        let mut manager = SnapshotLeaseManager::new(10);
        let err = manager.try_acquire_snapshot(0, 100, 5).unwrap_err();
        assert_eq!(err, SnapshotLeaseViolation::ZeroSnapshotId);
    }

    #[test]
    fn test_zero_lease_epochs_rejected() {
        let mut manager = SnapshotLeaseManager::new(10);
        let err = manager.try_acquire_snapshot(1, 100, 0).unwrap_err();
        assert_eq!(err, SnapshotLeaseViolation::ZeroLeaseEpochs);
    }

    #[test]
    fn test_duplicate_snapshot_id_rejected() {
        let mut manager = SnapshotLeaseManager::new(10);
        assert!(manager.try_acquire_snapshot(42, 100, 5).is_ok());
        let err = manager.try_acquire_snapshot(42, 200, 10).unwrap_err();
        assert_eq!(err, SnapshotLeaseViolation::DuplicateSnapshotId(42));
    }

    #[test]
    fn test_epoch_counter_overflow_rejected() {
        let mut manager = SnapshotLeaseManager::new(u64::MAX);
        let err = manager.try_advance_epoch().unwrap_err();
        assert_eq!(err, SnapshotLeaseViolation::EpochCounterOverflow);
        assert_eq!(manager.advance_epoch(), u64::MAX);
    }

    #[test]
    fn test_prune_expired_leases() {
        let mut manager = SnapshotLeaseManager::new(10);
        manager.try_acquire_snapshot(1, 100, 2).unwrap(); // expires at 12
        manager.try_acquire_snapshot(2, 200, 10).unwrap(); // expires at 20

        manager.current_epoch = 15;
        let pruned = manager.prune_expired_leases();
        assert_eq!(pruned, 1);
        assert!(!manager.active_leases.contains_key(&1));
        assert!(manager.active_leases.contains_key(&2));
    }
}

