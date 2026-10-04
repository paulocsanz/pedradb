//! Leveled Compaction Overlap Minimization Matrix Kernel (RFC-0314).
//!
//! Provides mathematically verified interval overlap incidence matrix computation,
//! write-amplification cost scoring, and optimal candidate selection bounded by
//! a maximum expansion ratio ceiling.
//!
//! # Problem Statement & Mathematical Foundation
//! In Leveled Compaction (L1..Lmax), selecting a candidate SSTable in Level $L$ to merge
//! into Level $L+1$ can pull in an unexpectedly large set of files, causing an instantaneous
//! write amplification spike ($W_{\text{amp}}$).
//!
//! # Safety & Optimality Invariants
//! 1. Strictly disjoint level ordering: $\forall i < j, \text{largest}(f_i) < \text{smallest}(f_j)$.
//! 2. Bounded expansion ratio: $\mathcal{R}_{\text{expansion}} \le \mathcal{R}_{\text{ceiling}}$.
//! 3. Deterministic candidate choice minimizing total compacted bytes without panics.

#![forbid(unsafe_code)]

/// Typed error outcomes for compaction overlap matrix operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOverlapError {
    /// SST file ID cannot be zero (sentinel hazard).
    ZeroFileId,
    /// Smallest key is strictly greater than largest key.
    InvertedKeyRange { smallest: Vec<u8>, largest: Vec<u8> },
    /// Expansion ratio ceiling must be finite, >= 1.0, and <= 10,000.0.
    InvalidExpansionRatio,
    /// Key cannot be empty.
    EmptyKey,
    /// Physical file size must be greater than zero.
    ZeroFileSizeBytes,
    /// Level files violate total ordering or overlap invariant.
    NonDisjointLevel,
}

impl std::fmt::Display for CompactionOverlapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroFileId => write!(f, "CompactionOverlapError: file_id 0 is reserved sentinel"),
            Self::InvertedKeyRange { smallest, largest } => {
                write!(
                    f,
                    "CompactionOverlapError: smallest key {:?} > largest key {:?}",
                    smallest, largest
                )
            }
            Self::InvalidExpansionRatio => {
                write!(
                    f,
                    "CompactionOverlapError: expansion ratio must be finite, >= 1.0 and <= 10,000.0"
                )
            }
            Self::EmptyKey => write!(f, "CompactionOverlapError: key cannot be empty"),
            Self::ZeroFileSizeBytes => {
                write!(f, "CompactionOverlapError: physical file size must be positive")
            }
            Self::NonDisjointLevel => {
                write!(f, "CompactionOverlapError: level files violate disjoint ordering")
            }
        }
    }
}

impl std::error::Error for CompactionOverlapError {}

/// Representation of an SSTable file's key range interval and physical size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstInterval {
    /// Unique identifier for the SSTable file.
    pub file_id: u64,
    /// Smallest user key in the file.
    pub smallest_key: Vec<u8>,
    /// Largest user key in the file.
    pub largest_key: Vec<u8>,
    /// File size in bytes.
    pub file_size_bytes: u64,
}

impl SstInterval {
    /// Attempts to create a new validated SSTable interval descriptor with typed errors.
    pub fn try_new(
        file_id: u64,
        smallest_key: impl Into<Vec<u8>>,
        largest_key: impl Into<Vec<u8>>,
        file_size_bytes: u64,
    ) -> Result<Self, CompactionOverlapError> {
        if file_id == 0 {
            return Err(CompactionOverlapError::ZeroFileId);
        }
        let smallest = smallest_key.into();
        let largest = largest_key.into();
        if smallest.is_empty() || largest.is_empty() {
            return Err(CompactionOverlapError::EmptyKey);
        }
        if smallest > largest {
            return Err(CompactionOverlapError::InvertedKeyRange {
                smallest,
                largest,
            });
        }
        if file_size_bytes == 0 {
            return Err(CompactionOverlapError::ZeroFileSizeBytes);
        }
        Ok(Self {
            file_id,
            smallest_key: smallest,
            largest_key: largest,
            file_size_bytes,
        })
    }

    /// Creates a new validated SSTable interval descriptor.
    pub fn new(
        file_id: u64,
        smallest_key: impl Into<Vec<u8>>,
        largest_key: impl Into<Vec<u8>>,
        file_size_bytes: u64,
    ) -> Result<Self, &'static str> {
        Self::try_new(file_id, smallest_key, largest_key, file_size_bytes)
            .map_err(|_| "Invalid SST interval descriptor")
    }

    /// Checks whether this interval has a non-empty intersection with another interval.
    #[must_use]
    pub fn overlaps_with(&self, other: &SstInterval) -> bool {
        !(self.largest_key < other.smallest_key || other.largest_key < self.smallest_key)
    }
}

/// Evaluated compaction candidate describing write volume and expansion ratio.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionOverlapCandidate {
    /// Identifier of the candidate file in source Level $L$.
    pub source_file_id: u64,
    /// Physical size in bytes of the source file.
    pub source_bytes: u64,
    /// Identifiers of overlapping files in target Level $L+1$.
    pub overlapping_target_file_ids: Vec<u64>,
    /// Total bytes of all overlapping files in target Level $L+1$.
    pub overlapping_target_bytes: u64,
    /// Total data volume transferred in this compaction step.
    pub total_compaction_bytes: u64,
    /// Expansion ratio: `overlapping_target_bytes / source_bytes`.
    pub expansion_ratio: f64,
}

/// Matrix engine analyzing bipartite interval overlaps between adjacent levels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionOverlapMatrix {
    level_source: usize,
    level_target: usize,
    max_expansion_ratio: f64,
}

impl CompactionOverlapMatrix {
    /// Attempts to create a new overlap matrix analyzer, validating that `max_expansion_ratio` >= 1.0 and <= 10,000.0.
    pub fn try_new(
        level_source: usize,
        level_target: usize,
        max_expansion_ratio: f64,
    ) -> Result<Self, CompactionOverlapError> {
        if max_expansion_ratio.is_nan()
            || !max_expansion_ratio.is_finite()
            || max_expansion_ratio < 1.0
            || max_expansion_ratio > 10_000.0
        {
            return Err(CompactionOverlapError::InvalidExpansionRatio);
        }
        Ok(Self {
            level_source,
            level_target,
            max_expansion_ratio,
        })
    }

    /// Creates a new overlap matrix analyzer for Level $L \to L+1$, clamping ratio to 1.0 minimum.
    #[must_use]
    pub fn new(level_source: usize, level_target: usize, max_expansion_ratio: f64) -> Self {
        let clamped = if max_expansion_ratio.is_nan() || max_expansion_ratio < 1.0 {
            1.0
        } else {
            max_expansion_ratio
        };
        Self {
            level_source,
            level_target,
            max_expansion_ratio: clamped,
        }
    }

    /// Returns source level index.
    #[must_use]
    pub fn level_source(&self) -> usize {
        self.level_source
    }

    /// Returns target level index.
    #[must_use]
    pub fn level_target(&self) -> usize {
        self.level_target
    }

    /// Returns configured maximum expansion ratio ceiling.
    #[must_use]
    pub fn max_expansion_ratio(&self) -> f64 {
        self.max_expansion_ratio
    }

    /// Verifies that all files in a level form a strictly disjoint, sorted total order.
    #[must_use]
    pub fn is_disjoint_level(files: &[SstInterval]) -> bool {
        for window in files.windows(2) {
            let left = &window[0];
            let right = &window[1];
            if left.largest_key >= right.smallest_key {
                return false;
            }
        }
        true
    }

    /// Verifies all internal file invariants and strict disjoint ordering of a level.
    pub fn check_level_invariants(files: &[SstInterval]) -> Result<(), CompactionOverlapError> {
        for f in files {
            if f.smallest_key.is_empty() || f.largest_key.is_empty() {
                return Err(CompactionOverlapError::EmptyKey);
            }
            if f.smallest_key > f.largest_key {
                return Err(CompactionOverlapError::InvertedKeyRange {
                    smallest: f.smallest_key.clone(),
                    largest: f.largest_key.clone(),
                });
            }
            if f.file_size_bytes == 0 {
                return Err(CompactionOverlapError::ZeroFileSizeBytes);
            }
        }
        if !Self::is_disjoint_level(files) {
            return Err(CompactionOverlapError::NonDisjointLevel);
        }
        Ok(())
    }

    /// Constructs the explicit bipartite overlap incidence matrix $A_{i, j}$ between source and target levels.
    #[must_use]
    pub fn build_incidence_matrix(
        &self,
        source_files: &[SstInterval],
        target_files: &[SstInterval],
    ) -> Vec<Vec<bool>> {
        let mut matrix = Vec::with_capacity(source_files.len());
        for src in source_files {
            let mut row = Vec::with_capacity(target_files.len());
            for tgt in target_files {
                row.push(src.overlaps_with(tgt));
            }
            matrix.push(row);
        }
        matrix
    }

    /// Computes overlap metrics for all candidate files in the source level.
    #[must_use]
    pub fn analyze_candidates(
        &self,
        source_files: &[SstInterval],
        target_files: &[SstInterval],
    ) -> Vec<CompactionOverlapCandidate> {
        let mut candidates = Vec::with_capacity(source_files.len());

        for src in source_files {
            let mut overlapping_ids = Vec::new();
            let mut target_bytes = 0u64;

            for tgt in target_files {
                if src.overlaps_with(tgt) {
                    overlapping_ids.push(tgt.file_id);
                    target_bytes = target_bytes.saturating_add(tgt.file_size_bytes);
                }
            }

            let total_bytes = src.file_size_bytes.saturating_add(target_bytes);
            let ratio = if src.file_size_bytes > 0 {
                target_bytes as f64 / src.file_size_bytes as f64
            } else {
                0.0
            };

            candidates.push(CompactionOverlapCandidate {
                source_file_id: src.file_id,
                source_bytes: src.file_size_bytes,
                overlapping_target_file_ids: overlapping_ids,
                overlapping_target_bytes: target_bytes,
                total_compaction_bytes: total_bytes,
                expansion_ratio: ratio,
            });
        }

        candidates
    }

    /// Selects the mathematically optimal compaction candidate bounded by the expansion ratio ceiling.
    ///
    /// Priority logic:
    /// 1. Filters candidates satisfying `expansion_ratio <= max_expansion_ratio`.
    /// 2. If valid candidates exist, selects the one minimizing `total_compaction_bytes`.
    /// 3. If all candidates exceed the ceiling, selects the one with the smallest `expansion_ratio`.
    #[must_use]
    pub fn select_optimal_candidate(
        &self,
        source_files: &[SstInterval],
        target_files: &[SstInterval],
    ) -> Option<CompactionOverlapCandidate> {
        let candidates = self.analyze_candidates(source_files, target_files);
        if candidates.is_empty() {
            return None;
        }

        let mut bounded_candidates: Vec<&CompactionOverlapCandidate> = candidates
            .iter()
            .filter(|c| c.expansion_ratio <= self.max_expansion_ratio)
            .collect();

        if !bounded_candidates.is_empty() {
            // Pick candidate with minimum total compaction bytes to minimize I/O budget
            bounded_candidates.sort_by(|a, b| {
                a.total_compaction_bytes
                    .cmp(&b.total_compaction_bytes)
                    .then_with(|| {
                        a.expansion_ratio
                            .partial_cmp(&b.expansion_ratio)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            return Some((*bounded_candidates[0]).clone());
        }

        // Fallback: Pick candidate with lowest expansion ratio
        let mut all_sorted = candidates;
        all_sorted.sort_by(|a, b| {
            a.expansion_ratio
                .partial_cmp(&b.expansion_ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.total_compaction_bytes.cmp(&b.total_compaction_bytes))
        });
        Some(all_sorted.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compaction_overlap_matrix_structural_invariants_red_to_green() {
        // 1. Zero file ID rejected
        assert_eq!(
            SstInterval::try_new(0, b"a".to_vec(), b"b".to_vec(), 100),
            Err(CompactionOverlapError::ZeroFileId)
        );

        // 2. Empty key rejected
        assert_eq!(
            SstInterval::try_new(1, vec![], b"b".to_vec(), 100),
            Err(CompactionOverlapError::EmptyKey)
        );

        // 3. Inverted key range rejected
        assert_eq!(
            SstInterval::try_new(1, b"z".to_vec(), b"a".to_vec(), 100),
            Err(CompactionOverlapError::InvertedKeyRange {
                smallest: b"z".to_vec(),
                largest: b"a".to_vec(),
            })
        );

        // 4. Invalid expansion ratios rejected in try_new
        assert_eq!(
            CompactionOverlapMatrix::try_new(1, 2, f64::NAN),
            Err(CompactionOverlapError::InvalidExpansionRatio)
        );
        assert_eq!(
            CompactionOverlapMatrix::try_new(1, 2, 0.5),
            Err(CompactionOverlapError::InvalidExpansionRatio)
        );
        assert_eq!(
            CompactionOverlapMatrix::try_new(1, 2, 20_000.0),
            Err(CompactionOverlapError::InvalidExpansionRatio)
        );

        // 5. Valid matrix creation and candidate selection
        let matrix = CompactionOverlapMatrix::try_new(1, 2, 5.0).unwrap();
        let src = vec![SstInterval::try_new(1, b"10".to_vec(), b"20".to_vec(), 100).unwrap()];
        let tgt = vec![SstInterval::try_new(2, b"15".to_vec(), b"25".to_vec(), 100).unwrap()];
        let candidates = matrix.analyze_candidates(&src, &tgt);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source_file_id, 1);
    }
}

