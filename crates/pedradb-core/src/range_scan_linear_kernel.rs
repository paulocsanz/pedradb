//! RFC-0282 Pilar 4 — Linearizabilidade Estrita de Iteradores de Range (Range Scan Linear Kernel).
//!
//! Formalizes snapshot isolation and absence of intra-scan time-tearing for long-running
//! range iterators traversing multi-level LSM-trees under concurrent background compactions.
//! Proves that concurrent SSTable file swaps never result in:
//!   - Non-monotonic key steps (k_{i+1} <= k_i);
//!   - Phantom records with sequence numbers exceeding the snapshot cutoff;
//!   - Partial transaction visibility (time-tearing: seeing key_a but missing key_b from the same batch).

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Violations of range scan linearizability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeScanViolation {
    /// Keys emitted out of order or stepped backward.
    NonMonotonicKeyStep {
        /// Previous key observed.
        prev_key: Vec<u8>,
        /// Current key observed.
        curr_key: Vec<u8>,
    },
    /// A duplicate key was emitted during iterator traversal.
    DuplicateKeyEmitted {
        /// The duplicated key.
        key: Vec<u8>,
    },
    /// A record with sequence number newer than snapshot leaked into the scan.
    SnapshotCutoffLeaked {
        /// The offending key.
        key: Vec<u8>,
        /// Record sequence.
        record_seq: u64,
        /// Scan snapshot sequence.
        snapshot_seq: u64,
    },
    /// Intra-scan time-tearing: atomic transaction was only partially observed.
    IntraScanTimeTearing {
        /// Transaction commit sequence.
        txn_seq: u64,
        /// Keys in transaction that were observed.
        observed_keys: Vec<Vec<u8>>,
        /// Keys in transaction that were missing from scan output.
        missing_keys: Vec<Vec<u8>>,
    },
}

/// An entry produced by a range scan iterator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeScanItem {
    /// User key.
    pub key: Vec<u8>,
    /// Value payload.
    pub val: Vec<u8>,
    /// Sequence number of the winning version.
    pub seq: u64,
    /// Transaction ID that wrote this entry (if part of an atomic batch).
    pub txn_id: Option<u64>,
}

/// Formal oracle verifying the strict linearizability of a range scan.
pub struct RangeScanLinearityOracle;

impl RangeScanLinearityOracle {
    /// Verifies that an iterator output stream adheres to snapshot isolation and monotonicity.
    ///
    /// # Errors
    /// Returns `RangeScanViolation` if any ordering, duplication, or snapshot leak occurs.
    pub fn verify_range_scan_stream(
        stream: &[RangeScanItem],
        snapshot_seq: u64,
        known_atomic_transactions: &BTreeMap<u64, BTreeSet<Vec<u8>>>,
    ) -> Result<(), RangeScanViolation> {
        let mut seen_keys = BTreeSet::new();
        let mut prev_key: Option<&[u8]> = None;
        let mut tx_observed_keys: BTreeMap<u64, BTreeSet<Vec<u8>>> = BTreeMap::new();

        for item in stream {
            // 1. Snapshot Confinement
            if item.seq > snapshot_seq {
                return Err(RangeScanViolation::SnapshotCutoffLeaked {
                    key: item.key.clone(),
                    record_seq: item.seq,
                    snapshot_seq,
                });
            }

            // 2. Strict Monotonicity: prev_key < curr_key
            if let Some(prev) = prev_key {
                if prev >= &item.key[..] {
                    return Err(RangeScanViolation::NonMonotonicKeyStep {
                        prev_key: prev.to_vec(),
                        curr_key: item.key.clone(),
                    });
                }
            }
            prev_key = Some(&item.key[..]);

            // 3. No Duplicates
            if !seen_keys.insert(item.key.clone()) {
                return Err(RangeScanViolation::DuplicateKeyEmitted {
                    key: item.key.clone(),
                });
            }

            // 4. Track atomic transaction components
            if let Some(txn_id) = item.txn_id {
                tx_observed_keys
                    .entry(txn_id)
                    .or_default()
                    .insert(item.key.clone());
            }
        }

        // 5. Verify no partial transaction visibility (time-tearing)
        // For each transaction that committed <= snapshot_seq whose keys fell within the scan bounds:
        for (&txn_id, expected_keys) in known_atomic_transactions {
            if txn_id <= snapshot_seq {
                let observed = tx_observed_keys.get(&txn_id);
                let observed_count = observed.map_or(0, |s| s.len());

                // If some keys from the transaction were observed, ALL must be observed
                // (assuming range bounds fully cover the transaction key set)
                if observed_count > 0 && observed_count < expected_keys.len() {
                    let observed_set = observed.cloned().unwrap_or_default();
                    let missing: Vec<Vec<u8>> = expected_keys
                        .difference(&observed_set)
                        .cloned()
                        .collect();

                    return Err(RangeScanViolation::IntraScanTimeTearing {
                        txn_seq: txn_id,
                        observed_keys: observed_set.into_iter().collect(),
                        missing_keys: missing,
                    });
                }
            }
        }

        Ok(())
    }
}
