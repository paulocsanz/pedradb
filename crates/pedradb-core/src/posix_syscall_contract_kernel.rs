//! RFC-0326: POSIX Syscall Contract Kernel.
//!
//! Provides formal precondition, postcondition, and error-handling contracts
//! for low-level POSIX and io_uring system calls (RFC-0273 Rule 4).
//! Ensures that any syscall failure (ENOSPC, EIO, EINTR, EBADF) deterministically
//! transitions the engine into a fail-closed or durability-fenced state.

#![forbid(unsafe_code)]

use std::fmt;

/// Standard POSIX error numbers.
pub const POSIX_EINTR: i32 = 4;
pub const POSIX_EIO: i32 = 5;
pub const POSIX_EBADF: i32 = 9;
pub const POSIX_EAGAIN: i32 = 11;
pub const POSIX_ENOSPC: i32 = 28;
pub const POSIX_EROFS: i32 = 30;
pub const POSIX_EDQUOT: i32 = 122;

/// Errors arising from POSIX syscall boundary contract evaluations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PosixContractError {
    /// Retry attempts exhausted after repeated EINTR signals.
    InterruptedExhausted { attempts: u32 },
    /// Physical block storage or quota capacity exhausted (ENOSPC/EDQUOT).
    StorageCapacityExhausted { path: String, errno: i32 },
    /// Permanent hardware I/O degradation (EIO/EFAULT/EROFS).
    HardwareDegraded { path: String, errno: i32 },
    /// Invalid or closed file descriptor encountered (EBADF).
    InvalidDescriptor { fd: i32 },
    /// Short write occurred where fewer bytes than requested were transferred.
    ShortWriteHazard { requested: usize, written: usize },
    /// Syscall invoked while the storage subsystem is actively durability-fenced.
    SyscallBlockedByDurabilityFence { op: &'static str },
}

impl fmt::Display for PosixContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InterruptedExhausted { attempts } => {
                write!(f, "POSIX EINTR signal retry limit reached after {attempts} attempts")
            }
            Self::StorageCapacityExhausted { path, errno } => {
                write!(f, "Storage capacity exhausted on '{path}' (errno {errno})")
            }
            Self::HardwareDegraded { path, errno } => {
                write!(f, "Hardware I/O degradation on '{path}' (errno {errno})")
            }
            Self::InvalidDescriptor { fd } => {
                write!(f, "Invalid file descriptor: {fd} is closed or corrupted")
            }
            Self::ShortWriteHazard { requested, written } => {
                write!(f, "Short write: requested {requested} bytes, only {written} written")
            }
            Self::SyscallBlockedByDurabilityFence { op } => {
                write!(f, "Syscall {op} blocked: engine is durability-fenced following an I/O error")
            }
        }
    }
}

impl std::error::Error for PosixContractError {}

/// Contract arbitrator guarding POSIX filesystem interactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosixSyscallContract {
    path: String,
    durability_fenced: bool,
    max_eintr_retries: u32,
    total_short_writes: u64,
}

impl PosixSyscallContract {
    /// Creates a new contract guard for a given file or partition path.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            durability_fenced: false,
            max_eintr_retries: 16,
            total_short_writes: 0,
        }
    }

    /// Evaluates the outcome of a `pwrite` syscall.
    ///
    /// - Fails closed if the engine is already durability-fenced.
    /// - Converts `ENOSPC`/`EDQUOT` into `StorageCapacityExhausted` and fences the contract.
    /// - Converts `EIO`/`EROFS` into `HardwareDegraded` and fences the contract.
    /// - Checks for short writes and flags them.
    pub fn evaluate_write_result(
        &mut self,
        fd: i32,
        requested_bytes: usize,
        result: Result<usize, i32>,
    ) -> Result<usize, PosixContractError> {
        if self.durability_fenced {
            return Err(PosixContractError::SyscallBlockedByDurabilityFence { op: "pwrite" });
        }
        if fd < 0 {
            return Err(PosixContractError::InvalidDescriptor { fd });
        }

        match result {
            Ok(written) => {
                if written < requested_bytes {
                    self.total_short_writes = self.total_short_writes.saturating_add(1);
                    Err(PosixContractError::ShortWriteHazard {
                        requested: requested_bytes,
                        written,
                    })
                } else {
                    Ok(written)
                }
            }
            Err(errno) => {
                if errno == POSIX_ENOSPC || errno == POSIX_EDQUOT {
                    self.durability_fenced = true;
                    Err(PosixContractError::StorageCapacityExhausted {
                        path: self.path.clone(),
                        errno,
                    })
                } else if errno == POSIX_EIO || errno == POSIX_EROFS {
                    self.durability_fenced = true;
                    Err(PosixContractError::HardwareDegraded {
                        path: self.path.clone(),
                        errno,
                    })
                } else if errno == POSIX_EBADF {
                    Err(PosixContractError::InvalidDescriptor { fd })
                } else {
                    self.durability_fenced = true;
                    Err(PosixContractError::HardwareDegraded {
                        path: self.path.clone(),
                        errno,
                    })
                }
            }
        }
    }

    /// Evaluates the outcome of an `fdatasync` syscall.
    pub fn evaluate_sync_result(
        &mut self,
        fd: i32,
        result: Result<(), i32>,
    ) -> Result<(), PosixContractError> {
        if self.durability_fenced {
            return Err(PosixContractError::SyscallBlockedByDurabilityFence { op: "fdatasync" });
        }
        if fd < 0 {
            return Err(PosixContractError::InvalidDescriptor { fd });
        }

        match result {
            Ok(()) => Ok(()),
            Err(errno) => {
                self.durability_fenced = true;
                if errno == POSIX_ENOSPC || errno == POSIX_EDQUOT {
                    Err(PosixContractError::StorageCapacityExhausted {
                        path: self.path.clone(),
                        errno,
                    })
                } else {
                    Err(PosixContractError::HardwareDegraded {
                        path: self.path.clone(),
                        errno,
                    })
                }
            }
        }
    }

    /// Evaluates an `EINTR` signal retry sequence, ensuring bounded repetition.
    pub fn check_eintr_retry(&self, attempts: u32) -> Result<(), PosixContractError> {
        if attempts >= self.max_eintr_retries {
            Err(PosixContractError::InterruptedExhausted { attempts })
        } else {
            Ok(())
        }
    }

    /// Whether the file/contract is currently durability-fenced.
    #[must_use]
    pub fn is_durability_fenced(&self) -> bool {
        self.durability_fenced
    }

    /// Total count of short writes observed.
    #[must_use]
    pub fn total_short_writes(&self) -> u64 {
        self.total_short_writes
    }

    /// Manually clears the durability fence following verified crash-recovery or healing.
    pub fn clear_fence_post_recovery(&mut self) {
        self.durability_fenced = false;
    }
}
