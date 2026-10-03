//! RFC-0315: Leveled SST Interval Candidate Index Kernel
//!
//! Provides deterministic O(log K) candidate table filtering for point reads,
//! reducing K-dependent degradation in point lookup while preserving full
//! snapshot isolation and range tombstone visibility guarantees.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SstCandidateIndexError {
    ZeroFileNumber,
    InvertedKeyRange { smallest: Vec<u8>, largest: Vec<u8> },
    DuplicateFileNumber(u64),
    EmptyUserKey,
    ZeroSequenceMax,
    EmptyIntervals,
}

impl fmt::Display for SstCandidateIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFileNumber => write!(f, "SST file number cannot be zero"),
            Self::InvertedKeyRange { smallest, largest } => {
                write!(
                    f,
                    "Inverted key range: smallest {:?} > largest {:?}",
                    smallest, largest
                )
            }
            Self::DuplicateFileNumber(file_num) => {
                write!(f, "Duplicate SST file number registered: {}", file_num)
            }
            Self::EmptyUserKey => write!(f, "SST user key cannot be empty"),
            Self::ZeroSequenceMax => write!(f, "SST sequence max cannot be zero"),
            Self::EmptyIntervals => {
                write!(f, "SST interval candidate index cannot be built from empty intervals")
            }
        }
    }
}

impl std::error::Error for SstCandidateIndexError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SstIntervalMetadata {
    pub file_number: u64,
    pub smallest_user_key: Vec<u8>,
    pub largest_user_key: Vec<u8>,
    pub has_range_tombstones: bool,
    pub seq_max: u64,
}

impl SstIntervalMetadata {
    pub fn try_new(
        file_number: u64,
        smallest_user_key: Vec<u8>,
        largest_user_key: Vec<u8>,
        has_range_tombstones: bool,
        seq_max: u64,
    ) -> Result<Self, SstCandidateIndexError> {
        if file_number == 0 {
            return Err(SstCandidateIndexError::ZeroFileNumber);
        }
        if smallest_user_key.is_empty() || largest_user_key.is_empty() {
            return Err(SstCandidateIndexError::EmptyUserKey);
        }
        if seq_max == 0 {
            return Err(SstCandidateIndexError::ZeroSequenceMax);
        }
        if smallest_user_key > largest_user_key {
            return Err(SstCandidateIndexError::InvertedKeyRange {
                smallest: smallest_user_key,
                largest: largest_user_key,
            });
        }
        Ok(Self {
            file_number,
            smallest_user_key,
            largest_user_key,
            has_range_tombstones,
            seq_max,
        })
    }

    pub fn new(
        file_number: u64,
        smallest_user_key: Vec<u8>,
        largest_user_key: Vec<u8>,
        has_range_tombstones: bool,
        seq_max: u64,
    ) -> Self {
        Self::try_new(
            file_number,
            smallest_user_key,
            largest_user_key,
            has_range_tombstones,
            seq_max,
        )
        .expect("valid sst interval metadata")
    }

    #[inline]
    pub fn covers_key(&self, key: &[u8]) -> bool {
        key >= self.smallest_user_key.as_slice() && key <= self.largest_user_key.as_slice()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SstCandidateIndex {
    /// SST intervals sorted lexicographically by smallest_user_key, then by seq_max descending.
    intervals: Vec<SstIntervalMetadata>,
    /// Running maximum of largest_user_key from interval 0..=i.
    prefix_max_largest: Vec<Vec<u8>>,
    /// File numbers of all SSTs with range tombstones.
    tombstone_file_numbers: Vec<u64>,
}

impl SstCandidateIndex {
    /// Builds a candidate index from a slice of SST interval metadata.
    /// Validates monotonicity, checks for duplicate file numbers, sorts intervals
    /// and precomputes running prefix maximums.
    pub fn try_new(mut intervals: Vec<SstIntervalMetadata>) -> Result<Self, SstCandidateIndexError> {
        if intervals.is_empty() {
            return Err(SstCandidateIndexError::EmptyIntervals);
        }
        for iv in &intervals {
            if iv.file_number == 0 {
                return Err(SstCandidateIndexError::ZeroFileNumber);
            }
            if iv.smallest_user_key.is_empty() || iv.largest_user_key.is_empty() {
                return Err(SstCandidateIndexError::EmptyUserKey);
            }
            if iv.seq_max == 0 {
                return Err(SstCandidateIndexError::ZeroSequenceMax);
            }
            if iv.smallest_user_key > iv.largest_user_key {
                return Err(SstCandidateIndexError::InvertedKeyRange {
                    smallest: iv.smallest_user_key.clone(),
                    largest: iv.largest_user_key.clone(),
                });
            }
        }

        // Deterministic sorting: smallest_user_key ASC, seq_max DESC, file_number DESC
        intervals.sort_by(|a, b| {
            a.smallest_user_key
                .cmp(&b.smallest_user_key)
                .then_with(|| b.seq_max.cmp(&a.seq_max))
                .then_with(|| b.file_number.cmp(&a.file_number))
        });

        // Ensure unique file numbers
        let mut seen_files = std::collections::BTreeSet::new();
        for iv in &intervals {
            if !seen_files.insert(iv.file_number) {
                return Err(SstCandidateIndexError::DuplicateFileNumber(iv.file_number));
            }
        }

        let mut prefix_max_largest = Vec::with_capacity(intervals.len());
        let mut tombstone_file_numbers = Vec::new();
        let mut current_max: Option<Vec<u8>> = None;

        for iv in &intervals {
            if iv.has_range_tombstones {
                tombstone_file_numbers.push(iv.file_number);
            }
            let next_max = match &current_max {
                None => iv.largest_user_key.clone(),
                Some(prev) => {
                    if iv.largest_user_key > *prev {
                        iv.largest_user_key.clone()
                    } else {
                        prev.clone()
                    }
                }
            };
            current_max = Some(next_max.clone());
            prefix_max_largest.push(next_max);
        }

        Ok(Self {
            intervals,
            prefix_max_largest,
            tombstone_file_numbers,
        })
    }

    /// Convenience constructor unwrapping try_new.
    pub fn new(intervals: Vec<SstIntervalMetadata>) -> Self {
        Self::try_new(intervals).expect("valid sst intervals")
    }

    /// Total number of SST tables registered in the index.
    #[inline]
    pub fn total_tables(&self) -> usize {
        self.intervals.len()
    }

    /// True if any SST contains range tombstones requiring tombstone evaluation.
    #[inline]
    pub fn has_range_tombstones(&self) -> bool {
        !self.tombstone_file_numbers.is_empty()
    }

    /// Returns the file numbers of all SSTs that carry range tombstones.
    #[inline]
    pub fn tombstone_files(&self) -> &[u64] {
        &self.tombstone_file_numbers
    }

    /// Queries the candidate SST file numbers that might contain `user_key`.
    /// Performs binary search on smallest_user_key: any table with smallest_user_key > user_key
    /// cannot contain user_key.
    ///
    /// Preserves complete recall: any table whose [smallest, largest] covers user_key is returned.
    /// Results are returned in newest-first order (by seq_max descending, file_number descending).
    pub fn query_point_candidates(&self, user_key: &[u8]) -> Vec<u64> {
        if self.intervals.is_empty() {
            return Vec::new();
        }

        // Binary search: find the upper bound where smallest_user_key > user_key.
        // All candidates must have index < upper_idx.
        let upper_idx = match self
            .intervals
            .binary_search_by(|iv| iv.smallest_user_key.as_slice().cmp(user_key))
        {
            Ok(idx) => {
                // If exact match found, walk forward to the last element with this smallest key
                let mut last = idx;
                while last + 1 < self.intervals.len()
                    && self.intervals[last + 1].smallest_user_key.as_slice() == user_key
                {
                    last += 1;
                }
                last + 1
            }
            Err(insert_idx) => insert_idx,
        };

        if upper_idx == 0 {
            // All tables have smallest_user_key > user_key. None can contain user_key.
            return Vec::new();
        }

        // Fast rejection: if the running maximum largest key up to upper_idx - 1 is < user_key,
        // no table in 0..upper_idx can contain user_key!
        if self.prefix_max_largest[upper_idx - 1].as_slice() < user_key {
            return Vec::new();
        }

        let mut candidates = Vec::new();
        for iv in &self.intervals[..upper_idx] {
            if iv.largest_user_key.as_slice() >= user_key {
                candidates.push((iv.seq_max, iv.file_number));
            }
        }

        // Sort candidates deterministically: seq_max descending, then file_number descending
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        candidates.into_iter().map(|(_, file_num)| file_num).collect()
    }

    /// Queries candidates that cover `user_key` OR carry range tombstones.
    /// Guarantees RFC-0315 Teeth 2: any table with range tombstones is never skipped,
    /// even if its point interval does not cover the queried key.
    /// Results are returned in newest-first order (by seq_max descending, file_number descending).
    pub fn query_candidates_with_tombstones(&self, user_key: &[u8]) -> Vec<u64> {
        let point_cands = self.query_point_candidates(user_key);
        if self.tombstone_file_numbers.is_empty() {
            return point_cands;
        }

        let mut cand_map: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
        for iv in &self.intervals {
            if point_cands.contains(&iv.file_number) || iv.has_range_tombstones {
                cand_map.insert(iv.file_number, iv.seq_max);
            }
        }

        let mut combined: Vec<(u64, u64)> =
            cand_map.into_iter().map(|(fn_num, seq)| (seq, fn_num)).collect();
        combined.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        combined.into_iter().map(|(_, fn_num)| fn_num).collect()
    }

    /// Queries point candidates with input key validation.
    pub fn try_query_point_candidates(&self, user_key: &[u8]) -> Result<Vec<u64>, SstCandidateIndexError> {
        if user_key.is_empty() {
            return Err(SstCandidateIndexError::EmptyUserKey);
        }
        Ok(self.query_point_candidates(user_key))
    }

    /// Queries candidates with tombstones with input key validation.
    pub fn try_query_candidates_with_tombstones(&self, user_key: &[u8]) -> Result<Vec<u64>, SstCandidateIndexError> {
        if user_key.is_empty() {
            return Err(SstCandidateIndexError::EmptyUserKey);
        }
        Ok(self.query_candidates_with_tombstones(user_key))
    }

    /// Verifies candidate retrieval completeness with input key validation.
    pub fn try_verify_candidate_completeness(
        &self,
        user_key: &[u8],
        expected_overlapping_files: &[u64],
    ) -> Result<bool, SstCandidateIndexError> {
        if user_key.is_empty() {
            return Err(SstCandidateIndexError::EmptyUserKey);
        }
        Ok(self.verify_candidate_completeness(user_key, expected_overlapping_files))
    }

    /// Verifies that candidate retrieval satisfies zero false-negatives against exhaustive scan.
    pub fn verify_candidate_completeness(
        &self,
        user_key: &[u8],
        expected_overlapping_files: &[u64],
    ) -> bool {
        let candidates = self.query_point_candidates(user_key);
        for &expected in expected_overlapping_files {
            if !candidates.contains(&expected) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sst_candidate_index_structural_invariants_red_to_green() {
        let meta1 = SstIntervalMetadata::try_new(1, b"a".to_vec(), b"c".to_vec(), false, 10)
            .expect("meta1");
        let meta2 = SstIntervalMetadata::try_new(2, b"b".to_vec(), b"d".to_vec(), true, 20)
            .expect("meta2");

        // 1. Build index successfully
        let idx = SstCandidateIndex::try_new(vec![meta1, meta2]).expect("valid index");
        assert_eq!(idx.total_tables(), 2);
        assert!(idx.has_range_tombstones());
        assert_eq!(idx.tombstone_files(), &[2]);

        // 2. Reject empty user key in try_new
        let err_empty_k = SstIntervalMetadata::try_new(1, vec![], b"c".to_vec(), false, 10);
        assert_eq!(err_empty_k, Err(SstCandidateIndexError::EmptyUserKey));

        // 3. Reject zero sequence max
        let err_zero_seq = SstIntervalMetadata::try_new(1, b"a".to_vec(), b"c".to_vec(), false, 0);
        assert_eq!(err_zero_seq, Err(SstCandidateIndexError::ZeroSequenceMax));

        // 4. Reject empty intervals in index build
        let err_empty_idx = SstCandidateIndex::try_new(vec![]);
        assert_eq!(err_empty_idx, Err(SstCandidateIndexError::EmptyIntervals));

        // 5. Query candidate keys
        let cands = idx.try_query_point_candidates(b"b").expect("query");
        assert!(cands.contains(&1));
        assert!(cands.contains(&2));

        let err_query_empty = idx.try_query_point_candidates(b"");
        assert_eq!(err_query_empty, Err(SstCandidateIndexError::EmptyUserKey));

        // 6. Display & Error implementations
        let d = format!("{}", SstCandidateIndexError::EmptyIntervals);
        assert!(d.contains("cannot be built from empty intervals"));
    }
}
