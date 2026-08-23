//! Multi-thread access with **Rocks-style group commit** on the write path.
//!
//! [`ConcurrentDb`] wraps [`Db`] in a [`parking_lot::RwLock`]:
//! - readers (`get` / `range` / `scan` / `stats`) take a **read** lock;
//! - writers (`put` / `delete` / `apply_batch`) **join a write group**: one leader
//!   holds the write lock, appends every queued WAL record, applies memtables,
//!   performs **one** `fsync` off that lock (if any member requested sync),
//!   then publishes the snapshot and wakes waiters (G1: Ok waits for fd).
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
    /// `None` for the group leader — `lead` returns that result directly
    /// so the leader skips an mpsc hop (RFC-0041 apply_mc4).
    reply: Option<SyncSender<Result<SequenceNumber>>>,
    /// OCC: snapshot + read-set. Validated under the leader write lock so
    /// concurrent non-conflicting txs share one fdatasync (`surreal_tx_rmw_mc8`).
    occ: Option<(SequenceNumber, Vec<Bytes>)>,
    /// Set when OCC validation fails; zipped over the group result.
    occ_err: Option<CoreError>,
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
    /// Last `submit` entry (ns).
    last_submit_ns: AtomicU64,
    /// Last `submit` **return** (ns). Host compact must wait on this, not
    /// `last_submit_ns`: an apply_mc4 batch of 5–14 ms would look idle
    /// under a 5 ms last-submit rule the instant it returns (RFC-0041).
    last_complete_ns: AtomicU64,
    /// RFC-0042 P0.2: cumulative lone-writer phase timings (ns) —
    /// `[group_start (lock+prepare+encode), apply (mem), wal io
    /// (write+fdatasync), publish]` — over `lone_count` commits.
    lone_phase_ns: [AtomicU64; 4],
    lone_count: AtomicU64,
    /// RFC-0042 P1.1: EMA (7/8 old + 1/8 sample) of the last successful WAL
    /// `fdatasync` duration (ns); `0` = no sample yet (see
    /// [`WriteGroup::fd_ema`]).
    fd_ema_ns: AtomicU64,
    /// RFC-0042 P1.1: total ns leaders spent in the catch-up wait, and how
    /// many groups entered it (diagnostics for the bound).
    catchup_wait_ns: AtomicU64,
    catchup_waits: AtomicU64,
    /// RFC-0044 P0.5: merge concurrent async writers into one group.
    async_group: bool,
    /// RFC-0045 P0.2: bounded spin before parking on the bypass write lock
    /// (`PEDRA_WRITE_SPIN`, default 0 = park immediately).
    write_spin: AtomicUsize,
    /// RFC-0045 P2.2: fair handoff on the bypass write lock
    /// (`PEDRA_WRITE_FAIR=1`, default off). Unfair release wakes every
    /// waiter per commit (measured herd at 50 writers: avg lock wait
    /// 175 µs vs 1.6 µs hold); fair acquire + `unlock_fair` hands the
    /// lock directly to the next waiter — one wake per handoff, no
    /// convoy. Prototype lever; the quiet arbiter decides.
    write_fair: bool,
    /// RFC-0045 P0.1: lock-wait accumulation for the async bypass
    /// (`PEDRA_WRITE_PHASE_STATS=1`); `None` when the env is unset.
    phase_stats: Option<Arc<crate::db::WritePhaseStats>>,
}

/// Default catch-up window (see [`ConcurrentDb::set_write_group_catchup_window`]).
/// RFC-0041 P1.1: raftlog is 16 ops (`CATCHUP_SKIP_OPS` is 32) so the leader
/// must wait for other MC clients or each client pays its own `fdatasync`.
/// 50 µs is the only **quiet-box** datapoint (head3: `deps_raftlog_mc4`
/// 1.792). 80 µs was tried without a quiet remesure and reverted: the
/// fat-batch wait costs every queued member the full window while saving at
/// most `(group-1)` serialized fds, so break-even caps the window near one
/// `fdatasync` (~26 µs here) — not above it. 1-op puts still wait only
/// `min(window, fd_ema/2)` (RFC-0042). `PEDRA_CATCHUP_US=0` still disables.
const CATCHUP_WINDOW_DEFAULT: Duration = Duration::from_micros(50);

/// How long after the last concurrent submit the lone-writer fast path stays
/// disabled (see `last_multi_ns`). 250 µs covers apply pre→com on this box.
const MULTI_HOLD: Duration = Duration::from_micros(250);

/// RFC-0044 P0.5: merge concurrent async writers into one group frame/`write()`
/// (leader encodes for all members, no catch-up wait). **Default off** — A/B
/// on the bench box (findings/rfc0044-p1, 5 paired rounds) the single leader
/// is a scheduling single point of failure under 50 threads / 12 CPUs:
/// merge 44–106 k qps vs bypass 311–636 k. The bypass (every writer takes
/// the write lock itself — the Rocks shape) is the default;
/// `PEDRA_ASYNC_GROUP=1` re-enables the merge for quiet-box experiments.
const ASYNC_GROUP_DEFAULT: bool = false;

/// Skip the catch-up wait only for apply-sized batches (64 ops). Raftlog is
/// 16 ops — skipping at 16 left `deps_raftlog_mc4` at ~0.6–0.8× (one fd per
/// client). A 20 µs hold on 64-op apply (fat20b) cut apply_mc4; do not wait
/// on apply. YCSB 1-op puts still wait so they share an fsync.
const CATCHUP_SKIP_OPS: usize = 32;

/// Seed for the fd EMA before the first real `fdatasync` sample (RFC-0042
/// P1.1): isolated p50 on the bench box is 22–26 µs (RFC-0041 P0.2).
const WAL_FD_SEED: Duration = Duration::from_micros(25);

/// RFC-0042 P1.1 / RFC-0043 P2.5 — break-even bound for the catch-up window.
///
/// - Fat raftlog (≥16 ops, still < [`CATCHUP_SKIP_OPS`]): wait the full
///   configured window so MC siblings can join.
/// - High concurrency (`active ≥ 16`): full configured window.
/// - Low concurrency 1-op (YCSB `_mc4`): cap is `fd_ema / 2`.
///
/// `None` = do not wait: knob off (`window == 0`) or every active writer
/// is already inside the batch (`batch_len >= active`).
fn catchup_wait_bound(
    window: Duration,
    fd_ema: Duration,
    batch_len: usize,
    active: usize,
    batch_ops: usize,
) -> Option<Duration> {
    if window.is_zero() || batch_len >= active {
        return None;
    }
    if batch_ops >= 16 {
        return Some(window);
    }
    if active >= 16 {
        return Some(window);
    }
    Some(window.min(fd_ema / 2))
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
            last_complete_ns: AtomicU64::new(0),
            lone_phase_ns: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            lone_count: AtomicU64::new(0),
            fd_ema_ns: AtomicU64::new(0),
            catchup_wait_ns: AtomicU64::new(0),
            catchup_waits: AtomicU64::new(0),
            async_group: std::env::var("PEDRA_ASYNC_GROUP")
                .ok()
                .and_then(|v| match v.as_str() {
                    "0" | "false" => Some(false),
                    "1" | "true" => Some(true),
                    _ => None,
                })
                .unwrap_or(ASYNC_GROUP_DEFAULT),
            write_spin: AtomicUsize::new(
                std::env::var("PEDRA_WRITE_SPIN")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            ),
            write_fair: std::env::var("PEDRA_WRITE_FAIR")
                .ok()
                .and_then(|v| match v.as_str() {
                    "1" | "true" => Some(true),
                    "0" | "false" => Some(false),
                    _ => None,
                })
                .unwrap_or(false),
            phase_stats: None,
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

    /// Recent WAL `fdatasync` duration (EMA); `WAL_FD_SEED` until the first
    /// real sample lands (RFC-0042 P1.1).
    fn fd_ema(&self) -> Duration {
        let ns = self.fd_ema_ns.load(Ordering::Relaxed);
        Duration::from_nanos(if ns == 0 {
            WAL_FD_SEED.as_nanos() as u64
        } else {
            ns
        })
    }

    fn update_fd_ema(&self, sample_ns: u64) {
        let prev = self.fd_ema_ns.load(Ordering::Relaxed);
        let next = if prev == 0 {
            sample_ns
        } else {
            (prev.saturating_mul(7).saturating_add(sample_ns)) / 8
        };
        self.fd_ema_ns.store(next, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn record_lone(&self, phase_ns: [u64; 4]) {
        for (slot, v) in self.lone_phase_ns.iter().zip(phase_ns) {
            slot.fetch_add(v, Ordering::Relaxed);
        }
        self.lone_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Enqueue `ops` and either lead a group commit or wait for the leader.
    fn submit<E: Env>(
        &self,
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
    ) -> Result<SequenceNumber> {
        self.submit_inner(db, ops, do_sync, None)
    }

    fn submit_occ<E: Env>(
        &self,
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
        snapshot: SequenceNumber,
        read_set: Vec<Bytes>,
    ) -> Result<SequenceNumber> {
        self.submit_inner(db, ops, do_sync, Some((snapshot, read_set)))
    }

    fn submit_inner<E: Env>(
        &self,
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
        occ: Option<(SequenceNumber, Vec<Bytes>)>,
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
        // Lone async (`do_sync=false`) takes the leaner `commit_async_ops`.
        if occ.is_none() && active == 1 && !self.recently_concurrent() {
            let result = if do_sync {
                Self::lone_commit(self, db, ops, do_sync, occ)
            } else {
                db.write().commit_async_ops(ops)
            };
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.batch_ops.fetch_add(1, Ordering::Relaxed);
            self.active.fetch_sub(1, Ordering::Relaxed);
            self.mark_complete();
            return result;
        }

        // Concurrent writers — sync or async — join the group. Async
        // members are merged into one frame and one `write()` at 64 KiB
        // (RFC-0044 P0.5); `lead` skips the catch-up wait when no member
        // syncs (there is no fd to share). `PEDRA_ASYNC_GROUP=0` keeps the
        // Rocks shape instead: every async writer takes the write lock
        // itself (no mpsc, no leader dependency).
        if occ.is_none() && !do_sync && !self.async_group {
            let t0 = self.phase_stats.as_ref().map(|_| Instant::now());
            // RFC-0045 P0.2: bounded spin-then-park (`PEDRA_WRITE_SPIN`).
            // Default 0 = plain park (current shape). P0 measured the 50-thread
            // bypass spending ~99% of writer time blocked on this lock (avg
            // wait ~344 µs vs ~2 µs hold) — if spinning moves qps, the gap is
            // the park/unpark convoy, not CPU.
            // RFC-0045 P2.2: `PEDRA_WRITE_FAIR=1` releases the bypass lock
            // with a direct handoff to the next waiter (`unlock_fair` —
            // lock_api 0.4 has no fair acquire; the fair release is the
            // herd killer: the handed-off token blocks barging, so one
            // directed wake per commit instead of wake-all + 49 re-parks).
            let fair = self.write_fair;
            let spins = self.write_spin.load(Ordering::Relaxed);
            let mut guard = if fair {
                db.write()
            } else if spins > 0 {
                let mut g = None;
                for _ in 0..spins {
                    if let Some(acquired) = db.try_write() {
                        g = Some(acquired);
                        break;
                    }
                    std::hint::spin_loop();
                }
                g.unwrap_or_else(|| db.write())
            } else {
                db.write()
            };
            if let (Some(st), Some(t0)) = (self.phase_stats.as_ref(), t0) {
                st.lock_wait_ns
                    .fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
            let result = guard.commit_async_ops(ops);
            if fair {
                parking_lot::RwLockWriteGuard::unlock_fair(guard);
            } else {
                drop(guard);
            }
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.batch_ops.fetch_add(1, Ordering::Relaxed);
            self.active.fetch_sub(1, Ordering::Relaxed);
            self.mark_complete();
            return result;
        }

        let (reply, rx) = {
            let mut g = self.queue.lock();
            let leader = !g.leader_active;
            if leader {
                g.leader_active = true;
                g.pending.push_back(PendingWrite {
                    ops,
                    do_sync,
                    reply: None,
                    occ,
                    occ_err: None,
                });
                (None, None)
            } else {
                let (tx, rx) = mpsc::sync_channel(1);
                g.pending.push_back(PendingWrite {
                    ops,
                    do_sync,
                    reply: Some(tx),
                    occ,
                    occ_err: None,
                });
                self.arrived.notify_all();
                (Some(()), Some(rx))
            }
        };
        let r = if reply.is_none() {
            // F197: a leader panic unwinds through `lead` without clearing
            // `leader_active`, so every future writer queues behind a dead
            // leader and blocks on `recv()` forever. Catch the unwind,
            // release the group (queued members get Err), fence the Db
            // (mid-commit in-memory state is uncertain — same stance as a
            // post-commit manifest error), then re-raise the panic.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.lead(db))) {
                Ok(r) => r,
                Err(payload) => {
                    let mut g = self.queue.lock();
                    g.leader_active = false;
                    let dead = std::mem::take(&mut g.pending);
                    drop(g);
                    for p in dead {
                        if let Some(tx) = p.reply {
                            let _ = tx.send(Err(CoreError::Internal(
                                "write group leader panicked mid-commit".into(),
                            )));
                        }
                    }
                    // This thread's `active` ticket never reaches the tail
                    // decrement after a resume; account it here.
                    self.active.fetch_sub(1, Ordering::Relaxed);
                    self.mark_complete();
                    if let Some(mut guard) = db.try_write() {
                        guard.fence_durability_post_commit(
                            &"write group leader panicked mid-commit",
                        );
                    }
                    std::panic::resume_unwind(payload);
                }
            }
        } else {
            self.queued.fetch_add(1, Ordering::Relaxed);
            rx.expect("follower has recv").recv().unwrap_or_else(|_| {
                Err(CoreError::Internal(
                    "write group leader dropped reply channel".into(),
                ))
            })
        };
        self.active.fetch_sub(1, Ordering::Relaxed);
        self.mark_complete();
        r
    }

    fn mark_complete(&self) {
        self.last_complete_ns
            .store(Self::now_ns(), Ordering::Relaxed);
    }

    fn lead<E: Env>(&self, db: &RwLock<Db<E>>) -> Result<SequenceNumber> {
        let mut leader_result: Option<Result<SequenceNumber>> = None;
        loop {
            let mut batch: Vec<PendingWrite> = {
                let mut g = self.queue.lock();
                if g.pending.is_empty() {
                    g.leader_active = false;
                    return leader_result.unwrap_or_else(|| {
                        Err(CoreError::Internal(
                            "write group leader had no member result".into(),
                        ))
                    });
                }
                g.pending.drain(..).collect()
            };

            // Catch-up window (RFC-0037 P2.2): writers counted in `active`
            // but not yet queued are waking between ops. Hold the group open
            // for them so they share this fsync instead of each forcing one
            // (measured: without it, 4 clients group ≈ 1.1 writes/fsync).
            // RFC-0042 P1.1: the hold is bounded by break-even — at most
            // `min(window, fd_ema/2)` — so a straggler that never arrives
            // cannot burn more than half an fd of everyone's latency.
            let window = Duration::from_micros(self.catchup_window_us.load(Ordering::Relaxed));
            let batch_ops: usize = batch.iter().map(|p| p.ops.len()).sum();
            let active = self.active.load(Ordering::Relaxed);
            // Async-only group (RFC-0044 P0.5): no fd to share, so the
            // catch-up hold is pure latency — the merge (one encode pass,
            // one `write()` at 64 KiB) is the whole win.
            let any_sync = batch.iter().any(|p| p.do_sync);
            if any_sync && batch_ops < CATCHUP_SKIP_OPS {
                if let Some(bound) =
                    catchup_wait_bound(window, self.fd_ema(), batch.len(), active, batch_ops)
                {
                    let t_wait = Instant::now();
                    let deadline = t_wait + bound;
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
                    self.catchup_wait_ns
                        .fetch_add(t_wait.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    self.catchup_waits.fetch_add(1, Ordering::Relaxed);
                }
            }

            // One write lock: append + absorb anyone who queued during
            // prepare (no extra wait) + one fsync + apply (RFC-0041 P1.1).
            let mut guard = db.write();
            Self::validate_occ_batch(&mut guard, &mut batch);
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
                        Self::validate_occ_batch(&mut guard, &mut extra);
                        let more: Vec<(Vec<BatchOp>, bool)> = extra
                            .iter_mut()
                            .map(|p| (std::mem::take(&mut p.ops), p.do_sync))
                            .collect();
                        guard.group_absorb(&mut inflight, more);
                        batch.extend(extra);
                    }
                    Self::finish_group_off_lock(
                        self,
                        db,
                        guard,
                        inflight,
                        Some(&mut batch),
                        || {
                            let mut q = self.queue.lock();
                            q.pending.drain(..).collect()
                        },
                        None,
                    )
                }
            };
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.batch_ops
                .fetch_add(batch.len() as u64, Ordering::Relaxed);

            for (pending, result) in batch.into_iter().zip(results) {
                let result = pending.occ_err.map(Err).unwrap_or(result);
                match pending.reply {
                    None => leader_result = Some(result),
                    Some(tx) => {
                        let _ = tx.send(result);
                    }
                }
            }
        }
    }

    fn validate_occ_batch<E: Env>(guard: &mut Db<E>, batch: &mut [PendingWrite]) {
        for p in batch.iter_mut() {
            let Some((snap, keys)) = p.occ.as_ref() else {
                continue;
            };
            if let Err(e) = guard.ensure_snapshot_readable(Snapshot::at(*snap)) {
                p.ops.clear();
                p.occ_err = Some(e);
                continue;
            }
            if guard.last_sequence() == *snap {
                continue;
            }
            let conflict = keys
                .iter()
                .any(|k| guard.key_has_write_after(k.as_ref(), *snap))
                || p.ops.iter().any(|op| match op {
                    BatchOp::Put { key, .. } | BatchOp::Delete { key } => {
                        guard.key_has_write_after(key, *snap)
                    }
                    BatchOp::DeleteRange { .. } => false,
                });
            if conflict {
                p.ops.clear();
                p.occ_err = Some(CoreError::TransactionConflict);
            }
        }
    }

    /// Sequential client: one batch, `fdatasync` off the write lock (G1).
    /// RFC-0042 P0.2: accumulates the phase split into `group.lone_phase_ns`.
    fn lone_commit<E: Env>(
        group: &WriteGroup,
        db: &RwLock<Db<E>>,
        ops: Vec<BatchOp>,
        do_sync: bool,
        occ: Option<(SequenceNumber, Vec<Bytes>)>,
    ) -> Result<SequenceNumber> {
        let mut guard = db.write();
        if let Some((snap, keys)) = occ.as_ref() {
            guard.ensure_snapshot_readable(Snapshot::at(*snap))?;
            if guard.last_sequence() != *snap {
                let conflict = keys
                    .iter()
                    .any(|k| guard.key_has_write_after(k.as_ref(), *snap))
                    || ops.iter().any(|op| match op {
                        BatchOp::Put { key, .. } | BatchOp::Delete { key } => {
                            guard.key_has_write_after(key, *snap)
                        }
                        BatchOp::DeleteRange { .. } => false,
                    });
                if conflict {
                    return Err(CoreError::TransactionConflict);
                }
            }
        }
        match guard.group_start(vec![(ops, do_sync)]) {
            Err(mut results) => results.pop().unwrap_or_else(|| {
                Err(CoreError::Internal(
                    "lone writer missing admit result".into(),
                ))
            }),
            Ok(inflight) => {
                Self::finish_group_off_lock(group, db, guard, inflight, None, Vec::new, None)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| {
                        Err(CoreError::Internal(
                            "lone writer missing commit result".into(),
                        ))
                    })
            }
        }
    }

    /// Apply mem under the write lock, absorb anyone who queued during that
    /// apply (same fd, no extra wait), drop the lock for WAL `fdatasync`,
    /// then publish (G1: Ok and default `get` wait for fd).
    ///
    /// RFC-0042: records the real `fdatasync` duration into the group's fd
    /// EMA; when `lone` is set, fills `[apply, io, publish]` phase timings.
    #[allow(clippy::too_many_arguments)]
    fn finish_group_off_lock<E: Env>(
        group: &WriteGroup,
        db: &RwLock<Db<E>>,
        mut guard: parking_lot::RwLockWriteGuard<'_, Db<E>>,
        inflight: crate::db::GroupInFlight,
        mut batch: Option<&mut Vec<PendingWrite>>,
        mut drain: impl FnMut() -> Vec<PendingWrite>,
        mut lone: Option<&mut [u64; 4]>,
    ) -> Vec<Result<SequenceNumber>> {
        let mut need_sync = inflight.needs_sync();
        let mut pub_seq = inflight.max_appended_seq();
        guard.begin_commit();
        let mut results = guard.group_apply(inflight);
        if let Some(batch) = batch.as_mut() {
            loop {
                let mut extra = drain();
                if extra.is_empty() {
                    break;
                }
                Self::validate_occ_batch(&mut guard, &mut extra);
                let more: Vec<(Vec<BatchOp>, bool)> = extra
                    .iter_mut()
                    .map(|p| (std::mem::take(&mut p.ops), p.do_sync))
                    .collect();
                match guard.group_start(more) {
                    Err(r) => results.extend(r),
                    Ok(inf) => {
                        need_sync |= inf.needs_sync();
                        pub_seq = pub_seq.max(inf.max_appended_seq());
                        results.extend(guard.group_apply(inf));
                    }
                }
                batch.extend(extra);
            }
        }
        let wal = guard.wal_arc();
        drop(guard);
        let io_err = {
            let mut w = wal.lock();
            // G1: write + fdatasync before Ok. Async: write() at 64 KiB
            // (Rocks WritableFileWriter); no fdatasync.
            w.write_pending_frame_if(need_sync).err().or_else(|| {
                if need_sync {
                    let t_fd = Instant::now();
                    let r = w.sync_data().err();
                    if r.is_none() {
                        group.update_fd_ema(t_fd.elapsed().as_nanos() as u64);
                    }
                    r
                } else {
                    None
                }
            })
        };
        if let Some(l) = lone.as_mut() {
            l[1] = 0;
            l[2] = 0;
        }
        if let Some(e) = io_err {
            let mut g = db.write();
            g.fence_durability(&e, crate::db::FenceClass::of_core(&e));
            g.end_commit();
            return results
                .into_iter()
                .map(|r| match r {
                    Ok(_) => Err(CoreError::Internal(format!(
                        "group wal write/sync failed: {e}"
                    ))),
                    Err(err) => Err(err),
                })
                .collect();
        }
        let g = db.read();
        if need_sync {
            g.note_wal_sync();
        }
        g.publish_sequence(pub_seq);
        g.end_commit();
        if let Some(l) = lone {
            l[3] = 0;
        }
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
    /// Shared point cache — a hit needs no Db read lock (YCSB C).
    point_cache: Arc<crate::cache::PointCache>,
    /// Shared count cache — a hit needs no Db read lock (`deps_scan`).
    count_cache: Arc<crate::cache::CountCache>,
    /// Invalidate epoch for compat TLS last-count (`deps_scan` zipf).
    read_cache_epoch: Arc<std::sync::atomic::AtomicU64>,
    /// Published sequence — OCC begin / visible_sequence without the Db lock.
    published_seq: Arc<std::sync::atomic::AtomicU64>,
    /// Snapshot-list version GC during parked folds (rust-rocksdb `Snapshot`
    /// semantics). Default off — the core keeps every version (F20) until
    /// the history-tier GC runs (archive-before-GC). The compat layer turns
    /// it on: Rocks parity drops superseded versions below the oldest live
    /// reader (pin / OCC begin), which also bounds parked-fold memory.
    fold_gc: Arc<std::sync::atomic::AtomicBool>,
    /// Live OCC transaction snapshots (id → lower-bound sequence). A fold
    /// with GC must not drop a version an open transaction can still read:
    /// entries register a published lower bound **before** the txn reads its
    /// snapshot, so a fold that did not see the entry cannot have a floor
    /// above it (published is monotone).
    occ_registry: Arc<Mutex<std::collections::BTreeMap<u64, SequenceNumber>>>,
    occ_next_id: Arc<std::sync::atomic::AtomicU64>,
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
        let point_cache = db.point_cache_handle();
        let count_cache = db.count_cache_handle();
        let read_cache_epoch = db.read_cache_epoch_handle();
        let published_seq = db.published_seq_handle();
        let phase_stats = db.write_phase_stats();
        let mut writes = WriteGroup::new();
        writes.phase_stats = phase_stats;
        // F201: share the OCC registry into the Db so reclaim / auto-compact
        // GC floors cannot pass an open transaction's snapshot.
        let occ_registry = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let mut db = db;
        db.set_occ_floor_registry(Arc::clone(&occ_registry));
        Self {
            inner: Arc::new(RwLock::new(db)),
            writes: Arc::new(writes),
            flush_lock: Arc::new(Mutex::new(())),
            persist_lock: Arc::new(Mutex::new(())),
            default_sync: Arc::new(AtomicBool::new(default_sync)),
            point_cache,
            count_cache,
            read_cache_epoch,
            published_seq,
            fold_gc: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            occ_registry,
            occ_next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// Open with an explicit [`Env`].
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        Ok(Self::from_db(Db::open_with_env(path, opts, env)?))
    }

    /// Point get. A point-cache hit answers without the Db read lock
    /// (misses fall through to the locked path, which fills the cache).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        if let Some(v) = self.point_cache.get(key) {
            return v;
        }
        self.inner.read().get(key)
    }

    /// Point-cache probe (`Some` = hit, including cached miss). OCC get.
    #[must_use]
    pub(crate) fn point_cache_get(&self, key: &[u8]) -> Option<Option<Bytes>> {
        self.point_cache.get(key)
    }

    /// Epoch bumped when published writes invalidate read caches.
    #[must_use]
    pub fn read_cache_epoch(&self) -> u64 {
        self.read_cache_epoch.load(Ordering::Acquire)
    }

    /// Count live keys in `[start, end)` at the published snapshot.
    ///
    /// A count-cache hit answers without the Db read lock (RFC-0041
    /// `deps_scan` zipf repeats).
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`].
    pub fn count_in_range(
        &self,
        start: std::ops::Bound<&[u8]>,
        end: std::ops::Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Result<usize> {
        if let Some(n) = self.count_cache.get(start, end, limit) {
            return Ok(n);
        }
        let g = self.inner.read();
        g.count_in_range(g.visible_sequence(), start, end, limit)
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

    /// Durable/published sequence default reads observe (lock-free).
    #[must_use]
    pub fn visible_sequence(&self) -> SequenceNumber {
        self.published_seq.load(Ordering::Acquire)
    }

    /// OCC begin snapshot: `last_sequence` if the write lock is free, else
    /// published (do not block `begin` behind apply).
    #[must_use]
    pub(crate) fn occ_snapshot(&self) -> SequenceNumber {
        match self.inner.try_read() {
            Some(g) => g.last_sequence(),
            None => self.published_seq.load(Ordering::Acquire),
        }
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

    /// Lone-writer phase split (RFC-0042 P0.2): `(commits, [start, apply,
    /// io, publish])` in cumulative ns. `start` = write lock + prepare +
    /// WAL encode; `apply` = memtable apply; `io` = WAL `write` +
    /// `fdatasync`; `publish` = visibility publish after the fd.
    #[must_use]
    pub fn lone_path_split(&self) -> (u64, [u64; 4]) {
        (
            self.writes.lone_count.load(Ordering::Relaxed),
            [
                self.writes.lone_phase_ns[0].load(Ordering::Relaxed),
                self.writes.lone_phase_ns[1].load(Ordering::Relaxed),
                self.writes.lone_phase_ns[2].load(Ordering::Relaxed),
                self.writes.lone_phase_ns[3].load(Ordering::Relaxed),
            ],
        )
    }

    /// Recent WAL `fdatasync` duration (EMA; seeded from the box baseline
    /// until the first commit, RFC-0042 P1.1).
    #[must_use]
    pub fn wal_fd_ema(&self) -> Duration {
        self.writes.fd_ema()
    }

    /// Catch-up wait diagnostics (RFC-0042 P1.1): `(total_ns, groups)` that
    /// leaders spent holding groups open for stragglers.
    #[must_use]
    pub fn catchup_wait_stats(&self) -> (u64, u64) {
        (
            self.writes.catchup_wait_ns.load(Ordering::Relaxed),
            self.writes.catchup_waits.load(Ordering::Relaxed),
        )
    }

    /// True when no writer is in `submit` / group `fdatasync` and the last
    /// Ok is older than `idle`. Uses submit **return** time so a long
    /// apply batch (p95 5–14 ms) is not treated as idle the moment it
    /// returns (RFC-0041: last-submit idle started compact in apply gaps).
    #[must_use]
    pub fn writes_idle_for(&self, idle: Duration) -> bool {
        if self.writes.active.load(Ordering::Relaxed) > 0 {
            return false;
        }
        if self.inner.read().commit_inflight() > 0 {
            return false;
        }
        let last = self.writes.last_complete_ns.load(Ordering::Relaxed);
        let last = if last == 0 {
            self.writes.last_submit_ns.load(Ordering::Relaxed)
        } else {
            last
        };
        if last == 0 {
            return true;
        }
        WriteGroup::now_ns().saturating_sub(last) >= idle.as_nanos() as u64
    }

    /// How long until [`Self::writes_idle_for`] becomes true, if not already.
    ///
    /// Host worker uses this so L0 compact can start ~2 ms after the last Ok
    /// (okidle waited a full 5 ms poll and left L0=20–22 at scan) without
    /// treating a long apply as idle the instant it returns.
    #[must_use]
    pub fn writes_until_idle(&self, idle: Duration) -> Option<Duration> {
        if self.writes_idle_for(idle) {
            return None;
        }
        if self.writes.active.load(Ordering::Relaxed) > 0 || self.inner.read().commit_inflight() > 0
        {
            return Some(idle);
        }
        let last = {
            let c = self.writes.last_complete_ns.load(Ordering::Relaxed);
            if c == 0 {
                self.writes.last_submit_ns.load(Ordering::Relaxed)
            } else {
                c
            }
        };
        if last == 0 {
            return None;
        }
        let ago = WriteGroup::now_ns().saturating_sub(last);
        let need = idle.as_nanos() as u64;
        if ago >= need {
            return None;
        }
        Some(Duration::from_nanos(need - ago))
    }

    /// Writers currently inside `submit` (group or lone).
    #[must_use]
    pub fn writes_active(&self) -> usize {
        self.writes.active.load(Ordering::Relaxed)
    }

    /// True when `active > 1` was seen within `hold` (apply_mc4 gaps).
    #[must_use]
    pub fn recently_multi(&self, hold: Duration) -> bool {
        let last = self.writes.last_multi_ns.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        WriteGroup::now_ns().saturating_sub(last) < hold.as_nanos() as u64
    }

    /// Parked mems that still have no L0 file.
    #[must_use]
    pub fn parked_unflushed_count(&self) -> usize {
        self.inner.read().parked_unflushed_count()
    }

    /// Whether an immutable memtable is waiting for the host to park/drain.
    #[must_use]
    pub fn has_imm(&self) -> bool {
        self.inner.read().has_imm()
    }

    /// Rotate a full active mem into imm when over the flush cap (no BTree spill).
    ///
    /// Host calls this after a write burst (`!recently_multi`) so apply does
    /// not stage on the Ok path.
    #[must_use]
    pub fn try_stage_if_full(&self) -> bool {
        let mut g = self.inner.write();
        let Some(limit) = g.auto_flush_threshold() else {
            return false;
        };
        if g.active_mem_usage() < limit {
            return false;
        }
        g.stage_flush_imm().unwrap_or(false)
    }

    /// Merge the two oldest parked mems into one BTree off the write lock.
    ///
    /// Originals stay visible until the swap (G2). Host worker folds during
    /// 1-client / idle ticks so scan merges one BTree instead of ~20 L0s
    /// (wake2 compact never finished before scan). Does not fold while
    /// apply_mc4 is multi-writer — that was incrfold (apply 1.25 → 0.67).
    ///
    /// With [`Self::set_fold_version_gc`] on, superseded versions below
    /// `min(oldest pin, oldest OCC snapshot, published)` are dropped
    /// (rust-rocksdb snapshot-list semantics) and the GC watermark ratchets
    /// to the floor — reads below it fail `SnapshotTooOld`, never
    /// silent-wrong. Every read at or above the floor is exact: the keep-set
    /// is `{seq > floor} ∪ {newest ≤ floor}` per key.
    #[must_use]
    pub fn fold_parked_once_off_lock(&self) -> bool {
        let gc = self.fold_gc_enabled();
        let (pair, floor) = {
            let mut g = self.inner.write();
            (
                g.parked_oldest_pair_arcs(),
                gc.then(|| self.fold_floor_locked(&g)),
            )
        };
        let Some((a, b)) = pair else {
            return false;
        };
        // Deep clone + absorb off the Db lock so MVCC/scan are not stalled
        // for the BTree copy (parkfold run1/3 MVCC max 16–18 ms).
        let mut built = (*a).clone();
        built.absorb_with_floor((*b).clone(), floor);
        // F174: the swap only lands if the front pair is still (a, b) —
        // a concurrent materialize may have drained it while we built.
        let mut g = self.inner.write();
        let landed = g.replace_oldest_parked_pair(built);
        if landed {
            if let Some(f) = floor {
                g.raise_earliest_readable(f);
            }
        }
        landed
    }

    /// Enable/disable snapshot-list version GC during parked folds
    /// (see [`Self::fold_parked_once_off_lock`]). Off by default.
    pub fn set_fold_version_gc(&self, on: bool) {
        self.fold_gc.store(on, std::sync::atomic::Ordering::Release);
    }

    /// Whether fold version GC is on.
    #[must_use]
    pub fn fold_gc_enabled(&self) -> bool {
        self.fold_gc.load(std::sync::atomic::Ordering::Acquire)
    }

    /// GC floor under the Db write lock: `min(oldest pin, oldest OCC
    /// snapshot, published)`. Registered readers (pins, OCC) are exact; the
    /// `published` term covers un-pinned readers created after this fold
    /// (their snapshot ≥ published ⇒ keep-set answers them exactly).
    fn fold_floor_locked(&self, g: &Db<E>) -> SequenceNumber {
        let mut floor = g.visible_sequence();
        if let Some(p) = g.oldest_pinned_sequence() {
            floor = floor.min(p);
        }
        if let Some(o) = self.occ_registry.lock().values().copied().min() {
            floor = floor.min(o);
        }
        floor
    }

    /// Register an OCC snapshot lower bound **before** the caller reads its
    /// snapshot sequence: a fold that does not see this entry cannot have a
    /// floor above `published` at that time, and every snapshot the caller
    /// can subsequently read is ≥ that published sequence (monotone). Returns
    /// the id for [`Self::occ_unregister_snapshot`].
    pub(crate) fn occ_register_snapshot(&self) -> u64 {
        let id = self
            .occ_next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bound = self
            .published_seq
            .load(std::sync::atomic::Ordering::Acquire);
        self.occ_registry.lock().insert(id, bound);
        id
    }

    /// Drop a registration from [`Self::occ_register_snapshot`] (idempotent).
    pub(crate) fn occ_unregister_snapshot(&self, id: u64) {
        self.occ_registry.lock().remove(&id);
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
            point_cache: _,
            count_cache: _,
            read_cache_epoch: _,
            published_seq: _,
            fold_gc: _,
            occ_registry: _,
            occ_next_id: _,
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

    /// Override the open-time [`crate::db::OpenOptions::sync`] default.
    /// Bench-only: `PEDRA_PARITY_ASYNC=1` drops G1 for a same-class column.
    pub fn set_default_write_sync(&self, sync: bool) {
        self.default_sync.store(sync, Ordering::Relaxed);
    }

    /// RFC-0045 P0.1: shared write-phase timings when
    /// `PEDRA_WRITE_PHASE_STATS=1` was set at open (`None` otherwise).
    #[must_use]
    pub fn write_phase_stats(&self) -> Option<Arc<crate::db::WritePhaseStats>> {
        self.writes.phase_stats.clone()
    }

    /// RFC-0047 P0.2: what a [`crate::db::WalRecovery::PointInTime`] open
    /// discarded (`None` = clean open or FailClosed mode).
    #[must_use]
    pub fn last_recovery_report(&self) -> Option<crate::db::RecoveryReport> {
        self.inner.read().last_recovery_report().cloned()
    }

    /// RFC-0047 P1.2: whether the kernel is currently durability-fenced
    /// (host auto-resume policy polls this; typed class via
    /// [`Self::fence_report`]).
    #[must_use]
    pub fn is_durability_fenced(&self) -> bool {
        self.inner.read().is_durability_fenced()
    }

    /// RFC-0047 P1.1/P1.2: the first fence's report (I/O error, retryability
    /// class, uncertain range), if this Db was ever fenced.
    #[must_use]
    pub fn fence_report(&self) -> Option<crate::db::FenceReport> {
        self.inner.read().fence_report().cloned()
    }

    /// RFC-0047 P1.1: assisted close+replay+reopen after a durability
    /// fence (kernel [`crate::db::Db::recover_from_fence`] does the swap;
    /// the shared caches/published watermark are carried over, so this
    /// handle and its session write-sync policy stay valid).
    /// `Ok(None)` = not fenced (no-op). Write-group phase diagnostics keep
    /// pointing at the pre-fence stats (diagnostic only).
    ///
    /// # Errors
    /// Drain timeout (a commit is still in flight) or reopen I/O — then
    /// the Db is unusable; drop this handle.
    pub fn recover_from_fence(&self) -> Result<Option<crate::db::FenceRecovery>> {
        if !self.inner.read().is_durability_fenced() {
            return Ok(None);
        }
        // Post-fence commits fail fast at ensure_not_fenced; bounded drain
        // so the reopen never races a mid-write WAL handle.
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.inner.read().commit_inflight() > 0 {
            if Instant::now() > deadline {
                return Err(CoreError::Internal(
                    "commit_batch still in flight at recover_from_fence".into(),
                ));
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        self.inner.write().recover_from_fence()
    }

    /// Current default write-sync (WAL `fdatasync` before Ok when true).
    #[must_use]
    pub fn default_write_sync(&self) -> bool {
        self.default_sync.load(Ordering::Relaxed)
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
        self.apply_batch_vec(ops.into_iter().collect())
    }

    /// Owned fast path: callers that already hold the ops `Vec` (compat
    /// `write_cf_owned` et al.) skip the identity `collect()` of
    /// [`ConcurrentDb::apply_batch`] — one alloc + memcpy per batch.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch_vec(&self, ops: Vec<BatchOp>) -> Result<SequenceNumber> {
        let do_sync = self.resolve_sync(WriteOptions::default());
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
        // F212: explicit flush is a CHANGELOG persist point (same tail as
        // `Db::flush`) — the rotate above dropped the WAL rebuild source
        // for the flushed keys.
        g.persist_changelog_after_explicit_flush();
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
            // Arc snapshot: the table stays immutable once parked (fold swaps
            // pairs wholesale), so the SST write needs no deep clone.
            let Some(imm) = g.parked_front_arc() else {
                return false;
            };
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
            let expect = Arc::as_ptr(&imm);
            let mut g = self.inner.write();
            g.apply_l0_install(table, file_num);
            let popped = g.take_oldest_parked_matching(expect);
            drop(imm);
            if let Some(popped) = popped {
                // Only this Arc remains (fold cannot run under the lock).
                let owned = Arc::try_unwrap(popped).unwrap_or_else(|a| (*a).clone());
                g.retire_mem_as_l0_cache(owned);
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
        match wrote {
            Ok(()) => Ok(()),
            // F196: CURRENT swung — the new inventory is committed on disk;
            // restoring the unsynced bookkeeping would fight it. Fence.
            Err(e @ CoreError::ManifestCommittedUnsynced { .. }) => {
                self.inner.write().fence_durability_post_commit(&e);
                Ok(())
            }
            Err(e) => {
                self.inner.write().restore_unsynced_ssts(paths);
                Err(e)
            }
        }
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
        match wrote {
            Err(e @ CoreError::ManifestCommittedUnsynced { .. }) => {
                // F196: CURRENT swung — the compact IS committed on disk.
                // Keep the installed inventory (undoing would put memory
                // behind disk) and fence; same shape as compact_vlog_promote.
                g.fence_durability_post_commit(&e);
            }
            Err(_) => {
                g.undo_prepared_l0_compact(undo);
                return false;
            }
            Ok(()) => {}
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
        // F205: serialize the copy loop with the off-lock MANIFEST persisters
        // (`persist_unsynced_l0s_off_lock` / `install_prepared_l0_off_lock`):
        // they swing CURRENT and GC older MANIFEST-* without the Db write
        // lock, so a concurrent copy can seal a CURRENT whose MANIFEST the
        // GC deletes before the copy loop reaches it (dest reopen: missing
        // MANIFEST).
        let _persist = self.persist_lock.lock();
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
    #[must_use]
    pub fn begin_occ(&self) -> OccTransaction<E> {
        OccTransaction::new(self.clone())
    }

    /// OCC commit: validate under the write lock, then the same group-commit
    /// path as [`Self::apply_batch`] (`fdatasync` **off** the lock).
    ///
    /// Write-set keys are validated **by reference** from `ops` (no clone);
    /// the read set moves in as-is. Single-writer fast path (no publish since
    /// the snapshot) skips the walk entirely.
    ///
    /// # Errors
    /// [`CoreError::TransactionConflict`], snapshot-too-old, or WAL I/O.
    pub fn apply_batch_occ(
        &self,
        snapshot: SequenceNumber,
        read_set: impl IntoIterator<Item = Bytes>,
        ops: Vec<BatchOp>,
    ) -> Result<SequenceNumber> {
        if ops.is_empty() {
            return Ok(self.last_sequence());
        }
        let do_sync = self.resolve_sync(WriteOptions::default());
        let keys: Vec<Bytes> = read_set.into_iter().collect();
        self.writes
            .submit_occ(&self.inner, ops, do_sync, snapshot, keys)
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
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, AtomicOrdering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-concurrent-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn async_tail_below_64kib_recovers_on_close_without_fsync() {
        let dir = temp_dir();
        {
            let db = ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                                        wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap();
            // 8 × 1 KiB ≪ 64 KiB: no write() required until close.
            let payload = vec![b'y'; 1024];
            for i in 0..8u8 {
                db.put_with([b't', i], &payload, WriteOptions::no_sync())
                    .unwrap();
            }
            db.close().unwrap();
        }
        let db = ConcurrentDb::open(&dir).unwrap();
        let payload = vec![b'y'; 1024];
        for i in 0..8u8 {
            assert_eq!(
                db.get(&[b't', i]).as_deref(),
                Some(payload.as_slice()),
                "close must drain async tail without fsync, t/{i}"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_ok_write_wal_without_fsync_survives_reopen() {
        let dir = temp_dir();
        {
            let db = ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                                        wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap();
            // 80 × 1 KiB fills the 64 KiB Rocks-shaped async buffer.
            let payload = vec![b'x'; 1024];
            for i in 0..80u16 {
                let k = [(i >> 8) as u8, i as u8];
                db.put_with(k, &payload, WriteOptions::no_sync()).unwrap();
            }
            let n = fs::metadata(dir.join(crate::db::WAL_FILE_NAME))
                .map(|m| m.len())
                .unwrap_or(0);
            assert!(
                n >= crate::wal::format::ASYNC_WAL_BUFFER as u64,
                "async must write() at 64 KiB, got {n}"
            );
            db.close().unwrap();
        }
        let db = ConcurrentDb::open(&dir).unwrap();
        let payload = vec![b'x'; 1024];
        for i in 0..80u16 {
            let k = [(i >> 8) as u8, i as u8];
            assert_eq!(
                db.get(&k).as_deref(),
                Some(payload.as_slice()),
                "lost async put {i} after close without fsync"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_concurrent_writers_recover() {
        let dir = temp_dir();
        const THREADS: u8 = 8;
        const PER: u8 = 24; // 8 × 24 × ~1 KiB ≫ 64 KiB async buffer
        {
            let db = ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                                        wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap();
            let payload = vec![b'm'; 1024];
            // Concurrent async writers: default = per-writer `commit_async_ops`
            // (Rocks shape; RFC-0044 P0.5 A/B killed the single-leader merge).
            // `PEDRA_ASYNC_GROUP=1` routes them through the group instead.
            // Both paths must encode before Ok and recover after close with
            // no `fdatasync` anywhere.
            std::thread::scope(|s| {
                for t in 0..THREADS {
                    let db = &db;
                    let payload = &payload;
                    s.spawn(move || {
                        for i in 0..PER {
                            db.put_with([b'g', t, i], payload, WriteOptions::no_sync())
                                .unwrap();
                        }
                    });
                }
            });
            db.close().unwrap();
        }
        let db = ConcurrentDb::open(&dir).unwrap();
        let payload = vec![b'm'; 1024];
        for t in 0..THREADS {
            for i in 0..PER {
                assert_eq!(
                    db.get(&[b'g', t, i]).as_deref(),
                    Some(payload.as_slice()),
                    "lost concurrent async put t{t}/{i}"
                );
            }
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_and_sync_concurrent_writers_recover() {
        let dir = temp_dir();
        const PER: u8 = 16; // each thread 16 × 1 KiB; sync member forces fd
        {
            let db = ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                                        wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap();
            let payload = vec![b's'; 1024];
            std::thread::scope(|s| {
                let async_t = s.spawn(|| {
                    for i in 0..PER {
                        db.put_with([b'h', 0, i], &payload, WriteOptions::no_sync())
                            .unwrap();
                    }
                });
                let sync_t = s.spawn(|| {
                    for i in 0..PER {
                        db.put_with([b'h', 1, i], &payload, WriteOptions::sync())
                            .unwrap();
                    }
                });
                async_t.join().unwrap();
                sync_t.join().unwrap();
            });
            db.close().unwrap();
        }
        let db = ConcurrentDb::open(&dir).unwrap();
        let payload = vec![b's'; 1024];
        for t in 0..2u8 {
            for i in 0..PER {
                assert_eq!(
                    db.get(&[b'h', t, i]).as_deref(),
                    Some(payload.as_slice()),
                    "lost mixed-group put t{t}/{i}"
                );
            }
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_1ops_coalesce_and_recover() {
        let dir = temp_dir();
        let payload = vec![b'k'; 256];
        {
            let db = ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                                        wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: false,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap();
            for i in 0..40u8 {
                db.put_with([b'q', i], &payload, WriteOptions::no_sync())
                    .unwrap();
            }
            db.sync().unwrap();
            db.close().unwrap();
        }
        let db = ConcurrentDb::open(&dir).unwrap();
        for i in 0..40u8 {
            assert_eq!(
                db.get(&[b'q', i]).as_deref(),
                Some(payload.as_slice()),
                "key q/{i}"
            );
        }
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    fn open_sync(dir: &std::path::Path) -> ConcurrentDb {
        ConcurrentDb::open_with(
            dir,
            OpenOptions {
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
        assert!(db.has_imm(), "has_imm is the host notify predicate");
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

    /// Fold version GC off (core default): every superseded version survives
    /// a parked fold (F20 keep-history).
    #[test]
    fn fold_without_gc_keeps_every_version() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        for round in 0..2u64 {
            for i in 0..50u64 {
                db.put(b"hot", format!("r{round}i{i}").into_bytes())
                    .unwrap();
            }
            assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
            assert!(db.park_imm_once());
        }
        assert_eq!(db.parked_unflushed_count(), 2);
        assert!(db.fold_parked_once_off_lock());
        // All 100 versions still readable at their snapshot seqs.
        assert_eq!(db.with_read(|d| d.count_mem_versions(b"hot")), 100);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Fold version GC on (rust-rocksdb snapshot-list semantics): superseded
    /// versions below the floor collapse, current reads stay exact, and the
    /// read below the floor fails closed (SnapshotTooOld), never silent-wrong.
    #[test]
    fn fold_with_gc_collapses_below_floor() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        for round in 0..2u64 {
            for i in 0..50u64 {
                db.put(b"hot", format!("r{round}i{i}").into_bytes())
                    .unwrap();
            }
            assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
            assert!(db.park_imm_once());
        }
        db.set_fold_version_gc(true);
        assert!(db.fold_parked_once_off_lock());
        assert_eq!(
            db.with_read(|d| d.count_mem_versions(b"hot")),
            1,
            "no pins/OCC: only the newest version survives"
        );
        assert_eq!(
            db.get(b"hot").as_deref(),
            Some(&b"r1i49"[..]),
            "current read is exact after GC"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A pin taken before the fold keeps every version it can read; the
    /// watermark never passes an open pin.
    #[test]
    fn fold_with_gc_respects_open_pin() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        for i in 0..50u64 {
            db.put(b"hot", format!("v{i}").into_bytes()).unwrap();
        }
        let pinned_seq = db.with_read(|d| d.visible_sequence());
        let pin = db.inner.write().pin_snapshot();
        for i in 50..100u64 {
            db.put(b"hot", format!("v{i}").into_bytes()).unwrap();
        }
        for round in 0..2u64 {
            for i in 0..50u64 {
                db.put(b"filler", format!("f{round}i{i}").into_bytes())
                    .unwrap();
            }
            assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
            assert!(db.park_imm_once());
        }
        db.set_fold_version_gc(true);
        assert!(db.fold_parked_once_off_lock());
        // Version at (or below) the pin is the newest-≤-floor: kept.
        let got = db
            .get_at(crate::db::Snapshot::at(pinned_seq), b"hot")
            .unwrap();
        assert_eq!(got.as_deref(), Some(&b"v49"[..]), "pin read is exact");
        drop(pin);
        // With the pin gone the next fold can collapse under it.
        assert!(db.fold_parked_once_off_lock() || db.parked_unflushed_count() < 2);
        let _ = fs::remove_dir_all(&dir);
    }

    /// An open OCC transaction holds the fold-GC floor: its snapshot reads
    /// stay exact while it is live.
    #[test]
    fn fold_with_gc_respects_open_occ_transaction() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        for i in 0..50u64 {
            db.put(b"hot", format!("v{i}").into_bytes()).unwrap();
        }
        let mut tx = db.begin_occ();
        let tx_snapshot = tx.snapshot();
        for i in 50..100u64 {
            db.put(b"hot", format!("v{i}").into_bytes()).unwrap();
        }
        for round in 0..2u64 {
            for i in 0..50u64 {
                db.put(b"filler", format!("f{round}i{i}").into_bytes())
                    .unwrap();
            }
            assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
            assert!(db.park_imm_once());
        }
        db.set_fold_version_gc(true);
        assert!(db.fold_parked_once_off_lock());
        let got = tx.get(b"hot").unwrap();
        assert_eq!(
            got.as_deref(),
            Some(&b"v49"[..]),
            "OCC read at its snapshot is exact across a GC fold"
        );
        tx.abort();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Pairwise parked fold: two tables become one, no SST, both keys stay
    /// on get + scan, WAL rotate still waits (G1).
    #[test]
    fn fold_parked_once_merges_two_tables_and_keeps_scan() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.set_defer_auto_compact(true);
        db.put(b"k", vec![b'v'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.park_imm_once());
        db.put(b"j", vec![b'w'; 64]).unwrap();
        assert!(db.with_write(|d| d.stage_flush_imm()).unwrap());
        assert!(db.park_imm_once());
        assert_eq!(db.parked_unflushed_count(), 2);
        assert_eq!(db.sst_count(), 0);
        assert!(db.fold_parked_once_off_lock());
        assert_eq!(
            db.parked_unflushed_count(),
            1,
            "pair collapses to one BTree"
        );
        assert_eq!(db.sst_count(), 0, "fold must not write SST");
        assert!(
            !db.fold_parked_once_off_lock(),
            "single parked table is a no-op"
        );
        assert_eq!(db.get(b"k").as_deref(), Some(&[b'v'; 64][..]));
        assert_eq!(db.get(b"j").as_deref(), Some(&[b'w'; 64][..]));
        let scanned = db.scan_collect(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded);
        assert!(
            scanned
                .iter()
                .any(|(k, v)| k.as_ref() == b"k" && v.as_ref() == [b'v'; 64]),
            "scan must see first parked key after fold"
        );
        assert!(
            scanned
                .iter()
                .any(|(k, v)| k.as_ref() == b"j" && v.as_ref() == [b'w'; 64]),
            "scan must see second parked key after fold"
        );
        let wal_before = db.stats().wal_bytes;
        db.rotate_wal_if_writers_idle().unwrap();
        assert_eq!(
            db.stats().wal_bytes,
            wal_before,
            "fold is not an L0; rotate must wait (G1)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// After Ok the write lock is free so the host can park without waiting
    /// on a post-fd apply/publish lock (RFC-0041).
    #[test]
    fn put_ok_does_not_hold_write_lock() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.put(b"k", b"v").unwrap();
        let t0 = Instant::now();
        db.with_write(|d| {
            assert_eq!(d.get(b"k").as_deref(), Some(&b"v"[..]));
        });
        assert!(
            t0.elapsed() < Duration::from_millis(50),
            "Ok must return the write lock before the caller continues"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// After Ok, default snapshot equals the assigned sequence (publish
    /// happens after `fdatasync`, G1).
    #[test]
    fn get_after_ok_sees_published_seq() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.put(b"k", b"v").unwrap();
        assert_eq!(
            db.snapshot().sequence(),
            db.last_sequence(),
            "Ok must publish the assigned seq"
        );
        assert_eq!(db.visible_sequence(), db.last_sequence());
        assert_eq!(db.get(b"k").as_deref(), Some(&b"v"[..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0047 P1.1 test env: one-shot WAL write / sync failures. The full
    /// `FailingEnv` lives in pedradb-sim (not a core dependency); this is
    /// the minimal fault surface the fence path needs.
    #[derive(Clone)]
    struct FenceEnv {
        inner: StdEnv,
        fail_write: std::rc::Rc<std::cell::Cell<bool>>,
        fail_sync: std::rc::Rc<std::cell::Cell<bool>>,
        /// ErrorKind of the injected write failure (default: Other).
        write_kind: std::rc::Rc<std::cell::Cell<std::io::ErrorKind>>,
    }

    impl FenceEnv {
        fn new() -> Self {
            Self {
                inner: StdEnv,
                fail_write: std::rc::Rc::new(std::cell::Cell::new(false)),
                fail_sync: std::rc::Rc::new(std::cell::Cell::new(false)),
                write_kind: std::rc::Rc::new(std::cell::Cell::new(std::io::ErrorKind::Other)),
            }
        }
    }

    struct FenceFile {
        inner: fs::File,
        env: FenceEnv,
    }

    impl std::io::Read for FenceFile {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl std::io::Write for FenceFile {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.env.fail_write.replace(false) {
                return Err(std::io::Error::new(
                    self.env.write_kind.get(),
                    "injected wal write failure",
                ));
            }
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
    impl std::io::Seek for FenceFile {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }
    impl crate::env::EnvFile for FenceFile {
        fn sync_data(&mut self) -> std::io::Result<()> {
            if self.env.fail_sync.replace(false) {
                return Err(std::io::Error::other("injected wal sync failure"));
            }
            self.inner.sync_data()
        }
        fn sync_all(&mut self) -> std::io::Result<()> {
            if self.env.fail_sync.replace(false) {
                return Err(std::io::Error::other("injected wal sync failure"));
            }
            self.inner.sync_all()
        }
        fn set_len(&mut self, len: u64) -> std::io::Result<()> {
            self.inner.set_len(len)
        }
        fn len(&mut self) -> std::io::Result<u64> {
            Ok(self.inner.metadata()?.len())
        }
    }

    impl crate::env::Env for FenceEnv {
        type File = FenceFile;
        fn create_dir_all(&self, path: &std::path::Path) -> std::io::Result<()> {
            self.inner.create_dir_all(path)
        }
        fn create(&self, path: &std::path::Path) -> std::io::Result<Self::File> {
            Ok(FenceFile {
                inner: self.inner.create(path)?,
                env: self.clone(),
            })
        }
        fn open_append(&self, path: &std::path::Path) -> std::io::Result<Self::File> {
            Ok(FenceFile {
                inner: self.inner.open_append(path)?,
                env: self.clone(),
            })
        }
        fn open_read(&self, path: &std::path::Path) -> std::io::Result<Self::File> {
            Ok(FenceFile {
                inner: self.inner.open_read(path)?,
                env: self.clone(),
            })
        }
        fn sync_dir(&self, path: &std::path::Path) -> std::io::Result<()> {
            self.inner.sync_dir(path)
        }
        fn read_dir_names(&self, path: &std::path::Path) -> std::io::Result<Vec<String>> {
            self.inner.read_dir_names(path)
        }
        fn remove_file(&self, path: &std::path::Path) -> std::io::Result<()> {
            self.inner.remove_file(path)
        }
        fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
            self.inner.rename(from, to)
        }
        fn exists(&self, path: &std::path::Path) -> bool {
            self.inner.exists(path)
        }
        fn metadata_len(&self, path: &std::path::Path) -> std::io::Result<u64> {
            self.inner.metadata_len(path)
        }
    }

    /// RFC-0047 P1.1: a durability fence refuses writes and
    /// `recover_from_fence()` (close+replay+reopen) reports the uncertain
    /// in-flight range. Two adjudications: (A) the WAL write itself failed
    /// → the write is lost; (B) the write landed but the sync failed → the
    /// uncertain write is actually there (the ack-vs-durability gap the
    /// report makes visible). Never silent in either direction.
    #[test]
    fn resume_after_fence_reports_uncertain_range() {
        // (A) write fails → fence → resume proves the write lost.
        let dir = temp_dir();
        let env = FenceEnv::new();
        let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), env.clone()).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        assert_eq!(db.visible_sequence(), 2);
        env.fail_write.set(true);
        assert!(db.put(b"c", b"3").is_err(), "injected WAL write failure");
        assert!(
            matches!(db.put(b"x", b"y"), Err(CoreError::DurabilityFenced)),
            "post-fence writes must refuse"
        );
        let rec = db.recover_from_fence().unwrap().expect("fenced");
        assert_eq!(rec.fence.uncertain_from, 3);
        assert_eq!(rec.fence.uncertain_through, 3);
        assert_eq!(rec.replayed_through, 2, "replay ends at the durable prefix");
        assert!(rec.lost_writes);
        assert!(!rec.fence.io_error.is_empty());
        assert_eq!(
            rec.fence.class,
            crate::db::FenceClass::Persistent,
            "generic write failure is not retryable"
        );
        // Resumed: healthy, the lost write stays lost, new writes land.
        db.put(b"d", b"4").unwrap();
        assert_eq!(db.get(b"d").as_deref(), Some(&b"4"[..]));
        assert_eq!(db.get(b"c"), None);
        assert_eq!(db.get(b"a").as_deref(), Some(&b"1"[..]));
        assert_eq!(db.get(b"b").as_deref(), Some(&b"2"[..]));
        // Not fenced anymore: a second recover is a no-op.
        assert!(db.recover_from_fence().unwrap().is_none());
        drop(db);
        let _ = fs::remove_dir_all(&dir);

        // (B) sync fails after the write landed → the uncertain write IS
        // there after resume (client saw Err; report explains).
        let dir = temp_dir();
        let env = FenceEnv::new();
        let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), env.clone()).unwrap();
        db.put(b"a", b"1").unwrap();
        assert_eq!(db.visible_sequence(), 1);
        env.fail_sync.set(true);
        assert!(db.put(b"b", b"2").is_err(), "injected WAL sync failure");
        let rec = db.recover_from_fence().unwrap().expect("fenced");
        assert_eq!(rec.fence.uncertain_from, 2);
        assert_eq!(rec.fence.uncertain_through, 2);
        assert_eq!(
            rec.replayed_through, 2,
            "frame reached the file before the sync error"
        );
        assert!(!rec.lost_writes);
        assert_eq!(db.get(b"b").as_deref(), Some(&b"2"[..]));
        drop(db);
        let _ = fs::remove_dir_all(&dir);

        // (C) ENOSPC write failure → typed Transient class (RFC-0047 P1.2:
        // the class a host auto-resume policy programs on).
        let dir = temp_dir();
        let env = FenceEnv::new();
        env.write_kind.set(std::io::ErrorKind::StorageFull);
        let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), env.clone()).unwrap();
        db.put(b"a", b"1").unwrap();
        env.fail_write.set(true);
        assert!(db.put(b"b", b"2").is_err(), "injected ENOSPC write failure");
        let report = db.fence_report().expect("fenced");
        assert_eq!(report.class, crate::db::FenceClass::Transient);
        assert!(db.is_durability_fenced());
        let rec = db.recover_from_fence().unwrap().expect("fenced");
        assert!(rec.lost_writes);
        db.put(b"c", b"3").unwrap();
        assert_eq!(db.get(b"c").as_deref(), Some(&b"3"[..]));
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Group apply must bump the point-cache generation once so a get after
    /// a multi-member group is not a stale hit (RFC-0041 apply_mc4 path).
    #[test]
    fn group_apply_invalidates_point_cache() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.put(b"k", b"old").unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(&b"old"[..]));
        db.apply_batch([BatchOp::put(b"k".as_slice(), b"new".as_slice())])
            .unwrap();
        assert_eq!(
            db.get(b"k").as_deref(),
            Some(&b"new"[..]),
            "point cache must miss after group apply"
        );
        db.apply_batch([
            BatchOp::put(b"a".as_slice(), b"1".as_slice()),
            BatchOp::put(b"b".as_slice(), b"2".as_slice()),
        ])
        .unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(&b"1"[..]));
        assert_eq!(db.get(b"b").as_deref(), Some(&b"2"[..]));
        // 64-op apply skips per-key dirty clones and gen-bumps at publish.
        db.put(b"hot", b"old").unwrap();
        assert_eq!(db.get(b"hot").as_deref(), Some(&b"old"[..]));
        let mut fat = vec![BatchOp::put(b"hot".as_slice(), b"new".as_slice())];
        for i in 0..63u8 {
            fat.push(BatchOp::put([b'x', i], [b'v', i]));
        }
        db.apply_batch(fat).unwrap();
        assert_eq!(
            db.get(b"hot").as_deref(),
            Some(&b"new"[..]),
            "fat apply must not leave a stale point-cache hit"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn count_in_range_cache_matches_locked_and_invalidates() {
        use std::ops::Bound;
        let dir = temp_dir();
        let db = open_sync(&dir);
        for i in 0..16u8 {
            db.put([b'k', i], [b'v', i]).unwrap();
        }
        let n1 = db
            .count_in_range(Bound::Unbounded, Bound::Unbounded, Some(25))
            .unwrap();
        assert_eq!(n1, 16);
        let n2 = db
            .count_in_range(Bound::Unbounded, Bound::Unbounded, Some(25))
            .unwrap();
        assert_eq!(n2, 16, "second count must hit the shared cache");
        db.put(b"kz", b"new").unwrap();
        let n3 = db
            .count_in_range(Bound::Unbounded, Bound::Unbounded, Some(25))
            .unwrap();
        assert_eq!(n3, 17, "publish must invalidate the count cache");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Four clients each apply a fat pre+com pair; every key is visible and
    /// WAL-durable (G1). Host is not notified per write — grouping still
    /// amortizes fsyncs.
    #[test]
    fn four_client_fat_apply_is_durable_without_per_write_wakeup() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        let n_clients = 4usize;
        let n_ops = 8usize;
        std::thread::scope(|s| {
            for c in 0..n_clients {
                let db = Arc::clone(&db);
                s.spawn(move || {
                    for i in 0..n_ops {
                        let mut pre = Vec::with_capacity(32);
                        let mut com = Vec::with_capacity(32);
                        for k in 0..16u16 {
                            let mut key = vec![b'p', c as u8];
                            key.extend_from_slice(&(i as u16).to_be_bytes());
                            key.extend_from_slice(&k.to_be_bytes());
                            pre.push(BatchOp::put(key.clone(), vec![b'v'; 64]));
                            com.push(BatchOp::put(key, b"c".as_slice()));
                        }
                        db.apply_batch(pre).unwrap();
                        db.apply_batch(com).unwrap();
                    }
                });
            }
        });
        assert!(
            db.wal_sync_count() >= 1,
            "fat apply must still fdatasync (G1)"
        );
        for c in 0..n_clients {
            for i in 0..n_ops {
                let mut key = vec![b'p', c as u8];
                key.extend_from_slice(&(i as u16).to_be_bytes());
                key.extend_from_slice(&0u16.to_be_bytes());
                assert_eq!(
                    db.get(&key).as_deref(),
                    Some(&b"c"[..]),
                    "missing apply c={c} i={i}"
                );
            }
        }
        drop(db);
        let re = open_sync(&dir);
        let mut key = vec![b'p', 0];
        key.extend_from_slice(&0u16.to_be_bytes());
        key.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(
            re.get(&key).as_deref(),
            Some(&b"c"[..]),
            "reopen must see fat apply"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Fat batches skip the catch-up wait; members that queue while the
    /// leader applies still share one `fdatasync` (post-apply absorb).
    #[test]
    fn fat_apply_late_join_after_apply_shares_fsync() {
        let dir = temp_dir();
        let db = Arc::new(open_sync(&dir));
        let n = 4usize;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        std::thread::scope(|s| {
            for c in 0..n {
                let db = Arc::clone(&db);
                let barrier = Arc::clone(&barrier);
                s.spawn(move || {
                    let ops: Vec<_> = (0..64u16)
                        .map(|k| {
                            let mut key = vec![b'J', c as u8];
                            key.extend_from_slice(&k.to_be_bytes());
                            BatchOp::put(key, vec![b'x'; 32])
                        })
                        .collect();
                    barrier.wait();
                    db.apply_batch(ops).unwrap();
                });
            }
        });
        assert!(
            db.wal_sync_count() < n as u64,
            "post-apply join must share fdatasync: syncs={} clients={n}",
            db.wal_sync_count()
        );
        assert!(db.wal_sync_count() >= 1, "G1: at least one fdatasync");
        drop(db);
        let re = open_sync(&dir);
        for c in 0..n {
            let mut key = vec![b'J', c as u8];
            key.extend_from_slice(&0u16.to_be_bytes());
            assert_eq!(
                re.get(&key).as_deref(),
                Some(&[b'x'; 32][..]),
                "reopen c={c}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Two sequential groups: WAL seq order is preserved so reopen (and
    /// the change-feed max-seq check) sees every key.
    #[test]
    fn two_groups_reopen_sees_all_keys() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        db.apply_batch([
            BatchOp::put(b"a".as_slice(), b"1".as_slice()),
            BatchOp::put(b"b".as_slice(), b"2".as_slice()),
        ])
        .unwrap();
        db.apply_batch([BatchOp::put(b"c".as_slice(), b"3".as_slice())])
            .unwrap();
        assert!(db.wal_sync_count() >= 2, "each group fdatasyncs (G1)");
        drop(db);
        let re = open_sync(&dir);
        assert_eq!(re.get(b"a").as_deref(), Some(&b"1"[..]));
        assert_eq!(re.get(b"b").as_deref(), Some(&b"2"[..]));
        assert_eq!(re.get(b"c").as_deref(), Some(&b"3"[..]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fat_apply_get_uses_tail_index_without_stage() {
        let dir = temp_dir();
        let db = open_sync(&dir);
        let mut ops = Vec::with_capacity(64);
        for i in 0..64u32 {
            let k = format!("k{i:04}");
            ops.push(BatchOp::put(k.into_bytes(), b"v".to_vec()));
        }
        db.apply_batch(ops).unwrap();
        assert_eq!(db.get(b"k0000").as_deref(), Some(&b"v"[..]));
        assert_eq!(db.get(b"k0063").as_deref(), Some(&b"v"[..]));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Park must be able to take the write lock while a fat group is
    /// `fdatasync`ing off it (RFC-0041: do not delay park).
    #[test]
    fn park_imm_runs_during_fat_apply() {
        let dir = temp_dir();
        let db = Arc::new(
            ConcurrentDb::open_with(
                &dir,
                OpenOptions {
                                        wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: Some(8 * 1024),
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap(),
        );
        db.set_defer_auto_compact(true);
        let stop = Arc::new(AtomicUsize::new(0));
        let writer = {
            let db = Arc::clone(&db);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                for i in 0..80u16 {
                    if stop.load(Ordering::Relaxed) > 0 {
                        break;
                    }
                    let ops: Vec<_> = (0..64u16)
                        .map(|k| {
                            let mut key = i.to_be_bytes().to_vec();
                            key.extend_from_slice(&k.to_be_bytes());
                            BatchOp::put(key, vec![b'x'; 128])
                        })
                        .collect();
                    db.apply_batch(ops).unwrap();
                }
            })
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut parked = 0usize;
        while std::time::Instant::now() < deadline {
            if db.has_imm() && db.park_imm_once() {
                parked = parked.saturating_add(1);
            }
            if parked >= 1 {
                break;
            }
            thread::yield_now();
        }
        stop.store(1, Ordering::Relaxed);
        writer.join().unwrap();
        assert!(
            parked >= 1 || db.parked_unflushed_count() >= 1,
            "host must park while the leader fdatasyncs off the write lock"
        );
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
        assert_eq!(CATCHUP_WINDOW_DEFAULT, Duration::from_micros(50));

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

    /// RFC-0042 P1.1 — pure break-even policy for the catch-up window.
    #[test]
    fn catchup_bound_policy() {
        let w = Duration::from_micros(50);
        let fd = Duration::from_micros(25);
        // Knob off (PEDRA_CATCHUP_US=0) never waits.
        assert_eq!(catchup_wait_bound(Duration::ZERO, fd, 1, 2, 1), None);
        // Every active writer already queued: nothing to wait for.
        assert_eq!(catchup_wait_bound(w, fd, 2, 2, 1), None);
        assert_eq!(catchup_wait_bound(w, fd, 3, 2, 1), None);
        // Break-even: fd/2 caps the configured window for 1-op puts.
        assert_eq!(
            catchup_wait_bound(w, fd, 1, 2, 1),
            Some(Duration::from_nanos(12_500))
        );
        assert_eq!(
            catchup_wait_bound(w, fd, 1, 8, 1),
            Some(Duration::from_nanos(12_500))
        );
        // The window stays the ceiling when the fd is much slower.
        assert_eq!(
            catchup_wait_bound(w, Duration::from_millis(5), 1, 2, 1),
            Some(w)
        );
        assert_eq!(
            catchup_wait_bound(Duration::from_micros(10), fd, 1, 4, 1),
            Some(Duration::from_micros(10))
        );
        // Raftlog-sized group: wait the full window, not fd/2.
        assert_eq!(catchup_wait_bound(w, fd, 1, 4, 16), Some(w));
        assert_eq!(
            catchup_wait_bound(Duration::from_micros(25), fd, 1, 4, 16),
            Some(Duration::from_micros(25))
        );
        // redis-benchmark -c 50 / 16: full window (async 1-op skips grouping).
        assert_eq!(catchup_wait_bound(w, fd, 1, 50, 1), Some(w));
        assert_eq!(catchup_wait_bound(w, fd, 1, 16, 1), Some(w));
        // mc4 stays on the fd/2 cap (do not regress official A/F_mc4).
        assert_eq!(
            catchup_wait_bound(w, fd, 1, 4, 1),
            Some(Duration::from_nanos(12_500))
        );
    }

    /// Test env whose file `fdatasync` sleeps a configurable time: makes the
    /// WAL fd slow deterministically for the catch-up bound test (no faults).
    #[derive(Clone)]
    struct SlowFdEnv {
        sleep_us: Arc<AtomicU64>,
    }

    struct SlowFdFile {
        file: std::fs::File,
        sleep_us: Arc<AtomicU64>,
    }

    impl std::io::Read for SlowFdFile {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.file.read(buf)
        }
    }

    impl std::io::Write for SlowFdFile {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.file.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }

    impl std::io::Seek for SlowFdFile {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.file.seek(pos)
        }
    }

    impl crate::env::EnvFile for SlowFdFile {
        fn sync_data(&mut self) -> std::io::Result<()> {
            let us = self.sleep_us.load(Ordering::Relaxed);
            if us > 0 {
                thread::sleep(Duration::from_micros(us));
            }
            crate::env::fdatasync_file(&self.file)
        }

        fn sync_all(&mut self) -> std::io::Result<()> {
            std::fs::File::sync_all(&self.file)
        }

        fn set_len(&mut self, len: u64) -> std::io::Result<()> {
            self.file.set_len(len)
        }

        fn len(&mut self) -> std::io::Result<u64> {
            use std::io::Seek;
            let pos = self.file.stream_position()?;
            let end = self.file.seek(std::io::SeekFrom::End(0))?;
            self.file.seek(std::io::SeekFrom::Start(pos))?;
            Ok(end)
        }
    }

    impl Env for SlowFdEnv {
        type File = SlowFdFile;

        fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
            StdEnv.create_dir_all(path)
        }

        fn create(&self, path: &Path) -> std::io::Result<Self::File> {
            Ok(SlowFdFile {
                file: StdEnv.create(path)?,
                sleep_us: Arc::clone(&self.sleep_us),
            })
        }

        fn open_append(&self, path: &Path) -> std::io::Result<Self::File> {
            Ok(SlowFdFile {
                file: StdEnv.open_append(path)?,
                sleep_us: Arc::clone(&self.sleep_us),
            })
        }

        fn open_read(&self, path: &Path) -> std::io::Result<Self::File> {
            Ok(SlowFdFile {
                file: StdEnv.open_read(path)?,
                sleep_us: Arc::clone(&self.sleep_us),
            })
        }

        fn sync_dir(&self, path: &Path) -> std::io::Result<()> {
            StdEnv.sync_dir(path)
        }

        fn read_dir_names(&self, path: &Path) -> std::io::Result<Vec<String>> {
            StdEnv.read_dir_names(path)
        }

        fn remove_file(&self, path: &Path) -> std::io::Result<()> {
            StdEnv.remove_file(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            StdEnv.rename(from, to)
        }

        fn exists(&self, path: &Path) -> bool {
            StdEnv.exists(path)
        }

        fn metadata_len(&self, path: &Path) -> std::io::Result<u64> {
            StdEnv.metadata_len(path)
        }
    }

    /// RFC-0042 P1.1 — a leader holding its group open for a straggler that
    /// is mid-`fdatasync` must give up after `fd_ema/2`, not the configured
    /// window. With the window at 500 ms and a 200 ms fd the recorded wait
    /// stays far below 100 ms (old fixed-window policy: ≈ the whole fd).
    /// End-to-end put latency is not the contract: the WAL mutex serializes
    /// the two commits regardless (single WAL file, G1), and the recorded
    /// wait includes scheduler overrun on a loaded box.
    #[test]
    fn catchup_wait_bounded_by_half_fd() {
        let dir = temp_dir();
        let sleep_us = Arc::new(AtomicU64::new(0));
        let env = SlowFdEnv {
            sleep_us: Arc::clone(&sleep_us),
        };
        let opts = OpenOptions {
                        wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        };
        let db = ConcurrentDb::open_with_env(&dir, opts.clone(), env.clone()).unwrap();
        db.set_write_group_catchup_window(Duration::from_millis(500));

        // One real (fast) lone commit seeds the fd EMA from a true sample.
        db.put(b"warm", b"v").unwrap();
        assert!(db.wal_fd_ema() > Duration::ZERO);
        assert!(
            db.wal_fd_ema() < Duration::from_millis(10),
            "seeded fd ema should be µs-class, got {:?}",
            db.wal_fd_ema()
        );
        assert_eq!(db.catchup_wait_stats(), (0, 0));

        // fd now costs ≥ 200 ms. A enters a lone commit; wait until it is
        // provably mid-commit, then B submits — B leads a group and the
        // catch-up wait engages for a straggler (A) that is counted in
        // `active` but can never queue.
        sleep_us.store(200_000, Ordering::Relaxed);
        let db_a = db.clone();
        let a = thread::spawn(move || db_a.put(b"a", b"va").unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while db.with_read(|d| d.commit_inflight()) == 0 {
            assert!(Instant::now() < deadline, "A never entered its commit");
            thread::sleep(Duration::from_micros(100));
        }
        db.put(b"b", b"vb").unwrap();
        sleep_us.store(0, Ordering::Relaxed);
        a.join().unwrap();

        // The wait ran and honored the fd/2 bound, not the 500 ms window:
        // old policy waits ≈ A's whole 200 ms fd; the bound is µs-class
        // plus scheduler overrun (ms-class on a loaded box).
        let (wait_ns, waits) = db.catchup_wait_stats();
        assert!(waits >= 1, "catch-up wait never engaged");
        let per_wait = wait_ns / waits;
        assert!(
            Duration::from_nanos(per_wait) < Duration::from_millis(100),
            "catch-up wait ignored the fd/2 bound: {per_wait} ns over {waits} waits"
        );

        // The EMA absorbed the slow fd samples (mechanism feeds the bound).
        assert!(
            db.wal_fd_ema() > Duration::from_millis(1),
            "fd ema should reflect the 200 ms syncs, got {:?}",
            db.wal_fd_ema()
        );

        // G1/G2 intact: both writes visible live and durable on reopen.
        assert_eq!(db.get(b"warm").as_deref(), Some(b"v".as_ref()));
        assert_eq!(db.get(b"a").as_deref(), Some(b"va".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"vb".as_ref()));
        drop(db);
        let re = ConcurrentDb::open_with_env(
            &dir,
            OpenOptions {
                                wal_full_fsync: true,
                sync: false,
                ..opts
            },
            env,
        )
        .unwrap();
        assert_eq!(re.get(b"warm").as_deref(), Some(b"v".as_ref()));
        assert_eq!(re.get(b"a").as_deref(), Some(b"va".as_ref()));
        assert_eq!(re.get(b"b").as_deref(), Some(b"vb".as_ref()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Leader returns its own result without an mpsc hop; followers still
    /// wait and every key is visible + durable (RFC-0041).
    #[test]
    fn leader_result_skips_mpsc_and_followers_see_keys() {
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
                    let k = [b'L', u8::try_from(i).expect("n fits u8")];
                    db.put(k, b"v").unwrap();
                });
            }
        });
        for i in 0..n {
            let k = [b'L', u8::try_from(i).expect("n fits u8")];
            assert_eq!(db.get(&k).as_deref(), Some(&b"v"[..]), "live {i}");
        }
        drop(db);
        let re = open_sync(&dir);
        for i in 0..n {
            let k = [b'L', u8::try_from(i).expect("n fits u8")];
            assert_eq!(re.get(&k).as_deref(), Some(&b"v"[..]), "reopen {i}");
        }
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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

    /// RFC-0041 P1.1: raftlog-sized (16) batches still catch-up so 4 clients
    /// share a `fdatasync`; apply-sized (64) skip the wait. drain_imm
    /// persists; keys survive reopen.
    #[test]
    fn large_batch_skips_catchup_and_flush_reopens() {
        let dir = temp_dir();
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
        assert_eq!(
            db.wal_sync_count(),
            8,
            "G1: exactly one WAL fdatasync per 1-client Ok (ceiling for ycsb_a/f)"
        );
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
        // A fat apply that itself lasts >1 ms must not look idle at Ok
        // (last-submit clock would fire; last-complete must not).
        let fat: Vec<_> = (0..64u16)
            .map(|i| BatchOp::put(i.to_be_bytes(), vec![b'x'; 1024]))
            .collect();
        db.apply_batch(fat).unwrap();
        assert!(
            !db.writes_idle_for(Duration::from_millis(1)),
            "idle clock is last Ok, not submit start of a long apply"
        );
        let rem = db.writes_until_idle(Duration::from_millis(2));
        assert!(
            rem.is_some_and(|d| d > Duration::ZERO && d <= Duration::from_millis(2)),
            "until_idle after a just-acked apply must be a short remaining wait, got {rem:?}"
        );
        std::thread::sleep(Duration::from_millis(3));
        assert!(
            db.writes_idle_for(Duration::from_millis(2)),
            "2 ms after last Ok the host may start L0 compact"
        );
        assert!(
            db.writes_until_idle(Duration::from_millis(2)).is_none(),
            "until_idle is None once idle"
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
                                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
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
