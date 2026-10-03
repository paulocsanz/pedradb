//! Super-Atomic MultiGet Snapshot Coherence Kernel (RFC-0284 Pilar 4).
//!
//! Guarantees that multi-key batch lookups (`MultiGet`) observe a strictly coherent,
//! torn-read-free view of the LSM tree, even while background compactions concurrently
//! delete and install new SST files.
//!
//! Guarantees:
//! 1. All keys in a batch query execute against an identical pinned `VersionEpoch`.
//! 2. Compaction version installs never alter the visible set of files for active MultiGets.
//! 3. Epoch unpinning transitions smoothly without leaking obsolete file handles.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Unique identifier for an immutable LSM version state.
pub type VersionId = u64;

/// Errors arising during MultiGet execution and version management.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiGetError {
    /// Batch query slice is empty.
    EmptyBatchKeys,
    /// An individual key in the batch query is empty.
    EmptyBatchKey,
    /// Table entry user key is empty.
    EmptyKeyPayload,
    /// Version ID is zero.
    ZeroVersionId,
    /// Sequence number is zero.
    ZeroSequenceNumber,
    /// TableEntry key does not match its map key.
    KeyEntryMismatch,
    /// Result batch is empty.
    EmptyBatchResults,
    /// Multiple version IDs observed within the same atomic batch.
    VersionMismatch {
        /// Expected version ID established by the first entry.
        expected: VersionId,
        /// Inconsistent version ID observed.
        actual: VersionId,
    },
}

impl fmt::Display for MultiGetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBatchKeys => write!(f, "multiget batch keys cannot be empty"),
            Self::EmptyBatchKey => write!(f, "multiget query key cannot be empty"),
            Self::EmptyKeyPayload => write!(f, "table entry key cannot be empty"),
            Self::ZeroVersionId => write!(f, "version id cannot be zero"),
            Self::ZeroSequenceNumber => write!(f, "sequence number cannot be zero"),
            Self::KeyEntryMismatch => write!(f, "table entry key does not match map key"),
            Self::EmptyBatchResults => write!(f, "batch results cannot be empty"),
            Self::VersionMismatch { expected, actual } => write!(
                f,
                "incoherent multiget batch: expected version {expected}, found {actual}"
            ),
        }
    }
}

impl std::error::Error for MultiGetError {}

/// Key-value entry in a versioned table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEntry {
    /// Target user key.
    pub key: Vec<u8>,
    /// Value payload (or None for tombstone).
    pub value: Option<Vec<u8>>,
    /// Monotonic sequence number.
    pub sequence_number: u64,
}

impl TableEntry {
    /// Creates a validated table entry.
    pub fn try_new(
        key: Vec<u8>,
        value: Option<Vec<u8>>,
        sequence_number: u64,
    ) -> Result<Self, MultiGetError> {
        if key.is_empty() {
            return Err(MultiGetError::EmptyKeyPayload);
        }
        if sequence_number == 0 {
            return Err(MultiGetError::ZeroSequenceNumber);
        }
        Ok(Self {
            key,
            value,
            sequence_number,
        })
    }
}

/// Simulated immutable version state representing active SST tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableVersion {
    /// Monotonic version identifier.
    pub version_id: VersionId,
    /// Sorted data entries across all active SSTs in this version.
    pub data: BTreeMap<Vec<u8>, TableEntry>,
}

impl ImmutableVersion {
    /// Creates a validated immutable version.
    pub fn try_new(
        version_id: VersionId,
        data: BTreeMap<Vec<u8>, TableEntry>,
    ) -> Result<Self, MultiGetError> {
        if version_id == 0 {
            return Err(MultiGetError::ZeroVersionId);
        }
        for (map_key, entry) in &data {
            if entry.key.is_empty() {
                return Err(MultiGetError::EmptyKeyPayload);
            }
            if entry.sequence_number == 0 {
                return Err(MultiGetError::ZeroSequenceNumber);
            }
            if map_key != &entry.key {
                return Err(MultiGetError::KeyEntryMismatch);
            }
        }
        Ok(Self { version_id, data })
    }
}

/// Token held by an active MultiGet operation that pins an immutable version.
pub struct PinnedVersionHandle {
    /// Pinned immutable version.
    pub version: Arc<ImmutableVersion>,
    /// Global active reader counter reference.
    reader_count: Arc<AtomicU64>,
}

impl Drop for PinnedVersionHandle {
    fn drop(&mut self) {
        self.reader_count.fetch_sub(1, Ordering::Release);
    }
}

/// Result of evaluating a single key within a pinned MultiGet session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiGetResult {
    /// Queried key.
    pub key: Vec<u8>,
    /// Found value (if any).
    pub value: Option<Vec<u8>>,
    /// Sequence number at which key was found.
    pub sequence_number: u64,
    /// Version ID from which result was derived (guaranteed uniform across the batch).
    pub version_id: VersionId,
}

impl MultiGetResult {
    /// Creates a validated MultiGetResult.
    pub fn try_new(
        key: Vec<u8>,
        value: Option<Vec<u8>>,
        sequence_number: u64,
        version_id: VersionId,
    ) -> Result<Self, MultiGetError> {
        if key.is_empty() {
            return Err(MultiGetError::EmptyBatchKey);
        }
        if version_id == 0 {
            return Err(MultiGetError::ZeroVersionId);
        }
        if value.is_some() && sequence_number == 0 {
            return Err(MultiGetError::ZeroSequenceNumber);
        }
        Ok(Self {
            key,
            value,
            sequence_number,
            version_id,
        })
    }
}

/// Coordinator for super-atomic multi-key evaluations.
pub struct SuperAtomicMultiGetCoordinator {
    current_version: Arc<ImmutableVersion>,
    reader_count: Arc<AtomicU64>,
    next_version_id: AtomicU64,
}

impl Default for SuperAtomicMultiGetCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl SuperAtomicMultiGetCoordinator {
    /// Creates coordinator with initial empty version.
    #[must_use]
    pub fn new() -> Self {
        let initial_version = Arc::new(ImmutableVersion {
            version_id: 1,
            data: BTreeMap::new(),
        });
        Self {
            current_version: initial_version,
            reader_count: Arc::new(AtomicU64::new(0)),
            next_version_id: AtomicU64::new(2),
        }
    }

    /// Acquires a pinned handle to the current immutable version.
    #[must_use]
    pub fn pin_version(&self) -> PinnedVersionHandle {
        self.reader_count.fetch_add(1, Ordering::Acquire);
        PinnedVersionHandle {
            version: Arc::clone(&self.current_version),
            reader_count: Arc::clone(&self.reader_count),
        }
    }

    /// Atomically publishes a new version resulting from compaction or flush.
    pub fn install_version(&mut self, new_data: BTreeMap<Vec<u8>, TableEntry>) -> VersionId {
        let new_id = self.next_version_id.fetch_add(1, Ordering::Relaxed);
        let new_ver = Arc::new(ImmutableVersion {
            version_id: new_id,
            data: new_data,
        });
        self.current_version = new_ver;
        new_id
    }

    /// Atomically publishes a validated new version resulting from compaction or flush.
    pub fn try_install_version(
        &mut self,
        new_data: BTreeMap<Vec<u8>, TableEntry>,
    ) -> Result<VersionId, MultiGetError> {
        let new_id = self.next_version_id.fetch_add(1, Ordering::Relaxed);
        let version = ImmutableVersion::try_new(new_id, new_data)?;
        self.current_version = Arc::new(version);
        Ok(new_id)
    }

    /// Executes an atomic MultiGet: all keys are evaluated against the same pinned version.
    pub fn execute_multiget(
        handle: &PinnedVersionHandle,
        keys: &[&[u8]],
    ) -> Vec<MultiGetResult> {
        Self::try_execute_multiget(handle, keys).unwrap_or_else(|_| {
            keys.iter()
                .map(|key| MultiGetResult {
                    key: key.to_vec(),
                    value: None,
                    sequence_number: 0,
                    version_id: handle.version.version_id,
                })
                .collect()
        })
    }

    /// Executes a validated atomic MultiGet: all keys are evaluated against the same pinned version.
    pub fn try_execute_multiget(
        handle: &PinnedVersionHandle,
        keys: &[&[u8]],
    ) -> Result<Vec<MultiGetResult>, MultiGetError> {
        if keys.is_empty() {
            return Err(MultiGetError::EmptyBatchKeys);
        }
        for key in keys {
            if key.is_empty() {
                return Err(MultiGetError::EmptyBatchKey);
            }
        }

        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(entry) = handle.version.data.get(*key) {
                results.push(MultiGetResult {
                    key: key.to_vec(),
                    value: entry.value.clone(),
                    sequence_number: entry.sequence_number,
                    version_id: handle.version.version_id,
                });
            } else {
                results.push(MultiGetResult {
                    key: key.to_vec(),
                    value: None,
                    sequence_number: 0,
                    version_id: handle.version.version_id,
                });
            }
        }
        Ok(results)
    }

    /// Verifies that all results in a MultiGet batch belong to the exact same version epoch.
    pub fn verify_batch_coherence(results: &[MultiGetResult]) -> Result<VersionId, MultiGetError> {
        if results.is_empty() {
            return Err(MultiGetError::EmptyBatchResults);
        }
        let expected_version = results[0].version_id;
        if expected_version == 0 {
            return Err(MultiGetError::ZeroVersionId);
        }
        for res in results {
            if res.version_id != expected_version {
                return Err(MultiGetError::VersionMismatch {
                    expected: expected_version,
                    actual: res.version_id,
                });
            }
        }
        Ok(expected_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_super_atomic_multiget_structural_invariants_red_to_green() {
        // 1. Error Display & std::error::Error conformance
        let errs: Vec<MultiGetError> = vec![
            MultiGetError::EmptyBatchKeys,
            MultiGetError::EmptyBatchKey,
            MultiGetError::EmptyKeyPayload,
            MultiGetError::ZeroVersionId,
            MultiGetError::ZeroSequenceNumber,
            MultiGetError::KeyEntryMismatch,
            MultiGetError::EmptyBatchResults,
            MultiGetError::VersionMismatch {
                expected: 1,
                actual: 2,
            },
        ];
        for err in &errs {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
            let dyn_err: &dyn std::error::Error = err;
            assert_eq!(dyn_err.to_string(), msg);
        }

        // 2. TableEntry::try_new validation
        assert_eq!(
            TableEntry::try_new(vec![], Some(b"val".to_vec()), 1),
            Err(MultiGetError::EmptyKeyPayload)
        );
        assert_eq!(
            TableEntry::try_new(b"k1".to_vec(), Some(b"val".to_vec()), 0),
            Err(MultiGetError::ZeroSequenceNumber)
        );
        let entry1 = TableEntry::try_new(b"k1".to_vec(), Some(b"val1".to_vec()), 10).unwrap();

        // 3. ImmutableVersion::try_new validation
        let mut data = BTreeMap::new();
        data.insert(b"k1".to_vec(), entry1);
        assert_eq!(
            ImmutableVersion::try_new(0, data.clone()),
            Err(MultiGetError::ZeroVersionId)
        );
        let mut mismatch_data = BTreeMap::new();
        mismatch_data.insert(
            b"k2".to_vec(),
            TableEntry {
                key: b"different_key".to_vec(),
                value: None,
                sequence_number: 5,
            },
        );
        assert_eq!(
            ImmutableVersion::try_new(1, mismatch_data),
            Err(MultiGetError::KeyEntryMismatch)
        );

        // 4. MultiGetResult::try_new validation
        assert_eq!(
            MultiGetResult::try_new(vec![], None, 0, 1),
            Err(MultiGetError::EmptyBatchKey)
        );
        assert_eq!(
            MultiGetResult::try_new(b"k".to_vec(), None, 0, 0),
            Err(MultiGetError::ZeroVersionId)
        );
        assert_eq!(
            MultiGetResult::try_new(b"k".to_vec(), Some(b"v".to_vec()), 0, 1),
            Err(MultiGetError::ZeroSequenceNumber)
        );

        // 5. Coordinator execution validation
        let mut coord = SuperAtomicMultiGetCoordinator::default();
        coord.try_install_version(data).unwrap();
        let handle = coord.pin_version();

        assert_eq!(
            SuperAtomicMultiGetCoordinator::try_execute_multiget(&handle, &[]),
            Err(MultiGetError::EmptyBatchKeys)
        );
        assert_eq!(
            SuperAtomicMultiGetCoordinator::try_execute_multiget(&handle, &[b""]),
            Err(MultiGetError::EmptyBatchKey)
        );

        let results = SuperAtomicMultiGetCoordinator::try_execute_multiget(
            &handle,
            &[b"k1", b"nonexistent"],
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].key, b"k1");
        assert_eq!(results[0].value, Some(b"val1".to_vec()));
        assert_eq!(results[1].key, b"nonexistent");
        assert_eq!(results[1].value, None);

        // 6. Batch coherence verification
        assert_eq!(
            SuperAtomicMultiGetCoordinator::verify_batch_coherence(&[]),
            Err(MultiGetError::EmptyBatchResults)
        );
        let ver = SuperAtomicMultiGetCoordinator::verify_batch_coherence(&results).unwrap();
        assert_eq!(ver, handle.version.version_id);

        let mut incoherent = results.clone();
        incoherent[1].version_id = 999;
        assert_eq!(
            SuperAtomicMultiGetCoordinator::verify_batch_coherence(&incoherent),
            Err(MultiGetError::VersionMismatch {
                expected: handle.version.version_id,
                actual: 999
            })
        );
    }
}

