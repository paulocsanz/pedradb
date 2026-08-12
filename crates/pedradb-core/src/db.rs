//! Single-process database handle: WAL + MemTable + SSTs, recover on open.
//!
//! Auto-commit put/delete/get (P0.3), multi-key [`Transaction`](crate::tx::Transaction)
//! (P0.4), and MemTable → SST flush (P1.1).
//!
//! # Durability (P0.5)
//!
//! With default [`OpenOptions::sync`] = `true`:
//!
//! - Successful [`Db::put`], [`Db::delete`], and [`Transaction::commit`](crate::tx::Transaction::commit)
//!   return only after the WAL record is appended and **`fsync`/`sync_all` completes**.
//! - After that `Ok`, a **process crash** (kill -9) must not lose the write if the OS
//!   and disk honor fsync. Reopen replays complete WAL records into the MemTable and
//!   loads SST files.
//! - A crash **during** append may leave a truncated trailing record; recovery **skips**
//!   it (no partial TX visible). Multi-key commit is one WAL record → all-or-nothing.
//! - Uncommitted transactions leave no WAL record (drop/abort = no durability side effect).
//! - [`OpenOptions::sync`] = `false` is for bulk load/benches only: process crash may
//!   still retain OS-buffered data; **power loss can lose recent acks** (JetStream/Jepsen lesson).
//!
//! # Flush (P1.1)
//!
//! [`Db::flush`] writes the MemTable to a new `.sst`, fsyncs it, clears the MemTable,
//! and truncates the WAL (data now lives on SST). `get` merges MemTable ∪ SSTs
//! (newest layer first).
//!
//! # Range, compaction & GC (RFC-0009 P1)
//!
//! [`Db::range`] scans user keys across MemTable ∪ SSTs at a snapshot.
//! [`Db::compact`] / [`Db::compact_with`] merge SSTs (tmp → rename); optional
//! version GC via [`CompactOptions`]. [`OpenOptions::auto_compact_sst_count`]
//! triggers compact after flush when SST count is high.
//!
//! # MANIFEST & exclusive open (RFC-0009 P2)
//!
//! Live SST inventory is written to `MANIFEST-*` + `CURRENT` after each flush/compact.
//! [`OpenOptions::exclusive`] (default true) takes a PID `LOCK` file so a second
//! process cannot open the same directory.

use std::ops::Bound;
use std::path::{Path, PathBuf};

use bytes::Bytes;

use crate::batch::{WriteOp, WriteRecord};
use crate::env::{Env, StdEnv};
use crate::error::{CoreError, Result};
use crate::key::{InternalKey, SequenceNumber, ValueType, MAX_SEQUENCE_NUMBER};
use crate::lock::DirLock;
use crate::manifest::{self, VersionSet};
use crate::memtable::{Lookup, MemTable};
use crate::merge::{visible_range, VisibleKv};
use crate::sst::{write_sst_entries_on, write_sst_on, SstTable};
use crate::tx::Transaction;
use crate::wal::Wal;

/// Default WAL file name inside the DB directory.
pub const WAL_FILE_NAME: &str = "CURRENT.log";

/// Options for [`Db::open`].
#[derive(Debug, Clone, Copy)]
pub struct OpenOptions {
    /// When true (default), each successful `put`/`delete`/`commit` syncs the WAL
    /// before returning (RFC-0001 O1). Overridable per write via [`WriteOptions`].
    pub sync: bool,
    /// When MemTable approximate size reaches this many bytes, flush to SST.
    /// `None` or `0` disables auto-flush (manual [`Db::flush`] only).
    pub auto_flush_bytes: Option<usize>,
    /// After a flush, if SST count is ≥ this, run [`Db::compact`].
    /// `None` or `0` disables auto-compact.
    pub auto_compact_sst_count: Option<usize>,
    /// When true (default), acquire exclusive `LOCK` in the DB directory.
    pub exclusive: bool,
}

/// Per-write durability / batching knobs (RFC-0009 P0.1).
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// If set, overrides [`OpenOptions::sync`] for this write.
    /// `false` = WAL write without fsync (group with later [`Db::sync`]).
    pub sync: Option<bool>,
}

impl WriteOptions {
    /// Use database default sync policy.
    #[must_use]
    pub fn default_sync() -> Self {
        Self::default()
    }

    /// Force fsync after this write.
    #[must_use]
    pub fn sync() -> Self {
        Self { sync: Some(true) }
    }

    /// Skip fsync (caller must [`Db::sync`] for durability).
    #[must_use]
    pub fn no_sync() -> Self {
        Self { sync: Some(false) }
    }
}

/// One operation in an ordered external apply batch (P2.3 — no OCC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchOp {
    /// Put `key → value`.
    Put {
        /// User key bytes.
        key: Bytes,
        /// Value bytes.
        value: Bytes,
    },
    /// Delete `key`.
    Delete {
        /// User key bytes.
        key: Bytes,
    },
}

impl BatchOp {
    /// Put helper.
    #[must_use]
    pub fn put(key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        Self::Put {
            key: Bytes::copy_from_slice(key.as_ref()),
            value: Bytes::copy_from_slice(value.as_ref()),
        }
    }

    /// Delete helper.
    #[must_use]
    pub fn delete(key: impl AsRef<[u8]>) -> Self {
        Self::Delete {
            key: Bytes::copy_from_slice(key.as_ref()),
        }
    }
}

/// Read snapshot: sequence number visible to get/range (P2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Snapshot {
    /// Highest committed sequence included in this snapshot.
    seq: SequenceNumber,
}

impl Snapshot {
    /// Sequence this snapshot pins.
    #[must_use]
    pub fn sequence(self) -> SequenceNumber {
        self.seq
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            sync: true,
            // 4 MiB default encourages SST creation under load without manual flush.
            auto_flush_bytes: Some(4 * 1024 * 1024),
            auto_compact_sst_count: None,
            exclusive: true,
        }
    }
}

/// Options for [`Db::compact_with`] (RFC-0009 compaction / version GC).
#[derive(Debug, Clone, Copy, Default)]
pub struct CompactOptions {
    /// Version GC policy applied while rewriting SSTs.
    pub gc: crate::merge::CompactGcOptions,
}

impl CompactOptions {
    /// Compact keeping only the newest version of each user key.
    #[must_use]
    pub fn latest_only() -> Self {
        Self {
            gc: crate::merge::CompactGcOptions::latest_only(),
        }
    }
}

/// Embedded database: one process, one directory.
///
/// Generic over [`Env`] so tests can inject disk faults (see `pedradb-sim::FailingEnv`).
/// Production default is [`StdEnv`].
pub struct Db<E: Env = StdEnv> {
    dir: PathBuf,
    env: E,
    wal: Wal<E::File>,
    mem: MemTable,
    /// Immutable tables, oldest → newest (get scans newest first).
    ssts: Vec<SstTable>,
    /// Next SST file number (`000001.sst`, …).
    next_file_num: u64,
    /// Last written MANIFEST file number (0 = none yet).
    manifest_file_num: u64,
    /// Next sequence to assign (1-based; 0 means “no writes yet”).
    next_seq: SequenceNumber,
    sync: bool,
    auto_flush_bytes: Option<usize>,
    auto_compact_sst_count: Option<usize>,
    /// Exclusive directory lock (dropped on close / drop).
    _lock: Option<DirLock>,
}

impl Db<StdEnv> {
    /// Open or create a database at `path` (directory) on the real filesystem.
    ///
    /// Loads `*.sst` files, then recovers the WAL into the MemTable.
    ///
    /// # Errors
    /// I/O failures, corrupt logical records, or CRC errors on non-truncated data.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Open with explicit options on the real filesystem.
    ///
    /// # Errors
    /// Same as [`Self::open`].
    pub fn open_with(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        Self::open_with_env(path, opts, StdEnv)
    }
}

impl<E: Env> Db<E> {
    /// Open with an explicit [`Env`] (fault injection, in-memory, …).
    ///
    /// # Errors
    /// I/O failures, corrupt logical records, CRC errors, [`CoreError::AlreadyOpen`],
    /// or corrupt MANIFEST.
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        let dir = path.as_ref().to_path_buf();
        env.create_dir_all(&dir)?;

        let lock = if opts.exclusive {
            Some(DirLock::acquire(&env, &dir)?)
        } else {
            None
        };

        manifest::cleanup_tmp_files(&env, &dir)?;

        let (ssts, next_file_num, manifest_file_num, mut max_seq) =
            recover_ssts(&env, &dir, opts.sync)?;

        let wal_path = dir.join(WAL_FILE_NAME);
        let mut mem = MemTable::new();

        if env.exists(&wal_path) {
            // Tiny WAL + Truncated(0): failed first append after rotate (or crash
            // before any complete record). Tolerate empty so SSTs still load (F6).
            // Large WAL + Truncated(0): bitrot of the first record — fail-stop (F4).
            let records = match Wal::recover_on(&env, &wal_path) {
                Ok(r) => r,
                Err(CoreError::Truncated(0)) => {
                    let len = env.metadata_len(&wal_path).unwrap_or(0);
                    if len < 64 {
                        Vec::new()
                    } else {
                        return Err(CoreError::Truncated(0));
                    }
                }
                Err(e) => return Err(e),
            };
            for raw in records {
                let rec = WriteRecord::decode(&raw)?;
                apply_record(&mut mem, &rec);
                if let Some(s) = rec.max_sequence() {
                    max_seq = max_seq.max(s);
                }
            }
        }

        let wal = if env.exists(&wal_path) {
            Wal::append_on(&env, &wal_path)?
        } else {
            Wal::create_on(&env, &wal_path)?
        };

        let next_seq = max_seq.saturating_add(1).max(1);
        if next_seq > MAX_SEQUENCE_NUMBER {
            return Err(CoreError::Internal("sequence number space exhausted".into()));
        }

        Ok(Self {
            dir,
            env,
            wal,
            mem,
            ssts,
            next_file_num,
            manifest_file_num,
            next_seq,
            sync: opts.sync,
            auto_flush_bytes: opts.auto_flush_bytes.filter(|n| *n > 0),
            auto_compact_sst_count: opts.auto_compact_sst_count.filter(|n| *n > 0),
            _lock: lock,
        })
    }

    /// Directory this DB was opened on.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Latest sequence that has been written (0 if empty).
    #[must_use]
    pub fn last_sequence(&self) -> SequenceNumber {
        self.next_seq.saturating_sub(1)
    }

    /// Number of SST files currently loaded.
    #[must_use]
    pub fn sst_count(&self) -> usize {
        self.ssts.len()
    }

    /// Capture a read snapshot of currently committed state (sequence export).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.last_sequence(),
        }
    }

    /// Point lookup at the latest committed sequence (MemTable ∪ SSTs).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.get_at(self.snapshot(), key)
    }

    /// Point lookup at an explicit [`Snapshot`].
    #[must_use]
    pub fn get_at(&self, snap: Snapshot, key: &[u8]) -> Option<Bytes> {
        if snap.seq == 0 {
            return None;
        }
        match self.lookup(key, snap.seq) {
            Lookup::Found(v) => Some(v),
            Lookup::Deleted | Lookup::NotFound => None,
        }
    }

    /// Range scan at the latest committed snapshot over MemTable ∪ SSTs.
    ///
    /// Yields `(user_key, value)` in ascending user-key order. Only the newest
    /// non-deleted version per key with `sequence <= last_sequence` is returned.
    #[must_use]
    pub fn range(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<(Bytes, Bytes)> {
        self.range_at(self.last_sequence(), start, end)
    }

    /// Range scan at an explicit snapshot sequence.
    #[must_use]
    pub fn range_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<(Bytes, Bytes)> {
        if snapshot == 0 {
            return Vec::new();
        }
        let mut entries: Vec<(InternalKey, Bytes)> = self
            .mem
            .iter_internal()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for table in &self.ssts {
            entries.extend(table.entries_cloned());
        }
        visible_range(entries, snapshot, start, end)
            .into_iter()
            .map(|VisibleKv { key, value }| (key, value))
            .collect()
    }

    /// Flush MemTable to a new SST, fsync, clear mem, rotate WAL.
    ///
    /// No-op if the MemTable is empty. May trigger auto-compact when SST count
    /// exceeds [`OpenOptions::auto_compact_sst_count`].
    ///
    /// # Errors
    /// I/O while writing SST or recreating the WAL.
    pub fn flush(&mut self) -> Result<()> {
        if self.mem.is_empty() {
            return Ok(());
        }

        let num = self.next_file_num;
        let final_path = self.dir.join(format!("{num:06}.sst"));
        let tmp_path = self.dir.join(format!("{num:06}.sst.tmp"));
        // Crash/fault-safe (F1): never leave a partial final `*.sst` that would
        // make the next open fail before WAL recovery. Same pattern as compact.
        match write_sst_on(&self.env, &tmp_path, &self.mem) {
            Ok(table) => {
                // `write_sst_on` opens the path it wrote; re-open after rename.
                drop(table);
                self.env.rename(&tmp_path, &final_path)?;
                if self.sync {
                    let _ = self.env.sync_dir(&self.dir);
                }
                let table = SstTable::open_on(&self.env, &final_path)?;
                self.next_file_num = num + 1;
                self.ssts.push(table);
            }
            Err(e) => {
                let _ = self.env.remove_file(&tmp_path);
                // Do not leave a half-written final name if a prior path used one.
                let _ = self.env.remove_file(&final_path);
                return Err(e);
            }
        }
        // Inventory must land before WAL rotate; otherwise a crash would drop the
        // new SST as an orphan while the WAL no longer holds the data.
        // F21: if MANIFEST install fails, roll back in-memory inventory so a
        // retry flush does not stack duplicate SSTs while mem still holds data.
        if let Err(e) = self.persist_manifest() {
            let _ = self.ssts.pop();
            self.next_file_num = num;
            return Err(e);
        }

        // Data is on SST; drop mem and start a fresh WAL.
        self.mem = MemTable::new();
        let wal_path = self.dir.join(WAL_FILE_NAME);
        // Close old WAL by replacing handle.
        let old = std::mem::replace(&mut self.wal, Wal::create_on(&self.env, &wal_path)?);
        old.close()?;
        if self.sync {
            // Ensure directory entries are durable (best-effort on platforms that support it).
            let _ = self.env.sync_dir(&self.dir);
        }
        // Auto-compact is opportunistic; flush already installed SST+MANIFEST+WAL rotate.
        // Do not fail the flush if compact I/O faults (same spirit as F18).
        let _ = self.maybe_auto_compact();
        Ok(())
    }

    /// Compact all SST files into a single SST (whole-merge strategy).
    ///
    /// Flushes the MemTable first. Writes to a `.sst.tmp` then renames (crash-safe).
    /// Uses default GC (keep all versions still present).
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact(&mut self) -> Result<()> {
        self.compact_with(CompactOptions::default())
    }

    /// Compact with version GC options (RFC-0009 P1.3).
    ///
    /// # Errors
    /// I/O while writing the compacted SST or deleting old files.
    pub fn compact_with(&mut self, options: CompactOptions) -> Result<()> {
        self.flush()?;
        if self.ssts.is_empty() {
            return Ok(());
        }
        // Single SST still benefits from GC rewrite.
        if self.ssts.len() == 1 && !options.gc.keep_only_latest && options.gc.min_sequence == 0 {
            return Ok(());
        }

        let mut merged: Vec<(InternalKey, Bytes)> = Vec::new();
        for table in &self.ssts {
            merged.extend(table.entries_cloned());
        }
        let merged = crate::merge::gc_compact_entries(merged, options.gc);

        let num = self.next_file_num;
        let final_path = self.dir.join(format!("{num:06}.sst"));
        let tmp_path = self.dir.join(format!("{num:06}.sst.tmp"));
        // Crash-safe: write temp, sync, rename, then drop old files.
        let new_table = write_sst_entries_on(&self.env, &tmp_path, &merged)?;
        // Path stored in table is the tmp path; re-open after rename.
        drop(new_table);
        self.env.rename(&tmp_path, &final_path)?;
        if self.sync {
            let _ = self.env.sync_dir(&self.dir);
        }
        let new_table = SstTable::open_on(&self.env, &final_path)?;
        self.next_file_num = num + 1;

        let old_paths: Vec<PathBuf> = self.ssts.iter().map(|t| t.path().to_path_buf()).collect();
        self.ssts = vec![new_table];
        // Swing MANIFEST to the single live SST before deleting inputs.
        self.persist_manifest()?;

        for path in old_paths {
            if path != final_path {
                let _ = self.env.remove_file(&path);
            }
        }
        Ok(())
    }

    /// Begin a multi-key write transaction (single-writer: exclusive `&mut self`).
    pub fn begin(&mut self) -> Transaction<'_, E> {
        Transaction::new(self)
    }

    /// Put `key → value` (auto-commit, one sequence).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_with(key, value, WriteOptions::default())
    }

    /// Put with explicit [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<()> {
        self.apply_batch_with([BatchOp::put(key, value)], opts)?;
        Ok(())
    }

    /// Delete `key` (auto-commit tombstone).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_with(key, WriteOptions::default())
    }

    /// Delete with explicit [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete_with(
        &mut self,
        key: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<()> {
        self.apply_batch_with([BatchOp::delete(key)], opts)?;
        Ok(())
    }

    /// Apply an ordered multi-op batch atomically (one WAL record, no OCC).
    ///
    /// For Raft/log apply and bulk import: sequences are assigned in order;
    /// either the whole batch is durable on success or none of it is visible
    /// after recovery.
    ///
    /// Returns the sequence of the last op in the batch (or current
    /// `last_sequence` if the batch is empty).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch(
        &mut self,
        ops: impl IntoIterator<Item = BatchOp>,
    ) -> Result<SequenceNumber> {
        self.apply_batch_with(ops, WriteOptions::default())
    }

    /// [`apply_batch`](Self::apply_batch) with [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch_with(
        &mut self,
        batch: impl IntoIterator<Item = BatchOp>,
        durability: WriteOptions,
    ) -> Result<SequenceNumber> {
        let mut records = Vec::new();
        for op in batch {
            let seq = self.alloc_seq()?;
            match op {
                BatchOp::Put { key, value } => {
                    records.push(WriteOp::put(seq, key, value));
                }
                BatchOp::Delete { key } => {
                    records.push(WriteOp::delete(seq, key));
                }
            }
        }
        if records.is_empty() {
            return Ok(self.last_sequence());
        }
        self.commit_ops_with(records, durability)?;
        // F18: the write is already durable (WAL fsync under sync=true). Auto-flush
        // is a background space concern — failing it must not surface as "put/commit
        // failed" or clients will retry and the operator loses the success signal.
        self.maybe_auto_flush_best_effort();
        Ok(self.last_sequence())
    }

    /// Flush WAL according to open options (for tests / graceful shutdown).
    ///
    /// After a series of `WriteOptions::no_sync()` writes, call this to make
    /// them durable (group fsync).
    ///
    /// # Errors
    /// I/O from fsync.
    pub fn sync(&mut self) -> Result<()> {
        self.wal.sync_all()
    }

    /// Close the WAL (flush). Prefer dropping after `sync` if durability matters.
    ///
    /// # Errors
    /// I/O from flush.
    pub fn close(self) -> Result<()> {
        self.wal.close()
    }

    /// Lookup visible version at `snapshot` across mem + SSTs.
    pub(crate) fn lookup(&self, key: &[u8], snapshot: SequenceNumber) -> Lookup {
        match self.mem.get(key, snapshot) {
            Lookup::NotFound => {}
            other => return other,
        }
        // Newest SST first (higher file numbers / later flushes).
        for table in self.ssts.iter().rev() {
            match table.get(key, snapshot) {
                Lookup::NotFound => {}
                other => return other,
            }
        }
        Lookup::NotFound
    }

    pub(crate) fn alloc_seq(&mut self) -> Result<SequenceNumber> {
        let seq = self.next_seq;
        if seq > MAX_SEQUENCE_NUMBER {
            return Err(CoreError::Internal("sequence number space exhausted".into()));
        }
        self.next_seq = seq + 1;
        Ok(seq)
    }

    pub(crate) fn commit_ops_with(
        &mut self,
        records: Vec<WriteOp>,
        durability: WriteOptions,
    ) -> Result<()> {
        let rec = WriteRecord { ops: records };
        self.wal.append_record(&rec.encode())?;
        let do_sync = durability.sync.unwrap_or(self.sync);
        if do_sync {
            self.wal.sync_all()?;
        }
        apply_record(&mut self.mem, &rec);
        Ok(())
    }

    pub(crate) fn maybe_auto_flush(&mut self) -> Result<()> {
        let Some(limit) = self.auto_flush_bytes else {
            return Ok(());
        };
        if self.mem.approx_memory_usage() >= limit {
            self.flush()?;
        }
        Ok(())
    }

    /// Like [`maybe_auto_flush`] but never fails the caller (F18).
    ///
    /// On flush error the MemTable still holds the data; a later explicit
    /// [`Db::flush`] or successful auto-flush can retry.
    pub(crate) fn maybe_auto_flush_best_effort(&mut self) {
        let _ = self.maybe_auto_flush();
    }

    fn maybe_auto_compact(&mut self) -> Result<()> {
        let Some(limit) = self.auto_compact_sst_count else {
            return Ok(());
        };
        if self.ssts.len() >= limit {
            // F20: do **not** use latest_only here — that dropped historical versions
            // and broke `get_at` / Snapshot for sequences still "open" in the app.
            // Space-bound GC is explicit: `compact_with(CompactOptions::latest_only())`.
            self.compact_with(CompactOptions::default())?;
        }
        Ok(())
    }

    /// Write MANIFEST + CURRENT for the live SST set.
    fn persist_manifest(&mut self) -> Result<()> {
        let mut nums = Vec::with_capacity(self.ssts.len());
        for table in &self.ssts {
            let name = table
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| CoreError::Internal("sst path missing file name".into()))?;
            let num = manifest::parse_sst_name(name)
                .ok_or_else(|| CoreError::Internal(format!("bad sst name {name}")))?;
            nums.push(num);
        }
        let mut vs = VersionSet {
            next_file_num: self.next_file_num,
            sst_file_nums: nums,
            manifest_file_num: self.manifest_file_num,
        };
        manifest::install_next(&self.env, &self.dir, &mut vs, self.sync)?;
        self.manifest_file_num = vs.manifest_file_num;
        Ok(())
    }
}

fn apply_record(mem: &mut MemTable, rec: &WriteRecord) {
    for op in &rec.ops {
        match op.kind {
            ValueType::Value => {
                mem.put(op.key.clone(), op.sequence, op.value.clone());
            }
            ValueType::Deletion => {
                mem.delete(op.key.clone(), op.sequence);
            }
        }
    }
}

/// Recover SST tables from MANIFEST when present, else directory scan (legacy).
///
/// Returns `(tables, next_file_num, manifest_file_num, max_sequence)`.
fn recover_ssts<E: Env>(
    env: &E,
    dir: &Path,
    sync: bool,
) -> Result<(Vec<SstTable>, u64, u64, SequenceNumber)> {
    if let Some(vs) = manifest::load(env, dir)? {
        // Drop SST files not listed (mid-compact / failed flush orphans).
        manifest::gc_orphan_ssts(env, dir, &vs.sst_file_nums)?;
        let mut max_seq = 0;
        let mut tables = Vec::with_capacity(vs.sst_file_nums.len());
        for num in &vs.sst_file_nums {
            let path = VersionSet::sst_path(dir, *num);
            if !env.exists(&path) {
                return Err(CoreError::CorruptManifest(format!(
                    "MANIFEST lists missing SST {num:06}.sst"
                )));
            }
            let t = SstTable::open_on(env, &path)?;
            max_seq = max_seq.max(t.max_sequence());
            tables.push(t);
        }
        return Ok((tables, vs.next_file_num, vs.manifest_file_num, max_seq));
    }

    // Legacy / first open: scan directory, then write initial MANIFEST.
    let (tables, next_file_num, max_seq) = load_ssts_scan(env, dir)?;
    let mut vs = VersionSet {
        next_file_num,
        sst_file_nums: tables
            .iter()
            .filter_map(|t| {
                t.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(manifest::parse_sst_name)
            })
            .collect(),
        manifest_file_num: 0,
    };
    // Always install so subsequent opens use inventory (even if empty).
    manifest::install_next(env, dir, &mut vs, sync)?;
    Ok((tables, next_file_num, vs.manifest_file_num, max_seq))
}

/// Load `NNNNNN.sst` files ascending; return tables, next file num, max sequence.
fn load_ssts_scan<E: Env>(
    env: &E,
    dir: &Path,
) -> Result<(Vec<SstTable>, u64, SequenceNumber)> {
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    if env.exists(dir) {
        for name in env.read_dir_names(dir)? {
            if let Some(num) = manifest::parse_sst_name(&name) {
                files.push((num, dir.join(name)));
            }
        }
    }
    files.sort_by_key(|(n, _)| *n);
    let next_file_num = files.last().map_or(1, |(n, _)| n + 1);
    let mut max_seq = 0;
    let mut tables = Vec::with_capacity(files.len());
    for (_, path) in files {
        let t = SstTable::open_on(env, path)?;
        max_seq = max_seq.max(t.max_sequence());
        tables.push(t);
    }
    Ok((tables, next_file_num, max_seq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-db-test-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn put_get_delete() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"v1").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v1".as_ref()));
        db.put(b"k", b"v2").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v2".as_ref()));
        db.delete(b"k").unwrap();
        assert_eq!(db.get(b"k"), None);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopen_recovers_puts() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.close().unwrap();
        }
        {
            let db = Db::open(&dir).unwrap();
            assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
            assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
            assert_eq!(db.last_sequence(), 2);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopen_recovers_delete() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.delete(b"k").unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k"), None);
        assert_eq!(db.last_sequence(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_after_reopen_continues_sequence() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.close().unwrap();
        }
        {
            let mut db = Db::open(&dir).unwrap();
            assert_eq!(db.last_sequence(), 1);
            db.put(b"b", b"2").unwrap();
            assert_eq!(db.last_sequence(), 2);
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_mode_still_recovers_after_clean_close() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    exclusive: true,
                },
            )
            .unwrap();
            db.put(b"x", b"y").unwrap();
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"x").as_deref(), Some(b"y".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_after_sync_put_reopen_recovers() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"durable", b"yes").unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"durable").as_deref(), Some(b"yes".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_after_multi_key_commit_reopen_recovers_both() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            let mut tx = db.begin();
            tx.put(b"row", b"R").unwrap();
            tx.put(b"idx", b"I").unwrap();
            tx.commit().unwrap();
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uncommitted_tx_not_on_disk_after_crash() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"keep", b"1").unwrap();
            let mut tx = db.begin();
            tx.put(b"ghost", b"2").unwrap();
            std::mem::forget(tx);
            std::mem::forget(db);
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"ghost"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_get_from_sst() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        assert_eq!(db.sst_count(), 0);
        db.flush().unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_reopen_loads_sst_without_wal_data() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        // WAL should be empty; data only in SST
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.last_sequence(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_overwrite_and_reopen() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"old").unwrap();
            db.flush().unwrap();
            db.put(b"k", b"new").unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_then_delete_hides_sst_value() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"v").unwrap();
        db.flush().unwrap();
        db.delete(b"k").unwrap();
        assert_eq!(db.get(b"k"), None);
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_flushes_reopen() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.flush().unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            assert_eq!(db.sst_count(), 2);
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 2);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tx_reads_from_sst_after_flush() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"row", b"R").unwrap();
        db.flush().unwrap();
        {
            let mut tx = db.begin();
            assert_eq!(tx.get(b"row").as_deref(), Some(b"R".as_ref()));
            tx.put(b"idx", b"I").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn range_after_flush_ordered_mvcc() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.put(b"c", b"3").unwrap();
        db.put(b"d", b"4").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2b").unwrap(); // newer in mem
        db.delete(b"c").unwrap();

        let mid = db.range(Bound::Included(b"b"), Bound::Excluded(b"d"));
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].0.as_ref(), b"b");
        assert_eq!(mid[0].1.as_ref(), b"2b");

        let all: Vec<_> = db
            .range(Bound::Unbounded, Bound::Unbounded)
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert_eq!(all, vec![b"a".to_vec(), b"b".to_vec(), b"d".to_vec()]);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_merges_ssts_preserves_live_keys() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.flush().unwrap();
            db.put(b"b", b"2").unwrap();
            db.flush().unwrap();
            db.put(b"a", b"1b").unwrap();
            db.flush().unwrap();
            assert!(db.sst_count() >= 2);
            let before: Vec<_> = db.range(Bound::Unbounded, Bound::Unbounded);
            db.compact().unwrap();
            assert_eq!(db.sst_count(), 1);
            let after: Vec<_> = db.range(Bound::Unbounded, Bound::Unbounded);
            assert_eq!(before, after);
            assert_eq!(db.get(b"a").as_deref(), Some(b"1b".as_ref()));
            assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1b".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let keys: Vec<_> = db
            .range(Bound::Unbounded, Bound::Unbounded)
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_flush_when_mem_exceeds_threshold() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: Some(200),
                auto_compact_sst_count: None,
                exclusive: true,
            },
        )
        .unwrap();
        // Each put ~ key+value+8; a few large values should trip flush.
        for i in 0..20u8 {
            let val = vec![i; 64];
            db.put([b'k', i], &val).unwrap();
        }
        assert!(
            db.sst_count() >= 1,
            "expected auto-flush to create at least one SST"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_sync_then_sync_groups_durability() {
        let dir = temp_dir();
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    exclusive: true,
                },
            )
            .unwrap();
            for i in 0..10u8 {
                db.put_with([b'x', i], b"v", WriteOptions::no_sync())
                    .unwrap();
            }
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        for i in 0..10u8 {
            assert_eq!(db.get(&[b'x', i]).as_deref(), Some(b"v".as_ref()));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_compact_when_sst_count_reaches_threshold() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(3),
                exclusive: true,
            },
        )
        .unwrap();
        for i in 0..3u8 {
            db.put([b'k', i], b"v").unwrap();
            db.flush().unwrap();
        }
        // Third flush should have triggered auto-compact → single SST.
        assert_eq!(
            db.sst_count(),
            1,
            "auto-compact should merge when count >= threshold"
        );
        for i in 0..3u8 {
            assert_eq!(db.get(&[b'k', i]).as_deref(), Some(b"v".as_ref()));
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F20: auto-compact must preserve versions needed by held snapshots.
    #[test]
    fn auto_compact_preserves_snapshot_history() {
        let dir = temp_dir();
        let mut db = Db::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: Some(2),
                exclusive: true,
            },
        )
        .unwrap();
        db.put(b"k", b"old").unwrap();
        let snap = db.snapshot();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap(); // triggers auto-compact at count >= 2
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        // Historical read at pre-overwrite snapshot must still see "old".
        assert_eq!(
            db.get_at(snap, b"k").as_deref(),
            Some(b"old".as_ref()),
            "F20: auto-compact must not GC versions still visible at open snapshots"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F19: file numbers past 6 digits still flush + reopen.
    #[test]
    fn high_file_number_sst_round_trip() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        // Jump allocator into the 7-digit regime.
        db.next_file_num = 1_000_000;
        db.put(b"hi", b"there").unwrap();
        db.flush().unwrap();
        assert!(db
            .ssts
            .iter()
            .any(|t| t.path().file_name().unwrap() == "1000000.sst"));
        db.close().unwrap();
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"hi").as_deref(), Some(b"there".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_with_latest_only_drops_old_versions() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"k", b"old").unwrap();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap();
        db.put(b"gone", b"x").unwrap();
        db.flush().unwrap();
        db.delete(b"gone").unwrap();
        db.flush().unwrap();

        let before_entries: usize = db.ssts.iter().map(SstTable::len).sum();
        assert!(
            before_entries >= 3,
            "history should include old put + tombstone before GC"
        );

        db.compact_with(CompactOptions::latest_only()).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        // Only live value for k remains; tombstoned key dropped entirely.
        assert_eq!(db.ssts[0].len(), 1);
        db.close().unwrap();

        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_cleans_orphan_sst_tmp_files() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        // Simulate crash mid-compact: orphan temp left behind.
        let tmp = dir.join("000099.sst.tmp");
        fs::write(&tmp, b"partial-garbage").unwrap();
        assert!(tmp.exists());

        let db = Db::open(&dir).unwrap();
        assert!(!tmp.exists(), "open must remove *.sst.tmp orphans");
        assert_eq!(db.sst_count(), 0);
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_writes_final_sst_not_tmp() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).unwrap();
        db.put(b"a", b"1").unwrap();
        db.flush().unwrap();
        db.put(b"b", b"2").unwrap();
        db.flush().unwrap();
        db.compact().unwrap();

        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        let is_final_sst = |n: &str| {
            let p = Path::new(n);
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sst"))
                && !n.ends_with(".sst.tmp")
        };
        assert!(
            names.iter().any(|n| is_final_sst(n)),
            "compact must leave a final .sst: {names:?}"
        );
        assert!(
            names.iter().all(|n| !n.ends_with(".sst.tmp")),
            "no leftover tmp after successful compact: {names:?}"
        );
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclusive_lock_blocks_other_process() {
        // Cross-process only: same-PID re-open steals (crash-sim). Hold LOCK with a child.
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::process::{Command, Stdio};
            let mut child = Command::new("sleep")
                .arg("30")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            fs::write(dir.join(crate::lock::LOCK_FILE), format!("{}\n", child.id())).unwrap();
            match Db::open(&dir) {
                Err(CoreError::AlreadyOpen { .. }) => {}
                Ok(_) => {
                    let _ = child.kill();
                    panic!("open must fail while foreign process holds LOCK");
                }
                Err(other) => {
                    let _ = child.kill();
                    panic!("expected AlreadyOpen, got {other:?}");
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            // Stale lock after child death: open steals.
            let db = Db::open(&dir).unwrap();
            db.close().unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_process_reopen_after_forget_steals_lock() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            std::mem::forget(db);
        }
        // Crash-sim: same PID steals LOCK and recovers.
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclusive_false_skips_lock_file() {
        let dir = temp_dir();
        let opts = OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            exclusive: false,
        };
        let db = Db::open_with(&dir, opts).unwrap();
        assert!(
            !dir.join(crate::lock::LOCK_FILE).exists(),
            "exclusive=false must not create LOCK"
        );
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_writes_manifest_and_current() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        assert!(dir.join(crate::manifest::CURRENT_FILE).exists());
        let current = fs::read_to_string(dir.join(crate::manifest::CURRENT_FILE)).unwrap();
        assert!(
            current.trim().starts_with(crate::manifest::MANIFEST_PREFIX),
            "CURRENT={current:?}"
        );
        let man_path = dir.join(current.trim());
        assert!(man_path.exists(), "manifest file missing");

        let db = Db::open(&dir).unwrap();
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_gcs_orphan_sst_not_in_manifest() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"keep", b"1").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        // Plant an orphan SST that is not in MANIFEST.
        let orphan = dir.join("009999.sst");
        fs::write(&orphan, b"not-a-real-sst").unwrap();
        assert!(orphan.exists());

        let db = Db::open(&dir).unwrap();
        assert!(!orphan.exists(), "orphan SST must be GC'd via MANIFEST");
        assert_eq!(db.get(b"keep").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_batch_atomic_and_snapshot() {
        let dir = temp_dir();
        {
            let mut db = Db::open(&dir).unwrap();
            let before = db.snapshot();
            assert_eq!(before.sequence(), 0);
            let last = db
                .apply_batch([
                    BatchOp::put(b"row", b"R"),
                    BatchOp::put(b"idx", b"I"),
                    BatchOp::delete(b"gone"),
                ])
                .unwrap();
            assert_eq!(last, 3);
            let snap = db.snapshot();
            assert_eq!(snap.sequence(), 3);
            assert_eq!(db.get_at(snap, b"row").as_deref(), Some(b"R".as_ref()));
            assert_eq!(db.get_at(snap, b"idx").as_deref(), Some(b"I".as_ref()));
            // Old snapshot still empty world
            assert_eq!(db.get_at(before, b"row"), None);
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"row").as_deref(), Some(b"R".as_ref()));
        assert_eq!(db.get(b"idx").as_deref(), Some(b"I".as_ref()));
        assert_eq!(db.get(b"gone"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wal_recover_from_offset_on_real_file() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("export.log");
        let mut offsets = Vec::new();
        {
            let mut wal = Wal::create(&path).unwrap();
            offsets.push(0u64);
            wal.append_record(b"r0").unwrap();
            wal.sync_all().unwrap();
            let o1 = wal.stream_position().unwrap();
            offsets.push(o1);
            wal.append_record(b"r1").unwrap();
            wal.sync_all().unwrap();
            let o2 = wal.stream_position().unwrap();
            offsets.push(o2);
            wal.append_record(b"r2").unwrap();
            wal.sync_all().unwrap();
            wal.close().unwrap();
        }
        let from_o1 = Wal::recover_from_offset(&path, offsets[1]).unwrap();
        assert_eq!(from_o1, vec![b"r1".to_vec(), b"r2".to_vec()]);
        let from_o2 = Wal::recover_from_offset(&path, offsets[2]).unwrap();
        assert_eq!(from_o2, vec![b"r2".to_vec()]);
        let _ = fs::remove_dir_all(&dir);
    }
}
