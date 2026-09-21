//! rust-rocksdb 0.22 types that the drop-in must expose with working semantics.

use super::{ColumnFamily, Error, ErrorKind, Options, Result, WriteBatch, DB, DEFAULT_CF};
use bytes::Bytes;
use pedradb_core::{write_sst, Env, InternalKey, MemTable, SstTable, ValueType};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// rust-rocksdb `DEFAULT_COLUMN_FAMILY_NAME`.
pub const DEFAULT_COLUMN_FAMILY_NAME: &str = DEFAULT_CF;

/// Accept `&ColumnFamily` and owned handles (rust-rocksdb `AsColumnFamilyRef`).
pub trait AsColumnFamilyRef {
    /// Column family name.
    fn name(&self) -> &str;
}

impl AsColumnFamilyRef for ColumnFamily {
    fn name(&self) -> &str {
        ColumnFamily::name(self)
    }
}

impl AsColumnFamilyRef for &ColumnFamily {
    fn name(&self) -> &str {
        ColumnFamily::name(self)
    }
}

/// rust-rocksdb compaction-filter decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionDecision {
    /// Keep the key.
    Keep,
    /// Drop the key (tombstone).
    Remove,
    /// Replace the value.
    Change(Vec<u8>),
}

/// rust-rocksdb `IngestExternalFileOptions`.
#[derive(Debug, Clone, Default)]
pub struct IngestExternalFileOptions {
    /// Move (unlink source) instead of copy after ingest.
    pub move_files: bool,
}

impl IngestExternalFileOptions {
    /// Move files after a successful ingest.
    pub fn set_move_files(&mut self, v: bool) {
        self.move_files = v;
    }
    /// Snapshot consistency (Pedra ingest is a regular write — snapshots taken
    /// after Ok see the keys; earlier snapshots do not).
    pub fn set_snapshot_consistency(&mut self, _v: bool) {}
    /// Overlap with existing keys is allowed (seq-assigned puts).
    pub fn set_allow_global_seqno(&mut self, _v: bool) {}
    /// Blocking flush is implicit: ingest `write`s then `flush`es.
    pub fn set_allow_blocking_flush(&mut self, _v: bool) {}
    /// Ingest behind (not used).
    pub fn set_ingest_behind(&mut self, _v: bool) {}
}

/// rust-rocksdb `LiveFile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFile {
    /// Column-family name (`default` if unknown / mixed).
    pub column_family_name: String,
    /// File name (`NNNNNN.sst`).
    pub name: String,
    /// Size in bytes.
    pub size: usize,
    /// LSM level (0 if unknown).
    pub level: i32,
    /// Smallest user key (empty if unread).
    pub start_key: Vec<u8>,
    /// Largest user key (empty if unread).
    pub end_key: Vec<u8>,
    /// Entry count (0 if unread).
    pub num_entries: u64,
}

/// rust-rocksdb `DBPinnableSlice` — owned bytes, `Deref<[u8]>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DBPinnableSlice<'a> {
    bytes: Vec<u8>,
    _life: std::marker::PhantomData<&'a ()>,
}

impl DBPinnableSlice<'_> {
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            _life: std::marker::PhantomData,
        }
    }
}

impl Deref for DBPinnableSlice<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsRef<[u8]> for DBPinnableSlice<'_> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// rust-rocksdb `Cache` — byte budget for Pedra's SST block cache (RFC-0153).
#[derive(Debug, Clone, Default)]
pub struct Cache {
    cap: usize,
}

impl Cache {
    /// LRU cache of `size` bytes.
    #[must_use]
    pub fn new_lru_cache(size: usize) -> Self {
        Self { cap: size }
    }

    /// Capacity in bytes.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.cap
    }
}

/// rust-rocksdb `BlockBasedOptions`.
#[derive(Debug, Clone, Default)]
pub struct BlockBasedOptions {
    pub(crate) checksum: ChecksumType,
    pub(crate) block_cache_bytes: Option<u64>,
}

impl BlockBasedOptions {
    /// Block size.
    pub fn set_block_size(&mut self, _n: usize) {}
    /// Bloom bits per key.
    pub fn set_bloom_filter(&mut self, _bits: f64, _block_based: bool) {}
    /// Cache. Sizes Pedra's SST block cache (RFC-0153).
    pub fn set_block_cache(&mut self, c: &Cache) {
        self.block_cache_bytes = Some(c.capacity() as u64);
    }
    /// Index/filter in block cache.
    pub fn set_cache_index_and_filter_blocks(&mut self, _v: bool) {}
    /// Pin L0 index/filter.
    pub fn set_pin_l0_filter_and_index_blocks_in_cache(&mut self, _v: bool) {}
    /// Whole-key filtering.
    pub fn set_whole_key_filtering(&mut self, _v: bool) {}
    /// Format version.
    pub fn set_format_version(&mut self, _n: i32) {}
    /// Checksum. [`ChecksumType::NoChecksum`] is recorded so `DB::open` can
    /// refuse it (G2); other values stay inert (Pedra SST is CRC32C).
    pub fn set_checksum_type(&mut self, t: ChecksumType) {
        self.checksum = t;
    }
}

/// rust-rocksdb checksum type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChecksumType {
    /// No checksum (not used on Pedra SST — CRC is always on).
    NoChecksum,
    /// CRC32c (Pedra SST).
    #[default]
    CRC32c,
    /// xxHash.
    XXHash,
    /// xxHash64.
    XXHash64,
    /// XXH3.
    XXH3,
}

/// rust-rocksdb `SstFileWriter` — writes a Pedra SST that [`DB::ingest_external_file`] loads.
pub struct SstFileWriter {
    path: Option<PathBuf>,
    mem: MemTable,
    seq: u64,
}

impl SstFileWriter {
    /// Create a writer (options unused — Pedra SST layout is fixed).
    #[must_use]
    pub fn create(_opts: &Options) -> Self {
        Self {
            path: None,
            mem: MemTable::new(),
            seq: 1,
        }
    }

    /// Open `path` for writing.
    ///
    /// # Errors
    /// Never today (path is recorded; I/O happens at [`finish`](Self::finish)).
    pub fn open<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        self.path = Some(path.as_ref().to_path_buf());
        self.mem = MemTable::new();
        self.seq = 1;
        Ok(())
    }

    /// Put a user key.
    ///
    /// # Errors
    /// Writer not opened.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.ensure_open()?;
        let ik = InternalKey::new(
            bytes::Bytes::copy_from_slice(key.as_ref()),
            self.seq,
            ValueType::Value,
        );
        self.mem
            .insert(ik, bytes::Bytes::copy_from_slice(value.as_ref()));
        self.seq = self.seq.saturating_add(1);
        Ok(())
    }

    /// Delete a user key.
    ///
    /// # Errors
    /// Writer not opened.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.ensure_open()?;
        let ik = InternalKey::new(
            bytes::Bytes::copy_from_slice(key.as_ref()),
            self.seq,
            ValueType::Deletion,
        );
        self.mem.insert(ik, bytes::Bytes::new());
        self.seq = self.seq.saturating_add(1);
        Ok(())
    }

    /// Write the Pedra SST to the path given to [`open`](Self::open).
    ///
    /// # Errors
    /// SST I/O; writer not opened.
    pub fn finish(&mut self) -> Result<()> {
        let path = self
            .path
            .take()
            .ok_or_else(|| Error::invalid("SstFileWriter::finish without open"))?;
        write_sst(&path, &self.mem).map_err(Error::from)?;
        self.mem = MemTable::new();
        Ok(())
    }

    /// Bytes in the in-progress file (approx memtable).
    #[must_use]
    pub fn file_size(&self) -> u64 {
        self.mem.approx_memory_usage() as u64
    }

    fn ensure_open(&self) -> Result<()> {
        if self.path.is_some() {
            Ok(())
        } else {
            Err(Error::invalid("SstFileWriter not opened"))
        }
    }
}

/// rust-rocksdb / TiKV `WriteBatchWithIndex` — last-write-wins overlay for
/// read-your-writes before `DB::write`.
///
/// RFC-0217 P1.3: the index is a flat staging vec of `Bytes` handles
/// sharing the CF `Arc` — a staged op allocates only its key/value copies
/// (no per-op `String` cf, no `BTreeMap` node, no lookup tuple). Lookups
/// scan newest-first; a `(cf, key)`-sorted view is materialized lazily on
/// the first lookup of a large batch (write-only batches never build it)
/// and reset by any later staged op.
#[derive(Debug, Default)]
pub struct WriteBatchWithIndex {
    batch: WriteBatch,
    /// Staged overlay in insertion order; lookup = newest-first scan.
    overlay: Vec<OverlayOp>,
    /// Lazily materialized `(cf, key)`-sorted view of `overlay` indices
    /// for batches above [`Self::SORT_MAX`]. `RefCell`: `get_from_batch`
    /// stays `&self` (rust-rocksdb shape) while the first big lookup pays
    /// the one-time sort.
    sorted: std::cell::RefCell<Option<Vec<u32>>>,
}

/// One staged overlay op: `Some(value)` put or `None` delete.
#[derive(Debug)]
struct OverlayOp {
    cf: Arc<str>,
    key: Bytes,
    val: Option<Bytes>,
}

impl WriteBatchWithIndex {
    /// Above this many staged ops, lookups switch to the sorted view.
    const SORT_MAX: usize = 32;

    /// Empty indexed batch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Put default CF.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.put_cf_arc(&crate::DEFAULT_CF_ARC, key, value);
    }

    /// Put named CF.
    pub fn put_cf(&mut self, cf: &ColumnFamily, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.put_cf_arc(&cf.name, key, value);
    }

    fn put_cf_arc(&mut self, cf: &Arc<str>, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        let k = Bytes::copy_from_slice(key.as_ref());
        let v = Bytes::copy_from_slice(value.as_ref());
        self.batch.put_cf_bytes(cf, k.clone(), v.clone());
        self.overlay.push(OverlayOp {
            cf: Arc::clone(cf),
            key: k,
            val: Some(v),
        });
        *self.sorted.borrow_mut() = None;
    }

    /// Delete default CF.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) {
        self.delete_cf_arc(&crate::DEFAULT_CF_ARC, key);
    }

    /// Delete named CF.
    pub fn delete_cf(&mut self, cf: &ColumnFamily, key: impl AsRef<[u8]>) {
        self.delete_cf_arc(&cf.name, key);
    }

    fn delete_cf_arc(&mut self, cf: &Arc<str>, key: impl AsRef<[u8]>) {
        let k = Bytes::copy_from_slice(key.as_ref());
        self.batch.delete_cf_bytes(cf, k.clone());
        self.overlay.push(OverlayOp {
            cf: Arc::clone(cf),
            key: k,
            val: None,
        });
        *self.sorted.borrow_mut() = None;
    }

    /// Number of indexed ops (last-write-wins keys).
    #[must_use]
    pub fn len(&self) -> usize {
        self.batch.len()
    }

    /// Empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        pedradb_core::write_admission_kernel::batch_is_empty(self.batch.len() as u64)
    }

    /// Underlying atomic batch for [`DB::write`].
    #[must_use]
    pub fn get_write_batch(&self) -> &WriteBatch {
        &self.batch
    }

    /// Read the batch only. `Ok(None)` = key not in the batch.
    /// Inner `None` = deleted in the batch.
    pub fn get_from_batch(&self, key: impl AsRef<[u8]>) -> Option<Option<Vec<u8>>> {
        self.get_from_batch_parts(DEFAULT_CF, key.as_ref())
    }

    /// Read the batch only for a CF.
    pub fn get_from_batch_cf(
        &self,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
    ) -> Option<Option<Vec<u8>>> {
        self.get_from_batch_parts(cf.name.as_ref(), key.as_ref())
    }

    fn get_from_batch_parts(&self, cf: &str, key: &[u8]) -> Option<Option<Vec<u8>>> {
        if self.overlay.len() > Self::SORT_MAX {
            return self
                .sorted_lookup(cf, key)
                .map(|op| op.val.clone().map(|b| b.to_vec()));
        }
        self.overlay
            .iter()
            .rev()
            .find(|op| op.cf.as_ref() == cf && op.key.as_ref() == key)
            .map(|op| op.val.clone().map(|b| b.to_vec()))
    }

    /// Binary-search lookup through the lazily built sorted view. Equal
    /// `(cf, key)` entries keep ascending index order, so the rightmost
    /// match is the newest write.
    fn sorted_lookup(&self, cf: &str, key: &[u8]) -> Option<&OverlayOp> {
        let mut sorted = self.sorted.borrow_mut();
        let sorted = sorted.get_or_insert_with(|| {
            let mut idx: Vec<u32> = (0..self.overlay.len() as u32).collect();
            idx.sort_by(|&a, &b| {
                let (x, y) = (&self.overlay[a as usize], &self.overlay[b as usize]);
                x.cf.as_ref()
                    .cmp(y.cf.as_ref())
                    .then_with(|| x.key.cmp(&y.key))
            });
            idx
        });
        let pos = sorted.partition_point(|&i| {
            let op = &self.overlay[i as usize];
            op.cf
                .as_ref()
                .cmp(cf)
                .then_with(|| op.key.as_ref().cmp(key))
                == std::cmp::Ordering::Less
        });
        // Equal `(cf, key)` entries sit in ascending index order; the
        // rightmost is the newest write.
        let mut end = pos;
        while sorted
            .get(end)
            .map(|&i| {
                let op = &self.overlay[i as usize];
                op.cf.as_ref() == cf && op.key.as_ref() == key
            })
            .unwrap_or(false)
        {
            end += 1;
        }
        sorted
            .get(end.wrapping_sub(1))
            .filter(|&&i| end > pos)
            .map(|&i| &self.overlay[i as usize])
    }

    /// Overlay then DB (raftstore apply path).
    ///
    /// # Errors
    /// Pedra get errors.
    pub fn get_from_batch_and_db<E: Env>(
        &self,
        db: &DB<E>,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        match self.get_from_batch(key.as_ref()) {
            Some(v) => Ok(v),
            None => db.get(key),
        }
    }

    /// Overlay then DB for a CF.
    ///
    /// # Errors
    /// Pedra get errors.
    pub fn get_from_batch_and_db_cf<E: Env>(
        &self,
        db: &DB<E>,
        cf: &ColumnFamily,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        match self.get_from_batch_cf(cf, key.as_ref()) {
            Some(v) => Ok(v),
            None => db.get_cf(cf, key),
        }
    }
}

/// Merge operands passed to a full-merge callback.
pub struct MergeOperands {
    ops: Vec<Vec<u8>>,
}

impl MergeOperands {
    pub(crate) fn one(op: Vec<u8>) -> Self {
        Self { ops: vec![op] }
    }

    /// Operand count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        pedradb_core::write_admission_kernel::batch_is_empty(self.ops.len() as u64)
    }

    /// Iterate operand slices.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.ops.iter().map(Vec::as_slice)
    }
}

/// Open a Pedra SST produced by [`SstFileWriter`].
pub(crate) fn open_writer_sst(path: &Path) -> Result<SstTable> {
    SstTable::open(path).map_err(Error::from)
}

impl Error {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            kind: ErrorKind::InvalidArgument,
        }
    }

    pub(crate) fn not_supported(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            kind: ErrorKind::NotSupported,
        }
    }
}
