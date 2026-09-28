//! RFC-0288: Sparse Tuple Homomorphism and Vertical Projection Soundness Kernel.
//!
//! Enforces temporal atomicity and snapshot coherence on vertical column projections.
//! Proves that sparse column slices never exhibit temporal tearing or chimeric rows
//! across separate SST files and compaction boundaries: pi_C(sigma_V(T)) == sigma_V(pi_C(T)).

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
}

impl fmt::Display for HomomorphismRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
        if tuple.fragments.is_empty() {
            if required_columns.is_empty() {
                return Ok(snapshot_seq);
            }
            return Err(HomomorphismRejection::MissingProjectedColumn {
                column_id: required_columns[0],
            });
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
            if !tuple.fragments.iter().any(|f| f.column_id == req_col) {
                return Err(HomomorphismRejection::MissingProjectedColumn {
                    column_id: req_col,
                });
            }
        }

        Ok(first_seq)
    }
}
