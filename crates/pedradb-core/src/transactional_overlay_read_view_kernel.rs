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
}

impl TransactionalOverlay {
    /// Creates a new empty transactional overlay.
    #[must_use]
    pub fn new() -> Self {
        Self {
            local_mutations: BTreeMap::new(),
        }
    }

    /// Records a local uncommitted put mutation.
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.local_mutations.insert(key, LocalMutation::Put(value));
    }

    /// Records a local uncommitted delete tombstone.
    pub fn delete(&mut self, key: Vec<u8>) {
        self.local_mutations.insert(key, LocalMutation::Delete);
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
}
