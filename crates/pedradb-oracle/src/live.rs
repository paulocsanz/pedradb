//! Live RocksDB oracle (feature `live-rocksdb`).
//!
//! Requires a C++ toolchain to compile `rocksdb`. Not used by default CI.

use std::path::Path;

use rocksdb::{IteratorMode, Options, DB};

use crate::{KvStore, Snapshot};

/// RocksDB-backed store for oracle diffs.
pub struct RocksStore {
    db: DB,
}

impl RocksStore {
    /// Open (or create) a RocksDB at `path`.
    ///
    /// # Errors
    /// RocksDB open failures.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rocksdb::Error> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)?;
        Ok(Self { db })
    }
}

impl KvStore for RocksStore {
    type Error = rocksdb::Error;

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.db.put(key, value)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        self.db.get(key)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error> {
        self.db.delete(key)
    }

    fn snapshot(&self) -> Result<Snapshot, Self::Error> {
        let mut out = Snapshot::new();
        let iter = self.db.iterator(IteratorMode::Start);
        for item in iter {
            let (k, v) = item?;
            out.push((k.to_vec(), v.to_vec()));
        }
        Ok(out)
    }
}

/// Replay `ops` on PedraDB and RocksDB; require identical visible snapshots.
///
/// # Errors
/// I/O, RocksDB, or snapshot mismatch.
pub fn assert_pedra_matches_rocks(
    pedra_path: impl AsRef<Path>,
    rocks_path: impl AsRef<Path>,
    ops: &[crate::Op],
) -> Result<(), String> {
    use crate::{apply_ops, apply_ops_pedra, diff_snapshots, PedraStore};

    let mut rocks = RocksStore::open(rocks_path).map_err(|e| e.to_string())?;
    apply_ops(ops, &mut rocks).map_err(|e| e.to_string())?;
    let rocks_snap = rocks.snapshot().map_err(|e| e.to_string())?;

    let mut pedra = PedraStore::open(pedra_path).map_err(|e| e.to_string())?;
    apply_ops_pedra(ops, &mut pedra).map_err(|e| e.to_string())?;
    let pedra_snap = pedra.snapshot().map_err(|e| e.to_string())?;

    diff_snapshots(&pedra_snap, &rocks_snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Op;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_pair() -> (std::path::PathBuf, std::path::PathBuf) {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("pedradb-live-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        (base.join("pedra"), base.join("rocks"))
    }

    #[test]
    fn pedra_matches_rocks_on_mixed_workload() {
        let (pedra, rocks) = temp_pair();
        let ops = vec![
            Op::Put(b"a".to_vec(), b"1".to_vec()),
            Op::Put(b"b".to_vec(), b"2".to_vec()),
            Op::Delete(b"b".to_vec()),
            Op::Put(b"a".to_vec(), b"1b".to_vec()),
            Op::Batch(vec![
                Op::Put(b"c".to_vec(), b"3".to_vec()),
                Op::Delete(b"c".to_vec()),
                Op::Put(b"d".to_vec(), b"4".to_vec()),
            ]),
        ];
        assert_pedra_matches_rocks(&pedra, &rocks, &ops).unwrap();
    }
}
