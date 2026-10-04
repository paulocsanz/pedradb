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
    /// Limite seguro do SO não pode ser zero.
    ZeroOsSafeLimit,
    /// Cota de armazenamento não pode ser zero.
    ZeroStorageLimit,
    /// Cota de rede mesh não pode ser zero.
    ZeroMeshLimit,
    /// Soma das cotas excede o teto seguro do SO.
    LimitsExceedOsCeiling { total: usize, os_limit: usize },
}

impl std::fmt::Display for FdQuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StorageQuotaExhausted { current, limit } => {
                write!(f, "Storage FD quota exhausted: {current} >= {limit}")
            }
            Self::MeshQuotaExhausted { current, limit } => {
                write!(f, "Mesh FD quota exhausted: {current} >= {limit}")
            }
            Self::GlobalCeilingReached { total_allocated, os_limit } => {
                write!(f, "Global ceiling reached: {total_allocated} >= {os_limit}")
            }
            Self::ZeroOsSafeLimit => write!(f, "OS safe limit cannot be zero"),
            Self::ZeroStorageLimit => write!(f, "Storage limit cannot be zero"),
            Self::ZeroMeshLimit => write!(f, "Mesh limit cannot be zero"),
            Self::LimitsExceedOsCeiling { total, os_limit } => {
                write!(f, "Limits {total} exceed OS ceiling {os_limit}")
            }
        }
    }
}

impl std::error::Error for FdQuotaError {}

/// Static FD partition and budget governor.
pub struct FdQuotaGovernor {
    storage_allocated: AtomicUsize,
    mesh_allocated: AtomicUsize,
    storage_limit: usize,
    mesh_limit: usize,
    os_safe_limit: usize,
}

impl FdQuotaGovernor {
    /// Creates a new governor with explicit safety partitions and bounds checking.
    pub fn try_new(storage_limit: usize, mesh_limit: usize, os_safe_limit: usize) -> Result<Self, FdQuotaError> {
        if os_safe_limit == 0 {
            return Err(FdQuotaError::ZeroOsSafeLimit);
        }
        if storage_limit == 0 {
            return Err(FdQuotaError::ZeroStorageLimit);
        }
        if mesh_limit == 0 {
            return Err(FdQuotaError::ZeroMeshLimit);
        }
        let total = storage_limit.saturating_add(mesh_limit);
        if total > os_safe_limit {
            return Err(FdQuotaError::LimitsExceedOsCeiling {
                total,
                os_limit: os_safe_limit,
            });
        }
        Ok(Self::new(storage_limit, mesh_limit, os_safe_limit))
    }

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
        let mut current = self.storage_allocated.load(Ordering::Acquire);
        loop {
            if current >= self.storage_limit {
                return Err(FdQuotaError::StorageQuotaExhausted {
                    current,
                    limit: self.storage_limit,
                });
            }

            let total = current + self.mesh_allocated.load(Ordering::Acquire);
            if total >= self.os_safe_limit {
                return Err(FdQuotaError::GlobalCeilingReached {
                    total_allocated: total,
                    os_limit: self.os_safe_limit,
                });
            }

            match self.storage_allocated.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(StorageFdLease { governor: self }),
                Err(actual) => current = actual,
            }
        }
    }

    /// Attempts to acquire a file descriptor lease for mesh network sockets.
    pub fn acquire_mesh_fd(&self) -> Result<MeshFdLease<'_>, FdQuotaError> {
        let mut current = self.mesh_allocated.load(Ordering::Acquire);
        loop {
            if current >= self.mesh_limit {
                return Err(FdQuotaError::MeshQuotaExhausted {
                    current,
                    limit: self.mesh_limit,
                });
            }

            let total = current + self.storage_allocated.load(Ordering::Acquire);
            if total >= self.os_safe_limit {
                return Err(FdQuotaError::GlobalCeilingReached {
                    total_allocated: total,
                    os_limit: self.os_safe_limit,
                });
            }

            match self.mesh_allocated.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(MeshFdLease { governor: self }),
                Err(actual) => current = actual,
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fd_quota_governor_bounds() {
        assert_eq!(
            FdQuotaGovernor::try_new(10, 10, 0).err(),
            Some(FdQuotaError::ZeroOsSafeLimit)
        );

        assert_eq!(
            FdQuotaGovernor::try_new(0, 10, 100).err(),
            Some(FdQuotaError::ZeroStorageLimit)
        );

        assert_eq!(
            FdQuotaGovernor::try_new(10, 0, 100).err(),
            Some(FdQuotaError::ZeroMeshLimit)
        );

        assert_eq!(
            FdQuotaGovernor::try_new(60, 50, 100).err(),
            Some(FdQuotaError::LimitsExceedOsCeiling {
                total: 110,
                os_limit: 100,
            })
        );

        let gov = FdQuotaGovernor::try_new(2, 2, 10).expect("valid governor");
        let lease1 = gov.acquire_storage_fd().expect("lease 1");
        let lease2 = gov.acquire_storage_fd().expect("lease 2");
        assert_eq!(
            gov.acquire_storage_fd().err(),
            Some(FdQuotaError::StorageQuotaExhausted {
                current: 2,
                limit: 2,
            })
        );
        drop(lease1);
        let _lease3 = gov.acquire_storage_fd().expect("lease 3");
        drop(lease2);
    }

    #[test]
    fn test_fd_quota_error_display() {
        let err = FdQuotaError::ZeroOsSafeLimit;
        assert_eq!(format!("{err}"), "OS safe limit cannot be zero");

        let err2 = FdQuotaError::LimitsExceedOsCeiling {
            total: 200,
            os_limit: 100,
        };
        assert_eq!(format!("{err2}"), "Limits 200 exceed OS ceiling 100");
    }
}

