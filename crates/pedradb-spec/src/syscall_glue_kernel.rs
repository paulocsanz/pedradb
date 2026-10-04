//! Syscall Glue Contracts for POSIX and io_uring (RFC-0332).
//!
//! Enforces mathematical contracts and axiomatic pre/post-conditions
//! on all low-level OS boundaries to eliminate "Uncontracted Glue" (AGENTS.md §4).
//! Guaranteed fail-closed on short writes, non-zero fsync returns,
//! CQ overflow, and unaligned Direct-I/O transfers.

#![forbid(unsafe_code)]

/// Violations resulting from POSIX syscall contract breaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosixSyscallViolation {
    /// File descriptor is negative or invalid.
    InvalidFd { fd: i32 },
    /// Buffer length for write operation is zero.
    ZeroLengthBuffer,
    /// Offset plus buffer length overflows `u64::MAX`.
    OffsetOverflow { offset: u64, len: u64 },
    /// Partial write occurred without complete loop or atomic guarantee.
    ShortWrite { requested: usize, actual: usize },
    /// `fdatasync` returned a non-zero exit code.
    FdatasyncFailed { rc: i32 },
    /// `fsync` returned a non-zero exit code.
    FsyncFailed { rc: i32 },
    /// `preallocate` / `fallocate` returned a non-zero exit code.
    PreallocateFailed { rc: i32 },
    /// `posix_fadvise` returned a non-zero exit code.
    FadviseFailed { rc: i32 },
    /// Direct-I/O boundary violation: offset, buffer address, or length not aligned to sector size.
    UnalignedSector {
        offset: u64,
        ptr_addr: usize,
        len: usize,
        sector_size: usize,
    },
    /// Memory-mapped file access exceeds file boundaries.
    MmapBoundsViolation {
        offset: u64,
        len: usize,
        file_len: u64,
    },
    /// Memory mapping syscall returned MAP_FAILED.
    MmapFailed,
    /// Unrecognized file advise hint.
    UnrecognizedAdvice { advice: i32 },
    /// EINTR occurred during a sync barrier where durability state is uncertain.
    UncertainEintrSync,
}

impl std::fmt::Display for PosixSyscallViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFd { fd } => write!(f, "Invalid file descriptor: {fd}"),
            Self::ZeroLengthBuffer => write!(f, "Attempted write with zero-length buffer"),
            Self::OffsetOverflow { offset, len } => {
                write!(f, "Write offset overflow: offset={offset}, len={len}")
            }
            Self::ShortWrite { requested, actual } => {
                write!(f, "Short write: requested {requested} bytes, wrote {actual} bytes")
            }
            Self::FdatasyncFailed { rc } => write!(f, "fdatasync failed with rc={rc}"),
            Self::FsyncFailed { rc } => write!(f, "fsync failed with rc={rc}"),
            Self::PreallocateFailed { rc } => write!(f, "preallocate failed with rc={rc}"),
            Self::FadviseFailed { rc } => write!(f, "fadvise failed with rc={rc}"),
            Self::UnalignedSector { offset, ptr_addr, len, sector_size } => {
                write!(
                    f,
                    "Unaligned Direct-I/O: offset={offset}, ptr={ptr_addr:#x}, len={len} (sector={sector_size})"
                )
            }
            Self::MmapBoundsViolation { offset, len, file_len } => {
                write!(f, "Mmap bounds violation: offset={offset}, len={len}, file_len={file_len}")
            }
            Self::MmapFailed => write!(f, "mmap syscall failed (MAP_FAILED)"),
            Self::UnrecognizedAdvice { advice } => write!(f, "Unrecognized advice hint: {advice}"),
            Self::UncertainEintrSync => write!(f, "EINTR on sync barrier leaves media state uncertain"),
        }
    }
}

impl std::error::Error for PosixSyscallViolation {}

/// Violations resulting from io_uring completion and queue operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoUringViolation {
    /// SQE user_data is 0 (reserved sentinel).
    ZeroUserData,
    /// SQE user_data tag is not strictly monotonic.
    NonMonotonicUserData { prev: u64, current: u64 },
    /// Submission queue depth exceeds ring capacity.
    SubmissionQueueOverflow { current_depth: usize, capacity: usize },
    /// CQE returned a negative result code (OS errno).
    NegativeCompletionResult { res: i32 },
    /// CQE user_data tag mismatch with expected operation.
    TagMismatch { expected: u64, actual: u64 },
    /// Kernel completion queue overflow detected (IORING_SQ_CQ_OVERFLOW).
    CompletionQueueOverflow,
    /// Incomplete harvest: pending in-flight operations remain uncollected.
    IncompleteDrain { expected: usize, harvested: usize },
    /// Buffer address is not aligned for Direct-I/O transfer.
    UnalignedBuffer { ptr_addr: usize, sector_size: usize },
}

impl std::fmt::Display for IoUringViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroUserData => write!(f, "io_uring user_data cannot be 0"),
            Self::NonMonotonicUserData { prev, current } => {
                write!(f, "io_uring user_data non-monotonic: prev={prev}, current={current}")
            }
            Self::SubmissionQueueOverflow { current_depth, capacity } => {
                write!(f, "io_uring SQ overflow: depth={current_depth} >= capacity={capacity}")
            }
            Self::NegativeCompletionResult { res } => {
                write!(f, "io_uring CQE returned negative result: {res}")
            }
            Self::TagMismatch { expected, actual } => {
                write!(f, "io_uring tag mismatch: expected={expected}, actual={actual}")
            }
            Self::CompletionQueueOverflow => {
                write!(f, "io_uring kernel CQ overflow detected (IORING_SQ_CQ_OVERFLOW)")
            }
            Self::IncompleteDrain { expected, harvested } => {
                write!(f, "io_uring drain incomplete: expected {expected}, got {harvested}")
            }
            Self::UnalignedBuffer { ptr_addr, sector_size } => {
                write!(f, "io_uring unaligned buffer: ptr={ptr_addr:#x} (sector={sector_size})")
            }
        }
    }
}

impl std::error::Error for IoUringViolation {}

// =========================================================================
// POSIX Syscall Contracts
// =========================================================================

/// Validates preconditions before issuing a `pwrite` syscall.
#[must_use]
pub fn verify_pwrite_pre(fd: i32, len: usize, offset: u64) -> Result<(), PosixSyscallViolation> {
    if fd < 0 {
        return Err(PosixSyscallViolation::InvalidFd { fd });
    }
    if len == 0 {
        return Err(PosixSyscallViolation::ZeroLengthBuffer);
    }
    if offset.checked_add(len as u64).is_none() {
        return Err(PosixSyscallViolation::OffsetOverflow {
            offset,
            len: len as u64,
        });
    }
    Ok(())
}

/// Validates postconditions after issuing a `pwrite` syscall.
#[must_use]
pub fn verify_pwrite_post(requested: usize, written: isize) -> Result<usize, PosixSyscallViolation> {
    if written < 0 {
        return Err(PosixSyscallViolation::FdatasyncFailed { rc: written as i32 });
    }
    let actual = written as usize;
    if actual != requested {
        return Err(PosixSyscallViolation::ShortWrite { requested, actual });
    }
    Ok(actual)
}

/// Validates preconditions before issuing `fdatasync`.
#[must_use]
pub fn verify_fdatasync_pre(fd: i32) -> Result<(), PosixSyscallViolation> {
    if fd < 0 {
        return Err(PosixSyscallViolation::InvalidFd { fd });
    }
    Ok(())
}

/// Validates postconditions after issuing `fdatasync`.
#[must_use]
pub fn verify_fdatasync_post(rc: i32) -> Result<(), PosixSyscallViolation> {
    if rc != 0 {
        return Err(PosixSyscallViolation::FdatasyncFailed { rc });
    }
    Ok(())
}

/// Validates preconditions before issuing `fsync`.
#[must_use]
pub fn verify_fsync_pre(fd: i32) -> Result<(), PosixSyscallViolation> {
    if fd < 0 {
        return Err(PosixSyscallViolation::InvalidFd { fd });
    }
    Ok(())
}

/// Validates postconditions after issuing `fsync`.
#[must_use]
pub fn verify_fsync_post(rc: i32) -> Result<(), PosixSyscallViolation> {
    if rc != 0 {
        return Err(PosixSyscallViolation::FsyncFailed { rc });
    }
    Ok(())
}

/// Validates preconditions before issuing `preallocate` / `fallocate`.
#[must_use]
pub fn verify_preallocate_pre(fd: i32, offset: u64, len: u64) -> Result<(), PosixSyscallViolation> {
    if fd < 0 {
        return Err(PosixSyscallViolation::InvalidFd { fd });
    }
    if len == 0 {
        return Err(PosixSyscallViolation::ZeroLengthBuffer);
    }
    if offset.checked_add(len).is_none() {
        return Err(PosixSyscallViolation::OffsetOverflow { offset, len });
    }
    Ok(())
}

/// Validates postconditions after issuing `preallocate` / `fallocate`.
#[must_use]
pub fn verify_preallocate_post(rc: i32) -> Result<(), PosixSyscallViolation> {
    if rc != 0 {
        return Err(PosixSyscallViolation::PreallocateFailed { rc });
    }
    Ok(())
}

/// Validates preconditions before issuing `posix_fadvise`.
#[must_use]
pub fn verify_fadvise_pre(fd: i32, offset: u64, len: u64) -> Result<(), PosixSyscallViolation> {
    if fd < 0 {
        return Err(PosixSyscallViolation::InvalidFd { fd });
    }
    if offset.checked_add(len).is_none() {
        return Err(PosixSyscallViolation::OffsetOverflow { offset, len });
    }
    Ok(())
}

/// Validates postconditions after issuing `posix_fadvise`.
#[must_use]
pub fn verify_fadvise_post(rc: i32) -> Result<(), PosixSyscallViolation> {
    if rc != 0 {
        return Err(PosixSyscallViolation::FadviseFailed { rc });
    }
    Ok(())
}

/// Validates Direct-I/O 4096-byte alignment invariants for offset, buffer pointer, and length.
#[must_use]
pub fn verify_direct_io_alignment(
    offset: u64,
    ptr_addr: usize,
    len: usize,
    sector_size: usize,
) -> Result<(), PosixSyscallViolation> {
    if sector_size == 0 {
        return Err(PosixSyscallViolation::UnalignedSector {
            offset,
            ptr_addr,
            len,
            sector_size,
        });
    }
    let sec = sector_size as u64;
    let sec_usize = sector_size;
    if offset % sec != 0 || ptr_addr % sec_usize != 0 || len % sec_usize != 0 {
        return Err(PosixSyscallViolation::UnalignedSector {
            offset,
            ptr_addr,
            len,
            sector_size,
        });
    }
    Ok(())
}

/// Validates memory-mapped file bounds and ensures no out-of-bounds access.
#[must_use]
pub fn verify_mmap_bounds(
    offset: u64,
    len: usize,
    file_len: u64,
) -> Result<(), PosixSyscallViolation> {
    let end = match offset.checked_add(len as u64) {
        Some(e) => e,
        None => {
            return Err(PosixSyscallViolation::OffsetOverflow {
                offset,
                len: len as u64,
            })
        }
    };
    if end > file_len {
        return Err(PosixSyscallViolation::MmapBoundsViolation {
            offset,
            len,
            file_len,
        });
    }
    Ok(())
}

// =========================================================================
// io_uring Contracts
// =========================================================================

/// Validates SQE submission invariants: user_data monotonicity and queue depth bounds.
#[must_use]
pub fn verify_sqe_submission(
    user_data: u64,
    prev_user_data: u64,
    sq_depth: usize,
    ring_capacity: usize,
) -> Result<(), IoUringViolation> {
    if user_data == 0 {
        return Err(IoUringViolation::ZeroUserData);
    }
    if prev_user_data > 0 && user_data <= prev_user_data {
        return Err(IoUringViolation::NonMonotonicUserData {
            prev: prev_user_data,
            current: user_data,
        });
    }
    if sq_depth >= ring_capacity {
        return Err(IoUringViolation::SubmissionQueueOverflow {
            current_depth: sq_depth,
            capacity: ring_capacity,
        });
    }
    Ok(())
}

/// Validates CQE harvest postconditions: negative results, tag matching, and overflow.
#[must_use]
pub fn verify_cqe_harvest(
    cqe_res: i32,
    cqe_user_data: u64,
    expected_user_data: u64,
    overflow_detected: bool,
) -> Result<usize, IoUringViolation> {
    if overflow_detected {
        return Err(IoUringViolation::CompletionQueueOverflow);
    }
    if cqe_res < 0 {
        return Err(IoUringViolation::NegativeCompletionResult { res: cqe_res });
    }
    if cqe_user_data != expected_user_data {
        return Err(IoUringViolation::TagMismatch {
            expected: expected_user_data,
            actual: cqe_user_data,
        });
    }
    Ok(cqe_res as usize)
}

/// Validates that an io_uring completion queue drain completely collected all in-flight ops.
#[must_use]
pub fn verify_cqe_drain(expected: usize, harvested: usize) -> Result<(), IoUringViolation> {
    if harvested < expected {
        return Err(IoUringViolation::IncompleteDrain { expected, harvested });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_syscall_contracts_red_to_green() {
        assert_eq!(
            verify_pwrite_pre(-1, 10, 0),
            Err(PosixSyscallViolation::InvalidFd { fd: -1 })
        );
        assert_eq!(
            verify_pwrite_pre(3, 0, 0),
            Err(PosixSyscallViolation::ZeroLengthBuffer)
        );
        assert_eq!(
            verify_pwrite_pre(3, 10, u64::MAX - 5),
            Err(PosixSyscallViolation::OffsetOverflow {
                offset: u64::MAX - 5,
                len: 10
            })
        );
        assert!(verify_pwrite_pre(3, 10, 100).is_ok());

        assert_eq!(
            verify_pwrite_post(100, 90),
            Err(PosixSyscallViolation::ShortWrite {
                requested: 100,
                actual: 90
            })
        );
        assert_eq!(verify_pwrite_post(100, 100), Ok(100));

        assert_eq!(verify_fdatasync_pre(-1), Err(PosixSyscallViolation::InvalidFd { fd: -1 }));
        assert!(verify_fdatasync_pre(3).is_ok());
        assert_eq!(verify_fdatasync_post(-5), Err(PosixSyscallViolation::FdatasyncFailed { rc: -5 }));
        assert!(verify_fdatasync_post(0).is_ok());

        assert_eq!(
            verify_direct_io_alignment(4095, 0x1000, 4096, 4096),
            Err(PosixSyscallViolation::UnalignedSector {
                offset: 4095,
                ptr_addr: 0x1000,
                len: 4096,
                sector_size: 4096
            })
        );
        assert!(verify_direct_io_alignment(8192, 0x2000, 4096, 4096).is_ok());
    }

    #[test]
    fn test_io_uring_contracts_red_to_green() {
        assert_eq!(
            verify_sqe_submission(0, 0, 1, 128),
            Err(IoUringViolation::ZeroUserData)
        );
        assert_eq!(
            verify_sqe_submission(10, 10, 1, 128),
            Err(IoUringViolation::NonMonotonicUserData { prev: 10, current: 10 })
        );
        assert_eq!(
            verify_sqe_submission(11, 10, 128, 128),
            Err(IoUringViolation::SubmissionQueueOverflow { current_depth: 128, capacity: 128 })
        );
        assert!(verify_sqe_submission(11, 10, 5, 128).is_ok());

        assert_eq!(
            verify_cqe_harvest(-5, 11, 11, false),
            Err(IoUringViolation::NegativeCompletionResult { res: -5 })
        );
        assert_eq!(
            verify_cqe_harvest(4096, 12, 11, false),
            Err(IoUringViolation::TagMismatch { expected: 11, actual: 12 })
        );
        assert_eq!(
            verify_cqe_harvest(4096, 11, 11, true),
            Err(IoUringViolation::CompletionQueueOverflow)
        );
        assert_eq!(verify_cqe_harvest(4096, 11, 11, false), Ok(4096));

        assert_eq!(
            verify_cqe_drain(10, 9),
            Err(IoUringViolation::IncompleteDrain { expected: 10, harvested: 9 })
        );
        assert!(verify_cqe_drain(10, 10).is_ok());
    }
}
