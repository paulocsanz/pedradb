//! RFC-0280 P0.2 — RAM Pre-Flush Barrier & DRAM Bit-Rot Protection Kernel.
//!
//! Enforces an inductive verification barrier immediately before MemTable serialization.
//! Verifies strict key comparator monotonicity and payload checksums, intercepting
//! volatile DRAM bit-rot or concurrent corruption before it can poison physical SSTable files on disk.

#![forbid(unsafe_code)]

/// An entry staged in volatile MemTable memory prior to disk flush.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedFlushEntry {
    /// User key.
    pub key: Vec<u8>,
    /// Commit sequence number.
    pub seq: u64,
    /// Value payload.
    pub value: Vec<u8>,
    /// In-memory payload checksum.
    pub in_memory_crc: u32,
}

impl StagedFlushEntry {
    /// Creates a staged entry with computed in-memory CRC.
    pub fn new(key: Vec<u8>, seq: u64, value: Vec<u8>, crc_fn: impl Fn(&[u8]) -> u32) -> Self {
        let in_memory_crc = crc_fn(&value);
        Self {
            key,
            seq,
            value,
            in_memory_crc,
        }
    }
}

/// Result of the pre-flush integrity barrier check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreFlushBarrierResult {
    /// All entries are strictly ordered and uncorrupted: safe to serialize SSTable.
    Pass,
    /// Bit-rot or corrupt comparator order detected: key order inverted at given index.
    KeyOrderInversion {
        /// Index where inversion was detected.
        index: usize,
    },
    /// Bit-rot detected in payload: in-memory checksum mismatch at given index.
    PayloadChecksumCorrupted {
        /// Index where checksum mismatch was detected.
        index: usize,
    },
}

/// Verifies the Pre-Flush Invariant:
/// 1. Monotonic Ordering: $\forall i: \text{Key}_i < \text{Key}_{i+1} \lor (\text{Key}_i == \text{Key}_{i+1} \land \text{Seq}_i > \text{Seq}_{i+1})$.
/// 2. Payload Integrity: $\forall i: \text{CRC}(\text{Value}_i) == \text{in\_memory\_crc}_i$.
pub fn verify_preflush_barrier(
    entries: &[StagedFlushEntry],
    crc_fn: impl Fn(&[u8]) -> u32,
) -> PreFlushBarrierResult {
    if entries.is_empty() {
        return PreFlushBarrierResult::Pass;
    }

    for i in 0..entries.len() {
        // 1. Verify in-memory payload integrity
        let computed_crc = crc_fn(&entries[i].value);
        if computed_crc != entries[i].in_memory_crc {
            return PreFlushBarrierResult::PayloadChecksumCorrupted { index: i };
        }

        // 2. Verify strict key comparator monotonicity against adjacent element
        if i + 1 < entries.len() {
            let curr = &entries[i];
            let next = &entries[i + 1];

            if curr.key > next.key {
                return PreFlushBarrierResult::KeyOrderInversion { index: i };
            }
            if curr.key == next.key && curr.seq <= next.seq {
                // Same key must be ordered with newest sequence number first
                return PreFlushBarrierResult::KeyOrderInversion { index: i };
            }
        }
    }

    PreFlushBarrierResult::Pass
}
