//! RFC-0316: Point-In-Time Continuous Checkpoint Barrier Kernel
//!
//! Enforces transactional consistency and crash-recovery closure for
//! live database backups and point-in-time snapshot cuts without pausing mutations.

use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointError {
    ZeroSnapshotSeq,
    EmptyManifest,
    ZeroFileNumber,
    CheckpointAlreadyPinned(u64),
    CheckpointNotPinned(u64),
    PinOverflow,
    MissingSstFile(u64),
    MissingWalSegment(u64),
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSnapshotSeq => write!(f, "Snapshot sequence cannot be zero"),
            Self::EmptyManifest => write!(f, "Checkpoint manifest must contain at least one SST or WAL file"),
            Self::ZeroFileNumber => write!(f, "File number cannot be zero"),
            Self::CheckpointAlreadyPinned(seq) => write!(f, "Checkpoint for snapshot sequence {} already pinned", seq),
            Self::CheckpointNotPinned(seq) => write!(f, "Checkpoint for snapshot sequence {} is not pinned", seq),
            Self::PinOverflow => write!(f, "Pin reference count overflow"),
            Self::MissingSstFile(file) => write!(f, "Missing required SST file {}", file),
            Self::MissingWalSegment(seg) => write!(f, "Missing required WAL segment {}", seg),
        }
    }
}

impl std::error::Error for CheckpointError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointManifest {
    pub snapshot_seq: u64,
    pub sst_files: Vec<u64>,
    pub wal_segments: Vec<u64>,
    pub created_at_secs: u64,
}

impl CheckpointManifest {
    pub fn try_new(
        snapshot_seq: u64,
        mut sst_files: Vec<u64>,
        mut wal_segments: Vec<u64>,
        created_at_secs: u64,
    ) -> Result<Self, CheckpointError> {
        if snapshot_seq == 0 {
            return Err(CheckpointError::ZeroSnapshotSeq);
        }
        if sst_files.is_empty() && wal_segments.is_empty() {
            return Err(CheckpointError::EmptyManifest);
        }
        for &f in &sst_files {
            if f == 0 {
                return Err(CheckpointError::ZeroFileNumber);
            }
        }
        for &w in &wal_segments {
            if w == 0 {
                return Err(CheckpointError::ZeroFileNumber);
            }
        }

        sst_files.sort_unstable();
        sst_files.dedup();
        wal_segments.sort_unstable();
        wal_segments.dedup();

        Ok(Self {
            snapshot_seq,
            sst_files,
            wal_segments,
            created_at_secs,
        })
    }

    pub fn new(
        snapshot_seq: u64,
        sst_files: Vec<u64>,
        wal_segments: Vec<u64>,
        created_at_secs: u64,
    ) -> Self {
        Self::try_new(snapshot_seq, sst_files, wal_segments, created_at_secs)
            .expect("valid checkpoint manifest")
    }

    #[inline]
    pub fn total_files(&self) -> usize {
        self.sst_files.len() + self.wal_segments.len()
    }

    #[inline]
    pub fn contains_sst(&self, file_num: u64) -> bool {
        self.sst_files.binary_search(&file_num).is_ok()
    }

    #[inline]
    pub fn contains_wal(&self, seg_num: u64) -> bool {
        self.wal_segments.binary_search(&seg_num).is_ok()
    }
}

/// Active reference tracking barrier preventing compactor or GC from unlinking
/// files referenced by in-flight or published checkpoints.
#[derive(Clone, Debug, Default)]
pub struct CheckpointPinTracker {
    pinned_checkpoints: HashSet<u64>,
    sst_pins: HashMap<u64, usize>,
    wal_pins: HashMap<u64, usize>,
}

impl CheckpointPinTracker {
    pub fn new() -> Self {
        Self {
            pinned_checkpoints: HashSet::new(),
            sst_pins: HashMap::new(),
            wal_pins: HashMap::new(),
        }
    }

    /// Attempts to pin all files referenced by a checkpoint manifest.
    /// Fails if the checkpoint snapshot sequence is already pinned.
    pub fn try_pin_checkpoint(&mut self, manifest: &CheckpointManifest) -> Result<(), CheckpointError> {
        if self.pinned_checkpoints.contains(&manifest.snapshot_seq) {
            return Err(CheckpointError::CheckpointAlreadyPinned(manifest.snapshot_seq));
        }

        for &sst in &manifest.sst_files {
            let cnt = self.sst_pins.entry(sst).or_insert(0);
            *cnt = cnt.checked_add(1).ok_or(CheckpointError::PinOverflow)?;
        }
        for &wal in &manifest.wal_segments {
            let cnt = self.wal_pins.entry(wal).or_insert(0);
            *cnt = cnt.checked_add(1).ok_or(CheckpointError::PinOverflow)?;
        }

        self.pinned_checkpoints.insert(manifest.snapshot_seq);
        Ok(())
    }

    /// Pins all files referenced by a checkpoint manifest (panics on error).
    pub fn pin_checkpoint(&mut self, manifest: &CheckpointManifest) {
        self.try_pin_checkpoint(manifest).expect("pin checkpoint");
    }

    /// Attempts to unpin all files referenced by a checkpoint manifest.
    /// Fails if the checkpoint snapshot sequence was never pinned, preserving
    /// the pin counts of all other checkpoints.
    pub fn try_unpin_checkpoint(&mut self, manifest: &CheckpointManifest) -> Result<(), CheckpointError> {
        if !self.pinned_checkpoints.remove(&manifest.snapshot_seq) {
            return Err(CheckpointError::CheckpointNotPinned(manifest.snapshot_seq));
        }

        for &sst in &manifest.sst_files {
            if let Some(cnt) = self.sst_pins.get_mut(&sst) {
                if *cnt <= 1 {
                    self.sst_pins.remove(&sst);
                } else {
                    *cnt -= 1;
                }
            }
        }
        for &wal in &manifest.wal_segments {
            if let Some(cnt) = self.wal_pins.get_mut(&wal) {
                if *cnt <= 1 {
                    self.wal_pins.remove(&wal);
                } else {
                    *cnt -= 1;
                }
            }
        }

        Ok(())
    }

    /// Unpins all files referenced by a checkpoint manifest upon completion or discard.
    pub fn unpin_checkpoint(&mut self, manifest: &CheckpointManifest) {
        let _ = self.try_unpin_checkpoint(manifest);
    }

    /// True if an SST file is pinned and CANNOT be unlinked by compaction.
    #[inline]
    pub fn is_sst_pinned(&self, sst_file: u64) -> bool {
        self.sst_pins.contains_key(&sst_file)
    }

    /// True if a WAL segment is pinned and CANNOT be pruned by WAL cleaner.
    #[inline]
    pub fn is_wal_pinned(&self, wal_seg: u64) -> bool {
        self.wal_pins.contains_key(&wal_seg)
    }

    /// Total number of unique pinned files.
    #[inline]
    pub fn total_pinned_files(&self) -> usize {
        self.sst_pins.len() + self.wal_pins.len()
    }

    /// Verifies that a proposed set of obsolete files does not delete any active pinned files.
    pub fn filter_safe_obsolete_ssts(&self, obsolete_ssts: &[u64]) -> Vec<u64> {
        obsolete_ssts
            .iter()
            .copied()
            .filter(|&f| !self.is_sst_pinned(f))
            .collect()
    }

    /// Verifies whether the checkpoint manifest files are fully available on disk.
    pub fn verify_manifest_closure(
        manifest: &CheckpointManifest,
        available_ssts: &[u64],
        available_wals: &[u64],
    ) -> bool {
        Self::verify_manifest_completeness(manifest, available_ssts, available_wals).is_ok()
    }

    /// Performs strict manifest closure verification returning typed error if any file is missing.
    pub fn verify_manifest_completeness(
        manifest: &CheckpointManifest,
        available_ssts: &[u64],
        available_wals: &[u64],
    ) -> Result<(), CheckpointError> {
        let sst_set: HashSet<u64> = available_ssts.iter().copied().collect();
        let wal_set: HashSet<u64> = available_wals.iter().copied().collect();

        for &sst in &manifest.sst_files {
            if !sst_set.contains(&sst) {
                return Err(CheckpointError::MissingSstFile(sst));
            }
        }
        for &wal in &manifest.wal_segments {
            if !wal_set.contains(&wal) {
                return Err(CheckpointError::MissingWalSegment(wal));
            }
        }
        Ok(())
    }
}
