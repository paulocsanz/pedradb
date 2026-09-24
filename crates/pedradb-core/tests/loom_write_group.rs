//! Exhaustive concurrency model checking for `WriteGroup` and channel synchronization using Loom.
//!
//! Verifies:
//! 1. Leader-follower election protocol: exactly one leader active at any time.
//! 2. No lost wakeups / deadlocks between leader draining queue and follower waiting on channel reply.
//! 3. Concurrent arrivals while leader is "off-lock" executing WAL fsync.
//! 4. Handoff / drain completion: all followers receive their monotonic commit sequence.
//! 5. Memory ordering on publication: followers observe committed state with Acquire-Release semantics.

use std::collections::VecDeque;
use std::sync::Arc;
use loom::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use loom::sync::{Mutex, Condvar};
use loom::sync::mpsc;
use loom::thread;

/// Loom model of the WriteGroup synchronization kernel
struct LoomWriteGroup {
    queue: Mutex<LoomWriteGroupState>,
    arrived: Condvar,
    active: AtomicUsize,
    published_seq: AtomicU64,
}

struct LoomWriteGroupState {
    pending: VecDeque<(u64, mpsc::Sender<u64>)>,
    leader_active: bool,
}

impl LoomWriteGroup {
    fn new() -> Self {
        Self {
            queue: Mutex::new(LoomWriteGroupState {
                pending: VecDeque::new(),
                leader_active: false,
            }),
            arrived: Condvar::new(),
            active: AtomicUsize::new(0),
            published_seq: AtomicU64::new(0),
        }
    }

    /// Client submits a write request. Returns committed sequence number.
    fn submit(self: &Arc<Self>, client_id: u64) -> u64 {
        self.active.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();

        let is_leader = {
            let mut g = self.queue.lock().unwrap();
            let leader = !g.leader_active;
            if leader {
                g.leader_active = true;
                g.pending.push_back((client_id, tx));
                true
            } else {
                g.pending.push_back((client_id, tx));
                self.arrived.notify_all();
                false
            }
        };

        if is_leader {
            self.lead();
        }

        let seq = rx.recv().expect("leader must not drop reply without sending");
        self.active.fetch_sub(1, Ordering::SeqCst);
        seq
    }

    /// Leader execution loop: drains pending writes, simulates WAL sync barrier,
    /// advances published_seq, and replies to all batched followers.
    fn lead(&self) {
        loop {
            // Drain current batch under queue lock
            let batch: Vec<(u64, mpsc::Sender<u64>)> = {
                let mut g = self.queue.lock().unwrap();
                g.pending.drain(..).collect()
            };

            assert!(!batch.is_empty(), "leader must have at least one batch item");

            // Off-lock WAL barrier simulation
            // Sequence numbers advance strictly monotonically
            let cur = self.published_seq.load(Ordering::Acquire);
            let next_seq = cur + batch.len() as u64;

            // Release publication
            self.published_seq.store(next_seq, Ordering::Release);

            // Reply to followers
            for (idx, (_client, reply)) in batch.into_iter().enumerate() {
                let assigned_seq = cur + 1 + idx as u64;
                let _ = reply.send(assigned_seq);
            }

            // Check if more writers arrived during off-lock WAL flight
            let mut g = self.queue.lock().unwrap();
            if g.pending.is_empty() {
                g.leader_active = false;
                break;
            }
            // More writes arrived: loop and process next batch as leader!
        }
    }
}

#[test]
fn loom_write_group_two_concurrent_writers() {
    loom::model(|| {
        let wg = Arc::new(LoomWriteGroup::new());

        let wg1 = Arc::clone(&wg);
        let h1 = thread::spawn(move || {
            wg1.submit(1)
        });

        let wg2 = Arc::clone(&wg);
        let h2 = thread::spawn(move || {
            wg2.submit(2)
        });

        let s1 = h1.join().unwrap();
        let s2 = h2.join().unwrap();

        // Both committed, different sequence numbers
        assert!(s1 == 1 || s1 == 2);
        assert!(s2 == 1 || s2 == 2);
        assert_ne!(s1, s2);
        assert_eq!(wg.published_seq.load(Ordering::Acquire), 2);
        assert_eq!(wg.active.load(Ordering::Acquire), 0);
    });
}

#[test]
fn loom_write_group_three_concurrent_writers() {
    loom::model(|| {
        let wg = Arc::new(LoomWriteGroup::new());

        let wg1 = Arc::clone(&wg);
        let h1 = thread::spawn(move || wg1.submit(1));

        let wg2 = Arc::clone(&wg);
        let h2 = thread::spawn(move || wg2.submit(2));

        let wg3 = Arc::clone(&wg);
        let h3 = thread::spawn(move || wg3.submit(3));

        let s1 = h1.join().unwrap();
        let s2 = h2.join().unwrap();
        let s3 = h3.join().unwrap();

        assert_ne!(s1, s2);
        assert_ne!(s2, s3);
        assert_ne!(s1, s3);
        assert_eq!(wg.published_seq.load(Ordering::Acquire), 3);
        assert_eq!(wg.active.load(Ordering::Acquire), 0);
    });
}

#[test]
fn loom_write_group_reader_writer_linearizability() {
    loom::model(|| {
        let wg = Arc::new(LoomWriteGroup::new());
        let data = Arc::new(AtomicU64::new(0));

        let wg1 = Arc::clone(&wg);
        let data1 = Arc::clone(&data);
        let writer = thread::spawn(move || {
            // Write data, then submit to write group
            data1.store(42, Ordering::Release);
            wg1.submit(100)
        });

        let wg2 = Arc::clone(&wg);
        let data2 = Arc::clone(&data);
        let reader = thread::spawn(move || {
            let pub_seq = wg2.published_seq.load(Ordering::Acquire);
            if pub_seq > 0 {
                // If reader sees published sequence > 0, it MUST observe the data written
                let val = data2.load(Ordering::Acquire);
                assert_eq!(val, 42, "reader observed published sequence but stale data!");
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    });
}
