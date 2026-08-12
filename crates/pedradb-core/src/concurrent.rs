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
use std::path::Path;
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::db::{
    BatchOp, CheckpointMeta, CompactOptions, Db, DbStats, OpenOptions, Snapshot, WriteOptions,
};
use crate::env::{Env, StdEnv};
use crate::error::{CoreError, Result};
use crate::key::SequenceNumber;
use crate::merge::{StreamingVisibleIter, VisibleKv};
use crate::occ::OccTransaction;

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
            let inputs: Vec<(Vec<BatchOp>, bool)> = batch
                .iter()
                .map(|p| (p.ops.clone(), p.do_sync))
                .collect();
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
        })
    }

    /// Point get (read lock).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.inner.read().get(key)
    }

    /// Snapshot (read lock).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.inner.read().snapshot()
    }

    /// Range collect (read lock).
    #[must_use]
    pub fn range(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<(Bytes, Bytes)> {
        self.inner.read().range(start, end)
    }

    /// Streaming scan collected under a read lock (iterator cannot outlive the lock).
    ///
    /// Values are resolved through the value log when large-value pointers are present.
    #[must_use]
    pub fn scan_collect(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<(Bytes, Bytes)> {
        self.inner
            .read()
            .scan(start, end)
            .map(|VisibleKv { key, value }| (key, value))
            .collect()
    }

    /// Stats (read lock).
    #[must_use]
    pub fn stats(&self) -> DbStats {
        self.inner.read().stats()
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
            .submit(
                &self.inner,
                vec![BatchOp::put(key, value)],
                do_sync,
            )
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
    pub fn delete_range(
        &self,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<()> {
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
    pub fn apply_batch(
        &self,
        ops: impl IntoIterator<Item = BatchOp>,
    ) -> Result<SequenceNumber> {
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
    /// table is written to L0.
    ///
    /// # Errors
    /// I/O.
    pub fn flush(&self) -> Result<()> {
        // At most two pipeline steps: drain existing imm, then switch+flush active.
        // Do **not** loop while concurrent puts refill mem (that would never end).
        for _ in 0..2 {
            let imm = {
                let mut g = self.inner.write();
                g.prepare_flush_imm()?
            };
            let Some(imm) = imm else {
                break;
            };
            // Heavy I/O without write lock — other threads group-commit freely.
            let write_result = {
                let g = self.inner.read();
                g.write_memtable_to_l0_file(&imm)
            };
            let (table, file_num) = match write_result {
                Ok((t, n, _)) => (t, n),
                Err(e) => {
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
    /// # Errors
    /// I/O.
    pub fn compact_for_reads(&self) -> Result<()> {
        self.inner.write().compact_for_reads()
    }

    /// Checkpoint (exclusive write lock — flushes first).
    ///
    /// # Errors
    /// I/O.
    pub fn create_checkpoint(&self, dest: impl AsRef<Path>) -> Result<CheckpointMeta> {
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
        assert_eq!(
            ok_count.load(Ordering::SeqCst),
            1,
            "exactly one CAS winner"
        );
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
}
