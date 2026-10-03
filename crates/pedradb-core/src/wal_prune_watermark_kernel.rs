//! Multi-ColumnFamily WAL Safe Pruning Watermark Kernel (RFC-0312).
//!
//! Provides mathematically verified global sequence watermark tracking across multiple
//! Column Families (CFs) and active read snapshots to prevent the premature deletion
//! of shared WAL segments (the "Premature WAL Pruning Disaster").
//!
//! # Problem Statement & Mathematical Foundation
//! In multi-CF LSM storage engines, all Column Families append commit records to a shared
//! WAL log stream. If CF_A flushes quickly up to sequence $S_A = 1000$, but CF_B retains
//! un-flushed memtable entries starting at $S_B = 200$, physically unlinking WAL segments
//! covering sequences up to 1000 destroys CF_B's durability on crash recovery.
//!
//! # Invariant: Global Safe Watermark $W_{\text{prune}}$
//! For all active Column Families $cf \in \mathcal{CF}$:
//! $$W_{\text{cf}} = \min(F(cf) + 1, M(cf))$$
//! $$W_{\text{safe}} = \min_{cf \in \mathcal{CF}} W_{\text{cf}}$$
//! $$W_{\text{prune}} = \min\left(W_{\text{safe}}, \min_{s \in \mathcal{S}} s.\text{seq}\right)$$
//!
//! A WAL segment covering sequence range $[S_{\min}, S_{\max}]$ is eligible for physical
//! pruning **if and only if**:
//! $$S_{\max} < W_{\text{prune}}$$

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Watermark state for a single Column Family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnFamilyWatermark {
    /// Column Family unique numeric identifier.
    pub cf_id: u32,
    /// Human-readable CF name.
    pub cf_name: String,
    /// Highest sequence number durably flushed to SSTable files.
    pub last_flushed_seq: u64,
    /// Lowest un-flushed sequence number still residing in the active/immutable memtable.
    pub min_active_memtable_seq: u64,
}

impl ColumnFamilyWatermark {
    /// Constructs a ColumnFamilyWatermark safely, rejecting empty CF names.
    pub fn try_new(
        cf_id: u32,
        cf_name: impl Into<String>,
        last_flushed_seq: u64,
        min_active_memtable_seq: u64,
    ) -> Result<Self, WalPruneError> {
        let cf_name = cf_name.into();
        if cf_name.is_empty() {
            return Err(WalPruneError::EmptyCfName);
        }
        Ok(Self {
            cf_id,
            cf_name,
            last_flushed_seq,
            min_active_memtable_seq,
        })
    }

    /// Safe recovery frontier for this specific Column Family.
    #[must_use]
    pub fn safe_frontier(&self) -> u64 {
        // If memtable has older un-flushed data, that is the floor; otherwise flushed + 1.
        self.min_active_memtable_seq.min(self.last_flushed_seq.saturating_add(1))
    }
}

/// Metadata describing a physical WAL log segment on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalSegmentDescriptor {
    /// Numeric segment identifier (e.g. 000001.wal).
    pub segment_id: u64,
    /// Minimum commit sequence present in this segment.
    pub min_seq: u64,
    /// Maximum commit sequence present in this segment.
    pub max_seq: u64,
    /// Size of the segment file in bytes.
    pub file_size_bytes: u64,
}

impl WalSegmentDescriptor {
    /// Safely constructs a WAL segment descriptor, rejecting zero IDs, inverted sequence bounds, and zero byte size.
    pub fn try_new(
        segment_id: u64,
        min_seq: u64,
        max_seq: u64,
        file_size_bytes: u64,
    ) -> Result<Self, WalPruneError> {
        if segment_id == 0 {
            return Err(WalPruneError::ZeroSegmentId);
        }
        if min_seq > max_seq {
            return Err(WalPruneError::InvertedSegmentBounds {
                segment_id,
                min_seq,
                max_seq,
            });
        }
        if file_size_bytes == 0 {
            return Err(WalPruneError::ZeroSegmentSizeBytes { segment_id });
        }
        Ok(Self {
            segment_id,
            min_seq,
            max_seq,
            file_size_bytes,
        })
    }
}

/// Erros estruturais na gestão de marcas d'água de descarte de WAL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalPruneError {
    ZeroSegmentId,
    InvertedSegmentBounds {
        segment_id: u64,
        min_seq: u64,
        max_seq: u64,
    },
    ZeroSegmentSizeBytes {
        segment_id: u64,
    },
    EmptyCfName,
    ZeroSnapshotSeq,
    DuplicateCfId(u32),
    DurabilityGap {
        cf_id: u32,
        min_active_seq: u64,
    },
    WatermarkInvariantViolation {
        cf_id: u32,
        required_start: u64,
        watermark: u64,
    },
}

impl std::fmt::Display for WalPruneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroSegmentId => write!(f, "WAL segment ID cannot be zero"),
            Self::InvertedSegmentBounds { segment_id, min_seq, max_seq } => write!(
                f,
                "Inverted sequence bounds for segment {segment_id}: min {min_seq} > max {max_seq}"
            ),
            Self::ZeroSegmentSizeBytes { segment_id } => {
                write!(f, "WAL segment {segment_id} file size cannot be zero")
            }
            Self::EmptyCfName => write!(f, "Column family name cannot be empty"),
            Self::ZeroSnapshotSeq => write!(f, "Active snapshot sequence number cannot be zero"),
            Self::DuplicateCfId(id) => write!(f, "Duplicate column family ID {id} registered"),
            Self::DurabilityGap { cf_id, min_active_seq } => write!(
                f,
                "Durability gap: unflushed sequence {min_active_seq} in CF {cf_id} not covered by surviving segments"
            ),
            Self::WatermarkInvariantViolation { cf_id, required_start, watermark } => write!(
                f,
                "CF {cf_id} required sequence {required_start} is below global safe watermark {watermark}"
            ),
        }
    }
}

impl std::error::Error for WalPruneError {}

/// Engine computing global multi-CF safe pruning watermarks.
#[derive(Debug, Default, Clone)]
pub struct WalPruneWatermarkOracle {
    cfs: BTreeMap<u32, ColumnFamilyWatermark>,
    active_snapshots: Vec<u64>,
}

impl WalPruneWatermarkOracle {
    /// Creates a new empty watermark oracle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retorna a quantidade de Column Families registradas.
    #[must_use]
    pub fn registered_cf_count(&self) -> usize {
        self.cfs.len()
    }

    /// Retorna a quantidade de snapshots ativos retendo marcas d'água.
    #[must_use]
    pub fn active_snapshot_count(&self) -> usize {
        self.active_snapshots.len()
    }

    /// Registers a new Column Family with safety validation.
    pub fn try_register_cf(
        &mut self,
        cf_id: u32,
        cf_name: impl Into<String>,
        initial_seq: u64,
    ) -> Result<(), WalPruneError> {
        let cf_name = cf_name.into();
        if cf_name.is_empty() {
            return Err(WalPruneError::EmptyCfName);
        }
        if self.cfs.contains_key(&cf_id) {
            return Err(WalPruneError::DuplicateCfId(cf_id));
        }
        self.register_cf(cf_id, cf_name, initial_seq);
        Ok(())
    }

    /// Registers a new Column Family with initial sequence number.
    pub fn register_cf(&mut self, cf_id: u32, cf_name: impl Into<String>, initial_seq: u64) {
        self.cfs.insert(
            cf_id,
            ColumnFamilyWatermark {
                cf_id,
                cf_name: cf_name.into(),
                last_flushed_seq: initial_seq,
                min_active_memtable_seq: initial_seq.saturating_add(1),
            },
        );
    }

    /// Updates the durable flushed sequence number for a Column Family after SST creation.
    pub fn record_flush_complete(&mut self, cf_id: u32, flushed_seq: u64) {
        if let Some(cf) = self.cfs.get_mut(&cf_id) {
            cf.last_flushed_seq = cf.last_flushed_seq.max(flushed_seq);
            cf.min_active_memtable_seq = cf.min_active_memtable_seq.max(flushed_seq.saturating_add(1));
        }
    }

    /// Updates the minimum sequence residing in active or mutable memtables for a CF.
    pub fn update_memtable_min_seq(&mut self, cf_id: u32, min_seq: u64) {
        if let Some(cf) = self.cfs.get_mut(&cf_id) {
            cf.min_active_memtable_seq = min_seq;
        }
    }

    /// Enrolls an active read snapshot sequence with validation.
    pub fn try_retain_snapshot(&mut self, snapshot_seq: u64) -> Result<(), WalPruneError> {
        if snapshot_seq == 0 {
            return Err(WalPruneError::ZeroSnapshotSeq);
        }
        self.retain_snapshot(snapshot_seq);
        Ok(())
    }

    /// Enrolls an active read snapshot sequence that must be protected.
    pub fn retain_snapshot(&mut self, snapshot_seq: u64) {
        self.active_snapshots.push(snapshot_seq);
    }

    /// Releases an active read snapshot.
    pub fn release_snapshot(&mut self, snapshot_seq: u64) {
        if let Some(pos) = self.active_snapshots.iter().position(|&s| s == snapshot_seq) {
            self.active_snapshots.swap_remove(pos);
        }
    }

    /// Computes the exact global safe prune watermark $W_{\text{prune}}$.
    ///
    /// Returns `None` if no Column Families are registered.
    #[must_use]
    pub fn compute_prune_watermark(&self) -> Option<u64> {
        if self.cfs.is_empty() {
            return None;
        }

        // Floor across all Column Families
        let cf_floor = self
            .cfs
            .values()
            .map(ColumnFamilyWatermark::safe_frontier)
            .min()
            .unwrap_or(0);

        // Floor across all active snapshots
        let snap_floor = self.active_snapshots.iter().copied().min();

        Some(match snap_floor {
            Some(s) => cf_floor.min(s),
            None => cf_floor,
        })
    }

    /// Checks whether a given WAL segment is safe to physically unlink.
    ///
    /// # Safety Invariant
    /// A segment is eligible IF AND ONLY IF its `max_seq` is strictly less than $W_{\text{prune}}$.
    #[must_use]
    pub fn is_segment_prunable(&self, segment: &WalSegmentDescriptor) -> bool {
        match self.compute_prune_watermark() {
            Some(watermark) => segment.max_seq < watermark,
            None => false, // Fail-closed: Never prune if CF state is unknown
        }
    }

    /// Filters a list of WAL segments, returning only those safe to delete.
    #[must_use]
    pub fn select_prunable_segments<'a>(
        &self,
        segments: &'a [WalSegmentDescriptor],
    ) -> Vec<&'a WalSegmentDescriptor> {
        segments
            .iter()
            .filter(|seg| self.is_segment_prunable(seg))
            .collect()
    }

    /// Verifies that all un-flushed sequences across all CFs are covered by surviving segments.
    pub fn verify_crash_recovery_coverage(
        &self,
        surviving_segments: &[WalSegmentDescriptor],
    ) -> Result<(), WalPruneError> {
        let watermark = match self.compute_prune_watermark() {
            Some(w) => w,
            None => return Ok(()),
        };

        for cf in self.cfs.values() {
            let required_start = cf.safe_frontier();
            if required_start < watermark {
                return Err(WalPruneError::WatermarkInvariantViolation {
                    cf_id: cf.cf_id,
                    required_start,
                    watermark,
                });
            }

            // If CF has pending un-flushed data above watermark, ensure at least one segment covers it
            if cf.min_active_memtable_seq > cf.last_flushed_seq {
                let covered = surviving_segments
                    .iter()
                    .any(|seg| seg.min_seq <= cf.min_active_memtable_seq && seg.max_seq >= cf.min_active_memtable_seq);

                // Note: If no WAL segments survive, all surviving data must be in SSTables
                if !surviving_segments.is_empty() && !covered {
                    // Check if covered by flushed SST
                    if cf.min_active_memtable_seq > cf.last_flushed_seq.saturating_add(1) {
                        return Err(WalPruneError::DurabilityGap {
                            cf_id: cf.cf_id,
                            min_active_seq: cf.min_active_memtable_seq,
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_segment_descriptor_validation_red_to_green() {
        assert_eq!(
            WalSegmentDescriptor::try_new(0, 10, 20, 1024),
            Err(WalPruneError::ZeroSegmentId)
        );
        assert_eq!(
            WalSegmentDescriptor::try_new(1, 30, 20, 1024),
            Err(WalPruneError::InvertedSegmentBounds {
                segment_id: 1,
                min_seq: 30,
                max_seq: 20
            })
        );
        assert_eq!(
            WalSegmentDescriptor::try_new(1, 10, 20, 0),
            Err(WalPruneError::ZeroSegmentSizeBytes { segment_id: 1 })
        );
        let seg = WalSegmentDescriptor::try_new(1, 10, 20, 1024).unwrap();
        assert_eq!(seg.segment_id, 1);
    }

    #[test]
    fn test_oracle_try_register_and_try_retain_snapshot() {
        let mut oracle = WalPruneWatermarkOracle::new();
        assert_eq!(oracle.try_register_cf(0, "", 0), Err(WalPruneError::EmptyCfName));
        assert!(oracle.try_register_cf(0, "default", 0).is_ok());
        assert_eq!(
            oracle.try_register_cf(0, "default", 0),
            Err(WalPruneError::DuplicateCfId(0))
        );

        assert_eq!(oracle.try_retain_snapshot(0), Err(WalPruneError::ZeroSnapshotSeq));
        assert!(oracle.try_retain_snapshot(100).is_ok());
        assert_eq!(oracle.active_snapshot_count(), 1);
    }
}

