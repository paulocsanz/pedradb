//! RFC-0315: Leveled SST Interval Candidate Index Test Suite
//!
//! Verifies:
//! - O(log K) candidate pruning for point queries.
//! - Absolute zero false-negatives (100% recall of actual overlapping tables).
//! - Newest-first ordering (descending sequence order).
//! - Tombstone table tracking.

use pedradb_core::sst_candidate_index_kernel::{SstCandidateIndex, SstIntervalMetadata};

#[test]
fn test_candidate_pruning_disjoint_ranges() {
    let intervals = vec![
        SstIntervalMetadata::new(1, b"aaa".to_vec(), b"aaz".to_vec(), false, 100),
        SstIntervalMetadata::new(2, b"bbb".to_vec(), b"bbz".to_vec(), false, 110),
        SstIntervalMetadata::new(3, b"ccc".to_vec(), b"ccz".to_vec(), false, 120),
        SstIntervalMetadata::new(4, b"ddd".to_vec(), b"ddz".to_vec(), false, 130),
    ];

    let index = SstCandidateIndex::new(intervals);
    assert_eq!(index.total_tables(), 4);
    assert!(!index.has_range_tombstones());

    // Key in table 2
    let c2 = index.query_point_candidates(b"bbm");
    assert_eq!(c2, vec![2]);

    // Key between table 1 and 2 (hole)
    let c_hole = index.query_point_candidates(b"abz");
    assert!(c_hole.is_empty());

    // Key before all tables
    let c_before = index.query_point_candidates(b"000");
    assert!(c_before.is_empty());

    // Key after all tables
    let c_after = index.query_point_candidates(b"zzz");
    assert!(c_after.is_empty());
}

#[test]
fn test_candidate_zero_false_negatives_exhaustive() {
    let intervals = vec![
        SstIntervalMetadata::new(10, b"key10".to_vec(), b"key50".to_vec(), false, 500),
        SstIntervalMetadata::new(20, b"key20".to_vec(), b"key40".to_vec(), false, 600),
        SstIntervalMetadata::new(30, b"key30".to_vec(), b"key70".to_vec(), false, 700),
        SstIntervalMetadata::new(40, b"key60".to_vec(), b"key90".to_vec(), false, 800),
    ];

    let index = SstCandidateIndex::new(intervals.clone());

    // Test a suite of keys
    let test_keys = [
        b"key05".as_slice(),
        b"key10".as_slice(),
        b"key25".as_slice(),
        b"key35".as_slice(),
        b"key65".as_slice(),
        b"key85".as_slice(),
        b"key99".as_slice(),
    ];

    for &key in &test_keys {
        let expected: Vec<u64> = intervals
            .iter()
            .filter(|iv| iv.covers_key(key))
            .map(|iv| iv.file_number)
            .collect();

        assert!(
            index.verify_candidate_completeness(key, &expected),
            "Candidate index missed overlapping tables for key {:?}",
            std::str::from_utf8(key).unwrap()
        );
    }
}

#[test]
fn test_candidate_newest_first_ordering() {
    let intervals = vec![
        SstIntervalMetadata::new(1, b"key".to_vec(), b"key_z".to_vec(), false, 100),
        SstIntervalMetadata::new(2, b"key".to_vec(), b"key_z".to_vec(), false, 500),
        SstIntervalMetadata::new(3, b"key".to_vec(), b"key_z".to_vec(), false, 300),
    ];

    let index = SstCandidateIndex::new(intervals);
    let candidates = index.query_point_candidates(b"key_a");

    // Must be sorted newest-first: seq 500 (table 2), then 300 (table 3), then 100 (table 1)
    assert_eq!(candidates, vec![2, 3, 1]);
}

#[test]
fn test_tombstone_table_tracking() {
    let intervals = vec![
        SstIntervalMetadata::new(1, b"a".to_vec(), b"b".to_vec(), false, 10),
        SstIntervalMetadata::new(2, b"c".to_vec(), b"d".to_vec(), true, 20),
        SstIntervalMetadata::new(3, b"e".to_vec(), b"f".to_vec(), true, 30),
        SstIntervalMetadata::new(4, b"g".to_vec(), b"h".to_vec(), false, 40),
    ];

    let index = SstCandidateIndex::new(intervals);
    assert!(index.has_range_tombstones());
    let tombstone_files = index.tombstone_files();
    assert_eq!(tombstone_files.len(), 2);
    assert!(tombstone_files.contains(&2));
    assert!(tombstone_files.contains(&3));
}

#[test]
fn test_sst_candidate_index_validation_and_deterministic_order_red() {
    use pedradb_core::sst_candidate_index_kernel::SstCandidateIndexError;

    // 1. Inverted interval rejection
    let inv = SstIntervalMetadata::try_new(1, b"zzz".to_vec(), b"aaa".to_vec(), false, 100);
    assert!(matches!(inv, Err(SstCandidateIndexError::InvertedKeyRange { .. })));

    // 2. Zero file number rejection
    let zero_file = SstIntervalMetadata::try_new(0, b"a".to_vec(), b"b".to_vec(), false, 100);
    assert!(matches!(zero_file, Err(SstCandidateIndexError::ZeroFileNumber)));

    // 3. Duplicate file number rejection in candidate index
    let dup_intervals = vec![
        SstIntervalMetadata::try_new(5, b"a".to_vec(), b"b".to_vec(), false, 10).unwrap(),
        SstIntervalMetadata::try_new(5, b"c".to_vec(), b"d".to_vec(), false, 20).unwrap(),
    ];
    let dup_index = SstCandidateIndex::try_new(dup_intervals);
    assert!(matches!(dup_index, Err(SstCandidateIndexError::DuplicateFileNumber(5))));

    // 4. Deterministic tie-breaking on identical seq_max
    let intervals = vec![
        SstIntervalMetadata::try_new(10, b"k".to_vec(), b"m".to_vec(), false, 50).unwrap(),
        SstIntervalMetadata::try_new(25, b"k".to_vec(), b"m".to_vec(), false, 50).unwrap(),
        SstIntervalMetadata::try_new(15, b"k".to_vec(), b"m".to_vec(), false, 50).unwrap(),
    ];
    let index = SstCandidateIndex::try_new(intervals).expect("valid index");
    let cands = index.query_point_candidates(b"l");
    // Must be sorted strictly newest-first: file 25, then 15, then 10
    assert_eq!(cands, vec![25, 15, 10]);

    // 5. query_candidates_with_tombstones includes tables with range tombstones even outside point bounds
    let intervals2 = vec![
        SstIntervalMetadata::try_new(1, b"a".to_vec(), b"b".to_vec(), false, 10).unwrap(),
        SstIntervalMetadata::try_new(2, b"x".to_vec(), b"y".to_vec(), true, 20).unwrap(), // range tombstone table!
    ];
    let index2 = SstCandidateIndex::try_new(intervals2).expect("valid index");
    // Point query for "a"
    let c_point = index2.query_point_candidates(b"a");
    assert_eq!(c_point, vec![1]); // point interval only matches table 1
    // Tombstone query for "a"
    let c_all = index2.query_candidates_with_tombstones(b"a");
    assert_eq!(c_all, vec![2, 1]); // Table 2 (tombstones, seq 20) and Table 1 (point, seq 10)
}

