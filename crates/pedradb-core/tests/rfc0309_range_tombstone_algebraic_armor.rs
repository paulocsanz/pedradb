//! RFC-0309: Range Tombstone Algebraic Surface & Property-Based Armor.
//!
//! Enforces:
//! 1. Canonical decomposition: overlapping range deletions partition into pairwise disjoint intervals.
//! 2. Exact point coverage: `IsDeleted(k, t) <=> exists [s, e) @ t_tomb: s <= k < e and t_tomb > t`.
//! 3. Strict half-open boundary consistency: `k == start` is shadowed; `k == end` is strictly non-inclusive.
//! 4. MVCC snapshot horizon visibility: tombstones with `seq > snapshot_seq` are invisible; tombstones with `seq <= snapshot_seq` shadow older versions.
//! 5. Metamorphic equivalence: point coverage is 100% identical between unfragmented and fragmented sets.

use pedradb_core::range_tombstone_fragmentation_kernel::{RangeTombstone, RangeTombstoneSet};

#[test]
fn test_exact_point_coverage_invariant_preservation_under_fragmentation() {
    let mut set = RangeTombstoneSet::new();

    // 1. Nested ranges
    set.add(RangeTombstone::new(b"a".to_vec(), b"z".to_vec(), 10).unwrap());
    set.add(RangeTombstone::new(b"c".to_vec(), b"x".to_vec(), 20).unwrap());
    set.add(RangeTombstone::new(b"e".to_vec(), b"v".to_vec(), 30).unwrap());
    set.add(RangeTombstone::new(b"g".to_vec(), b"t".to_vec(), 40).unwrap());

    // 2. Staggered overlapping ranges
    set.add(RangeTombstone::new(b"b".to_vec(), b"h".to_vec(), 50).unwrap());
    set.add(RangeTombstone::new(b"m".to_vec(), b"p".to_vec(), 70).unwrap());

    // 3. Identical ranges with different sequences
    set.add(RangeTombstone::new(b"d".to_vec(), b"j".to_vec(), 15).unwrap());
    set.add(RangeTombstone::new(b"d".to_vec(), b"j".to_vec(), 85).unwrap());

    // 4. Disjoint / abutting ranges
    set.add(RangeTombstone::new(b"w".to_vec(), b"y".to_vec(), 95).unwrap());

    let fragmented = set.fragment();
    let frag_set = RangeTombstoneSet::new();
    let mut frag_set = frag_set;
    for tomb in fragmented.iter().cloned() {
        frag_set.add(tomb);
    }

    // Verify point coverage equivalence for every single character from 'a' to 'z'
    // and across multiple point sequence numbers
    for b in b'a'..=b'z' {
        let key = vec![b];
        for point_seq in [0, 5, 10, 15, 20, 30, 40, 50, 70, 85, 95, 100] {
            let orig_shadowed = set.is_key_shadowed(&key, point_seq);
            let frag_shadowed = frag_set.is_key_shadowed(&key, point_seq);
            assert_eq!(
                orig_shadowed, frag_shadowed,
                "Point coverage mismatch for key {:?} at point_seq {}",
                std::str::from_utf8(&key),
                point_seq
            );

            let orig_max = set.max_covering_seq(&key);
            let frag_max = frag_set.max_covering_seq(&key);
            assert_eq!(
                orig_max, frag_max,
                "Max sequence mismatch for key {:?}",
                std::str::from_utf8(&key)
            );
        }
    }
}

#[test]
fn test_half_open_interval_boundary_non_inclusivity() {
    let mut set = RangeTombstoneSet::new();
    set.add(RangeTombstone::new(b"key_100".to_vec(), b"key_200".to_vec(), 50).unwrap());

    // Start boundary is inclusive: key_100 is shadowed
    assert!(set.is_key_shadowed(b"key_100", 49));
    assert_eq!(set.max_covering_seq(b"key_100"), Some(50));

    // Interior keys are shadowed
    assert!(set.is_key_shadowed(b"key_150", 49));
    assert!(set.is_key_shadowed(b"key_199", 49));

    // End boundary is strictly exclusive: key_200 is NOT shadowed
    assert!(!set.is_key_shadowed(b"key_200", 49));
    assert_eq!(set.max_covering_seq(b"key_200"), None);

    // Keys outside the interval are NOT shadowed
    assert!(!set.is_key_shadowed(b"key_099", 49));
    assert!(!set.is_key_shadowed(b"key_201", 49));

    // Point write at the exact tombstone sequence or later is NOT shadowed
    assert!(!set.is_key_shadowed(b"key_150", 50));
    assert!(!set.is_key_shadowed(b"key_150", 51));
}

#[test]
fn test_mvcc_snapshot_horizon_visibility_and_anti_retroactivity() {
    let mut set = RangeTombstoneSet::new();
    set.add(RangeTombstone::new(b"user_1".to_vec(), b"user_9".to_vec(), 100).unwrap());

    let point_key = b"user_5";

    // 1. Point written before tombstone (seq = 50)
    let point_seq = 50;

    // Snapshot at seq = 80: tombstone (100) is in future, so point is NOT shadowed
    assert!(!set.is_key_shadowed_at_snapshot(point_key, point_seq, 80));

    // Snapshot at seq = 99: tombstone is still in future
    assert!(!set.is_key_shadowed_at_snapshot(point_key, point_seq, 99));

    // Snapshot at seq = 100: tombstone is visible, point (50) is older than tombstone (100) -> shadowed!
    assert!(set.is_key_shadowed_at_snapshot(point_key, point_seq, 100));

    // Snapshot at seq = 150: tombstone is visible -> shadowed!
    assert!(set.is_key_shadowed_at_snapshot(point_key, point_seq, 150));

    // 2. Point written AFTER tombstone (seq = 120)
    let point_seq_after = 120;

    // Snapshot at seq = 130: tombstone (100) is older than point (120) -> NOT shadowed!
    assert!(!set.is_key_shadowed_at_snapshot(point_key, point_seq_after, 130));

    // Snapshot at seq = 110: point (120) not even visible to snapshot
    assert!(!set.is_key_shadowed_at_snapshot(point_key, point_seq_after, 110));
}

#[test]
fn test_canonical_fragmentation_pairwise_disjointness_and_coalescence() {
    let mut set = RangeTombstoneSet::new();
    // Overlapping and abutting intervals
    set.add(RangeTombstone::new(b"a".to_vec(), b"d".to_vec(), 100).unwrap());
    set.add(RangeTombstone::new(b"d".to_vec(), b"g".to_vec(), 100).unwrap()); // contiguous with same seq
    set.add(RangeTombstone::new(b"b".to_vec(), b"e".to_vec(), 200).unwrap()); // overlaps higher seq
    set.add(RangeTombstone::new(b"h".to_vec(), b"m".to_vec(), 300).unwrap()); // disjoint
    set.add(RangeTombstone::new(b"j".to_vec(), b"k".to_vec(), 400).unwrap()); // nested inside [h, m)

    let fragmented = set.fragment();
    assert!(!fragmented.is_empty());

    // 1. Strict positive length: start < end
    for tomb in &fragmented {
        assert!(tomb.start < tomb.end, "Interval must have start < end");
    }

    // 2. Pairwise disjointness: fragmented[i].end <= fragmented[i + 1].start
    for window in fragmented.windows(2) {
        assert!(
            window[0].end <= window[1].start,
            "Fragments must be pairwise disjoint: {:?} vs {:?}",
            window[0],
            window[1]
        );

        // 3. Coalescence: contiguous intervals must NOT have identical sequence number
        if window[0].end == window[1].start {
            assert_ne!(
                window[0].seq_num, window[1].seq_num,
                "Contiguous intervals sharing same seq must be coalesced!"
            );
        }
    }
}

#[test]
fn test_chaos_property_based_interval_metamorphic_oracle() {
    // Deterministic pseudo-random generator
    let mut state: u64 = 0xCAFE_BABE_DEAD_BEEF;
    let mut next_u64 = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        state
    };

    for _round in 0..100 {
        let mut set = RangeTombstoneSet::new();
        let num_tombstones = (next_u64() % 15) as usize + 2;

        for _ in 0..num_tombstones {
            let s_byte = b'a' + (next_u64() % 24) as u8;
            let len = (next_u64() % 5) as u8 + 1;
            let e_byte = (s_byte + len).min(b'z');

            if s_byte < e_byte {
                let seq = (next_u64() % 100) + 1;
                set.add(RangeTombstone::new(vec![s_byte], vec![e_byte], seq).unwrap());
            }
        }

        let fragmented = set.fragment();
        let mut frag_set = RangeTombstoneSet::new();
        for t in fragmented.iter().cloned() {
            frag_set.add(t);
        }

        // Verify pairwise disjointness
        for window in fragmented.windows(2) {
            assert!(window[0].end <= window[1].start);
            if window[0].end == window[1].start {
                assert_ne!(window[0].seq_num, window[1].seq_num);
            }
        }

        // Verify point coverage across entire key domain 'a'..='z'
        for b in b'a'..=b'z' {
            let key = vec![b];
            for probe_seq in [0, 10, 25, 50, 75, 100] {
                assert_eq!(
                    set.is_key_shadowed(&key, probe_seq),
                    frag_set.is_key_shadowed(&key, probe_seq),
                    "Metamorphic mismatch at key {:?} seq {}",
                    std::str::from_utf8(&key),
                    probe_seq
                );
            }
            assert_eq!(set.max_covering_seq(&key), frag_set.max_covering_seq(&key));
        }
    }
}
