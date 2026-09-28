//! RFC-0288: MANIFEST VersionEdit Semiring and Idempotent Closure Kernel.
//!
//! Models VersionEdit applications as an idempotent semiring (V + E + E == V + E).
//! Guarantees convergence to an identical canonical LSM version state under arbitrary
//! repeated or replayed manifest deltas following crashes.

use std::collections::BTreeMap;

/// Metadata for an active SST file in the version set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstFileMetadata {
    /// Unique physical file number.
    pub file_number: u64,
    /// LSM level containing this file (0 = L0, 1 = L1, ...).
    pub level: usize,
    /// Total file size in bytes.
    pub file_size_bytes: u64,
    /// Lexicographically smallest user key.
    pub smallest_key: Vec<u8>,
    /// Lexicographically largest user key.
    pub largest_key: Vec<u8>,
}

/// An incremental delta record applied to the version state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionDelta {
    /// New SST files added by compaction or flush.
    pub added_files: Vec<SstFileMetadata>,
    /// SST files deleted/superseded: (level, file_number).
    pub deleted_files: Vec<(usize, u64)>,
    /// Optional updated next file number.
    pub next_file_number: Option<u64>,
    /// Optional updated last sequence number.
    pub last_sequence: Option<u64>,
}

/// The consolidated version state of an LSM engine instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionState {
    /// Files active at each level: Level -> (FileNumber -> SstFileMetadata).
    pub files: BTreeMap<usize, BTreeMap<u64, SstFileMetadata>>,
    /// Next available physical file number.
    pub next_file_number: u64,
    /// Last committed MVCC sequence number.
    pub last_sequence: u64,
}

impl VersionState {
    /// Creates a new empty version state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            next_file_number: 1,
            last_sequence: 0,
        }
    }

    /// Returns the total number of files across all levels.
    #[must_use]
    pub fn total_file_count(&self) -> usize {
        self.files.values().map(|m| m.len()).sum()
    }

    /// Checks if a given file number exists at a specific level.
    #[must_use]
    pub fn contains_file(&self, level: usize, file_number: u64) -> bool {
        self.files.get(&level).is_some_and(|m| m.contains_key(&file_number))
    }
}

impl Default for VersionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Semiring operator applying idempotent delta transformations.
pub struct VersionEditSemiring;

impl VersionEditSemiring {
    /// Applies a delta E to state V: `V' = V + E`.
    ///
    /// Semiring Invariants:
    /// 1. Idempotence: `apply(apply(V, E), E) == apply(V, E)`
    /// 2. Monotonicity: `next_file_number` and `last_sequence` never decrease.
    #[must_use]
    pub fn apply(mut state: VersionState, delta: &VersionDelta) -> VersionState {
        // 1. Process deletions
        for &(level, file_number) in &delta.deleted_files {
            if let Some(level_files) = state.files.get_mut(&level) {
                level_files.remove(&file_number);
            }
        }

        // 2. Process additions (idempotent insert: overwriting identical file metadata is safe)
        for file in &delta.added_files {
            state
                .files
                .entry(file.level)
                .or_default()
                .insert(file.file_number, file.clone());
        }

        // 3. Monotonic counter progression
        if let Some(next_fn) = delta.next_file_number {
            state.next_file_number = state.next_file_number.max(next_fn);
        }
        if let Some(last_seq) = delta.last_sequence {
            state.last_sequence = state.last_sequence.max(last_seq);
        }

        state
    }

    /// Composes two deltas into a single merged delta: `E_12 = E_1 + E_2`.
    #[must_use]
    pub fn compose_deltas(delta1: &VersionDelta, delta2: &VersionDelta) -> VersionDelta {
        let mut added = delta1.added_files.clone();
        for file in &delta2.added_files {
            if !added.iter().any(|f| f.file_number == file.file_number && f.level == file.level) {
                added.push(file.clone());
            }
        }

        let mut deleted = delta1.deleted_files.clone();
        for del in &delta2.deleted_files {
            if !deleted.contains(del) {
                deleted.push(*del);
            }
        }

        // Deletions cancel additions
        added.retain(|f| !deleted.contains(&(f.level, f.file_number)));

        let next_file_number = match (delta1.next_file_number, delta2.next_file_number) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };

        let last_sequence = match (delta1.last_sequence, delta2.last_sequence) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };

        VersionDelta {
            added_files: added,
            deleted_files: deleted,
            next_file_number,
            last_sequence,
        }
    }
}
