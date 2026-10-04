//! RFC-0324: WAL Crash Recovery Reconciler Kernel.
//!
//! Eradicates O4 crash recovery failure modes (`reopen after crash failed: internal error:
//! WAL resync skipped damaged region mid-log`) by implementing a deterministic classification
//! and reconciliation state machine for torn writes, short writes, preallocated trailing zeros,
//! Sentinel Seals (RFC-0314), and mid-log corruption isolation (RFC-0321).

#![forbid(unsafe_code)]

use std::fmt;

/// Sentinel magic for sealed WAL segments (RFC-0314: `0x5345414C` == ASCII 'SEAL').
pub const WAL_SENTINEL_SEAL_MAGIC: u32 = 0x5345_414C;

/// Standard WAL frame header magic (0x50454452 == ASCII 'PEDR').
pub const WAL_RECORD_MAGIC: u32 = 0x5045_4452;

/// Diagnostic classification of an encountered WAL region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Valid record with monotonic sequence and valid CRC.
    AcceptRecord {
        seq: u64,
        payload_len: u32,
        next_offset: u64,
    },
    /// Clean sentinel seal encountered: recovery terminates successfully at logical offset.
    SentinelSealEncountered {
        seal_seq: u64,
        logical_offset: u64,
    },
    /// Clean EOF reached at end of file with no trailing anomalies.
    CleanEof {
        total_records: u64,
        final_offset: u64,
    },
    /// Trailing zeros detected in preallocated segment: safe torn tail truncate.
    TrailingZerosTruncate {
        valid_offset: u64,
        zero_bytes: u64,
    },
    /// Partial torn write at the end of the log: truncated to last valid offset.
    TornTailTruncate {
        valid_offset: u64,
        torn_bytes: u64,
        reason: &'static str,
    },
    /// Mid-log damage isolated and quarantined; recovery continues without blocking reopen.
    QuarantinedRegion {
        start_offset: u64,
        quarantined_bytes: u64,
        reason: &'static str,
    },
}

/// Recovery error domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalRecoveryError {
    /// Non-monotonic sequence encountered in validly framed records.
    NonMonotonicSequence { previous: u64, found: u64, offset: u64 },
    /// File size is shorter than claimed logical seal offset.
    SealOffsetExceedsFileSize { seal_offset: u64, file_size: u64 },
    /// Unrecoverable structural header mismatch at offset 0.
    CorruptedSegmentHeader { found_magic: u32 },
}

impl fmt::Display for WalRecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicSequence { previous, found, offset } => {
                write!(f, "Non-monotonic sequence: previous {previous} >= found {found} at offset {offset}")
            }
            Self::SealOffsetExceedsFileSize { seal_offset, file_size } => {
                write!(f, "Seal offset {seal_offset} exceeds total file size {file_size}")
            }
            Self::CorruptedSegmentHeader { found_magic } => {
                write!(f, "Invalid segment superblock magic: 0x{found_magic:08x}")
            }
        }
    }
}

impl std::error::Error for WalRecoveryError {}

/// State machine reconciling WAL frames during crash recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalCrashRecoveryReconciler {
    last_valid_seq: u64,
    last_valid_offset: u64,
    total_valid_records: u64,
    quarantined_regions_count: u64,
    is_sealed: bool,
}

impl Default for WalCrashRecoveryReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl WalCrashRecoveryReconciler {
    /// Creates a new recovery reconciler starting at offset 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_valid_seq: 0,
            last_valid_offset: 0,
            total_valid_records: 0,
            quarantined_regions_count: 0,
            is_sealed: false,
        }
    }

    /// Evaluates a raw header and available buffer at `current_offset`.
    ///
    /// - `header`: 16-byte candidate slice `[magic: 4, seq: 8, len: 4]`.
    /// - `crc_matches`: boolean indicating whether CRC of payload matches expected CRC.
    /// - `file_size`: total physical size of the WAL file on disk.
    pub fn reconcile_chunk(
        &mut self,
        current_offset: u64,
        header: &[u8],
        crc_matches: bool,
        file_size: u64,
    ) -> Result<RecoveryAction, WalRecoveryError> {
        if self.is_sealed {
            return Ok(RecoveryAction::CleanEof {
                total_records: self.total_valid_records,
                final_offset: self.last_valid_offset,
            });
        }

        // EOF check
        if current_offset >= file_size {
            return Ok(RecoveryAction::CleanEof {
                total_records: self.total_valid_records,
                final_offset: self.last_valid_offset,
            });
        }

        let remaining = file_size.saturating_sub(current_offset);

        // Header underflow check: if remaining bytes are fewer than 16 bytes
        if header.len() < 16 || remaining < 16 {
            // If all remaining bytes are zeros, it is preallocated padding
            if header.iter().all(|&b| b == 0) {
                return Ok(RecoveryAction::TrailingZerosTruncate {
                    valid_offset: self.last_valid_offset,
                    zero_bytes: remaining,
                });
            }
            // Otherwise, it is a torn write at the tail
            return Ok(RecoveryAction::TornTailTruncate {
                valid_offset: self.last_valid_offset,
                torn_bytes: remaining,
                reason: "Short header at tail",
            });
        }

        let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let seq = u64::from_le_bytes([
            header[4], header[5], header[6], header[7],
            header[8], header[9], header[10], header[11],
        ]);
        let len = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);

        // Check for Sentinel Seal (RFC-0314)
        if magic == WAL_SENTINEL_SEAL_MAGIC {
            if current_offset > file_size {
                return Err(WalRecoveryError::SealOffsetExceedsFileSize {
                    seal_offset: current_offset,
                    file_size,
                });
            }
            self.is_sealed = true;
            return Ok(RecoveryAction::SentinelSealEncountered {
                seal_seq: seq,
                logical_offset: current_offset,
            });
        }

        // Check for all-zero padding block (common in preallocated fallocate blocks)
        if magic == 0 && seq == 0 && len == 0 && header.iter().all(|&b| b == 0) {
            return Ok(RecoveryAction::TrailingZerosTruncate {
                valid_offset: self.last_valid_offset,
                zero_bytes: remaining,
            });
        }

        // Check for standard record magic
        if magic != WAL_RECORD_MAGIC {
            // Is this at the tail of the file? (within the final 4096-byte page)
            if remaining <= 4096 {
                return Ok(RecoveryAction::TornTailTruncate {
                    valid_offset: self.last_valid_offset,
                    torn_bytes: remaining,
                    reason: "Invalid record magic at file tail",
                });
            }
            // Mid-log damage: isolate region into quarantine without aborting reopen
            self.quarantined_regions_count = self.quarantined_regions_count.saturating_add(1);
            return Ok(RecoveryAction::QuarantinedRegion {
                start_offset: current_offset,
                quarantined_bytes: 16,
                reason: "Mid-log corrupted magic skipped to next boundary",
            });
        }

        // CRC check
        if !crc_matches {
            if remaining <= 16 + (len as u64) + 4096 {
                return Ok(RecoveryAction::TornTailTruncate {
                    valid_offset: self.last_valid_offset,
                    torn_bytes: remaining,
                    reason: "CRC mismatch in tail record",
                });
            }
            self.quarantined_regions_count = self.quarantined_regions_count.saturating_add(1);
            let skip = 16 + (len as u64);
            return Ok(RecoveryAction::QuarantinedRegion {
                start_offset: current_offset,
                quarantined_bytes: skip,
                reason: "CRC mismatch in mid-log record",
            });
        }

        // Sequence monotonicity check
        if self.total_valid_records > 0 && seq <= self.last_valid_seq {
            return Err(WalRecoveryError::NonMonotonicSequence {
                previous: self.last_valid_seq,
                found: seq,
                offset: current_offset,
            });
        }

        // Valid record accepted
        let total_frame_bytes = 16u64.saturating_add(len as u64);
        let next_offset = current_offset.saturating_add(total_frame_bytes);

        self.last_valid_seq = seq;
        self.last_valid_offset = next_offset;
        self.total_valid_records = self.total_valid_records.saturating_add(1);

        Ok(RecoveryAction::AcceptRecord {
            seq,
            payload_len: len,
            next_offset,
        })
    }

    /// Highest monotonic sequence accepted during recovery.
    #[must_use]
    pub fn last_valid_seq(&self) -> u64 {
        self.last_valid_seq
    }

    /// Offset up to which valid records have been recovered.
    #[must_use]
    pub fn last_valid_offset(&self) -> u64 {
        self.last_valid_offset
    }

    /// Total count of accepted records.
    #[must_use]
    pub fn total_valid_records(&self) -> u64 {
        self.total_valid_records
    }

    /// Number of damaged mid-log regions quarantined.
    #[must_use]
    pub fn quarantined_regions_count(&self) -> u64 {
        self.quarantined_regions_count
    }

    /// Whether a Sentinel Seal (RFC-0314) was encountered.
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.is_sealed
    }
}
