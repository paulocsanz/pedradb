//! RFC-0289: Testes Unitários das Cinco Fronteiras Matemáticas Avançadas do LSM.
//!
//! Valida:
//! 1. Oráculo Construtivo de Separadores Monótonos sob Comparadores Abstratos
//! 2. Invariante de Alinhamento de Buracos e Continuidade de Offset no Compacting do VLog
//! 3. Álgebra de Leases Temporais Bounded com Desacoplamento Automático de Pinned Slices
//! 4. Convergência de Onda Dinâmica de Compactação por Teorema de Contração de Banach
//! 5. Prova Construtiva de Não-Reúso de Nonce e Integridade de Criptografia em Repouso

use pedradb_core::compaction_banach_contraction_kernel::{
    BanachCompactionError, BanachCompactionScheduler, CompactionDebtVector,
};
use pedradb_core::crypto_nonce_space_time_kernel::{
    CryptoNonceError, CryptoNonceGenerator, CryptoSpaceTimeCoords, AEAD_NONCE_SIZE,
    CANONICAL_NONCE_SIZE,
};
use pedradb_core::pinned_slice_lease_kernel::{LeaseState, PinnedSliceLease};
use pedradb_core::separator_order_preservation_kernel::SeparatorOrderOracle;
use pedradb_core::vlog_hole_alignment_kernel::{
    DeadValueSpan, SafeHolePunch, VLogHoleAlignmentError, VLogHolePunchPlanner,
    DEFAULT_FS_BLOCK_SIZE,
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
fn test_separator_order_preservation_boundary_and_successor_green() {
    let standard_cmp = |a: &[u8], b: &[u8]| a.cmp(b);

    // 1. Truncation when diff_byte + 1 == b[diff_idx] and b has trailing bytes:
    // a = "abcdef" (6 bytes), b = "abde" (4 bytes).
    // diff_idx = 2 ('c' vs 'd'). diff_byte + 1 == 'd'.
    // candidate = "abd" (3 bytes). "abcdef" < "abd" < "abde".
    // Must be successfully shortened to "abd" (saving 3 bytes).
    let res = SeparatorOrderOracle::find_shortest_separator(b"abcdef", b"abde", &standard_cmp);
    assert!(res.was_truncated, "Must successfully truncate to 'abd'");
    assert_eq!(res.separator_key, b"abd");
    assert!(SeparatorOrderOracle::verify_bounds(b"abcdef", b"abde", &res.separator_key, &standard_cmp));

    // 2. No length reduction when candidate.len() == a.len()
    // a = "ab" (len 2), b = "ad" (len 2).
    // An equal-length candidate saves 0 bytes, so was_truncated must be false.
    let res_no_len_reduction = SeparatorOrderOracle::find_shortest_separator(b"ab", b"ad", &standard_cmp);
    assert!(!res_no_len_reduction.was_truncated, "Equal length separator must not report was_truncated=true");
    assert_eq!(res_no_len_reduction.separator_key, b"ab");

    // 3. Short successor computation for SST block builders
    let succ = SeparatorOrderOracle::find_short_successor(b"abcdef", &standard_cmp);
    assert!(succ.was_truncated);
    assert_eq!(succ.separator_key, b"b");
    assert_eq!(standard_cmp(b"abcdef", &succ.separator_key), Ordering::Less);

    // 4. Short successor with all 0xff bytes
    let all_ff = vec![0xff, 0xff, 0xff];
    let succ_ff = SeparatorOrderOracle::find_short_successor(&all_ff, &standard_cmp);
    assert!(!succ_ff.was_truncated);
    assert_eq!(succ_ff.separator_key, all_ff);

    // 5. Short successor under inverted comparator
    let inverted_cmp = |x: &[u8], y: &[u8]| y.cmp(x);
    let inv_succ = SeparatorOrderOracle::find_short_successor(b"middle", &inverted_cmp);
    assert!(!inv_succ.was_truncated);
    assert_eq!(inv_succ.separator_key, b"middle");
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

    // 5. Invariant: Contiguous sub-block spans must coalesce in byte space before alignment
    let spans_sub = vec![
        DeadValueSpan::new(1000, 5000), // 4000 dead bytes
        DeadValueSpan::new(5000, 9000), // 4000 dead bytes
    ];
    let punches_sub = planner.plan_multi_punch(&spans_sub);
    assert_eq!(
        punches_sub,
        vec![SafeHolePunch {
            aligned_start: 4096,
            aligned_end: 8192,
            punched_bytes: 4096,
        }],
        "Contiguous sub-block dead spans must coalesce to reclaim enclosed block"
    );

    // 6. Invariant: Live record span isolation must detect overlap with punch
    let punch_test = vec![SafeHolePunch {
        aligned_start: 4096,
        aligned_end: 8192,
        punched_bytes: 4096,
    }];
    // Live record from 4000 to 4200 crosses into 4096..8192 punch -> MUST NOT be reported as isolated!
    assert!(
        !planner.verify_live_span_isolated(4000, 200, &punch_test),
        "Live record crossing into hole punch must be detected as destroyed"
    );
}

#[test]
fn test_vlog_hole_alignment_validation_and_overflow_green() {
    // 1. try_new validation for block size
    assert_eq!(
        VLogHolePunchPlanner::try_new(0).err(),
        Some(VLogHoleAlignmentError::InvalidBlockSize(0))
    );
    assert_eq!(
        VLogHolePunchPlanner::try_new(3000).err(),
        Some(VLogHoleAlignmentError::InvalidBlockSize(3000))
    );
    let planner = VLogHolePunchPlanner::try_new(4096).expect("valid block size");

    // 2. DeadValueSpan::try_new validation
    assert_eq!(
        DeadValueSpan::try_new(100, 50).err(),
        Some(VLogHoleAlignmentError::InvalidSpan { start: 100, end: 50 })
    );
    let valid_span = DeadValueSpan::try_new(100, 500).expect("valid span");
    assert_eq!(valid_span.len(), 400);

    // 3. Inverted span len() does not panic under release/debug
    let inverted = DeadValueSpan { start_offset: 500, end_offset: 100 };
    assert_eq!(inverted.len(), 0);

    // 4. Overflow at near u64::MAX boundary:
    // When start_offset cannot be rounded up to a block boundary within u64,
    // it must return None and NEVER produce aligned_start < start_offset!
    let near_max_span = DeadValueSpan::new(u64::MAX - 2000, u64::MAX);
    let punch = planner.plan_span_punch(&near_max_span);
    if let Some(p) = punch {
        assert!(
            p.aligned_start >= near_max_span.start_offset,
            "CRITICAL INVARIANT VIOLATION: aligned_start ({}) < start_offset ({})",
            p.aligned_start, near_max_span.start_offset
        );
    }

    // 5. Total reclaimed bytes helper
    let punches = vec![
        SafeHolePunch { aligned_start: 0, aligned_end: 4096, punched_bytes: 4096 },
        SafeHolePunch { aligned_start: 8192, aligned_end: 16384, punched_bytes: 8192 },
    ];
    assert_eq!(VLogHolePunchPlanner::total_reclaimed_bytes(&punches), 12288);
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
fn test_pinned_slice_lease_eviction_and_cleanup_green() {
    let payload = b"block_data_for_unpin_testing".to_vec();
    let mut lease = PinnedSliceLease::new_pinned(77, payload.clone(), 1000, 20);

    // 1. Initial state inspection
    assert_eq!(lease.block_id, 77);
    assert_eq!(lease.remaining_pin_ticks(1005), 15);
    assert_eq!(lease.get_slice(), Some(payload.as_slice()));
    assert!(!lease.is_released());

    // 2. check_and_decouple_evict returns the block_id to unpin from cache
    assert_eq!(lease.check_and_decouple_evict(1015), None); // 15 <= 20
    assert_eq!(lease.check_and_decouple_evict(1025), Some(77)); // decoupled!
    assert_eq!(lease.check_and_decouple_evict(1030), None); // already decoupled
    assert!(lease.is_detached_private());
    assert_eq!(lease.remaining_pin_ticks(1030), 0);
    assert_eq!(lease.get_slice(), Some(payload.as_slice()));

    // 3. release_and_unpin when already detached: block was already unpinned
    assert_eq!(lease.release_and_unpin(), None);
    assert!(lease.is_released());
    assert_eq!(lease.get_slice(), None);
    assert!(lease.payload.is_empty(), "Payload memory must be reclaimed on release");

    // 4. release_and_unpin while still pinned: MUST return block_id to unpin
    let mut pinned_lease = PinnedSliceLease::new_pinned(88, vec![1, 2, 3], 2000, 100);
    assert_eq!(pinned_lease.release_and_unpin(), Some(88));
    assert!(pinned_lease.is_released());
    assert_eq!(pinned_lease.get_slice(), None);

    // 5. Zero-tick policy: decouples immediately
    let mut immediate_lease = PinnedSliceLease::new_pinned(99, vec![9, 9], 500, 0);
    assert_eq!(immediate_lease.check_and_decouple_evict(500), Some(99));
    assert!(immediate_lease.is_detached_private());
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
fn test_compaction_banach_validation_and_fixed_point_green() {
    // 1. try_new validation for gamma
    assert_eq!(
        BanachCompactionScheduler::try_new(0.0).err(),
        Some(BanachCompactionError::InvalidGamma(0.0))
    );
    assert_eq!(
        BanachCompactionScheduler::try_new(1.0).err(),
        Some(BanachCompactionError::InvalidGamma(1.0))
    );
    assert_eq!(
        BanachCompactionScheduler::try_new(-0.5).err(),
        Some(BanachCompactionError::InvalidGamma(-0.5))
    );
    assert!(BanachCompactionScheduler::try_new(f64::NAN).is_err());
    let scheduler = BanachCompactionScheduler::try_new(0.5).expect("valid gamma");

    // 2. CompactionDebtVector validation against NaN and negative debts
    assert_eq!(
        CompactionDebtVector::try_new(vec![1.0, -0.5]).err(),
        Some(BanachCompactionError::InvalidDebt(-0.5))
    );
    assert!(CompactionDebtVector::try_new(vec![1.0, f64::NAN]).is_err());
    let valid_vector = CompactionDebtVector::try_new(vec![10.0, 5.0, 2.0]).expect("valid debts");
    assert_eq!(valid_vector.norm_inf(), 10.0);

    // 3. Safe steps_to_convergence validation
    assert_eq!(
        scheduler.try_steps_to_convergence(16.0, 0.0).err(),
        Some(BanachCompactionError::InvalidEpsilon(0.0))
    );
    assert_eq!(
        scheduler.try_steps_to_convergence(16.0, -1.0).err(),
        Some(BanachCompactionError::InvalidEpsilon(-1.0))
    );
    assert_eq!(
        scheduler.try_steps_to_convergence(-5.0, 0.1).err(),
        Some(BanachCompactionError::InvalidDebt(-5.0))
    );
    // d0 = 16.0, eps = 0.015625 (which is 16 * 0.5^10) -> exactly 10 steps
    let steps = scheduler.try_steps_to_convergence(16.0, 0.015625).expect("valid steps");
    assert_eq!(steps, 10);

    // 4. Exact theoretical fixed point computation D* = W / (1 - gamma)
    let write_load = vec![1.5, 3.0, 0.5];
    let fixed_point = scheduler.compute_fixed_point(&write_load).expect("fixed point");
    // With gamma = 0.5, 1 / (1 - 0.5) = 2.0 -> D* = 2 * write_load
    assert_eq!(fixed_point.level_debts, vec![3.0, 6.0, 1.0]);

    // Verify fixed-point idempotency: T(D*) == D*
    let transitioned = scheduler.transition_step(&fixed_point, &write_load);
    assert!(fixed_point.distance_inf(&transitioned) < 1e-12, "T(D*) must be identical to D*");
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

#[test]
fn test_crypto_nonce_epoch_overflow_and_parse_green() {
    let valid_uuid = [7u8; 16];
    let nil_uuid = [0u8; 16];

    // 1. try_new rejects Nil UUID
    assert_eq!(
        CryptoSpaceTimeCoords::try_new(nil_uuid, 1, 1, 0).err(),
        Some(CryptoNonceError::NilSuperblockUuid)
    );
    let coords = CryptoSpaceTimeCoords::try_new(valid_uuid, 100, 200, 4096).expect("valid coords");

    // 2. Canonical nonce roundtrip via parse_canonical_nonce
    let encoded = CryptoNonceGenerator::generate_canonical_nonce(&coords);
    let decoded = CryptoNonceGenerator::parse_canonical_nonce(&encoded).expect("valid roundtrip");
    assert_eq!(decoded, coords);

    // 3. Slice parsing with wrong length
    assert_eq!(
        CryptoNonceGenerator::parse_canonical_nonce_slice(&[1, 2, 3]).err(),
        Some(CryptoNonceError::InvalidCanonicalNonceLength(3))
    );

    // 4. CRITICAL: High 32-bit epoch barrier collision eradication in AEAD nonce!
    // If epoch barrier exceeds 2^32, (epoch as u32) without high-bit diffusion collides with epoch % 2^32!
    let coords_low = CryptoSpaceTimeCoords::new(valid_uuid, 42, 100, 4096);
    let coords_high = CryptoSpaceTimeCoords::new(valid_uuid, (1u64 << 32) | 42, 100, 4096);
    let aead_low = CryptoNonceGenerator::derive_aead_nonce_96(&coords_low);
    let aead_high = CryptoNonceGenerator::derive_aead_nonce_96(&coords_high);

    assert_ne!(
        aead_low, aead_high,
        "CRITICAL NONCE REUSE: AEAD nonce collided across epoch barrier overflow (42 vs (1<<32)|42)!"
    );
}

