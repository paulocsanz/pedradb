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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use parking_lot::{Condvar, Mutex, RwLock};

use crate::db::{
    BatchOp, BlobGcCandidate, CheckpointMeta, CompactOptions, Db, DbStats, OpenOptions,
    PreparedL0Compact, Snapshot, SnapshotPin, WriteOptions,
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
    /// Signalled whenever a writer pushes onto the queue, so a leader holding
    /// its catch-up window open can absorb the arrival immediately.
    arrived: Condvar,
    /// Submits currently in flight (`submit` entry → reply consumed). Writers
    /// counted here but absent from the queue are waking between ops — exactly
    /// the stragglers the catch-up window waits for.
    active: AtomicUsize,
    /// Catch-up window length in µs (RFC-0037 P2.2): `0` disables. Runtime
    /// knob via [`ConcurrentDb::set_write_group_catchup_window`];
    /// `PEDRA_CATCHUP_US` seeds the default for lab sweeps.
    catchup_window_us: AtomicU64,
    /// Diagnostics (RFC-0037 P2.2): submits total / queued-behind-leader /
    /// groups led / ops inside led groups.
    submits: AtomicU64,
    queued: AtomicU64,
    batches: AtomicU64,
    batch_ops: AtomicU64,
    /// Last time `active > 1` (ns, `WriteGroup::now_ns`). Fast path stays
    /// off for [`MULTI_HOLD`] after a concurrent burst so apply's pre+com
    /// from 4 clients share fsyncs instead of each taking the lone-writer
    /// path between the two `write()`s (RFC-0040 P1.2).
    last_multi_ns: AtomicU64,
    /// Last `submit` entry (ns). Host compact waits for this to go idle so
    /// L0 rewrite does not run in the gaps of an apply/raftlog burst.
    last_submit_ns: AtomicU64,
}

/// Default catch-up window (see [`ConcurrentDb::set_write_group_catchup_window`]).
/// Measured on the bench box: a parked follower needs ~30–100 µs to wake and
/// resubmit, while one fsync window is ~30 µs — without holding groups open,
/// arrivals stagger one group per fsync (group_size ≈ 1.1 at 4 clients; ≈ 3
/// with it).
const CATCHUP_WINDOW_DEFAULT: Duration = Duration::from_micros(50);

/// How long after the last concurrent submit the lone-writer fast path stays
/// disabled (see `last_multi_ns`). 250 µs covers apply pre→com on this box.
const MULTI_HOLD: Duration = Duration::from_micros(250);

/// Skip the catch-up wait when the drained group already has this many user
/// ops (apply = 64, raftlog = 16). A 20 µs fat hold (fat20b) raised
/// avg_group 1.54→1.73 and cut apply_mc4 2.1 k→1.5 k. Small YCSB puts
/// still wait so they can share an fsync.
const CATCHUP_SKIP_OPS: usize = 16;

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
            arrived: Condvar::new(),
            active: AtomicUsize::new(0),
            catchup_window_us: AtomicU64::new(
                std::env::var("PEDRA_CATCHUP_US")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(CATCHUP_WINDOW_DEFAULT.as_micros() as u64),
            ),
            submits: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            batches: AtomicU64::new(0),
            batch_ops: AtomicU64::new(0),
            last_multi_ns: AtomicU64::new(0),
            last_submit_ns: AtomicU64::new(0),
        }
    }

    fn now_ns() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    fn recently_concurrent(&self) -> bool {
        let last = self.last_multi_ns.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        Self::now_ns().saturating_sub(last) < MULTI_HOLD.as_nanos() as u64
    }

    /// Enqueue `ops` and either lead a group commit or wait for the leader.
    fn submit<E: Env>(
        &self,
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
    ) -> Result<SequenceNumber> {
        self.active.fetch_add(1, Ordering::Relaxed);
        self.submits.fetch_add(1, Ordering::Relaxed);
        self.last_submit_ns.store(Self::now_ns(), Ordering::Relaxed);

        let active = self.active.load(Ordering::Relaxed);
        if active > 1 {
            self.last_multi_ns.store(Self::now_ns(), Ordering::Relaxed);
        }

        // Lone writer (parity bench, sequential client): skip the mpsc hop.
        // WAL append under the write lock, `fdatasync` off it (same as the
        // group leader) so the host worker can drain imm during the fd.
        // Stay off this path for MULTI_HOLD after a concurrent burst so
        // apply's second write() still joins the group (RFC-0040 P1.2).
        if active == 1 && !self.recently_concurrent() {
            let result = Self::lone_commit(db, ops, do_sync);
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.batch_ops.fetch_add(1, Ordering::Relaxed);
            self.active.fetch_sub(1, Ordering::Relaxed);
            return result;
        }

        let (tx, rx) = mpsc::sync_channel(1);
        let become_leader = {
            let mut g = self.queue.lock();
            g.pending.push_back(PendingWrite {
                ops,
                do_sync,
                reply: tx,
            });
            let leader = !g.leader_active;
            if leader {
                g.leader_active = true;
            } else {
                // A leader may be holding its catch-up window open for us.
                self.arrived.notify_all();
            }
            leader
        };
        if !become_leader {
            self.queued.fetch_add(1, Ordering::Relaxed);
        }

        if become_leader {
            self.lead(db);
        }

        let r = rx.recv().unwrap_or_else(|_| {
            Err(CoreError::Internal(
                "write group leader dropped reply channel".into(),
            ))
        });
        self.active.fetch_sub(1, Ordering::Relaxed);
        r
    }

    fn lead<E: Env>(&self, db: &RwLock<Db<E>>) {
        loop {
            let mut batch: Vec<PendingWrite> = {
                let mut g = self.queue.lock();
                if g.pending.is_empty() {
                    g.leader_active = false;
                    return;
                }
                g.pending.drain(..).collect()
            };

            // Catch-up window (RFC-0037 P2.2): writers counted in `active`
            // but not yet queued are waking between ops. Hold the group open
            // for them so they share this fsync instead of each forcing one
            // (measured: without it, 4 clients group ≈ 1.1 writes/fsync).
            // No-op when every active writer is already queued — a lone
            // client never waits.
            let window = Duration::from_micros(self.catchup_window_us.load(Ordering::Relaxed));
            let batch_ops: usize = batch.iter().map(|p| p.ops.len()).sum();
            if !window.is_zero()
                && batch_ops < CATCHUP_SKIP_OPS
                && batch.len() < self.active.load(Ordering::Relaxed)
            {
                let deadline = Instant::now() + window;
                let mut g = self.queue.lock();
                while batch.len() < self.active.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    let _timed_out = self.arrived.wait_for(&mut g, deadline - now);
                    batch.extend(g.pending.drain(..));
                }
                drop(g);
            }

            // One write lock: append + absorb anyone who queued during
            // prepare (no extra wait) + one fsync + apply (RFC-0041 P1.1).
            let mut guard = db.write();
            let inputs: Vec<(Vec<BatchOp>, bool)> = batch
                .iter_mut()
                .map(|p| (std::mem::take(&mut p.ops), p.do_sync))
                .collect();
            let results = match guard.group_start(inputs) {
                Err(results) => {
                    drop(guard);
                    results
                }
                Ok(mut inflight) => {
                    loop {
                        let mut extra: Vec<PendingWrite> = {
                            let mut q = self.queue.lock();
                            if q.pending.is_empty() {
                                break;
                            }
                            q.pending.drain(..).collect()
                        };
                        let more: Vec<(Vec<BatchOp>, bool)> = extra
                            .iter_mut()
                            .map(|p| (std::mem::take(&mut p.ops), p.do_sync))
                            .collect();
                        guard.group_absorb(&mut inflight, more);
                        batch.extend(extra);
                    }
                    // fdatasync off the write lock so flush/readers proceed.
                    // Ok still waits (G1). Rotate is blocked via commit_inflight.
                    Self::finish_group_off_lock(db, guard, inflight)
                }
            };
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.batch_ops
                .fetch_add(batch.len() as u64, Ordering::Relaxed);

            for (pending, result) in batch.into_iter().zip(results) {
                let _ = pending.reply.send(result);
            }
        }
    }

    /// Sequential client: one batch, `fdatasync` off the write lock (G1).
    fn lone_commit<E: Env>(
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
    ) -> Result<SequenceNumber> {
        let mut guard = db.write();
        match guard.group_start(vec![(ops, do_sync)]) {
            Err(mut results) => results.pop().unwrap_or_else(|| {
                Err(CoreError::Internal(
                    "lone writer missing admit result".into(),
                ))
            }),
            Ok(inflight) => Self::finish_group_off_lock(db, guard, inflight)
                .into_iter()
                .next()
                .unwrap_or_else(|| {
                    Err(CoreError::Internal(
                        "lone writer missing commit result".into(),
                    ))
                }),
        }
    }

    /// Drop the write lock across WAL `fdatasync`; apply mem after Ok-path sync.
    fn finish_group_off_lock<E: Env>(
        db: &RwLock<Db<E>>,
        guard: parking_lot::RwLockWriteGuard<'_, Db<E>>,
        inflight: crate::db::GroupInFlight,
    ) -> Vec<Result<SequenceNumber>> {
        let need_sync = inflight.needs_sync();
        guard.begin_commit();
        let wal = guard.wal_arc();
        drop(guard);
        let sync_err = if need_sync {
            wal.lock().sync_data().err()
        } else {
            None
        };
        let mut guard = db.write();
        let results = if let Some(e) = sync_err {
            guard.fence_durability();
            inflight.fail_sync(e)
        } else {
            if need_sync {
                guard.note_wal_sync();
            }
            guard.group_apply(inflight)
        };
        guard.end_commit();
        results
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
    /// Serializes MANIFEST/`CURRENT` persist so flush + compact cannot tear
    /// `CURRENT` when I/O runs off the Db write lock (RFC-0041 P1.1).
    persist_lock: Arc<Mutex<()>>,
    /// Cached [`OpenOptions::sync`]; never mutates after open.
    default_sync: Arc<AtomicBool>,
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
        Ok(Self::from_db(Db::open_with(path, opts)?))
    }
}

impl<E: Env> ConcurrentDb<E> {
    /// Wrap an existing `Db`.
    #[must_use]
    pub fn from_db(db: Db<E>) -> Self {
        let default_sync = db.default_write_sync();
        Self {
            inner: Arc::new(RwLock::new(db)),
            writes: Arc::new(WriteGroup::new()),
            flush_lock: Arc::new(Mutex::new(())),
            persist_lock: Arc::new(Mutex::new(())),
            default_sync: Arc::new(AtomicBool::new(default_sync)),
        }
    }

    /// Open with an explicit [`Env`].
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        Ok(Self::from_db(Db::open_with_env(path, opts, env)?))
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

    /// Skip inline auto-compact; host drains L0 (RFC-0037).
    pub fn set_defer_auto_compact(&self, enabled: bool) {
        self.inner.write().set_defer_auto_compact(enabled);
    }

    /// Whether flush leaves L0 for a host worker.
    #[must_use]
    pub fn defer_auto_compact(&self) -> bool {
        self.inner.read().defer_auto_compact()
    }

    /// L0→L1 with SST write off the write lock (RFC-0037 P1.2).
    ///
    /// Prepare + install take the write lock; merge I/O does not. Puts may
    /// group-commit while the output SST is written. Returns whether a job ran.
    ///
    /// # Errors
    /// SST / MANIFEST I/O. Failed write does not publish; L0 stays.
    pub fn compact_l0_off_lock(&self) -> Result<bool> {
        let job = {
            let mut g = self.inner.write();
            if g.level_file_count(0) == 0 {
                return Ok(false);
            }
            match g.prepare_l0_compact(CompactOptions::default())? {
                None => return Ok(false),
                Some(j) => j,
            }
        };
        let table = match job.write() {
            Ok(t) => t,
            Err(e) => {
                return Err(e);
            }
        };
        self.inner.write().install_prepared_l0_compact(job, table)?;
        Ok(true)
    }

    /// Whether auto-compact uses snapshot-safe reclaim.
    #[must_use]
    pub fn auto_reclaim(&self) -> bool {
        self.inner.read().auto_reclaim()
    }

    /// L0 write-stall threshold (see [`Db::set_write_stall_l0`]).
    pub fn set_write_stall_l0(&self, limit: Option<usize>) {
        self.inner.write().set_write_stall_l0(limit);
    }

    /// Current L0 write-stall threshold, if enabled.
    #[must_use]
    pub fn write_stall_l0(&self) -> Option<usize> {
        self.inner.read().write_stall_l0()
    }

    /// One compact drain before WriteStall (see [`Db::set_write_stall_drain`]).
    pub fn set_write_stall_drain(&self, enabled: bool) {
        self.inner.write().set_write_stall_drain(enabled);
    }

    /// Whether drain-before-stall is enabled.
    #[must_use]
    pub fn write_stall_drain(&self) -> bool {
        self.inner.read().write_stall_drain()
    }

    /// Writes refused by L0 / mem stall.
    #[must_use]
    pub fn write_stall_count(&self) -> u64 {
        self.inner.read().write_stall_count()
    }

    /// Memtable stall threshold (see [`Db::set_write_stall_mem_bytes`]).
    pub fn set_write_stall_mem_bytes(&self, bytes: Option<usize>) {
        self.inner.write().set_write_stall_mem_bytes(bytes);
    }

    /// Current memtable stall threshold in bytes, if enabled.
    #[must_use]
    pub fn write_stall_mem_bytes(&self) -> Option<usize> {
        self.inner.read().write_stall_mem_bytes()
    }

    /// Soft L0 pressure drain (see [`Db::set_write_pressure_l0`]).
    pub fn set_write_pressure_l0(&self, limit: Option<usize>) {
        self.inner.write().set_write_pressure_l0(limit);
    }

    /// Current soft L0 pressure threshold, if enabled.
    #[must_use]
    pub fn write_pressure_l0(&self) -> Option<usize> {
        self.inner.read().write_pressure_l0()
    }

    /// Soft pressure drain count.
    #[must_use]
    pub fn write_pressure_count(&self) -> u64 {
        self.inner.read().write_pressure_count()
    }

    /// Pebble-shaped L0 backpressure defaults (see [`Db::enable_write_backpressure_defaults`]).
    pub fn enable_write_backpressure_defaults(&self) {
        self.inner.write().enable_write_backpressure_defaults();
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
    #[deprecated(
        since = "0.1.0",
        note = "materialises the whole interval into RAM (OOM footgun on large DBs); \
                use `scan`/`scan_collect` for streaming or `range_limited` \
                for a bounded collect"
    )]
    #[must_use]
    pub fn range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Vec<(Bytes, Bytes)> {
        self.range_limited(start, end, None)
    }

    /// Range at snapshot (read lock).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    #[deprecated(
        since = "0.1.0",
        note = "materialises the whole interval into RAM (OOM footgun on large DBs); \
                use `scan_at` for streaming or `range_at_limited` for a bounded collect"
    )]
    pub fn range_at(
        &self,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        self.range_at_limited(snapshot, start, end, None)
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

    /// Write-group diagnostics (RFC-0037 P2.2): `(submits, queued_behind_leader,
    /// groups_committed, ops_in_groups)`.
    ///
    /// `ops_in_groups / groups_committed` is the achieved average group size;
    /// `queued_behind_leader / submits` says how often a submitter found a
    /// leader already active (parked instead of leading).
    #[must_use]
    pub fn write_group_stats(&self) -> (u64, u64, u64, u64) {
        (
            self.writes.submits.load(Ordering::Relaxed),
            self.writes.queued.load(Ordering::Relaxed),
            self.writes.batches.load(Ordering::Relaxed),
            self.writes.batch_ops.load(Ordering::Relaxed),
        )
    }

    /// True when no writer is in `submit` / group `fdatasync` and the last
    /// submit is older than `idle`. Host compact uses this so L0 rewrite
    /// does not start in a 5 ms poll gap of an apply burst (RFC-0041 P1.1).
    #[must_use]
    pub fn writes_idle_for(&self, idle: Duration) -> bool {
        if self.writes.active.load(Ordering::Relaxed) > 0 {
            return false;
        }
        if self.inner.read().commit_inflight() > 0 {
            return false;
        }
        let last = self.writes.last_submit_ns.load(Ordering::Relaxed);
        if last == 0 {
            return true;
        }
        WriteGroup::now_ns().saturating_sub(last) >= idle.as_nanos() as u64
    }

    /// Write-group catch-up window (RFC-0037 P2.2). Default 50 µs
    /// (`PEDRA_CATCHUP_US` overrides at open). The leader holds a group open
    /// up to this long for writers that are in flight but not yet queued, so
    /// they share one `fdatasync` instead of each forcing one.
    ///
    /// **Latency mode:** `Duration::ZERO` disables the wait — groups close as
    /// soon as the queue drains (group_size drops toward 1 per fsync; each op
    /// saves up to one window of added latency). Only affects multi-writer
    /// workloads; a lone writer never waits either way.
    #[must_use]
    pub fn write_group_catchup_window(&self) -> Duration {
        Duration::from_micros(self.writes.catchup_window_us.load(Ordering::Relaxed))
    }

    /// Set the catch-up window (see [`Self::write_group_catchup_window`]).
    /// Takes effect on the next group; concurrent leaders observe it relaxed.
    pub fn set_write_group_catchup_window(&self, window: Duration) {
        let micros = window.as_micros().min(u64::MAX as u128) as u64;
        self.writes
            .catchup_window_us
            .store(micros, Ordering::Relaxed);
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
            persist_lock: _,
            default_sync: _,
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
            .unwrap_or_else(|| self.default_sync.load(Ordering::Relaxed))
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
                        let (env, dir, sync) = g.l0_write_ctx();
                        Some((imm, num, env, dir, sync))
                    }
                }
            };
            let Some((imm, file_num, env, dir, sync)) = prepared else {
                break;
            };
            // Heavy I/O with **no** Db lock — a read guard here would block
            // writers for the whole SST write (parking_lot RwLock).
            let write_result = Db::write_imm_l0_file(&env, &dir, sync, &imm, file_num);
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

    /// Drain **one existing** immutable memtable → L0 without forcing an
    /// active→imm switch (host compact-worker shape, RFC-0037 P2.1).
    ///
    /// Staged like [`Self::flush`] — prepare + install under short write
    /// locks, SST I/O (`tmp` + rename + `sync_dir`) off-lock — and
    /// single-flight with it via the flush lock (F45: one imm in flight).
    /// Returns whether a step ran, so host workers loop until `false`.
    #[must_use]
    pub fn drain_imm_once(&self) -> bool {
        // Cheap no-op: a write lock here every poll (20 ms) stalls every
        // reader on the idle path. Check under a read guard first.
        if !self.inner.read().has_imm() {
            return false;
        }
        let _flush = self.flush_lock.lock();
        let prepared = {
            let mut g = self.inner.write();
            if !g.has_imm() {
                return false;
            }
            match g.prepare_flush_imm() {
                Ok(Some(imm)) => {
                    let num = g.alloc_file_num();
                    let (env, dir, sync) = g.l0_write_ctx();
                    Some((imm, num, env, dir, sync))
                }
                _ => None,
            }
        };
        let Some((imm, file_num, env, dir, sync)) = prepared else {
            return false;
        };
        let table = match Db::write_imm_l0_file(&env, &dir, sync, &imm, file_num) {
            Ok((t, n, _)) => {
                debug_assert_eq!(n, file_num);
                t
            }
            Err(_) => {
                self.inner.write().restore_imm(imm);
                return false;
            }
        };
        // In-memory L0 only. Do **not** rotate here: after a stage the
        // active mem is empty, so try_rotate would fsync the new 64 MiB
        // SST + MANIFEST mid-apply (224 ms tail, RFC-0041 streamsst).
        // The host worker rotates only when `writes_idle_for`.
        {
            let mut g = self.inner.write();
            g.apply_l0_install(table, file_num);
            g.retire_flush_pin();
        }
        true
    }

    /// Stage an existing imm into [`Db::parked_unflushed`] with **no SST I/O**.
    ///
    /// Host worker uses this during a write burst so apply does not pay lz4
    /// encode of every 4 MiB table (RFC-0041). WAL still covers the keys;
    /// [`Self::rotate_wal_if_writers_idle`] no-ops until
    /// [`Self::materialize_parked_once`] writes the files.
    #[must_use]
    pub fn park_imm_once(&self) -> bool {
        if !self.inner.read().has_imm() {
            return false;
        }
        let _flush = self.flush_lock.lock();
        let mut g = self.inner.write();
        if !g.has_imm() {
            return false;
        }
        let Some(imm) = g.take_imm_no_pin() else {
            return false;
        };
        g.push_parked_unflushed(imm);
        true
    }

    /// Write **one** parked mem to L0 (idle path). Leaves the table on the
    /// read path until the file is installed, then keeps it as a point/MVCC
    /// cache. Returns whether a file was written.
    #[must_use]
    pub fn materialize_parked_once(&self) -> bool {
        if self.inner.read().parked_unflushed_count() == 0 {
            return false;
        }
        let _flush = self.flush_lock.lock();
        let prepared = {
            let mut g = self.inner.write();
            let Some(front) = g.parked_front() else {
                return false;
            };
            // Clone only on the idle path so readers keep the original.
            let imm = front.clone();
            let num = g.alloc_file_num();
            let (env, dir, sync) = g.l0_write_ctx();
            Some((imm, num, env, dir, sync))
        };
        let Some((imm, file_num, env, dir, sync)) = prepared else {
            return false;
        };
        let table = match Db::write_imm_l0_file(&env, &dir, sync, &imm, file_num) {
            Ok((t, n, _)) => {
                debug_assert_eq!(n, file_num);
                t
            }
            Err(_) => return false,
        };
        {
            let mut g = self.inner.write();
            g.apply_l0_install(table, file_num);
            if let Some(orig) = g.take_oldest_parked() {
                g.retire_mem_as_l0_cache(orig);
            }
        }
        true
    }

    /// Persist pending L0s + MANIFEST and rotate WAL when no writer is in
    /// flight. No-op if mem/imm still hold acked keys (G1).
    ///
    /// # Errors
    /// SST / MANIFEST / WAL I/O.
    pub fn rotate_wal_if_writers_idle(&self) -> Result<()> {
        if !self.writes_idle_for(Duration::ZERO) {
            return Ok(());
        }
        self.inner.write().try_rotate_wal_if_idle()
    }

    /// Merge parked flush pins into one retired BTree **off** the write lock.
    ///
    /// Drain only pushes pins (apply must not absorb under the write lock).
    /// Safe during a write burst: absorb does not hold the Db write lock.
    pub fn fold_retired_pending_off_lock(&self) {
        let pending = self.inner.write().take_retired_pending();
        if pending.is_empty() {
            return;
        }
        let mut built = crate::memtable::MemTable::new();
        for pin in pending {
            built.absorb(pin);
        }
        self.inner.write().install_retired_fold(built);
    }

    /// `fdatasync` pending L0s + persist MANIFEST without holding the write lock.
    ///
    /// WAL is kept (mem may still hold keys). Compact can then rewrite L0
    /// without paying a 4–64 MiB fd under the write lock mid-scan (RFC-0041).
    ///
    /// # Errors
    /// SST / MANIFEST I/O.
    pub fn persist_unsynced_l0s_off_lock(&self) -> Result<()> {
        let prepared = {
            let mut g = self.inner.write();
            if g.unsynced_sst_count() == 0 {
                return Ok(());
            }
            let paths = g.take_unsynced_ssts();
            let (env, dir, sync) = g.l0_write_ctx();
            Some((paths, env, dir, sync))
        };
        let Some((paths, env, dir, sync)) = prepared else {
            return Ok(());
        };
        if let Err(e) = Db::fsync_sst_paths(&env, &dir, &paths, sync) {
            self.inner.write().restore_unsynced_ssts(paths);
            return Err(e);
        }
        let persist = {
            let mut g = self.inner.write();
            match g.take_manifest_persist() {
                Ok(p) => p,
                Err(e) => {
                    g.restore_unsynced_ssts(paths);
                    return Err(e);
                }
            }
        };
        let wrote = {
            let _p = self.persist_lock.lock();
            persist.write()
        };
        if let Err(e) = wrote {
            self.inner.write().restore_unsynced_ssts(paths);
            return Err(e);
        }
        Ok(())
    }

    /// Publish a prepared L0→L1 compact: mem install under the write lock,
    /// MANIFEST `fsync` off-lock (RFC-0041 P1.1).
    #[must_use]
    pub fn install_prepared_l0_off_lock(
        &self,
        job: PreparedL0Compact<E>,
        table: crate::sst::SstTable,
    ) -> bool {
        let staged = {
            let mut g = self.inner.write();
            let Some(undo) = g.apply_prepared_l0_compact(job, table) else {
                return true;
            };
            let old_paths = undo.old_paths().to_vec();
            if g.fsync_unsynced_ssts().is_err() {
                g.undo_prepared_l0_compact(undo);
                return false;
            }
            match g.take_manifest_persist() {
                Ok(persist) => Some((undo, persist, old_paths)),
                Err(_) => {
                    g.undo_prepared_l0_compact(undo);
                    return false;
                }
            }
        };
        let Some((undo, persist, old_paths)) = staged else {
            return true;
        };
        let wrote = {
            let _p = self.persist_lock.lock();
            persist.write()
        };
        let mut g = self.inner.write();
        if wrote.is_err() {
            g.undo_prepared_l0_compact(undo);
            return false;
        }
        for path in old_paths {
            let _ = g.env().remove_file(&path);
        }
        g.note_l0_compact();
        true
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

    /// Files at LSM `level` (read lock).
    #[must_use]
    pub fn level_file_count(&self, level: u32) -> usize {
        self.inner.read().level_file_count(level)
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
fn _stream_ty(_: StreamingVisibleIter<'_>) {}

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

    /// Host-worker drain: only an existing imm is written; active mem stays.
    #[test]
    fn drain_imm_once_writes_existing_imm_only() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: Some(256),
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        db.set_defer_auto_compact(true);
        let payload = vec![b'x'; 200];
        db.put(b"a", &payload).unwrap();
        db.put(b"b", &payload).unwrap();
        // Two ~200 B puts trip 256 B auto-flush → stage_flush_imm.
        assert!(
            db.with_read(|d| d.has_imm()),
            "auto-flush under defer must leave an imm"
        );
        assert!(db.drain_imm_once(), "worker must drain that imm");
        assert!(!db.with_read(|d| d.has_imm()));
        assert!(db.sst_count() >= 1);
        assert_eq!(db.get(b"a").as_deref(), Some(payload.as_slice()));
        assert_eq!(db.get(b"b").as_deref(), Some(payload.as_slice()));
        // No imm and small active mem: drain is a no-op.
        db.put(b"c", b"tiny").unwrap();
        assert!(!db.drain_imm_once());
        assert_eq!(db.get(b"c").as_deref(), Some(&b"tiny"[..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Host drain must not rotate WAL (SST fsync + MANIFEST) just because
    /// active mem is empty after a stage — that was the apply 224 ms tail.
    #[test]
    fn drain_imm_does_not_rotate_wal() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        db.put(b"k", vec![b'v'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        let wal_before = db.stats().wal_bytes;
        assert!(wal_before > 32, "put must have appended WAL");
        assert!(db.drain_imm_once());
        let wal_after = db.stats().wal_bytes;
        assert_eq!(
            wal_after, wal_before,
            "drain must keep WAL; rotate is idle-only"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        db.rotate_wal_if_writers_idle().unwrap();
        assert!(
            db.stats().wal_bytes < wal_before,
            "idle rotate may replace WAL"
        );
        drop(db);
        // After rotate, SST+MANIFEST hold the key even if WAL is gone.
        let wal = dir.join(crate::db::WAL_FILE_NAME);
        if wal.exists() {
            let _ = fs::remove_file(&wal);
        }
        let re = open_sync(&dir);
        assert_eq!(re.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Flushed mem stays on the read path until L0 compact; WAL rotate is
    /// still allowed (retired is a cache, SST+WAL are the source).
    #[test]
    fn retired_mem_serves_reads_and_does_not_block_rotate() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        db.put(b"k", vec![b'v'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.drain_imm_once());
        db.put(b"j", vec![b'w'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.drain_imm_once());
        assert_eq!(
            db.with_read(|d| d.retired_mem_count()),
            2,
            "each drain parks one L0 pin"
        );
        // Scan uses L0 SSTs, not the retired BTree chain (retire2 qps tail).
        let pre_fold = db.scan_collect(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded);
        assert!(
            pre_fold
                .iter()
                .any(|(k, v)| k.as_ref() == b"k" && v.as_ref() == [b'v'; 64]),
            "scan must see keys via L0 SST before fold"
        );
        assert!(
            !db.writes_idle_for(Duration::from_secs(1)),
            "fold must work with a recent submit, not only after a long idle"
        );
        db.fold_retired_pending_off_lock();
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        assert_eq!(db.get(b"j").as_deref(), Some(&[b'w'; 64][..]));
        let scanned = db.scan_collect(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded);
        assert!(
            scanned
                .iter()
                .any(|(k, v)| k.as_ref() == b"k" && v.as_ref() == [b'v'; 64]),
            "scan must see first folded key without the covering L0"
        );
        assert!(
            scanned
                .iter()
                .any(|(k, v)| k.as_ref() == b"j" && v.as_ref() == [b'w'; 64]),
            "scan must see second folded key in the same index"
        );
        let wal_before = db.stats().wal_bytes;
        db.rotate_wal_if_writers_idle().unwrap();
        assert!(
            db.stats().wal_bytes < wal_before,
            "retired mem must not block idle WAL rotate"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        db.compact().unwrap();
        assert_eq!(
            db.with_read(|d| d.level_file_count(0)),
            0,
            "compact must drain L0"
        );
        assert_eq!(
            db.with_read(|d| d.retired_mem_count()),
            0,
            "retired cache must drop with L0"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        drop(db);
        let re = open_sync(&dir);
        assert_eq!(re.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Park moves imm off the live table with no SST; WAL still covers the
    /// key (G1). Rotate must wait until materialize writes the file.
    #[test]
    fn park_imm_blocks_rotate_until_materialized() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        db.put(b"k", vec![b'v'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.park_imm_once());
        assert_eq!(db.with_read(|d| d.parked_unflushed_count()), 1);
        assert_eq!(db.sst_count(), 0, "park must not write an SST");
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        let scanned = db.scan_collect(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded);
        assert!(
            scanned
                .iter()
                .any(|(k, v)| k.as_ref() == b"k" && v.as_ref() == [b'v'; 64]),
            "scan must see parked keys that have no SST yet"
        );
        let wal_before = db.stats().wal_bytes;
        db.rotate_wal_if_writers_idle().unwrap();
        assert_eq!(
            db.stats().wal_bytes,
            wal_before,
            "rotate must wait for parked mems to become L0 (G1)"
        );
        assert!(db.materialize_parked_once());
        assert_eq!(db.with_read(|d| d.parked_unflushed_count()), 0);
        assert!(db.sst_count() >= 1);
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        db.persist_unsynced_l0s_off_lock().unwrap();
        db.rotate_wal_if_writers_idle().unwrap();
        assert!(
            db.stats().wal_bytes < wal_before,
            "after materialize, idle rotate may replace WAL"
        );
        drop(db);
        let wal = dir.join(crate::db::WAL_FILE_NAME);
        if wal.exists() {
            let _ = fs::remove_file(&wal);
        }
        let re = open_sync(&dir);
        assert_eq!(re.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// One idle tick materializes one parked mem, not the whole pile.
    #[test]
    fn materialize_parked_once_is_one_file() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        db.put(b"k", vec![b'v'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.park_imm_once());
        db.put(b"j", vec![b'w'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.park_imm_once());
        assert_eq!(db.with_read(|d| d.parked_unflushed_count()), 2);
        assert!(db.materialize_parked_once());
        assert_eq!(db.with_read(|d| d.parked_unflushed_count()), 1);
        assert_eq!(db.sst_count(), 1);
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        assert_eq!(db.get(b"j").as_deref(), Some(&[b'w'; 64][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Off-lock L0 persist: SST+MANIFEST hold the key if WAL is deleted.
    #[test]
    fn persist_unsynced_off_lock_makes_sst_sufficient() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        db.put(b"k", vec![b'v'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.drain_imm_once());
        assert!(db.with_read(|d| d.unsynced_sst_count()) >= 1);
        db.persist_unsynced_l0s_off_lock().unwrap();
        assert_eq!(db.with_read(|d| d.unsynced_sst_count()), 0);
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        drop(db);
        let wal = dir.join(crate::db::WAL_FILE_NAME);
        if wal.exists() {
            let _ = fs::remove_file(&wal);
        }
        let re = open_sync(&dir);
        assert_eq!(re.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Group commit: N concurrent sync puts share fewer fsyncs than N.
    /// Catch-up window knob: defaults to 50 µs, ZERO disables waiting, and
    /// writes stay correct (visible + durable) with it disabled.
    #[test]
    fn catchup_window_knob_roundtrip_and_latency_mode() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        assert_eq!(db.write_group_catchup_window(), CATCHUP_WINDOW_DEFAULT);

        // Latency mode: no group is ever held open for stragglers.
        db.set_write_group_catchup_window(Duration::ZERO);
        assert_eq!(db.write_group_catchup_window(), Duration::ZERO);
        let n = 8usize;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let db = db.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                for j in 0..4u8 {
                    db.put(
                        [u8::try_from(i).expect("n fits u8"), j],
                        [u8::try_from(i).expect("n fits u8"), j, 7],
                    )
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        for i in 0..n {
            for j in 0..4u8 {
                let k = [u8::try_from(i).expect("n fits u8"), j];
                let v = [u8::try_from(i).expect("n fits u8"), j, 7];
                assert_eq!(db.get(&k).as_deref(), Some(v.as_ref()));
            }
        }
        let (submits, _queued, groups, group_ops) = db.write_group_stats();
        assert_eq!(submits, (n * 4) as u64);
        assert_eq!(group_ops, (n * 4) as u64);
        assert!(groups >= 1 && groups <= (n * 4) as u64);

        // Knob takes effect again after re-enabling.
        db.set_write_group_catchup_window(Duration::from_micros(1234));
        assert_eq!(db.write_group_catchup_window(), Duration::from_micros(1234));
        let _ = fs::remove_dir_all(&dir);
    }

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
        // Diagnostics agree: every put submitted, all inside led groups, and
        // group commits covered multiple puts on average.
        let (submits, _queued, groups, group_ops) = db.write_group_stats();
        assert_eq!(submits, n as u64);
        assert_eq!(group_ops, n as u64);
        assert!(
            groups < n as u64,
            "groups={groups} should amortize over {n}"
        );
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

    /// RFC-0040 P1.2: two sequential puts per client after a burst must not
    /// all take the lone-writer fast path (that would be 2N fsyncs).
    #[test]
    fn sticky_concurrent_groups_second_write() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        let n = 8usize;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                db.put([b'a', i as u8], b"1").unwrap();
                db.put([b'b', i as u8], b"2").unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let syncs = db.wal_sync_count();
        let (submits, _, groups, group_ops) = db.write_group_stats();
        assert_eq!(submits, (n * 2) as u64);
        assert_eq!(group_ops, (n * 2) as u64);
        assert!(
            syncs < (n * 2) as u64,
            "second write per client must share fsyncs: syncs={syncs} puts={}",
            n * 2
        );
        assert!(
            groups < (n * 2) as u64,
            "groups={groups} should be < {}",
            n * 2
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0041 P1.1: apply-sized batches skip the long catch-up; late-join
    /// still shares a `fdatasync` when clients are already queued; drain_imm
    /// persists; keys survive reopen.
    #[test]
    fn large_batch_skips_catchup_and_flush_reopens() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: Some(8 * 1024),
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap();
        db.set_defer_auto_compact(true);
        let payload = vec![b'p'; 200];
        let n_clients = 4usize;
        let batch = 16usize;
        let barrier = Arc::new(std::sync::Barrier::new(n_clients));
        std::thread::scope(|s| {
            for c in 0..n_clients {
                let db = &db;
                let payload = &payload;
                let barrier = &barrier;
                s.spawn(move || {
                    barrier.wait();
                    let mut ops = Vec::with_capacity(batch);
                    for i in 0..batch {
                        let mut k = vec![b'k', c as u8];
                        k.extend_from_slice(&(i as u32).to_be_bytes());
                        ops.push(BatchOp::put(k, payload.as_slice()));
                    }
                    db.apply_batch(ops).unwrap();
                });
            }
        });
        while db.drain_imm_once() {}
        let (submits, _, groups, group_ops) = db.write_group_stats();
        assert_eq!(submits, n_clients as u64);
        assert_eq!(group_ops, n_clients as u64);
        assert!(
            groups < n_clients as u64,
            "fat-batch short catch-up must share fsyncs: groups={groups} clients={n_clients}"
        );
        assert!(
            db.wal_sync_count() < n_clients as u64,
            "fat batches must amortize fdatasync: syncs={} clients={n_clients}",
            db.wal_sync_count()
        );
        for c in 0..n_clients {
            for i in 0..batch {
                let mut k = vec![b'k', c as u8];
                k.extend_from_slice(&(i as u32).to_be_bytes());
                assert_eq!(
                    db.get(&k).as_deref(),
                    Some(payload.as_slice()),
                    "live c={c} i={i}"
                );
            }
        }
        drop(db);
        let re = ConcurrentDb::open(&dir).unwrap();
        for c in 0..n_clients {
            for i in 0..batch {
                let mut k = vec![b'k', c as u8];
                k.extend_from_slice(&(i as u32).to_be_bytes());
                assert_eq!(
                    re.get(&k).as_deref(),
                    Some(payload.as_slice()),
                    "reopen c={c} i={i}"
                );
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Sequential 1-client path also `fdatasync`s off the write lock (G1).
    #[test]
    fn off_lock_lone_fsync_is_durable_on_reopen() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        for i in 0..8u8 {
            db.put([b'l', i], [b'v', i]).unwrap();
        }
        assert!(db.wal_sync_count() >= 8, "each lone put must fdatasync");
        assert!(
            db.writes_idle_for(Duration::ZERO),
            "no writer in flight after sequential puts return"
        );
        db.put([b'z'], [b'1']).unwrap();
        assert!(
            !db.writes_idle_for(Duration::from_millis(1)),
            "1 ms idle must not fire immediately after a submit (apply gaps)"
        );
        std::thread::sleep(Duration::from_millis(3));
        assert!(
            db.writes_idle_for(Duration::from_millis(1)),
            "1 ms idle is true a few ms after the last Ok"
        );
        drop(db);
        let re = open_sync(&dir);
        for i in 0..8u8 {
            assert_eq!(
                re.get(&[b'l', i]).as_deref(),
                Some(&[b'v', i][..]),
                "reopen must see acked lone put {i}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0041: group `fdatasync` runs off the Db write lock; Ok still waits
    /// and reopen sees every acked key (G1).
    #[test]
    fn off_lock_group_fsync_is_durable_on_reopen() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        let n = 8usize;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        std::thread::scope(|s| {
            for i in 0..n {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                s.spawn(move || {
                    barrier.wait();
                    let k = [b'd', i as u8];
                    db.put(k, b"dur").unwrap();
                });
            }
        });
        assert!(db.wal_sync_count() >= 1, "leader must fdatasync before Ok");
        for i in 0..n {
            assert_eq!(db.get(&[b'd', i as u8]).as_deref(), Some(&b"dur"[..]));
        }
        drop(db);
        let re = open_sync(&dir);
        for i in 0..n {
            assert_eq!(
                re.get(&[b'd', i as u8]).as_deref(),
                Some(&b"dur"[..]),
                "reopen must see acked put {i}"
            );
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
        let ranged = db.range_limited(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded, None);
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
            .range_at_limited(old.sequence(), Bound::Unbounded, Bound::Unbounded, None)
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
        let live = db.range_limited(Bound::Unbounded, Bound::Unbounded, None);
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

    #[test]
    fn compact_l0_off_lock_keeps_puts_visible() {
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
        db.set_defer_auto_compact(true);
        for i in 0..crate::db::L0_COMPACTION_TRIGGER {
            db.put([b'a', i as u8], [b'1', i as u8]).unwrap();
            db.flush().unwrap();
        }
        assert!(db.compact_l0_off_lock().unwrap());
        assert_eq!(db.get(&[b'a', 0]).as_deref(), Some([b'1', 0].as_slice()));
        db.put(b"after", b"ok").unwrap();
        assert_eq!(db.get(b"after").as_deref(), Some(b"ok".as_slice()));
        let _ = fs::remove_dir_all(&dir);
    }
}
