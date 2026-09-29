//! Live RocksDB (C++) vs `rocksdb-compat` on the rust-rocksdb 0.22 names.
//!
//! Feature `live-rocksdb` only. Divergences that are Pedra-by-design
//! (`delete_file_in_range` tombstone vs unlink) are **not** in this tape.

use std::path::Path;

use rocksdb::{Options as RocksOptions, WriteBatch as RocksBatch, DB as RocksDB};
use rocksdb_compat::{Options as CompatOptions, WriteBatch as CompatBatch, DB as CompatDB};

use crate::Op;

fn apply_rocks(db: &RocksDB, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Put(k, v) => db.put(k, v).map_err(|e| e.to_string())?,
            Op::Delete(k) => db.delete(k).map_err(|e| e.to_string())?,
            Op::Batch(inner) => {
                let mut wb = RocksBatch::default();
                for i in inner {
                    match i {
                        Op::Put(k, v) => wb.put(k, v),
                        Op::Delete(k) => wb.delete(k),
                        Op::Batch(_) => return Err("nested batch".into()),
                    }
                }
                db.write(wb).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn apply_compat(db: &CompatDB, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Put(k, v) => db.put(k, v).map_err(|e| e.to_string())?,
            Op::Delete(k) => db.delete(k).map_err(|e| e.to_string())?,
            Op::Batch(inner) => {
                let mut wb = CompatBatch::new();
                for i in inner {
                    match i {
                        Op::Put(k, v) => wb.put(k, v),
                        Op::Delete(k) => wb.delete(k),
                        Op::Batch(_) => return Err("nested batch".into()),
                    }
                }
                db.write(&wb).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn snap_rocks(db: &RocksDB) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let iter = db.iterator(rocksdb::IteratorMode::Start);
    for item in iter {
        let (k, v) = item.map_err(|e| e.to_string())?;
        out.push((k.to_vec(), v.to_vec()));
    }
    Ok(out)
}

fn snap_compat(db: &CompatDB) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut it = db
        .iterator(rocksdb_compat::IteratorMode::Start)
        .map_err(|e| e.to_string())?;
    while it.valid() {
        out.push((it.key().to_vec(), it.value().to_vec()));
        it.next();
    }
    Ok(out)
}

/// Replay `ops` on Pedra-compat and live Rocks; require identical visible maps.
///
/// # Errors
/// I/O, RocksDB, or snapshot mismatch.
pub fn assert_compat_matches_rocks(
    compat_path: impl AsRef<Path>,
    rocks_path: impl AsRef<Path>,
    ops: &[Op],
) -> Result<(), String> {
    let mut ropts = RocksOptions::default();
    ropts.create_if_missing(true);
    let rocks = RocksDB::open(&ropts, rocks_path).map_err(|e| e.to_string())?;
    apply_rocks(&rocks, ops)?;
    let rocks_snap = snap_rocks(&rocks)?;

    let mut copts = CompatOptions::new();
    copts.create_if_missing(true);
    let compat = CompatDB::open(&copts, compat_path).map_err(|e| e.to_string())?;
    apply_compat(&compat, ops)?;
    let compat_snap = snap_compat(&compat)?;

    if compat_snap != rocks_snap {
        return Err(format!(
            "compat vs live-rocks mismatch:\n  compat={compat_snap:?}\n  rocks={rocks_snap:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Op;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp(tag: &str) -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "pedra-compat-live-{tag}-{}-{i}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn mixed_put_delete_batch_matches_live_rocks() {
        let c = temp("compat");
        let r = temp("rocks");
        let ops = vec![
            Op::Put(b"a".to_vec(), b"1".to_vec()),
            Op::Put(b"b".to_vec(), b"2".to_vec()),
            Op::Delete(b"a".to_vec()),
            Op::Batch(vec![
                Op::Put(b"c".to_vec(), b"3".to_vec()),
                Op::Delete(b"b".to_vec()),
            ]),
            Op::Put(b"d".to_vec(), b"4".to_vec()),
        ];
        assert_compat_matches_rocks(&c, &r, &ops).expect("compat ≡ live Rocks");
        let _ = std::fs::remove_dir_all(&c);
        let _ = std::fs::remove_dir_all(&r);
    }
}
