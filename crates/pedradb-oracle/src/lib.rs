//! PedraDB oracle harness (P2.4).
//!
//! Cross-checks PedraDB against a reference implementation:
//! - **Default:** in-memory [`ModelStore`] (BTreeMap, last-write-wins per key).
//! - **Optional `live-rocksdb`:** real RocksDB when a C++ toolchain is available.
//!
//! Never linked into production engine binaries; for correctness tests only.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(not(feature = "live-rocksdb"), allow(dead_code))]

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;

use pedradb_core::{BatchOp, Db, OpenOptions};

/// Trait implemented by any KV store we want to cross-check.
pub trait KvStore {
    /// Error type for this store.
    type Error: std::fmt::Debug;

    /// Insert/overwrite.
    ///
    /// # Errors
    /// Store-specific.
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error>;

    /// Point lookup.
    ///
    /// # Errors
    /// Store-specific.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Delete a key.
    ///
    /// # Errors
    /// Store-specific.
    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error>;

    /// Full sorted snapshot of live keys (for diff).
    ///
    /// # Errors
    /// Store-specific.
    fn snapshot(&self) -> Result<Snapshot, Self::Error>;
}

/// A sorted snapshot of an engine's visible state.
pub type Snapshot = Vec<(Vec<u8>, Vec<u8>)>;

/// Workload operation (shared by PedraDB and reference engines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `put(key, value)`
    Put(Vec<u8>, Vec<u8>),
    /// `delete(key)`
    Delete(Vec<u8>),
    /// Atomic multi-op batch (PedraDB `apply_batch`; model applies in order).
    Batch(Vec<Op>),
}

/// In-memory last-write-wins model (default reference engine).
#[derive(Debug, Default, Clone)]
pub struct ModelStore {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl ModelStore {
    /// Empty model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl KvStore for ModelStore {
    type Error = std::convert::Infallible;

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.map.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(self.map.get(key).cloned())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error> {
        self.map.remove(key);
        Ok(())
    }

    fn snapshot(&self) -> Result<Snapshot, Self::Error> {
        Ok(self
            .map
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
}

/// PedraDB adapter for the oracle harness.
pub struct PedraStore {
    db: Db,
}

impl PedraStore {
    /// Open PedraDB at `path` with sync durability.
    ///
    /// # Errors
    /// Open failures.
    pub fn open(path: impl AsRef<Path>) -> pedradb_core::Result<Self> {
        Ok(Self {
            db: Db::open_with(
                path,
                OpenOptions {
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )?,
        })
    }

    /// Underlying DB (tests / cleanup).
    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }
}

impl KvStore for PedraStore {
    type Error = pedradb_core::CoreError;

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.db.put(key, value)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(self.db.get(key).map(|b| b.to_vec()))
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error> {
        self.db.delete(key)
    }

    fn snapshot(&self) -> Result<Snapshot, Self::Error> {
        Ok(self
            .db
            .range_limited(Bound::Unbounded, Bound::Unbounded, None)
            .into_iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect())
    }
}

/// Apply a workload to a store (including nested batches).
///
/// # Errors
/// Propagates store errors.
pub fn apply_ops<S: KvStore>(ops: &[Op], store: &mut S) -> Result<(), S::Error> {
    for op in ops {
        match op {
            Op::Put(k, v) => store.put(k, v)?,
            Op::Delete(k) => store.delete(k)?,
            Op::Batch(inner) => {
                // Model / generic path: sequential apply (atomicity is Pedra-specific).
                apply_ops(inner, store)?;
            }
        }
    }
    Ok(())
}

/// Apply ops to PedraDB using `apply_batch` for [`Op::Batch`] (true atomic multi-op).
///
/// # Errors
/// PedraDB errors.
pub fn apply_ops_pedra(ops: &[Op], store: &mut PedraStore) -> pedradb_core::Result<()> {
    for op in ops {
        match op {
            Op::Put(k, v) => store.put(k, v)?,
            Op::Delete(k) => store.delete(k)?,
            Op::Batch(inner) => {
                let batch: Vec<BatchOp> = inner
                    .iter()
                    .map(|o| match o {
                        Op::Put(k, v) => BatchOp::put(k, v),
                        Op::Delete(k) => BatchOp::delete(k),
                        Op::Batch(_) => {
                            // Flatten one level only in tests; nested batch → sequential.
                            // Callers should not nest in oracle workloads.
                            BatchOp::put(b"__nested__", b"")
                        }
                    })
                    .filter(|b| {
                        !matches!(
                            b,
                            BatchOp::Put { key, .. } if key.as_ref() == b"__nested__"
                        )
                    })
                    .collect();
                // If nested appeared, fall back to sequential for nested parts.
                if inner.iter().any(|o| matches!(o, Op::Batch(_))) {
                    apply_ops_pedra(inner, store)?;
                } else {
                    store.db.apply_batch(batch)?;
                }
            }
        }
    }
    Ok(())
}

/// Diff two snapshots; `Ok(())` if equal.
///
/// # Errors
/// Returns a human-readable mismatch description.
pub fn diff_snapshots(left: &Snapshot, right: &Snapshot) -> Result<(), String> {
    if left == right {
        return Ok(());
    }
    Err(format!(
        "snapshot mismatch:\n  left ({}) = {left:?}\n  right ({}) = {right:?}",
        left.len(),
        right.len()
    ))
}

/// Replay `ops` on PedraDB and on the model; require identical snapshots.
///
/// # Errors
/// PedraDB I/O or snapshot mismatch.
pub fn assert_pedra_matches_model(path: impl AsRef<Path>, ops: &[Op]) -> Result<(), String> {
    let mut model = ModelStore::new();
    apply_ops(ops, &mut model).map_err(|e| format!("{e:?}"))?;
    let model_snap = model.snapshot().map_err(|e| format!("{e:?}"))?;

    let mut pedra = PedraStore::open(path).map_err(|e| e.to_string())?;
    apply_ops_pedra(ops, &mut pedra).map_err(|e| e.to_string())?;
    let pedra_snap = pedra.snapshot().map_err(|e| e.to_string())?;

    diff_snapshots(&pedra_snap, &model_snap)
}

/// Live RocksDB backend when compiled with `--features live-rocksdb`.
#[cfg(feature = "live-rocksdb")]
pub mod live;

#[cfg(feature = "live-rocksdb")]
pub use live::{assert_pedra_matches_rocks, RocksStore};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-oracle-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn pedra_matches_model_on_mixed_workload() {
        let dir = temp_dir();
        let ops = vec![
            Op::Put(b"a".to_vec(), b"1".to_vec()),
            Op::Put(b"b".to_vec(), b"2".to_vec()),
            Op::Batch(vec![
                Op::Put(b"c".to_vec(), b"3".to_vec()),
                Op::Put(b"d".to_vec(), b"4".to_vec()),
            ]),
            Op::Delete(b"b".to_vec()),
            Op::Put(b"a".to_vec(), b"1b".to_vec()),
            Op::Batch(vec![
                Op::Put(b"e".to_vec(), b"5".to_vec()),
                Op::Delete(b"c".to_vec()),
            ]),
        ];
        assert_pedra_matches_model(&dir, &ops).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_snapshot_sorted() {
        let mut m = ModelStore::new();
        m.put(b"z", b"1").unwrap();
        m.put(b"a", b"2").unwrap();
        let s = m.snapshot().unwrap();
        assert_eq!(s[0].0, b"a");
        assert_eq!(s[1].0, b"z");
    }
}
