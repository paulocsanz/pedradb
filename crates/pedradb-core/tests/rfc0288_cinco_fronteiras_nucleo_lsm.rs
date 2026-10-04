//! RFC-0288: Testes Unitários das Cinco Fronteiras Matemáticas e Estruturais do Núcleo LSM.
//!
//! Valida:
//! 1. Invariante de Teto de Descompressão e Streaming Bounded-M
//! 2. Bisimulação de Caminho e Barreira de Descida em MemTable SkipList
//! 3. Álgebra de Recomposição de Tuplas e Barreira de Homomorfismo Vertical
//! 4. Semianel de Transições de Versão e Idempotência de Fechamento no MANIFEST
//! 5. Autômato de Inversão de Direção em Iteradores de Fusão

use pedradb_core::bidi_iterator_reversal_kernel::{
    BidiCursorStateMachine, BidiIteratorError, CursorDirection,
};
use pedradb_core::decompression_expansion_cap_kernel::{
    DecompressionCapError, DecompressionCapGovernor, DecompressionCapPolicy,
    StreamingBoundedDecoder,
};
use pedradb_core::manifest_version_edit_semiring_kernel::{
    SstFileMetadata, VersionDelta, VersionEditSemiring, VersionState,
};
use pedradb_core::skiplist_weak_memory_barrier_kernel::{
    MonotonicityViolation, PathTerminalOutcome, SearchStep, SkipListBarrierValidator,
    StepTransition,
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
fn test_decompression_streaming_bounded_m_and_exact_length_green() {
    let policy = DecompressionCapPolicy {
        max_expansion_ratio: 1024,
        max_absolute_bytes: 10 * 1024 * 1024,
    };
    let governor = DecompressionCapGovernor::new(policy);

    // 1. safe_unpack_exact: detecta saída truncada quando stream produz menos bytes que o declarado
    let mut comp_rep = Vec::new();
    comp_rep.push(1); // repeat tag
    comp_rep.extend_from_slice(&(5u16).to_le_bytes()); // apenas 5 bytes 'X'
    comp_rep.push(b'X');
    // Declarado 10, mas payload só tem 5: deve falhar com TruncatedOutput
    let err_truncated = governor.safe_unpack_exact(&comp_rep, 10);
    assert_eq!(
        err_truncated,
        Err(DecompressionCapError::TruncatedOutput { produced: 5, expected: 10 })
    );

    // 2. safe_unpack_into com reuso de buffer pré-alocado (zero alocações extras)
    let mut buffer = Vec::with_capacity(64);
    let bytes_written = governor
        .safe_unpack_into(&comp_rep, 5, &mut buffer)
        .expect("deve descompactar em buffer existente");
    assert_eq!(bytes_written, 5);
    assert_eq!(buffer, vec![b'X'; 5]);

    // 3. Streaming Bounded-M decoder: decodifica em pedaços de no máximo M bytes
    // Cria stream com 3 blocos literais: [10 bytes 'A', 10 bytes 'B', 10 bytes 'C'] = 30 bytes
    let mut stream_comp = Vec::new();
    for &ch in &[b'A', b'B', b'C'] {
        stream_comp.push(1); // repeat
        stream_comp.extend_from_slice(&(10u16).to_le_bytes());
        stream_comp.push(ch);
    }

    let mut decoder = StreamingBoundedDecoder::new(&governor, 12, 30); // M = 12 bytes max chunk
    let mut accumulated = Vec::new();
    let mut chunks = 0;
    while let Some(chunk) = decoder.decode_next_chunk(&stream_comp).expect("chunk decode ok") {
        assert!(chunk.len() <= 12, "chunk size {} exceeds bounded M=12", chunk.len());
        accumulated.extend_from_slice(&chunk);
        chunks += 1;
    }
    assert_eq!(accumulated.len(), 30);
    assert!(chunks >= 3, "deve ter fatiado em pelo menos 3 chunks bounded-M");
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
fn test_skiplist_barrier_missing_key_and_complete_search_green() {
    let target = b"key_050";

    // 1. Eliminação de pânico: nó sem chave em avanço horizontal retorna MissingNodeKey em vez de expect panic
    let corrupt_path = vec![
        SearchStep { level: 1, node_key: Some(b"key_020".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: None, transition: StepTransition::HorizontalAdvance },
    ];
    let err_missing = SkipListBarrierValidator::verify_path(target, &corrupt_path);
    assert_eq!(err_missing, Err(MonotonicityViolation::MissingNodeKey { level: 1 }));

    // 2. Busca completa: caminho vazio deve falhar
    let err_empty = SkipListBarrierValidator::verify_complete_search(target, &[]);
    assert_eq!(err_empty, Err(MonotonicityViolation::EmptySearchPath));

    // 3. Busca completa que não desceu até o nível 0 é rejeitada como IncompleteDescent
    let incomplete_path = vec![
        SearchStep { level: 2, node_key: None, transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 2, node_key: Some(b"key_020".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: Some(b"key_020".to_vec()), transition: StepTransition::VerticalDescent },
    ];
    let err_incomplete = SkipListBarrierValidator::verify_complete_search(target, &incomplete_path);
    assert_eq!(err_incomplete, Err(MonotonicityViolation::IncompleteDescent { final_level: 1 }));

    // 4. Busca completa válida terminando em ExactMatch
    let complete_match = vec![
        SearchStep { level: 1, node_key: None, transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: Some(b"key_030".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 0, node_key: Some(b"key_030".to_vec()), transition: StepTransition::VerticalDescent },
        SearchStep { level: 0, node_key: Some(b"key_050".to_vec()), transition: StepTransition::HorizontalAdvance },
    ];
    let outcome_match = SkipListBarrierValidator::verify_complete_search(target, &complete_match)
        .expect("busca completa deve ser válida");
    assert_eq!(
        outcome_match,
        PathTerminalOutcome::ExactMatch { key: b"key_050".to_vec() }
    );

    // 5. Busca completa válida terminando no predecessor imediato
    let complete_pred = vec![
        SearchStep { level: 1, node_key: None, transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 1, node_key: Some(b"key_030".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 0, node_key: Some(b"key_030".to_vec()), transition: StepTransition::VerticalDescent },
        SearchStep { level: 0, node_key: Some(b"key_040".to_vec()), transition: StepTransition::HorizontalAdvance },
    ];
    let outcome_pred = SkipListBarrierValidator::verify_complete_search(target, &complete_pred)
        .expect("busca completa deve ser válida");
    assert_eq!(
        outcome_pred,
        PathTerminalOutcome::Predecessor { key: Some(b"key_040".to_vec()) }
    );
}

#[test]
fn test_skiplist_overshoot_in_single_step_and_level_exceeds_in_verify_path_red() {
    let target = b"key_050";

    // 1. Single-step path with key > target_key MUST NOT be accepted as Predecessor!
    let overshoot_single = vec![
        SearchStep { level: 0, node_key: Some(b"key_099".to_vec()), transition: StepTransition::HorizontalAdvance },
    ];
    let err_single = SkipListBarrierValidator::verify_path(target, &overshoot_single);
    assert!(
        matches!(err_single, Err(MonotonicityViolation::HorizontalOvershoot { .. })),
        "Single step with key > target must fail verify_path with HorizontalOvershoot"
    );

    let err_complete = SkipListBarrierValidator::verify_complete_search(target, &overshoot_single);
    assert!(
        matches!(err_complete, Err(MonotonicityViolation::HorizontalOvershoot { .. })),
        "Single step with key > target must fail verify_complete_search with HorizontalOvershoot"
    );

    // 2. verify_path must reject step.level >= MAX_SKIPLIST_HEIGHT
    let high_level_step = vec![
        SearchStep { level: 64, node_key: None, transition: StepTransition::HorizontalAdvance },
    ];
    let err_high = SkipListBarrierValidator::verify_path(target, &high_level_step);
    assert!(
        matches!(err_high, Err(MonotonicityViolation::LevelExceedsMaxHeight { level: 64, max_height: 32 })),
        "verify_path must reject step level exceeding MAX_SKIPLIST_HEIGHT"
    );

    // 3. Vertical descent carrying an overshot key must be rejected
    let vertical_overshoot = vec![
        SearchStep { level: 1, node_key: Some(b"key_080".to_vec()), transition: StepTransition::HorizontalAdvance },
        SearchStep { level: 0, node_key: Some(b"key_080".to_vec()), transition: StepTransition::VerticalDescent },
    ];
    let err_vert = SkipListBarrierValidator::verify_path(target, &vertical_overshoot);
    assert!(
        matches!(err_vert, Err(MonotonicityViolation::HorizontalOvershoot { .. })),
        "Vertical descent with key > target must fail with HorizontalOvershoot"
    );
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
fn test_sparse_tuple_duplicate_fragment_and_project_reconstruct_green() {
    let row_key = b"user:1001".to_vec();

    // 1. Rejeição de chave de linha vazia
    let empty_row_tuple = SparseTuple {
        row_key: Vec::new(),
        fragments: vec![ColumnFragment { column_id: 1, seq_num: 10, value: b"val".to_vec() }],
    };
    let err_empty_key = SparseTupleHomomorphismVerifier::verify_homomorphism(&empty_row_tuple, 20, &[1]);
    assert_eq!(err_empty_key, Err(HomomorphismRejection::EmptyRowKey));

    // 2. Rejeição de fragmentos duplicados da mesma coluna no mesmo registro
    let dup_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 1, seq_num: 42, value: b"first".to_vec() },
            ColumnFragment { column_id: 1, seq_num: 42, value: b"second_conflicting".to_vec() },
        ],
    };
    let err_dup = SparseTupleHomomorphismVerifier::verify_homomorphism(&dup_tuple, 50, &[1]);
    assert_eq!(err_dup, Err(HomomorphismRejection::DuplicateColumnFragment { column_id: 1 }));

    // 3. Projeção e reconstrução ordenada de valores zero-copy
    let multi_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 3, seq_num: 50, value: b"val_col3".to_vec() },
            ColumnFragment { column_id: 1, seq_num: 50, value: b"val_col1".to_vec() },
            ColumnFragment { column_id: 2, seq_num: 50, value: b"val_col2".to_vec() },
        ],
    };
    // Requisita projeção na ordem [2, 3, 1]
    let projected = SparseTupleHomomorphismVerifier::project_and_reconstruct(
        &multi_tuple,
        60,
        &[2, 3, 1],
    ).expect("deve projetar na ordem solicitada");
    assert_eq!(projected, vec![b"val_col2".as_slice(), b"val_col3".as_slice(), b"val_col1".as_slice()]);
}

#[test]
fn test_sparse_tuple_zero_seq_duplicate_req_and_empty_fragments_red() {
    let row_key = b"user:1002".to_vec();

    // 1. Fragment with seq_num == 0 must be rejected
    let zero_seq_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 1, seq_num: 0, value: b"val".to_vec() },
        ],
    };
    let err_zero = SparseTupleHomomorphismVerifier::verify_homomorphism(&zero_seq_tuple, 10, &[1]);
    assert_eq!(err_zero, Err(HomomorphismRejection::ZeroSequenceNumber { column_id: 1 }));

    // 2. Duplicate column ID in required_columns must be rejected
    let valid_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: vec![
            ColumnFragment { column_id: 1, seq_num: 10, value: b"val".to_vec() },
        ],
    };
    let err_dup_req = SparseTupleHomomorphismVerifier::verify_homomorphism(&valid_tuple, 20, &[1, 1]);
    assert_eq!(err_dup_req, Err(HomomorphismRejection::DuplicateRequiredColumn { column_id: 1 }));

    // 3. Empty fragments must be rejected even if required_columns is empty
    let empty_frag_tuple = SparseTuple {
        row_key: row_key.clone(),
        fragments: Vec::new(),
    };
    let err_empty = SparseTupleHomomorphismVerifier::verify_homomorphism(&empty_frag_tuple, 20, &[]);
    assert_eq!(err_empty, Err(HomomorphismRejection::EmptyFragments { row_key: row_key.clone() }));
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
fn test_manifest_semiring_composition_readd_and_level_disjointness_green() {
    let sst1_old = SstFileMetadata {
        file_number: 10,
        level: 1,
        file_size_bytes: 4096,
        smallest_key: b"a".to_vec(),
        largest_key: b"m".to_vec(),
    };
    let sst1_new = SstFileMetadata {
        file_number: 10,
        level: 1,
        file_size_bytes: 8192,
        smallest_key: b"a".to_vec(),
        largest_key: b"m".to_vec(),
    };

    // 1. Estado inicial V possui sst1_old
    let mut initial_state = VersionState::new();
    initial_state = VersionEditSemiring::apply(initial_state, &VersionDelta {
        added_files: vec![sst1_old.clone()],
        deleted_files: vec![],
        next_file_number: Some(11),
        last_sequence: Some(10),
    });
    assert!(initial_state.contains_file(1, 10));

    // Delta A deleta arquivo 10
    let delta_a = VersionDelta {
        added_files: vec![],
        deleted_files: vec![(1, 10)],
        next_file_number: None,
        last_sequence: Some(20),
    };

    // Delta B readiciona arquivo 10 com novos metadados
    let delta_b = VersionDelta {
        added_files: vec![sst1_new.clone()],
        deleted_files: vec![],
        next_file_number: None,
        last_sequence: Some(30),
    };

    // Aplicação sequencial: apply(apply(V, delta_a), delta_b)
    let state_seq = VersionEditSemiring::apply(
        VersionEditSemiring::apply(initial_state.clone(), &delta_a),
        &delta_b,
    );
    assert!(state_seq.contains_file(1, 10));
    assert_eq!(state_seq.files[&1][&10].file_size_bytes, 8192);

    // Aplicação composta: apply(V, compose(delta_a, delta_b))
    let composed = VersionEditSemiring::compose_deltas(&delta_a, &delta_b);
    let state_composed = VersionEditSemiring::apply(initial_state, &composed);

    // Homomorfismo estrito do semianel: V + (A . B) == (V + A) + B
    assert_eq!(
        state_composed.contains_file(1, 10),
        true,
        "delta B adicionando arquivo 10 após deleção pelo delta A deve prevalecer na composição"
    );
    assert_eq!(state_composed, state_seq);

    // 2. Validação de sobreposição proibida em L1+
    let sst_overlap_1 = SstFileMetadata {
        file_number: 21,
        level: 1,
        file_size_bytes: 4096,
        smallest_key: b"c".to_vec(),
        largest_key: b"h".to_vec(),
    };
    let sst_overlap_2 = SstFileMetadata {
        file_number: 22,
        level: 1,
        file_size_bytes: 4096,
        smallest_key: b"e".to_vec(),
        largest_key: b"k".to_vec(),
    };
    let mut overlap_state = VersionState::new();
    overlap_state = VersionEditSemiring::apply(overlap_state, &VersionDelta {
        added_files: vec![sst_overlap_1, sst_overlap_2],
        deleted_files: vec![],
        next_file_number: Some(30),
        last_sequence: Some(40),
    });
    // Deve detectar sobreposição em L1
    assert!(VersionEditSemiring::check_level_disjoint(&overlap_state, 1).is_err());
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

#[test]
fn test_bidi_iterator_seek_and_reversal_green() {
    // 1. try_new detecta itens desordenados
    let unsorted = vec![
        (b"key_z".to_vec(), b"val_z".to_vec()),
        (b"key_a".to_vec(), b"val_a".to_vec()),
    ];
    let err_unsorted = BidiCursorStateMachine::try_new(unsorted);
    assert_eq!(
        err_unsorted.err(),
        Some(BidiIteratorError::UnsortedKeys {
            prev_key: b"key_z".to_vec(),
            next_key: b"key_a".to_vec(),
        })
    );

    let items = vec![
        (b"key_10".to_vec(), b"val_10".to_vec()),
        (b"key_20".to_vec(), b"val_20".to_vec()),
        (b"key_30".to_vec(), b"val_30".to_vec()),
        (b"key_40".to_vec(), b"val_40".to_vec()),
    ];
    let mut it = BidiCursorStateMachine::try_new(items).expect("itens ordenados válidos");

    // 2. seek exato: key_20
    let (k, v) = it.seek(b"key_20").expect("seek key_20");
    assert_eq!(k, b"key_20");
    assert_eq!(v, b"val_20");
    assert!(it.is_valid());
    assert_eq!(it.current_value(), Some(b"val_20".as_slice()));

    // Imediata reversão após seek forward: prev() deve entregar key_10 sem off-by-one!
    assert_eq!(it.prev().map(|(k, _)| k), Some(b"key_10".as_slice()));
    assert_eq!(it.direction(), CursorDirection::Backward);

    // 3. seek_for_prev inexato: busca "key_25", deve posicionar em "key_20"
    let (k2, v2) = it.seek_for_prev(b"key_25").expect("seek_for_prev key_25");
    assert_eq!(k2, b"key_20");
    assert_eq!(v2, b"val_20");
    assert_eq!(it.direction(), CursorDirection::Backward);

    // Imediata reversão após seek_for_prev backward: next() deve entregar key_30 sem off-by-one!
    assert_eq!(it.next().map(|(k, _)| k), Some(b"key_30".as_slice()));
    assert_eq!(it.direction(), CursorDirection::Forward);
}
