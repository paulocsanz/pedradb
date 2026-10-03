//! Atomic WAL Segment Seal and Preallocated Extent Reconciliation Kernel (RFC-0314).
//!
//! Provides mathematically verified invariants for preallocated physical WAL segments,
//! distinguishing legitimate unwritten trailing zeroes from physical bit-rot, and
//! encoding immutable EOF sentinel seal frames.
//!
//! # Problem Statement & Mathematical Foundation
//! High-throughput NVMe write paths preallocate WAL files (e.g. 64 MiB blocks) via `fallocate`
//! to eliminate metadata allocation locks.
//!
//! During crash recovery:
//! - Bytes $[0, P_{\text{logical}})$ contain valid, CRC-protected committed frames.
//! - Bytes $[P_{\text{logical}}, P_{\text{logical}} + 16)$ may contain the Sentinel Seal Frame $\mathcal{S}$.
//! - Bytes $[P_{\text{logical}} + |\mathcal{S}|, P_{\text{allocated}})$ are physical preallocated zeroes.
//!
//! # Safety Invariant
//! 1. `logical_bytes <= allocated_bytes` strictly conserved.
//! 2. Seal frame contains CRC32C over its magic and logical EOF offset.
//! 3. All trailing bytes past logical EOF or Seal are verified all-zero without panic.

#![forbid(unsafe_code)]

/// 4-byte ASCII magic identifier for Sentinel Seal Frame (`"SEAL"`).
pub const SEAL_MAGIC: u32 = 0x5345_414C;

/// Total wire size in bytes of the Sentinel Seal Frame.
pub const SEAL_FRAME_LEN: usize = 16;

/// Typed error outcomes for WAL segment seal operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalSealError {
    /// Attempted to write to or seal an already sealed WAL segment.
    AlreadySealed,
    /// Append exceeds the preallocated physical file capacity.
    ExceedsCapacity { requested: u64, available: u64 },
    /// Insufficient preallocated physical headroom to append the 16-byte seal frame.
    InsufficientHeadroomForSeal { available: u64, needed: usize },
    /// Preallocated physical size must be greater than zero.
    InvalidPreallocationSize(u64),
}

/// Result of parsing a candidate seal or trailing block in a WAL file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealParseResult {
    /// Valid sentinel seal frame found, confirming exact logical EOF.
    ValidSeal {
        /// Logical EOF byte offset where data frames cease.
        logical_eof: u64,
    },
    /// The span consists entirely of physical preallocated zeroes (clean crash boundary).
    TrailingZeroes,
    /// Invalid header or CRC mismatch.
    CorruptFrame(String),
}

/// State tracking physical allocation versus logical data within a WAL segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalPreallocationTracker {
    allocated_bytes: u64,
    logical_bytes: u64,
    sealed: bool,
}

impl WalPreallocationTracker {
    /// Attempts to create a new preallocation tracker, validating non-zero file capacity.
    pub fn try_new(allocated_bytes: u64) -> Result<Self, WalSealError> {
        if allocated_bytes == 0 {
            return Err(WalSealError::InvalidPreallocationSize(0));
        }
        Ok(Self {
            allocated_bytes,
            logical_bytes: 0,
            sealed: false,
        })
    }

    /// Creates a new preallocation tracker with configured physical file capacity.
    ///
    /// # Panics
    /// Panics if `allocated_bytes == 0`.
    #[must_use]
    pub fn new(allocated_bytes: u64) -> Self {
        Self::try_new(allocated_bytes).expect("valid preallocation size")
    }

    /// Physical bytes preallocated on disk.
    #[must_use]
    pub fn allocated_bytes(&self) -> u64 {
        self.allocated_bytes
    }

    /// Logical bytes currently occupied by committed records.
    #[must_use]
    pub fn logical_bytes(&self) -> u64 {
        self.logical_bytes
    }

    /// Whether this WAL segment has been gracefully closed with a sentinel seal.
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Remaining preallocated physical headroom in bytes.
    #[must_use]
    pub fn remaining_headroom(&self) -> u64 {
        self.allocated_bytes.saturating_sub(self.logical_bytes)
    }

    /// Checks if a proposed append of `len` bytes fits within the preallocated extent.
    #[must_use]
    pub fn can_append(&self, len: u64) -> bool {
        if self.sealed {
            return false;
        }
        self.logical_bytes.saturating_add(len) <= self.allocated_bytes
    }

    /// Advances the logical write offset by `len` bytes after durable write.
    pub fn advance_logical(&mut self, len: u64) -> Result<(), WalSealError> {
        if self.sealed {
            return Err(WalSealError::AlreadySealed);
        }
        let new_offset = self.logical_bytes.saturating_add(len);
        if new_offset > self.allocated_bytes {
            return Err(WalSealError::ExceedsCapacity {
                requested: len,
                available: self.remaining_headroom(),
            });
        }
        self.logical_bytes = new_offset;
        Ok(())
    }

    /// Encodes an immutable 16-byte Sentinel Seal Frame marking logical EOF.
    pub fn encode_seal(&mut self) -> Result<[u8; SEAL_FRAME_LEN], WalSealError> {
        if self.sealed {
            return Err(WalSealError::AlreadySealed);
        }
        if self.logical_bytes.saturating_add(SEAL_FRAME_LEN as u64) > self.allocated_bytes {
            return Err(WalSealError::InsufficientHeadroomForSeal {
                available: self.remaining_headroom(),
                needed: SEAL_FRAME_LEN,
            });
        }

        let mut buf = [0u8; SEAL_FRAME_LEN];
        let magic_bytes = SEAL_MAGIC.to_le_bytes();
        let eof_bytes = self.logical_bytes.to_le_bytes();

        buf[0..4].copy_from_slice(&magic_bytes);
        buf[4..12].copy_from_slice(&eof_bytes);

        // Compute CRC32C over magic + logical_bytes (first 12 bytes)
        let crc = crc32c::crc32c(&buf[0..12]);
        buf[12..16].copy_from_slice(&crc.to_le_bytes());

        self.logical_bytes = self.logical_bytes.saturating_add(SEAL_FRAME_LEN as u64);
        self.sealed = true;
        Ok(buf)
    }

    /// Parses a buffer of bytes at a candidate boundary.
    #[must_use]
    pub fn parse_frame_or_seal(buf: &[u8]) -> SealParseResult {
        if buf.is_empty() {
            return SealParseResult::TrailingZeroes;
        }

        // Fast-path: check if entire buffer is zeroed
        if Self::verify_trailing_zeroes(buf) {
            return SealParseResult::TrailingZeroes;
        }

        if buf.len() < SEAL_FRAME_LEN {
            return SealParseResult::CorruptFrame(
                "Buffer too short to contain a valid seal frame".to_string(),
            );
        }

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != SEAL_MAGIC {
            return SealParseResult::CorruptFrame(format!(
                "Invalid seal magic: expected {SEAL_MAGIC:#x}, got {magic:#x}"
            ));
        }

        let logical_eof = u64::from_le_bytes([
            buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
        ]);
        let expected_crc = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        let actual_crc = crc32c::crc32c(&buf[0..12]);

        if actual_crc != expected_crc {
            return SealParseResult::CorruptFrame(format!(
                "Seal frame CRC mismatch: expected {expected_crc:#x}, computed {actual_crc:#x}"
            ));
        }

        SealParseResult::ValidSeal { logical_eof }
    }

    /// Parses candidate seal and verifies Invariant 2: all trailing bytes in `buf` past
    /// the seal frame must be strictly preallocated zeroes.
    #[must_use]
    pub fn parse_and_verify_seal_extent(buf: &[u8]) -> SealParseResult {
        if buf.is_empty() {
            return SealParseResult::TrailingZeroes;
        }
        if Self::verify_trailing_zeroes(buf) {
            return SealParseResult::TrailingZeroes;
        }

        let seal_window_len = SEAL_FRAME_LEN.min(buf.len());
        let seal_res = Self::parse_frame_or_seal(&buf[..seal_window_len]);

        match seal_res {
            SealParseResult::ValidSeal { logical_eof } => {
                let trailing_slice = &buf[SEAL_FRAME_LEN..];
                if !Self::verify_trailing_zeroes(trailing_slice) {
                    SealParseResult::CorruptFrame(
                        "Non-zero trailing bytes found after sentinel seal frame".to_string(),
                    )
                } else {
                    SealParseResult::ValidSeal { logical_eof }
                }
            }
            other => other,
        }
    }

    /// Determines the exact physical file truncation offset for crash recovery.
    #[must_use]
    pub fn determine_recovery_truncation_offset(
        result: &SealParseResult,
        last_valid_record_offset: u64,
    ) -> Option<u64> {
        match result {
            SealParseResult::ValidSeal { logical_eof } => {
                Some(logical_eof.saturating_add(SEAL_FRAME_LEN as u64))
            }
            SealParseResult::TrailingZeroes => Some(last_valid_record_offset),
            SealParseResult::CorruptFrame(_) => None,
        }
    }

    /// Verifies whether all bytes in the slice are physical zeroes using 64-bit word chunks.
    #[must_use]
    pub fn verify_trailing_zeroes(buf: &[u8]) -> bool {
        let mut chunks = buf.chunks_exact(8);
        for chunk in &mut chunks {
            if u64::from_ne_bytes(chunk.try_into().unwrap()) != 0 {
                return false;
            }
        }
        chunks.remainder().iter().all(|&b| b == 0)
    }
}
