//! `rocksdb-compat` — rust-rocksdb-shaped API subset implemented on
//! **pedradb-core** [`ConcurrentDb`](pedradb_core::ConcurrentDb).
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

use bytes::Bytes;
use parking_lot::Mutex;
use pedradb_core::{
    BatchOp, CompactOptions, ConcurrentDb, CoreError, Env, Snapshot as CoreSnapshot, StdEnv,
};
use std::collections::VecDeque;
use std::fmt;
use std::ops::Bound;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

enum CompactCmd {
    Run,
    Shutdown,
}

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
#[derive(Debug, Clone)]
pub struct Options {
    /// Whether to create the database directory when absent.
    pub create_if_missing: bool,
    /// Memtable flush threshold (Pedra `auto_flush_bytes`, default 4 MiB).
    /// `0` disables auto-flush (manual [`DB::flush`] only).
    ///
    /// Isolated apply (2000× pre+com): 4 MiB + drain **2251** qps vs 64 MiB
    /// drain **1228** (one 64 MiB SST write at the end). 64 MiB matched Rocks
    /// `write_buffer_size` and lost apply (RFC-0041).
    pub write_buffer_size: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            create_if_missing: false,
            write_buffer_size: 4 * 1024 * 1024,
        }
    }
}

impl Options {
    /// New default options (4 MiB write buffer — Pedra core default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: create the DB directory when missing.
    pub fn create_if_missing(&mut self, v: bool) -> &mut Self {
        self.create_if_missing = v;
        self
    }

    /// Builder: memtable flush threshold in bytes. Rocks default is 64 MiB.
    pub fn set_write_buffer_size(&mut self, n: usize) -> &mut Self {
        self.write_buffer_size = n;
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
        self.encode_with(cf, key, <[u8]>::to_vec)
    }

    /// Encode into a stack buffer when the key fits (RFC-0035 P1.2).
    fn encode_with<R>(&self, cf: &str, key: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
        const STACK: usize = 192;
        let effective = if cf == DEFAULT_CF && self.default_raw {
            ""
        } else {
            cf
        };
        if effective.is_empty() {
            return f(key);
        }
        let n = effective.len() + 1 + key.len();
        if n <= STACK {
            let mut buf = [0u8; STACK];
            buf[..effective.len()].copy_from_slice(effective.as_bytes());
            buf[effective.len()] = 0;
            buf[effective.len() + 1..n].copy_from_slice(key);
            f(&buf[..n])
        } else {
            let mut v = Vec::with_capacity(n);
            v.extend_from_slice(effective.as_bytes());
            v.push(0);
            v.extend_from_slice(key);
            f(&v)
        }
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

fn bound_as_ref(b: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match b {
        Bound::Included(k) => Bound::Included(k.as_slice()),
        Bound::Excluded(k) => Bound::Excluded(k.as_slice()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// First encoded key strictly greater than `enc`, if any.
fn encoded_succ(enc: &[u8]) -> Option<Vec<u8>> {
    let mut e = enc.to_vec();
    for i in (0..e.len()).rev() {
        if e[i] < 0xff {
            e[i] += 1;
            e.truncate(i + 1);
            return Some(e);
        }
    }
    None
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
        self.put_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
            value,
        );
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
        self.delete_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
        );
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

/// Page size (RFC-0032 P0.1). Forward refills; never materialises the whole CF.
const ITER_WINDOW: usize = 64;

/// Windowed CF iterator (RFC-0032 P0.1). Same positioning semantics as v0.
pub struct DBIterator<E: Env = StdEnv> {
    items: Vec<(Vec<u8>, Vec<u8>)>,
    idx: usize,
    reverse: bool,
    inner: ConcurrentDb<E>,
    codec: KeyCodec,
    cf: String,
    seq: pedradb_core::SequenceNumber,
    cf_start: Bound<Vec<u8>>,
    cf_end: Bound<Vec<u8>>,
    exhausted: bool,
}

impl<E: Env> DBIterator<E> {
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
            if self.idx == 0 {
                self.refill_reverse();
            } else {
                self.idx -= 1;
            }
        } else {
            self.idx += 1;
            if self.idx >= self.items.len() {
                self.refill_forward();
            }
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

    /// Remaining entries from here to the CF bound (refills pages).
    pub fn collect_rest(&mut self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        while self.valid() {
            out.push((self.key().to_vec(), self.value().to_vec()));
            self.next();
        }
        out
    }

    fn invalidate(&mut self) {
        self.exhausted = true;
        self.idx = if self.reverse {
            usize::MAX
        } else {
            self.items.len()
        };
    }

    fn refill_forward(&mut self) {
        if self.exhausted || self.items.is_empty() {
            self.invalidate();
            return;
        }
        let last = self.items[self.items.len() - 1].0.clone();
        let start = Bound::Excluded(self.codec.encode(&self.cf, &last));
        match page_forward(
            &self.inner,
            &self.codec,
            &self.cf,
            self.seq,
            start,
            bound_as_ref(&self.cf_end),
            ITER_WINDOW,
        ) {
            Ok(page) if !page.is_empty() => {
                self.items = page;
                self.idx = 0;
            }
            _ => self.invalidate(),
        }
    }

    fn refill_reverse(&mut self) {
        if self.exhausted || self.items.is_empty() {
            self.invalidate();
            return;
        }
        let first = self.items[0].0.clone();
        let end = Bound::Excluded(self.codec.encode(&self.cf, &first));
        match page_last_n(
            &self.inner,
            &self.codec,
            &self.cf,
            self.seq,
            bound_as_ref(&self.cf_start),
            end,
            ITER_WINDOW,
        ) {
            Ok(page) if !page.is_empty() => {
                self.idx = page.len() - 1;
                self.items = page;
            }
            _ => self.invalidate(),
        }
    }
}

fn page_forward<E: Env>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<Vec<u8>>,
    end: Bound<&[u8]>,
    limit: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let s = bound_as_ref(&start);
    inner
        .with_read(|db| db.range_at_limited(seq, s, end, Some(limit)))
        .map_err(Error::from)
        .map(|rows| {
            rows.into_iter()
                .map(|(k, v)| (codec.decode(cf, &k).to_vec(), v.to_vec()))
                .collect()
        })
}

fn page_last_n<E: Env>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    seq: pedradb_core::SequenceNumber,
    start: Bound<&[u8]>,
    end: Bound<Vec<u8>>,
    n: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let e = bound_as_ref(&end);
    inner
        .with_read(|db| {
            // Iterator borrows the Db — consume the ring window under the guard.
            let mut ring: VecDeque<(Vec<u8>, Vec<u8>)> =
                VecDeque::with_capacity(n.saturating_add(1));
            for pair in db.try_scan_at(seq, start, e, None)? {
                if ring.len() == n {
                    ring.pop_front();
                }
                ring.push_back((codec.decode(cf, &pair.key).to_vec(), pair.value.to_vec()));
            }
            Ok::<Vec<(Vec<u8>, Vec<u8>)>, CoreError>(ring.into_iter().collect())
        })
        .map_err(Error::from)
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
    pub fn iterator(&self, mode: IteratorMode) -> Result<DBIterator<E>> {
        self.iterator_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            mode,
        )
    }

    /// CF iterator pinned at the snapshot sequence.
    ///
    /// # Errors
    /// Unknown CF or snapshot-too-old.
    pub fn iterator_cf(&self, cf: &ColumnFamily, mode: IteratorMode) -> Result<DBIterator<E>> {
        scan_cf_at(
            &self.db.inner,
            &self.db.codec,
            &cf.name,
            mode,
            self.snap.sequence(),
            &self.db.cfs,
        )
    }
}

fn cf_bounds(codec: &KeyCodec, cf: &str) -> (Bound<Vec<u8>>, Bound<Vec<u8>>) {
    if codec.default_raw && cf == DEFAULT_CF {
        (Bound::Unbounded, Bound::Unbounded)
    } else {
        let start = Bound::Included(codec.encode(cf, &[]));
        let mut succ = codec.encode(cf, &[]);
        *succ.last_mut().expect("prefix non-empty") = 1;
        (start, Bound::Excluded(succ))
    }
}

fn scan_cf_at<E: Env>(
    inner: &ConcurrentDb<E>,
    codec: &KeyCodec,
    cf: &str,
    mode: IteratorMode,
    seq: pedradb_core::SequenceNumber,
    known: &[String],
) -> Result<DBIterator<E>> {
    if cf != DEFAULT_CF && !known.iter().any(|c| c == cf) {
        return Err(Error(format!("column family not found: {cf}")));
    }
    let (cf_start, cf_end) = cf_bounds(codec, cf);
    let (items, idx, reverse) = match mode {
        IteratorMode::Start | IteratorMode::From(_, Direction::Forward) => {
            let user_lo = match mode {
                IteratorMode::From(k, _) => Bound::Included(codec.encode(cf, k)),
                _ => cf_start.clone(),
            };
            let page = page_forward(
                inner,
                codec,
                cf,
                seq,
                user_lo,
                bound_as_ref(&cf_end),
                ITER_WINDOW,
            )?;
            (page, 0, false)
        }
        IteratorMode::End => {
            let page = page_last_n(
                inner,
                codec,
                cf,
                seq,
                bound_as_ref(&cf_start),
                cf_end.clone(),
                ITER_WINDOW,
            )?;
            let i = page.len().saturating_sub(1);
            (page, i, true)
        }
        IteratorMode::From(k, Direction::Reverse) => {
            let enc = codec.encode(cf, k);
            let hi = match encoded_succ(&enc) {
                Some(s) => Bound::Excluded(s),
                None => cf_end.clone(),
            };
            let page = page_last_n(
                inner,
                codec,
                cf,
                seq,
                bound_as_ref(&cf_start),
                hi,
                ITER_WINDOW,
            )?;
            let i = page.len().saturating_sub(1);
            (page, i, true)
        }
    };
    let exhausted = items.is_empty();
    Ok(DBIterator {
        items,
        idx,
        reverse,
        inner: inner.clone(),
        codec: codec.clone(),
        cf: cf.to_string(),
        seq,
        cf_start,
        cf_end,
        exhausted,
    })
}

/// rust-rocksdb-shaped database on top of a Pedra `ConcurrentDb`.
///
/// Writes join the Rocks-style write group (one leader takes the write lock
/// per group: appends + a single fdatasync + apply); reads take RwLock read
/// guards; the host compact worker reuses the core staged flush pipeline.
pub struct DB<E: Env = StdEnv> {
    inner: ConcurrentDb<E>,
    cfs: Vec<String>,
    codec: KeyCodec,
    /// Host compact worker (RFC-0037 P2.1). None when the caller injected Env
    /// (adversarial FailingEnv stays single-threaded / deterministic).
    compact_tx: Option<SyncSender<CompactCmd>>,
    compact_thread: Option<JoinHandle<()>>,
    compact_gate: Arc<Mutex<()>>,
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
        let mut db = Self::open_cf_with_env(opts, path, cfs, StdEnv)?;
        let (tx, th) = spawn_compact_worker(db.inner.clone(), Arc::clone(&db.compact_gate));
        if th.is_some() {
            db.inner.set_defer_auto_compact(true);
            db.compact_tx = tx;
            db.compact_thread = th;
        }
        Ok(db)
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
        Self::open_cf_inner(opts, path, cfs, env)
    }

    fn open_cf_inner(
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
        let mut core_opts = pedradb_core::OpenOptions::default();
        core_opts.auto_flush_bytes = if opts.write_buffer_size == 0 {
            None
        } else {
            Some(opts.write_buffer_size)
        };
        let db = ConcurrentDb::open_with_env(dir, core_opts, env)?;
        let codec = KeyCodec::new(&names);
        Ok(Self {
            inner: db,
            cfs: names,
            codec,
            compact_tx: None,
            compact_thread: None,
            compact_gate: Arc::new(Mutex::new(())),
        })
    }

    fn notify_compact(&self) {
        if let Some(tx) = &self.compact_tx {
            let _ = tx.try_send(CompactCmd::Run);
        }
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
        self.put_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
            value,
        )
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
        let encoded = self.codec.encode(&cf.name, key.as_ref());
        let r = self.inner.put(encoded, value.as_ref()).map_err(Error::from);
        self.notify_compact();
        r
    }

    /// Get from the default CF.
    ///
    /// # Errors
    /// Pedra read errors.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.get_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
        )
    }

    /// Get from a named CF.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn get_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.check_cf(&cf.name)?;
        self.codec.encode_with(&cf.name, key.as_ref(), |enc| {
            Ok(self.inner.with_read(|db| db.get(enc)).map(|b| b.to_vec()))
        })
    }

    fn get_at(
        &self,
        snap: CoreSnapshot,
        cf: &str,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        self.check_cf(cf)?;
        let encoded = self.codec.encode(cf, key.as_ref());
        self.inner
            .with_read(|db| db.get_at(snap, &encoded).map(|v| v.map(|b| b.to_vec())))
            .map_err(Error::from)
    }

    /// Delete from the default CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete(&self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            key,
        )
    }

    /// Delete from a named CF.
    ///
    /// # Errors
    /// WAL I/O or unknown CF.
    pub fn delete_cf(&self, cf: &ColumnFamily, key: impl AsRef<[u8]>) -> Result<()> {
        self.check_cf(&cf.name)?;
        let encoded = self.codec.encode(&cf.name, key.as_ref());
        let r = self.inner.delete(encoded).map_err(Error::from);
        self.notify_compact();
        r
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
        let lo = self.codec.encode(&cf.name, start.as_ref());
        let hi = self.codec.encode(&cf.name, end.as_ref());
        let r = self.inner.delete_range(lo, hi).map_err(Error::from);
        self.notify_compact();
        r
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
        let r = self.inner.apply_batch(ops).map(|_| ()).map_err(Error::from);
        self.notify_compact();
        r
    }

    /// Sequence-pinned snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<'_, E> {
        let snap = self.inner.snapshot();
        Snapshot { db: self, snap }
    }

    /// Iterator over the default CF at the latest sequence.
    ///
    /// # Errors
    /// Pedra scan errors.
    pub fn iterator(&self, mode: IteratorMode) -> Result<DBIterator<E>> {
        self.iterator_cf(
            &ColumnFamily {
                name: DEFAULT_CF.into(),
            },
            mode,
        )
    }

    /// Iterator over a named CF at the latest sequence.
    ///
    /// # Errors
    /// Unknown CF or Pedra scan errors.
    pub fn iterator_cf(&self, cf: &ColumnFamily, mode: IteratorMode) -> Result<DBIterator<E>> {
        let seq = self.inner.last_sequence();
        scan_cf_at(&self.inner, &self.codec, &cf.name, mode, seq, &self.cfs)
    }

    /// Last user key in `cf` that starts with `prefix` (RFC-0033).
    ///
    /// Same visibility as `get` (newest live version, no tombstones). Does not
    /// walk the prefix. WAL / fencing / accept-set unchanged.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn last_key_with_prefix(
        &self,
        cf: &ColumnFamily,
        prefix: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        self.check_cf(&cf.name)?;
        let encoded = self.codec.encode(&cf.name, prefix.as_ref());
        self.inner
            .with_read(|db| {
                let seq = db.last_sequence();
                db.last_under_user_prefix(seq, &encoded)
                    .map(|k| k.map(|k| self.codec.decode(&cf.name, &k).to_vec()))
            })
            .map_err(Error::from)
    }

    /// Latest key under `prefix` in `last_cf`, then point-get that user key in
    /// `get_cf` (RFC-0035 P1.1). One mutex — same visibility as the two calls.
    ///
    /// # Errors
    /// Unknown CF or Pedra read errors.
    pub fn last_prefix_then_get(
        &self,
        last_cf: &ColumnFamily,
        prefix: impl AsRef<[u8]>,
        get_cf: &ColumnFamily,
    ) -> Result<Option<Vec<u8>>> {
        self.check_cf(&last_cf.name)?;
        self.check_cf(&get_cf.name)?;
        let t_enc0 = Instant::now();
        self.codec
            .encode_with(&last_cf.name, prefix.as_ref(), |enc| {
                let ns_enc0 = u64::try_from(t_enc0.elapsed().as_nanos()).unwrap_or(u64::MAX);
                self.inner.with_read(|db| {
                    let seq = db.last_sequence();
                    let t_last = Instant::now();
                    let Some(k) = db.last_under_user_prefix(seq, enc)? else {
                        return Ok(None);
                    };
                    let ns_last = u64::try_from(t_last.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    let user = self.codec.decode(&last_cf.name, &k);
                    let t_enc1 = Instant::now();
                    self.codec.encode_with(&get_cf.name, user, |gk| {
                        let ns_enc = ns_enc0.saturating_add(
                            u64::try_from(t_enc1.elapsed().as_nanos()).unwrap_or(u64::MAX),
                        );
                        let t_get = Instant::now();
                        let got = db.get(gk);
                        let ns_get = u64::try_from(t_get.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        let t_copy = Instant::now();
                        let out = got.map(|b| b.to_vec());
                        let ns_copy =
                            u64::try_from(t_copy.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        db.record_mvcc_split(ns_enc, ns_last, ns_get, ns_copy);
                        Ok(out)
                    })
                })
            })
    }

    /// Count live keys in `[start, end)` in `cf`, stopping at `limit` (RFC-0033).
    ///
    /// Key-only projection: same visibility as a forward iterator, no value
    /// resolve. Used by deps_scan; does not change iterator value semantics.
    ///
    /// # Errors
    /// Unknown CF or Pedra scan errors.
    pub fn count_cf(
        &self,
        cf: &ColumnFamily,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
        limit: usize,
    ) -> Result<usize> {
        self.check_cf(&cf.name)?;
        self.codec.encode_with(&cf.name, start.as_ref(), |lo| {
            self.codec.encode_with(&cf.name, end.as_ref(), |hi| {
                self.inner
                    .with_read(|db| {
                        let seq = db.last_sequence();
                        db.count_in_range(
                            seq,
                            Bound::Included(lo),
                            Bound::Excluded(hi),
                            Some(limit),
                        )
                    })
                    .map_err(Error::from)
            })
        })
    }

    /// Zero latest/scan probe counters (RFC-0035).
    pub fn reset_read_probe(&self) {
        self.inner.with_read(|db| db.reset_read_probe());
    }

    /// Snapshot latest/scan counters + LSM shape (RFC-0035).
    #[must_use]
    pub fn read_probe(&self) -> pedradb_core::ReadProbeSnap {
        self.inner.with_read(|db| db.read_probe())
    }

    /// Write-group diagnostics (RFC-0040 P1.2): submits / queued / groups / ops.
    #[must_use]
    pub fn write_group_stats(&self) -> (u64, u64, u64, u64) {
        self.inner.write_group_stats()
    }

    /// Flush memtable to SST (staged pipeline; SST I/O off the write lock).
    ///
    /// # Errors
    /// Pedra flush errors (I/O).
    pub fn flush(&self) -> Result<()> {
        // Serialize with the host L0 worker: compact deletes retired files
        // and must not race an in-flight L0 install (ENOENT on put/flush).
        let _gate = self.compact_gate.lock();
        let r = self.inner.flush().map_err(Error::from);
        drop(_gate);
        self.notify_compact();
        r
    }

    /// Manual compaction (whole merge).
    ///
    /// # Errors
    /// Pedra compaction errors.
    pub fn compact(&self) -> Result<()> {
        let _gate = self.compact_gate.lock();
        self.inner.compact().map_err(Error::from)
    }
}

impl<E: Env> Drop for DB<E> {
    fn drop(&mut self) {
        if let Some(tx) = self.compact_tx.take() {
            let _ = tx.send(CompactCmd::Shutdown);
        }
        if let Some(h) = self.compact_thread.take() {
            let _ = h.join();
        }
    }
}

fn spawn_compact_worker(
    inner: ConcurrentDb<StdEnv>,
    gate: Arc<Mutex<()>>,
) -> (Option<SyncSender<CompactCmd>>, Option<JoinHandle<()>>) {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name("pedra-compat-compact".into())
        .spawn(move || loop {
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(CompactCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    while inner.park_imm_once() {}
                    while inner.materialize_parked_once() {}
                    while inner.drain_imm_once() {}
                    break;
                }
                Ok(CompactCmd::Run) | Err(RecvTimeoutError::Timeout) => {
                    // Drain imm → L0 during writes (scansst MVCC 2.58 /
                    // scan 1.15). Park-during-writes (idleinc) scan 0.24.
                    // One compact/tick (b4c1) left L0=24 and scan 0.28.
                    // Idle `while` still drains leftover L0s.
                    while inner.drain_imm_once() {}
                    if inner.writes_idle_for(Duration::from_millis(5)) {
                        let _ = inner.persist_unsynced_l0s_off_lock();
                        let _ = inner.rotate_wal_if_writers_idle();
                        while compat_compact_once(&inner, &gate) {}
                    }
                }
            }
        })
        .ok();
    (Some(tx), handle)
}

/// One L0→L1 job. I/O runs without the write lock (G5: failed write is not installed).
fn compat_compact_once<E: Env>(inner: &ConcurrentDb<E>, gate: &Mutex<()>) -> bool {
    // Only invoked when writers are idle — drain every leftover L0 so a
    // mid-loop compact that hits L0=0 cannot leave a sub-trigger remnant.
    let l0 = inner.with_read(|db| db.level_file_count(0));
    if l0 == 0 {
        return false;
    }
    let _gate = gate.lock();
    let job = inner.with_write(|db| {
        if db.level_file_count(0) == 0 {
            return None;
        }
        db.prepare_l0_compact(CompactOptions::default())
            .ok()
            .flatten()
    });
    let Some(job) = job else {
        return false;
    };
    let table = match job.write() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if !inner.install_prepared_l0_off_lock(job, table) {
        return false;
    }
    inner.with_read(|db| db.level_file_count(0)) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("rdbcompat-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn default_write_buffer_is_4_mib() {
        assert_eq!(Options::new().write_buffer_size, 4 * 1024 * 1024);
    }

    #[test]
    fn host_worker_flush_writes_sst_and_keeps_keys() {
        let dir = tmp("worker-flush");
        let mut opts = Options::new();
        opts.create_if_missing(true);
        opts.set_write_buffer_size(256 * 1024);
        let db = DB::open(&opts, &dir).unwrap();
        let payload = vec![b'x'; 2048];
        for i in 0..3000u32 {
            db.put(i.to_be_bytes(), &payload).unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if db.read_probe().sst_count >= 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            db.read_probe().sst_count >= 1,
            "host worker must write L0, probe={:?}",
            db.read_probe()
        );
        for i in [0u32, 1500, 2999] {
            assert_eq!(
                db.get(i.to_be_bytes()).unwrap().as_deref(),
                Some(payload.as_slice()),
                "acked key {i}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_worker_compacts_l0_without_hiding_keys() {
        let dir = tmp("worker-l0");
        let db = DB::open_default(&dir).unwrap();
        // 4 MiB auto-flush is huge for this test — flush explicitly.
        for i in 0..8u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
            db.flush().unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let p = db.read_probe();
            if p.l0_files == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            db.read_probe().l0_files,
            0,
            "host worker should drain every L0, got {}",
            db.read_probe().l0_files
        );
        for i in 0..8u8 {
            assert_eq!(db.get(&[b'k', i]).unwrap().as_deref(), Some(&[b'v', i][..]));
        }
        let _ = std::fs::remove_dir_all(&dir);
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
        let ghost = ColumnFamily {
            name: "ghost".into(),
        };
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
        assert_eq!(
            db.get_cf(&raft, b"log-1").unwrap().as_deref(),
            Some(&b"r1"[..])
        );
        assert_eq!(db.get(b"log-1").unwrap().as_deref(), Some(&b"d1"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn iterator_window_forward_matches_model() {
        let dir = tmp("iterwin");
        let db = DB::open_default(&dir).unwrap();
        let n = 200usize;
        for i in 0..n {
            db.put(format!("k{i:04}").as_bytes(), [i as u8]).unwrap();
        }
        let mid = format!("k{:04}", 150);
        let mut it = db
            .iterator(IteratorMode::From(mid.as_bytes(), Direction::Forward))
            .unwrap();
        let got = it.collect_rest();
        assert_eq!(got.len(), n - 150, "got {}", got.len());
        assert_eq!(got[0].0, b"k0150");
        assert_eq!(got.last().unwrap().0, b"k0199");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_prefix_does_not_see_other_users() {
        let dir = tmp("latestp");
        let db = DB::open_default(&dir).unwrap();
        for u in 0..80u8 {
            for ver in 1..=3u8 {
                let mut k = format!("u/{u:03}").into_bytes();
                k.extend_from_slice(&u64::from(ver).to_be_bytes());
                db.put(&k, [ver]).unwrap();
            }
        }
        let prefix = format!("u/{:03}", 40).into_bytes();
        let mut it = db
            .iterator(IteratorMode::From(prefix.as_slice(), Direction::Forward))
            .unwrap();
        let mut last = None;
        while it.valid() && it.key().starts_with(&prefix) {
            last = Some(it.key().to_vec());
            it.next();
        }
        let last = last.expect("user 40 has versions");
        assert!(last.starts_with(&prefix));
        assert_eq!(&last[prefix.len()..], &3u64.to_be_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_key_with_prefix_and_count_cf() {
        let dir = tmp("lastpref");
        let db = DB::open_cf(&Options::new(), &dir, &["write"]).unwrap();
        let cf = db.cf_handle("write").unwrap();
        for u in 0..8u8 {
            for ver in 1..=3u8 {
                let mut k = format!("u/{u:02}").into_bytes();
                k.extend_from_slice(&u64::from(ver).to_be_bytes());
                db.put_cf(&cf, &k, [ver]).unwrap();
            }
        }
        let prefix = b"u/03".as_slice();
        let last = db
            .last_key_with_prefix(&cf, prefix)
            .unwrap()
            .expect("user 03");
        assert!(last.starts_with(prefix), "{last:?}");
        assert_eq!(&last[prefix.len()..], &3u64.to_be_bytes());
        db.delete_cf(&cf, &last).unwrap();
        let prev = db
            .last_key_with_prefix(&cf, prefix)
            .unwrap()
            .expect("older");
        assert_eq!(&prev[prefix.len()..], &2u64.to_be_bytes());
        let n = db.count_cf(&cf, b"u/00", b"u/05", 25).unwrap();
        assert_eq!(n, 14); // 5 users × 3 vers − 1 delete
        let def = db.cf_handle(DEFAULT_CF).unwrap();
        db.put_cf(&def, &prev, b"val").unwrap();
        let got = db
            .last_prefix_then_get(&cf, prefix, &def)
            .unwrap()
            .expect("combined");
        assert_eq!(got, b"val");
        let probe = db.read_probe();
        assert_eq!(probe.mvcc_split_ops, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
