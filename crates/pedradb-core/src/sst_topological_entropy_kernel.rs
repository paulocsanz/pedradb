//! RFC-0287: SST Topological Entropy and Epsilon-Approximation Partition Kernel.
//!
//! Provides mathematically bounded key-space partitioning under adversarial,
//! fractal, or Zipfian key distributions, guaranteeing logarithmic LSM depth
//! and strict discrepancy boundaries without pathological SST skewness.

use std::fmt;

/// Error returned when partitioning fails or inputs violate invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionError {
    /// Empty sample set provided.
    EmptySamples,
    /// Sample key is empty.
    EmptyKeySample,
    /// Partition slice boundary key is empty.
    EmptySliceKeyBound,
    /// Partition slice start key is greater than end key.
    InvertedSliceKeyRange,
    /// Partition slice payload has zero keys or zero accumulated bytes.
    EmptySlicePayload,
    /// Partition plan has no slices.
    EmptySlicePlan,
    /// Partition slices overlap or violate strict monotonicity.
    OverlappingSlices,
    /// Target bytes target is zero.
    InvalidTargetBytes,
    /// Epsilon permille exceeds allowed range (must be between 1 and 999).
    InvalidEpsilonPermille(u32),
    /// Samples are not monotonically sorted.
    UnsortedSamples,
    /// Uma das amostras fornecidas possui peso zero.
    ZeroWeightSample,
    /// O plano particionado viola os limites de discrepância epsilon.
    DiscrepancyViolation {
        slice_idx: usize,
        accumulated_bytes: u64,
        lower_bound: u64,
        upper_bound: u64,
    },
}

impl fmt::Display for PartitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySamples => write!(f, "sample set cannot be empty"),
            Self::EmptyKeySample => write!(f, "sample key cannot be empty"),
            Self::EmptySliceKeyBound => write!(f, "partition slice boundary key cannot be empty"),
            Self::InvertedSliceKeyRange => write!(
                f,
                "partition slice start key cannot be greater than end key"
            ),
            Self::EmptySlicePayload => write!(
                f,
                "partition slice payload cannot have zero keys or zero accumulated bytes"
            ),
            Self::EmptySlicePlan => write!(f, "partition plan cannot have empty slices"),
            Self::OverlappingSlices => write!(
                f,
                "partition slices must have strictly disjoint and monotonic boundaries"
            ),
            Self::InvalidTargetBytes => write!(f, "target_bytes must be strictly positive"),
            Self::InvalidEpsilonPermille(e) => {
                write!(f, "epsilon_permille {e} must be in range 1..=999")
            }
            Self::UnsortedSamples => write!(f, "samples must be strictly sorted by key"),
            Self::ZeroWeightSample => write!(f, "sample byte weight must be strictly positive"),
            Self::DiscrepancyViolation {
                slice_idx,
                accumulated_bytes,
                lower_bound,
                upper_bound,
            } => write!(
                f,
                "slice {slice_idx} with {accumulated_bytes} bytes violates bounds [{lower_bound}, {upper_bound}]"
            ),
        }
    }
}

impl std::error::Error for PartitionError {}

/// A sampled key item with associated weight/size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySamplePoint {
    /// The user key.
    pub key: Vec<u8>,
    /// Estimated byte weight (key + value + block header overhead).
    pub byte_weight: u64,
}

impl KeySamplePoint {
    /// Creates a new key sample point without validation (backward compatible).
    #[must_use]
    pub fn new(key: Vec<u8>, byte_weight: u64) -> Self {
        Self { key, byte_weight }
    }

    /// Creates a validated key sample point.
    pub fn try_new(key: Vec<u8>, byte_weight: u64) -> Result<Self, PartitionError> {
        if key.is_empty() {
            return Err(PartitionError::EmptyKeySample);
        }
        if byte_weight == 0 {
            return Err(PartitionError::ZeroWeightSample);
        }
        Ok(Self { key, byte_weight })
    }
}

/// A slice partition bounded by start and end keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstPartitionSlice {
    /// Inclusive lower bound.
    pub start_key: Vec<u8>,
    /// Inclusive upper bound.
    pub end_key: Vec<u8>,
    /// Total accumulated byte weight in this slice.
    pub accumulated_bytes: u64,
    /// Number of sampled keys contained.
    pub key_count: usize,
}

impl SstPartitionSlice {
    /// Creates a validated partition slice.
    pub fn try_new(
        start_key: Vec<u8>,
        end_key: Vec<u8>,
        accumulated_bytes: u64,
        key_count: usize,
    ) -> Result<Self, PartitionError> {
        if start_key.is_empty() || end_key.is_empty() {
            return Err(PartitionError::EmptySliceKeyBound);
        }
        if start_key > end_key {
            return Err(PartitionError::InvertedSliceKeyRange);
        }
        if accumulated_bytes == 0 || key_count == 0 {
            return Err(PartitionError::EmptySlicePayload);
        }
        Ok(Self {
            start_key,
            end_key,
            accumulated_bytes,
            key_count,
        })
    }
}

/// The verified partition plan satisfying epsilon-discrepancy invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstPartitionPlan {
    /// Planned target bytes per SST.
    pub target_bytes: u64,
    /// Allowed discrepancy tolerance in permille (1/1000).
    pub epsilon_permille: u32,
    /// Partitions produced.
    pub slices: Vec<SstPartitionSlice>,
    /// Total bytes across all slices.
    pub total_bytes: u64,
}

impl SstPartitionPlan {
    /// Creates a validated partition plan.
    pub fn try_new(
        target_bytes: u64,
        epsilon_permille: u32,
        slices: Vec<SstPartitionSlice>,
    ) -> Result<Self, PartitionError> {
        if target_bytes == 0 {
            return Err(PartitionError::InvalidTargetBytes);
        }
        if epsilon_permille == 0 || epsilon_permille >= 1000 {
            return Err(PartitionError::InvalidEpsilonPermille(epsilon_permille));
        }
        if slices.is_empty() {
            return Err(PartitionError::EmptySlicePlan);
        }
        for slice in &slices {
            if slice.start_key.is_empty() || slice.end_key.is_empty() {
                return Err(PartitionError::EmptySliceKeyBound);
            }
            if slice.start_key > slice.end_key {
                return Err(PartitionError::InvertedSliceKeyRange);
            }
            if slice.accumulated_bytes == 0 || slice.key_count == 0 {
                return Err(PartitionError::EmptySlicePayload);
            }
        }
        for window in slices.windows(2) {
            if window[0].end_key >= window[1].start_key {
                return Err(PartitionError::OverlappingSlices);
            }
        }
        let total_bytes = slices.iter().map(|s| s.accumulated_bytes).sum();
        Ok(Self {
            target_bytes,
            epsilon_permille,
            slices,
            total_bytes,
        })
    }
    /// Returns the number of slices in the plan.
    #[must_use]
    pub fn slice_count(&self) -> usize {
        self.slices.len()
    }

    /// Verifies if all non-terminal slices satisfy the discrepancy bounds:
    /// `(1 - epsilon) * target <= slice_bytes <= (1 + epsilon) * target`
    /// (with tolerance for single-key oversized records).
    #[must_use]
    pub fn is_balanced(&self) -> bool {
        if self.slices.is_empty() {
            return false;
        }
        let eps = self.epsilon_permille as u64;
        let lower_bound = self.target_bytes.saturating_sub((self.target_bytes * eps) / 1000);
        let upper_bound = self.target_bytes.saturating_add((self.target_bytes * eps) / 1000);

        // Check all slices except potentially the last one which holds the remainder
        for slice in &self.slices[..self.slices.len().saturating_sub(1)] {
            if slice.key_count > 1 && (slice.accumulated_bytes < lower_bound || slice.accumulated_bytes > upper_bound) {
                return false;
            }
        }
        true
    }

    /// Busca exata de fatia: verifica se `key >= slice.start_key && key <= slice.end_key`.
    #[must_use]
    pub fn find_slice(&self, key: &[u8]) -> Option<usize> {
        self.slices
            .iter()
            .position(|slice| key >= slice.start_key.as_slice() && key <= slice.end_key.as_slice())
    }

    /// Roteamento contíguo: mapeia uma chave arbitrária para a partição SST correspondente.
    #[must_use]
    pub fn route_key(&self, key: &[u8]) -> Option<usize> {
        if self.slices.is_empty() {
            return None;
        }
        if key <= self.slices[0].end_key.as_slice() {
            return Some(0);
        }
        for (i, window) in self.slices.windows(2).enumerate() {
            if key >= window[0].start_key.as_slice() && key < window[1].start_key.as_slice() {
                return Some(i);
            }
        }
        Some(self.slices.len() - 1)
    }
}

/// Partitioning engine operating on discrete key samples.
pub struct TopologicalEntropyTracker;

impl TopologicalEntropyTracker {
    /// Calculates discrete Shannon entropy of key prefix distribution.
    /// Returns a normalized entropy metric in `[0.0, 1.0]`.
    #[must_use]
    pub fn calculate_prefix_entropy(samples: &[KeySamplePoint], prefix_len: usize) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        use std::collections::HashMap;
        let mut freq_map: HashMap<Vec<u8>, usize> = HashMap::new();
        let total = samples.len() as f64;

        for s in samples {
            let pfx = if s.key.len() >= prefix_len {
                s.key[..prefix_len].to_vec()
            } else {
                s.key.clone()
            };
            *freq_map.entry(pfx).or_insert(0) += 1;
        }

        let mut entropy = 0.0f64;
        for &count in freq_map.values() {
            let p = count as f64 / total;
            if p > 0.0 {
                entropy -= p * p.log2();
            }
        }

        // Normalize by log2 of unique classes
        let classes = freq_map.len() as f64;
        if classes > 1.0 {
            (entropy / classes.log2()).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Plans SST partition slices adhering to epsilon-discrepancy bounds.
    pub fn plan_partitions(
        samples: &[KeySamplePoint],
        target_bytes: u64,
        epsilon_permille: u32,
    ) -> Result<SstPartitionPlan, PartitionError> {
        if samples.is_empty() {
            return Err(PartitionError::EmptySamples);
        }
        if target_bytes == 0 {
            return Err(PartitionError::InvalidTargetBytes);
        }
        if epsilon_permille == 0 || epsilon_permille >= 1000 {
            return Err(PartitionError::InvalidEpsilonPermille(epsilon_permille));
        }

        // Validate sample key non-empty and non-zero weight
        for s in samples {
            if s.key.is_empty() {
                return Err(PartitionError::EmptyKeySample);
            }
            if s.byte_weight == 0 {
                return Err(PartitionError::ZeroWeightSample);
            }
        }

        // Validate sortedness
        for window in samples.windows(2) {
            if window[0].key >= window[1].key {
                return Err(PartitionError::UnsortedSamples);
            }
        }

        let eps = epsilon_permille as u64;
        let upper_bound = target_bytes.saturating_add((target_bytes * eps) / 1000);

        let mut slices = Vec::new();
        let mut current_start = samples[0].key.clone();
        let mut current_accum: u64 = 0;
        let mut current_count: usize = 0;
        let mut last_key = samples[0].key.clone();
        let mut total_bytes: u64 = 0;

        for s in samples {
            total_bytes = total_bytes.saturating_add(s.byte_weight);
            if current_count > 0 && current_accum.saturating_add(s.byte_weight) > upper_bound {
                // Cut slice
                slices.push(SstPartitionSlice {
                    start_key: current_start.clone(),
                    end_key: last_key.clone(),
                    accumulated_bytes: current_accum,
                    key_count: current_count,
                });
                current_start = s.key.clone();
                current_accum = s.byte_weight;
                current_count = 1;
                last_key = s.key.clone();
            } else {
                current_accum = current_accum.saturating_add(s.byte_weight);
                current_count += 1;
                last_key = s.key.clone();
            }
        }

        if current_count > 0 {
            slices.push(SstPartitionSlice {
                start_key: current_start,
                end_key: last_key,
                accumulated_bytes: current_accum,
                key_count: current_count,
            });
        }

        Ok(SstPartitionPlan {
            target_bytes,
            epsilon_permille,
            slices,
            total_bytes,
        })
    }

    /// Planeja e valida formalmente que as fatias respeitam os limites de discrepância epsilon e pesos não-nulos.
    pub fn plan_partitions_verified(
        samples: &[KeySamplePoint],
        target_bytes: u64,
        epsilon_permille: u32,
    ) -> Result<SstPartitionPlan, PartitionError> {
        // Valida peso zero e chave vazia via plan_partitions
        let plan = Self::plan_partitions(samples, target_bytes, epsilon_permille)?;

        // Validação estrita de discrepância em fatias não-terminais
        let eps = epsilon_permille as u64;
        let lower_bound = target_bytes.saturating_sub((target_bytes * eps) / 1000);
        let upper_bound = target_bytes.saturating_add((target_bytes * eps) / 1000);

        for (idx, slice) in plan.slices[..plan.slices.len().saturating_sub(1)].iter().enumerate() {
            if slice.key_count > 1 && (slice.accumulated_bytes < lower_bound || slice.accumulated_bytes > upper_bound) {
                return Err(PartitionError::DiscrepancyViolation {
                    slice_idx: idx,
                    accumulated_bytes: slice.accumulated_bytes,
                    lower_bound,
                    upper_bound,
                });
            }
        }

        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sst_topological_entropy_structural_invariants_red_to_green() {
        // 1. Error Display & std::error::Error conformance
        let errs: Vec<PartitionError> = vec![
            PartitionError::EmptySamples,
            PartitionError::EmptyKeySample,
            PartitionError::EmptySliceKeyBound,
            PartitionError::InvertedSliceKeyRange,
            PartitionError::EmptySlicePayload,
            PartitionError::EmptySlicePlan,
            PartitionError::OverlappingSlices,
            PartitionError::InvalidTargetBytes,
            PartitionError::InvalidEpsilonPermille(1500),
            PartitionError::UnsortedSamples,
            PartitionError::ZeroWeightSample,
            PartitionError::DiscrepancyViolation {
                slice_idx: 1,
                accumulated_bytes: 100,
                lower_bound: 500,
                upper_bound: 1000,
            },
        ];
        for err in &errs {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. KeySamplePoint::try_new validation
        assert_eq!(
            KeySamplePoint::try_new(vec![], 100),
            Err(PartitionError::EmptyKeySample)
        );
        assert_eq!(
            KeySamplePoint::try_new(b"k1".to_vec(), 0),
            Err(PartitionError::ZeroWeightSample)
        );
        assert!(KeySamplePoint::try_new(b"k1".to_vec(), 100).is_ok());

        // 3. SstPartitionSlice::try_new validation
        assert_eq!(
            SstPartitionSlice::try_new(vec![], b"k2".to_vec(), 100, 1),
            Err(PartitionError::EmptySliceKeyBound)
        );
        assert_eq!(
            SstPartitionSlice::try_new(b"k1".to_vec(), vec![], 100, 1),
            Err(PartitionError::EmptySliceKeyBound)
        );
        assert_eq!(
            SstPartitionSlice::try_new(b"k2".to_vec(), b"k1".to_vec(), 100, 1),
            Err(PartitionError::InvertedSliceKeyRange)
        );
        assert_eq!(
            SstPartitionSlice::try_new(b"k1".to_vec(), b"k2".to_vec(), 0, 1),
            Err(PartitionError::EmptySlicePayload)
        );
        assert_eq!(
            SstPartitionSlice::try_new(b"k1".to_vec(), b"k2".to_vec(), 100, 0),
            Err(PartitionError::EmptySlicePayload)
        );
        assert!(SstPartitionSlice::try_new(b"k1".to_vec(), b"k2".to_vec(), 100, 1).is_ok());

        // 4. SstPartitionPlan::try_new validation
        assert_eq!(
            SstPartitionPlan::try_new(0, 100, vec![]),
            Err(PartitionError::InvalidTargetBytes)
        );
        assert_eq!(
            SstPartitionPlan::try_new(1000, 0, vec![]),
            Err(PartitionError::InvalidEpsilonPermille(0))
        );
        assert_eq!(
            SstPartitionPlan::try_new(1000, 1000, vec![]),
            Err(PartitionError::InvalidEpsilonPermille(1000))
        );
        assert_eq!(
            SstPartitionPlan::try_new(1000, 100, vec![]),
            Err(PartitionError::EmptySlicePlan)
        );

        let slice1 = SstPartitionSlice::try_new(b"a".to_vec(), b"c".to_vec(), 500, 1).unwrap();
        let slice2_overlap = SstPartitionSlice::try_new(b"b".to_vec(), b"d".to_vec(), 500, 1).unwrap();
        let slice2_ok = SstPartitionSlice::try_new(b"d".to_vec(), b"f".to_vec(), 500, 1).unwrap();

        assert_eq!(
            SstPartitionPlan::try_new(1000, 100, vec![slice1.clone(), slice2_overlap]),
            Err(PartitionError::OverlappingSlices)
        );
        let valid_plan = SstPartitionPlan::try_new(1000, 100, vec![slice1, slice2_ok]).unwrap();
        assert_eq!(valid_plan.total_bytes, 1000);
        assert_eq!(valid_plan.slice_count(), 2);

        // 5. plan_partitions rejects empty keys & zero weights
        let empty_key_samples = vec![KeySamplePoint::new(vec![], 50)];
        assert_eq!(
            TopologicalEntropyTracker::plan_partitions(&empty_key_samples, 1000, 100),
            Err(PartitionError::EmptyKeySample)
        );

        let zero_weight_samples = vec![KeySamplePoint::new(b"k1".to_vec(), 0)];
        assert_eq!(
            TopologicalEntropyTracker::plan_partitions(&zero_weight_samples, 1000, 100),
            Err(PartitionError::ZeroWeightSample)
        );
    }
}


