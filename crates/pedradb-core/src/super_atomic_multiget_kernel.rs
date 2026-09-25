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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Unique identifier for an immutable LSM version state.
pub type VersionId = u64;

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

/// Simulated immutable version state representing active SST tables.
#[derive(Debug, Clone)]
pub struct ImmutableVersion {
    /// Monotonic version identifier.
    pub version_id: VersionId,
    /// Sorted data entries across all active SSTs in this version.
    pub data: BTreeMap<Vec<u8>, TableEntry>,
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

/// Coordinator for super-atomic multi-key evaluations.
pub struct SuperAtomicMultiGetCoordinator {
    current_version: Arc<ImmutableVersion>,
    reader_count: Arc<AtomicU64>,
    next_version_id: AtomicU64,
}

impl SuperAtomicMultiGetCoordinator {
    /// Creates coordinator with initial empty version.
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

    /// Executes an atomic MultiGet: all keys are evaluated against the same pinned version.
    pub fn execute_multiget(
        handle: &PinnedVersionHandle,
        keys: &[&[u8]],
    ) -> Vec<MultiGetResult> {
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
        results
    }
}
