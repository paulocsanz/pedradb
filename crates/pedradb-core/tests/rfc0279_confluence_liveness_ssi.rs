//! RFC-0279 Test Suite:
//! - P0.1: Manifest Confluence (Church-Rosser LSM Diamond Property)
//! - P0.2: Liveness, Starvation-Freedom & Deadlock-Free Backpressure
//! - P1.1: Serializable Snapshot Isolation (SSI) & Write-Skew Detection
//! - P1.2: Merge Operator Associativity & CRDT Determinism
//! - P2.1: Causal Seam Order & Handler Invariant Enforcement

use std::collections::BTreeSet;

use pedradb_core::causal_seam_kernel::CausalWriteToken;
use pedradb_core::liveness_progress_kernel::{
    verify_backpressure_acyclicity, BackpressureResource, GroupCommitQueue,
};
use pedradb_core::manifest_confluence_kernel::{
    FileMetadata, VersionDelta, VersionSetState,
};
use pedradb_core::merge_determinism_kernel::{
    verify_associativity, verify_online_vs_compaction_equivalence, MergeOperand,
};
use pedradb_core::ssi_conflict_kernel::{SsiConflictGraph, TxFootprint};

#[test]
fn test_manifest_confluence_diamond_property() {
    let mut v0 = VersionSetState::empty();
    // Add initial base SSTs in L0 and L2
    let base_delta = VersionDelta {
        level: 0,
        deleted_files: BTreeSet::new(),
        added_files: vec![
            FileMetadata {
                file_num: 1,
                level: 0,
                smallest_key: 10,
                largest_key: 50,
                max_seq: 100,
            },
            FileMetadata {
                file_num: 2,
                level: 0,
                smallest_key: 60,
                largest_key: 90,
                max_seq: 110,
            },
            FileMetadata {
                file_num: 10,
                level: 2,
                smallest_key: 100,
                largest_key: 200,
                max_seq: 80,
            },
        ],
    };
    v0 = v0.apply_delta(&base_delta).expect("Base delta should apply");
    assert!(v0.verify_inventory_integrity());

    // Compaction 1: L0 -> L1 (deletes file 1, adds file 3 at L1)
    let delta_1 = VersionDelta {
        level: 1,
        deleted_files: [1].into_iter().collect(),
        added_files: vec![FileMetadata {
            file_num: 3,
            level: 1,
            smallest_key: 10,
            largest_key: 50,
            max_seq: 100,
        }],
    };

    // Compaction 2: L2 -> L3 (deletes file 10, adds file 11 at L3)
    let delta_2 = VersionDelta {
        level: 3,
        deleted_files: [10].into_iter().collect(),
        added_files: vec![FileMetadata {
            file_num: 11,
            level: 3,
            smallest_key: 100,
            largest_key: 200,
            max_seq: 80,
        }],
    };

    // Verify Church-Rosser Diamond Property: (V ⊕ Δ1) ⊕ Δ2 == (V ⊕ Δ2) ⊕ Δ1
    assert!(
        v0.verify_confluence(&delta_1, &delta_2),
        "Independent compactions must commute algebraically"
    );

    // Path 1
    let v1 = v0.apply_delta(&delta_1).unwrap();
    let v1_2 = v1.apply_delta(&delta_2).unwrap();

    // Path 2
    let v2 = v0.apply_delta(&delta_2).unwrap();
    let v2_1 = v2.apply_delta(&delta_1).unwrap();

    assert_eq!(v1_2, v2_1);
    assert!(v1_2.verify_inventory_integrity());
}

#[test]
fn test_liveness_potential_decrease_and_bounded_wait() {
    let mut q = GroupCommitQueue::new();
    // Enqueue 100 writers
    for client_id in 1..=100 {
        q.enqueue(client_id);
    }

    let initial_potential = q.potential();
    assert!(initial_potential > 0);

    // Advance epoch 1 (drains first 64 writers)
    let mut q_next = q.clone();
    let drained = q_next.advance_epoch();
    assert_eq!(drained, 64);
    assert!(q.verify_fair_progress(&q_next));
    assert!(q_next.potential() < initial_potential);

    // Advance epoch 2 (drains remaining 36 writers)
    let mut q_final = q_next.clone();
    let drained2 = q_final.advance_epoch();
    assert_eq!(drained2, 36);
    assert_eq!(q_final.potential(), 0);
    assert!(q_final.waiting_writers.is_empty());
}

#[test]
fn test_backpressure_acyclic_dag() {
    let valid_chain = vec![
        BackpressureResource::Admission,
        BackpressureResource::MemtableBuffer,
        BackpressureResource::DiskBandwidth,
        BackpressureResource::FlushReclaim,
    ];
    assert!(
        verify_backpressure_acyclicity(&valid_chain),
        "Monotonic backpressure pipeline must be strictly acyclic"
    );

    let cyclic_chain = vec![
        BackpressureResource::Admission,
        BackpressureResource::FlushReclaim,
        BackpressureResource::MemtableBuffer, // Cycle back to buffer allocation!
    ];
    assert!(
        !verify_backpressure_acyclicity(&cyclic_chain),
        "Backward dependency in backpressure must be rejected as potential deadlock"
    );
}

#[test]
fn test_ssi_write_skew_detection() {
    let mut graph = SsiConflictGraph::new();

    // Classic Write Skew scenario:
    // Constraint: Balance(A) + Balance(B) >= 0. Initially A=100, B=100.
    // T1 reads A and B, decides to withdraw 150 from A (writes A).
    // T2 concurrently reads A and B, decides to withdraw 150 from B (writes B).
    // Under Snapshot Isolation, both commit because write sets are disjoint!
    // But under Serializable Snapshot Isolation, this is a Write-Skew cycle.

    let t1 = TxFootprint {
        id: 1,
        snapshot_seq: 10,
        read_set: [1001, 1002].into_iter().collect(), // Reads A, B
        write_set: [1001].into_iter().collect(),       // Writes A
    };

    let t2 = TxFootprint {
        id: 2,
        snapshot_seq: 10,
        read_set: [1001, 1002].into_iter().collect(), // Reads A, B
        write_set: [1002].into_iter().collect(),       // Writes B
    };

    graph.register_transaction(t1).unwrap();
    graph.register_transaction(t2).unwrap();

    // In SSI, T1 has an rw-antidependency with T2 on key B (T1 read B, T2 overwrote B)
    // and T2 has an rw-antidependency with T1 on key A (T2 read A, T1 overwrote A).
    // This creates the cycle T1 -> T2 -> T1!
    assert!(
        graph.has_serialization_cycle(),
        "SSI must detect the rw-antidependency cycle and flag Write-Skew anomaly"
    );
    assert!(
        !graph.verify_write_skew_prevention(),
        "Hazardous execution must not pass write-skew verification"
    );
}

#[test]
fn test_merge_operator_associativity_and_determinism() {
    // 1. Counter addition associativity: (a + b) + c == a + (b + c)
    let c1 = MergeOperand::CounterAdd(10);
    let c2 = MergeOperand::CounterAdd(-3);
    let c3 = MergeOperand::CounterAdd(25);
    assert!(verify_associativity(&c1, &c2, &c3));

    // 2. String append associativity: (s1 ++ s2) ++ s3 == s1 ++ (s2 ++ s3)
    let s1 = MergeOperand::StringAppend(b"hello_".to_vec());
    let s2 = MergeOperand::StringAppend(b"pedra_".to_vec());
    let s3 = MergeOperand::StringAppend(b"db".to_vec());
    assert!(verify_associativity(&s1, &s2, &s3));

    // 3. Online vs Compaction equivalence
    let mem_ops = vec![
        MergeOperand::CounterAdd(100),
        MergeOperand::CounterAdd(50),
    ];
    let sst_ops = vec![
        MergeOperand::CounterAdd(-20),
        MergeOperand::CounterAdd(5),
    ];
    assert!(
        verify_online_vs_compaction_equivalence(&mem_ops, &sst_ops),
        "Compacted SSTs must yield identical value to dynamic online merge"
    );
}

#[test]
fn test_causal_seam_strict_ordering() {
    let mut token = CausalWriteToken::new(1001, true);

    // Cannot jump directly to ClientAck
    assert!(token.emit_client_ack().is_err());

    // Follow causal lifecycle
    assert!(token.mark_staged().is_ok());
    assert!(token.mark_written_to_os().is_ok());

    // Cannot ack before fsync
    assert!(token.emit_client_ack().is_err());

    // Perform physical fsync
    assert!(token.mark_fsynced().is_ok());

    // Commit manifest
    assert!(token.mark_manifest_committed().is_ok());

    // Now client ack is 100% legal and causal
    assert!(token.emit_client_ack().is_ok());
    assert!(token.verify_causal_soundness());
}
