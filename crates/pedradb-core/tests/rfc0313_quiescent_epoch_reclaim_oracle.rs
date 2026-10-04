//! RFC-0313: Lock-Free Quiescent Epoch Memory Reclamation Oracle Test Suite.
//!
//! Validates mathematical properties:
//! - Generational memory safety: items are never collected while concurrent readers remain in epoch $\le e_{\text{retire}}$.
//! - Dynamic safe frontier advancement under thread unpinning.
//! - Quarantine bounded capacity saturation and stalled thread detection.

use pedradb_core::quiescent_epoch_reclaim_kernel::{
    EpochReclaimError, QuiescentEpochReclaimer,
};

#[test]
fn test_epoch_critical_section_lifecycle() {
    let mut reclaimer = QuiescentEpochReclaimer::new(1024 * 1024);

    // Register threads
    reclaimer.register_thread(1);
    reclaimer.register_thread(2);
    assert!(reclaimer.is_thread_registered(1));
    assert!(reclaimer.is_thread_registered(2));
    assert!(!reclaimer.is_thread_registered(3));

    // Initial global epoch is 1
    assert_eq!(reclaimer.global_epoch(), 1);

    // Thread 1 enters critical section
    assert_eq!(reclaimer.enter_critical_section(1).unwrap(), 1);

    // Re-entering returns AlreadyPinned
    assert_eq!(
        reclaimer.enter_critical_section(1).unwrap_err(),
        EpochReclaimError::AlreadyPinned(1)
    );

    // Unregistered thread returns UnregisteredThread
    assert_eq!(
        reclaimer.enter_critical_section(99).unwrap_err(),
        EpochReclaimError::UnregisteredThread(99)
    );

    // Thread 1 exits critical section
    assert!(reclaimer.exit_critical_section(1).is_ok());

    // Exiting again returns AlreadyQuiescent
    assert_eq!(
        reclaimer.exit_critical_section(1).unwrap_err(),
        EpochReclaimError::AlreadyQuiescent(1)
    );
}

#[test]
fn test_safe_reclamation_frontier_monotonicity() {
    let mut reclaimer = QuiescentEpochReclaimer::new(1024 * 1024);
    reclaimer.register_thread(1);
    reclaimer.register_thread(2);

    // When all threads are quiescent, frontier matches global epoch
    assert_eq!(reclaimer.safe_reclaim_frontier(), 1);

    // Thread 1 pins at epoch 1
    reclaimer.enter_critical_section(1).unwrap();
    assert_eq!(reclaimer.safe_reclaim_frontier(), 1);

    // Advance global epoch to 2
    assert_eq!(reclaimer.advance_epoch(), 2);

    // Thread 2 pins at epoch 2
    reclaimer.enter_critical_section(2).unwrap();

    // Advance global epoch to 5
    reclaimer.advance_epoch();
    reclaimer.advance_epoch();
    reclaimer.advance_epoch();
    assert_eq!(reclaimer.global_epoch(), 5);

    // Frontier must be held back by oldest active thread (thread 1 at epoch 1)
    assert_eq!(reclaimer.safe_reclaim_frontier(), 1);

    // Thread 1 exits: frontier advances to thread 2's epoch (2)
    reclaimer.exit_critical_section(1).unwrap();
    assert_eq!(reclaimer.safe_reclaim_frontier(), 2);

    // Thread 2 exits: all threads quiescent, frontier jumps to global epoch (5)
    reclaimer.exit_critical_section(2).unwrap();
    assert_eq!(reclaimer.safe_reclaim_frontier(), 5);
}

#[test]
fn test_deferred_reclamation_safety_and_collection() {
    let mut reclaimer = QuiescentEpochReclaimer::new(1024 * 1024);
    reclaimer.register_thread(1);

    // Global Epoch = 1. Retire Item 101.
    reclaimer.retire(101, 100).unwrap();

    // Advance to Epoch 2. Thread 1 enters at Epoch 2.
    reclaimer.advance_epoch();
    reclaimer.enter_critical_section(1).unwrap();

    // Retire Item 102 at Epoch 2.
    reclaimer.retire(102, 200).unwrap();

    // Advance to Epoch 3. Retire Item 103 at Epoch 3.
    reclaimer.advance_epoch();
    reclaimer.retire(103, 300).unwrap();

    // Currently: Thread 1 is pinned at Epoch 2.
    // Safe frontier = 2.
    // Therefore:
    // - Item 101 (retired at 1) < frontier 2 -> SAFE TO RECLAIM
    // - Item 102 (retired at 2) not < frontier 2 -> PROTECTED
    // - Item 103 (retired at 3) not < frontier 2 -> PROTECTED
    let reclaimed = reclaimer.collect_garbage();
    assert_eq!(reclaimed, vec![101]);
    assert_eq!(reclaimer.reclaimed_bytes(), 100);
    assert_eq!(reclaimer.quarantine_bytes(), 500); // 200 + 300

    // Thread 1 exits critical section. All threads quiescent.
    reclaimer.exit_critical_section(1).unwrap();
    // Advance to Epoch 4. Frontier is now 4.
    reclaimer.advance_epoch();

    // Now Items 102 and 103 are safe to reclaim!
    let reclaimed2 = reclaimer.collect_garbage();
    assert_eq!(reclaimed2, vec![102, 103]);
    assert_eq!(reclaimer.reclaimed_bytes(), 600); // 100 + 200 + 300
    assert_eq!(reclaimer.quarantine_bytes(), 0);
    assert_eq!(reclaimer.pending_items_count(), 0);
}

#[test]
fn test_quarantine_capacity_saturation_guard() {
    // Quarantine capacity = 1000 bytes
    let mut reclaimer = QuiescentEpochReclaimer::new(1000);

    // Retire 600 bytes -> Ok
    assert!(reclaimer.retire(1, 600).is_ok());
    assert_eq!(reclaimer.quarantine_bytes(), 600);

    // Retire 500 bytes -> Exceeds 1000 -> QuarantineSaturated Error!
    let err = reclaimer.retire(2, 500).unwrap_err();
    assert_eq!(
        err,
        EpochReclaimError::QuarantineSaturated {
            current_bytes: 600,
            capacity_bytes: 1000,
        }
    );
    assert_eq!(reclaimer.quarantine_bytes(), 600);
}

#[test]
fn test_stalled_thread_detection() {
    let mut reclaimer = QuiescentEpochReclaimer::new(1024 * 1024);
    reclaimer.register_thread(10);
    reclaimer.register_thread(20);

    // Thread 10 pins at epoch 1
    reclaimer.enter_critical_section(10).unwrap();

    // Advance 8 epochs (to epoch 9)
    for _ in 0..8 {
        reclaimer.advance_epoch();
    }
    assert_eq!(reclaimer.global_epoch(), 9);

    // Thread 20 pins at epoch 9
    reclaimer.enter_critical_section(20).unwrap();

    // Thread 10 is lagging by 8 epochs (9 - 1 = 8).
    // Stalled threads with max_lag = 5:
    let stalled = reclaimer.detect_stalled_threads(5);
    assert_eq!(stalled, vec![10]);

    // Thread 20 is at epoch 9 (lag = 0), not stalled
    let stalled_high_tol = reclaimer.detect_stalled_threads(10);
    assert!(stalled_high_tol.is_empty());
}
