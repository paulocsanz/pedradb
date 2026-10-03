//! Exhaustive Test Suite for RFC-0285:
//! Os Dez Pilares de Robustez e Verificação Formal do Ecossistema Caixote (Federation, Metal e Malha).
//!
//! Validates all 10 formal pillars with strict anti-vacuity oracles.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use bytes::Bytes;

use pedradb_core::asymmetric_partition_quorum_kernel::{
    AsymmetricQuorumError, AsymmetricQuorumGuard,
};
use pedradb_core::async_pool_decoupling_kernel::{
    DecoupledCommitScheduler, PoolDecouplingError,
};
use pedradb_core::boot_id_lockfile_kernel::{
    BootIdLockCoordinator, LockAction, LockfileError, LockfileToken,
};
use pedradb_core::fd_quota_governor_kernel::{FdQuotaError, FdQuotaGovernor};
use pedradb_core::federated_cursor_continuity_kernel::{
    ContinuityError, DeltaOp, FederatedFoldState, SequencedDelta,
};
use pedradb_core::lease_expiration_guard_kernel::{
    check_lease_validity, check_lease_validity_as_is, LeaseConfig, LeaseGrant, LeaseVerdict,
};
use pedradb_core::mesh_mtu_fragmentation_kernel::{
    fragment_message, FragmentationError, MessageId, Reassembler, MESH_SAFE_PAYLOAD_LIMIT,
};
use pedradb_core::metal_fsync_barrier_kernel::{
    step_unsafe_fast_rename_as_is, FsyncBarrierTracker, FsyncPhase, FsyncViolation,
};
use pedradb_core::multitenant_prefix_isolation_kernel::TenantPrefixCodec;
use pedradb_core::rolling_upgrade_homomorphism_kernel::ExtensibleMessage;

#[test]
fn test_pilar1_lease_expiration_and_vm_freeze_guard() {
    let grant = LeaseGrant {
        granted_at_ns: 1_000_000_000,
        config: LeaseConfig {
            duration_ns: 5_000_000_000, // 5s duration
            slack_ns: 500_000_000,     // 500ms slack
            max_drift_ns: 50_000_000,  // 50ms drift
        },
    };

    // 1. Within safe window (1s + 2s = 3s timestamp -> safe window has 1.4s remaining)
    match check_lease_validity(&grant, 3_000_000_000) {
        LeaseVerdict::Valid { remaining_safe_ns } => {
            assert!(remaining_safe_ns > 0);
        }
        other => panic!("Expected valid lease, got: {other:?}"),
    }

    // 2. Near boundary (inside slack grace zone -> fail-closed)
    assert_eq!(
        check_lease_validity(&grant, 5_600_000_000),
        LeaseVerdict::InGraceZone
    );

    // 3. VM Freeze scenario: time jumps by 10 seconds -> Expired
    assert_eq!(
        check_lease_validity(&grant, 11_000_000_000),
        LeaseVerdict::Expired
    );

    // 4. Time inversion (retroactive timestamp anomaly)
    assert_eq!(
        check_lease_validity(&grant, 500_000_000),
        LeaseVerdict::TimeInversionDetected
    );

    // 5. Anti-vacuity check: as-is mutator permits stale reads in grace zone
    assert_eq!(
        check_lease_validity_as_is(&grant, 5_600_000_000),
        LeaseVerdict::Valid { remaining_safe_ns: 400_000_000 }
    );
}

#[test]
fn test_pilar2_mesh_mtu_fragmentation_and_blackhole_defense() {
    let payload = vec![0xABu8; 3500]; // Exceeds 1320B safe MTU
    let msg_id = MessageId {
        sender_node: 42,
        sequence: 100,
    };

    let fragments = fragment_message(msg_id, &payload, MESH_SAFE_PAYLOAD_LIMIT)
        .expect("fragmentation should succeed");
    assert_eq!(fragments.len(), 3);

    for frag in &fragments {
        assert!(frag.payload.len() <= MESH_SAFE_PAYLOAD_LIMIT);
    }

    // 1. Orderly reassembly
    let mut reassembler = Reassembler::new(&fragments[0].header);
    let mut assembled = None;
    for frag in fragments.clone() {
        if let Some(res) = reassembler.add_fragment(frag).unwrap() {
            assembled = Some(res);
        }
    }
    assert_eq!(assembled.unwrap().as_ref(), payload.as_slice());

    // 2. Anti-vacuity: corrupted fragment CRC triggers failure
    let mut corrupted = fragments[1].clone();
    corrupted.header.fragment_crc32c ^= 0xFFFFFFFF; // Corrupt CRC
    let mut reassembler2 = Reassembler::new(&fragments[0].header);
    reassembler2.add_fragment(fragments[0].clone()).unwrap();
    assert!(matches!(
        reassembler2.add_fragment(corrupted),
        Err(FragmentationError::CorruptFragmentCrc { .. })
    ));
}

#[test]
fn test_pilar3_federated_cursor_continuity_and_bisimulation() {
    let mut state = FederatedFoldState::default();

    // 1. Initial atomic snapshot at sequence 100
    let snapshot_data = vec![
        (Bytes::from("alpha"), Bytes::from("val_a")),
        (Bytes::from("beta"), Bytes::from("val_b")),
    ];
    state.apply_atomic_snapshot(100, snapshot_data).unwrap();
    assert_eq!(state.last_sequence, 100);
    assert_eq!(state.entries.len(), 2);

    // 2. Strict contiguous delta application (101, 102)
    state.apply_deltas(&[SequencedDelta {
        sequence: 101,
        op: DeltaOp::Put {
            key: Bytes::from("gamma"),
            value: Bytes::from("val_g"),
        },
    }]).unwrap();
    assert_eq!(state.last_sequence, 101);

    state.apply_deltas(&[SequencedDelta {
        sequence: 102,
        op: DeltaOp::Delete {
            key: Bytes::from("alpha"),
        },
    }]).unwrap();
    assert_eq!(state.last_sequence, 102);
    assert!(!state.entries.contains_key(&Bytes::from("alpha")));

    // 3. Gap detection fail-closed (receiving 104 when 103 is expected)
    assert!(matches!(
        state.apply_deltas(&[SequencedDelta {
            sequence: 104,
            op: DeltaOp::Put {
                key: Bytes::from("omega"),
                value: Bytes::from("val_w"),
            },
        }]),
        Err(ContinuityError::SequenceGapDetected { expected: 103, got: 104 })
    ));

    // 4. Stale snapshot rejected
    assert!(matches!(
        state.apply_atomic_snapshot(99, vec![]),
        Err(ContinuityError::SnapshotOutdated { current_seq: 102, snapshot_seq: 99 })
    ));
}

#[test]
fn test_pilar4_metal_parent_dir_fsync_barrier() {
    let mut tracker = FsyncBarrierTracker::new("/var/data/0001.sst.tmp", "/var/data/0001.sst");

    // 1. Step-by-step rigorous state progression
    assert_eq!(tracker.current_phase, FsyncPhase::Uninitialized);
    tracker.step_write_data().unwrap();
    assert_eq!(tracker.current_phase, FsyncPhase::DataWritten);
    tracker.step_fdatasync_file().unwrap();
    assert_eq!(tracker.current_phase, FsyncPhase::FileSynced);
    tracker.step_fsync_parent_pre_rename().unwrap();
    assert_eq!(tracker.current_phase, FsyncPhase::PreRenameDirSynced);
    tracker.step_rename().unwrap();
    assert_eq!(tracker.current_phase, FsyncPhase::Renamed);
    tracker.step_fsync_parent_post_rename().unwrap();
    assert_eq!(tracker.current_phase, FsyncPhase::PostRenameDirSynced);

    // 2. Full barrier is crash-sound
    assert_eq!(tracker.assert_crash_soundness(), Ok(true));

    // 3. Anti-vacuity: crash between rename and post-rename dir sync is unsafe
    let mut uncommitted = FsyncBarrierTracker::new("/tmp/a", "/dest/a");
    step_unsafe_fast_rename_as_is(&mut uncommitted);
    assert!(matches!(
        uncommitted.assert_crash_soundness(),
        Err(FsyncViolation::CrashBeforeCommit { phase_at_crash: FsyncPhase::Renamed })
    ));
}

#[test]
fn test_pilar5_multitenant_prefix_isolation_and_non_interference() {
    let tenant_a = "tenant_alpha";
    let tenant_b = "tenant_alpha_beta"; // Prefix of another string!

    let key_1 = b"profile\x00secret";
    let key_2 = b"profile";

    let enc_a = TenantPrefixCodec::encode_key(tenant_a, key_1).unwrap();
    let enc_b = TenantPrefixCodec::encode_key(tenant_b, key_2).unwrap();

    // 1. Strict non-interference: length prefix prevents tenant_a from matching tenant_b
    assert!(TenantPrefixCodec::verify_non_interference(tenant_a, key_1, tenant_b, key_2));
    assert!(!enc_a.starts_with(&enc_b));
    assert!(!enc_b.starts_with(&enc_a));

    // 2. Decode isolates user key
    let decoded = TenantPrefixCodec::decode_key(tenant_a, &enc_a).unwrap();
    assert_eq!(decoded, key_1);

    // 3. Scan bounds
    let (lower, upper) = TenantPrefixCodec::tenant_scan_bounds(tenant_a).unwrap();
    assert!(enc_a >= lower && enc_a < upper);
    assert!(!(enc_b >= lower && enc_b < upper)); // Tenant B key falls outside Tenant A scan
}

#[test]
fn test_pilar6_async_pool_decoupling_and_starvation_freedom() {
    let scheduler = DecoupledCommitScheduler::new(5);

    // 1. Async submit produces monotonic tickets without blocking
    let t1 = scheduler.submit_async(1024, true).unwrap();
    let t2 = scheduler.submit_async(2048, false).unwrap();
    let t3 = scheduler.submit_async(512, true).unwrap();

    assert_eq!(t1.ticket_id, 1);
    assert_eq!(t2.ticket_id, 2);
    assert_eq!(t3.ticket_id, 3);
    assert_eq!(scheduler.pending_depth(), 3);

    // 2. Drain batch for IO worker
    let drained = scheduler.drain_for_io_worker(2);
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].ticket_id, 1);
    assert_eq!(drained[1].ticket_id, 2);

    // 3. Acknowledge committed batch
    assert_eq!(scheduler.acknowledge_committed_batch(&drained), Ok(2));
    assert!(scheduler.is_committed(1));
    assert!(scheduler.is_committed(2));
    assert!(!scheduler.is_committed(3));

    // 4. Backpressure under queue saturation (capacity is 5; 1 currently pending: ticket 3)
    let _t4 = scheduler.submit_async(100, false).unwrap();
    let _t5 = scheduler.submit_async(100, false).unwrap();
    let _t6 = scheduler.submit_async(100, false).unwrap();
    let _t7 = scheduler.submit_async(100, false).unwrap();
    assert_eq!(scheduler.pending_depth(), 5);
    assert!(matches!(
        scheduler.submit_async(100, false),
        Err(PoolDecouplingError::QueueSaturated { .. })
    ));
}

#[test]
fn test_pilar7_asymmetric_partition_quorum_and_split_brain_defense() {
    let nodes = [1, 2, 3, 4, 5];
    let mut guard = AsymmetricQuorumGuard::new(1, nodes);

    // Symmetrical links to 2 and 3
    guard.record_directed_link(1, 2, true, 5);
    guard.record_directed_link(2, 1, true, 5);

    guard.record_directed_link(1, 3, true, 10);
    guard.record_directed_link(3, 1, true, 10);

    // 1. Valid quorum with nodes 1, 2, 3 (majority 3 of 5)
    let q = guard.evaluate_quorum(&[1, 2, 3]).unwrap();
    assert_eq!(q, BTreeSet::from([1, 2, 3]));

    // 2. Asymmetric link to node 4: node 1 can send to 4, but 4 cannot send back
    guard.record_directed_link(1, 4, true, 20);
    guard.record_directed_link(4, 1, false, 0); // Broken return path!

    assert!(matches!(
        guard.evaluate_quorum(&[1, 2, 4]),
        Err(AsymmetricQuorumError::AsymmetricLinkDetected { from_node: 1, to_node: 4 })
    ));
}

#[test]
fn test_pilar8_fd_quota_governance_and_emfile_prevention() {
    let storage_limit = 3;
    let mesh_limit = 2;
    let os_limit = 6;
    let gov = FdQuotaGovernor::new(storage_limit, mesh_limit, os_limit);

    // 1. Acquire storage FDs
    let _s1 = gov.acquire_storage_fd().unwrap();
    let _s2 = gov.acquire_storage_fd().unwrap();
    let s3 = gov.acquire_storage_fd().unwrap();
    assert_eq!(gov.storage_allocated(), 3);

    // Storage quota exhausted
    assert!(matches!(
        gov.acquire_storage_fd(),
        Err(FdQuotaError::StorageQuotaExhausted { .. })
    ));

    // Mesh FDs can still be acquired independently
    let _m1 = gov.acquire_mesh_fd().unwrap();
    let _m2 = gov.acquire_mesh_fd().unwrap();
    assert_eq!(gov.mesh_allocated(), 2);

    // Mesh quota exhausted
    assert!(matches!(
        gov.acquire_mesh_fd(),
        Err(FdQuotaError::MeshQuotaExhausted { .. })
    ));

    // 2. Dropping a lease frees up quota
    drop(s3);
    assert_eq!(gov.storage_allocated(), 2);
    let _s_reclaimed = gov.acquire_storage_fd().unwrap();
    assert_eq!(gov.storage_allocated(), 3);
}

#[test]
fn test_pilar9_boot_id_lockfile_stale_reclaim_and_recovery() {
    let current_boot = "boot-uuid-2026-09-25";
    let current_pid = 12345;
    let coord = BootIdLockCoordinator::new(current_boot.to_string(), current_pid);

    // 1. Fresh acquisition
    assert_eq!(coord.evaluate_lock(&[], |_| false), Ok(LockAction::AcquireFresh));

    // 2. Stale lock from past boot (machine rebooted)
    let past_boot_token = LockfileToken {
        boot_id: "boot-uuid-OLD-2026-09-24".to_string(),
        pid: 9999,
        created_at_secs: 1727220000,
    };
    let encoded_past = past_boot_token.encode();
    assert!(matches!(
        coord.evaluate_lock(&encoded_past, |_| false),
        Ok(LockAction::ReclaimStaleLock { .. })
    ));

    // 3. Same boot, but process died (SIGKILL)
    let dead_process_token = LockfileToken {
        boot_id: current_boot.to_string(),
        pid: 54321,
        created_at_secs: 1727225000,
    };
    let encoded_dead = dead_process_token.encode();
    assert!(matches!(
        coord.evaluate_lock(&encoded_dead, |pid| pid == 12345), // 54321 does not exist
        Ok(LockAction::ReclaimStaleLock { .. })
    ));

    // 4. Live process contention
    let live_process_token = LockfileToken {
        boot_id: current_boot.to_string(),
        pid: 8888,
        created_at_secs: 1727226000,
    };
    let encoded_live = live_process_token.encode();
    assert!(matches!(
        coord.evaluate_lock(&encoded_live, |pid| pid == 8888),
        Err(LockfileError::ContendedByLiveProcess { pid: 8888, .. })
    ));
}

#[test]
fn test_pilar10_rolling_upgrade_schema_homomorphism() {
    use std::collections::BTreeMap;

    // V2 node sends message with Base Field (1) and Future Extension Field (2)
    let mut base = BTreeMap::new();
    base.insert(1, b"route_version_v1_base".to_vec());

    let mut ext = BTreeMap::new();
    ext.insert(2, b"quantum_wireguard_future_flag".to_vec());

    let v2_msg = ExtensibleMessage {
        wire_version: 2,
        base_fields: base,
        unknown_extensions: ext,
    };

    let encoded_v2 = v2_msg.encode();

    // V1 node (which only knows field <= 1) decodes the V2 message
    let v1_decoded = ExtensibleMessage::decode(&encoded_v2, 1).expect("V1 should decode V2 cleanly");
    assert_eq!(v1_decoded.wire_version, 2);
    assert!(v1_decoded.base_fields.contains_key(&1));
    assert!(v1_decoded.unknown_extensions.contains_key(&2));

    // Homomorphic round-trip: V1 node re-encodes the message and it is bit-for-bit identical to V2
    let re_encoded = v1_decoded.encode();
    assert_eq!(re_encoded, encoded_v2);
    assert!(ExtensibleMessage::verify_homomorphic_roundtrip(&encoded_v2, 1));
}
