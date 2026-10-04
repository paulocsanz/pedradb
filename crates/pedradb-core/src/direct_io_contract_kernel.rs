//! RFC-0281 P2.1 — Direct I/O 4096-Byte Alignment & DMA Coherence Contract Kernel.
//!
//! Formalizes hardware-level DMA contracts required by `O_DIRECT` and `io_uring`:
//!   1. Buffer Address Alignment: ptr_addr % 4096 == 0
//!   2. File Offset Alignment: file_offset % 4096 == 0
//!   3. Transfer Length Alignment: transfer_len % 4096 == 0 (and len > 0)
//!   4. Memory Coherence: no arithmetic pointer overflow during DMA transfer.
//!
//! Prevents EINVAL, kernel page bouncing, torn physical sector writes, and silent memory corruption.

#![forbid(unsafe_code)]

/// Standard page-aligned Direct I/O boundary (4096 bytes).
pub const DIRECT_IO_PAGE_ALIGNMENT: usize = 4096;

/// Legacy block-aligned Direct I/O boundary (512 bytes).
pub const DIRECT_IO_SECTOR_ALIGNMENT: usize = 512;

/// Maximum direct I/O transfer size (2 GiB - 4096 bytes) for POSIX SSIZE_MAX compliance.
pub const MAX_DIRECT_IO_TRANSFER_LEN: usize = 0x7fff_f000;

/// Maximum direct I/O alignment requirement supported (2 MiB huge page boundary).
pub const MAX_DIRECT_IO_ALIGNMENT: usize = 2 * 1024 * 1024;

/// Violations of Direct I/O alignment invariants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectIoAlignmentViolation {
    /// Memory buffer pointer is not aligned to the required block boundary.
    MisalignedBufferPointer {
        /// Address observed.
        ptr_addr: usize,
        /// Required alignment.
        required_alignment: usize,
    },
    /// File offset is not aligned to the required block boundary.
    MisalignedFileOffset {
        /// File offset observed.
        file_offset: u64,
        /// Required alignment.
        required_alignment: usize,
    },
    /// Transfer length is not a multiple of the required block boundary.
    MisalignedTransferLength {
        /// Transfer length observed.
        transfer_len: usize,
        /// Required alignment.
        required_alignment: usize,
    },
    /// Transfer length cannot be zero for Direct I/O DMA.
    ZeroLengthTransfer,
    /// Transfer span wraps around the address space.
    PointerArithmeticOverflow {
        /// Base address.
        ptr_addr: usize,
        /// Transfer length.
        transfer_len: usize,
    },
    /// File offset plus transfer length wraps around u64.
    FileOffsetOverflow {
        /// File offset.
        file_offset: u64,
        /// Transfer length.
        transfer_len: usize,
    },
    /// Transfer length exceeds maximum allowed by POSIX/hardware DMA.
    TransferLengthExceedsMaximum {
        /// Transfer length observed.
        transfer_len: usize,
        /// Maximum allowed transfer length.
        max_allowed: usize,
    },
    /// Memory allocation capacity overflowed during alignment rounding.
    AllocationOverflow {
        /// Requested capacity.
        capacity: usize,
        /// Required alignment.
        alignment: usize,
    },
    /// Alignment specified is invalid (must be a power of two between 512 and 2 MiB).
    InvalidAlignmentRequirement {
        /// Invalid alignment value.
        alignment: usize,
    },
    /// Requested slice range exceeds usable buffer boundary.
    SliceOutOfBounds {
        /// Offset.
        offset: usize,
        /// Length.
        len: usize,
        /// Total usable length.
        usable_len: usize,
    },
}

impl std::fmt::Display for DirectIoAlignmentViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MisalignedBufferPointer { ptr_addr, required_alignment } => {
                write!(f, "buffer pointer {ptr_addr:#x} is not aligned to {required_alignment} bytes")
            }
            Self::MisalignedFileOffset { file_offset, required_alignment } => {
                write!(f, "file offset {file_offset:#x} is not aligned to {required_alignment} bytes")
            }
            Self::MisalignedTransferLength { transfer_len, required_alignment } => {
                write!(f, "transfer length {transfer_len} is not a multiple of {required_alignment} bytes")
            }
            Self::ZeroLengthTransfer => write!(f, "transfer length cannot be zero for Direct I/O DMA"),
            Self::PointerArithmeticOverflow { ptr_addr, transfer_len } => {
                write!(f, "pointer arithmetic overflow: base {ptr_addr:#x} + len {transfer_len}")
            }
            Self::FileOffsetOverflow { file_offset, transfer_len } => {
                write!(f, "file offset overflow: offset {file_offset:#x} + len {transfer_len}")
            }
            Self::TransferLengthExceedsMaximum { transfer_len, max_allowed } => {
                write!(f, "transfer length {transfer_len} exceeds maximum allowed {max_allowed}")
            }
            Self::AllocationOverflow { capacity, alignment } => {
                write!(f, "allocation arithmetic overflow: capacity {capacity} with alignment {alignment}")
            }
            Self::InvalidAlignmentRequirement { alignment } => {
                write!(f, "invalid alignment requirement {alignment} (must be power of two between 512 and 2 MiB)")
            }
            Self::SliceOutOfBounds { offset, len, usable_len } => {
                write!(f, "slice range [{offset}..{}] out of usable bounds {usable_len}", offset + len)
            }
        }
    }
}

impl std::error::Error for DirectIoAlignmentViolation {}

/// Verifies whether an I/O request satisfies the Direct I/O contract.
///
/// # Errors
/// Returns `DirectIoAlignmentViolation` if any alignment or memory boundary invariant fails.
pub fn verify_direct_io_request(
    ptr_addr: usize,
    file_offset: u64,
    transfer_len: usize,
    required_alignment: usize,
) -> Result<(), DirectIoAlignmentViolation> {
    if !required_alignment.is_power_of_two()
        || required_alignment < DIRECT_IO_SECTOR_ALIGNMENT
        || required_alignment > MAX_DIRECT_IO_ALIGNMENT
    {
        return Err(DirectIoAlignmentViolation::InvalidAlignmentRequirement {
            alignment: required_alignment,
        });
    }

    if transfer_len == 0 {
        return Err(DirectIoAlignmentViolation::ZeroLengthTransfer);
    }

    if transfer_len > MAX_DIRECT_IO_TRANSFER_LEN {
        return Err(DirectIoAlignmentViolation::TransferLengthExceedsMaximum {
            transfer_len,
            max_allowed: MAX_DIRECT_IO_TRANSFER_LEN,
        });
    }

    if ptr_addr.checked_add(transfer_len).is_none() {
        return Err(DirectIoAlignmentViolation::PointerArithmeticOverflow {
            ptr_addr,
            transfer_len,
        });
    }

    if file_offset.checked_add(transfer_len as u64).is_none() {
        return Err(DirectIoAlignmentViolation::FileOffsetOverflow {
            file_offset,
            transfer_len,
        });
    }

    if ptr_addr % required_alignment != 0 {
        return Err(DirectIoAlignmentViolation::MisalignedBufferPointer {
            ptr_addr,
            required_alignment,
        });
    }

    if file_offset % (required_alignment as u64) != 0 {
        return Err(DirectIoAlignmentViolation::MisalignedFileOffset {
            file_offset,
            required_alignment,
        });
    }

    if transfer_len % required_alignment != 0 {
        return Err(DirectIoAlignmentViolation::MisalignedTransferLength {
            transfer_len,
            required_alignment,
        });
    }

    Ok(())
}

/// A safe, verified aligned memory container for Direct I/O operations.
/// Allocates sufficient headroom to guarantee 4096-byte alignment without unsafe code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlignedDirectBuffer {
    raw_storage: Vec<u8>,
    aligned_offset: usize,
    usable_len: usize,
}

impl AlignedDirectBuffer {
    /// Allocates an aligned buffer with `capacity` rounded up to the nearest multiple of `alignment`,
    /// returning a Result instead of panicking on invalid configuration.
    pub fn try_allocate(capacity: usize, alignment: usize) -> Result<Self, DirectIoAlignmentViolation> {
        if !alignment.is_power_of_two()
            || alignment < DIRECT_IO_SECTOR_ALIGNMENT
            || alignment > MAX_DIRECT_IO_ALIGNMENT
        {
            return Err(DirectIoAlignmentViolation::InvalidAlignmentRequirement { alignment });
        }

        let aligned_len = if capacity == 0 {
            alignment
        } else {
            let added = capacity
                .checked_add(alignment - 1)
                .ok_or(DirectIoAlignmentViolation::AllocationOverflow { capacity, alignment })?;
            added & !(alignment - 1)
        };

        let total_alloc = aligned_len
            .checked_add(alignment)
            .ok_or(DirectIoAlignmentViolation::AllocationOverflow { capacity, alignment })?;

        let raw_storage = vec![0u8; total_alloc];

        let base_ptr = raw_storage.as_ptr() as usize;
        let misaligned_rem = base_ptr % alignment;
        let aligned_offset = if misaligned_rem == 0 {
            0
        } else {
            alignment - misaligned_rem
        };

        Ok(Self {
            raw_storage,
            aligned_offset,
            usable_len: aligned_len,
        })
    }

    /// Allocates an aligned buffer with `capacity` rounded up to the nearest multiple of `alignment`.
    ///
    /// # Panics
    /// Panics if alignment is not a power of two between 512 and 2 MiB, or if allocation overflows.
    #[must_use]
    #[track_caller]
    pub fn allocate(capacity: usize, alignment: usize) -> Self {
        Self::try_allocate(capacity, alignment).expect("valid capacity and alignment required")
    }

    /// Returns the virtual memory address of the aligned start of the buffer.
    #[must_use]
    pub fn aligned_address(&self) -> usize {
        (self.raw_storage.as_ptr() as usize) + self.aligned_offset
    }

    /// Returns a slice over the aligned buffer content.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.raw_storage[self.aligned_offset..self.aligned_offset + self.usable_len]
    }

    /// Returns a mutable slice over the aligned buffer content.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.raw_storage[self.aligned_offset..self.aligned_offset + self.usable_len]
    }

    /// Returns a verified aligned sub-slice from the buffer.
    pub fn aligned_sub_slice(
        &self,
        offset: usize,
        len: usize,
        alignment: usize,
    ) -> Result<&[u8], DirectIoAlignmentViolation> {
        if offset % alignment != 0 {
            return Err(DirectIoAlignmentViolation::MisalignedBufferPointer {
                ptr_addr: self.aligned_address() + offset,
                required_alignment: alignment,
            });
        }
        if len % alignment != 0 {
            return Err(DirectIoAlignmentViolation::MisalignedTransferLength {
                transfer_len: len,
                required_alignment: alignment,
            });
        }
        let end = offset.checked_add(len).ok_or(DirectIoAlignmentViolation::PointerArithmeticOverflow {
            ptr_addr: offset,
            transfer_len: len,
        })?;
        if end > self.usable_len {
            return Err(DirectIoAlignmentViolation::SliceOutOfBounds {
                offset,
                len,
                usable_len: self.usable_len,
            });
        }
        Ok(&self.as_slice()[offset..end])
    }

    /// Returns the usable length of the aligned buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.usable_len
    }

    /// Returns whether the aligned buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.usable_len == 0
    }

    /// Verifies that this buffer strictly conforms to the direct I/O alignment requirement.
    ///
    /// # Errors
    /// Returns `DirectIoAlignmentViolation` if this buffer violates alignment.
    pub fn verify_conformance(
        &self,
        file_offset: u64,
        alignment: usize,
    ) -> Result<(), DirectIoAlignmentViolation> {
        verify_direct_io_request(self.aligned_address(), file_offset, self.usable_len, alignment)
    }
}
