//! RFC-0314: Leveled Compaction Overlap Minimization Matrix Test Suite.
//!
//! Validates mathematical properties:
//! - Exact key range interval intersection logic.
//! - Strictly disjoint level ordering validation.
//! - Accurate bipartite overlap matrix cost scoring.
//! - Expansion-ratio ceiling bounded candidate selection.

use pedradb_core::compaction_overlap_matrix_kernel::{
    CompactionOverlapError, CompactionOverlapMatrix, SstInterval,
};

#[test]
fn test_sst_interval_overlap_logic() {
    // 1. Validation error if smallest > largest
    assert!(SstInterval::new(1, b"z".to_vec(), b"a".to_vec(), 100).is_err());

    // 2. Disjoint intervals: [10, 20] and [30, 40]
    let int1 = SstInterval::new(1, b"10".to_vec(), b"20".to_vec(), 100).unwrap();
    let int2 = SstInterval::new(2, b"30".to_vec(), b"40".to_vec(), 100).unwrap();
    assert!(!int1.overlaps_with(&int2));
    assert!(!int2.overlaps_with(&int1));

    // 3. Overlapping intervals: [10, 25] and [20, 35]
    let int3 = SstInterval::new(3, b"20".to_vec(), b"35".to_vec(), 100).unwrap();
    assert!(int1.overlaps_with(&int3));
    assert!(int3.overlaps_with(&int1));

    // 4. Abutting boundary overlap: [10, 20] and [20, 30]
    let int4 = SstInterval::new(4, b"20".to_vec(), b"30".to_vec(), 100).unwrap();
    assert!(int1.overlaps_with(&int4));
    assert!(int4.overlaps_with(&int1));
}

#[test]
fn test_level_disjointness_verification() {
    // Disjoint level: [10, 19], [20, 29], [30, 39]
    let valid_level = vec![
        SstInterval::new(1, b"10".to_vec(), b"19".to_vec(), 100).unwrap(),
        SstInterval::new(2, b"20".to_vec(), b"29".to_vec(), 100).unwrap(),
        SstInterval::new(3, b"30".to_vec(), b"39".to_vec(), 100).unwrap(),
    ];
    assert!(CompactionOverlapMatrix::is_disjoint_level(&valid_level));

    // Overlapping level: [10, 25], [20, 29] -> INVALID
    let invalid_overlap = vec![
        SstInterval::new(1, b"10".to_vec(), b"25".to_vec(), 100).unwrap(),
        SstInterval::new(2, b"20".to_vec(), b"29".to_vec(), 100).unwrap(),
    ];
    assert!(!CompactionOverlapMatrix::is_disjoint_level(&invalid_overlap));

    // Unsorted level: [30, 39], [10, 19] -> INVALID
    let invalid_order = vec![
        SstInterval::new(1, b"30".to_vec(), b"39".to_vec(), 100).unwrap(),
        SstInterval::new(2, b"10".to_vec(), b"19".to_vec(), 100).unwrap(),
    ];
    assert!(!CompactionOverlapMatrix::is_disjoint_level(&invalid_order));
}

#[test]
fn test_overlap_candidate_analysis_and_cost() {
    let matrix = CompactionOverlapMatrix::new(1, 2, 2.5);
    assert_eq!(matrix.level_source(), 1);
    assert_eq!(matrix.level_target(), 2);
    assert_eq!(matrix.max_expansion_ratio(), 2.5);

    // Source Level 1:
    // f1: [10, 20], size 100
    // f2: [50, 60], size 100
    let source_files = vec![
        SstInterval::new(101, b"10".to_vec(), b"20".to_vec(), 100).unwrap(),
        SstInterval::new(102, b"50".to_vec(), b"60".to_vec(), 100).unwrap(),
    ];

    // Target Level 2:
    // g1: [05, 15], size 150 (overlaps with f1)
    // g2: [18, 25], size 150 (overlaps with f1)
    // g3: [55, 65], size 100 (overlaps with f2)
    let target_files = vec![
        SstInterval::new(201, b"05".to_vec(), b"15".to_vec(), 150).unwrap(),
        SstInterval::new(202, b"18".to_vec(), b"25".to_vec(), 150).unwrap(),
        SstInterval::new(203, b"55".to_vec(), b"65".to_vec(), 100).unwrap(),
    ];

    let candidates = matrix.analyze_candidates(&source_files, &target_files);
    assert_eq!(candidates.len(), 2);

    // Candidate 1 (f1):
    // Overlaps with g1 (150) and g2 (150) = 300 target bytes
    // Total bytes = 100 + 300 = 400
    // Expansion ratio = 300 / 100 = 3.0
    assert_eq!(candidates[0].source_file_id, 101);
    assert_eq!(candidates[0].overlapping_target_file_ids, vec![201, 202]);
    assert_eq!(candidates[0].overlapping_target_bytes, 300);
    assert_eq!(candidates[0].total_compaction_bytes, 400);
    assert!((candidates[0].expansion_ratio - 3.0).abs() < 1e-6);

    // Candidate 2 (f2):
    // Overlaps with g3 (100) = 100 target bytes
    // Total bytes = 100 + 100 = 200
    // Expansion ratio = 100 / 100 = 1.0
    assert_eq!(candidates[1].source_file_id, 102);
    assert_eq!(candidates[1].overlapping_target_file_ids, vec![203]);
    assert_eq!(candidates[1].overlapping_target_bytes, 100);
    assert_eq!(candidates[1].total_compaction_bytes, 200);
    assert!((candidates[1].expansion_ratio - 1.0).abs() < 1e-6);
}

#[test]
fn test_optimal_candidate_selection_within_ceiling() {
    let matrix = CompactionOverlapMatrix::new(1, 2, 2.0);

    let source_files = vec![
        SstInterval::new(101, b"10".to_vec(), b"20".to_vec(), 100).unwrap(), // Ratio 3.0 (> 2.0)
        SstInterval::new(102, b"50".to_vec(), b"60".to_vec(), 100).unwrap(), // Ratio 1.0 (<= 2.0)
    ];

    let target_files = vec![
        SstInterval::new(201, b"05".to_vec(), b"15".to_vec(), 150).unwrap(),
        SstInterval::new(202, b"18".to_vec(), b"25".to_vec(), 150).unwrap(),
        SstInterval::new(203, b"55".to_vec(), b"65".to_vec(), 100).unwrap(),
    ];

    // Must pick file 102 because its expansion ratio (1.0) satisfies ceiling (2.0)
    let best = matrix.select_optimal_candidate(&source_files, &target_files).unwrap();
    assert_eq!(best.source_file_id, 102);
    assert_eq!(best.total_compaction_bytes, 200);

    // If max_expansion_ratio is tightened to 0.5 (where neither qualifies):
    // Fallback picks the one with minimum expansion ratio (file 102 with ratio 1.0 vs file 101 with 3.0)
    let strict_matrix = CompactionOverlapMatrix::new(1, 2, 0.5); // clamps to 1.0
    let fallback_best = strict_matrix.select_optimal_candidate(&source_files, &target_files).unwrap();
    assert_eq!(fallback_best.source_file_id, 102);
}

#[test]
fn test_compaction_overlap_matrix_validation_and_incidence_green() {
    // 1. CompactionOverlapMatrix::try_new validates expansion ratio
    assert_eq!(
        CompactionOverlapMatrix::try_new(1, 2, 0.5).err(),
        Some(CompactionOverlapError::InvalidExpansionRatio)
    );
    assert_eq!(
        CompactionOverlapMatrix::try_new(1, 2, f64::NAN).err(),
        Some(CompactionOverlapError::InvalidExpansionRatio)
    );
    let matrix = CompactionOverlapMatrix::try_new(1, 2, 2.5).expect("valid matrix");

    // 2. SstInterval::try_new typed errors
    assert_eq!(
        SstInterval::try_new(1, vec![], b"10".to_vec(), 100).err(),
        Some(CompactionOverlapError::EmptyKey)
    );
    assert_eq!(
        SstInterval::try_new(1, b"10".to_vec(), vec![], 100).err(),
        Some(CompactionOverlapError::EmptyKey)
    );
    assert_eq!(
        SstInterval::try_new(1, b"20".to_vec(), b"10".to_vec(), 100).err(),
        Some(CompactionOverlapError::InvertedKeyRange {
            smallest: b"20".to_vec(),
            largest: b"10".to_vec(),
        })
    );
    assert_eq!(
        SstInterval::try_new(1, b"10".to_vec(), b"20".to_vec(), 0).err(),
        Some(CompactionOverlapError::ZeroFileSizeBytes)
    );

    // 3. Mathematical Bipartite Overlap Incidence Matrix A_{i, j} (RFC-0314 Definition 2.2)
    let source_files = vec![
        SstInterval::new(101, b"10".to_vec(), b"20".to_vec(), 100).unwrap(),
        SstInterval::new(102, b"50".to_vec(), b"60".to_vec(), 100).unwrap(),
    ];
    let target_files = vec![
        SstInterval::new(201, b"05".to_vec(), b"15".to_vec(), 150).unwrap(),
        SstInterval::new(202, b"18".to_vec(), b"25".to_vec(), 150).unwrap(),
        SstInterval::new(203, b"55".to_vec(), b"65".to_vec(), 100).unwrap(),
    ];

    let incidence = matrix.build_incidence_matrix(&source_files, &target_files);
    // Row 0 (source 101): overlaps with target 201, 202, not 203 -> [true, true, false]
    // Row 1 (source 102): overlaps with target 203, not 201, 202 -> [false, false, true]
    assert_eq!(incidence.len(), 2);
    assert_eq!(incidence[0], vec![true, true, false]);
    assert_eq!(incidence[1], vec![false, false, true]);

    // 4. Complete Level Invariants check
    assert!(CompactionOverlapMatrix::check_level_invariants(&source_files).is_ok());
    let mut invalid_files = source_files.clone();
    invalid_files.push(SstInterval::new(103, b"55".to_vec(), b"70".to_vec(), 100).unwrap()); // overlaps 102!
    assert!(CompactionOverlapMatrix::check_level_invariants(&invalid_files).is_err());
}

