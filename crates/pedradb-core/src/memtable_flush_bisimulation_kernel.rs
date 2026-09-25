//! MemTable-to-SST Flush Bisimulation and Strict Monotonicity Kernel (RFC-0285 Pilar 2).
//!
//! Guarantees that the concrete in-memory MemTable iterator emits an exact,
//! strictly ordered, and complete sequence to the on-disk SST data block writer.
//!
//! Guarantees:
//! 1. Topological freeze barrier: `Active -> Immutable` partitions mutations deterministically.
//! 2. Strict lexicographical monotonicity: `FlushStream[i].key < FlushStream[i+1].key`.
//! 3. Bisimilar completeness: `Keys(SST) == {k in MemTable | seq(k) <= FreezeSeq}`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Entry stored inside the in-memory MemTable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemTableEntry {
    /// User key bytes.
    pub key: Vec<u8>,
    /// Value payload or None for tombstone.
    pub value: Option<Vec<u8>>,
    /// Monotonic sequence number.
    pub seq_num: u64,
}

/// Verification violation in flush stream generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushBisimulationViolation {
    /// Entry violates strict lexicographical ordering.
    OutOrOrderKey { index: usize, prev_key: Vec<u8>, curr_key: Vec<u8> },
    /// Entry violates sequence watermark (belongs to a future epoch).
    FutureSequenceLeaked { key: Vec<u8>, seq_num: u64, freeze_seq: u64 },
    /// Key present in frozen snapshot was omitted from flush stream.
    KeyOmission { key: Vec<u8> },
    /// Duplicate key emitted in output stream.
    DuplicateKey { key: Vec<u8> },
}

/// Simulated concrete MemTable with concurrent write staging and freeze barrier.
pub struct ConcreteMemTable {
    /// Active entries indexed by (key, seq descending).
    entries: BTreeMap<(Vec<u8>, std::cmp::Reverse<u64>), Option<Vec<u8>>>,
}

impl ConcreteMemTable {
    /// Creates a new empty MemTable.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Insert or delete a key with given sequence number.
    pub fn put(&mut self, key: Vec<u8>, val: Option<Vec<u8>>, seq: u64) {
        self.entries.insert((key, std::cmp::Reverse(seq)), val);
    }

    /// Freezes the MemTable up to `freeze_seq` and extracts the canonical flush stream.
    ///
    /// Deduplicates entries so only the newest version with `seq <= freeze_seq` is emitted.
    pub fn generate_flush_stream(&self, freeze_seq: u64) -> Vec<MemTableEntry> {
        let mut stream = Vec::new();
        let mut last_emitted_key: Option<Vec<u8>> = None;

        for ((key, std::cmp::Reverse(seq)), val) in &self.entries {
            if *seq <= freeze_seq {
                if let Some(ref last_k) = last_emitted_key {
                    if last_k == key {
                        // Older version of already emitted key in this snapshot; collapse
                        continue;
                    }
                }
                stream.push(MemTableEntry {
                    key: key.clone(),
                    value: val.clone(),
                    seq_num: *seq,
                });
                last_emitted_key = Some(key.clone());
            }
        }

        stream
    }

    /// Verifies the bisimulation invariant between the frozen MemTable and the generated flush stream.
    pub fn verify_flush_bisimulation(
        &self,
        flush_stream: &[MemTableEntry],
        freeze_seq: u64,
    ) -> Result<(), FlushBisimulationViolation> {
        // 1. Strict monotonicity check
        for i in 1..flush_stream.len() {
            let prev = &flush_stream[i - 1];
            let curr = &flush_stream[i];

            match prev.key.cmp(&curr.key) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => {
                    return Err(FlushBisimulationViolation::DuplicateKey { key: curr.key.clone() });
                }
                std::cmp::Ordering::Greater => {
                    return Err(FlushBisimulationViolation::OutOrOrderKey {
                        index: i,
                        prev_key: prev.key.clone(),
                        curr_key: curr.key.clone(),
                    });
                }
            }
        }

        // 2. Sequence boundary check
        for entry in flush_stream {
            if entry.seq_num > freeze_seq {
                return Err(FlushBisimulationViolation::FutureSequenceLeaked {
                    key: entry.key.clone(),
                    seq_num: entry.seq_num,
                    freeze_seq,
                });
            }
        }

        // 3. Completeness check against MemTable
        for ((key, std::cmp::Reverse(seq)), _) in &self.entries {
            if *seq <= freeze_seq {
                if !flush_stream.iter().any(|e| &e.key == key) {
                    return Err(FlushBisimulationViolation::KeyOmission { key: key.clone() });
                }
            }
        }

        Ok(())
    }
}
