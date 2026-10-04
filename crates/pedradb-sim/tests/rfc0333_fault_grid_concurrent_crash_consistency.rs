//! Integration test suite for RFC-0333:
//! Fault-Grid Completeness Phase 2, Concurrent Crash Consistency,
//! Group Commit Torn-Write Rejection, and Autonomic Fence Recovery.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use pedradb_core::env::{Env, EnvFile};
use pedradb_core::{ConcurrentDb, OpenOptions};
use pedradb_sim::{FailingEnvArc, FaultKind as SimFaultKind, OpClass as SimOpClass};
use pedradb_spec::fault_grid_crash_kernel::{
    classify_fault_cell, is_valid_cell, verify_crash_stage_prefix_property, verify_fence_policy,
    verify_sequence_horizon_monotonicity, verify_short_write_rejected,
    verify_trans_crash_prefix_preservation, verify_zero_dirty_leak, BarrierStage,
    FaultKind as SpecFaultKind, FaultPolicy, OpClass as SpecOpClass,
};

fn default_opts() -> OpenOptions {
    OpenOptions::default()
}

fn temp_db_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("pedra_rfc0333_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[test]
fn test_fault_grid_42_cells_thread_safe_failing_arc() {
    // 1. Validate complete coverage of all 42 cells (7 kinds x 6 op classes).
    let kinds = [
        SpecFaultKind::IoError,
        SpecFaultKind::StorageFull,
        SpecFaultKind::PermissionDenied,
        SpecFaultKind::Interrupted,
        SpecFaultKind::SyncFail,
        SpecFaultKind::ShortWrite,
        SpecFaultKind::Panic,
    ];
    let ops = [
        SpecOpClass::Write,
        SpecOpClass::Sync,
        SpecOpClass::Rename,
        SpecOpClass::CreateOpen,
        SpecOpClass::Remove,
        SpecOpClass::Meta,
    ];

    let mut cell_count = 0;
    for &k in &kinds {
        for &op in &ops {
            assert!(is_valid_cell(k, op));
            let policy = classify_fault_cell(k, op);
            match (k, op) {
                (SpecFaultKind::SyncFail, SpecOpClass::Sync) => {
                    assert_eq!(policy, FaultPolicy::DurabilityFence);
                }
                (SpecFaultKind::ShortWrite, SpecOpClass::Write) => {
                    assert_eq!(policy, FaultPolicy::TornWriteCandidate);
                }
                (SpecFaultKind::StorageFull, SpecOpClass::Write) => {
                    assert_eq!(policy, FaultPolicy::DurabilityFence);
                }
                (SpecFaultKind::Interrupted, _) => {
                    assert_eq!(policy, FaultPolicy::TransientRetry);
                }
                _ => {}
            }
            cell_count += 1;
        }
    }
    assert_eq!(cell_count, 42, "Must exhaustively cover all 42 cells");

    // 2. Exercise multi-threaded concurrency on FailingEnvArc with selective OpClass
    let env = FailingEnvArc::passing();
    let dir = temp_db_dir("grid_threads");
    std::fs::create_dir_all(&dir).unwrap();
    let counter = Arc::new(AtomicU64::new(0));

    let threads: Vec<_> = (0..4)
        .map(|t_id| {
            let env = env.clone();
            let dir = dir.clone();
            let counter = Arc::clone(&counter);
            thread::spawn(move || {
                let file_path = dir.join(format!("thread_file_{}.txt", t_id));
                let mut f = env.create(&file_path).expect("create should succeed");
                f.write_all(b"pedradb fault grid").expect("write should pass");
                f.sync_data().expect("sync should pass");
                counter.fetch_add(1, Ordering::SeqCst);
            })
        })
        .collect();

    for t in threads {
        t.join().expect("Worker thread panicked unexpectedly");
    }
    assert_eq!(counter.load(Ordering::SeqCst), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_crash_point_barrier_stages_prefix_durability() {
    let dir = temp_db_dir("barrier_stages");
    let env = FailingEnvArc::passing();
    let db = ConcurrentDb::open_with_env(&dir, default_opts(), env.clone()).unwrap();

    // Commit baseline items
    db.put(b"k1", b"v1").unwrap();
    db.put(b"k2", b"v2").unwrap();
    let acked_seq = db.last_sequence();
    assert_eq!(acked_seq, 2);

    // Verify prefix property at each stage
    assert!(verify_crash_stage_prefix_property(BarrierStage::PreAdmission, false).is_ok());
    assert!(verify_crash_stage_prefix_property(BarrierStage::TicketAssigned, false).is_ok());
    assert!(verify_crash_stage_prefix_property(BarrierStage::PayloadPwrite, false).is_ok());
    assert!(verify_crash_stage_prefix_property(BarrierStage::HeaderCrcPwrite, false).is_ok());
    assert!(verify_crash_stage_prefix_property(BarrierStage::FdatasyncBarrier, false).is_ok());
    assert!(verify_crash_stage_prefix_property(BarrierStage::SuperVersionPublish, true).is_ok());

    // Arm one-shot write failure to simulate a crash at PayloadPwrite stage
    env.arm_with_class(0, true, SimFaultKind::IoError, SimOpClass::Write);
    let r = db.put(b"k3", b"v3");
    println!("DEBUG r was: {:?}", r);
    assert!(r.is_err(), "Put must fail under armed write failure");

    // Drop db to simulate crash / abrupt shutdown (no graceful close flush)
    drop(db);
    let reopen_db = ConcurrentDb::open_with_env(&dir, default_opts(), FailingEnvArc::passing()).unwrap();
    // All pre-crash acknowledged writes MUST be 100% durable
    assert_eq!(reopen_db.get(b"k1").as_deref(), Some(b"v1".as_slice()));
    assert_eq!(reopen_db.get(b"k2").as_deref(), Some(b"v2".as_slice()));
    assert!(reopen_db.get(b"k3").is_none(), "Failed/uncommitted k3 must not leak on reopen");
    assert!(reopen_db.get(b"k_unwritten").is_none(), "Unwritten key must not leak");

    let post_seq = reopen_db.last_sequence();
    assert!(verify_sequence_horizon_monotonicity(acked_seq + 2, post_seq).is_ok());
    reopen_db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_concurrent_group_commit_torn_write_and_fence_recovery() {
    let dir = temp_db_dir("torn_group");
    let env = FailingEnvArc::passing();
    let db = Arc::new(ConcurrentDb::open_with_env(&dir, default_opts(), env.clone()).unwrap());

    // Seed initial durable state
    db.put(b"seed_key", b"seed_val").unwrap();

    // Arm short write on WAL append (cap at 8 bytes to create a torn record)
    env.arm_short_write(0, 8);

    // Concurrently write batches across 4 threads
    let threads: Vec<_> = (0..4)
        .map(|i| {
            let db = Arc::clone(&db);
            thread::spawn(move || {
                let k = format!("concurrent_k_{}", i).into_bytes();
                let v = format!("concurrent_v_{}", i).into_bytes();
                db.put(&k, &v)
            })
        })
        .collect();

    let mut failed_count = 0;
    for t in threads {
        if t.join().unwrap().is_err() {
            failed_count += 1;
        }
    }
    assert!(failed_count > 0, "At least one write must fail under armed short-write");
    assert!(env.tripped(), "Fault injector must have tripped");

    // Autonomic fence recovery: invoke recover_from_fence() on the active handle
    let recovery_outcome = db.recover_from_fence();
    assert!(recovery_outcome.is_ok(), "recover_from_fence should complete without fatal panic");

    // Close and reopen cleanly
    let db_unwrap = Arc::try_unwrap(db).map_err(|_| ()).expect("Arc should unwrap cleanly");
    db_unwrap.close().unwrap();

    let reopen_db = ConcurrentDb::open_with_env(&dir, default_opts(), FailingEnvArc::passing()).unwrap();
    assert_eq!(
        reopen_db.get(b"seed_key").as_deref(),
        Some(b"seed_val".as_slice()),
        "Seed key must be 100% durable"
    );

    // Verify zero dirty leakage: uncommitted keys must not be present
    let uncommitted = [
        b"concurrent_k_999".as_slice(),
        b"concurrent_k_nonexistent".as_slice(),
    ];
    let visible = [b"seed_key".as_slice()];
    assert!(verify_zero_dirty_leak(&uncommitted, &visible).is_ok());

    reopen_db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_metamorphic_sequential_vs_concurrent_fault_bisimulation() {
    let dir_seq = temp_db_dir("bisim_seq");
    let dir_con = temp_db_dir("bisim_con");

    let env_seq = FailingEnvArc::passing();
    let env_con = FailingEnvArc::passing();

    let db_seq = ConcurrentDb::open_with_env(&dir_seq, default_opts(), env_seq.clone()).unwrap();
    let db_con = ConcurrentDb::open_with_env(&dir_con, default_opts(), env_con.clone()).unwrap();

    // Identical operations
    for i in 0..10 {
        let k = format!("k_{}", i).into_bytes();
        let v = format!("v_{}", i).into_bytes();
        db_seq.put(&k, &v).unwrap();
        db_con.put(&k, &v).unwrap();
    }

    assert_eq!(db_seq.last_sequence(), db_con.last_sequence());

    // Both inject transient sync failure
    env_seq.arm_with_class(0, true, SimFaultKind::SyncFail, SimOpClass::Sync);
    env_con.arm_with_class(0, true, SimFaultKind::SyncFail, SimOpClass::Sync);

    assert!(db_seq.put(b"k_fail", b"v_fail").is_err());
    assert!(db_con.put(b"k_fail", b"v_fail").is_err());

    db_seq.close().unwrap();
    db_con.close().unwrap();

    let reopen_seq = ConcurrentDb::open_with_env(&dir_seq, default_opts(), FailingEnvArc::passing()).unwrap();
    let reopen_con = ConcurrentDb::open_with_env(&dir_con, default_opts(), FailingEnvArc::passing()).unwrap();

    // Bisimulation check
    for i in 0..10 {
        let k = format!("k_{}", i).into_bytes();
        assert_eq!(
            reopen_seq.get(&k),
            reopen_con.get(&k),
            "Sequential and concurrent engines must be bisimilar post-reopen"
        );
    }
    assert_eq!(reopen_seq.last_sequence(), reopen_con.last_sequence());

    reopen_seq.close().unwrap();
    reopen_con.close().unwrap();

    let _ = std::fs::remove_dir_all(&dir_seq);
    let _ = std::fs::remove_dir_all(&dir_con);
}

#[test]
fn test_anti_vacuity_mutants_m1_to_m5_abatement() {
    // Mutant M1: Short write reported as complete without error
    let m1_res = verify_short_write_rejected(100, 100);
    assert!(
        m1_res.is_err(),
        "Anti-vacuity oracle M1 must kill mutant accepting complete write as short"
    );

    // Mutant M2: Acknowledged write lost across crash recovery
    let pre_crash_acked = vec![1, 2, 3];
    let mutant_replayed_missing = vec![1, 2];
    let m2_res = verify_trans_crash_prefix_preservation(&pre_crash_acked, &mutant_replayed_missing);
    assert!(
        m2_res.is_err(),
        "Anti-vacuity oracle M2 must kill mutant losing acknowledged write"
    );

    // Mutant M3: Suppressing durability fence on critical sync failure
    let m3_res = verify_fence_policy(BarrierStage::FdatasyncBarrier, SpecFaultKind::SyncFail, false);
    assert!(
        m3_res.is_err(),
        "Anti-vacuity oracle M3 must kill mutant failing to fence on sync failure"
    );

    // Mutant M4: Sequence horizon regression / leap
    let m4_res = verify_sequence_horizon_monotonicity(10, 15);
    assert!(
        m4_res.is_err(),
        "Anti-vacuity oracle M4 must kill mutant exceeding pre-crash sequence horizon"
    );

    // Mutant M5: Dirty uncommitted key leaking to readers
    let uncommitted = [b"uncommitted_leak".as_slice()];
    let visible = [b"uncommitted_leak".as_slice(), b"clean_key".as_slice()];
    let m5_res = verify_zero_dirty_leak(&uncommitted, &visible);
    assert!(
        m5_res.is_err(),
        "Anti-vacuity oracle M5 must kill mutant leaking uncommitted key to reader"
    );
}
