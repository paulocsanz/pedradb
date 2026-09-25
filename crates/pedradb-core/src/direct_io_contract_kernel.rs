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
    /// Alignment specified is invalid (must be a power of two >= 512).
    InvalidAlignmentRequirement {
        /// Invalid alignment value.
        alignment: usize,
    },
}

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
    if !required_alignment.is_power_of_two() || required_alignment < DIRECT_IO_SECTOR_ALIGNMENT {
        return Err(DirectIoAlignmentViolation::InvalidAlignmentRequirement {
            alignment: required_alignment,
        });
    }

    if transfer_len == 0 {
        return Err(DirectIoAlignmentViolation::ZeroLengthTransfer);
    }

    if ptr_addr.checked_add(transfer_len).is_none() {
        return Err(DirectIoAlignmentViolation::PointerArithmeticOverflow {
            ptr_addr,
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
    /// Allocates an aligned buffer with `capacity` rounded up to the nearest multiple of `alignment`.
    ///
    /// # Panics
    /// Panics if alignment is not a power of two >= 512.
    #[must_use]
    pub fn allocate(capacity: usize, alignment: usize) -> Self {
        assert!(
            alignment.is_power_of_two() && alignment >= DIRECT_IO_SECTOR_ALIGNMENT,
            "Alignment must be a power of two >= 512"
        );

        let aligned_len = if capacity == 0 {
            alignment
        } else {
            (capacity + alignment - 1) & !(alignment - 1)
        };

        // Allocate extra bytes so that regardless of where `raw_storage` lands,
        // we can slice out an `aligned_len` block that starts on an `alignment` boundary.
        let total_alloc = aligned_len + alignment;
        let raw_storage = vec![0u8; total_alloc];

        let base_ptr = raw_storage.as_ptr() as usize;
        let misaligned_rem = base_ptr % alignment;
        let aligned_offset = if misaligned_rem == 0 {
            0
        } else {
            alignment - misaligned_rem
        };

        Self {
            raw_storage,
            aligned_offset,
            usable_len: aligned_len,
        }
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
