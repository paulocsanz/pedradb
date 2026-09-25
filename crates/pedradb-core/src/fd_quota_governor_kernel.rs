//! File Descriptor Quota Governor and EMFILE Prevention Kernel (RFC-0285 Pilar 8).
//!
//! Provides hard static budgeting across storage files and network sockets,
//! ensuring that reconnection storms and SST file opens never crash the node with EMFILE/ENFILE.
//!
//! Axiom:
//! FD_storage + FD_mesh <= MaxCapacity < OS_Limit.

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

/// Error when requesting file descriptor allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdQuotaError {
    /// Storage FD quota exceeded.
    StorageQuotaExhausted { current: usize, limit: usize },
    /// Network mesh FD quota exceeded (triggers graceful backpressure).
    MeshQuotaExhausted { current: usize, limit: usize },
    /// Process-wide global safety headroom limit reached.
    GlobalCeilingReached { total_allocated: usize, os_limit: usize },
}

/// Static FD partition and budget governor.
pub struct FdQuotaGovernor {
    storage_allocated: AtomicUsize,
    mesh_allocated: AtomicUsize,
    storage_limit: usize,
    mesh_limit: usize,
    os_safe_limit: usize,
}

impl FdQuotaGovernor {
    /// Creates a new governor with explicit safety partitions.
    /// Ensures `storage_limit + mesh_limit <= os_safe_limit`.
    pub fn new(storage_limit: usize, mesh_limit: usize, os_safe_limit: usize) -> Self {
        assert!(
            storage_limit + mesh_limit <= os_safe_limit,
            "Partition limits must not exceed OS safe ceiling"
        );
        Self {
            storage_allocated: AtomicUsize::new(0),
            mesh_allocated: AtomicUsize::new(0),
            storage_limit,
            mesh_limit,
            os_safe_limit,
        }
    }

    /// Attempts to acquire a file descriptor lease for storage (SST, WAL, Manifest).
    pub fn acquire_storage_fd(&self) -> Result<StorageFdLease<'_>, FdQuotaError> {
        let current = self.storage_allocated.load(Ordering::Relaxed);
        if current >= self.storage_limit {
            return Err(FdQuotaError::StorageQuotaExhausted {
                current,
                limit: self.storage_limit,
            });
        }

        let total = current + self.mesh_allocated.load(Ordering::Relaxed);
        if total >= self.os_safe_limit {
            return Err(FdQuotaError::GlobalCeilingReached {
                total_allocated: total,
                os_limit: self.os_safe_limit,
            });
        }

        self.storage_allocated.fetch_add(1, Ordering::SeqCst);
        Ok(StorageFdLease { governor: self })
    }

    /// Attempts to acquire a file descriptor lease for mesh network sockets.
    pub fn acquire_mesh_fd(&self) -> Result<MeshFdLease<'_>, FdQuotaError> {
        let current = self.mesh_allocated.load(Ordering::Relaxed);
        if current >= self.mesh_limit {
            return Err(FdQuotaError::MeshQuotaExhausted {
                current,
                limit: self.mesh_limit,
            });
        }

        let total = current + self.storage_allocated.load(Ordering::Relaxed);
        if total >= self.os_safe_limit {
            return Err(FdQuotaError::GlobalCeilingReached {
                total_allocated: total,
                os_limit: self.os_safe_limit,
            });
        }

        self.mesh_allocated.fetch_add(1, Ordering::SeqCst);
        Ok(MeshFdLease { governor: self })
    }

    /// Total active allocated FDs.
    pub fn total_allocated(&self) -> usize {
        self.storage_allocated.load(Ordering::Relaxed) + self.mesh_allocated.load(Ordering::Relaxed)
    }

    /// Active storage FDs.
    pub fn storage_allocated(&self) -> usize {
        self.storage_allocated.load(Ordering::Relaxed)
    }

    /// Active mesh network FDs.
    pub fn mesh_allocated(&self) -> usize {
        self.mesh_allocated.load(Ordering::Relaxed)
    }
}

/// RAII lease for storage file descriptor.
pub struct StorageFdLease<'a> {
    governor: &'a FdQuotaGovernor,
}

impl<'a> Drop for StorageFdLease<'a> {
    fn drop(&mut self) {
        self.governor.storage_allocated.fetch_sub(1, Ordering::SeqCst);
    }
}

/// RAII lease for mesh network socket descriptor.
pub struct MeshFdLease<'a> {
    governor: &'a FdQuotaGovernor,
}

impl<'a> Drop for MeshFdLease<'a> {
    fn drop(&mut self) {
        self.governor.mesh_allocated.fetch_sub(1, Ordering::SeqCst);
    }
}
