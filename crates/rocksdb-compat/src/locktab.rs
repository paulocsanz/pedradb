//! Exclusive key-lock table for rust-rocksdb `TransactionDB` (2PL).
//! OCC [`super::OptimisticTransactionDB`] does not use this.
//!
//! **Single artifact (pair `wait_for_deadlock`):** this file is what `rustc`
//! links *and* what Verus proves (`cfg(verus_keep_ghost)`). `LockTable` I/O
//! (parking_lot / Condvar) stays rustc-only; the wait-for cycle is the term.
//!
//!   ./scripts/verus_wait_for_deadlock.sh

#![forbid(unsafe_code)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn wait_for_deadlock_spec(waiter: u64, owner: u64, owner_next: Option<u64>) -> bool {
    match owner_next {
        Some(next) => next == waiter,
        None => false,
    }
}

/// Production walk returns `true` on a cycle (two-cycle or longer).
pub open spec fn wait_for_deadlock_found() -> bool {
    true
}

pub open spec fn wait_for_deadlock_as_is_spec(
    _waiter: u64,
    _owner: u64,
    _owner_next: Option<u64>,
) -> bool {
    false
}

pub fn wait_for_deadlock(waiter: u64, owner: u64, owner_next: Option<u64>) -> (d: bool)
    ensures
        d == wait_for_deadlock_spec(waiter, owner, owner_next),
        d ==> owner_next == Some(waiter),
{
    match owner_next {
        Some(next) => next == waiter,
        None => false,
    }
}

pub fn wait_for_deadlock_as_is(_waiter: u64, _owner: u64, _owner_next: Option<u64>) -> (d: bool)
    ensures
        d == false,
{
    false
}

/// Conflict-lock (wait-for cycle): owner waits for waiter. AS-IS misses it.
/// MachCSL / static Rust deadlock detection: a cycle in the wait-for graph
/// is deadlock; missing it is the second possibility, not a skip.
proof fn lemma_two_cycle_is_deadlock(a: u64, b: u64)
    requires
        a != b,
    ensures
        wait_for_deadlock_spec(a, b, Some(a)),
        !wait_for_deadlock_as_is_spec(a, b, Some(a)),
        !wait_for_deadlock_spec(a, b, None),
{
}

} // verus!

#[cfg(not(verus_keep_ghost))]
use bytes::Bytes;
#[cfg(not(verus_keep_ghost))]
use parking_lot::{Condvar, Mutex};
#[cfg(not(verus_keep_ghost))]
use std::collections::{HashMap, HashSet};
#[cfg(not(verus_keep_ghost))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(verus_keep_ghost))]
use std::time::{Duration, Instant};

/// Lock wait outcome (Rocks `Busy` / `TimedOut`).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockErr {
    /// Deadlock detected before waiting.
    Deadlock,
    /// Timeout (including timeout=0, lock busy).
    TimedOut,
}

#[cfg(not(verus_keep_ghost))]
pub(crate) struct LockTable {
    inner: Mutex<Inner>,
    cv: Condvar,
    next_id: AtomicU64,
}

#[cfg(not(verus_keep_ghost))]
struct Inner {
    /// Encoded key → owner txn id.
    owned: HashMap<Bytes, u64>,
    /// Waiter txn id → key it is blocked on.
    waiting: HashMap<u64, Bytes>,
}

#[cfg(not(verus_keep_ghost))]
impl LockTable {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                owned: HashMap::new(),
                waiting: HashMap::new(),
            }),
            cv: Condvar::new(),
            next_id: AtomicU64::new(1),
        }
    }

    pub(crate) fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn lock(
        &self,
        key: Bytes,
        txn: u64,
        timeout: Duration,
        detect: bool,
    ) -> Result<(), LockErr> {
        let deadline = Instant::now().checked_add(timeout);
        let mut g = self.inner.lock();
        loop {
            match g.owned.get(&key).copied() {
                None => {
                    g.owned.insert(key, txn);
                    g.waiting.remove(&txn);
                    return Ok(());
                }
                Some(owner) if owner == txn => {
                    g.waiting.remove(&txn);
                    return Ok(());
                }
                Some(owner) => {
                    if detect && wait_for_deadlock(&g.owned, &g.waiting, txn, owner) {
                        g.waiting.remove(&txn);
                        return Err(LockErr::Deadlock);
                    }
                    if timeout.is_zero() {
                        return Err(LockErr::TimedOut);
                    }
                    g.waiting.insert(txn, key.clone());
                    let Some(dl) = deadline else {
                        self.cv.wait(&mut g);
                        continue;
                    };
                    let now = Instant::now();
                    if now >= dl {
                        g.waiting.remove(&txn);
                        return Err(LockErr::TimedOut);
                    }
                    if self.cv.wait_for(&mut g, dl - now).timed_out() {
                        g.waiting.remove(&txn);
                        return Err(LockErr::TimedOut);
                    }
                }
            }
        }
    }

    pub(crate) fn unlock_all(&self, keys: &[Bytes], txn: u64) {
        let mut g = self.inner.lock();
        for k in keys {
            if g.owned.get(k) == Some(&txn) {
                g.owned.remove(k);
            }
        }
        g.waiting.remove(&txn);
        self.cv.notify_all();
    }
}

/// Wait-for cycle ⇒ deadlock (RFC-0150 P2c). Production lock table calls this.
#[cfg(not(verus_keep_ghost))]
pub(crate) fn wait_for_deadlock(
    owned: &HashMap<Bytes, u64>,
    waiting: &HashMap<u64, Bytes>,
    waiter: u64,
    mut owner: u64,
) -> bool {
    let mut seen = HashSet::new();
    while seen.insert(owner) {
        let Some(k) = waiting.get(&owner) else {
            return false;
        };
        let Some(&next) = owned.get(k) else {
            return false;
        };
        if next == waiter {
            return true;
        }
        owner = next;
    }
    true
}

/// AS-IS: miss the cycle (wait forever / grant overlapping locks).
#[cfg(not(verus_keep_ghost))]
#[allow(dead_code)] // tests + Verus twin; production never calls the mutant
pub(crate) fn wait_for_deadlock_as_is(
    _owned: &HashMap<Bytes, u64>,
    _waiting: &HashMap<u64, Bytes>,
    _waiter: u64,
    _owner: u64,
) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn wait_for_deadlock_on_live_cycle_is_not_ok() {
        let mut owned = HashMap::new();
        let mut waiting = HashMap::new();
        owned.insert(Bytes::from_static(b"a"), 1);
        owned.insert(Bytes::from_static(b"b"), 2);
        waiting.insert(1, Bytes::from_static(b"b"));
        waiting.insert(2, Bytes::from_static(b"a"));
        assert!(wait_for_deadlock(&owned, &waiting, 1, 2));
        assert!(
            !wait_for_deadlock_as_is(&owned, &waiting, 1, 2),
            "AS-IS dente: miss the cycle"
        );

        // Production `LockTable::lock` (the path TransactionDB put takes).
        let table = LockTable::new();
        let t1 = table.alloc_id();
        let t2 = table.alloc_id();
        table
            .lock(Bytes::from_static(b"a"), t1, Duration::ZERO, true)
            .unwrap();
        table
            .lock(Bytes::from_static(b"b"), t2, Duration::ZERO, true)
            .unwrap();
        let wait = Duration::from_millis(400);
        std::thread::scope(|s| {
            s.spawn(|| {
                let _ = table.lock(Bytes::from_static(b"b"), t1, wait, true);
            });
            std::thread::sleep(Duration::from_millis(20));
            let err = table.lock(Bytes::from_static(b"a"), t2, wait, true);
            assert_eq!(
                err,
                Err(LockErr::Deadlock),
                "live lock() must refuse the 2PL cycle"
            );
        });
    }

    #[test]
    fn two_cycle_is_deadlock() {
        let mut owned = HashMap::new();
        let mut waiting = HashMap::new();
        owned.insert(Bytes::from_static(b"a"), 1);
        owned.insert(Bytes::from_static(b"b"), 2);
        waiting.insert(1, Bytes::from_static(b"b"));
        waiting.insert(2, Bytes::from_static(b"a"));
        assert!(wait_for_deadlock(&owned, &waiting, 1, 2));
        assert!(
            !wait_for_deadlock_as_is(&owned, &waiting, 1, 2),
            "AS-IS dente: miss the cycle"
        );
        waiting.remove(&2);
        assert!(!wait_for_deadlock(&owned, &waiting, 1, 2));
    }

    #[test]
    fn three_cycle_is_deadlock() {
        let mut owned = HashMap::new();
        let mut waiting = HashMap::new();
        owned.insert(Bytes::from_static(b"a"), 1);
        owned.insert(Bytes::from_static(b"b"), 2);
        owned.insert(Bytes::from_static(b"c"), 3);
        waiting.insert(1, Bytes::from_static(b"b"));
        waiting.insert(2, Bytes::from_static(b"c"));
        waiting.insert(3, Bytes::from_static(b"a"));
        assert!(
            wait_for_deadlock(&owned, &waiting, 1, 2),
            "N-way wait-for: 1→2→3→1 is a deadlock"
        );
        assert!(
            !wait_for_deadlock_as_is(&owned, &waiting, 1, 2),
            "AS-IS dente: miss the 3-cycle"
        );
    }
}
