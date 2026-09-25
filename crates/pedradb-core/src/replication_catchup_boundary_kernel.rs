//! RFC-0283 Pilar 10 — Continuidade Monotônica Snapshot-para-Log na Replicação (Replication Catch-Up Boundary Kernel).
//!
//! Formalizes the boundary transition between bulk SSTable snapshot installation
//! and live streaming WAL replication on lagging distributed replicas.
//! Proves that the transition boundary is hermetically sealed:
//!   S_first_stream == S_snapshot + 1  ⇒  Gap == ∅ ∧ Overlap == ∅.
//!
//! Guarantees that the follower's reconstructed state is strictly identical to an
//! unbroken linear log replay without dropped mutations or duplicate execution.

#![forbid(unsafe_code)]

/// Replicated mutation entry received from the leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicatedLogEntry {
    /// Global monotonic sequence number.
    pub seq: u64,
    /// Key payload.
    pub key: Vec<u8>,
    /// Value payload.
    pub value: Option<Vec<u8>>,
}

/// Violations resulting from broken snapshot-to-log replication continuity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicationBoundaryViolation {
    /// A sequence gap was detected (mutations between snapshot and log stream were dropped).
    ReplicationGapDetected {
        /// Sequence number of the snapshot cutoff.
        snapshot_cutoff_seq: u64,
        /// First sequence number in the streaming log.
        stream_first_seq: u64,
        /// Missing sequence count.
        gap_size: u64,
    },
    /// Log stream started before snapshot cutoff without idempotency protection.
    UncheckedStreamOverlap {
        /// Sequence number of the snapshot cutoff.
        snapshot_cutoff_seq: u64,
        /// First sequence number in the streaming log.
        stream_first_seq: u64,
    },
    /// Log stream sequences went backwards during replication catch-up.
    NonMonotonicStream {
        /// Previous sequence.
        prev_seq: u64,
        /// Current sequence.
        curr_seq: u64,
    },
}

/// Reconciler managing the boundary between snapshot ingestion and live streaming catch-up.
pub struct ReplicationBoundaryReconciler;

impl ReplicationBoundaryReconciler {
    /// Verifies that a streaming WAL log correctly stitches onto an installed snapshot.
    ///
    /// # Errors
    /// Returns `ReplicationBoundaryViolation` if a gap or non-monotonic sequence is found.
    pub fn verify_boundary_continuity(
        snapshot_cutoff_seq: u64,
        stream: &[ReplicatedLogEntry],
    ) -> Result<Vec<ReplicatedLogEntry>, ReplicationBoundaryViolation> {
        if stream.is_empty() {
            // Empty stream after snapshot is valid (up to date)
            return Ok(Vec::new());
        }

        let first_seq = stream[0].seq;

        // Condition 1: Check for gap
        if first_seq > snapshot_cutoff_seq + 1 {
            return Err(ReplicationBoundaryViolation::ReplicationGapDetected {
                snapshot_cutoff_seq,
                stream_first_seq: first_seq,
                gap_size: first_seq - (snapshot_cutoff_seq + 1),
            });
        }

        let mut filtered_entries = Vec::new();
        let mut last_seq = snapshot_cutoff_seq;

        for entry in stream {
            if entry.seq <= snapshot_cutoff_seq {
                // Idempotently skip entries already incorporated into the physical snapshot
                continue;
            }

            // Verify strict monotonic progression: entry.seq == last_seq + 1
            if entry.seq <= last_seq {
                return Err(ReplicationBoundaryViolation::NonMonotonicStream {
                    prev_seq: last_seq,
                    curr_seq: entry.seq,
                });
            }

            if entry.seq > last_seq + 1 {
                return Err(ReplicationBoundaryViolation::ReplicationGapDetected {
                    snapshot_cutoff_seq: last_seq,
                    stream_first_seq: entry.seq,
                    gap_size: entry.seq - (last_seq + 1),
                });
            }

            last_seq = entry.seq;
            filtered_entries.push(entry.clone());
        }

        Ok(filtered_entries)
    }
}
