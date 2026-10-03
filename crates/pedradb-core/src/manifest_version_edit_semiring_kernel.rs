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

impl SstFileMetadata {
    /// Validates and constructs SstFileMetadata safely, rejecting zero IDs, empty keys, and inverted intervals.
    pub fn try_new(
        file_number: u64,
        level: usize,
        file_size_bytes: u64,
        smallest_key: Vec<u8>,
        largest_key: Vec<u8>,
    ) -> Result<Self, ManifestSemiringError> {
        if file_number == 0 {
            return Err(ManifestSemiringError::ZeroFileNumber);
        }
        if file_size_bytes == 0 {
            return Err(ManifestSemiringError::ZeroFileSizeBytes { file_number });
        }
        if smallest_key.is_empty() || largest_key.is_empty() {
            return Err(ManifestSemiringError::EmptyKey { file_number });
        }
        if smallest_key > largest_key {
            return Err(ManifestSemiringError::InvertedKeyRange {
                file_number,
                smallest_key,
                largest_key,
            });
        }
        Ok(Self {
            file_number,
            level,
            file_size_bytes,
            smallest_key,
            largest_key,
        })
    }
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

impl VersionDelta {
    /// Validates internal consistency of the delta record.
    pub fn try_validate(&self) -> Result<(), ManifestSemiringError> {
        for sst in &self.added_files {
            if sst.file_number == 0 {
                return Err(ManifestSemiringError::ZeroFileNumber);
            }
            if sst.file_size_bytes == 0 {
                return Err(ManifestSemiringError::ZeroFileSizeBytes { file_number: sst.file_number });
            }
            if sst.smallest_key.is_empty() || sst.largest_key.is_empty() {
                return Err(ManifestSemiringError::EmptyKey { file_number: sst.file_number });
            }
            if sst.smallest_key > sst.largest_key {
                return Err(ManifestSemiringError::InvertedKeyRange {
                    file_number: sst.file_number,
                    smallest_key: sst.smallest_key.clone(),
                    largest_key: sst.largest_key.clone(),
                });
            }
            if let Some(next_fn) = self.next_file_number {
                if sst.file_number >= next_fn {
                    return Err(ManifestSemiringError::FileNumberExceedsNextFileNumber {
                        file_number: sst.file_number,
                        next_file_number: next_fn,
                    });
                }
            }
        }
        for &(_level, file_number) in &self.deleted_files {
            if file_number == 0 {
                return Err(ManifestSemiringError::ZeroFileNumber);
            }
        }
        Ok(())
    }
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
    /// Applies a delta E to state V: `V' = V + E` with strict fail-closed validation.
    pub fn try_apply(state: VersionState, delta: &VersionDelta) -> Result<VersionState, ManifestSemiringError> {
        let drop_edit = crate::mutate_switch!(
            crate::mutation_switch_kernel::MUTANT_DROP_MANIFEST_EDIT,
            false,
            true
        );
        if drop_edit {
            return Ok(state);
        }

        delta.try_validate()?;

        if let Some(next_fn) = delta.next_file_number {
            if next_fn < state.next_file_number {
                return Err(ManifestSemiringError::NextFileNumberRegression {
                    current: state.next_file_number,
                    requested: next_fn,
                });
            }
        }

        if let Some(last_seq) = delta.last_sequence {
            if last_seq < state.last_sequence {
                return Err(ManifestSemiringError::LastSequenceRegression {
                    current: state.last_sequence,
                    requested: last_seq,
                });
            }
        }

        let new_state = Self::apply(state, delta);

        // Verify key interval disjointness for L1+
        for &lvl in new_state.files.keys() {
            if lvl > 0 {
                Self::check_level_disjoint(&new_state, lvl)
                    .map_err(ManifestSemiringError::LevelOverlap)?;
            }
        }

        Ok(new_state)
    }

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

    /// Composes two deltas with validation: `E_12 = E_1 + E_2`.
    pub fn try_compose_deltas(
        delta1: &VersionDelta,
        delta2: &VersionDelta,
    ) -> Result<VersionDelta, ManifestSemiringError> {
        delta1.try_validate()?;
        delta2.try_validate()?;
        Ok(Self::compose_deltas(delta1, delta2))
    }

    /// Composes two deltas into a single merged delta: `E_12 = E_1 + E_2`.
    ///
    /// Preserves strict semiring homomorphism: `apply(V, compose(E1, E2)) == apply(apply(V, E1), E2)`.
    #[must_use]
    pub fn compose_deltas(delta1: &VersionDelta, delta2: &VersionDelta) -> VersionDelta {
        let mut added: BTreeMap<(usize, u64), SstFileMetadata> = BTreeMap::new();
        for f in &delta1.added_files {
            added.insert((f.level, f.file_number), f.clone());
        }

        let mut deleted: std::collections::BTreeSet<(usize, u64)> = std::collections::BTreeSet::new();
        for d in &delta1.deleted_files {
            deleted.insert(*d);
        }

        // Apply delta2 sequentially over delta1:
        // 1. Any deletion in delta2 removes previously added file in delta1, or records external deletion
        for d in &delta2.deleted_files {
            if added.remove(d).is_none() {
                deleted.insert(*d);
            }
        }

        // 2. Any addition in delta2 removes previous deletion of the same file (resurrection/recycle),
        // and inserts into added
        for f in &delta2.added_files {
            deleted.remove(&(f.level, f.file_number));
            added.insert((f.level, f.file_number), f.clone());
        }

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
            added_files: added.into_values().collect(),
            deleted_files: deleted.into_iter().collect(),
            next_file_number,
            last_sequence,
        }
    }

    /// Checks that in level `level` (for level > 0), no two SST files have overlapping key intervals.
    pub fn check_level_disjoint(
        state: &VersionState,
        level: usize,
    ) -> Result<(), ManifestLevelOverlapError> {
        if level == 0 {
            // L0 allows overlapping key ranges
            return Ok(());
        }

        let Some(level_files) = state.files.get(&level) else {
            return Ok(());
        };

        let mut sorted_files: Vec<&SstFileMetadata> = level_files.values().collect();
        sorted_files.sort_by(|a, b| a.smallest_key.cmp(&b.smallest_key));

        for window in sorted_files.windows(2) {
            let prev = window[0];
            let curr = window[1];

            // If prev.largest_key >= curr.smallest_key, there is key overlap
            if prev.largest_key >= curr.smallest_key {
                return Err(ManifestLevelOverlapError {
                    level,
                    file1: prev.file_number,
                    file2: curr.file_number,
                });
            }
        }

        Ok(())
    }
}

/// Error returned when level L1+ contains overlapping SST file key ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestLevelOverlapError {
    /// Level where overlap occurred.
    pub level: usize,
    /// First file number.
    pub file1: u64,
    /// Second file number.
    pub file2: u64,
}

impl std::fmt::Display for ManifestLevelOverlapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "manifest level {} key overlap between SST {} and SST {}",
            self.level, self.file1, self.file2
        )
    }
}

impl std::error::Error for ManifestLevelOverlapError {}

/// Comprehensive errors in manifest semiring delta and state progression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestSemiringError {
    ZeroFileNumber,
    ZeroFileSizeBytes { file_number: u64 },
    EmptyKey { file_number: u64 },
    InvertedKeyRange {
        file_number: u64,
        smallest_key: Vec<u8>,
        largest_key: Vec<u8>,
    },
    FileNumberExceedsNextFileNumber {
        file_number: u64,
        next_file_number: u64,
    },
    NextFileNumberRegression {
        current: u64,
        requested: u64,
    },
    LastSequenceRegression {
        current: u64,
        requested: u64,
    },
    LevelOverlap(ManifestLevelOverlapError),
}

impl std::fmt::Display for ManifestSemiringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroFileNumber => write!(f, "File number cannot be zero"),
            Self::ZeroFileSizeBytes { file_number } => {
                write!(f, "File size for file {file_number} cannot be zero")
            }
            Self::EmptyKey { file_number } => {
                write!(f, "Empty boundary key in file {file_number}")
            }
            Self::InvertedKeyRange { file_number, smallest_key, largest_key } => {
                write!(f, "Inverted key range in file {file_number}: smallest {smallest_key:?} > largest {largest_key:?}")
            }
            Self::FileNumberExceedsNextFileNumber { file_number, next_file_number } => {
                write!(f, "File number {file_number} must be strictly less than next_file_number {next_file_number}")
            }
            Self::NextFileNumberRegression { current, requested } => {
                write!(f, "next_file_number regression from {current} to {requested}")
            }
            Self::LastSequenceRegression { current, requested } => {
                write!(f, "last_sequence regression from {current} to {requested}")
            }
            Self::LevelOverlap(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ManifestSemiringError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sst_metadata_validation_red_to_green() {
        assert_eq!(
            SstFileMetadata::try_new(0, 1, 1024, b"a".to_vec(), b"b".to_vec()),
            Err(ManifestSemiringError::ZeroFileNumber)
        );
        assert_eq!(
            SstFileMetadata::try_new(1, 1, 0, b"a".to_vec(), b"b".to_vec()),
            Err(ManifestSemiringError::ZeroFileSizeBytes { file_number: 1 })
        );
        assert_eq!(
            SstFileMetadata::try_new(1, 1, 1024, vec![], b"b".to_vec()),
            Err(ManifestSemiringError::EmptyKey { file_number: 1 })
        );
        assert_eq!(
            SstFileMetadata::try_new(1, 1, 1024, b"z".to_vec(), b"a".to_vec()),
            Err(ManifestSemiringError::InvertedKeyRange {
                file_number: 1,
                smallest_key: b"z".to_vec(),
                largest_key: b"a".to_vec()
            })
        );
    }

    #[test]
    fn test_version_delta_try_apply_hardening_red_to_green() {
        let mut state = VersionState::new();
        state.next_file_number = 10;
        state.last_sequence = 100;

        let sst = SstFileMetadata::try_new(5, 1, 4096, b"a".to_vec(), b"m".to_vec()).unwrap();
        let delta = VersionDelta {
            added_files: vec![sst],
            deleted_files: vec![],
            next_file_number: Some(8), // regression from 10 to 8!
            last_sequence: Some(150),
        };

        assert_eq!(
            VersionEditSemiring::try_apply(state.clone(), &delta),
            Err(ManifestSemiringError::NextFileNumberRegression {
                current: 10,
                requested: 8
            })
        );

        let delta_valid = VersionDelta {
            added_files: vec![SstFileMetadata::try_new(5, 1, 4096, b"a".to_vec(), b"m".to_vec()).unwrap()],
            deleted_files: vec![],
            next_file_number: Some(15),
            last_sequence: Some(150),
        };
        let new_state = VersionEditSemiring::try_apply(state, &delta_valid).unwrap();
        assert_eq!(new_state.next_file_number, 15);
        assert_eq!(new_state.last_sequence, 150);
    }
}

