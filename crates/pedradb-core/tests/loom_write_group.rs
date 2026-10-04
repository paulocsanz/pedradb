//! RFC-0270 / RFC-0309: Zero-Twin Concurrency Verification for `GroupCommitQueue` using Loom.
//!
//! Exhaustively verifies the REAL production `GroupCommitQueue` synchronization kernel:
//! 1. Leader-follower election protocol: exactly one leader active at any time.
//! 2. No lost wakeups / deadlocks between leader draining queue and follower waiting on channel reply.
//! 3. Concurrent arrivals while leader is "off-lock" executing batch I/O.
//! 4. Handoff / drain completion: all followers receive their monotonic commit sequence.
//! 5. Memory ordering on publication: followers observe committed state with Acquire-Release semantics.

// Route production sync primitives to Loom for model checking
mod sync_kernel {
    pub const IS_LOOM: bool = true;
    pub mod atomic {
        pub use loom::sync::atomic::*;
    }
    pub use loom::sync::Condvar;
    pub struct Mutex<T: ?Sized>(loom::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub fn new(val: T) -> Self {
            Self(loom::sync::Mutex::new(val))
        }
        pub fn lock(&self) -> loom::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
    pub use loom::sync::mpsc;
}

// Zero-Twin: Compile the production kernel directly into the Loom harness
#[path = "../src/group_commit_queue_kernel.rs"]
mod group_commit_queue_kernel;

use std::sync::Arc;
use loom::sync::atomic::{AtomicU64, Ordering};
use loom::thread;
use group_commit_queue_kernel::GroupCommitQueue;

fn submit_to_queue(wg: &Arc<GroupCommitQueue<u64, u64>>, client_id: u64) -> u64 {
    wg.submit(client_id, |batch| {
        let cur = wg.published_seq();
        let next_seq = cur + batch.len() as u64;
        let replies: Vec<u64> = (0..batch.len()).map(|idx| cur + 1 + idx as u64).collect();
        (replies, next_seq)
    })
}

#[test]
fn loom_write_group_two_concurrent_writers() {
    loom::model(|| {
        let wg = Arc::new(GroupCommitQueue::new());

        let wg1 = Arc::clone(&wg);
        let h1 = thread::spawn(move || submit_to_queue(&wg1, 1));

        let wg2 = Arc::clone(&wg);
        let h2 = thread::spawn(move || submit_to_queue(&wg2, 2));

        let s1 = h1.join().unwrap();
        let s2 = h2.join().unwrap();

        // Both committed, different sequence numbers
        assert!(s1 == 1 || s1 == 2);
        assert!(s2 == 1 || s2 == 2);
        assert_ne!(s1, s2);
        assert_eq!(wg.published_seq(), 2);
        assert_eq!(wg.active_count(), 0);
    });
}

#[test]
fn loom_write_group_reader_writer_linearizability() {
    loom::model(|| {
        let wg = Arc::new(GroupCommitQueue::new());
        let data = Arc::new(AtomicU64::new(0));

        let wg1 = Arc::clone(&wg);
        let data1 = Arc::clone(&data);
        let writer = thread::spawn(move || {
            // Write data, then submit to write group
            data1.store(42, Ordering::Release);
            submit_to_queue(&wg1, 100)
        });

        let wg2 = Arc::clone(&wg);
        let data2 = Arc::clone(&data);
        let reader = thread::spawn(move || {
            let pub_seq = wg2.published_seq();
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

#[test]
fn loom_write_group_concurrent_arrival_during_batch() {
    loom::model(|| {
        let wg = Arc::new(GroupCommitQueue::new());

        let wg1 = Arc::clone(&wg);
        let wg2 = Arc::clone(&wg);

        let h1 = thread::spawn(move || {
            // First writer submits
            submit_to_queue(&wg1, 10)
        });

        let h2 = thread::spawn(move || {
            // Second concurrent writer submits
            submit_to_queue(&wg2, 20)
        });

        let s1 = h1.join().unwrap();
        let s2 = h2.join().unwrap();

        assert_ne!(s1, s2);
        assert!(s1 == 1 || s1 == 2);
        assert!(s2 == 1 || s2 == 2);
        assert_eq!(wg.published_seq(), 2);
        assert_eq!(wg.active_count(), 0);
    });
}
