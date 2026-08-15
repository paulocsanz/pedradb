//! Multi-thread access with **Rocks-style group commit** on the write path.
//!
//! [`ConcurrentDb`] wraps [`Db`] in a [`parking_lot::RwLock`]:
//! - readers (`get` / `range` / `scan` / `stats`) take a **read** lock;
//! - writers (`put` / `delete` / `apply_batch`) **join a write group**: one leader
//!   holds the write lock, appends every queued WAL record, performs **one**
//!   `fsync` for the group (if any member requested sync), applies all memtables,
//!   then wakes waiters.
//!
//! # Flush / compact (fine write lock — RFC-0016 P1.2–P1.3)
//!
//! - **Dual memtable:** flush switches active → immutable under a short write
//!   lock; SST write runs **without** holding the lock so puts can group-commit
//!   into the new active mem.
//! - **Compact:** heavy SST rewrite is prepared under lock, file write can run
//!   with only a brief install lock at the end when using [`Self::compact`].
//!
//! # Concurrency (RFC-0016 P1.1)
//!
//! Group commit + dual-mem pipeline: concurrent client threads, amortized fsync,
//! inserts not blocked for the full duration of SST I/O.

use std::collections::VecDeque;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::db::{
    BatchOp, BlobGcCandidate, CheckpointMeta, CompactOptions, Db, DbStats, OpenOptions, Snapshot,
    SnapshotPin, WriteOptions,
};
use crate::env::{Env, StdEnv};
use crate::error::{CoreError, Result};
use crate::key::SequenceNumber;
use crate::merge::{StreamingVisibleIter, VisibleKv};
use crate::occ::OccTransaction;
use crate::vlog::VlogRewriteStats;

struct PendingWrite {
    ops: Vec<BatchOp>,
    do_sync: bool,
    reply: SyncSender<Result<SequenceNumber>>,
}

struct WriteGroup {
    /// Queued client writes waiting for a leader group.
    queue: Mutex<WriteGroupState>,
}

struct WriteGroupState {
    pending: VecDeque<PendingWrite>,
    /// True while a leader is draining / committing a group.
    leader_active: bool,
}

impl WriteGroup {
    fn new() -> Self {
        Self {
            queue: Mutex::new(WriteGroupState {
                pending: VecDeque::new(),
                leader_active: false,
            }),
        }
    }

    /// Enqueue `ops` and either lead a group commit or wait for the leader.
    fn submit<E: Env>(
        &self,
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
    ) -> Result<SequenceNumber> {
        let (tx, rx) = mpsc::sync_channel(1);
        let become_leader = {
            let mut g = self.queue.lock();
            g.pending.push_back(PendingWrite {
                ops,
                do_sync,
                reply: tx,
            });
            if g.leader_active {
                false
            } else {
                g.leader_active = true;
                true
            }
        };

        if become_leader {
            self.lead(db);
        }

        rx.recv().unwrap_or_else(|_| {
            Err(CoreError::Internal(
                "write group leader dropped reply channel".into(),
            ))
        })
    }

    fn lead<E: Env>(&self, db: &RwLock<Db<E>>) {
        loop {
            let batch: Vec<PendingWrite> = {
                let mut g = self.queue.lock();
                if g.pending.is_empty() {
                    g.leader_active = false;
                    return;
                }
                g.pending.drain(..).collect()
            };

            // One write lock for the whole group: append all + one fsync + apply all.
            let mut guard = db.write();
            let inputs: Vec<(Vec<BatchOp>, bool)> =
                batch.iter().map(|p| (p.ops.clone(), p.do_sync)).collect();
            let results = guard.group_commit(inputs);
            drop(guard);

            for (pending, result) in batch.into_iter().zip(results) {
                let _ = pending.reply.send(result);
            }
            // Loop: more work may have arrived while we held the Db lock.
        }
    }
}

/// Thread-safe handle: one open directory, multi-thread get/put/flush/compact.
#[derive(Clone)]
pub struct ConcurrentDb<E: Env = StdEnv> {
    inner: Arc<RwLock<Db<E>>>,
    writes: Arc<WriteGroup>,
    /// Single-flight flush/compact pipeline (F45): dual concurrent `prepare_flush_imm`
    /// + failed `restore_imm` could otherwise race on the one imm slot.
    flush_lock: Arc<Mutex<()>>,
}

impl ConcurrentDb<StdEnv> {
    /// Open on the real filesystem (same as [`Db::open`]).
    ///
    /// # Errors
    /// Same as [`Db::open`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Open with options on the real filesystem.
    ///
    /// # Errors
    /// Same as [`Db::open_with`].
    pub fn open_with(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(RwLock::new(Db::open_with(path, opts)?)),
            writes: Arc::new(WriteGroup::new()),
            flush_lock: Arc::new(Mutex::new(())),
        })
    }
}

impl<E: Env> ConcurrentDb<E> {
    /// Wrap an existing `Db`.
    #[must_use]
    pub fn from_db(db: Db<E>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(db)),
            writes: Arc::new(WriteGroup::new()),
            flush_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Open with an explicit [`Env`].
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(RwLock::new(Db::open_with_env(path, opts, env)?)),
            writes: Arc::new(WriteGroup::new()),
            flush_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Point get (read lock).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.inner.read().get(key)
    }

    /// Point get at an explicit snapshot (read lock).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snap` is below the GC watermark.
    pub fn get_at(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        self.inner.read().get_at(snap, key)
    }

    /// Multi-get at snapshot (read lock).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn multi_get_at(
        &self,
        snap: Snapshot,
        keys: &[impl AsRef<[u8]>],
    ) -> Result<Vec<Option<Bytes>>> {
        self.inner.read().multi_get_at(snap, keys)
    }

    /// Snapshot (read lock). Bare sequence — does not register a pin.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.inner.read().snapshot()
    }

    /// Register a snapshot pin (write lock; open-items §2.1).
    pub fn pin_snapshot(&self) -> SnapshotPin {
        self.inner.write().pin_snapshot()
    }

    /// Release a pin from [`Self::pin_snapshot`].
    pub fn release_snapshot_pin(&self, pin: SnapshotPin) {
        self.inner.write().release_snapshot_pin(pin);
    }

    /// Oldest open pin sequence, if any.
    #[must_use]
    pub fn oldest_pinned_sequence(&self) -> Option<SequenceNumber> {
        self.inner.read().oldest_pinned_sequence()
    }

    /// Version-GC watermark (see [`Db::earliest_readable_sequence`]).
    #[must_use]
    pub fn earliest_readable_sequence(&self) -> SequenceNumber {
        self.inner.read().earliest_readable_sequence()
    }

    /// Opt-in auto-compact reclaim (see [`Db::set_auto_reclaim`]).
    pub fn set_auto_reclaim(&self, enabled: bool) {
        self.inner.write().set_auto_reclaim(enabled);
    }

    /// Whether auto-compact uses snapshot-safe reclaim.
    #[must_use]
    pub fn auto_reclaim(&self) -> bool {
        self.inner.read().auto_reclaim()
    }

    /// Blob rotate cap (see [`Db::set_vlog_rotate_bytes`]).
    pub fn set_vlog_rotate_bytes(&self, bytes: Option<u64>) {
        self.inner.write().set_vlog_rotate_bytes(bytes);
    }

    /// Scan prefetch window (see [`Db::set_scan_prefetch`]).
    pub fn set_scan_prefetch(&self, n: usize) {
        self.inner.write().set_scan_prefetch(n);
    }

    /// Current scan prefetch window.
    #[must_use]
    pub fn scan_prefetch(&self) -> usize {
        self.inner.read().scan_prefetch()
    }

    /// Best-effort auto blob GC threshold (see [`Db::set_auto_blob_gc_min_ratio`]).
    pub fn set_auto_blob_gc_min_ratio(&self, min_dead_ratio: Option<f64>) {
        self.inner
            .write()
            .set_auto_blob_gc_min_ratio(min_dead_ratio);
    }

    /// Current auto blob-GC threshold, if enabled.
    #[must_use]
    pub fn auto_blob_gc_min_ratio(&self) -> Option<f64> {
        self.inner.read().auto_blob_gc_min_ratio()
    }

    /// Open snapshot pin count.
    #[must_use]
    pub fn snapshot_pin_count(&self) -> usize {
        self.inner.read().snapshot_pin_count()
    }

    /// Active blob generation.
    #[must_use]
    pub fn blob_active(&self) -> u32 {
        self.inner.read().blob_active()
    }

    /// Fail closed when a snapshot is below the GC watermark.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn ensure_snapshot_readable(&self, snap: Snapshot) -> Result<()> {
        self.inner.read().ensure_snapshot_readable(snap)
    }

    /// Snapshot-safe compact reclaim (see [`Db::compact_reclaim`]).
    ///
    /// # Errors
    /// I/O.
    pub fn compact_reclaim(&self) -> Result<()> {
        let _flush = self.flush_lock.lock();
        self.inner.write().compact_reclaim()
    }

    /// Range collect at latest (read lock).
    #[must_use]
    pub fn range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Vec<(Bytes, Bytes)> {
        self.inner.read().range(start, end)
    }

    /// Range at snapshot (read lock).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn range_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        self.inner.read().range_at(snapshot, start, end)
    }

    /// Bounded range at latest (read lock).
    #[must_use]
    pub fn range_limited(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Vec<(Bytes, Bytes)> {
        self.inner.read().range_limited(start, end, limit)
    }

    /// Bounded range at snapshot (read lock).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn range_at_limited(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        self.inner
            .read()
            .range_at_limited(snapshot, start, end, limit)
    }

    /// Streaming scan collected under a read lock (iterator cannot outlive the lock).
    ///
    /// Values are resolved through the value log when large-value pointers are present.
    #[must_use]
    pub fn scan_collect(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Vec<(Bytes, Bytes)> {
        self.inner
            .read()
            .scan(start, end)
            .map(|VisibleKv { key, value }| (key, value))
            .collect()
    }

    /// Historical scan collected under a read lock (fail-closed on too-old snap).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn scan_collect_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        // Collect under the lock via range_at_limited (same fail-closed path).
        self.inner
            .read()
            .range_at_limited(snapshot, start, end, limit)
    }

    /// Stats (read lock).
    #[must_use]
    pub fn stats(&self) -> DbStats {
        self.inner.read().stats()
    }

    /// DB directory (read lock; path is stable for the open lifetime).
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.inner.read().path().to_path_buf()
    }

    /// Last sequence (read lock).
    #[must_use]
    pub fn last_sequence(&self) -> SequenceNumber {
        self.inner.read().last_sequence()
    }

    /// WAL fsync count since open (group commit amortization metric).
    #[must_use]
    pub fn wal_sync_count(&self) -> u64 {
        self.inner.read().wal_sync_count()
    }

    /// Group fsync for prior `WriteOptions::no_sync` writes (write lock).
    ///
    /// # Errors
    /// Same as [`Db::sync`].
    pub fn sync(&self) -> Result<()> {
        self.inner.write().sync()
    }

    /// Flush WAL and release directory lock via Env (consumes this handle).
    ///
    /// Other Arc clones of the same DB (if any) are not closed; prefer a single
    /// owner for exclusive open.
    ///
    /// # Errors
    /// Same as [`Db::close`].
    pub fn close(self) -> Result<()> {
        // Drop write-group / flush locks first, then close the sole Db if unique.
        let ConcurrentDb {
            inner,
            writes: _,
            flush_lock: _,
        } = self;
        match Arc::try_unwrap(inner) {
            Ok(lock) => lock.into_inner().close(),
            Err(shared) => {
                // Still referenced — best-effort WAL sync under write lock only.
                shared.write().sync()?;
                Ok(())
            }
        }
    }

    fn resolve_sync(&self, opts: WriteOptions) -> bool {
        opts.sync
            .unwrap_or_else(|| self.inner.read().default_write_sync())
    }

    /// Put via write group (may share fsync with concurrent writers).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_with(key, value, WriteOptions::default())
    }

    /// Put with options via write group.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<()> {
        self.put_with_seq(key, value, opts).map(|_| ())
    }

    /// Put via write group and return the commit sequence (RFC-0019 P0.2).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with_seq(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        let do_sync = self.resolve_sync(opts);
        self.writes
            .submit(&self.inner, vec![BatchOp::put(key, value)], do_sync)
    }

    /// Put only if key is absent (atomic under write lock; RFC-0019 CAS).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_absent(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        // Hold write lock for get+put so concurrent CAS cannot race.
        self.inner.write().put_if_absent(key, value)
    }

    /// Put only if live value equals `expected` (RFC-0019 CAS).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_eq(
        &self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.inner.write().put_if_eq(key, expected, value)
    }

    /// Alias for [`put_if_eq`](Self::put_if_eq).
    ///
    /// # Errors
    /// Same as [`put_if_eq`](Self::put_if_eq).
    pub fn compare_and_swap(
        &self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_eq(key, expected, value)
    }

    /// Delete via write group.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete(&self, key: impl AsRef<[u8]>) -> Result<()> {
        let do_sync = self.resolve_sync(WriteOptions::default());
        self.writes
            .submit(&self.inner, vec![BatchOp::delete(key)], do_sync)
            .map(|_| ())
    }

    /// Range delete via write group.
    ///
    /// # Errors
    /// WAL I/O, bounds, or sequence exhaustion.
    pub fn delete_range(&self, start: impl AsRef<[u8]>, end: impl AsRef<[u8]>) -> Result<()> {
        let do_sync = self.resolve_sync(WriteOptions::default());
        self.writes
            .submit(
                &self.inner,
                vec![BatchOp::delete_range(start, end)],
                do_sync,
            )
            .map(|_| ())
    }

    /// Apply batch via write group.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch(&self, ops: impl IntoIterator<Item = BatchOp>) -> Result<SequenceNumber> {
        let do_sync = self.resolve_sync(WriteOptions::default());
        let ops: Vec<_> = ops.into_iter().collect();
        self.writes.submit(&self.inner, ops, do_sync)
    }

    /// Point lookups for many keys (RFC-0019 P1.1).
    #[must_use]
    pub fn multi_get(&self, keys: &[impl AsRef<[u8]>]) -> Vec<Option<Bytes>> {
        self.inner.read().multi_get(keys)
    }

    /// Changes with `from_seq < sequence <= to_seq` (RFC-0019 change feed).
    ///
    /// # Errors
    /// Same as [`Db::changes`].
    pub fn changes(
        &self,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) -> Result<Vec<crate::ChangeEntry>> {
        self.inner.read().changes(from_seq, to_seq)
    }

    /// Tail of change feed after `from_seq`.
    #[must_use]
    pub fn changes_after(&self, from_seq: SequenceNumber) -> Vec<crate::ChangeEntry> {
        self.inner.read().changes_after(from_seq)
    }

    /// Flush with dual-memtable pipeline: short lock to switch, SST I/O off-lock.
    ///
    /// Concurrent `put`s may proceed into the new active mem while the immutable
    /// table is written to L0. Flush itself is **single-flight** across threads
    /// (F45) so only one imm is off-lock at a time; puts still group-commit freely.
    ///
    /// # Errors
    /// I/O.
    pub fn flush(&self) -> Result<()> {
        let _flush = self.flush_lock.lock();
        // At most two pipeline steps: drain existing imm, then switch+flush active.
        // Do **not** loop while concurrent puts refill mem (that would never end).
        for _ in 0..2 {
            // F43: allocate SST file number under the write lock so concurrent
            // flushes cannot both read the same next_file_num during off-lock I/O.
            let prepared = {
                let mut g = self.inner.write();
                match g.prepare_flush_imm()? {
                    None => None,
                    Some(imm) => {
                        let num = g.alloc_file_num();
                        Some((imm, num))
                    }
                }
            };
            let Some((imm, file_num)) = prepared else {
                break;
            };
            // Heavy I/O without write lock — other threads group-commit freely.
            let write_result = {
                let g = self.inner.read();
                g.write_memtable_to_l0_file_num(&imm, file_num)
            };
            let table = match write_result {
                Ok((t, n, _)) => {
                    debug_assert_eq!(n, file_num);
                    t
                }
                Err(e) => {
                    // Leave a file-num gap (harmless); put imm back for retry/safety.
                    self.inner.write().restore_imm(imm);
                    return Err(e);
                }
            };
            {
                let mut g = self.inner.write();
                if let Err(e) = g.install_l0_sst(table, file_num) {
                    g.restore_imm(imm);
                    return Err(e);
                }
            }
        }
        let mut g = self.inner.write();
        g.finish_flush_pipeline()?;
        Ok(())
    }

    /// Compact: flush pipeline first, then compact under write lock.
    ///
    /// Flush I/O releases the lock (see [`Self::flush`]); the compact merge still
    /// needs exclusive access to the SST inventory for install safety.
    ///
    /// # Errors
    /// I/O.
    pub fn compact(&self) -> Result<()> {
        self.flush()?;
        self.inner.write().compact_ssts_only()
    }

    /// Compact with options (flush pipeline + exclusive compact install).
    ///
    /// # Errors
    /// I/O.
    pub fn compact_with(&self, options: CompactOptions) -> Result<()> {
        self.flush()?;
        self.inner.write().compact_with_ssts_only(options)
    }

    /// Read-oriented full collapse (RFC-0019 P2.2).
    ///
    /// Single-flight with [`Self::flush`] so `Db::flush` cannot rotate WAL
    /// while another flush holds acked keys only in the read pin.
    ///
    /// # Errors
    /// I/O.
    pub fn compact_for_reads(&self) -> Result<()> {
        let _flush = self.flush_lock.lock();
        self.inner.write().compact_for_reads()
    }

    /// Blob GC candidates (read lock).
    ///
    /// # Errors
    /// Same as [`Db::blob_gc_candidates`].
    pub fn blob_gc_candidates(&self) -> Result<Vec<BlobGcCandidate>> {
        self.inner.read().blob_gc_candidates()
    }

    /// Sealed + active blob file numbers (read lock).
    #[must_use]
    pub fn blob_file_nums(&self) -> Vec<u32> {
        self.inner.read().blob_file_nums()
    }

    /// GC one sealed blob generation (single-flight with flush).
    ///
    /// # Errors
    /// Same as [`Db::compact_blob`].
    pub fn compact_blob(&self, file_num: u32) -> Result<VlogRewriteStats> {
        let _flush = self.flush_lock.lock();
        self.inner.write().compact_blob(file_num)
    }

    /// Auto-pick worst sealed blob with dead_ratio ≥ `min_dead_ratio`.
    ///
    /// # Errors
    /// Same as [`Db::compact_blob_auto`].
    pub fn compact_blob_auto(
        &self,
        min_dead_ratio: f64,
    ) -> Result<Option<(u32, VlogRewriteStats)>> {
        let _flush = self.flush_lock.lock();
        self.inner.write().compact_blob_auto(min_dead_ratio)
    }

    /// Full value-log rewrite (single-flight with flush).
    ///
    /// # Errors
    /// Same as [`Db::compact_vlog`].
    pub fn compact_vlog(&self) -> Result<VlogRewriteStats> {
        let _flush = self.flush_lock.lock();
        self.inner.write().compact_vlog()
    }

    /// Checkpoint (single-flight with flush — flushes first).
    ///
    /// Takes [`Self::flush_lock`] so this cannot run during off-lock SST I/O.
    /// `Db::flush` inside also refuses to rotate WAL while a flush read pin is
    /// live (acked keys would otherwise vanish from the copied WAL).
    ///
    /// # Errors
    /// I/O.
    pub fn create_checkpoint(&self, dest: impl AsRef<Path>) -> Result<CheckpointMeta> {
        let _flush = self.flush_lock.lock();
        self.inner.write().create_checkpoint(dest)
    }

    /// Verify checksums (read lock).
    ///
    /// # Errors
    /// Corrupt data or I/O.
    pub fn verify_checksums(&self) -> Result<()> {
        self.inner.read().verify_checksums()
    }

    /// SST count (read lock).
    #[must_use]
    pub fn sst_count(&self) -> usize {
        self.inner.read().sst_count()
    }

    /// Max LSM level (read lock).
    #[must_use]
    pub fn max_level(&self) -> u32 {
        self.inner.read().max_level()
    }

    /// Run a closure with a read guard (for tests needing many gets).
    pub fn with_read<R>(&self, f: impl FnOnce(&Db<E>) -> R) -> R {
        f(&self.inner.read())
    }

    /// Run a closure with a write guard (bypasses write group — for OCC validate).
    pub fn with_write<R>(&self, f: impl FnOnce(&mut Db<E>) -> R) -> R {
        f(&mut self.inner.write())
    }

    /// Begin an optimistic multi-writer transaction (RFC-0014 P2.1).
    ///
    /// Commit still takes the exclusive write lock for validation + apply
    /// (not merged into the put write-group).
    #[must_use]
    pub fn begin_occ(&self) -> OccTransaction<E> {
        OccTransaction::new(self.clone())
    }
}

// Silence unused import if StreamingVisibleIter only used in docs.
#[allow(dead_code)]
fn _stream_ty(_: StreamingVisibleIter) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedradb-concurrent-{n}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn open_sync(dir: &std::path::Path) -> ConcurrentDb {
        ConcurrentDb::open_with(
            dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn scan_collect_resolves_large_vlog_values() {
        use std::ops::Bound;
        let dir = temp_dir();
        let big = vec![0x22u8; 2500];
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
            },
        )
        .unwrap();
        db.put(b"k", &big).unwrap();
        let rows = db.scan_collect(Bound::Unbounded, Bound::Unbounded);
        let hit = rows.iter().find(|(k, _)| k.as_ref() == b"k").unwrap();
        assert_eq!(hit.1.as_ref(), big.as_slice());
        assert_eq!(hit.1.len(), 2500);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_gets_during_puts_and_flush() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();

        let writers = 4usize;
        let puts_per = 50usize;
        let mut handles = Vec::new();
        for w in 0..writers {
            let db = db.clone();
            handles.push(thread::spawn(move || {
                for i in 0..puts_per {
                    let k = format!("w{w}-k{i}");
                    let v = format!("v{i}");
                    db.put(k.as_bytes(), v.as_bytes()).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        db.flush().unwrap();
        for w in 0..writers {
            for i in 0..puts_per {
                let k = format!("w{w}-k{i}");
                assert!(db.get(k.as_bytes()).is_some(), "missing {k}");
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0019: concurrent CAS — only one winner, no silent lost update.
    #[test]
    fn rfc19_concurrent_cas_no_lost_update() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        db.put_if_absent(b"flag", b"0").unwrap();

        let ok_count = Arc::new(AtomicUsize::new(0));
        let mismatch_count = Arc::new(AtomicUsize::new(0));
        let n = 8usize;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let db = Arc::clone(&db);
            let ok_count = Arc::clone(&ok_count);
            let mismatch_count = Arc::clone(&mismatch_count);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                // All threads try to CAS 0 → i; exactly one must win.
                let new_val = [b'v', u8::try_from(i).unwrap()];
                match db.put_if_eq(b"flag", b"0", new_val) {
                    Ok(_) => {
                        ok_count.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(CoreError::CasMismatch) => {
                        mismatch_count.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) => panic!("unexpected CAS error: {e}"),
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(ok_count.load(Ordering::SeqCst), 1, "exactly one CAS winner");
        assert_eq!(
            mismatch_count.load(Ordering::SeqCst),
            n - 1,
            "all others fail closed"
        );
        let live = db.get(b"flag").expect("flag present");
        assert_eq!(live[0], b'v');
        assert!(live[1] < n as u8);
        drop(db);
        let re = open_sync(&dir);
        assert_eq!(re.get(b"flag").as_deref(), Some(live.as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Dual-mem pipeline: puts succeed while another thread flushes.
    #[test]
    fn puts_proceed_during_concurrent_flush() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        for i in 0..200u16 {
            let k = i.to_le_bytes();
            db.put(k, b"pre").unwrap();
        }
        let flusher = {
            let db = Arc::clone(&db);
            thread::spawn(move || {
                for _ in 0..5 {
                    db.flush().unwrap();
                }
            })
        };
        let mut handles = Vec::new();
        for t in 0..4u8 {
            let db = Arc::clone(&db);
            handles.push(thread::spawn(move || {
                for i in 0..100u8 {
                    db.put([b'c', t, i], [t, i]).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        flusher.join().unwrap();
        for t in 0..4u8 {
            for i in 0..100u8 {
                assert_eq!(
                    db.get(&[b'c', t, i]).as_deref(),
                    Some([t, i].as_slice()),
                    "lost put during flush t={t} i={i}"
                );
            }
        }
        // Reopen: dual-mem + WAL recovery must not lose concurrent puts.
        drop(db);
        let re = open_sync(&dir);
        for t in 0..4u8 {
            for i in 0..100u8 {
                assert!(re.get(&[b'c', t, i]).is_some());
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Group commit: N concurrent sync puts share fewer fsyncs than N.
    #[test]
    fn group_commit_amortizes_wal_syncs() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        let n = 32usize;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                let k = [b'k', u8::try_from(i).expect("n fits u8")];
                db.put(k, b"v").unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let syncs = db.wal_sync_count();
        // Without group commit, syncs == n. With grouping under a barrier,
        // expect strictly fewer fsyncs than puts (often 1–few groups).
        assert!(
            syncs < n as u64,
            "expected group commit to amortize fsyncs: syncs={syncs} puts={n}"
        );
        assert!(syncs >= 1, "at least one fsync for durable puts");
        // All keys present and durable on reopen.
        for i in 0..n {
            let k = [b'k', u8::try_from(i).expect("n fits u8")];
            assert_eq!(db.get(&k).as_deref(), Some(b"v".as_ref()));
        }
        drop(db);
        let re = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        for i in 0..n {
            let k = [b'k', u8::try_from(i).expect("n fits u8")];
            assert_eq!(re.get(&k).as_deref(), Some(b"v".as_ref()), "reopen key {i}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_puts_serialise_all_keys_visible() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        let ok = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for t in 0..8u8 {
            let db = db.clone();
            let ok = Arc::clone(&ok);
            handles.push(thread::spawn(move || {
                for i in 0..20u8 {
                    db.put([t, i], [t, i, 1]).unwrap();
                    ok.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(ok.load(Ordering::Relaxed), 8 * 20);
        for t in 0..8u8 {
            for i in 0..20u8 {
                assert_eq!(db.get(&[t, i]).as_deref(), Some([t, i, 1].as_slice()));
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_get_during_dedicated_flush_compact() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: false,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        for i in 0..100u8 {
            db.put([b'x', i], [i]).unwrap();
        }
        let db2 = db.clone();
        let flusher = thread::spawn(move || {
            db2.flush().unwrap();
            db2.compact().unwrap();
        });
        for _ in 0..50 {
            let _ = db.get(b"x\x00");
        }
        flusher.join().unwrap();
        assert!(db.get(b"x\x00").is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    /// F110: acked get/range stay visible after prepare takes the table off-lock.
    #[test]
    fn get_sees_acked_key_while_flush_imm_off_lock() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.put(b"k", b"acked").unwrap();
        let (imm, num) = db.with_write(|d| {
            let imm = d.prepare_flush_imm().unwrap().expect("imm");
            let num = d.alloc_file_num();
            (imm, num)
        });
        assert_eq!(
            db.get(b"k").as_deref(),
            Some(b"acked".as_ref()),
            "prepare_flush_imm must pin the taken table for readers"
        );
        let ranged = db.range(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded);
        assert!(
            ranged
                .iter()
                .any(|(k, v)| k.as_ref() == b"k" && v.as_ref() == b"acked"),
            "range must also see pin during off-lock flush: {ranged:?}"
        );
        let (table, n, _) = db
            .with_read(|d| d.write_memtable_to_l0_file_num(&imm, num))
            .unwrap();
        assert_eq!(n, num);
        db.with_write(|d| d.install_l0_sst(table, num).unwrap());
        assert_eq!(db.get(b"k").as_deref(), Some(b"acked".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// F116: checkpoint while flush I/O holds the memtable off-lock must still
    /// restore the acked key (WAL must not rotate past the pin; F110 residual).
    #[test]
    fn checkpoint_during_off_lock_flush_keeps_acked() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.put(b"k", b"acked").unwrap();
        let (imm, num) = db.with_write(|d| {
            let imm = d.prepare_flush_imm().unwrap().expect("imm");
            let num = d.alloc_file_num();
            (imm, num)
        });
        let dest = dir.join("ckpt");
        db.create_checkpoint(&dest).unwrap();
        let restored = ConcurrentDb::open_with(
            &dest,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        assert_eq!(
            restored.get(b"k").as_deref(),
            Some(b"acked".as_ref()),
            "checkpoint mid off-lock flush must keep acked k"
        );
        let (table, n, _) = db
            .with_read(|d| d.write_memtable_to_l0_file_num(&imm, num))
            .unwrap();
        assert_eq!(n, num);
        db.with_write(|d| d.install_l0_sst(table, num).unwrap());
        assert_eq!(db.get(b"k").as_deref(), Some(b"acked".as_ref()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dest);
    }

    /// F45: dual concurrent flush + failed restore must not drop another imm's data.
    ///
    /// Interleaving:
    /// 1. A prepares immA (keys `a*`) off-lock
    /// 2. Concurrent puts land in new mem (`b*`)
    /// 3. B prepares immB
    /// 4. A fails SST I/O and `restore_imm(immA)`
    /// 5. B succeeds `install_l0_sst` (which clears `imm`)
    /// Pre-fix: step 5 wiped immA → silent loss of `a*`.
    #[test]
    fn dual_flush_restore_then_install_does_not_drop_other_imm() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        for i in 0..20u8 {
            db.put([b'a', i], [b'A', i]).unwrap();
        }
        let (imm_a, num_a) = db.with_write(|d| {
            let imm = d.prepare_flush_imm().unwrap().expect("immA");
            let num = d.alloc_file_num();
            (imm, num)
        });
        for i in 0..20u8 {
            db.put([b'b', i], [b'B', i]).unwrap();
        }
        let (imm_b, num_b) = db.with_write(|d| {
            let imm = d.prepare_flush_imm().unwrap().expect("immB");
            let num = d.alloc_file_num();
            (imm, num)
        });
        assert_ne!(num_a, num_b);
        // A "fails" and restores (production ConcurrentDb::flush error path).
        db.with_write(|d| d.restore_imm(imm_a));
        // B succeeds install of its SST.
        let (table_b, n_b, _) = db
            .with_read(|d| d.write_memtable_to_l0_file_num(&imm_b, num_b))
            .unwrap();
        assert_eq!(n_b, num_b);
        db.with_write(|d| d.install_l0_sst(table_b, num_b).unwrap());
        // Keys from immA must still be visible (mem/imm/SST), not silently dropped.
        for i in 0..20u8 {
            assert_eq!(
                db.get(&[b'a', i]).as_deref(),
                Some([b'A', i].as_slice()),
                "lost immA key a{i} after dual flush restore/install"
            );
            assert_eq!(
                db.get(&[b'b', i]).as_deref(),
                Some([b'B', i].as_slice()),
                "lost immB key b{i}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// F43: concurrent flush prep must allocate distinct SST file numbers.
    #[test]
    fn concurrent_flush_allocates_distinct_file_nums() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        for i in 0..30u8 {
            db.put([b'a', i], [b'v', i]).unwrap();
        }
        // Drain first mem under exclusive prepare+alloc semantics.
        let imm1_num = {
            let g = db.with_write(|d| {
                let imm = d.prepare_flush_imm().unwrap().expect("imm1");
                let num = d.alloc_file_num();
                (imm, num)
            });
            // Put more while holding first imm offline (simulates concurrent flush I/O).
            for i in 0..30u8 {
                db.put([b'b', i], [b'w', i]).unwrap();
            }
            let (imm1, num1) = g;
            let imm2_num = db.with_write(|d| {
                let imm = d.prepare_flush_imm().unwrap().expect("imm2");
                let num = d.alloc_file_num();
                (imm, num)
            });
            let (imm2, num2) = imm2_num;
            assert_ne!(num1, num2, "concurrent imm must get distinct file nums");
            assert!(num2 > num1);
            // Off-lock writes with pre-allocated nums must not collide paths.
            let db_r = Arc::clone(&db);
            let barrier = Arc::new(Barrier::new(2));
            let b1 = Arc::clone(&barrier);
            let b2 = Arc::clone(&barrier);
            let h1 = thread::spawn({
                let db_r = Arc::clone(&db_r);
                move || {
                    b1.wait();
                    db_r.with_read(|d| d.write_memtable_to_l0_file_num(&imm1, num1))
                }
            });
            let h2 = thread::spawn({
                let db_r = Arc::clone(&db_r);
                move || {
                    b2.wait();
                    db_r.with_read(|d| d.write_memtable_to_l0_file_num(&imm2, num2))
                }
            });
            let (t1, n1, _) = h1.join().unwrap().unwrap();
            let (t2, n2, _) = h2.join().unwrap().unwrap();
            assert_eq!(n1, num1);
            assert_eq!(n2, num2);
            db.with_write(|d| {
                d.install_l0_sst(t1, num1).unwrap();
                d.install_l0_sst(t2, num2).unwrap();
            });
            (num1, num2)
        };
        let _ = imm1_num;
        for i in 0..30u8 {
            assert!(db.get(&[b'a', i]).is_some(), "lost a{i}");
            assert!(db.get(&[b'b', i]).is_some(), "lost b{i}");
        }
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sst"))
            .collect();
        let mut uniq = names.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), names.len(), "duplicate SST paths: {names:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Concurrent flush + puts: SST file numbers must stay unique and no lost keys.
    #[test]
    fn concurrent_flush_distinct_sst_file_nums() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        let n_threads = 8usize;
        let barrier = Arc::new(Barrier::new(n_threads));
        let mut handles = Vec::new();
        for t in 0..n_threads {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                for i in 0..40u8 {
                    db.put([b'k', t as u8, i], [b'v', t as u8, i]).unwrap();
                }
                barrier.wait();
                // All threads flush together after data is in.
                for _ in 0..3 {
                    for i in 0..10u8 {
                        let _ = db.put([b'x', t as u8, i], [b'y', t as u8, i]);
                    }
                    db.flush().unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let mut missing = 0usize;
        for t in 0..n_threads {
            for i in 0..40u8 {
                if db.get(&[b'k', t as u8, i]).is_none() {
                    missing += 1;
                }
            }
        }
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sst") && !n.contains(".tmp"))
            .collect();
        names.sort();
        let mut stems = names.clone();
        stems.dedup();
        eprintln!(
            "ssts={} unique={} missing={}",
            names.len(),
            stems.len(),
            missing
        );
        assert_eq!(stems.len(), names.len(), "duplicate SST files: {names:?}");
        assert_eq!(missing, 0, "lost keys after concurrent flush stress");
        // reopen durability
        drop(db);
        let db2 = open_sync(&dir);
        let mut miss2 = 0;
        for t in 0..n_threads {
            for i in 0..40u8 {
                if db2.get(&[b'k', t as u8, i]).is_none() {
                    miss2 += 1;
                }
            }
        }
        assert_eq!(miss2, 0, "lost keys after reopen");
        let _ = fs::remove_dir_all(&dir);
    }

    /// compact_blob_auto / candidates work through ConcurrentDb (flush_lock).
    #[test]
    fn concurrent_compact_blob_auto_path() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
            },
        )
        .unwrap();
        db.set_vlog_rotate_bytes(Some(3_500));
        let v1 = vec![0x11u8; 1800];
        let v2 = vec![0x22u8; 1800];
        db.put(b"a", &v1).unwrap();
        db.put(b"b", &v1).unwrap();
        db.flush().unwrap();
        db.put(b"a", &v2).unwrap();
        db.put(b"c", &v2).unwrap();
        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let cands = db.blob_gc_candidates().unwrap();
        assert!(
            cands.iter().any(|c| !c.is_active && c.bytes > 0),
            "expected sealed blob: {cands:?}"
        );
        let got = db.compact_blob_auto(0.0).unwrap();
        assert!(got.is_some(), "auto should pick a sealed file");
        assert_eq!(db.get(b"a").as_deref(), Some(v2.as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(v1.as_slice()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Session setters for blob/GC are available without with_write.
    #[test]
    fn concurrent_blob_gc_setters() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
            },
        )
        .unwrap();
        assert_eq!(db.auto_blob_gc_min_ratio(), None);
        db.set_auto_blob_gc_min_ratio(Some(0.5));
        assert_eq!(db.auto_blob_gc_min_ratio(), Some(0.5));
        db.set_vlog_rotate_bytes(Some(64 * 1024));
        db.set_scan_prefetch(8);
        assert_eq!(db.scan_prefetch(), 8);
        assert_eq!(db.snapshot_pin_count(), 0);
        let pin = db.pin_snapshot();
        assert_eq!(db.snapshot_pin_count(), 1);
        db.release_snapshot_pin(pin);
        assert_eq!(db.snapshot_pin_count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    /// path / sync / close parity with Db for ops tooling.
    #[test]
    fn concurrent_path_sync_close() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        assert_eq!(db.path(), dir);
        db.put(b"k", b"v").unwrap();
        db.sync().unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        // Exclusive open after close.
        let db2 = open_sync(&dir);
        assert_eq!(db2.get(b"k").as_deref(), Some(b"v".as_ref()));
        db2.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// get_at / range_at / scan_collect_at fail closed after reclaim (API parity).
    #[test]
    fn concurrent_snapshot_reads_fail_closed_after_reclaim() {
        use std::ops::Bound;
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        db.put(b"k", b"old").unwrap();
        db.flush().unwrap();
        let old = db.snapshot();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap();
        assert_eq!(
            db.get_at(old, b"k").unwrap().as_deref(),
            Some(b"old".as_ref())
        );
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let err = db.get_at(old, b"k").unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "get_at: {err:?}"
        );
        let err = db
            .range_at(old.sequence(), Bound::Unbounded, Bound::Unbounded)
            .unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "range_at: {err:?}"
        );
        let err = db
            .scan_collect_at(old.sequence(), Bound::Unbounded, Bound::Unbounded, None)
            .unwrap_err();
        assert!(
            matches!(err, CoreError::SnapshotTooOld { .. }),
            "scan_collect_at: {err:?}"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(b"new".as_ref()));
        let live = db.range(Bound::Unbounded, Bound::Unbounded);
        assert_eq!(live.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    /// ConcurrentDb::flush → finish_flush_pipeline must run auto blob GC
    /// (parity with single-threaded Db::flush).
    #[test]
    fn concurrent_flush_runs_auto_blob_gc() {
        let dir = temp_dir();
        let v1 = vec![0x11u8; 1800];
        let v2 = vec![0x22u8; 1800];
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: Some(512),
            },
        )
        .unwrap();
        db.with_write(|d| {
            d.set_vlog_rotate_bytes(Some(3_500));
        });
        db.put(b"a", &v1).unwrap();
        db.put(b"b", &v1).unwrap();
        db.flush().unwrap();
        db.put(b"a", &v2).unwrap();
        db.put(b"c", &v2).unwrap();
        db.flush().unwrap();
        // Drop dead SST pointers without auto GC (auto still off).
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let sealed_before: Vec<u32> = db.with_read(|d| {
            d.blob_file_nums()
                .into_iter()
                .filter(|n| *n != d.blob_active())
                .collect()
        });
        assert!(
            !sealed_before.is_empty(),
            "need a sealed blob for auto GC: {sealed_before:?}"
        );
        let gc_before = db.stats().vlog_gc_count;
        // Enable auto and flush (empty mem still finishes the pipeline).
        db.with_write(|d| d.set_auto_blob_gc_min_ratio(Some(0.0)));
        db.flush().unwrap();
        let sealed_after: Vec<u32> = db.with_read(|d| {
            d.blob_file_nums()
                .into_iter()
                .filter(|n| *n != d.blob_active())
                .collect()
        });
        assert!(
            db.stats().vlog_gc_count > gc_before || sealed_after.len() < sealed_before.len(),
            "ConcurrentDb::flush must run auto blob GC: before={sealed_before:?} after={sealed_after:?} gc_before={gc_before} gc={}",
            db.stats().vlog_gc_count
        );
        assert_eq!(db.get(b"a").as_deref(), Some(v2.as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(v1.as_slice()));
        assert_eq!(db.get(b"c").as_deref(), Some(v2.as_slice()));
        let _ = fs::remove_dir_all(&dir);
    }
}
