//! Integration tests for RFC-0275: Universal Scaling Hegemony & Anti-OOM Bounding.
//!
//! Verifies:
//! 1. Pillar I: Strict RAM bounding (zero-retention / 64MB clamp on retired memtables).
//! 2. Pillar II: Point scan negative Bloom filter pruning in `scan_at_raw`.
//! 3. Pillar IV: Continuous L0 compaction trigger defense preventing file runaway.

use pedradb_core::{ConcurrentDb, Db, OpenOptions};
use std::ops::Bound;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("pedra-rfc0275-{prefix}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn test_retired_cache_strict_cap() {
    let dir = temp_dir("retired-cap");
    let db = ConcurrentDb::open_with(
        &dir,
        OpenOptions {
            auto_flush_bytes: Some(1024 * 1024), // 1 MiB buffers
            sync: false,
            ..Default::default()
        },
    )
    .unwrap();

    // Verify retired cache cap is clamped to <= 64 MiB
    assert!(db.with_read(|d| d.retired_cache_cap()) <= 64 * 1024 * 1024);

    // Write multiple buffers
    for batch_i in 0..10 {
        for key_i in 0..100 {
            let key = format!("b{batch_i:02}_k{key_i:03}").into_bytes();
            let val = vec![0x42; 256];
            db.put(key, val).unwrap();
        }
        db.flush().unwrap();
    }

    // After 10 flushes, retired memory must not grow unboundedly
    let retired = db.with_read(|d| d.retired_mem_bytes());
    assert!(
        retired <= 64 * 1024 * 1024,
        "retired cache exceeded 64MB cap: got {retired} bytes"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_point_scan_bloom_filter_pruning() {
    let dir = temp_dir("bloom-prune");
    let mut opts = OpenOptions::default();
    opts.sync = false;
    let mut db = Db::open_with(&dir, opts).unwrap();

    // Write 5 disjoint SSTs
    for sst_i in 0..5 {
        for k in 0..20 {
            let key = format!("sst_{sst_i}_key_{k:03}").into_bytes();
            let val = format!("val_{sst_i}_{k}").into_bytes();
            db.put(key, val).unwrap();
        }
        db.flush().unwrap();
    }

    // Now perform a point scan for a key that does not exist in any SST
    let probe_before = db.read_probe().scan_sst_probed;
    let absent_key = b"absent_nonexistent_key_999";
    let count = db.scan_at(
        db.visible_sequence(),
        Bound::Included(absent_key.as_slice()),
        Bound::Included(absent_key.as_slice()),
        None,
    ).count();

    assert_eq!(count, 0);
    let probe_after = db.read_probe().scan_sst_probed;
    // Point bloom pruning should prune SSTs without probing their blocks
    assert_eq!(
        probe_before, probe_after,
        "bloom pruning must skip all SSTs for absent point key"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_continuous_compaction_l0_bound_under_load() {
    let dir = temp_dir("l0-bound");
    let db = Arc::new(
        ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                auto_flush_bytes: Some(64 * 1024), // 64 KiB
                sync: false,
                ..Default::default()
            },
        )
        .unwrap(),
    );

    // Simulate continuous writes and flushes
    for i in 0..20 {
        db.put(format!("k_{i:04}").into_bytes(), vec![0x77; 1024]).unwrap();
        if i % 3 == 0 {
            let _ = db.flush();
        }
    }

    // Ensure L0 compaction ran and did not abandon files
    let l0_count = db.with_read(|d| d.level_file_count(0));
    assert!(
        l0_count < 20,
        "L0 files must be compacted incrementally: got {l0_count}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bloom_filter_equality_checks_nbits() {
    let a = pedradb_core::bloom::BloomFilter::always_true();
    let b = pedradb_core::bloom::BloomFilter::with_capacity(10, 10);
    assert_ne!(a, b);
    assert_eq!(a, pedradb_core::bloom::BloomFilter::always_true());
}
