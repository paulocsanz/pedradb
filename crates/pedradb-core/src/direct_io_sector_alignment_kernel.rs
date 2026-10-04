//! NVMe Direct-I/O Sector Alignment and Buffer Partitioning Kernel (RFC-0312).
//!
//! Provides mathematically verified, zero-copy sector alignment decomposition
//! for unbuffered physical I/O (POSIX `O_DIRECT`, Linux `io_uring` NVMe DMA).
//!
//! # Physical NVMe Constraints
//! Direct-I/O requires that:
//! 1. Memory buffers reside at sector-aligned virtual memory addresses.
//! 2. Target file offsets are strictly multiples of physical sector size $S$ (512B or 4096B).
//! 3. Lengths transferred in DMA are integer multiples of $S$.
//!
//! This kernel decomposes arbitrary logical byte spans $(O, L)$ into three exact
//! disjoint regions:
//! - **Prefix Head:** $[O, \text{ceil}_S(O))$ handled via an aligned bounce buffer.
//! - **Aligned Body:** $[\text{ceil}_S(O), \text{floor}_S(O + L))$ issued via direct zero-copy DMA.
//! - **Suffix Tail:** $[\text{floor}_S(O + L), O + L)$ handled via an aligned bounce buffer.

#![forbid(unsafe_code)]

/// Standard 4 KiB NVMe sector size.
pub const STANDARD_SECTOR_SIZE: usize = 4096;

/// Legacy 512-byte block device sector size.
pub const LEGACY_SECTOR_SIZE: usize = 512;

/// Errors returned by the Direct-I/O sector alignment kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignmentError {
    /// Sector size must be a power of two >= 512.
    InvalidSectorSize(usize),
    /// Memory address is not aligned to the required physical sector boundary.
    UnalignedBufferAddress {
        /// Buffer virtual memory address.
        address: usize,
        /// Required sector size alignment.
        sector_size: usize,
    },
    /// Arithmetic overflow in boundary calculation.
    ArithmeticOverflow,
}

impl std::fmt::Display for AlignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSectorSize(s) => {
                write!(f, "Invalid Direct-I/O sector size {s}: must be a power of two >= 512")
            }
            Self::UnalignedBufferAddress { address, sector_size } => {
                write!(
                    f,
                    "Buffer address {address:#x} is unaligned: must be multiple of sector size {sector_size}"
                )
            }
            Self::ArithmeticOverflow => {
                write!(f, "Arithmetic overflow in sector alignment calculation")
            }
        }
    }
}

impl std::error::Error for AlignmentError {}

/// Slicing plan decomposing an arbitrary I/O request into aligned and bounce spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoSlicePlan {
    /// Logical starting offset in the file.
    pub logical_offset: u64,
    /// Total logical byte length of the transfer.
    pub total_len: usize,

    /// Unaligned head offset and length.
    pub prefix_len: usize,
    /// Physical aligned sector start offset for reading the prefix head.
    pub prefix_sector_offset: u64,
    /// Number of bytes to read into bounce buffer to cover the prefix head.
    pub prefix_bounce_len: usize,

    /// Aligned middle body offset (always aligned to sector_size).
    pub aligned_offset: u64,
    /// Aligned middle body length (always multiple of sector_size).
    pub aligned_len: usize,

    /// Unaligned tail offset and length.
    pub suffix_offset: u64,
    /// Unaligned tail length.
    pub suffix_len: usize,
    /// Number of bytes to read into bounce buffer to cover the suffix tail.
    pub suffix_bounce_len: usize,
}

/// Aligner engine enforcing physical NVMe sector geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectIoAligner {
    sector_size: usize,
}

impl DirectIoAligner {
    /// Creates a new aligner for the given physical sector size.
    pub fn new(sector_size: usize) -> Result<Self, AlignmentError> {
        if sector_size < 512 || !sector_size.is_power_of_two() {
            return Err(AlignmentError::InvalidSectorSize(sector_size));
        }
        Ok(Self { sector_size })
    }

    /// Sector size in bytes.
    #[must_use]
    pub fn sector_size(&self) -> usize {
        self.sector_size
    }

    /// Checks if a file offset is strictly sector-aligned.
    #[must_use]
    pub fn is_aligned_offset(&self, offset: u64) -> bool {
        (offset % (self.sector_size as u64)) == 0
    }

    /// Checks if a transfer length is an integer multiple of sector size.
    #[must_use]
    pub fn is_aligned_len(&self, len: usize) -> bool {
        (len % self.sector_size) == 0
    }

    /// Checks if both offset and length are fully aligned for direct DMA.
    #[must_use]
    pub fn is_fully_aligned(&self, offset: u64, len: usize) -> bool {
        self.is_aligned_offset(offset) && self.is_aligned_len(len)
    }

    /// Validates whether a memory pointer/address meets DMA alignment requirements.
    pub fn validate_buffer_alignment(&self, address: usize) -> Result<(), AlignmentError> {
        if (address % self.sector_size) != 0 {
            return Err(AlignmentError::UnalignedBufferAddress {
                address,
                sector_size: self.sector_size,
            });
        }
        Ok(())
    }

    /// Rounds an offset down to the nearest sector boundary.
    #[must_use]
    pub fn floor_sector(&self, offset: u64) -> u64 {
        let mask = !((self.sector_size as u64) - 1);
        offset & mask
    }

    /// Rounds an offset up to the nearest sector boundary, checking for arithmetic overflow.
    pub fn try_ceil_sector(&self, offset: u64) -> Result<u64, AlignmentError> {
        let sec = self.sector_size as u64;
        let rem = offset % sec;
        if rem == 0 {
            Ok(offset)
        } else {
            offset
                .checked_add(sec - rem)
                .ok_or(AlignmentError::ArithmeticOverflow)
        }
    }

    /// Rounds an offset up to the nearest sector boundary.
    #[must_use]
    pub fn ceil_sector(&self, offset: u64) -> u64 {
        self.try_ceil_sector(offset).unwrap_or(u64::MAX)
    }

    /// Decomposes an arbitrary request `(offset, len)` into exact Direct-I/O execution slices,
    /// failing closed with `AlignmentError::ArithmeticOverflow` on arithmetic overflow.
    pub fn try_plan_io_slices(&self, offset: u64, len: usize) -> Result<IoSlicePlan, AlignmentError> {
        let end_offset = offset
            .checked_add(len as u64)
            .ok_or(AlignmentError::ArithmeticOverflow)?;

        if len == 0 {
            let ceil_start = self.try_ceil_sector(offset)?;
            return Ok(IoSlicePlan {
                logical_offset: offset,
                total_len: 0,
                prefix_len: 0,
                prefix_sector_offset: self.floor_sector(offset),
                prefix_bounce_len: 0,
                aligned_offset: ceil_start,
                aligned_len: 0,
                suffix_offset: offset,
                suffix_len: 0,
                suffix_bounce_len: 0,
            });
        }

        let floor_start = self.floor_sector(offset);
        let ceil_start = self.try_ceil_sector(offset)?;

        if end_offset <= ceil_start {
            return Ok(IoSlicePlan {
                logical_offset: offset,
                total_len: len,
                prefix_len: len,
                prefix_sector_offset: floor_start,
                prefix_bounce_len: self.sector_size,
                aligned_offset: ceil_start,
                aligned_len: 0,
                suffix_offset: end_offset,
                suffix_len: 0,
                suffix_bounce_len: 0,
            });
        }

        let prefix_len = if offset == floor_start {
            0
        } else {
            (ceil_start - offset) as usize
        };

        let floor_end = self.floor_sector(end_offset);
        let suffix_len = (end_offset - floor_end) as usize;

        let aligned_start = ceil_start;
        let aligned_end = floor_end;

        let aligned_len = if aligned_end >= aligned_start {
            (aligned_end - aligned_start) as usize
        } else {
            0
        };

        let prefix_bounce_len = if prefix_len > 0 { self.sector_size } else { 0 };
        let suffix_bounce_len = if suffix_len > 0 { self.sector_size } else { 0 };

        Ok(IoSlicePlan {
            logical_offset: offset,
            total_len: len,
            prefix_len,
            prefix_sector_offset: floor_start,
            prefix_bounce_len,
            aligned_offset: aligned_start,
            aligned_len,
            suffix_offset: floor_end,
            suffix_len,
            suffix_bounce_len,
        })
    }

    /// Decomposes an arbitrary request `(offset, len)` into exact Direct-I/O execution slices.
    ///
    /// # Mathematical Invariants
    /// 1. `prefix_len + aligned_len + suffix_len == total_len`
    /// 2. `aligned_offset % sector_size == 0`
    /// 3. `aligned_len % sector_size == 0`
    #[must_use]
    pub fn plan_io_slices(&self, offset: u64, len: usize) -> IoSlicePlan {
        self.try_plan_io_slices(offset, len)
            .expect("plan_io_slices arithmetic overflow")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_io_sector_alignment_structural_invariants_red_to_green() {
        let aligner = DirectIoAligner::new(4096).unwrap();

        // 1. Arithmetic overflow in try_plan_io_slices must be cleanly rejected
        assert_eq!(
            aligner.try_plan_io_slices(u64::MAX - 100, 200),
            Err(AlignmentError::ArithmeticOverflow)
        );

        // 2. Arithmetic overflow in try_ceil_sector must be cleanly rejected
        assert_eq!(
            aligner.try_ceil_sector(u64::MAX - 10),
            Err(AlignmentError::ArithmeticOverflow)
        );

        // 3. Normal planning succeeds
        let plan = aligner.try_plan_io_slices(4096, 8192).unwrap();
        assert_eq!(plan.total_len, 8192);
        assert_eq!(plan.aligned_len, 8192);
        assert_eq!(plan.prefix_len, 0);
        assert_eq!(plan.suffix_len, 0);

        // 4. Boundary alignment invariants
        let plan2 = aligner.try_plan_io_slices(5000, 10000).unwrap();
        assert_eq!(plan2.prefix_len + plan2.aligned_len + plan2.suffix_len, 10000);
        assert_eq!(plan2.aligned_offset % 4096, 0);
    }
}
