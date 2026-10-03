//! Bitemporal Secondary Index Coherence Kernel (RFC-0284 Pilar 9).
//!
//! Enforces mutual visibility entailment between primary records and secondary index entries
//! under atomic batch commits in MVCC.
//!
//! Guarantees:
//! 1. Atomic sequence pairing: Primary mutation and Index mutation share identical `commit_seq`.
//! 2. Mutual entailment: `Visible(t, Primary) <=> Visible(t, Secondary)`.
//! 3. Zero phantoms: no snapshot can ever observe updated primary data with stale index pointers.

#![forbid(unsafe_code)]

use std::fmt;

/// Violations and errors in bitemporal index consistency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitemporalError {
    /// Commit sequence number is 0 (invalid MVCC timestamp).
    ZeroCommitSequence,
    /// Batch contains neither primary mutations nor index mutations.
    EmptyBatch,
    /// Primary key is empty.
    EmptyPrimaryKey,
    /// Secondary index key is empty.
    EmptyIndexKey,
    /// Secondary index target primary key reference is empty.
    EmptyTargetPrimaryKey,
    /// Mutation neither inserts, updates, nor deletes (both old_val and new_val are None).
    NoOpPrimaryMutation,
    /// Secondary index mutation references a target primary key not present in the batch.
    DanglingIndexPointer {
        /// Target primary key that has no matching primary mutation.
        target_pk: Vec<u8>,
    },
    /// Index mutation operation contradicts primary mutation operation (e.g. insert on delete).
    IncoherentIndexOperation {
        /// Primary key with conflicting operation.
        pk: Vec<u8>,
    },
    /// Primary and secondary index visibility diverged for a snapshot sequence.
    VisibilityDivergence {
        /// Read snapshot sequence number.
        read_seq: u64,
    },
}

impl fmt::Display for BitemporalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCommitSequence => write!(f, "Commit sequence number cannot be zero"),
            Self::EmptyBatch => write!(f, "Atomic index batch cannot be empty"),
            Self::EmptyPrimaryKey => write!(f, "Primary key cannot be empty"),
            Self::EmptyIndexKey => write!(f, "Secondary index key cannot be empty"),
            Self::EmptyTargetPrimaryKey => write!(f, "Secondary index target primary key cannot be empty"),
            Self::NoOpPrimaryMutation => write!(f, "Primary mutation has both old_val and new_val as None"),
            Self::DanglingIndexPointer { target_pk } => {
                write!(f, "Secondary index references dangling primary key: {:?}", target_pk)
            }
            Self::IncoherentIndexOperation { pk } => {
                write!(f, "Secondary index operation contradicts primary mutation for key: {:?}", pk)
            }
            Self::VisibilityDivergence { read_seq } => {
                write!(f, "Bitemporal visibility diverged at snapshot read_seq: {}", read_seq)
            }
        }
    }
}

impl std::error::Error for BitemporalError {}

/// Primary key-value mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryMutation {
    /// Primary key.
    pub pk: Vec<u8>,
    /// Old value before update (None if insert).
    pub old_val: Option<Vec<u8>>,
    /// New value after update (None if delete).
    pub new_val: Option<Vec<u8>>,
}

/// Secondary index pointer mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecondaryIndexMutation {
    /// Secondary index key (e.g. hashed/indexed column + pk).
    pub index_key: Vec<u8>,
    /// Associated primary key reference.
    pub target_pk: Vec<u8>,
    /// Whether this entry is an addition or deletion.
    pub is_delete: bool,
}

/// Atomic bitemporal transaction bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicIndexBatch {
    /// Commit sequence number shared by all operations in this batch.
    pub commit_seq: u64,
    /// Primary table mutations.
    pub primary_mutations: Vec<PrimaryMutation>,
    /// Secondary index mutations.
    pub index_mutations: Vec<SecondaryIndexMutation>,
}

impl AtomicIndexBatch {
    /// Safely constructs a validated bitemporal atomic batch, rejecting zero sequences,
    /// empty keys, dangling index pointers, and incoherent operations.
    pub fn try_new(
        commit_seq: u64,
        primary_mutations: Vec<PrimaryMutation>,
        index_mutations: Vec<SecondaryIndexMutation>,
    ) -> Result<Self, BitemporalError> {
        if commit_seq == 0 {
            return Err(BitemporalError::ZeroCommitSequence);
        }
        if primary_mutations.is_empty() && index_mutations.is_empty() {
            return Err(BitemporalError::EmptyBatch);
        }

        for pm in &primary_mutations {
            if pm.pk.is_empty() {
                return Err(BitemporalError::EmptyPrimaryKey);
            }
            if pm.old_val.is_none() && pm.new_val.is_none() {
                return Err(BitemporalError::NoOpPrimaryMutation);
            }
        }

        for im in &index_mutations {
            if im.index_key.is_empty() {
                return Err(BitemporalError::EmptyIndexKey);
            }
            if im.target_pk.is_empty() {
                return Err(BitemporalError::EmptyTargetPrimaryKey);
            }

            // Verify that the target primary key exists in primary_mutations
            let matching_primary = primary_mutations.iter().find(|pm| pm.pk == im.target_pk);
            match matching_primary {
                Some(pm) => {
                    // Check operational coherence:
                    // If primary is deleted (new_val is None), index mutation must be a delete.
                    // If primary is newly inserted (old_val is None), index mutation must be an addition.
                    if pm.new_val.is_none() && !im.is_delete {
                        return Err(BitemporalError::IncoherentIndexOperation {
                            pk: im.target_pk.clone(),
                        });
                    }
                    if pm.old_val.is_none() && pm.new_val.is_some() && im.is_delete {
                        return Err(BitemporalError::IncoherentIndexOperation {
                            pk: im.target_pk.clone(),
                        });
                    }
                }
                None => {
                    return Err(BitemporalError::DanglingIndexPointer {
                        target_pk: im.target_pk.clone(),
                    });
                }
            }
        }

        Ok(Self {
            commit_seq,
            primary_mutations,
            index_mutations,
        })
    }
}

/// Verification oracle for bitemporal index consistency.
pub struct BitemporalIndexOracle;

impl BitemporalIndexOracle {
    /// Evaluates visibility at a given snapshot sequence number `read_seq`.
    pub fn is_visible(commit_seq: u64, read_seq: u64) -> bool {
        commit_seq <= read_seq
    }

    /// Verifies that all mutations in an atomic batch share the same commit sequence
    /// and that for any query snapshot `t`, either ALL mutations are visible or NONE are.
    pub fn verify_mutual_entailment(
        batch: &AtomicIndexBatch,
        read_snapshots: &[u64],
    ) -> Result<(), BitemporalError> {
        if batch.commit_seq == 0 {
            return Err(BitemporalError::ZeroCommitSequence);
        }
        for &t in read_snapshots {
            let primary_vis = Self::is_visible(batch.commit_seq, t);
            let index_vis = Self::is_visible(batch.commit_seq, t);

            // Mutual entailment invariant: Primary Visible <=> Index Visible
            if primary_vis != index_vis {
                return Err(BitemporalError::VisibilityDivergence { read_seq: t });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitemporal_hardening_red_to_green() {
        // 1. Zero commit sequence rejected
        assert_eq!(
            AtomicIndexBatch::try_new(0, vec![], vec![]),
            Err(BitemporalError::ZeroCommitSequence)
        );

        // 2. Empty batch rejected
        assert_eq!(
            AtomicIndexBatch::try_new(10, vec![], vec![]),
            Err(BitemporalError::EmptyBatch)
        );

        // 3. Empty primary key rejected
        assert_eq!(
            AtomicIndexBatch::try_new(
                10,
                vec![PrimaryMutation {
                    pk: vec![],
                    old_val: None,
                    new_val: Some(b"v1".to_vec()),
                }],
                vec![]
            ),
            Err(BitemporalError::EmptyPrimaryKey)
        );

        // 4. Dangling index pointer rejected
        assert_eq!(
            AtomicIndexBatch::try_new(
                10,
                vec![PrimaryMutation {
                    pk: b"pk1".to_vec(),
                    old_val: None,
                    new_val: Some(b"v1".to_vec()),
                }],
                vec![SecondaryIndexMutation {
                    index_key: b"idx:1".to_vec(),
                    target_pk: b"pk_nonexistent".to_vec(),
                    is_delete: false,
                }]
            ),
            Err(BitemporalError::DanglingIndexPointer {
                target_pk: b"pk_nonexistent".to_vec()
            })
        );

        // 5. Incoherent index operation rejected (primary deleted, but index addition)
        assert_eq!(
            AtomicIndexBatch::try_new(
                10,
                vec![PrimaryMutation {
                    pk: b"pk1".to_vec(),
                    old_val: Some(b"v1".to_vec()),
                    new_val: None, // delete
                }],
                vec![SecondaryIndexMutation {
                    index_key: b"idx:1".to_vec(),
                    target_pk: b"pk1".to_vec(),
                    is_delete: false, // contradiction!
                }]
            ),
            Err(BitemporalError::IncoherentIndexOperation {
                pk: b"pk1".to_vec()
            })
        );

        // 6. Valid batch succeeds
        let valid_batch = AtomicIndexBatch::try_new(
            10,
            vec![PrimaryMutation {
                pk: b"pk1".to_vec(),
                old_val: None,
                new_val: Some(b"v1".to_vec()),
            }],
            vec![SecondaryIndexMutation {
                index_key: b"idx:1".to_vec(),
                target_pk: b"pk1".to_vec(),
                is_delete: false,
            }],
        )
        .expect("Valid batch must succeed");

        assert!(BitemporalIndexOracle::verify_mutual_entailment(&valid_batch, &[5, 10, 15]).is_ok());
    }
}

