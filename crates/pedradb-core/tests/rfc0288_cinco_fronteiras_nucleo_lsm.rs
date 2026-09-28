//! RFC-0288: Testes Unitários das Cinco Fronteiras Matemáticas e Estruturais do Núcleo LSM.
//!
//! Valida:
//! 1. Invariante de Teto de Descompressão e Streaming Bounded-M
//! 2. Bisimulação de Caminho e Barreira de Descida em MemTable SkipList
//! 3. Álgebra de Recomposição de Tuplas e Barreira de Homomorfismo Vertical
//! 4. Semianel de Transições de Versão e Idempotência de Fechamento no MANIFEST
//! 5. Autômato de Inversão de Direção em Iteradores de Fusão

use pedradb_core::bidi_iterator_reversal_kernel::{BidiCursorStateMachine, CursorDirection};
use pedradb_core::decompression_expansion_cap_kernel::{
    DecompressionCapError, DecompressionCapGovernor, DecompressionCapPolicy,
};
use pedradb_core::manifest_version_edit_semiring_kernel::{
    SstFileMetadata, VersionDelta, VersionEditSemiring, VersionState,
};
use pedradb_core::skiplist_weak_memory_barrier_kernel::{
    MonotonicityViolation, SearchStep, SkipListBarrierValidator, StepTransition,
};
use pedradb_core::sparse_tuple_homomorphism_kernel::{
    ColumnFragment, HomomorphismRejection, SparseTuple, SparseTupleHomomorphismVerifier,
};

#[test]
fn test_decompression_expansion_cap_governor() {
    let policy = DecompressionCapPolicy {
        max_expansion_ratio: 1024,
        max_absolute_bytes: 10 * 1024 * 1024, // 10 MiB
    };
    let governor = DecompressionCapGovernor::new(policy);

    // 1. Legitimate small expansion (e.g. 5x)
    assert!(governor.pre_validate_header(100, 500).is_ok());

    // 2. Excessive expansion ratio (> 1024x: 10 bytes -> 20,000 bytes)
    let err_ratio = governor.pre_validate_header(10, 20_000);
    assert!(matches!(err_ratio, Err(DecompressionCapError::ExcessiveDeclaredExpansion { ratio: 2000, max_ratio: 1024, .. })));

    // 3. Absolute ceiling exceeded (> 10 MiB)
    let err_ceil = governor.pre_validate_header(100 * 1024, 20 * 1024 * 1024);
    assert!(matches!(err_ceil, Err(DecompressionCapError::AbsoluteCeilingExceeded { .. })));

    // 4. Safe unpack of literal run
    let mut compressed = Vec::new();
    compressed.push(0); // literal tag
    compressed.extend_from_slice(&(4u16).to_le_bytes()); // length 4
    compressed.extend_from_slice(b"test");
    let unpacked = governor.safe_unpack(&compressed, 4).expect("unpack literal should succeed");
    assert_eq!(unpacked, b"test");

    // 5. Safe unpack of repeat run
    let mut comp_rep = Vec::new();
    comp_rep.push(1); // repeat tag
    comp_rep.extend_from_slice(&(10u16).to_le_bytes()); // repeat 10 times
    comp_rep.push(b'A');
    let unpacked_rep = governor.safe_unpack(&comp_rep, 10).expect("unpack repeat should succeed");
    assert_eq!(unpacked_rep, vec![b'A'; 10]);

    // 6. Runtime budget exceeded (declared 5, but stream produces 10)
    let err_runtime = governor.safe_unpack(&comp_rep, 5);
    assert!(matches!(err_runtime, Err(DecompressionCapError::RuntimeBudgetExceeded { .. })));
}

#[test]
fn test_skiplist_weak_memory_barrier_validator() {
    let target = b"key_050";

    // 1. Valid descending path: level 2 (head -> key_020) -> descend to level 1 (key_020 -> key_040) -> descend to level 0 (key_040 -> key_050)
    let valid_path = vec![
        SearchStep { level: 2, node_key: None, transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 2, node_key: Some(b"key_020".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: Some(b"key_020".to_vec()), transition: StepTransition::VerticalDescent },
        SearchStep { level: 1, node_key: Some(b"key_040".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 0, node_key: Some(b"key_040".to_vec()), transition: StepTransition::VerticalDescent },
        SearchStep { level: 0, node_key: Some(b"key_050".to_vec()), transition: StepTransition::HorizontalAdvance },
    ];
    assert!(SkipListBarrierValidator::verify_path(target, &valid_path).is_ok());

    // 2. Horizontal order inversion: advancing from key_040 to key_010 on level 1
    let inverted_path = vec![
        SearchStep { level: 1, node_key: Some(b"key_040".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: Some(b"key_010".to_vec()), transition: StepTransition::HorizontalAdvance },
    ];
    let err_inv = SkipListBarrierValidator::verify_path(target, &inverted_path);
    assert!(matches!(err_inv, Err(MonotonicityViolation::HorizontalOrderInversion { .. })));

    // 3. Horizontal overshoot: jumping past target key_050 to key_080
    let overshoot_path = vec![
        SearchStep { level: 0, node_key: Some(b"key_040".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 0, node_key: Some(b"key_080".to_vec()), transition: StepTransition::HorizontalAdvance },
    ];
    let err_over = SkipListBarrierValidator::verify_path(target, &overshoot_path);
    assert!(matches!(err_over, Err(MonotonicityViolation::HorizontalOvershoot { .. })));

    // 4. Vertical key shift: node key changes during descent
    let shift_path = vec![
        SearchStep { level: 2, node_key: Some(b"key_020".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: Some(b"key_030".to_vec()), transition: StepTransition::VerticalDescent },
    ];
    let err_shift = SkipListBarrierValidator::verify_path(target, &shift_path);
    assert!(matches!(err_shift, Err(MonotonicityViolation::VerticalKeyShift { .. })));

    // 5. Illegal level jump: descending from level 2 directly to level 0
    let jump_path = vec![
        SearchStep { level: 2, node_key: Some(b"key_020".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 0, node_key: Some(b"key_020".to_vec()), transition: StepTransition::VerticalDescent },
    ];
    let err_jump = SkipListBarrierValidator::verify_path(target, &jump_path);
    assert!(matches!(err_jump, Err(MonotonicityViolation::IllegalLevelJump { from_level: 2, to_level: 0 })));
}

#[test]
fn test_sparse_tuple_homomorphism() {
    let row_key = b"user:1001".to_vec();

    // 1. Clean tuple: all columns committed at seq 42 under snapshot 50
    let clean_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 1, seq_num: 42, value: b"alice".to_vec() },
            ColumnFragment { column_id: 2, seq_num: 42, value: b"active".to_vec() },
            ColumnFragment { column_id: 3, seq_num: 42, value: b"premium".to_vec() },
        ],
    };
    let verified_seq = SparseTupleHomomorphismVerifier::verify_homomorphism(&clean_tuple, 50, &[1, 2, 3])
        .expect("clean tuple should verify");
    assert_eq!(verified_seq, 42);

    // 2. Snapshot violation: column 2 has seq 55 > snapshot 50
    let future_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 1, seq_num: 42, value: b"alice".to_vec() },
            ColumnFragment { column_id: 2, seq_num: 55, value: b"updated".to_vec() },
        ],
    };
    let err_snap = SparseTupleHomomorphismVerifier::verify_homomorphism(&future_tuple, 50, &[1, 2]);
    assert!(matches!(err_snap, Err(HomomorphismRejection::SnapshotViolation { fragment_seq: 55, snapshot_seq: 50, .. })));

    // 3. Temporal tearing: column 1 at seq 40, column 2 at seq 42
    let torn_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 1, seq_num: 40, value: b"alice_old".to_vec() },
            ColumnFragment { column_id: 2, seq_num: 42, value: b"alice_new".to_vec() },
        ],
    };
    let err_tear = SparseTupleHomomorphismVerifier::verify_homomorphism(&torn_tuple, 50, &[1, 2]);
    assert!(matches!(err_tear, Err(HomomorphismRejection::TemporalTearing { min_seq: 40, max_seq: 42, .. })));

    // 4. Missing required column: requested [1, 2, 4], but column 4 is missing
    let err_miss = SparseTupleHomomorphismVerifier::verify_homomorphism(&clean_tuple, 50, &[1, 2, 4]);
    assert!(matches!(err_miss, Err(HomomorphismRejection::MissingProjectedColumn { column_id: 4 })));
}

#[test]
fn test_manifest_version_edit_semiring() {
    let mut state = VersionState::new();
    state.next_file_number = 10;
    state.last_sequence = 100;

    let sst1 = SstFileMetadata {
        file_number: 10,
        level: 1,
        file_size_bytes: 4096,
        smallest_key: b"a".to_vec(),
        largest_key: b"m".to_vec(),
    };
    let sst2 = SstFileMetadata {
        file_number: 11,
        level: 1,
        file_size_bytes: 4096,
        smallest_key: b"n".to_vec(),
        largest_key: b"z".to_vec(),
    };

    let delta1 = VersionDelta {
        added_files: vec![sst1.clone()],
        deleted_files: vec![],
        next_file_number: Some(12),
        last_sequence: Some(150),
    };

    // Apply delta1
    state = VersionEditSemiring::apply(state, &delta1);
    assert_eq!(state.total_file_count(), 1);
    assert!(state.contains_file(1, 10));
    assert_eq!(state.next_file_number, 12);
    assert_eq!(state.last_sequence, 150);

    // Semiring Idempotence Invariant: apply delta1 AGAIN
    let state_duplicate = VersionEditSemiring::apply(state.clone(), &delta1);
    assert_eq!(state, state_duplicate); // V + E + E == V + E

    // Apply compaction delta: deletes file 10, adds file 11
    let delta2 = VersionDelta {
        added_files: vec![sst2.clone()],
        deleted_files: vec![(1, 10)],
        next_file_number: Some(13),
        last_sequence: Some(200),
    };

    state = VersionEditSemiring::apply(state, &delta2);
    assert_eq!(state.total_file_count(), 1);
    assert!(!state.contains_file(1, 10));
    assert!(state.contains_file(1, 11));
    assert_eq!(state.next_file_number, 13);
    assert_eq!(state.last_sequence, 200);

    // Delta composition: E_12 = E_1 + E_2
    let composed = VersionEditSemiring::compose_deltas(&delta1, &delta2);
    let state_fresh = VersionEditSemiring::apply(VersionState::new(), &composed);
    assert_eq!(state_fresh.total_file_count(), 1);
    assert!(state_fresh.contains_file(1, 11));
    assert!(!state_fresh.contains_file(1, 10));
}

#[test]
fn test_bidi_iterator_reversal_automaton() {
    let items = vec![
        (b"key_a".to_vec(), b"val_a".to_vec()),
        (b"key_b".to_vec(), b"val_b".to_vec()),
        (b"key_c".to_vec(), b"val_c".to_vec()),
        (b"key_d".to_vec(), b"val_d".to_vec()),
    ];

    let mut it = BidiCursorStateMachine::new(items);

    // Seek to first
    let (k, _) = it.seek_to_first().expect("seek to first");
    assert_eq!(k, b"key_a");
    assert_eq!(it.direction(), CursorDirection::Forward);

    // Forward steps
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_b".as_slice()));
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_c".as_slice()));

    // DIRECTION REVERSAL: Prev(Next(it)) == it
    // Currently at "key_c". A prev() must immediately yield the preceding item "key_b"!
    assert_eq!(it.prev().map(|(k, _)| k), Some(b"key_b".as_slice()));
    assert_eq!(it.direction(), CursorDirection::Backward);

    assert_eq!(it.prev().map(|(k, _)| k), Some(b"key_a".as_slice()));
    assert_eq!(it.prev().map(|(k, _)| k), None); // Reached beginning

    // DIRECTION REVERSAL: moving forward after reaching beginning
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_a".as_slice()));
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_b".as_slice()));
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_c".as_slice()));
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_d".as_slice()));
    assert_eq!(it.next().map(|(k, _)| k), None); // Reached end

    // DIRECTION REVERSAL: moving backward after reaching end
    assert_eq!(it.prev().map(|(k, _)| k), Some(b"key_d".as_slice()));
    assert_eq!(it.prev().map(|(k, _)| k), Some(b"key_c".as_slice()));
}
