//! Exclusive key-lock table for rust-rocksdb `TransactionDB` (2PL).
//! OCC [`super::OptimisticTransactionDB`] does not use this.
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_locktab.sh`). A Verus flattened `u64` two-cycle stand-in
//! of rustc `wait_for_deadlock(&HashMap, &HashMap, u64, u64)` is a model twin
//! — not last-wins (deleted). `LockTable` I/O (parking_lot / Condvar) stays
//! rustc-only; the wait-for walk is the term.
//!
//!   ./scripts/aeneas_locktab.sh --required
//!
//! Aeneas of the rustc body is the term. A Verus stand-in is not last-wins.

#![forbid(unsafe_code)]

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
    false
}

/// AS-IS: miss the cycle (wait forever / grant overlapping locks).
#[allow(dead_code)] // tests + catalog as-is dente; production never calls the mutant
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
    fn locktab_has_no_verus_cartoon() {
        let src = include_str!("locktab_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "flattened u64 two-cycle is not last-wins of rustc HashMap walk"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

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

    /// Regression test for Bug 1: external innocent waiter T3 waiting on T1
    /// must not be flagged as deadlocked when T1 and T2 are in a separate cycle (T1 <-> T2).
    #[test]
    fn external_innocent_waiter_does_not_deadlock() {
        let mut owned = HashMap::new();
        let mut waiting = HashMap::new();
        // T1 owns "a", T2 owns "b"
        owned.insert(Bytes::from_static(b"a"), 1);
        owned.insert(Bytes::from_static(b"b"), 2);
        // T1 waits for "b" (owned by T2)
        // T2 waits for "a" (owned by T1) -> Cycle between T1 and T2!
        waiting.insert(1, Bytes::from_static(b"b"));
        waiting.insert(2, Bytes::from_static(b"a"));

        // T3 wants to lock "a" (owned by T1). T3 is an external innocent waiter.
        assert!(
            !wait_for_deadlock(&owned, &waiting, 3, 1),
            "T3 is NOT in the cycle T1<->T2; must NOT be falsely marked deadlocked!"
        );
    }

    /// Mathematical Property-Based Test (Barreira 4):
    /// Verifies that for 10,000 arbitrary pseudo-random directed graph topologies,
    /// `wait_for_deadlock` matches the formal reachability definition:
    /// `wait_for_deadlock(waiter, owner) <=> waiter in Reachable(owner)`.
    #[test]
    fn proptest_wait_for_deadlock_exact_reachability() {
        // Simple deterministic xorshift PRNG for reproducible property-based testing
        struct Lcg(u64);
        impl Lcg {
            fn next_u64(&mut self) -> u64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0
            }
            fn gen_range(&mut self, low: usize, high: usize) -> usize {
                if low >= high { return low; }
                low + (self.next_u64() as usize % (high - low))
            }
        }

        let mut rng = Lcg(0xDEAD_BEEF_CAFE_BABE);

        // Reference oracle: independent Breadth-First-Search reachability
        fn oracle_reachability(
            owned: &HashMap<Bytes, u64>,
            waiting: &HashMap<u64, Bytes>,
            start: u64,
            target: u64,
        ) -> bool {
            let mut visited = HashSet::new();
            let mut curr = start;
            while visited.insert(curr) {
                if let Some(key) = waiting.get(&curr) {
                    if let Some(&next_tx) = owned.get(key) {
                        if next_tx == target {
                            return true;
                        }
                        curr = next_tx;
                        continue;
                    }
                }
                break;
            }
            false
        }

        // Buggy mutant from the original code (Barreira 4 / Anti-vacuity check)
        fn buggy_mutant_false_deadlock(
            owned: &HashMap<Bytes, u64>,
            waiting: &HashMap<u64, Bytes>,
            waiter: u64,
            mut owner: u64,
        ) -> bool {
            let mut seen = HashSet::new();
            while seen.insert(owner) {
                let Some(k) = waiting.get(&owner) else { return false; };
                let Some(&next) = owned.get(k) else { return false; };
                if next == waiter { return true; }
                owner = next;
            }
            true // The bug: returns true on ANY cycle, even if waiter is not in it
        }

        let mut killed_false_deadlock_mutant = 0usize;
        let mut killed_as_is_mutant = 0usize;
        let mut positive_deadlocks_found = 0usize;

        for _iteration in 0..10_000 {
            let num_txs = rng.gen_range(2, 25);
            let num_keys = rng.gen_range(2, 25);

            let mut owned: HashMap<Bytes, u64> = HashMap::new();
            let mut waiting: HashMap<u64, Bytes> = HashMap::new();

            for k in 0..num_keys {
                if rng.next_u64() % 3 != 0 {
                    let owner_tx = (rng.gen_range(0, num_txs) + 1) as u64;
                    owned.insert(Bytes::from(format!("key_{k}")), owner_tx);
                }
            }

            for t in 1..=num_txs {
                let tx_id = t as u64;
                if rng.next_u64() % 2 == 0 {
                    let waited_key = Bytes::from(format!("key_{}", rng.gen_range(0, num_keys)));
                    waiting.insert(tx_id, waited_key);
                }
            }

            let waiter = (rng.gen_range(0, num_txs) + 1) as u64;
            let owner = (rng.gen_range(0, num_txs) + 1) as u64;

            let expected = oracle_reachability(&owned, &waiting, owner, waiter);
            let actual = wait_for_deadlock(&owned, &waiting, waiter, owner);

            assert_eq!(
                actual, expected,
                "Invariant violation: wait_for_deadlock failed graph reachability contract! waiter={waiter}, owner={owner}"
            );

            if actual {
                positive_deadlocks_found += 1;
            }

            // Anti-vacuity: verify that mutant implementations are caught
            let mutant_bug = buggy_mutant_false_deadlock(&owned, &waiting, waiter, owner);
            if mutant_bug != expected {
                killed_false_deadlock_mutant += 1;
            }

            let mutant_as_is = wait_for_deadlock_as_is(&owned, &waiting, waiter, owner);
            if mutant_as_is != expected {
                killed_as_is_mutant += 1;
            }
        }

        assert!(
            positive_deadlocks_found > 0,
            "Property test must generate at least some actual deadlocks to be sound!"
        );
        assert!(
            killed_false_deadlock_mutant > 0,
            "Anti-vacuity check failed: false-deadlock mutant was never killed!"
        );
        assert!(
            killed_as_is_mutant > 0,
            "Anti-vacuity check failed: as-is mutant was never killed!"
        );
    }
}
