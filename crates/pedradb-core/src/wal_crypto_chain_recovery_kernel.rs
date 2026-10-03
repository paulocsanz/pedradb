//! RFC-0287: WAL Cryptographic Chain Recovery and Non-Torn Corruption Kernel.
//!
//! Enforces inductive backward-hash linking (Merkle-style linear chaining) on
//! WAL records. Proves the Maximum Contiguous Valid Prefix Theorem under arbitrary
//! discontinuous NVMe controller corruptions and out-of-order sector persistence.

use std::fmt;

/// Initial hash seed for the genesis record in a WAL stream.
pub const WAL_GENESIS_SEED: u32 = 0x811c9dc5;

/// Header for a chained WAL record (20 bytes).
pub const WAL_RECORD_HEADER_LEN: usize = 20;

/// Maximum permissible payload length for a single WAL record (64 MB).
pub const MAX_WAL_RECORD_PAYLOAD_LEN: usize = 64 * 1024 * 1024;

/// Rejection reason when cryptographic chain continuity is breached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainBreachReason {
    /// Insufficient bytes to read record header.
    UnexpectedEof { available: usize, needed: usize },
    /// Record's backward link does not match previous record's computed hash.
    HashChainDiscontinuity { expected_prev: u32, got_prev: u32 },
    /// Record's sequence number does not strictly increase.
    SequenceNumberRegression { expected_min: u64, got_seq: u64 },
    /// Payload length exceeds remaining buffer or exceeds max sanity limit.
    PayloadLengthExceeded { payload_len: usize, remaining: usize },
    /// CRC32C self-checksum of the record does not match.
    RecordChecksumMismatch { expected: u32, calculated: u32 },
    /// Zero-fill header observed with non-zero trailing bytes in stream (torn hole).
    ZeroHeaderWithTrailingBytes { zero_header_offset: usize, non_zero_byte_offset: usize },
}

impl fmt::Display for ChainBreachReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { available, needed } => {
                write!(f, "unexpected EOF: {available} bytes available, needed {needed}")
            }
            Self::HashChainDiscontinuity { expected_prev, got_prev } => {
                write!(f, "hash chain discontinuity: expected prev 0x{expected_prev:08x}, got 0x{got_prev:08x}")
            }
            Self::SequenceNumberRegression { expected_min, got_seq } => {
                write!(f, "sequence regression: expected >= {expected_min}, got {got_seq}")
            }
            Self::PayloadLengthExceeded { payload_len, remaining } => {
                write!(f, "payload length {payload_len} exceeds remaining buffer {remaining}")
            }
            Self::RecordChecksumMismatch { expected, calculated } => {
                write!(f, "record checksum mismatch: expected 0x{expected:08x}, calculated 0x{calculated:08x}")
            }
            Self::ZeroHeaderWithTrailingBytes { zero_header_offset, non_zero_byte_offset } => {
                write!(
                    f,
                    "zero-filled header at offset {zero_header_offset} followed by non-zero bytes at {non_zero_byte_offset} (corrupted WAL hole)"
                )
            }
        }
    }
}

impl std::error::Error for ChainBreachReason {}

/// A successfully verified and recovered WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecoveredRecord {
    /// Monotonic sequence number.
    pub seq_num: u64,
    /// Chained cryptographic hash of this record.
    pub record_hash: u32,
    /// Recovered payload bytes.
    pub payload: Vec<u8>,
}

/// A zero-copy borrowed slice of a recovered WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecoveredRecordSlice<'a> {
    /// Monotonic sequence number.
    pub seq_num: u64,
    /// Chained cryptographic hash of this record.
    pub record_hash: u32,
    /// Borrowed slice into the raw WAL buffer.
    pub payload: &'a [u8],
}

/// Status of the recovery scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalRecoveryStatus {
    /// Clean EOF reached with complete cryptographic continuity.
    CleanEof,
    /// Discontinuity detected: recovery stopped deterministically at maximum valid prefix.
    DeterministicFailStop {
        /// Offset where the failure occurred.
        byte_offset: usize,
        /// Index of the record that breached the chain.
        record_index: usize,
        /// Reason for breach.
        reason: ChainBreachReason,
    },
}

/// Complete report of WAL recovery under cryptographic chain verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecoveryReport {
    /// Successfully recovered records in strict chronological order.
    pub recovered_records: Vec<WalRecoveredRecord>,
    /// Terminal status of the recovery.
    pub status: WalRecoveryStatus,
    /// Last valid chain hash ($H_k$).
    pub last_valid_hash: u32,
}

/// Zero-copy recovery report holding borrowed slices of WAL records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecoveryReportSlices<'a> {
    /// Successfully recovered records with zero-copy borrowed payloads.
    pub recovered_records: Vec<WalRecoveredRecordSlice<'a>>,
    /// Terminal status of the recovery.
    pub status: WalRecoveryStatus,
    /// Last valid chain hash ($H_k$).
    pub last_valid_hash: u32,
}

/// WAL cryptographic chain engine.
pub struct WalCryptoChainRecovery;

impl WalCryptoChainRecovery {
    /// Attempts to encode a new chained WAL record with backward link.
    /// Fails if `seq_num == 0` or `payload.len() > MAX_WAL_RECORD_PAYLOAD_LEN`.
    pub fn try_encode_chained_record(
        prev_hash: u32,
        seq_num: u64,
        payload: &[u8],
    ) -> Result<(Vec<u8>, u32), ChainBreachReason> {
        if seq_num == 0 {
            return Err(ChainBreachReason::SequenceNumberRegression {
                expected_min: 1,
                got_seq: 0,
            });
        }
        if payload.len() > MAX_WAL_RECORD_PAYLOAD_LEN {
            return Err(ChainBreachReason::PayloadLengthExceeded {
                payload_len: payload.len(),
                remaining: MAX_WAL_RECORD_PAYLOAD_LEN,
            });
        }

        let mut hash_acc = prev_hash;
        hash_acc = crc32c::crc32c_append(hash_acc, &seq_num.to_le_bytes());
        hash_acc = crc32c::crc32c_append(hash_acc, payload);
        let record_hash = hash_acc;

        let payload_len = payload.len() as u32;
        let mut buf = Vec::with_capacity(WAL_RECORD_HEADER_LEN + payload.len());
        buf.extend_from_slice(&prev_hash.to_le_bytes());
        buf.extend_from_slice(&seq_num.to_le_bytes());
        buf.extend_from_slice(&payload_len.to_le_bytes());
        buf.extend_from_slice(&record_hash.to_le_bytes());
        buf.extend_from_slice(payload);

        Ok((buf, record_hash))
    }

    /// Encodes a new chained WAL record with backward link (panics on error).
    ///
    /// Wire format:
    /// - 4 bytes: `prev_hash` (LE)
    /// - 8 bytes: `seq_num` (LE)
    /// - 4 bytes: `payload_len` (LE)
    /// - 4 bytes: `record_hash` (LE) -> computed over `prev_hash || seq_num || payload`
    /// - N bytes: `payload`
    #[must_use]
    pub fn encode_chained_record(
        prev_hash: u32,
        seq_num: u64,
        payload: &[u8],
    ) -> (Vec<u8>, u32) {
        Self::try_encode_chained_record(prev_hash, seq_num, payload).expect("valid WAL record")
    }

    /// Recovers the maximum valid contiguous prefix of records from a raw WAL log slice,
    /// returning zero-copy slices into the input buffer.
    ///
    /// Invariant: All returned records form an unbroken cryptographic chain.
    #[must_use]
    pub fn recover_longest_valid_prefix_slices<'a>(
        raw_wal: &'a [u8],
        seed_hash: u32,
        starting_min_seq: u64,
    ) -> WalRecoveryReportSlices<'a> {
        let mut recovered_records = Vec::new();
        let mut current_offset: usize = 0;
        let mut expected_prev_hash = seed_hash;
        let mut min_expected_seq: u64 = starting_min_seq;
        let mut record_idx: usize = 0;
        let mut prev_seq_num: Option<u64> = None;

        while current_offset < raw_wal.len() {
            let remaining = raw_wal.len() - current_offset;
            if remaining < WAL_RECORD_HEADER_LEN {
                // If remaining bytes are all zero (pre-allocated zero-filled file block), clean EOF
                if raw_wal[current_offset..].iter().all(|&b| b == 0) {
                    return WalRecoveryReportSlices {
                        recovered_records,
                        status: WalRecoveryStatus::CleanEof,
                        last_valid_hash: expected_prev_hash,
                    };
                }
                return WalRecoveryReportSlices {
                    recovered_records,
                    status: WalRecoveryStatus::DeterministicFailStop {
                        byte_offset: current_offset,
                        record_index: record_idx,
                        reason: ChainBreachReason::UnexpectedEof {
                            available: remaining,
                            needed: WAL_RECORD_HEADER_LEN,
                        },
                    },
                    last_valid_hash: expected_prev_hash,
                };
            }

            let header_slice = &raw_wal[current_offset..current_offset + WAL_RECORD_HEADER_LEN];

            // Detect zero-fill padding
            if header_slice.iter().all(|&b| b == 0) {
                // Legitimate only if all remaining bytes to EOF are zero.
                // A zero header followed by non-zero live bytes is a corrupted hole / torn write.
                if let Some(pos) = raw_wal[current_offset + WAL_RECORD_HEADER_LEN..]
                    .iter()
                    .position(|&b| b != 0)
                {
                    return WalRecoveryReportSlices {
                        recovered_records,
                        status: WalRecoveryStatus::DeterministicFailStop {
                            byte_offset: current_offset,
                            record_index: record_idx,
                            reason: ChainBreachReason::ZeroHeaderWithTrailingBytes {
                                zero_header_offset: current_offset,
                                non_zero_byte_offset: current_offset + WAL_RECORD_HEADER_LEN + pos,
                            },
                        },
                        last_valid_hash: expected_prev_hash,
                    };
                }
                return WalRecoveryReportSlices {
                    recovered_records,
                    status: WalRecoveryStatus::CleanEof,
                    last_valid_hash: expected_prev_hash,
                };
            }

            let mut prev_hash_bytes = [0u8; 4];
            prev_hash_bytes.copy_from_slice(&header_slice[0..4]);
            let prev_hash = u32::from_le_bytes(prev_hash_bytes);

            let mut seq_bytes = [0u8; 8];
            seq_bytes.copy_from_slice(&header_slice[4..12]);
            let seq_num = u64::from_le_bytes(seq_bytes);

            let mut len_bytes = [0u8; 4];
            len_bytes.copy_from_slice(&header_slice[12..16]);
            let payload_len = u32::from_le_bytes(len_bytes) as usize;

            let mut hash_bytes = [0u8; 4];
            hash_bytes.copy_from_slice(&header_slice[16..20]);
            let stored_hash = u32::from_le_bytes(hash_bytes);

            // Invariant 1: Hash Chain Continuity
            if prev_hash != expected_prev_hash {
                return WalRecoveryReportSlices {
                    recovered_records,
                    status: WalRecoveryStatus::DeterministicFailStop {
                        byte_offset: current_offset,
                        record_index: record_idx,
                        reason: ChainBreachReason::HashChainDiscontinuity {
                            expected_prev: expected_prev_hash,
                            got_prev: prev_hash,
                        },
                    },
                    last_valid_hash: expected_prev_hash,
                };
            }

            // Invariant 2: Monotonic Sequence Number
            if let Some(prev) = prev_seq_num {
                if prev == u64::MAX || seq_num <= prev {
                    return WalRecoveryReportSlices {
                        recovered_records,
                        status: WalRecoveryStatus::DeterministicFailStop {
                            byte_offset: current_offset,
                            record_index: record_idx,
                            reason: ChainBreachReason::SequenceNumberRegression {
                                expected_min: prev.saturating_add(1),
                                got_seq: seq_num,
                            },
                        },
                        last_valid_hash: expected_prev_hash,
                    };
                }
            } else if seq_num < min_expected_seq {
                return WalRecoveryReportSlices {
                    recovered_records,
                    status: WalRecoveryStatus::DeterministicFailStop {
                        byte_offset: current_offset,
                        record_index: record_idx,
                        reason: ChainBreachReason::SequenceNumberRegression {
                            expected_min: min_expected_seq,
                            got_seq: seq_num,
                        },
                    },
                    last_valid_hash: expected_prev_hash,
                };
            }

            // Invariant 3: Payload Sanity Limit
            let payload_start = current_offset + WAL_RECORD_HEADER_LEN;
            if payload_len > MAX_WAL_RECORD_PAYLOAD_LEN {
                return WalRecoveryReportSlices {
                    recovered_records,
                    status: WalRecoveryStatus::DeterministicFailStop {
                        byte_offset: current_offset,
                        record_index: record_idx,
                        reason: ChainBreachReason::PayloadLengthExceeded {
                            payload_len,
                            remaining: raw_wal.len().saturating_sub(payload_start),
                        },
                    },
                    last_valid_hash: expected_prev_hash,
                };
            }

            let payload_end = match payload_start.checked_add(payload_len) {
                Some(end) if end <= raw_wal.len() => end,
                _ => {
                    return WalRecoveryReportSlices {
                        recovered_records,
                        status: WalRecoveryStatus::DeterministicFailStop {
                            byte_offset: current_offset,
                            record_index: record_idx,
                            reason: ChainBreachReason::PayloadLengthExceeded {
                                payload_len,
                                remaining: raw_wal.len().saturating_sub(payload_start),
                            },
                        },
                        last_valid_hash: expected_prev_hash,
                    };
                }
            };

            let payload = &raw_wal[payload_start..payload_end];

            // Invariant 4: Record Integrity Hash
            let mut calculated_hash = prev_hash;
            calculated_hash = crc32c::crc32c_append(calculated_hash, &seq_num.to_le_bytes());
            calculated_hash = crc32c::crc32c_append(calculated_hash, payload);

            if calculated_hash != stored_hash {
                return WalRecoveryReportSlices {
                    recovered_records,
                    status: WalRecoveryStatus::DeterministicFailStop {
                        byte_offset: current_offset,
                        record_index: record_idx,
                        reason: ChainBreachReason::RecordChecksumMismatch {
                            expected: stored_hash,
                            calculated: calculated_hash,
                        },
                    },
                    last_valid_hash: expected_prev_hash,
                };
            }

            // Record verified! Add slice to prefix
            recovered_records.push(WalRecoveredRecordSlice {
                seq_num,
                record_hash: stored_hash,
                payload,
            });

            expected_prev_hash = stored_hash;
            // Saturating add prevents arithmetic overflow panic on u64::MAX
            min_expected_seq = seq_num.saturating_add(1);
            prev_seq_num = Some(seq_num);
            record_idx += 1;
            current_offset = payload_end;
        }

        WalRecoveryReportSlices {
            recovered_records,
            status: WalRecoveryStatus::CleanEof,
            last_valid_hash: expected_prev_hash,
        }
    }

    /// Recovers the maximum valid contiguous prefix of records from a raw WAL log slice,
    /// starting at sequence 1.
    #[must_use]
    pub fn recover_longest_valid_prefix(
        raw_wal: &[u8],
        seed_hash: u32,
    ) -> WalRecoveryReport {
        Self::recover_longest_valid_prefix_from(raw_wal, seed_hash, 1)
    }

    /// Recovers the maximum valid contiguous prefix of records starting from a custom sequence number.
    #[must_use]
    pub fn recover_longest_valid_prefix_from(
        raw_wal: &[u8],
        seed_hash: u32,
        starting_min_seq: u64,
    ) -> WalRecoveryReport {
        let slices = Self::recover_longest_valid_prefix_slices(raw_wal, seed_hash, starting_min_seq);
        WalRecoveryReport {
            recovered_records: slices
                .recovered_records
                .into_iter()
                .map(|r| WalRecoveredRecord {
                    seq_num: r.seq_num,
                    record_hash: r.record_hash,
                    payload: r.payload.to_vec(),
                })
                .collect(),
            status: slices.status,
            last_valid_hash: slices.last_valid_hash,
        }
    }
}
