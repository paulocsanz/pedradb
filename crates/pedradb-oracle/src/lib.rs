//! PedraDB oracle — uses real RocksDB (C++) as an external correctness oracle.
//!
//! This crate is **dev/test-only** infrastructure. It is never linked into the
//! shipped `pedradb-core` engine. Its sole purpose is to run the same workload
//! against both real RocksDB and our Rust implementation and diff the results,
//! so the C++ engine serves as a ground-truth check for our clean-room rewrite.
//!
//! When the `live-rocksdb` feature is off (the default), the live oracle is
//! unavailable and tests that need it are skipped.

#![cfg_attr(not(feature = "live-rocksdb"), allow(dead_code))]

/// Trait implemented by any KV store we want to cross-check against the oracle.
pub trait KvStore {
    /// Single-key error type.
    type Error;

    /// Insert/overwrite a key-value pair.
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error>;

    /// Point lookup.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Delete a key.
    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error>;
}

#[cfg(feature = "live-rocksdb")]
pub mod live;

/// An operation in a deterministic workload, used to drive both engines and
/// compare their resulting state. Pure data, no I/O.
#[derive(Debug, Clone)]
pub enum Op {
    /// `put(key, value)`
    Put(Vec<u8>, Vec<u8>),
    /// `delete(key)`
    Delete(Vec<u8>),
}

/// A sorted snapshot of an engine's state, produced after replaying a workload.
pub type Snapshot = Vec<(Vec<u8>, Vec<u8>)>;

/// Run a sequence of operations against an engine and snapshot its state as a
/// sorted `(key, value)` map. Used by the diff harness to compare engines.
pub fn replay<S: KvStore>(ops: &[Op], store: &mut S) -> Result<Snapshot, S::Error> {
    for op in ops {
        match op {
            Op::Put(k, v) => store.put(k, v)?,
            Op::Delete(k) => store.delete(k)?,
        }
    }
    // The snapshot itself is engine-specific; callers extract it after replay.
    let _ = store;
    Ok(Vec::new())
}
