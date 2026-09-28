//! RFC-0289: Testes Unitários das Cinco Fronteiras Matemáticas Avançadas do LSM.
//!
//! Valida:
//! 1. Oráculo Construtivo de Separadores Monótonos sob Comparadores Abstratos
//! 2. Invariante de Alinhamento de Buracos e Continuidade de Offset no Compacting do VLog
//! 3. Álgebra de Leases Temporais Bounded com Desacoplamento Automático de Pinned Slices
//! 4. Convergência de Onda Dinâmica de Compactação por Teorema de Contração de Banach
//! 5. Prova Construtiva de Não-Reúso de Nonce e Integridade de Criptografia em Repouso

use pedradb_core::compaction_banach_contraction_kernel::{
    BanachCompactionScheduler, CompactionDebtVector,
};
use pedradb_core::crypto_nonce_space_time_kernel::{
    CryptoNonceGenerator, CryptoSpaceTimeCoords, AEAD_NONCE_SIZE, CANONICAL_NONCE_SIZE,
};
use pedradb_core::pinned_slice_lease_kernel::{LeaseState, PinnedSliceLease};
use pedradb_core::separator_order_preservation_kernel::SeparatorOrderOracle;
use pedradb_core::vlog_hole_alignment_kernel::{
    DeadValueSpan, SafeHolePunch, VLogHolePunchPlanner, DEFAULT_FS_BLOCK_SIZE,
};
use std::cmp::Ordering;

#[test]
fn test_separator_order_preservation_oracle() {
    let standard_cmp = |a: &[u8], b: &[u8]| a.cmp(b);

    // 1. Valid truncation between "abcdef" and "abczzz"
    let a = b"abcdef";
    let b = b"abczzz";
    let res = SeparatorOrderOracle::find_shortest_separator(a, b, &standard_cmp);
    assert!(res.was_truncated);
    assert_eq!(res.separator_key, b"abce");
    assert!(SeparatorOrderOracle::verify_bounds(a, b, &res.separator_key, &standard_cmp));

    // 2. Adjacent difference with no intermediate byte: "abcdef" and "abcdeg"
    // 'f' + 1 == 'g', so cannot increment 'f' without reaching 'g'
    let a_adj = b"abcdef";
    let b_adj = b"abcdeg";
    let res_adj = SeparatorOrderOracle::find_shortest_separator(a_adj, b_adj, &standard_cmp);
    assert!(!res_adj.was_truncated);
    assert_eq!(res_adj.separator_key, a_adj);
    assert!(SeparatorOrderOracle::verify_bounds(a_adj, b_adj, &res_adj.separator_key, &standard_cmp));

    // 3. Pre-condition violation (A >= B) -> returns A
    let res_inv = SeparatorOrderOracle::find_shortest_separator(b"key_b", b"key_a", &standard_cmp);
    assert!(!res_inv.was_truncated);
    assert_eq!(res_inv.separator_key, b"key_b");

    // 4. Custom inverted comparator where standard truncation would violate order
    let inverted_cmp = |x: &[u8], y: &[u8]| y.cmp(x);
    // Under inverted_cmp, "zzz" is LESS than "aaa"
    let inv_a = b"zzz";
    let inv_b = b"aaa";
    assert_eq!(inverted_cmp(inv_a, inv_b), Ordering::Less);
    let res_custom = SeparatorOrderOracle::find_shortest_separator(inv_a, inv_b, &inverted_cmp);
    // Standard candidate would violate inverted_cmp, oracle falls back safely to inv_a
    assert!(!res_custom.was_truncated);
    assert_eq!(res_custom.separator_key, inv_a);
    assert!(SeparatorOrderOracle::verify_bounds(inv_a, inv_b, &res_custom.separator_key, &inverted_cmp));
}

#[test]
fn test_vlog_hole_alignment_planner() {
    let planner = VLogHolePunchPlanner::new(DEFAULT_FS_BLOCK_SIZE); // 4096B

    // 1. Dead span crossing block boundaries: [100, 9000]
    // Ceil(100) = 4096, Floor(9000) = 8192
    let span1 = DeadValueSpan::new(100, 9000);
    let punch1 = planner.plan_span_punch(&span1).expect("punch should be planned");
    assert_eq!(punch1.aligned_start, 4096);
    assert_eq!(punch1.aligned_end, 8192);
    assert_eq!(punch1.punched_bytes, 4096);
    // Prove adjacent live bytes are never zeroed:
    assert!(punch1.aligned_start >= span1.start_offset);
    assert!(punch1.aligned_end <= span1.end_offset);

    // 2. Sub-block dead span: [1000, 3000]
    // Ceil(1000) = 4096, Floor(3000) = 0 -> start >= end, reject punch
    let span_small = DeadValueSpan::new(1000, 3000);
    assert_eq!(planner.plan_span_punch(&span_small), None);

    // 3. Exact block-aligned span: [4096, 12288]
    let span_exact = DeadValueSpan::new(4096, 12288);
    let punch_exact = planner.plan_span_punch(&span_exact).expect("exact punch");
    assert_eq!(punch_exact.aligned_start, 4096);
    assert_eq!(punch_exact.aligned_end, 12288);
    assert_eq!(punch_exact.punched_bytes, 8192);

    // 4. Multi-span planning with coalescing
    let spans = vec![
        DeadValueSpan::new(0, 4096),
        DeadValueSpan::new(4096, 8192),
        DeadValueSpan::new(9000, 9500), // sub-block, ignored
        DeadValueSpan::new(16384, 24576),
    ];
    let punches = planner.plan_multi_punch(&spans);
    assert_eq!(punches.len(), 2);
    // Spans 0..4096 and 4096..8192 coalesced into 0..8192
    assert_eq!(
        punches[0],
        SafeHolePunch {
            aligned_start: 0,
            aligned_end: 8192,
            punched_bytes: 8192,
        }
    );
    assert_eq!(
        punches[1],
        SafeHolePunch {
            aligned_start: 16384,
            aligned_end: 24576,
            punched_bytes: 8192,
        }
    );
}

#[test]
fn test_pinned_slice_lease_decoupling() {
    let payload = b"critical_column_payload_block_42".to_vec();
    let mut lease = PinnedSliceLease::new_pinned(
        42,              // block_id
        payload.clone(), // payload
        100,             // acquired_tick
        50,              // max_pin_ticks
    );

    assert!(lease.is_pinned_shared());
    assert!(!lease.is_detached_private());
    assert_eq!(lease.payload, payload);

    // 1. Tick 130 (elapsed = 30 <= 50): still within pin quota
    assert!(!lease.check_and_maybe_decouple(130));
    assert!(lease.is_pinned_shared());

    // 2. Tick 160 (elapsed = 60 > 50): quota exceeded, automatic transparent decoupling
    assert!(lease.check_and_maybe_decouple(160));
    assert!(!lease.is_pinned_shared());
    assert!(lease.is_detached_private());
    assert_eq!(
        lease.state,
        LeaseState::DetachedPrivateCopy {
            copied_bytes: payload.len()
        }
    );
    // Data remains fully accessible to the reader
    assert_eq!(lease.payload, payload);

    // 3. Subsequent tick: already decoupled, no redundant transition
    assert!(!lease.check_and_maybe_decouple(200));

    // 4. Explicit release
    lease.release();
    assert_eq!(lease.state, LeaseState::Released);
}

#[test]
fn test_compaction_banach_contraction_scheduler() {
    let gamma = 0.5f64;
    let scheduler = BanachCompactionScheduler::new(gamma);
    assert_eq!(scheduler.gamma(), 0.5);

    // 1. Contraction invariant verification: ||T(D1) - T(D2)||_inf <= gamma * ||D1 - D2||_inf
    let d1 = CompactionDebtVector::new(vec![5.0, 3.0, 2.0, 1.0]);
    let d2 = CompactionDebtVector::new(vec![1.0, 1.0, 0.5, 0.0]);
    let injection = vec![0.5, 0.2, 0.1, 0.0];

    assert!(scheduler.verify_contraction(&d1, &d2, &injection));

    // 2. Exponential decay towards zero under zero write injection
    let mut state = CompactionDebtVector::new(vec![16.0, 8.0, 4.0, 2.0]);
    let zero_injection = vec![0.0; 4];
    for _ in 0..10 {
        let prev_norm = state.norm_inf();
        state = scheduler.transition_step(&state, &zero_injection);
        let next_norm = state.norm_inf();
        // Each step must shrink the norm by exactly gamma
        assert!((next_norm - prev_norm * gamma).abs() < 1e-9);
    }
    // After 10 steps, 16.0 * (0.5)^10 = 16.0 / 1024 = 0.015625
    assert!((state.norm_inf() - 0.015625).abs() < 1e-9);

    // 3. Convergence to fixed point under constant write load
    // Fixed point D* = W / (1 - gamma) = 1.0 / (1 - 0.5) = 2.0
    let mut fixed_state = CompactionDebtVector::new(vec![10.0, 0.0, 50.0, 25.0]);
    let const_load = vec![1.0; 4];
    for _ in 0..30 {
        fixed_state = scheduler.transition_step(&fixed_state, &const_load);
    }
    for debt in &fixed_state.level_debts {
        assert!((debt - 2.0).abs() < 1e-6, "debt {} must converge to 2.0", debt);
    }
}

#[test]
fn test_crypto_nonce_space_time_generator() {
    let uuid_a = [1u8; 16];
    let uuid_b = [2u8; 16];

    let base_coords = CryptoSpaceTimeCoords::new(uuid_a, 10, 100, 4096);

    // 1. Exact coordinates equality check
    let dup_coords = CryptoSpaceTimeCoords::new(uuid_a, 10, 100, 4096);
    assert!(!CryptoNonceGenerator::are_disjoint(&base_coords, &dup_coords));

    // 2. Orthogonality across each of the 4 dimensions
    let diff_uuid = CryptoSpaceTimeCoords::new(uuid_b, 10, 100, 4096);
    let diff_epoch = CryptoSpaceTimeCoords::new(uuid_a, 11, 100, 4096);
    let diff_file = CryptoSpaceTimeCoords::new(uuid_a, 10, 101, 4096);
    let diff_offset = CryptoSpaceTimeCoords::new(uuid_a, 10, 100, 8192);

    assert!(CryptoNonceGenerator::are_disjoint(&base_coords, &diff_uuid));
    assert!(CryptoNonceGenerator::are_disjoint(&base_coords, &diff_epoch));
    assert!(CryptoNonceGenerator::are_disjoint(&base_coords, &diff_file));
    assert!(CryptoNonceGenerator::are_disjoint(&base_coords, &diff_offset));

    // 3. Injective canonical nonce generation (320 bits / 40 bytes)
    let n_base = CryptoNonceGenerator::generate_canonical_nonce(&base_coords);
    let n_uuid = CryptoNonceGenerator::generate_canonical_nonce(&diff_uuid);
    let n_epoch = CryptoNonceGenerator::generate_canonical_nonce(&diff_epoch);
    let n_file = CryptoNonceGenerator::generate_canonical_nonce(&diff_file);
    let n_offset = CryptoNonceGenerator::generate_canonical_nonce(&diff_offset);

    assert_eq!(n_base.len(), CANONICAL_NONCE_SIZE);
    assert_ne!(n_base, n_uuid);
    assert_ne!(n_base, n_epoch);
    assert_ne!(n_base, n_file);
    assert_ne!(n_base, n_offset);

    // 4. AEAD 96-bit (12 bytes) non-colliding derivation
    let aead_base = CryptoNonceGenerator::derive_aead_nonce_96(&base_coords);
    let aead_epoch = CryptoNonceGenerator::derive_aead_nonce_96(&diff_epoch);
    let aead_file = CryptoNonceGenerator::derive_aead_nonce_96(&diff_file);
    let aead_offset = CryptoNonceGenerator::derive_aead_nonce_96(&diff_offset);

    assert_eq!(aead_base.len(), AEAD_NONCE_SIZE);
    assert_ne!(aead_base, aead_epoch);
    assert_ne!(aead_base, aead_file);
    assert_ne!(aead_base, aead_offset);
}
