//! RFC-0317: Asynchronous I/O Completion Ring Order Verifier test suite.
//!
//! Verifies:
//! - In-order monotonic watermark advancement.
//! - Out-of-order CQE tracking and cascading watermark resolution upon gap fill.
//! - Duplicate completion detection and rejection.
//! - Stale ticket (< watermark) rejection.
//! - Bounded sliding window enforcement.

use pedradb_core::io_ring_completion_order_kernel::{
    IoRingCompletionError, IoRingCompletionTracker,
};

#[test]
fn test_in_order_completion_advancement() {
    let mut tracker = IoRingCompletionTracker::new(100, 16).expect("valid tracker");
    assert_eq!(tracker.watermark(), 100);
    assert_eq!(tracker.pending_count(), 0);

    assert_eq!(tracker.complete_ticket(100).unwrap(), 101);
    assert_eq!(tracker.complete_ticket(101).unwrap(), 102);
    assert_eq!(tracker.complete_ticket(102).unwrap(), 103);
    assert_eq!(tracker.watermark(), 103);
    assert_eq!(tracker.pending_count(), 0);
    assert!(tracker.verify_internal_invariants());
}

#[test]
fn test_out_of_order_completion_and_cascading_advance() {
    let mut tracker = IoRingCompletionTracker::new(0, 32).expect("valid tracker");

    // Complete tickets 2, 4, 3, 1 out of order (ticket 0 still missing)
    assert_eq!(tracker.complete_ticket(2).unwrap(), 0); // Watermark remains 0
    assert_eq!(tracker.complete_ticket(4).unwrap(), 0);
    assert_eq!(tracker.complete_ticket(3).unwrap(), 0);
    assert_eq!(tracker.complete_ticket(1).unwrap(), 0);
    assert_eq!(tracker.watermark(), 0);
    assert_eq!(tracker.pending_count(), 4);

    // Now complete ticket 0. Watermark should cascade from 0 past 1, 2, 3, 4 up to 5!
    assert_eq!(tracker.complete_ticket(0).unwrap(), 5);
    assert_eq!(tracker.watermark(), 5);
    assert_eq!(tracker.pending_count(), 0);
    assert!(tracker.verify_internal_invariants());
}

#[test]
fn test_duplicate_and_stale_completion_rejection() {
    let mut tracker = IoRingCompletionTracker::new(10, 16).expect("valid tracker");
    tracker.complete_ticket(10).unwrap();
    assert_eq!(tracker.watermark(), 11);

    // Ticket 10 is now stale (already retired)
    assert_eq!(
        tracker.complete_ticket(10).err(),
        Some(IoRingCompletionError::StaleTicket {
            ticket: 10,
            watermark: 11,
        })
    );

    // Complete ticket 12 (out of order, pending)
    tracker.complete_ticket(12).unwrap();
    assert_eq!(tracker.pending_count(), 1);

    // Re-completing ticket 12 must trigger DuplicateCompletion
    assert_eq!(
        tracker.complete_ticket(12).err(),
        Some(IoRingCompletionError::DuplicateCompletion(12))
    );
}

#[test]
fn test_window_bound_overflow_rejection() {
    let mut tracker = IoRingCompletionTracker::new(0, 10).expect("valid tracker");

    // Window covers [0..10). Ticket 10 is at or exceeds watermark + 10
    assert!(matches!(
        tracker.complete_ticket(10).err(),
        Some(IoRingCompletionError::TicketExceedsWindow { ticket: 10, max_allowed: 9 })
    ));

    // Zero window size is rejected on constructor
    assert_eq!(
        IoRingCompletionTracker::new(0, 0).err(),
        Some(IoRingCompletionError::InvalidWindowSize(0))
    );
}

#[test]
fn test_io_ring_red_invariants() {
    // 1. Error implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(IoRingCompletionError::InvalidWindowSize(0));
    assert!(!err.to_string().is_empty());

    // 2. Watermark overflow rejection when approaching u64::MAX
    let mut tracker = IoRingCompletionTracker::new(u64::MAX - 1, 10).expect("valid tracker");
    assert_eq!(tracker.complete_ticket(u64::MAX - 1).unwrap(), u64::MAX);
    // Completing u64::MAX would advance watermark past u64::MAX
    assert_eq!(
        tracker.complete_ticket(u64::MAX).err(),
        Some(IoRingCompletionError::WatermarkOverflow)
    );

    // 3. Batch completion: atomic progress
    let mut tracker_batch = IoRingCompletionTracker::new(0, 16).expect("valid tracker");
    let batch = vec![0, 2, 1, 3];
    let new_wm = tracker_batch.complete_batch(&batch).expect("batch complete");
    assert_eq!(new_wm, 4);
    assert_eq!(tracker_batch.watermark(), 4);
    assert_eq!(tracker_batch.pending_count(), 0);
}

