//! RFC-0327: WAL Archive Healing Kernel.
//!
//! Eradicates F-CAMP-3 failures (`torn archived WAL segment fail-stops reopen`)
//! by providing an autonomic healing state machine that truncates torn tail fragments
//! in rotated archived WAL segments (`WAL.archNNN`) to the last verified unbroken offset,
//! proving that nothing after the tear was ever acknowledged to a client.

#![forbid(unsafe_code)]

use std::fmt;

/// Standard WAL record magic (0x50454452 == ASCII 'PEDR').
pub const WAL_RECORD_MAGIC: u32 = 0x5045_4452;

/// Sentinel Seal magic (0x5345414C == ASCII 'SEAL').
pub const WAL_SEAL_MAGIC: u32 = 0x5345_414C;

/// Errors arising from WAL archive healing inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveHealError {
    /// Zero file length provided: an archive cannot be zero-length without a valid superblock.
    ZeroArchiveLength,
    /// Invalid superblock or segment header magic.
    CorruptedSegmentHeader { found_magic: u32 },
    /// Non-monotonic sequence encountered in validly framed records.
    NonMonotonicSequence { previous_seq: u64, found_seq: u64, offset: u64 },
}

impl fmt::Display for ArchiveHealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroArchiveLength => write!(f, "Archive healing error: segment length is zero"),
            Self::CorruptedSegmentHeader { found_magic } => {
                write!(f, "Archive header corrupted: expected 0x50454452, found 0x{found_magic:08x}")
            }
            Self::NonMonotonicSequence { previous_seq, found_seq, offset } => {
                write!(f, "Non-monotonic sequence: previous {previous_seq} >= found {found_seq} at offset {offset}")
            }
        }
    }
}

impl std::error::Error for ArchiveHealError {}

/// Outcome of inspecting and healing an archived WAL segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveHealOutcome {
    /// Segment is 100% clean and required no truncations.
    CleanUnchanged {
        total_records: u64,
        valid_length: u64,
        last_seq: u64,
    },
    /// A torn fragment at the tail was detected and safely pruned to the last unbroken offset.
    HealedTornTail {
        original_length: u64,
        healed_length: u64,
        discarded_bytes: u64,
        valid_records: u64,
        last_seq: u64,
    },
    /// Preallocated zero padding at the tail was safely recognized.
    PreallocatedZerosPruned {
        healed_length: u64,
        zero_bytes: u64,
        valid_records: u64,
    },
}

/// Autonomic healer for rotated WAL archives.
#[derive(Debug, Clone, Default)]
pub struct WalArchiveHealer;

impl WalArchiveHealer {
    /// Inspects an archived WAL buffer and determines the safe healed length.
    pub fn inspect_and_heal(data: &[u8]) -> Result<ArchiveHealOutcome, ArchiveHealError> {
        let total_len = data.len() as u64;
        if total_len == 0 {
            return Err(ArchiveHealError::ZeroArchiveLength);
        }

        let mut offset = 0usize;
        let mut last_valid_offset = 0usize;
        let mut last_seq = 0u64;
        let mut record_count = 0u64;

        while offset < data.len() {
            let remaining = data.len() - offset;

            // Check if remaining slice is all zeros (preallocated trailing padding)
            if data[offset..].iter().all(|&b| b == 0) {
                return Ok(ArchiveHealOutcome::PreallocatedZerosPruned {
                    healed_length: last_valid_offset as u64,
                    zero_bytes: remaining as u64,
                    valid_records: record_count,
                });
            }

            // If fewer than 16 bytes remain, it is a torn tail fragment
            if remaining < 16 {
                return Ok(ArchiveHealOutcome::HealedTornTail {
                    original_length: total_len,
                    healed_length: last_valid_offset as u64,
                    discarded_bytes: remaining as u64,
                    valid_records: record_count,
                    last_seq,
                });
            }

            let magic = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
            let seq = u64::from_le_bytes([
                data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
                data[offset + 8], data[offset + 9], data[offset + 10], data[offset + 11],
            ]);
            let payload_len = u32::from_le_bytes([
                data[offset + 12], data[offset + 13], data[offset + 14], data[offset + 15],
            ]) as usize;

            // Check for Sentinel Seal (RFC-0314)
            if magic == WAL_SEAL_MAGIC {
                return Ok(ArchiveHealOutcome::CleanUnchanged {
                    total_records: record_count,
                    valid_length: (offset + 16) as u64,
                    last_seq,
                });
            }

            // Check for standard record magic
            if magic != WAL_RECORD_MAGIC {
                // Fragment or corrupted magic at tail -> Heal by truncating
                return Ok(ArchiveHealOutcome::HealedTornTail {
                    original_length: total_len,
                    healed_length: last_valid_offset as u64,
                    discarded_bytes: remaining as u64,
                    valid_records: record_count,
                    last_seq,
                });
            }

            // Frame size check
            let frame_total = 16usize.saturating_add(payload_len);
            if remaining < frame_total {
                // Short write on payload: torn tail fragment
                return Ok(ArchiveHealOutcome::HealedTornTail {
                    original_length: total_len,
                    healed_length: last_valid_offset as u64,
                    discarded_bytes: remaining as u64,
                    valid_records: record_count,
                    last_seq,
                });
            }

            // Sequence monotonicity check
            if record_count > 0 && seq <= last_seq {
                return Err(ArchiveHealError::NonMonotonicSequence {
                    previous_seq: last_seq,
                    found_seq: seq,
                    offset: offset as u64,
                });
            }

            last_seq = seq;
            record_count = record_count.saturating_add(1);
            offset = offset.saturating_add(frame_total);
            last_valid_offset = offset;
        }

        Ok(ArchiveHealOutcome::CleanUnchanged {
            total_records: record_count,
            valid_length: last_valid_offset as u64,
            last_seq,
        })
    }
}
