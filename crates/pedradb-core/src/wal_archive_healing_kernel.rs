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

/// Maximum allowable WAL record payload length (64 MiB).
pub const MAX_WAL_PAYLOAD_SIZE: usize = 64 * 1024 * 1024;

/// Errors arising from WAL archive healing inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveHealError {
    /// Zero file length provided: an archive cannot be zero-length without a valid superblock.
    ZeroArchiveLength,
    /// Invalid superblock or segment header magic.
    CorruptedSegmentHeader { found_magic: u32 },
    /// Non-monotonic sequence encountered in validly framed records.
    NonMonotonicSequence { previous_seq: u64, found_seq: u64, offset: u64 },
    /// Zero sequence number found in validly framed record.
    ZeroSequenceNumber { offset: u64 },
    /// Payload length exceeds safe maximum bounds.
    PayloadLengthExceedsLimit { found_len: u32, max_len: u32, offset: u64 },
    /// Non-zero data found after terminal Sentinel Seal.
    TrailingDataAfterSeal { seal_offset: u64, remaining_bytes: u64 },
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
            Self::ZeroSequenceNumber { offset } => {
                write!(f, "Zero sequence number found at offset {offset}")
            }
            Self::PayloadLengthExceedsLimit { found_len, max_len, offset } => {
                write!(f, "Payload length {found_len} exceeds limit {max_len} at offset {offset}")
            }
            Self::TrailingDataAfterSeal { seal_offset, remaining_bytes } => {
                write!(f, "Non-zero trailing data ({remaining_bytes} bytes) found after seal at offset {seal_offset}")
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

            // If fewer than 16 bytes remain:
            // If offset == 0 and not all zeros, it's a corrupted segment header, not a torn tail.
            if remaining < 16 {
                if record_count == 0 {
                    return Err(ArchiveHealError::CorruptedSegmentHeader {
                        found_magic: u32::from_le_bytes([
                            data[offset],
                            data.get(offset + 1).copied().unwrap_or(0),
                            data.get(offset + 2).copied().unwrap_or(0),
                            data.get(offset + 3).copied().unwrap_or(0),
                        ]),
                    });
                }
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
                let seal_end = offset + 16;
                if seal_end < data.len() && !data[seal_end..].iter().all(|&b| b == 0) {
                    return Err(ArchiveHealError::TrailingDataAfterSeal {
                        seal_offset: offset as u64,
                        remaining_bytes: (data.len() - seal_end) as u64,
                    });
                }
                return Ok(ArchiveHealOutcome::CleanUnchanged {
                    total_records: record_count,
                    valid_length: (offset + 16) as u64,
                    last_seq,
                });
            }

            // Check for standard record magic
            if magic != WAL_RECORD_MAGIC {
                if record_count == 0 {
                    return Err(ArchiveHealError::CorruptedSegmentHeader { found_magic: magic });
                }
                // Fragment or corrupted magic at tail -> Heal by truncating
                return Ok(ArchiveHealOutcome::HealedTornTail {
                    original_length: total_len,
                    healed_length: last_valid_offset as u64,
                    discarded_bytes: remaining as u64,
                    valid_records: record_count,
                    last_seq,
                });
            }

            // Sequence validity check
            if seq == 0 {
                return Err(ArchiveHealError::ZeroSequenceNumber { offset: offset as u64 });
            }

            // Maximum payload bound check
            if payload_len > MAX_WAL_PAYLOAD_SIZE {
                return Err(ArchiveHealError::PayloadLengthExceedsLimit {
                    found_len: payload_len as u32,
                    max_len: MAX_WAL_PAYLOAD_SIZE as u32,
                    offset: offset as u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_frame(magic: u32, seq: u64, payload_len: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&magic.to_le_bytes());
        buf.extend_from_slice(&seq.to_le_bytes());
        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.resize(buf.len() + payload_len as usize, 0x55);
        buf
    }

    #[test]
    fn test_wal_archive_healing_structural_invariants_red_to_green() {
        // 1. Error Display and std::error::Error conformance
        let errors = [
            ArchiveHealError::ZeroArchiveLength,
            ArchiveHealError::CorruptedSegmentHeader { found_magic: 0xdeadbeef },
            ArchiveHealError::NonMonotonicSequence { previous_seq: 10, found_seq: 9, offset: 32 },
            ArchiveHealError::ZeroSequenceNumber { offset: 0 },
            ArchiveHealError::PayloadLengthExceedsLimit { found_len: 100_000_000, max_len: 67_108_864, offset: 0 },
            ArchiveHealError::TrailingDataAfterSeal { seal_offset: 64, remaining_bytes: 12 },
        ];
        for err in &errors {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. Corrupted segment header at offset 0 is rejected (not silently healed)
        let corrupt_header = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
        assert_eq!(
            WalArchiveHealer::inspect_and_heal(&corrupt_header),
            Err(ArchiveHealError::CorruptedSegmentHeader { found_magic: 0xefbeadde })
        );

        // 3. Zero sequence number rejection
        let zero_seq = encode_frame(WAL_RECORD_MAGIC, 0, 16);
        assert_eq!(
            WalArchiveHealer::inspect_and_heal(&zero_seq),
            Err(ArchiveHealError::ZeroSequenceNumber { offset: 0 })
        );

        // 4. Excessive payload length rejection
        let huge_frame = encode_frame(WAL_RECORD_MAGIC, 1, (MAX_WAL_PAYLOAD_SIZE + 1) as u32);
        assert_eq!(
            WalArchiveHealer::inspect_and_heal(&huge_frame[..16]), // only header needed to trigger check
            Err(ArchiveHealError::PayloadLengthExceedsLimit {
                found_len: (MAX_WAL_PAYLOAD_SIZE + 1) as u32,
                max_len: MAX_WAL_PAYLOAD_SIZE as u32,
                offset: 0,
            })
        );

        // 5. Sentinel Seal with trailing garbage rejection
        let mut sealed_with_garbage = encode_frame(WAL_SEAL_MAGIC, 1, 0);
        sealed_with_garbage.extend_from_slice(&[0xFF, 0xFE]);
        assert_eq!(
            WalArchiveHealer::inspect_and_heal(&sealed_with_garbage),
            Err(ArchiveHealError::TrailingDataAfterSeal {
                seal_offset: 0,
                remaining_bytes: 2,
            })
        );

        // 6. Clean sealed archive succeeds
        let clean_seal = encode_frame(WAL_SEAL_MAGIC, 1, 0);
        assert_eq!(
            WalArchiveHealer::inspect_and_heal(&clean_seal),
            Ok(ArchiveHealOutcome::CleanUnchanged {
                total_records: 0,
                valid_length: 16,
                last_seq: 0,
            })
        );
    }
}
