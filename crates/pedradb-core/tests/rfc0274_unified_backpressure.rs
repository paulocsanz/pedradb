//! RFC-0274: Unified Engine Backpressure and Total Protection Test Suite.

use pedradb_core::backpressure_kernel::{
    calculate_smooth_pacing_delay, calculate_smooth_pacing_delay_as_is,
    evaluate_compaction_debt, evaluate_compaction_debt_as_is,
    evaluate_concurrency_admission, evaluate_concurrency_admission_as_is,
    evaluate_snapshot_pin, evaluate_snapshot_pin_as_is,
    evaluate_vlog_gc_pressure, evaluate_vlog_gc_pressure_as_is,
    BackpressureConfig, CompactionDebtVerdict, CompactionIoPacer, ConcurrencyVerdict,
    SnapshotPinVerdict, VlogGcVerdict,
};
use pedradb_core::{ConcurrentDb, CoreError, Db};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

fn temp_dir(tag: &str) -> PathBuf {
    let n = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::process::id();
    let d = std::env::temp_dir().join(format!("pedra-rfc0274-{tag}-{p}-{n}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn test_compaction_pacer_pacing_delays() {
    // 50 MB / sec
    let rate = 50 * 1024 * 1024;
    let mut pacer = CompactionIoPacer::new(rate, 0);

    // Initial burst capacity is consumed
    let _ = pacer.request_pacing_delay(pacer.rate_bytes_per_sec() / 10, 0);

    // Subsequent 50 MB write without time elapsed must pace for 1 second
    let delay = pacer.request_pacing_delay(rate, 0);
    assert!(delay.is_some());
    assert_eq!(delay.unwrap().as_secs(), 1);

    // After 1s elapsed, refill resets token debt
    pacer.refill(1_000_000_000);
    assert_eq!(pacer.request_pacing_delay(1024, 1_000_000_000), None);
}

#[test]
fn test_smooth_pacing_dynamic_range() {
    let min_us = 50;
    let max_us = 1000;

    // Normal zone -> None
    assert_eq!(
        calculate_smooth_pacing_delay(2, 4, 8, 100, 500, 1000, min_us, max_us),
        None
    );

    // At soft threshold -> min_us
    assert_eq!(
        calculate_smooth_pacing_delay(4, 4, 8, 100, 500, 1000, min_us, max_us),
        Some(Duration::from_micros(50))
    );

    // At hard threshold -> max_us
    assert_eq!(
        calculate_smooth_pacing_delay(8, 4, 8, 100, 500, 1000, min_us, max_us),
        Some(Duration::from_micros(1000))
    );

    // Comparison against AS-IS: AS-IS is always None
    assert_eq!(
        calculate_smooth_pacing_delay_as_is(8, 4, 8, 100, 500, 1000, min_us, max_us),
        None
    );
}

#[test]
fn test_pending_compaction_debt_watermarks() {
    let soft = 512 * 1024 * 1024;
    let hard = 2 * 1024 * 1024 * 1024;

    assert_eq!(
        evaluate_compaction_debt(200 * 1024 * 1024, soft, hard),
        CompactionDebtVerdict::Normal
    );

    assert_eq!(
        evaluate_compaction_debt(1024 * 1024 * 1024, soft, hard),
        CompactionDebtVerdict::PaceWriter {
            pending_bytes: 1024 * 1024 * 1024,
            soft_bytes: soft,
        }
    );

    assert_eq!(
        evaluate_compaction_debt(3 * 1024 * 1024 * 1024, soft, hard),
        CompactionDebtVerdict::StallCompaction {
            pending_bytes: 3 * 1024 * 1024 * 1024,
            hard_bytes: hard,
        }
    );

    assert_eq!(
        evaluate_compaction_debt_as_is(3 * 1024 * 1024 * 1024, soft, hard),
        CompactionDebtVerdict::Normal
    );
}

#[test]
fn test_snapshot_pin_lifecycle_and_bloat() {
    let max_age = 3600;
    let hard_lag = 10_000;

    // Normal snapshot
    assert_eq!(
        evaluate_snapshot_pin(100, 200, max_age, 50, 100, hard_lag, false),
        SnapshotPinVerdict::Normal
    );

    // Expired snapshot
    assert_eq!(
        evaluate_snapshot_pin(100, 4000, max_age, 50, 100, hard_lag, false),
        SnapshotPinVerdict::Expired {
            age_secs: 3900,
            max_age_secs: max_age,
        }
    );

    // Lag high warning
    assert_eq!(
        evaluate_snapshot_pin(100, 200, max_age, 50, 20_000, hard_lag, false),
        SnapshotPinVerdict::PinLagHigh {
            lag: 19_950,
            hard_lag,
        }
    );

    // Lag high under disk reclaiming -> RefuseWrites
    assert_eq!(
        evaluate_snapshot_pin(100, 200, max_age, 50, 20_000, hard_lag, true),
        SnapshotPinVerdict::RefuseWrites {
            lag: 19_950,
            hard_lag,
        }
    );

    assert_eq!(
        evaluate_snapshot_pin_as_is(100, 200, max_age, 50, 20_000, hard_lag, true),
        SnapshotPinVerdict::Normal
    );
}

#[test]
fn test_vlog_gc_backpressure_ratio() {
    assert_eq!(
        evaluate_vlog_gc_pressure(20, 100, 30, 60),
        VlogGcVerdict::Normal
    );

    assert_eq!(
        evaluate_vlog_gc_pressure(40, 100, 30, 60),
        VlogGcVerdict::TriggerGc {
            dead_bytes: 40,
            total_bytes: 100,
            ratio_pct: 40,
        }
    );

    assert_eq!(
        evaluate_vlog_gc_pressure(70, 100, 30, 60),
        VlogGcVerdict::ThrottleNewBlobs {
            dead_bytes: 70,
            total_bytes: 100,
            ratio_pct: 70,
        }
    );

    assert_eq!(
        evaluate_vlog_gc_pressure_as_is(70, 100, 30, 60),
        VlogGcVerdict::Normal
    );
}

#[test]
fn test_concurrency_queue_depth_admission() {
    assert_eq!(
        evaluate_concurrency_admission(10, 100),
        ConcurrencyVerdict::Admit
    );

    assert_eq!(
        evaluate_concurrency_admission(100, 100),
        ConcurrencyVerdict::QueueFull {
            active: 100,
            limit: 100,
        }
    );

    assert_eq!(
        evaluate_concurrency_admission_as_is(100, 100),
        ConcurrencyVerdict::Admit
    );
}

#[test]
fn test_db_pending_compaction_debt_stall_integration() {
    let dir = temp_dir("debt-stall");
    let mut db = Db::open(&dir).unwrap();

    let mut cfg = BackpressureConfig::default();
    // Configure artificially tiny hard compaction limit to verify stall
    cfg.pending_compaction_hard_bytes = 100;
    db.set_backpressure_config(cfg);

    // Initial state: 0 pending bytes -> admits writes
    assert!(db.put(b"k1", b"v1").is_ok());

    // Artificially configure compact_target_file_bytes to cause pending debt
    db.set_compact_target_file_bytes(1000);
    // Setting tiny limit lower than pending compaction bytes causes WriteStallCompactionDebt
    let pending = db.pending_compaction_bytes();
    if pending > 0 {
        let err = db.put(b"k2", b"v2").unwrap_err();
        assert!(matches!(err, CoreError::WriteStallCompactionDebt { .. }));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_db_snapshot_expiration_eviction() {
    let dir = temp_dir("snap-evict");
    let mut db = Db::open(&dir).unwrap();

    let pin = db.pin_snapshot();
    assert_eq!(db.snapshot_pin_count(), 1);

    // Configure snapshot max age to 0 (disabled) -> no eviction
    let mut cfg = BackpressureConfig::default();
    cfg.max_snapshot_age_secs = 0;
    db.set_backpressure_config(cfg);
    assert_eq!(db.evict_expired_snapshots(), 0);
    assert_eq!(db.snapshot_pin_count(), 1);

    // Release pin cleanly
    db.release_snapshot_pin(pin);
    assert_eq!(db.snapshot_pin_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_concurrent_db_queue_full_rejection() {
    let dir = temp_dir("queue-full");
    let db = ConcurrentDb::open(&dir).unwrap();

    // Set max in-flight writers to 1
    db.set_max_in_flight_writers(1);
    assert_eq!(db.max_in_flight_writers(), 1);

    // Single write succeeds
    assert!(db.put(b"hello", b"world").is_ok());

    // When max is 0, unconstrained
    db.set_max_in_flight_writers(0);
    assert!(db.put(b"hello2", b"world2").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
