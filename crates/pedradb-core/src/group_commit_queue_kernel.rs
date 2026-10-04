//! RFC-0309: Zero-Twin Group Commit Queue Concurrency Kernel.
//!
//! Provides the canonical leader-follower batch queue synchronization kernel,
//! routed strictly through `crate::sync_kernel` (Loom compatible).
//!
//! Enforces:
//! 1. Exactly one leader active across concurrent arrivals.
//! 2. Zero lost wakeups / deadlocks between leader draining queue and follower waiting on reply.
//! 3. Concurrent arrivals while leader is executing batch I/O.
//! 4. Strict Acquire-Release memory ordering on sequence publication.

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use crate::sync_kernel::atomic::{AtomicU64, AtomicUsize, Ordering};
use crate::sync_kernel::{Condvar, Mutex};
use crate::sync_kernel::mpsc::{self, Receiver, Sender};

/// Admission outcome for a participant entering the group commit queue.
pub enum Admission<T, R> {
    /// Participant was elected Leader of the upcoming batch.
    Leader {
        /// Initial item submitted by the leader itself.
        item: T,
    },
    /// Participant joined as a Follower waiting for the leader's commit.
    Follower {
        /// Channel receiver on which the follower awaits its reply.
        rx: Receiver<R>,
    },
}

/// Typed error outcomes for group commit queue operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupCommitQueueError {
    /// Attempted to publish a non-monotonic sequence number.
    RegressiveSequencePublication { current: u64, attempted: u64 },
}

impl std::fmt::Display for GroupCommitQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RegressiveSequencePublication { current, attempted } => {
                write!(
                    f,
                    "GroupCommitQueueError: cannot publish regressive sequence {attempted} <= current {current}"
                )
            }
        }
    }
}

impl std::error::Error for GroupCommitQueueError {}

/// Production synchronization kernel for group commit leader-follower batching.
pub struct GroupCommitQueue<T, R> {
    state: Mutex<GroupCommitQueueState<T, R>>,
    arrived: Condvar,
    active: AtomicUsize,
    published_seq: AtomicU64,
}

struct GroupCommitQueueState<T, R> {
    pending: VecDeque<(T, Sender<R>)>,
    leader_active: bool,
}

impl<T, R> Default for GroupCommitQueue<T, R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, R> GroupCommitQueue<T, R> {
    /// Constructs a new empty group commit queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(GroupCommitQueueState {
                pending: VecDeque::new(),
                leader_active: false,
            }),
            arrived: Condvar::new(),
            active: AtomicUsize::new(0),
            published_seq: AtomicU64::new(0),
        }
    }

    /// Enqueues a write request, electing a leader or assigning as follower.
    pub fn join(&self, item: T) -> Admission<T, R> {
        self.active.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();

        let mut g = self.state.lock();
        if !g.leader_active {
            g.leader_active = true;
            Admission::Leader { item }
        } else {
            g.pending.push_back((item, tx));
            self.arrived.notify_all();
            Admission::Follower { rx }
        }
    }

    /// Drains all currently queued follower requests from the queue.
    pub fn drain_pending(&self) -> Vec<(T, Sender<R>)> {
        let mut g = self.state.lock();
        g.pending.drain(..).collect()
    }

    /// Checks if more followers arrived. If empty, clears leader status and retires.
    /// Returns `true` if retired, or `false` if more items are pending.
    pub fn retire_leader(&self) -> bool {
        let mut g = self.state.lock();
        if g.pending.is_empty() {
            g.leader_active = false;
            true
        } else {
            false
        }
    }

    /// Publishes the committed sequence number with Release ordering.
    pub fn publish_seq(&self, seq: u64) {
        let _ = self.try_publish_seq(seq);
    }

    /// Atomically and monotonically publishes the next sequence number with Release ordering.
    pub fn try_publish_seq(&self, seq: u64) -> Result<(), GroupCommitQueueError> {
        let mut curr = self.published_seq.load(Ordering::Acquire);
        loop {
            if seq <= curr {
                return Err(GroupCommitQueueError::RegressiveSequencePublication {
                    current: curr,
                    attempted: seq,
                });
            }
            match self.published_seq.compare_exchange_weak(
                curr,
                seq,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => curr = actual,
            }
        }
    }

    /// Reads the current published sequence number with Acquire ordering.
    #[must_use]
    pub fn published_seq(&self) -> u64 {
        self.published_seq.load(Ordering::Acquire)
    }

    /// Reads the count of active participants currently in flight.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    /// Completes a participant ticket, decrementing in-flight count with underflow protection.
    pub fn complete_ticket(&self) {
        let _ = self.active.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
            Some(val.saturating_sub(1))
        });
    }

    /// Submits a work item and executes the leader-follower batching protocol.
    ///
    /// If elected leader, `process_batch` is invoked with the accumulated items
    /// and returns `(replies, next_published_seq)`.
    pub fn submit<F>(&self, item: T, mut process_batch: F) -> R
    where
        F: FnMut(Vec<T>) -> (Vec<R>, u64),
    {
        struct TicketGuard<'a, T, R>(&'a GroupCommitQueue<T, R>);
        impl<'a, T, R> Drop for TicketGuard<'a, T, R> {
            fn drop(&mut self) {
                self.0.complete_ticket();
            }
        }

        struct LeaderGuard<'a, T, R> {
            queue: &'a GroupCommitQueue<T, R>,
            active: bool,
        }
        impl<'a, T, R> Drop for LeaderGuard<'a, T, R> {
            fn drop(&mut self) {
                if self.active {
                    let mut g = self.queue.state.lock();
                    g.leader_active = false;
                    self.queue.arrived.notify_all();
                }
            }
        }

        let admission = self.join(item);
        let _ticket_guard = TicketGuard(self);

        match admission {
            Admission::Leader { item: leader_item } => {
                let mut leader_guard = LeaderGuard {
                    queue: self,
                    active: true,
                };
                let mut leader_reply = None;
                let mut current_items = vec![leader_item];
                loop {
                    let drained = self.drain_pending();
                    let mut follower_senders = Vec::with_capacity(drained.len());
                    for (pending_item, tx) in drained {
                        current_items.push(pending_item);
                        follower_senders.push(tx);
                    }

                    let batch_len = current_items.len();
                    let (mut replies, new_seq) = process_batch(std::mem::take(&mut current_items));
                    assert_eq!(
                        replies.len(),
                        batch_len,
                        "replies count must match batch size"
                    );

                    self.publish_seq(new_seq);

                    if leader_reply.is_none() && !replies.is_empty() {
                        leader_reply = Some(replies.remove(0));
                    }

                    for (tx, reply) in follower_senders.into_iter().zip(replies.into_iter()) {
                        let _ = tx.send(reply);
                    }

                    if self.retire_leader() {
                        leader_guard.active = false;
                        break;
                    }
                }
                leader_reply.expect("leader reply must be set")
            }
            Admission::Follower { rx } => {
                rx.recv().expect("leader must not drop reply without sending")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn leader_panic_in_process_batch_clears_leader_active_so_next_caller_can_lead() {
        if crate::sync_kernel::IS_LOOM {
            return;
        }
        let queue = Arc::new(GroupCommitQueue::<u64, u64>::new());
        let q_clone = Arc::clone(&queue);

        // Leader thread panics during process_batch
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            q_clone.submit(1, |_batch| {
                panic!("simulated I/O failure during batch commit");
            });
        }));

        // After the leader panicked, the queue MUST NOT be permanently locked with leader_active = true.
        // The next participant MUST be admitted as Leader so the system can recover!
        match queue.join(2) {
            Admission::Leader { item } => {
                assert_eq!(item, 2);
            }
            Admission::Follower { .. } => {
                panic!("Queue is permanently poisoned! Subsequent caller admitted as Follower with no leader!");
            }
        }
    }

    #[test]
    fn test_group_commit_queue_structural_invariants_red_to_green() {
        let queue = GroupCommitQueue::<u64, u64>::new();

        // 1. Calling complete_ticket at 0 must not underflow to usize::MAX
        assert_eq!(queue.active_count(), 0);
        queue.complete_ticket();
        assert_eq!(queue.active_count(), 0);

        // 2. Monotonic sequence publication
        assert_eq!(queue.try_publish_seq(10), Ok(()));
        assert_eq!(queue.published_seq(), 10);

        // 3. Regressive sequence publication rejected
        assert_eq!(
            queue.try_publish_seq(5),
            Err(GroupCommitQueueError::RegressiveSequencePublication {
                current: 10,
                attempted: 5,
            })
        );
        assert_eq!(
            queue.try_publish_seq(10),
            Err(GroupCommitQueueError::RegressiveSequencePublication {
                current: 10,
                attempted: 10,
            })
        );
        assert_eq!(queue.published_seq(), 10);
    }
}

