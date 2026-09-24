//! RFC-0278 Comprehensive Mathematical Verification Suite:
//! - P0: LSM Bisimulation & Snapshot Equivalence Invariant
//! - P1: FSCQ-Class Crash-Recovery Refinement Relation
//! - P2: Bounded Allocation & Stack Limits Immunity

use pedradb_core::bounded_alloc_kernel::{compute_merge_stack_depth, AllocationBudget, PreallocatedWriteBuffer};
use pedradb_core::crash_refinement_kernel::{DiskLog, TxRecord};
use pedradb_core::lsm_bisimulation_kernel::{
    AbstractValue, CompactionPlan, LsmTreeState, PhysicalEntry,
};

#[test]
fn test_lsm_shadowing_monotonicity_sound() {
    let mut state = LsmTreeState::empty();
    // MemTable: Key 10 @ Seq 100
    state.memtable.push(PhysicalEntry {
        key: 10,
        seq: 100,
        val: 999,
        is_tombstone: false,
    });
    // L0: Key 10 @ Seq 80
    state.levels[0].push(PhysicalEntry {
        key: 10,
        seq: 80,
        val: 888,
        is_tombstone: false,
    });
    // L1: Key 10 @ Seq 50
    state.levels[1].push(PhysicalEntry {
        key: 10,
        seq: 50,
        val: 777,
        is_tombstone: false,
    });

    assert!(
        state.verify_shadowing_monotonicity(),
        "Shadowing monotonicity should hold when newer levels have higher seqs"
    );

    // Invert shadowing: lower level has higher sequence number
    state.levels[1].push(PhysicalEntry {
        key: 10,
        seq: 150, // BUG: L1 has seq 150 > Memtable's 100!
        val: 666,
        is_tombstone: false,
    });

    assert!(
        !state.verify_shadowing_monotonicity(),
        "Shadowing monotonicity must fail on sequence inversion across levels"
    );
}

#[test]
fn test_lsm_compaction_bisimulation_and_snapshot_isolation() {
    let mut state = LsmTreeState::empty();

    // L0 entries:
    // Key 1 @ Seq 10 (val: 101)
    // Key 2 @ Seq 20 (val: 201)
    // Key 1 @ Seq 30 (tombstone)
    state.levels[0].push(PhysicalEntry {
        key: 1,
        seq: 10,
        val: 101,
        is_tombstone: false,
    });
    state.levels[0].push(PhysicalEntry {
        key: 2,
        seq: 20,
        val: 201,
        is_tombstone: false,
    });
    state.levels[0].push(PhysicalEntry {
        key: 1,
        seq: 30,
        val: 0,
        is_tombstone: true,
    });

    // L1 entries:
    // Key 2 @ Seq 5 (val: 200)
    // Key 3 @ Seq 15 (val: 301)
    state.levels[1].push(PhysicalEntry {
        key: 2,
        seq: 5,
        val: 200,
        is_tombstone: false,
    });
    state.levels[1].push(PhysicalEntry {
        key: 3,
        seq: 15,
        val: 301,
        is_tombstone: false,
    });

    // Pre-compaction lookups at snapshot 15:
    // Key 1 should be visible @ seq 10
    // Key 2 should be visible @ seq 5
    // Key 3 should be visible @ seq 15
    assert_eq!(
        state.lookup(1, 15),
        AbstractValue::Present { val: 101, seq: 10 }
    );
    assert_eq!(
        state.lookup(2, 15),
        AbstractValue::Present { val: 200, seq: 5 }
    );
    assert_eq!(
        state.lookup(3, 15),
        AbstractValue::Present { val: 301, seq: 15 }
    );

    // At snapshot 35:
    // Key 1 should be Deleted @ seq 30
    // Key 2 should be Present @ seq 20
    assert_eq!(state.lookup(1, 35), AbstractValue::Deleted { seq: 30 });
    assert_eq!(
        state.lookup(2, 35),
        AbstractValue::Present { val: 201, seq: 20 }
    );

    // Apply Compaction from L0 to L1 with oldest active snapshot = 12
    let plan = CompactionPlan {
        source_level: 0,
        target_level: 1,
        oldest_active_snapshot: 12,
    };
    let compacted_state = plan.apply(&state).expect("Compaction should succeed");

    // Verify Bisimulation Equivalence Theorem
    let sample_keys = vec![1, 2, 3, 4];
    let active_snapshots = vec![15, 25, 35];
    assert!(
        plan.verify_bisimulation(&state, &compacted_state, &sample_keys, &active_snapshots),
        "Compaction must preserve exact point lookup and range scan results"
    );

    // Level 0 must be empty post-compaction
    assert!(compacted_state.levels[0].is_empty());
}

#[test]
fn test_crash_refinement_prefix_and_d1_durability() {
    let crc_fn = |seq: u64, len: u32| ((seq * 31) ^ (len as u64)) as u32;

    // Simulate runtime history with 5 transactions:
    // Tx 1, 2, 3 were synced and acked.
    // Tx 4 was written to OS buffers but NOT synced.
    // Tx 5 was in userland buffer.
    let history = vec![
        TxRecord {
            seq: 1,
            crc: crc_fn(1, 64),
            payload_len: 64,
            acked: true,
            synced: true,
        },
        TxRecord {
            seq: 2,
            crc: crc_fn(2, 128),
            payload_len: 128,
            acked: true,
            synced: true,
        },
        TxRecord {
            seq: 3,
            crc: crc_fn(3, 32),
            payload_len: 32,
            acked: true,
            synced: true,
        },
        TxRecord {
            seq: 4,
            crc: crc_fn(4, 96),
            payload_len: 96,
            acked: false,
            synced: false,
        },
        TxRecord {
            seq: 5,
            crc: crc_fn(5, 50),
            payload_len: 50,
            acked: false,
            synced: false,
        },
    ];

    // Scenario A: Clean crash, Tx 4 survived
    let disk_clean = DiskLog::simulate_crash(&history, 1, false);
    let recovered_clean = disk_clean.recover(crc_fn);
    assert_eq!(recovered_clean.len(), 4);
    assert!(DiskLog::verify_refinement(&history, &recovered_clean));

    // Scenario B: Power-cut torn sector on unsynced Tx 4
    let disk_torn = DiskLog::simulate_crash(&history, 1, true);
    let recovered_torn = disk_torn.recover(crc_fn);
    // Torn record 4 must be cleanly rejected, recovering exactly the 3 synced records
    assert_eq!(recovered_torn.len(), 3);
    assert!(DiskLog::verify_refinement(&history, &recovered_torn));

    // Scenario C: Total loss of unsynced records
    let disk_synced_only = DiskLog::simulate_crash(&history, 0, false);
    let recovered_synced = disk_synced_only.recover(crc_fn);
    assert_eq!(recovered_synced.len(), 3);
    assert!(DiskLog::verify_refinement(&history, &recovered_synced));
}

#[test]
fn test_bounded_alloc_and_stack_limits() {
    let mut budget = AllocationBudget::new();
    let mut write_buf = PreallocatedWriteBuffer::new();

    // 100 appends of 256 bytes each
    let data = [0xAAu8; 256];
    for _ in 0..100 {
        let res = write_buf.append(&data, &mut budget);
        assert!(res.is_ok());
    }

    // Mathematical zero-dynamic-allocation proof on critical write path
    assert!(
        budget.verify_zero_alloc_in_critical_path(),
        "Write path must never trigger heap reallocations when pre-allocated"
    );
    assert_eq!(budget.bytes_allocated, 25600);

    // Test MergeIterator bounded stack depth for all valid levels 0..7
    for levels in 1..=7 {
        let depth = compute_merge_stack_depth(levels).expect("Valid levels");
        budget.max_stack_depth = depth;
        assert!(
            budget.verify_bounded_stack_depth(),
            "LSM merge depth must be bounded <= 16 to guarantee stack safety"
        );
    }
}
