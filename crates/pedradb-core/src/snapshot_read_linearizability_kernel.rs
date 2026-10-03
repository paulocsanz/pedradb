//! kernel: snapshot_read_linearizability
//! Distributed snapshot read linearizability and monotonic client watermark oracle.
//!
//! Enforces monotonic snapshot acquisition across multi-partition replicas,
//! preventing causal inversions and rejecting uncommitted future snapshots.

/// Typed errors produced during distributed snapshot linearizability verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearizabilityError {
    /// Snapshot requested is zero (empty/uncommitted sequence).
    ZeroSnapshotSeq,
    /// Snapshot requested is greater than the committed consensus high-water mark.
    FutureSnapshot { snapshot_seq: u64, committed_high_water: u64 },
    /// Client read watermark attempted to regress backwards in logical time.
    WatermarkRegressed { previous: u64, attempted: u64 },
    /// Committed high-water mark attempted to regress backwards in logical time.
    HighWaterRegressed { current: u64, attempted: u64 },
    /// Snapshot is stale compared to client's causally established read watermark.
    StaleRead { snapshot_seq: u64, required_watermark: u64 },
}

impl core::fmt::Display for LinearizabilityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroSnapshotSeq => write!(f, "Snapshot sequence cannot be zero"),
            Self::FutureSnapshot { snapshot_seq, committed_high_water } => {
                write!(
                    f,
                    "Future snapshot {} rejected (committed high water is {})",
                    snapshot_seq, committed_high_water
                )
            }
            Self::WatermarkRegressed { previous, attempted } => {
                write!(
                    f,
                    "Client read watermark regression: previous {}, attempted {}",
                    previous, attempted
                )
            }
            Self::HighWaterRegressed { current, attempted } => {
                write!(
                    f,
                    "Committed high-water mark regression: current {}, attempted {}",
                    current, attempted
                )
            }
            Self::StaleRead { snapshot_seq, required_watermark } => {
                write!(
                    f,
                    "Stale read: snapshot {} < required client watermark {}",
                    snapshot_seq, required_watermark
                )
            }
        }
    }
}

impl std::error::Error for LinearizabilityError {}

/// Tracks committed consensus state and enforces linearizable read watermarks for a client session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotWatermarkTracker {
    /// The highest durably committed consensus sequence number across the cluster partition.
    pub committed_high_water: u64,
    /// Highest snapshot sequence number observed by this client session.
    pub client_read_watermark: u64,
}

impl SnapshotWatermarkTracker {
    /// Creates a new tracker initialized at `initial_committed_high_water`.
    pub fn new(initial_committed_high_water: u64) -> Self {
        Self {
            committed_high_water: initial_committed_high_water,
            client_read_watermark: 0,
        }
    }

    /// Attempts to advance the committed consensus high water mark, failing if regressed.
    pub fn try_advance_committed_high_water(&mut self, new_high_water: u64) -> Result<u64, LinearizabilityError> {
        if new_high_water < self.committed_high_water {
            return Err(LinearizabilityError::HighWaterRegressed {
                current: self.committed_high_water,
                attempted: new_high_water,
            });
        }
        self.committed_high_water = new_high_water;
        Ok(self.committed_high_water)
    }

    /// Advances the committed consensus high water mark (ignores regressions).
    pub fn advance_committed_high_water(&mut self, new_high_water: u64) {
        let _ = self.try_advance_committed_high_water(new_high_water);
    }

    /// Updates the client's established read watermark from a causal token (e.g. RPC context).
    pub fn update_client_watermark(&mut self, token: u64) -> Result<(), LinearizabilityError> {
        if token > self.committed_high_water {
            return Err(LinearizabilityError::FutureSnapshot {
                snapshot_seq: token,
                committed_high_water: self.committed_high_water,
            });
        }
        if token < self.client_read_watermark {
            return Err(LinearizabilityError::WatermarkRegressed {
                previous: self.client_read_watermark,
                attempted: token,
            });
        }
        self.client_read_watermark = token;
        Ok(())
    }

    /// Acquires a snapshot for read, verifying linearizability bounds.
    ///
    /// If `requested_seq` is `None`, acquires the latest committed high water mark.
    pub fn acquire_snapshot_for_read(&mut self, requested_seq: Option<u64>) -> Result<u64, LinearizabilityError> {
        let seq = requested_seq.unwrap_or(self.committed_high_water);

        if seq == 0 {
            return Err(LinearizabilityError::ZeroSnapshotSeq);
        }

        if seq > self.committed_high_water {
            return Err(LinearizabilityError::FutureSnapshot {
                snapshot_seq: seq,
                committed_high_water: self.committed_high_water,
            });
        }

        if seq < self.client_read_watermark {
            return Err(LinearizabilityError::StaleRead {
                snapshot_seq: seq,
                required_watermark: self.client_read_watermark,
            });
        }

        self.client_read_watermark = seq;
        debug_assert!(self.verify_internal_invariants());
        Ok(seq)
    }

    /// Verifies all internal linearizability invariants.
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        self.client_read_watermark <= self.committed_high_water
    }
}
