//! Cooperative PCT turnstile (RFC-0051 P0) — real OS threads, deterministic π.
//!
//! Feature `pct` only. Exactly one worker owns the "CPU" between yield
//! points; [`maybe_yield`] parks the calling worker and the scheduler (in
//! `pedradb-world::pct_concurrent`) grants the next one. Between yields a
//! worker runs alone, so model-level atomic sections are real mutual
//! exclusion — no data races, exact PCT preemption semantics at the
//! annotated sites. Production builds without the feature compile every
//! hook away.
//!
//! Hook rule: call [`maybe_yield`] **before** acquiring a lock, never while
//! holding one — a parked worker must never block the granted one.

use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A parked worker longer than this is a hook-placement bug (lock held
/// across a yield) — fail the trial loudly instead of hanging CI.
const DEADLOCK_TIMEOUT: Duration = Duration::from_secs(60);

struct Inner {
    turn: Option<usize>,
    /// One entry per parked worker: `(task, yield site)`. Invariant: a task
    /// contributes at most one entry (running tasks hold the turn instead).
    ready: Vec<(usize, &'static str)>,
    finished: usize,
}

/// CPU-token scheduler primitive shared by workers and the scheduler thread.
pub struct Turnstile {
    n: usize,
    inner: Mutex<Inner>,
    cv: Condvar,
    /// Per-group returned-seq ranges `(min, max)` of every atomic group
    /// commit finished on this turnstile (test forensics). Members of one
    /// group commit at the same atomic instant, so seq order between them
    /// is NOT serialization order; oracles use this to classify read/write
    /// windows as same-group (simultaneous) vs cross-group (ordered).
    /// Per-run state (not a global) so parallel trials never interleave.
    group_ranges: Mutex<Vec<(u64, u64)>>,
}

impl Turnstile {
    /// Turnstile for `n` worker tasks.
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self {
            n,
            inner: Mutex::new(Inner {
                turn: None,
                ready: Vec::new(),
                finished: 0,
            }),
            cv: Condvar::new(),
            group_ranges: Mutex::new(Vec::new()),
        }
    }

    /// Drain the recorded group seq-ranges (forensics; see
    /// [`Self::record_group_range`]). Called by the runner after all
    /// workers finish.
    #[must_use]
    pub fn take_group_ranges(&self) -> Vec<(u64, u64)> {
        std::mem::take(&mut *self.group_ranges.lock().expect("pct ranges mutex poisoned"))
    }

    fn wait_turn(&self, mut g: MutexGuard<'_, Inner>, task: usize) {
        let deadline = Instant::now() + DEADLOCK_TIMEOUT;
        while g.turn != Some(task) {
            let now = Instant::now();
            assert!(
                now < deadline,
                "pct turnstile deadlock: task {task} never re-granted (yield site holding a lock?)"
            );
            let (ng, _t) = self
                .cv
                .wait_timeout(g, deadline - now)
                .expect("pct turnstile mutex poisoned");
            g = ng;
        }
    }

    /// Worker start: register as ready and block until first grant.
    pub fn worker_enter(&self, task: usize) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        g.ready.push((task, "enter"));
        drop(g);
        self.cv.notify_all();
        let g = self.inner.lock().expect("pct turnstile mutex poisoned");
        self.wait_turn(g, task);
    }

    /// Yield point: park the calling worker (releases the CPU token).
    pub fn yield_point(&self, task: usize, site: &'static str) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        assert_eq!(
            g.turn,
            Some(task),
            "task {task} yielded at {site} without the CPU token"
        );
        if std::env::var_os("PCT_TRACE").is_some() {
            eprintln!("TRACE yield task={task} site={site}");
        }
        g.ready.push((task, site));
        g.turn = None;
        drop(g);
        self.cv.notify_all();
        let g = self.inner.lock().expect("pct turnstile mutex poisoned");
        self.wait_turn(g, task);
    }

    /// Worker done: release the token for the last time.
    pub fn worker_exit(&self, task: usize) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        if g.turn == Some(task) {
            g.turn = None;
        }
        g.finished += 1;
        drop(g);
        self.cv.notify_all();
    }

    /// Blocking section begin: release the CPU token **without** joining
    /// the ready queue — the worker is blocked on a real OS wait (channel
    /// recv), so it leaves the enabled set until the wait completes.
    /// Without this, a follower holding the token in `recv()` deadlocks
    /// against a parked leader (RFC-0051 P1.1).
    pub fn block_begin(&self, task: usize) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        assert_eq!(
            g.turn,
            Some(task),
            "task {task} block_begin without the CPU token"
        );
        g.turn = None;
        drop(g);
        self.cv.notify_all();
    }

    /// Blocking section end: the wait completed — re-enter the ready set
    /// at `site` and wait for the CPU again.
    pub fn block_end(&self, task: usize, site: &'static str) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        g.ready.push((task, site));
        drop(g);
        self.cv.notify_all();
        let g = self.inner.lock().expect("pct turnstile mutex poisoned");
        self.wait_turn(g, task);
    }

    /// Distinct tasks currently parked and grantable.
    pub fn ready_tasks(&self) -> Vec<usize> {
        let g = self.inner.lock().expect("pct turnstile mutex poisoned");
        let mut v: Vec<usize> = g.ready.iter().map(|&(t, _)| t).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Block the scheduler until all `n` workers have entered (parked at
    /// `"enter"`). Called once before the first grant: after the barrier
    /// every enabled set is a pure function of the grant history, so the
    /// schedule (and its hash) depends only on (seed, policy, n) — never
    /// on OS thread-start ordering.
    pub fn wait_all_entered(&self) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        let deadline = Instant::now() + DEADLOCK_TIMEOUT;
        while g.ready.len() < self.n {
            let now = Instant::now();
            assert!(
                now < deadline,
                "pct turnstile: not all {} workers ever entered",
                self.n
            );
            let (ng, _t) = self
                .cv
                .wait_timeout(g, deadline - now)
                .expect("pct turnstile mutex poisoned");
            g = ng;
        }
    }

    /// Block the scheduler until someone is grantable **and the CPU is
    /// free**, or everyone finished.
    pub fn wait_ready(&self) {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        let deadline = Instant::now() + DEADLOCK_TIMEOUT;
        while (g.ready.is_empty() || g.turn.is_some()) && g.finished < self.n {
            let now = Instant::now();
            assert!(
                now < deadline,
                "pct scheduler starved: no grantable worker with a free CPU"
            );
            let (ng, _t) = self
                .cv
                .wait_timeout(g, deadline - now)
                .expect("pct turnstile mutex poisoned");
            g = ng;
        }
    }

    /// Grant `task` the CPU; returns the yield site this grant resumes.
    pub fn grant(&self, task: usize) -> &'static str {
        let mut g = self.inner.lock().expect("pct turnstile mutex poisoned");
        if std::env::var_os("PCT_TRACE").is_some() {
            eprintln!(
                "TRACE grant task={task} turn_was={:?} ready={:?}",
                g.turn, g.ready
            );
        }
        let idx = g
            .ready
            .iter()
            .position(|&(t, _)| t == task)
            .expect("grant of a non-ready task");
        let (_, site) = g.ready.remove(idx);
        g.turn = Some(task);
        drop(g);
        self.cv.notify_all();
        site
    }

    /// Workers finished so far.
    pub fn finished(&self) -> usize {
        self.inner
            .lock()
            .expect("pct turnstile mutex poisoned")
            .finished
    }

    /// Total workers.
    #[must_use]
    pub fn n(&self) -> usize {
        self.n
    }
}

thread_local! {
    static CURRENT: RefCell<Option<(Arc<Turnstile>, usize)>> = const { RefCell::new(None) };
}

/// Bind this worker thread to `task` of `ts` (call first in the thread).
pub fn install_worker(ts: Arc<Turnstile>, task: usize) {
    CURRENT.with(|c| *c.borrow_mut() = Some((ts, task)));
}

/// Unbind the worker thread (drop turnstile references).
pub fn clear_worker() {
    CURRENT.with(|c| *c.borrow_mut() = None);
}

/// Record one atomic group commit's returned-seq range on the current
/// run's turnstile (forensics for group-aware oracles; feature `pct` only,
/// no-op off a registered worker thread).
pub fn record_group_range(lo: u64, hi: u64) {
    CURRENT.with(|c| {
        let borrow = c.borrow();
        if let Some((ts, _)) = borrow.as_ref() {
            ts.group_ranges
                .lock()
                .expect("pct ranges mutex poisoned")
                .push((lo, hi));
        }
    });
}

/// Engine yield site (feature `pct` only). No-op unless the calling thread
/// is a registered PCT worker.
pub fn maybe_yield(site: &'static str) {
    CURRENT.with(|c| {
        let (ts, task) = {
            let borrow = c.borrow();
            match borrow.as_ref() {
                Some((ts, task)) => (Arc::clone(ts), *task),
                None => return,
            }
        };
        ts.yield_point(task, site);
    });
}

/// Engine blocking section (feature `pct` only): run `f` (a real blocking
/// wait — channel recv, condvar) with the CPU token released and the
/// worker out of the enabled set; re-enter ready at `site` when it
/// completes. No-op unless the calling thread is a registered PCT worker.
pub fn blocking_section<T>(site: &'static str, f: impl FnOnce() -> T) -> T {
    CURRENT.with(|c| {
        let (ts, task) = {
            let borrow = c.borrow();
            match borrow.as_ref() {
                Some((ts, task)) => (Arc::clone(ts), *task),
                None => return f(),
            }
        };
        ts.block_begin(task);
        let out = f();
        ts.block_end(task, site);
        out
    })
}
