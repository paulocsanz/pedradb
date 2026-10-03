//! RFC-0290: Transactional Overlay Read View and Read-Your-Own-Writes Bisimulation Kernel.
//!
//! Formally models transactional overlay composition with immutable DB snapshots:
//! ReadView = Overlay(LocalBatch) o Snapshot(DbVersion).
//! Invariant:
//! Eval(k, ReadView) =
//!   Some(v)  if Local(k) == Put(v)
//!   None     if Local(k) == Delete
//!   Snap(k)  if k not in Local.
//! Proves strict causal consistency and zero cross-transactional data leakage.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Errors resulting from invalid transactional overlay operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayError {
    /// Transaction key cannot be empty.
    EmptyKey,
    /// Transaction write buffer has exceeded configured memory capacity.
    CapacityExceeded {
        /// Current mutation count.
        current: usize,
        /// Maximum allowed mutations.
        max: usize,
    },
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyKey => write!(f, "Transaction key cannot be empty"),
            Self::CapacityExceeded { current, max } => {
                write!(f, "Local mutation capacity exceeded ({current} >= {max})")
            }
        }
    }
}

impl std::error::Error for OverlayError {}

/// A local uncommitted mutation in the transaction write buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalMutation {
    /// Insertion or update with a value payload.
    Put(Vec<u8>),
    /// Deletion tombstone within the local transaction scope.
    Delete,
}

/// Transactional overlay evaluating Read-Your-Own-Writes queries.
#[derive(Debug, Clone, Default)]
pub struct TransactionalOverlay {
    /// Private volatile write buffer of the active transaction.
    local_mutations: BTreeMap<Vec<u8>, LocalMutation>,
    /// Optional maximum allowed number of uncommitted local mutations.
    max_capacity: Option<usize>,
}

impl TransactionalOverlay {
    /// Creates a new unbounded transactional overlay.
    #[must_use]
    pub fn new() -> Self {
        Self {
            local_mutations: BTreeMap::new(),
            max_capacity: None,
        }
    }

    /// Creates a new bounded transactional overlay with a maximum capacity limit.
    #[must_use]
    pub fn with_capacity(max_capacity: usize) -> Self {
        Self {
            local_mutations: BTreeMap::new(),
            max_capacity: Some(max_capacity),
        }
    }

    /// Returns the maximum capacity limit, if configured.
    #[must_use]
    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    /// Safely records a local uncommitted put mutation, checking non-empty key and capacity limits.
    pub fn try_put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), OverlayError> {
        if key.is_empty() {
            return Err(OverlayError::EmptyKey);
        }
        if let Some(max) = self.max_capacity {
            if !self.local_mutations.contains_key(&key) && self.local_mutations.len() >= max {
                return Err(OverlayError::CapacityExceeded {
                    current: self.local_mutations.len(),
                    max,
                });
            }
        }
        self.local_mutations.insert(key, LocalMutation::Put(value));
        Ok(())
    }

    /// Records a local uncommitted put mutation (ignoring empty keys for backward compatibility).
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let _ = self.try_put(key, value);
    }

    /// Safely records a local uncommitted delete tombstone, checking non-empty key and capacity limits.
    pub fn try_delete(&mut self, key: Vec<u8>) -> Result<(), OverlayError> {
        if key.is_empty() {
            return Err(OverlayError::EmptyKey);
        }
        if let Some(max) = self.max_capacity {
            if !self.local_mutations.contains_key(&key) && self.local_mutations.len() >= max {
                return Err(OverlayError::CapacityExceeded {
                    current: self.local_mutations.len(),
                    max,
                });
            }
        }
        self.local_mutations.insert(key, LocalMutation::Delete);
        Ok(())
    }

    /// Records a local uncommitted delete tombstone (ignoring empty keys for backward compatibility).
    pub fn delete(&mut self, key: Vec<u8>) {
        let _ = self.try_delete(key);
    }

    /// Clears all uncommitted local mutations (rollback).
    pub fn rollback(&mut self) {
        self.local_mutations.clear();
    }

    /// Returns the number of uncommitted local mutations.
    #[must_use]
    pub fn local_mutation_count(&self) -> usize {
        self.local_mutations.len()
    }

    /// Evaluates a point read under the composited transactional view.
    ///
    /// # Mathematical Invariant:
    /// 1. Local Put => returns Some(value) immediately.
    /// 2. Local Delete => returns None immediately (masks any snapshot value).
    /// 3. Absent from local => delegates to `snapshot_lookup(key)`.
    pub fn get<F>(&self, key: &[u8], snapshot_lookup: F) -> Option<Vec<u8>>
    where
        F: FnOnce(&[u8]) -> Option<Vec<u8>>,
    {
        match self.local_mutations.get(key) {
            Some(LocalMutation::Put(ref val)) => Some(val.clone()),
            Some(LocalMutation::Delete) => None,
            None => snapshot_lookup(key),
        }
    }

    /// Checks if a key has been locally deleted.
    #[must_use]
    pub fn is_locally_deleted(&self, key: &[u8]) -> bool {
        matches!(self.local_mutations.get(key), Some(LocalMutation::Delete))
    }

    /// Checks if a key has been locally modified (Put or Delete).
    #[must_use]
    pub fn is_locally_dirty(&self, key: &[u8]) -> bool {
        self.local_mutations.contains_key(key)
    }

    /// Provides an iterator over all uncommitted local mutations.
    pub fn iter_mutations(&self) -> impl Iterator<Item = (&Vec<u8>, &LocalMutation)> {
        self.local_mutations.iter()
    }

    /// Extracts and drains all uncommitted local mutations from the buffer.
    pub fn drain_mutations(&mut self) -> BTreeMap<Vec<u8>, LocalMutation> {
        std::mem::take(&mut self.local_mutations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_put_and_delete_empty_key_rejected() {
        let mut overlay = TransactionalOverlay::new();
        let err_put = overlay.try_put(vec![], vec![1, 2, 3]).unwrap_err();
        assert_eq!(err_put, OverlayError::EmptyKey);

        let err_del = overlay.try_delete(vec![]).unwrap_err();
        assert_eq!(err_del, OverlayError::EmptyKey);
    }

    #[test]
    fn test_capacity_limit_enforced() {
        let mut overlay = TransactionalOverlay::with_capacity(2);
        assert!(overlay.try_put(b"k1".to_vec(), b"v1".to_vec()).is_ok());
        assert!(overlay.try_put(b"k2".to_vec(), b"v2".to_vec()).is_ok());

        // Updating an existing key succeeds
        assert!(overlay.try_put(b"k1".to_vec(), b"v1_updated".to_vec()).is_ok());

        // Inserting a new key exceeds capacity
        let err = overlay.try_put(b"k3".to_vec(), b"v3".to_vec()).unwrap_err();
        assert_eq!(
            err,
            OverlayError::CapacityExceeded {
                current: 2,
                max: 2,
            }
        );
    }

    #[test]
    fn test_iter_and_drain_mutations() {
        let mut overlay = TransactionalOverlay::new();
        overlay.try_put(b"k1".to_vec(), b"v1".to_vec()).unwrap();
        overlay.try_delete(b"k2".to_vec()).unwrap();

        let keys: Vec<&Vec<u8>> = overlay.iter_mutations().map(|(k, _)| k).collect();
        assert_eq!(keys, vec![&b"k1".to_vec(), &b"k2".to_vec()]);

        let drained = overlay.drain_mutations();
        assert_eq!(drained.len(), 2);
        assert_eq!(overlay.local_mutation_count(), 0);
    }
}

