//! Exclusive key-lock table for rust-rocksdb `TransactionDB` (2PL).
//! OCC [`super::OptimisticTransactionDB`] does not use this.

use bytes::Bytes;
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Lock wait outcome (Rocks `Busy` / `TimedOut`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockErr {
    /// Deadlock detected before waiting.
    Deadlock,
    /// Timeout (including timeout=0, lock busy).
    TimedOut,
}

pub(crate) struct LockTable {
    inner: Mutex<Inner>,
    cv: Condvar,
    next_id: AtomicU64,
}

struct Inner {
    /// Encoded key → owner txn id.
    owned: HashMap<Bytes, u64>,
    /// Waiter txn id → key it is blocked on.
    waiting: HashMap<u64, Bytes>,
}

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
}
