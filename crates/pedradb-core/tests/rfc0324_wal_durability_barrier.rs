//! RFC-0324: WAL Durability Barrier Verification Suite.
//!
//! Mechanically verifies that no client write can be acknowledged without
//! physical `fdatasync` proof (`DurabilityReceipt`), eradicating O1 data loss bugs.

#![forbid(unsafe_code)]

use pedradb_core::wal_durability_barrier_kernel::{
    DurabilityBarrierError, WalDurabilityBarrier, WriteTicket,
};

#[test]
fn rfc0324_write_ticket_construction_and_rejection() {
    // Ticket 0 is rejected
    assert_eq!(
        WriteTicket::try_new(0),
        Err(DurabilityBarrierError::ZeroTicketHazard)
    );

    let t1 = WriteTicket::try_new(1).expect("ticket 1 valid");
    assert_eq!(t1.as_u64(), 1);

    let t2 = WriteTicket::try_new(100).expect("ticket 100 valid");
    assert_eq!(t2.as_u64(), 100);
}

#[test]
fn rfc0324_durability_barrier_lifecycle_and_invariants() {
    let barrier = WalDurabilityBarrier::new();
    assert!(barrier.verify_durability_invariants());
    assert_eq!(barrier.issued_watermark(), 0);
    assert_eq!(barrier.flushed_watermark(), 0);
    assert_eq!(barrier.fsynced_watermark(), 0);
    assert_eq!(barrier.acked_watermark(), 0);

    // 1. Issue 3 tickets
    let t1 = barrier.issue_ticket();
    let t2 = barrier.issue_ticket();
    let t3 = barrier.issue_ticket();

    assert_eq!(t1.as_u64(), 1);
    assert_eq!(t2.as_u64(), 2);
    assert_eq!(t3.as_u64(), 3);
    assert_eq!(barrier.issued_watermark(), 3);
    assert!(barrier.verify_durability_invariants());

    // 2. Cannot fsync before flush
    let sync_err = barrier.record_fdatasync(t2);
    assert_eq!(
        sync_err,
        Err(DurabilityBarrierError::FsyncAheadOfFlush { ticket: 2, flushed: 0 })
    );

    // 3. Record flush up to t2
    barrier.record_flush(t2);
    assert_eq!(barrier.flushed_watermark(), 2);
    assert!(barrier.verify_durability_invariants());

    // 4. Record fsync up to t2 (satisfies t1 and t2)
    let receipt_t2 = barrier.record_fdatasync(t2).expect("fsync up to t2 ok");
    assert_eq!(receipt_t2.ticket(), t2);
    assert_eq!(barrier.fsynced_watermark(), 2);
    assert!(barrier.verify_durability_invariants());

    // 5. Acknowledge client with valid receipt
    barrier.acknowledge_client(&receipt_t2).expect("ack t2 ok");
    assert_eq!(barrier.acked_watermark(), 2);
    assert!(barrier.verify_durability_invariants());

    // 6. Attempt to acknowledge un-fsynced ticket t3 fails closed
    let _receipt_t1 = barrier.record_fdatasync(t1).expect("idempotent receipt for t1");
    // Tamper simulation: manually testing an un-fsynced ticket check
    let fake_unfsynced_receipt = pedradb_core::wal_durability_barrier_kernel::DurabilityReceipt::from_parts_for_test(t3, 2);
    let ack_err = barrier.acknowledge_client(&fake_unfsynced_receipt);
    assert_eq!(
        ack_err,
        Err(DurabilityBarrierError::PrematureAckHazard { ticket: 3, fsynced: 2 })
    );
    assert_eq!(barrier.acked_watermark(), 2);

    // 7. Flush and fsync t3 completes normally
    barrier.record_flush(t3);
    let receipt_t3 = barrier.record_fdatasync(t3).expect("fsync t3 ok");
    barrier.acknowledge_client(&receipt_t3).expect("ack t3 ok");

    assert_eq!(barrier.acked_watermark(), 3);
    assert_eq!(barrier.fsynced_watermark(), 3);
    assert_eq!(barrier.flushed_watermark(), 3);
    assert_eq!(barrier.issued_watermark(), 3);
    assert!(barrier.verify_durability_invariants());
}
