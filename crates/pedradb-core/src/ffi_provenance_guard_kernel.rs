//! RFC-0326: FFI Provenance & Pointer Safety Guard Kernel.
//!
//! Provides strict validation of raw memory buffers passing through the C-ABI / FFI
//! boundary (`pedradb-capi`), enforcing non-nullity, alignment, size bounds
//! (< `isize::MAX`), and absence of mutable aliasing under Miri and Tree Borrows.

#![forbid(unsafe_code)]

use std::fmt;

/// Maximum allowed buffer size across the FFI boundary (isize::MAX on target).
pub const MAX_SAFE_FFI_BUFFER_SIZE: usize = isize::MAX as usize;

/// Errors arising from FFI raw buffer provenance and boundary verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfiProvenanceError {
    /// Null raw pointer provided across the FFI boundary.
    NullPointerHazard,
    /// Memory address violates the required hardware alignment.
    MisalignedPointer { address: usize, required: usize },
    /// Buffer length exceeds safe target address space limits.
    LengthExceedsTargetLimit { len: usize, limit: usize },
    /// Buffer length is smaller than the required fixed schema or struct header.
    BufferLengthUnderflow { expected: usize, actual: usize },
    /// Memory ranges overlap, violating Rust aliasing and Tree Borrows rules.
    OverlappingBufferAliasing {
        first_range: (usize, usize),
        second_range: (usize, usize),
    },
}

impl fmt::Display for FfiProvenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullPointerHazard => write!(f, "FFI error: null raw pointer provided"),
            Self::MisalignedPointer { address, required } => {
                write!(f, "FFI error: pointer address 0x{address:x} is not aligned to {required} bytes")
            }
            Self::LengthExceedsTargetLimit { len, limit } => {
                write!(f, "FFI error: buffer length {len} exceeds safety limit {limit}")
            }
            Self::BufferLengthUnderflow { expected, actual } => {
                write!(f, "FFI error: buffer length {actual} is smaller than required {expected}")
            }
            Self::OverlappingBufferAliasing { first_range, second_range } => {
                write!(
                    f,
                    "FFI aliasing violation: range [0x{:x}..0x{:x}) overlaps with [0x{:x}..0x{:x})",
                    first_range.0, first_range.1, second_range.0, second_range.1
                )
            }
        }
    }
}

impl std::error::Error for FfiProvenanceError {}

/// Provenance-validated descriptor representing an FFI memory region.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SafeFfiBufferDescriptor {
    address: usize,
    len: usize,
}

impl SafeFfiBufferDescriptor {
    /// Validates raw pointer parameters from C-ABI, enforcing non-nullity, alignment, and size bounds.
    pub fn try_new(
        address: usize,
        len: usize,
        required_alignment: usize,
    ) -> Result<Self, FfiProvenanceError> {
        if address == 0 {
            return Err(FfiProvenanceError::NullPointerHazard);
        }
        if required_alignment > 0 && address % required_alignment != 0 {
            return Err(FfiProvenanceError::MisalignedPointer {
                address,
                required: required_alignment,
            });
        }
        if len > MAX_SAFE_FFI_BUFFER_SIZE {
            return Err(FfiProvenanceError::LengthExceedsTargetLimit {
                len,
                limit: MAX_SAFE_FFI_BUFFER_SIZE,
            });
        }

        // Check for arithmetic overflow on address range calculation
        if address.checked_add(len).is_none() {
            return Err(FfiProvenanceError::LengthExceedsTargetLimit {
                len,
                limit: usize::MAX - address,
            });
        }

        Ok(Self { address, len })
    }

    /// Verifies that two FFI buffers are strictly disjoint (no aliasing violation).
    pub fn verify_disjoint(&self, other: &Self) -> Result<(), FfiProvenanceError> {
        let a_start = self.address;
        let a_end = self.address.saturating_add(self.len);

        let b_start = other.address;
        let b_end = other.address.saturating_add(other.len);

        // Disjoint condition: a_end <= b_start || b_end <= a_start
        let overlaps = !(a_end <= b_start || b_end <= a_start);
        if overlaps && (self.len > 0 && other.len > 0) {
            return Err(FfiProvenanceError::OverlappingBufferAliasing {
                first_range: (a_start, a_end),
                second_range: (b_start, b_end),
            });
        }

        Ok(())
    }

    /// Base address of the validated buffer.
    #[must_use]
    pub fn address(&self) -> usize {
        self.address
    }

    /// Validated length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer has zero length.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
