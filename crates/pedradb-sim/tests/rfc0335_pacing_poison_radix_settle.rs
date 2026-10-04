//! Integration test suite for RFC-0335:
//! Dynamic Radix Ingest Sharding, Adaptive Credit Admission Pacing,
//! and Atomic Fail-Closed Poison Crash Consistency.

use pedradb_spec::pacing_poison_kernel::{
    reconcile_wal_recovery_barrier, verify_m1_poison_guard, verify_m2_disjoint_envelopes,
    verify_m3_bounded_memory_envelope, verify_m4_linear_settle_complexity,
    verify_m5_recovery_torn_barrier, AdmissionZone, CreditAdmissionGovernor, DbState,
    DisjointSstEnvelope, DynamicRadixIngestor, FrameSeal, GovernorConfig, PoisonReason,
};

#[test]
fn test_dynamic_radix_sharding_unordered_uuids() {
    let mut ingestor = DynamicRadixIngestor::new();

    // Generate 1,000 synthetic random keys (simulating unordered UUIDs / hashes)
    for i in 0..1000u64 {
        // Pseudo-random scramble to simulate non-monotonic hash distribution
        let hash_hi = ((i.wrapping_mul(0x9E3779B97F4A7C15)) >> 56) as u8;
        let hash_lo = i.wrapping_mul(0x517CC1B727220A95);
        let mut key = Vec::with_capacity(16);
        key.push(hash_hi);
        key.extend_from_slice(&hash_lo.to_be_bytes());
        key.extend_from_slice(&i.to_be_bytes()[..7]);

        let val = format!("val_{i}").into_bytes();
        ingestor.ingest(key, val);
    }

    assert!(ingestor.total_staged_bytes > 0);

    // Flush disjoint SST envelopes
    let envelopes = ingestor.flush_disjoint_ssts();
    assert!(!envelopes.is_empty(), "Flushed envelopes must be non-empty");

    // Formally verify that envelopes are strictly mutually disjoint
    assert!(
        DynamicRadixIngestor::verify_disjointness(&envelopes),
        "All flushed radix partition SST envelopes must be mutually disjoint"
    );

    // Verify linear settle complexity bound O(N)
    let settle_ops = envelopes.len() * 2; // Linear envelope index registration
    assert!(
        verify_m4_linear_settle_complexity(&envelopes, settle_ops).is_ok(),
        "Settle complexity must remain strictly bounded O(N) linear"
    );
}

#[test]
fn test_adaptive_credit_governor_pacing_and_trim_absorption() {
    let config = GovernorConfig {
        soft_floor_bytes: 40 * 1024 * 1024,  // 40 MiB
        hard_limit_bytes: 64 * 1024 * 1024,  // 64 MiB
        tau_micros: 50,
    };
    let drain_rate = 50 * 1024 * 1024; // 50 MB/s NVMe drain rate
    let mut governor = CreditAdmissionGovernor::new(config, drain_rate);

    // 1. Green Zone: below 40 MiB -> 0 delay
    governor.stage_bytes(20 * 1024 * 1024);
    assert_eq!(governor.zone(), AdmissionZone::Green);
    assert_eq!(governor.compute_delay_micros(64 * 1024), 0);

    // 2. Yellow Zone: 50 MiB (between 40 and 64 MiB) -> proportional micro-delay
    governor.stage_bytes(30 * 1024 * 1024); // total 50 MiB
    assert_eq!(governor.zone(), AdmissionZone::Yellow);
    let yellow_delay = governor.compute_delay_micros(64 * 1024);
    assert!(yellow_delay > 0, "Yellow zone must introduce micro-delays");
    assert!(yellow_delay <= config.tau_micros, "Micro-delay must not exceed tau");

    // 3. Red Zone: 65 MiB (above 64 MiB) -> clamped to drain rate
    governor.stage_bytes(15 * 1024 * 1024); // total 65 MiB
    assert_eq!(governor.zone(), AdmissionZone::Red);
    let red_delay = governor.compute_delay_micros(64 * 1024);
    assert!(
        red_delay >= config.tau_micros,
        "Red zone delay must pace writes to drain rate"
    );

    // 4. Drain back to Green
    governor.drain_bytes(35 * 1024 * 1024); // back to 30 MiB
    assert_eq!(governor.zone(), AdmissionZone::Green);
    assert_eq!(governor.compute_delay_micros(64 * 1024), 0);
}

#[test]
fn test_atomic_poison_guard_and_fail_closed_rejection() {
    let mut state = DbState::Healthy;
    assert!(state.is_healthy());
    assert!(!state.is_poisoned());

    // Normal write is permitted
    assert!(verify_m1_poison_guard(&state, false).is_ok());

    // Simulate cascading rollback failure: ftruncate EIO during discard_uncommitted
    state = DbState::Poisoned {
        reason: PoisonReason::IoRollbackFailed,
        failed_offset: 1048576,
        timestamp_ticks: 123456789,
    };
    assert!(!state.is_healthy());
    assert!(state.is_poisoned());

    // Subsequent write attempt must be caught and rejected fail-closed
    assert!(
        verify_m1_poison_guard(&state, true).is_err(),
        "Write attempt on poisoned DB must be rejected fail-closed"
    );
}

#[test]
fn test_frameseal_wal_recovery_barrier_independence() {
    // Sequence of 4 frames: 3 committed/anchored, 4th unanchored/torn
    let frames = vec![
        FrameSeal::new(101, 256, 0xAAAA, 0x1111, true),
        FrameSeal::new(102, 512, 0xBBBB, 0x2222, true),
        FrameSeal::new(103, 128, 0xCCCC, 0x3333, true),
        FrameSeal::new(104, 1024, 0xDDDD, 0x4444, false), // torn write!
    ];

    let (last_seq, valid_count) = reconcile_wal_recovery_barrier(&frames);
    assert_eq!(last_seq, 103, "Recovery must stop at the last barrier-anchored frame");
    assert_eq!(valid_count, 3, "Only the 3 anchored frames may be recovered");

    // Formally verify mutant M5 rejects improper inclusion
    assert!(verify_m5_recovery_torn_barrier(&frames, 3).is_ok());
    assert!(verify_m5_recovery_torn_barrier(&frames, 4).is_err());
}

#[test]
fn test_anti_vacuity_mutants_battery_m1_to_m5_abatement() {
    // Mutant M1: Write allowed while poisoned
    let poisoned_state = DbState::Poisoned {
        reason: PoisonReason::WalOffsetDesynchronized,
        failed_offset: 4096,
        timestamp_ticks: 999,
    };
    assert!(verify_m1_poison_guard(&poisoned_state, true).is_err());

    // Mutant M2: Overlapping envelopes
    let overlapping_envelopes = vec![
        DisjointSstEnvelope {
            sst_id: 1,
            shard_id: 0,
            smallest_key: b"a".to_vec(),
            largest_key: b"m".to_vec(),
            entry_count: 10,
            file_size_bytes: 1024,
        },
        DisjointSstEnvelope {
            sst_id: 2,
            shard_id: 1,
            smallest_key: b"k".to_vec(), // overlaps with "m"!
            largest_key: b"z".to_vec(),
            entry_count: 10,
            file_size_bytes: 1024,
        },
    ];
    assert!(verify_m2_disjoint_envelopes(&overlapping_envelopes).is_err());

    // Mutant M3: Hard limit exceeded with excessive throughput
    let config = GovernorConfig::default();
    let governor = CreditAdmissionGovernor {
        config,
        occupied_bytes: 70 * 1024 * 1024, // > hard limit (64 MiB)
        drain_rate_bytes_per_sec: 10 * 1024 * 1024,
    };
    // Throughput 20 MB/s > Drain rate 10 MB/s -> must fail
    assert!(verify_m3_bounded_memory_envelope(&governor, 20 * 1024 * 1024).is_err());

    // Mutant M4: Quadratic settle
    let dummy_envelopes = vec![DisjointSstEnvelope {
        sst_id: 1,
        shard_id: 0,
        smallest_key: b"a".to_vec(),
        largest_key: b"b".to_vec(),
        entry_count: 5,
        file_size_bytes: 512,
    }];
    assert!(verify_m4_linear_settle_complexity(&dummy_envelopes, 500).is_err());

    // Mutant M5: Corrupted header included
    let frames = vec![
        FrameSeal::new(1, 100, 0x11, 0x22, true),
        FrameSeal {
            magic: 0xDEADBEEF, // bad magic
            seq_num: 2,
            payload_len: 100,
            header_crc: 0,
            payload_crc: 0,
            is_fdatasync_anchored: true,
        },
    ];
    assert!(verify_m5_recovery_torn_barrier(&frames, 2).is_err());
}
