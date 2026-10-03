//! kernel: sector_aligned_io
//! Sector-aligned Direct I/O (O_DIRECT / NVMe) plan computation and scatter-gather vector validation.
//!
//! Provides mathematically verified sub-sector padding offsets, sector boundary alignment,
//! and scatter-gather contiguous memory slice validation without allocation.

/// Maximum realistic hardware direct I/O sector size supported (64 KiB).
pub const MAX_DIRECT_IO_SECTOR_SIZE: u32 = 64 * 1024;

/// Typed errors produced during sector alignment calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorAlignError {
    /// Sector size must be a non-zero power of two <= 64 KiB (typically 512, 1024, 2048, 4096).
    InvalidSectorSize(u32),
    /// Payload length cannot be zero for Direct I/O operations.
    ZeroPayloadLength,
    /// Logical offset and length computation caused an integer overflow.
    OffsetOverflow,
    /// Scatter-gather slice contains an empty chunk (len == 0).
    EmptySliceChunk,
    /// Scatter-gather cumulative length does not match expected length.
    LengthMismatch { expected: usize, actual: usize },
    /// Scatter-gather slice offset and length overflowed address space.
    SliceAddressOverflow,
    /// Scatter-gather slice ranges overlap in memory, causing potential data corruption.
    OverlappingSliceRange { prev_end: usize, next_start: usize },
}

impl core::fmt::Display for SectorAlignError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSectorSize(s) => write!(f, "Invalid sector size (must be power of 2 <= 64 KiB): {}", s),
            Self::ZeroPayloadLength => write!(f, "Payload length cannot be zero"),
            Self::OffsetOverflow => write!(f, "Logical offset + length caused integer overflow"),
            Self::EmptySliceChunk => write!(f, "Scatter-gather chunk cannot have zero length"),
            Self::LengthMismatch { expected, actual } => {
                write!(f, "Scatter-gather length mismatch: expected {}, got {}", expected, actual)
            }
            Self::SliceAddressOverflow => write!(f, "Scatter-gather slice bounds overflow address space"),
            Self::OverlappingSliceRange { prev_end, next_start } => {
                write!(
                    f,
                    "Scatter-gather memory overlap detected: prev_end {} > next_start {}",
                    prev_end, next_start
                )
            }
        }
    }
}

impl std::error::Error for SectorAlignError {}

/// Direct I/O sector alignment execution plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorAlignedIoPlan {
    /// Target physical sector size in bytes (e.g. 4096).
    pub sector_size: u32,
    /// Logical starting offset in the file.
    pub logical_offset: u64,
    /// Exact payload length in bytes.
    pub payload_len: u64,
    /// Sector-aligned starting file offset (<= logical_offset).
    pub aligned_offset: u64,
    /// Total sector-aligned byte length including head and tail padding.
    pub aligned_len: u64,
    /// Prefix zero-padding bytes needed before the payload.
    pub head_padding: u32,
    /// Suffix zero-padding bytes needed after the payload to reach sector boundary.
    pub tail_padding: u32,
}

impl SectorAlignedIoPlan {
    /// Calculates the optimal sector-aligned I/O plan for a given logical offset and length.
    ///
    /// Validates power-of-two sector size <= 64 KiB, non-zero payload, and guards against arithmetic overflows.
    pub fn calculate(logical_offset: u64, payload_len: u64, sector_size: u32) -> Result<Self, SectorAlignError> {
        if sector_size == 0 || !sector_size.is_power_of_two() || sector_size > MAX_DIRECT_IO_SECTOR_SIZE {
            return Err(SectorAlignError::InvalidSectorSize(sector_size));
        }
        if payload_len == 0 {
            return Err(SectorAlignError::ZeroPayloadLength);
        }

        let sector_mask = (sector_size as u64) - 1;
        let head_padding = (logical_offset & sector_mask) as u32;
        let aligned_offset = logical_offset - (head_padding as u64);

        let logical_end = logical_offset
            .checked_add(payload_len)
            .ok_or(SectorAlignError::OffsetOverflow)?;

        let tail_remainder = logical_end & sector_mask;
        let tail_padding = if tail_remainder == 0 {
            0u32
        } else {
            sector_size - (tail_remainder as u32)
        };

        let aligned_end = logical_end
            .checked_add(tail_padding as u64)
            .ok_or(SectorAlignError::OffsetOverflow)?;

        let aligned_len = aligned_end
            .checked_sub(aligned_offset)
            .ok_or(SectorAlignError::OffsetOverflow)?;

        let plan = Self {
            sector_size,
            logical_offset,
            payload_len,
            aligned_offset,
            aligned_len,
            head_padding,
            tail_padding,
        };

        debug_assert!(plan.verify_internal_invariants());
        Ok(plan)
    }

    /// Returns `true` if the I/O operation is already perfectly sector-aligned with 0 padding.
    #[must_use]
    pub fn is_exact_sector_aligned(&self) -> bool {
        self.head_padding == 0 && self.tail_padding == 0
    }

    /// Returns the total number of physical sectors touched by this I/O extent.
    #[must_use]
    pub fn total_sectors(&self) -> u64 {
        if self.sector_size == 0 {
            0
        } else {
            self.aligned_len / (self.sector_size as u64)
        }
    }

    /// Verifies all mathematical invariants of the sector alignment plan.
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        if self.sector_size == 0 || !self.sector_size.is_power_of_two() || self.sector_size > MAX_DIRECT_IO_SECTOR_SIZE {
            return false;
        }
        let sec = self.sector_size as u64;
        if self.aligned_offset % sec != 0 {
            return false;
        }
        if self.aligned_len % sec != 0 {
            return false;
        }
        if self.aligned_offset > self.logical_offset {
            return false;
        }
        if self.head_padding as u64 != self.logical_offset - self.aligned_offset {
            return false;
        }
        let logical_end = match self.logical_offset.checked_add(self.payload_len) {
            Some(end) => end,
            None => return false,
        };
        let expected_aligned_end = match self.aligned_offset.checked_add(self.aligned_len) {
            Some(end) => end,
            None => return false,
        };
        let end_plus_tail = match logical_end.checked_add(self.tail_padding as u64) {
            Some(end) => end,
            None => return false,
        };
        if expected_aligned_end != end_plus_tail {
            return false;
        }
        true
    }
}

/// Validates scatter-gather I/O memory vector slice entries.
pub struct ScatterGatherValidator;

impl ScatterGatherValidator {
    /// Validates an array of scatter-gather slices (offset, len) against expected total length.
    /// Ensures non-empty slices, non-overlapping addresses, and zero address overflow.
    pub fn validate_iov_slices(slices: &[(usize, usize)], total_expected_len: usize) -> Result<usize, SectorAlignError> {
        let mut cumulative_len: usize = 0;
        let mut last_end: Option<usize> = None;

        for &(offset, len) in slices {
            if len == 0 {
                return Err(SectorAlignError::EmptySliceChunk);
            }
            let slice_end = offset
                .checked_add(len)
                .ok_or(SectorAlignError::SliceAddressOverflow)?;

            if let Some(prev_end) = last_end {
                if offset < prev_end {
                    return Err(SectorAlignError::OverlappingSliceRange {
                        prev_end,
                        next_start: offset,
                    });
                }
            }
            last_end = Some(slice_end);

            cumulative_len = cumulative_len
                .checked_add(len)
                .ok_or(SectorAlignError::SliceAddressOverflow)?;
        }

        if cumulative_len != total_expected_len {
            return Err(SectorAlignError::LengthMismatch {
                expected: total_expected_len,
                actual: cumulative_len,
            });
        }

        Ok(cumulative_len)
    }
}
