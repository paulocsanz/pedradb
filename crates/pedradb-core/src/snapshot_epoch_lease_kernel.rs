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
}

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

    /// Advances the engine epoch (triggered by compactions, flushes, or wall clock).
    pub fn advance_epoch(&mut self) -> u64 {
        self.current_epoch += 1;
        self.current_epoch
    }

    /// Allocates a new snapshot lease.
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
