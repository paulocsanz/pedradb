//! `rocksdb-compat` — rust-rocksdb-shaped API subset implemented on **pedradb-core**.
//!
//! Goal: let small rust-rocksdb-dependent programs swap
//! `rocksdb = { package = "rocksdb-compat", path = ... }` and run, then feed the
//! same workload through Pedra's adversarial suite (FailingEnv crash campaigns).
//!
//! **Not drop-in TiKV** (see `docs/rocksdb-compat.md`): no ingest, compaction
//! filters, properties, statistics, Titan, or `delete_files_in_range`. Column
//! families are emulated by key prefix (`cf_name \x00 key`; `default` is raw).
//! Cross-CF key collisions from embedded `\x00` in keys are a documented
//! constraint of the emulation, not of Pedra itself.
//!
//! Coverage: `open_default` / `open_cf`, `put`/`get`/`delete` (± CF),
//! `delete_range_cf`, atomic `write(WriteBatch)`, point + iterator reads on
//! `snapshot()`, `flush`, `compact`.

#![forbid(unsafe_code)]

use pedradb_core::{BatchOp, CoreError, Db, Env, Snapshot as CoreSnapshot, StdEnv};
use bytes::Bytes;
use std::fmt;
use std::ops::Bound;
use std::sync::Mutex;

/// Compatibility error surface (rust-rocksdb exposes one opaque `Error`).
#[derive(Debug, Clone)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<CoreError> for Error {
    fn from(e: CoreError) -> Self {
        Self(e.to_string())
    }
}

/// Result alias matching rust-rocksdb's shape.
pub type Result<T> = std::result::Result<T, Error>;

/// Open options (builder subset). `create_if_missing` mirrors rust-rocksdb;
/// Pedra always requires the directory to be creatable.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Whether to create the database directory when absent.
    pub create_if_missing: bool,
}

impl Options {
    /// New default options (nothing enabled).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: create the DB directory when missing.
    pub fn create_if_missing(&mut self, v: bool) -> &mut Self {
        self.create_if_missing = v;
        self
    }
}

/// Column family handle (name-keyed emulation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnFamily {
    name: String,
}

impl ColumnFamily {
    /// CF name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Default CF name (raw keys when it is the only CF; prefixed otherwise).
pub const DEFAULT_CF: &str = "default";

/// CF↔keyspace codec. `default` is raw **only** when no named CF exists;
/// otherwise it is prefixed too, so full-CF range scans never leak another
/// CF's encoded keys.
#[derive(Debug, Clone)]
struct KeyCodec {
    default_raw: bool,
}

impl KeyCodec {
    fn new(cfs: &[String]) -> Self {
        Self {
            default_raw: cfs.len() <= 1,
        }
    }

    fn encode(&self, cf: &str, key: &[u8]) -> Vec<u8> {
        let effective = if cf == DEFAULT_CF && self.default_raw {
            ""
        } else {
            cf
        };
        if effective.is_empty() {
            return key.to_vec();
        }
        let mut v = Vec::with_capacity(effective.len() + 1 + key.len());
        v.extend_from_slice(effective.as_bytes());
        v.push(0);
        v.extend_from_slice(key);
        v
    }

    fn decode<'a>(&self, cf: &str, encoded: &'a [u8]) -> &'a [u8] {
        let effective = if cf == DEFAULT_CF && self.default_raw {
            ""
        } else {
            cf
        };
        if effective.is_empty() {
            return encoded;
        }
        encoded.get(effective.len() + 1..).unwrap_or(&[])
    }
}

fn encode_bound_ref(codec: &KeyCodec, cf: &str, b: Bound<&[u8]>) -> Bound<Vec<u8>> {
    match b {
        Bound::Included(k) => Bound::Included(codec.encode(cf, k)),
        Bound::Excluded(k) => Bound::Excluded(codec.encode(cf, k)),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn bound_as_ref(b: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match b {
        Bound::Included(k) => Bound::Included(k.as_slice()),
        Bound::Excluded(k) => Bound::Excluded(k.as_slice()),
        Bound::Unbounded => Bound::Unbounded,
    }
}



/// Atomic write batch (one Pedra `apply_batch` = all-or-nothing).
#[derive(Debug, Default)]
pub struct WriteBatch {
    ops: Vec<(Option<String>, BatchOp)>,
}

impl WriteBatch {
    /// Empty batch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of staged ops.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Put into the default CF.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.put_cf(&ColumnFamily { name: DEFAULT_CF.into() }, key, value);
    }

    /// Put into a named CF.
    pub fn put_cf(&mut self, cf: &ColumnFamily, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.ops.push((
            Some(cf.name.clone()),
            BatchOp::Put {
                key: Bytes::copy_from_slice(key.as_ref()),
                value: Bytes::copy_from_slice(value.as_ref()),
            },
        ));
    }

    /// Delete from the default CF.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) {
        self.delete_cf(&ColumnFamily { name: DEFAULT_CF.into() }, key);
    }

    /// Delete from a named CF.
    pub fn delete_cf(&mut self, cf: &ColumnFamily, key: impl AsRef<[u8]>) {
        self.ops.push((
            Some(cf.name.clone()),
            BatchOp::Delete {
                key: Bytes::copy_from_slice(key.as_ref()),
            },
        ));
    }

    /// Range-delete `[start, end)` in a named CF.
    pub fn delete_range_cf(
        &mut self,
        cf: &ColumnFamily,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) {
        self.ops.push((
            Some(cf.name.clone()),
            BatchOp::DeleteRange {
                start: Bytes::copy_from_slice(start.as_ref()),
                end: Bytes::copy_from_slice(end.as_ref()),
            },
        ));
    }
}

/// Iterator direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Ascending keys.
    Forward,
    /// Descending keys.
    Reverse,
}

/// Where an iterator starts (rust-rocksdb shape).
#[derive(Debug, Clone, Copy)]
pub enum IteratorMode<'a> {
    /// First key.
    Start,
    /// Last key.
    End,
    /// From `key` in `direction`.
    From(&'a [u8], Direction),
}

/// Materialized iterator over one CF snapshot (eager; compat v0 scope).
#[derive(Debug)]
pub struct DBIterator {
    items: Vec<(Vec<u8>, Vec<u8>)>,
    idx: usize,
    reverse: bool,
}

impl DBIterator {
    /// Whether positioned on a valid entry.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.idx < self.items.len()
    }

    /// Advance (forward or backward per mode). Stepping past either end
    /// invalidates (reverse uses wrapping so index 0 → invalid, not clamped).
    pub fn next(&mut self) {
        if !self.valid() {
            return;
        }
        if self.reverse {
            self.idx = self.idx.wrapping_sub(1);
        } else {
            self.idx += 1;
        }
    }

    /// Current user key (empty when invalid).
    #[must_use]
    pub fn key(&self) -> &[u8] {
        self.items
            .get(self.idx)
            .map(|(k, _)| k.as_slice())
            .unwrap_or(&[])
    }

    /// Current value (empty when invalid).
    #[must_use]
    pub fn value(&self) -> &[u8] {
        self.items
            .get(self.idx)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }

    /// Collect remaining entries from the current position (harness helper).
    #[must_use]
    pub fn collect_rest(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        if !self.valid() {
            return Vec::new();
        }
        self.items[self.idx..].to_vec()
    }
}

/// Read snapshot (sequence-pinned point + iterator reads).
pub struct Snapshot<'a, E: Env = StdEnv> {
    db: &'a DB<E>,
    snap: CoreSnapshot,
}

impl<E: Env> Snapshot<'_, E> {
    /// Point read pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Propagates Pedra errors (I/O, snapshot-too-old after version GC).
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.db.get_at(self.snap, DEFAULT_CF, key)
    }

    /// Point read on a CF pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or Pedra errors.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.db.get_at(self.snap, &cf.name, key)
    }

    /// Iterator pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or snapshot-too-old.
    pub fn iterator(&self, mode: IteratorMode) -> Result<DBIterator> {
        self.iterator_cf(&ColumnFamily { name: DEFAULT_CF.into() }, mode)
    }

    /// CF iterator pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or snapshot-too-old.
    pub fn iterator_cf(&self, cf: &ColumnFamily, mode: IteratorMode) -> Result<DBIterator> {
        let guard = self.db.inner.lock().expect("db mutex");
        scan_cf_at(
            &guard,
            &self.db.codec,
            &cf.name,
            mode,
            self.snap.sequence(),
            &self.db.cfs,
        )
    }
}

fn scan_cf_at<E: Env>(
    db: &Db<E>,
    codec: &KeyCodec,
    cf: &str,
    mode: IteratorMode,
    seq: pedradb_core::SequenceNumber,
    known: &[String],
) -> Result<DBIterator> {
    if cf != DEFAULT_CF && !known.iter().any(|c| c == cf) {
        return Err(Error(format!("column family not found: {cf}")));
    }
    let start = match mode {
        IteratorMode::From(k, Direction::Forward) => Bound::Included(k),
        _ => Bound::Unbounded,
    };
    // Full-CF scan, then position (compat v0: eager).
    // Bound the scan to this CF's keyspace so encoded keys of other CFs
    // never leak into a full-CF iteration. Raw default (no named CFs) is the
    // whole keyspace; prefixed CFs scan [prefix, prefix\x01).
    let (start_b, end_b) = if codec.default_raw && cf == DEFAULT_CF {
        (
            encode_bound_ref(codec, cf, start),
            Bound::<Vec<u8>>::Unbounded,
        )
    } else {
        let s = match start {
            Bound::Unbounded => Bound::Included(codec.encode(cf, &[])),
            other => encode_bound_ref(codec, cf, other),
        };
        let mut succ = codec.encode(cf, &[]);
        *succ.last_mut().expect("prefix non-empty") = 1; // \0 -> \x01 successor
        (s, Bound::Excluded(succ))
    };
    let (s_ref, e_ref) = (bound_as_ref(&start_b), bound_as_ref(&end_b));
    let items: Vec<(Vec<u8>, Vec<u8>)> = db
        .range_at(seq, s_ref, e_ref)?
        .into_iter()
        .map(|(k, v)| (codec.decode(cf, &k).to_vec(), v.to_vec()))
        .collect();
    let (idx, reverse) = match mode {
        IteratorMode::Start => (0, false),
        IteratorMode::End => (items.len().saturating_sub(1), true),
        IteratorMode::From(k, Direction::Forward) => (
            items.partition_point(|(ik, _)| ik.as_slice() < k),
            false,
        ),
        IteratorMode::From(k, Direction::Reverse) => {
            // Last index with key <= k.
            let le = items.partition_point(|(ik, _)| ik.as_slice() <= k);
            (le.saturating_sub(1), true)
        }
    };
    Ok(DBIterator {
        items,
        idx,
        reverse,
    })
}

/// rust-rocksdb-shaped database on top of a Pedra `Db`.
pub struct DB<E: Env = StdEnv> {
    inner: Mutex<Db<E>>,
    cfs: Vec<String>,
    codec: KeyCodec,
}

impl DB<StdEnv> {
    /// Open (create if missing) with only the default CF.
    ///
    /// # Errors
    /// Pedra open errors (lock, manifest, I/O).
    pub fn open_default(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open_cf(&Options::new(), path, &[])
    }

    /// Open (create if missing) with explicit CFs (must include `default` only
    /// implicitly — `default` always exists).
    ///
    /// # Errors
    /// Pedra open errors.
    pub fn open(opts: &Options, path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open_cf(opts, path, &[])
    }

    /// Open with named CFs registered.
    ///
    /// # Errors
    /// Pedra open errors; duplicate CF names.
    pub fn open_cf(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
    ) -> Result<Self> {
        Self::open_cf_with_env(opts, path, cfs, StdEnv)
    }
}

impl<E: Env> DB<E> {
    /// Open with an explicit [`Env`] (adversarial `FailingEnv` campaigns).
    ///
    /// # Errors
    /// Pedra open errors; duplicate CF names.
    pub fn open_cf_with_env(
        opts: &Options,
        path: impl AsRef<std::path::Path>,
        cfs: &[&str],
        env: E,
    ) -> Result<Self> {
        let dir = path.as_ref();
        if !dir.exists() {
            if !opts.create_if_missing {
                return Err(Error(format!("db path missing: {}", dir.display())));
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| Error(format!("mkdir {}: {e}", dir.display())))?;
        }
        let mut names = vec![DEFAULT_CF.to_string()];
        for c in cfs {
            if *c == DEFAULT_CF {
                continue;
            }
            if names.iter().any(|n| n == c) {
                return Err(Error(format!("duplicate column family: {c}")));
            }
            names.push((*c).to_string());
        }
        let db = Db::open_with_env(dir, pedradb_core::OpenOptions::default(), env)?;
        let codec = KeyCodec::new(&names);
        Ok(Self {
            inner: Mutex::new(db),
            cfs: names,
            codec,
        })
    }

    /// Handle for a registered CF.
    #[must_use]
    pub fn cf_handle(&self, name: &str) -> Option<ColumnFamily> {
        self.cfs
            .iter()
            .find(|c| c.as_str() == name)
            .map(|n| ColumnFamily { name: n.clone() })
    }

    fn check_cf(&self, cf: &str) -> Result<()> {
        if cf == DEFAULT_CF || self.cfs.iter().any(|c| c == cf) {
            Ok(())
        } else {
            Err(Error(format!("column family not found: {cf}")))
        }
    }

    /// Put into the default CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_cf(&ColumnFamily { name: DEFAULT_CF.into() }, key, value)
    }

    /// Put into a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn put_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.check_cf(&cf.name)?;
        let mut guard = self.inner.lock().expect("db mutex");
        guard
            .put(self.codec.encode(&cf.name, key.as_ref()), value.as_ref())
            .map_err(Error::from)
    }

    /// Get from the default CF.
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.get_cf(&ColumnFamily { name: DEFAULT_CF.into() }, key)
    }

    /// Get from a named CF.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.check_cf(&cf.name)?;
        let guard = self.inner.lock().expect("db mutex");
        Ok(guard
            .get(&self.codec.encode(&cf.name, key.as_ref()))
            .map(|b| b.to_vec()))
    }

    fn get_at(&self, snap: CoreSnapshot, cf: &str, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.check_cf(cf)?;
        let guard = self.inner.lock().expect("db mutex");
        guard
            .get_at(snap, &self.codec.encode(cf, key.as_ref()))
            .map(|v| v.map(|b| b.to_vec()))
            .map_err(Error::from)
    }

    /// Delete from the default CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete(&self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_cf(&ColumnFamily { name: DEFAULT_CF.into() }, key)
    }

    /// Delete from a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<()> {
        self.check_cf(&cf.name)?;
        let mut guard = self.inner.lock().expect("db mutex");
        guard
            .delete(self.codec.encode(&cf.name, key.as_ref()))
            .map_err(Error::from)
    }

    /// Range-delete `[start, end)` in a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete_range_cf(
        &self,
        cf: &ColumnFamily,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.check_cf(&cf.name)?;
        let mut guard = self.inner.lock().expect("db mutex");
        guard
            .delete_range(
                self.codec.encode(&cf.name, start.as_ref()),
                self.codec.encode(&cf.name, end.as_ref()),
            )
            .map_err(Error::from)
    }

    /// Apply a `WriteBatch` atomically (one Pedra batch = one WAL record group).
    ///
    /// # Errors
    /// WAL I/O; nothing partially applied on error.
    pub fn write(&self, batch: &WriteBatch) -> Result<()> {
        let mut ops = Vec::with_capacity(batch.ops.len());
        for (cf, op) in &batch.ops {
            let name = cf.as_deref().unwrap_or(DEFAULT_CF);
            self.check_cf(name)?;
            let encoded = match op {
                BatchOp::Put { key, value } => BatchOp::Put {
                    key: Bytes::from(self.codec.encode(name, key)),
                    value: value.clone(),
                },
                BatchOp::Delete { key } => BatchOp::Delete {
                    key: Bytes::from(self.codec.encode(name, key)),
                },
                BatchOp::DeleteRange { start, end } => BatchOp::DeleteRange {
                    start: Bytes::from(self.codec.encode(name, start)),
                    end: Bytes::from(self.codec.encode(name, end)),
                },
            };
            ops.push(encoded);
        }
        let mut guard = self.inner.lock().expect("db mutex");
        guard.apply_batch(ops).map(|_| ()).map_err(Error::from)
    }

    /// Sequence-pinned snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<'_, E> {
        let guard = self.inner.lock().expect("db mutex");
        let snap = guard.snapshot();
        Snapshot { db: self, snap }
    }

    /// Iterator over the default CF at the latest sequence.
    ///
    /// # Errors
    /// Pedra scan errors.
    pub fn iterator(&self, mode: IteratorMode) -> Result<DBIterator> {
        self.iterator_cf(&ColumnFamily { name: DEFAULT_CF.into() }, mode)
    }

    /// Iterator over a named CF at the latest sequence.
    ///
    /// # Errors
    /// Unknown CF or Pedra scan errors.
    pub fn iterator_cf(&self, cf: &ColumnFamily, mode: IteratorMode) -> Result<DBIterator> {
        let guard = self.inner.lock().expect("db mutex");
        scan_cf_at(
            &guard,
            &self.codec,
            &cf.name,
            mode,
            guard.last_sequence(),
            &self.cfs,
        )
    }

    /// Flush memtable to SST.
    ///
    /// # Errors
    /// Pedra flush errors (I/O).
    pub fn flush(&self) -> Result<()> {
        let mut guard = self.inner.lock().expect("db mutex");
        guard.flush().map_err(Error::from)
    }

    /// Manual compaction (whole merge).
    ///
    /// # Errors
    /// Pedra compaction errors.
    pub fn compact(&self) -> Result<()> {
        let mut guard = self.inner.lock().expect("db mutex");
        guard.compact().map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rdbcompat-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn basic_put_get_delete_reopen() {
        let dir = tmp("basic");
        {
            let db = DB::open_default(&dir).unwrap();
            db.put(b"k1", b"v1").unwrap();
            db.put(b"k2", b"v2").unwrap();
            assert_eq!(db.get(b"k1").unwrap().as_deref(), Some(&b"v1"[..]));
            db.delete(b"k1").unwrap();
            assert_eq!(db.get(b"k1").unwrap(), None);
            db.flush().unwrap();
        }
        let db = DB::open_default(&dir).unwrap();
        assert_eq!(db.get(b"k1").unwrap(), None);
        assert_eq!(db.get(b"k2").unwrap().as_deref(), Some(&b"v2"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_batch_atomic_and_cfs() {
        let dir = tmp("batch");
        let db = DB::open_cf(&Options::new(), &dir, &["cf_a", "cf_b"]).unwrap();
        let a = db.cf_handle("cf_a").unwrap();
        let b = db.cf_handle("cf_b").unwrap();
        assert!(db.cf_handle("nope").is_none());

        let mut wb = WriteBatch::new();
        wb.put(b"dk", b"dv");
        wb.put_cf(&a, b"ak", b"av");
        wb.put_cf(&b, b"bk", b"bv");
        wb.delete(b"tmp");
        db.write(&wb).unwrap();

        assert_eq!(db.get(b"dk").unwrap().as_deref(), Some(&b"dv"[..]));
        assert_eq!(db.get_cf(&a, b"ak").unwrap().as_deref(), Some(&b"av"[..]));
        assert_eq!(db.get_cf(&b, b"bk").unwrap().as_deref(), Some(&b"bv"[..]));

        // Atomicity: a failing batch applies nothing (unknown CF short-circuits).
        let ghost = ColumnFamily { name: "ghost".into() };
        let mut wb = WriteBatch::new();
        wb.put(b"staged", b"x");
        wb.put_cf(&ghost, b"gk", b"gv");
        assert!(db.write(&wb).is_err());
        assert_eq!(db.get(b"staged").unwrap(), None);

        // Range delete scoped to one CF.
        db.delete_range_cf(&a, b"a", b"az").unwrap();
        assert_eq!(db.get_cf(&a, b"ak").unwrap(), None);
        assert_eq!(db.get_cf(&b, b"bk").unwrap().as_deref(), Some(&b"bv"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn iterator_modes_and_snapshot_isolation() {
        let dir = tmp("iter");
        let db = DB::open_cf(&Options::new(), &dir, &["cf"]).unwrap();
        let cf = db.cf_handle("cf").unwrap();
        for i in 0..6u8 {
            db.put_cf(&cf, [b'k', i], [i]).unwrap();
        }
        let mut it = db.iterator_cf(&cf, IteratorMode::Start).unwrap();
        assert_eq!(it.key(), &[b'k', 0]);
        it.next();
        assert_eq!(it.key(), &[b'k', 1]);

        let mut it = db
            .iterator_cf(&cf, IteratorMode::From(&[b'k', 3], Direction::Forward))
            .unwrap();
        assert_eq!(it.key(), &[b'k', 3]);
        it.next();
        assert_eq!(it.key(), &[b'k', 4]);

        let mut it = db
            .iterator_cf(&cf, IteratorMode::From(&[b'k', 3], Direction::Reverse))
            .unwrap();
        assert_eq!(it.key(), &[b'k', 3]);
        it.next();
        assert_eq!(it.key(), &[b'k', 2]);

        let mut it = db.iterator_cf(&cf, IteratorMode::End).unwrap();
        assert_eq!(it.key(), &[b'k', 5]);
        for expect in [4u8, 3, 2, 1, 0] {
            it.next();
            assert_eq!(it.key(), &[b'k', expect]);
        }
        it.next();
        assert!(!it.valid());

        // Snapshot isolation: pinned view does not see later writes.
        let snap = db.snapshot();
        db.put_cf(&cf, b"k9", b"new").unwrap();
        assert_eq!(snap.get_cf(&cf, b"k9").unwrap(), None);
        assert_eq!(db.get_cf(&cf, b"k9").unwrap().as_deref(), Some(&b"new"[..]));
        let it = snap.iterator_cf(&cf, IteratorMode::End).unwrap();
        assert_eq!(it.key(), &[b'k', 5]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cf_prefix_isolation() {
        let dir = tmp("cfiso");
        let db = DB::open_cf(&Options::new(), &dir, &["raft"]).unwrap();
        let raft = db.cf_handle("raft").unwrap();
        db.put_cf(&raft, b"log-1", b"r1").unwrap();
        db.put(b"log-1", b"d1").unwrap();
        assert_eq!(db.get_cf(&raft, b"log-1").unwrap().as_deref(), Some(&b"r1"[..]));
        assert_eq!(db.get(b"log-1").unwrap().as_deref(), Some(&b"d1"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
