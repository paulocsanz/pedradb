//! RFC-0288: Sparse Tuple Homomorphism and Vertical Projection Soundness Kernel.
//!
//! Enforces temporal atomicity and snapshot coherence on vertical column projections.
//! Proves that sparse column slices never exhibit temporal tearing or chimeric rows
//! across separate SST files and compaction boundaries: pi_C(sigma_V(T)) == sigma_V(pi_C(T)).

#![forbid(unsafe_code)]

use std::fmt;

/// A single projected column cell with its originating sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnFragment {
    /// Logical column identifier.
    pub column_id: u32,
    /// MVCC Sequence number at which this column value was committed.
    pub seq_num: u64,
    /// Column value bytes.
    pub value: Vec<u8>,
}

/// A collection of projected column fragments for a given row key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseTuple {
    /// Row primary key.
    pub row_key: Vec<u8>,
    /// Column fragments gathered across storage layers.
    pub fragments: Vec<ColumnFragment>,
}

/// Violations detected during sparse tuple reconstruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomomorphismRejection {
    /// Row key is empty.
    EmptyRowKey,
    /// Duplicate column fragment observed within the same tuple.
    DuplicateColumnFragment {
        /// Duplicate column id.
        column_id: u32,
    },
    /// Column fragments originate from divergent sequence numbers (temporal tearing).
    TemporalTearing {
        /// Row key affected.
        row_key: Vec<u8>,
        /// Lowest sequence number observed among columns.
        min_seq: u64,
        /// Highest sequence number observed among columns.
        max_seq: u64,
    },
    /// A fragment has a sequence number newer than the query snapshot.
    SnapshotViolation {
        /// Row key affected.
        row_key: Vec<u8>,
        /// Fragment's sequence number.
        fragment_seq: u64,
        /// Query snapshot upper bound.
        snapshot_seq: u64,
    },
    /// A mandatory projected column was missing from the fragments.
    MissingProjectedColumn {
        /// Missing column id.
        column_id: u32,
    },
    /// A column fragment has sequence number 0.
    ZeroSequenceNumber {
        column_id: u32,
    },
    /// Duplicate column requested in projection list.
    DuplicateRequiredColumn {
        column_id: u32,
    },
    /// Tuple has empty fragments.
    EmptyFragments {
        row_key: Vec<u8>,
    },
}

impl fmt::Display for HomomorphismRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRowKey => {
                write!(f, "row key is empty")
            }
            Self::DuplicateColumnFragment { column_id } => {
                write!(f, "duplicate fragment for column {column_id} in tuple")
            }
            Self::TemporalTearing { row_key, min_seq, max_seq } => {
                write!(
                    f,
                    "temporal tearing on row {row_key:?}: columns span inconsistent seqs [{min_seq}, {max_seq}]"
                )
            }
            Self::SnapshotViolation { row_key, fragment_seq, snapshot_seq } => {
                write!(
                    f,
                    "snapshot violation on row {row_key:?}: fragment seq {fragment_seq} > snapshot {snapshot_seq}"
                )
            }
            Self::MissingProjectedColumn { column_id } => {
                write!(f, "missing required projected column {column_id}")
            }
            Self::ZeroSequenceNumber { column_id } => {
                write!(f, "column fragment {column_id} has invalid sequence number 0")
            }
            Self::DuplicateRequiredColumn { column_id } => {
                write!(f, "duplicate required column {column_id} in projection specification")
            }
            Self::EmptyFragments { row_key } => {
                write!(f, "sparse tuple for row {row_key:?} contains no column fragments")
            }
        }
    }
}

impl std::error::Error for HomomorphismRejection {}

/// Engine validating homomorphism and temporal invariance of sparse projections.
pub struct SparseTupleHomomorphismVerifier;

impl SparseTupleHomomorphismVerifier {
    /// Validates that all fragments in a sparse tuple share an identical, cohesive
    /// commit snapshot sequence number <= `snapshot_seq`, with all `required_columns` present.
    pub fn verify_homomorphism(
        tuple: &SparseTuple,
        snapshot_seq: u64,
        required_columns: &[u32],
    ) -> Result<u64, HomomorphismRejection> {
        if tuple.row_key.is_empty() {
            return Err(HomomorphismRejection::EmptyRowKey);
        }

        if tuple.fragments.is_empty() {
            return Err(HomomorphismRejection::EmptyFragments {
                row_key: tuple.row_key.clone(),
            });
        }

        // Validate required_columns has no duplicates
        let mut seen_req = std::collections::HashSet::with_capacity(required_columns.len());
        for &col_id in required_columns {
            if !seen_req.insert(col_id) {
                return Err(HomomorphismRejection::DuplicateRequiredColumn { column_id: col_id });
            }
        }

        // Verify no duplicate column fragments in tuple, and non-zero sequence numbers
        let mut seen_cols = std::collections::HashSet::with_capacity(tuple.fragments.len());
        for frag in &tuple.fragments {
            if frag.seq_num == 0 {
                return Err(HomomorphismRejection::ZeroSequenceNumber {
                    column_id: frag.column_id,
                });
            }
            if !seen_cols.insert(frag.column_id) {
                return Err(HomomorphismRejection::DuplicateColumnFragment {
                    column_id: frag.column_id,
                });
            }
        }

        // Verify no fragment exceeds snapshot
        for frag in &tuple.fragments {
            if frag.seq_num > snapshot_seq {
                return Err(HomomorphismRejection::SnapshotViolation {
                    row_key: tuple.row_key.clone(),
                    fragment_seq: frag.seq_num,
                    snapshot_seq,
                });
            }
        }

        // Verify temporal consistency across all fragments (all must share identical seq_num)
        let first_seq = tuple.fragments[0].seq_num;
        let mut min_seq = first_seq;
        let mut max_seq = first_seq;

        for frag in &tuple.fragments[1..] {
            if frag.seq_num < min_seq {
                min_seq = frag.seq_num;
            }
            if frag.seq_num > max_seq {
                max_seq = frag.seq_num;
            }
        }

        if min_seq != max_seq {
            return Err(HomomorphismRejection::TemporalTearing {
                row_key: tuple.row_key.clone(),
                min_seq,
                max_seq,
            });
        }

        // Verify required columns presence
        for &req_col in required_columns {
            if !seen_cols.contains(&req_col) {
                return Err(HomomorphismRejection::MissingProjectedColumn {
                    column_id: req_col,
                });
            }
        }

        Ok(first_seq)
    }

    /// Verifies homomorphism and reconstructs the projected column values in the exact
    /// order requested by `required_columns`.
    pub fn project_and_reconstruct<'a>(
        tuple: &'a SparseTuple,
        snapshot_seq: u64,
        required_columns: &[u32],
    ) -> Result<Vec<&'a [u8]>, HomomorphismRejection> {
        Self::verify_homomorphism(tuple, snapshot_seq, required_columns)?;

        let mut projected = Vec::with_capacity(required_columns.len());
        for &req_col in required_columns {
            let frag = tuple
                .fragments
                .iter()
                .find(|f| f.column_id == req_col)
                .expect("verified present in verify_homomorphism");
            projected.push(frag.value.as_slice());
        }

        Ok(projected)
    }
}
