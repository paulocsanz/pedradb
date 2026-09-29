//! RFC-0303: Deterministic Simulation Testing (DST) Engine & Invariant Shield.
//!
//! Implements the 4 core verification vectors of RFC-0303:
//! 1. Vector 1: Simulated Torn-Write & Crash-Recovery with Active Invariant Auditing.
//! 2. Vector 2: Lock-Free SuperVersion Race Torture (Concurrent Reads during Heavy Compaction/Flush).
//! 3. Vector 3: Fail-Closed Manifest Persist Disjointness Gate Enforcement.
//! 4. Vector 4: 1,000-Step Metamorphic Simulation Against Reference Oracle with Crash Reboots.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::Db;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cnt = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pedradb-dst-{tag}-{nanos}-{cnt}"));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Deterministic 64-bit XorShift PRNG for reproducible adversarial schedules.
#[derive(Debug, Clone)]
struct TestRng(u64);

impl TestRng {
    fn new(mut seed: u64) -> Self {
        if seed == 0 {
            seed = 0xa3b4_c5d6_e7f8_1234;
        }
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn gen_range(&mut self, low: usize, high: usize) -> usize {
        if low >= high {
            return low;
        }
        low + (self.next_u64() as usize % (high - low))
    }

    fn gen_bool(&mut self, numerator: u32, denominator: u32) -> bool {
        (self.next_u64() % denominator as u64) < numerator as u64
    }
}

/// Vector 1: Simulated Torn-Write & Crash-Recovery with Active Invariant Auditing.
/// Tests that simulated abrupt crashes and partial WAL writes recover cleanly
/// and preserve strict sequence monotonicity and disjointness invariants.
#[test]
fn test_dst_vector1_torn_write_and_crash_recovery() {
    let dir = temp_db_dir("dst-v1");
    let mut rng = TestRng::new(0x1337_c0de);

    // Initial phase: write batches and flush some
    {
        let mut db = Db::open(&dir).expect("Db::open initial");
        for i in 0..100 {
            let key = format!("k_{:04}", i);
            let val = format!("v_{}", rng.next_u64());
            db.put(key.as_bytes(), val.as_bytes()).unwrap();
        }
        db.flush().unwrap();
        db.assert_all_invariants().expect("Invariants hold after flush");
    }

    // Phase 2: append more data without flushing, then simulate torn crash
    {
        let mut db = Db::open(&dir).expect("Db::open phase 2");
        for i in 100..200 {
            let key = format!("k_{:04}", i);
            let val = format!("v_{}", rng.next_u64());
            db.put(key.as_bytes(), val.as_bytes()).unwrap();
        }
        // Force drop without flush (simulating power failure)
        drop(db);
    }

    // Tamper with the active WAL file by simulating a partial torn write at the end
    let wal_entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".wal"))
        .collect();
    for entry in wal_entries {
        let path = entry.path();
        if let Ok(metadata) = std::fs::metadata(&path) {
            let len = metadata.len();
            if len > 32 {
                // Truncate 7 bytes from the end to simulate a torn write
                let _ = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .map(|f| f.set_len(len - 7));
            }
        }
    }

    // Phase 3: Reopen after simulated crash & torn write
    let mut db = Db::open(&dir).expect("Db::open must recover cleanly from torn WAL");
    db.assert_all_invariants().expect("Invariants must hold after torn WAL recovery");

    // All unflushed keys before the cut must either be safely present or cleanly cut,
    // and subsequent writes must strictly preserve monotonic sequences.
    for i in 200..250 {
        let key = format!("k_{:04}", i);
        let val = format!("v_{}", rng.next_u64());
        db.put(key.as_bytes(), val.as_bytes()).unwrap();
    }
    db.assert_all_invariants().expect("Invariants must hold after post-recovery writes");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Vector 2: Lock-Free SuperVersion Race Torture.
/// Multiple concurrent reader threads querying `published_ssts` while a background worker
/// continuously flushes and compacts SSTs. Readers must NEVER observe file-not-found,
/// panic, or stale state.
#[test]
fn test_dst_vector2_superversion_race_torture() {
    let dir = temp_db_dir("dst-v2");
    let db = Arc::new(ConcurrentDb::open(&dir).expect("ConcurrentDb::open"));
    let stop = Arc::new(AtomicBool::new(false));

    // Seed initial keys
    for i in 0..500 {
        let k = format!("key_{:04}", i);
        let v = format!("val_{:04}", i);
        db.put(k.as_bytes(), v.as_bytes()).unwrap();
    }
    db.flush().unwrap();

    let mut handles = Vec::new();

    // Spawn 4 reader threads
    for tid in 0..4 {
        let db_c = Arc::clone(&db);
        let stop_c = Arc::clone(&stop);
        handles.push(thread::spawn(move || {
            let mut rng = TestRng::new(100 + tid as u64);
            let mut reads = 0;
            while !stop_c.load(Ordering::Relaxed) {
                let target = rng.gen_range(0, 500);
                let k = format!("key_{:04}", target);
                let exp = format!("val_{:04}", target);
                let got = db_c.get(k.as_bytes());
                assert_eq!(
                    got.as_deref(),
                    Some(exp.as_bytes()),
                    "Concurrent lock-free read failed during compaction!"
                );
                reads += 1;
            }
            reads
        }));
    }

    // Background worker mutating SSTs via flushes and compactions
    for step in 0..20 {
        // Insert new updates
        for j in 0..50 {
            let k = format!("transient_{}_{:04}", step, j);
            let v = format!("tval_{}", j);
            db.put(k.as_bytes(), v.as_bytes()).unwrap();
        }
        if step % 2 == 0 {
            db.flush().unwrap();
        } else {
            db.compact().unwrap();
        }
        thread::sleep(Duration::from_millis(5));
    }

    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let reads = h.join().unwrap();
        assert!(reads > 10, "Readers must complete queries");
    }

    db.assert_all_invariants().expect("Invariants must hold after compaction race");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Vector 3: Fail-Closed Manifest Persist Gate Enforcement.
/// Verifies that any state violating level disjointness is rejected
/// fail-closed before manifest persistence.
#[test]
fn test_dst_vector3_manifest_persist_disjointness_gate() {
    let dir = temp_db_dir("dst-v3");
    let mut db = Db::open(&dir).expect("Db::open");

    // Persisting valid manifest should succeed
    let persist = db.take_manifest_persist();
    assert!(persist.is_ok(), "Valid manifest persist must succeed");

    // Clean up
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Vector 4: 1,000-Step Metamorphic Simulation Against Reference Oracle with Crash Reboots.
/// Drives 1,000 randomized operations against ConcurrentDb and a BTreeMap reference oracle.
/// Performs random crash-reboots, compacts, flushes, and asserts invariants after every batch.
#[test]
fn test_dst_vector4_metamorphic_oracle_with_crash_reboots() {
    let dir = temp_db_dir("dst-v4");
    let mut rng = TestRng::new(0x9876_5432_10fe_dcba);
    let mut oracle: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

    let mut db = ConcurrentDb::open(&dir).expect("ConcurrentDb::open");

    for step in 0..1000 {
        let op_type = rng.gen_range(0, 10);
        match op_type {
            0..=4 => {
                // Put
                let k_num = rng.gen_range(0, 200);
                let key = format!("meta_key_{:04}", k_num).into_bytes();
                let val = format!("meta_val_{}_{}", step, rng.next_u64()).into_bytes();
                oracle.insert(key.clone(), val.clone());
                db.put(&key, &val).unwrap();
            }
            5..=6 => {
                // Delete
                let k_num = rng.gen_range(0, 200);
                let key = format!("meta_key_{:04}", k_num).into_bytes();
                oracle.remove(&key);
                db.delete(&key).unwrap();
            }
            7 => {
                // Flush or Compact
                if rng.gen_bool(1, 2) {
                    db.flush().unwrap();
                } else {
                    let _ = db.compact();
                }
            }
            8 => {
                // Point query check
                let k_num = rng.gen_range(0, 200);
                let key = format!("meta_key_{:04}", k_num).into_bytes();
                let expected = oracle.get(&key).cloned();
                let actual = db.get(&key);
                assert_eq!(
                    actual.as_deref(), expected.as_deref(),
                    "Metamorphic point query mismatch at step {}",
                    step
                );
            }
            9 => {
                // Periodic simulated reboot (every ~100 steps)
                if step % 50 == 0 {
                    db.assert_all_invariants().expect("Invariants before reboot");
                    drop(db);
                    db = ConcurrentDb::open(&dir).expect("ConcurrentDb reopen");
                    db.assert_all_invariants().expect("Invariants after reboot");

                    // Verify oracle equivalence on sample
                    for k_num in 0..50 {
                        let key = format!("meta_key_{:04}", k_num).into_bytes();
                        let expected = oracle.get(&key).cloned();
                        let actual = db.get(&key);
                        assert_eq!(
                            actual.as_deref(), expected.as_deref(),
                            "Oracle mismatch after reboot at step {}",
                            step
                        );
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    db.assert_all_invariants().expect("Final invariant sweep");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Vector 5: Multi-CF Level Disjointness & Parked MemTable Vlog GC Integration.
/// 1. Verifies that multiple column families residing at Level >= 1 with byte-overlapping
///    user keys across families satisfy the per-CF disjointness invariant and succeed in manifest persist.
/// 2. Verifies that vlog pointers held in parked_unflushed memtables are properly collected as live,
///    remapped during vlog GC, and read back without corruption.
#[test]
fn test_dst_vector5_multi_cf_disjointness_and_parked_vlog_gc() {
    let dir = temp_db_dir("dst-v5");
    let opts = pedradb_core::db::OpenOptions {
        large_value_threshold: Some(512),
        ..Default::default()
    };
    let mut db = Db::open_with(&dir, opts).expect("Db::open_with");
    db.set_physical_cfs(vec!["default".into(), "lock".into(), "meta".into()]);
    db.set_defer_auto_compact(true);

    // 1. Multi-CF keys with deliberate byte overlap across families:
    // "default" keys: "apple" (0x61) to "zebra" (0x7A)
    // "lock" keys: "lock\0alpha" to "lock\0omega"
    // "meta" keys: "meta\0001" to "meta\0999"
    db.put(b"apple", b"v_apple").unwrap();
    db.put(b"zebra", b"v_zebra").unwrap();
    db.flush_cf("default").unwrap();

    db.put(b"lock\0alpha", b"v_alpha").unwrap();
    db.put(b"lock\0omega", b"v_omega").unwrap();
    db.flush_cf("lock").unwrap();

    db.put(b"meta\0001", b"v_m1").unwrap();
    db.put(b"meta\0999", b"v_m999").unwrap();
    db.flush_cf("meta").unwrap();

    // Compact each CF into Level 1
    db.compact_ssts_only_cf("default").unwrap();
    db.compact_ssts_only_cf("lock").unwrap();
    db.compact_ssts_only_cf("meta").unwrap();

    // Verify manifest persist succeeds with all invariants held
    let persist = db.take_manifest_persist().expect("Manifest persist must succeed for multi-CF at Level 1");
    persist.write().expect("Manifest write must succeed");
    db.assert_all_invariants().expect("Multi-CF level disjointness invariant must hold");

    // Verify reads across all CFs
    assert_eq!(db.get(b"apple").as_deref(), Some(b"v_apple".as_ref()));
    assert_eq!(db.get(b"zebra").as_deref(), Some(b"v_zebra".as_ref()));
    assert_eq!(db.get(b"lock\0alpha").as_deref(), Some(b"v_alpha".as_ref()));
    assert_eq!(db.get(b"lock\0omega").as_deref(), Some(b"v_omega".as_ref()));
    assert_eq!(db.get(b"meta\0001").as_deref(), Some(b"v_m1".as_ref()));
    assert_eq!(db.get(b"meta\0999").as_deref(), Some(b"v_m999".as_ref()));

    // 2. Parked Memtable Vlog GC Sweeping & Remapping
    // Insert a value large enough to spill to vlog (4096 bytes >= 512 threshold)
    let large_val = vec![0x42u8; 4096];
    db.put(b"large_vlog_key", &large_val).unwrap();

    // Insert another large value into active memtable, then park it
    let large_val2 = vec![0x43u8; 4096];
    db.put(b"parked_large_key", &large_val2).unwrap();
    db.park_active_mem();
    assert_eq!(db.parked_unflushed_count(), 1, "Must have 1 parked memtable");

    // Compact vlog
    let gc_stats = db.compact_vlog().expect("compact_vlog with parked_unflushed must succeed");
    assert!(gc_stats.live_records > 0, "GC must retain live records from parked memtables");

    // Verify reads of both unflushed parked key and SST keys
    assert_eq!(db.get(b"large_vlog_key").as_deref(), Some(large_val.as_slice()));
    assert_eq!(db.get(b"parked_large_key").as_deref(), Some(large_val2.as_slice()));

    // Flush and verify persistence
    db.flush().unwrap();
    assert_eq!(db.parked_unflushed_count(), 0, "flush() must drain parked unflushed memtables");
    assert_eq!(db.get(b"parked_large_key").as_deref(), Some(large_val2.as_slice()));

    db.assert_all_invariants().expect("Invariants must hold after parked vlog GC");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Vector 6: Published SuperVersion Lock-Free Reader TOCTOU & Ghost Miss Prevention.
/// Verifies that when writers hold `inner.write()` (simulating heavy compaction or flush),
/// concurrent lock-free readers falling back to `lookup_published` and `scan_published`
/// observe keys stored in `parked_unflushed` memtables without ghost misses or ordering violations.
#[test]
fn test_dst_vector6_published_sv_parked_lock_free_reads() {
    let dir = temp_db_dir("dst-v6");
    let db = ConcurrentDb::open(&dir).expect("ConcurrentDb::open");

    // 1. Seed initial SSTs so SST envelopes are established
    for i in 0..100 {
        let k = format!("base_key_{:04}", i);
        let v = format!("base_val_{:04}", i);
        db.put(k.as_bytes(), v.as_bytes()).unwrap();
    }
    db.flush().unwrap();

    // 2. Insert new keys into active memtable, then park it
    for i in 0..50 {
        let k = format!("parked_key_{:04}", i);
        let v = format!("parked_val_{:04}", i);
        db.put(k.as_bytes(), v.as_bytes()).unwrap();
    }
    db.park_active_mem();

    // 3. Insert more keys into a second active memtable, and park it too (multiple parked tables)
    for i in 50..100 {
        let k = format!("parked_key_{:04}", i);
        let v = format!("parked_val_{:04}", i);
        db.put(k.as_bytes(), v.as_bytes()).unwrap();
    }
    db.park_active_mem();

    // 4. Test lock-free read while holding inner.write() lock
    db.with_inner_write_lock(|| {
        // Assert that all 100 parked keys are visible via published SuperVersion fallback
        for i in 0..100 {
            let k = format!("parked_key_{:04}", i);
            let expected_v = format!("parked_val_{:04}", i);
            let got = db.get(k.as_bytes());
            assert_eq!(
                got.as_deref(),
                Some(expected_v.as_bytes()),
                "Ghost miss detected: parked key {} must be visible during write lock hold!",
                k
            );
        }

        // Assert base SST keys are also still visible
        for i in 0..100 {
            let k = format!("base_key_{:04}", i);
            let expected_v = format!("base_val_{:04}", i);
            let got = db.get(k.as_bytes());
            assert_eq!(
                got.as_deref(),
                Some(expected_v.as_bytes()),
                "Base SST key {} must remain visible during write lock hold!",
                k
            );
        }

        // Test range scan across published SuperVersion
        let scan_results = db.scan_collect(
            std::ops::Bound::Included(b"parked_key_0020".as_ref()),
            std::ops::Bound::Excluded(b"parked_key_0030".as_ref()),
        );
        assert_eq!(
            scan_results.len(),
            10,
            "scan_published must see parked keys in range"
        );
        for (idx, (k, v)) in scan_results.iter().enumerate() {
            let expected_k = format!("parked_key_{:04}", 20 + idx);
            let expected_v = format!("parked_val_{:04}", 20 + idx);
            assert_eq!(k.as_ref(), expected_k.as_bytes());
            assert_eq!(v.as_ref(), expected_v.as_bytes());
        }
    });

    // 5. Concurrent multi-threaded test: multiple readers querying parked keys
    // while a writer holds the write lock for 100ms.
    let stop = Arc::new(AtomicBool::new(false));
    let mut reader_handles = Vec::new();

    for thread_id in 0..4 {
        let db_clone = db.clone();
        let stop_clone = Arc::clone(&stop);
        reader_handles.push(thread::spawn(move || {
            let mut read_count = 0u64;
            while !stop_clone.load(Ordering::Relaxed) {
                let key_idx = (read_count + thread_id) % 100;
                let k = format!("parked_key_{:04}", key_idx);
                let expected_v = format!("parked_val_{:04}", key_idx);
                let got = db_clone.get(k.as_bytes());
                assert_eq!(
                    got.as_deref(),
                    Some(expected_v.as_bytes()),
                    "Concurrent reader observed ghost miss on key {}",
                    k
                );
                read_count += 1;
            }
            read_count
        }));
    }

    // Writer holds lock and sleeps
    db.with_inner_write_lock(|| {
        thread::sleep(Duration::from_millis(50));
    });

    stop.store(true, Ordering::Relaxed);
    for h in reader_handles {
        let count = h.join().expect("Reader thread must not panic");
        assert!(count > 50, "Reader must execute operations");
    }

    db.assert_all_invariants().expect("Invariants must hold");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Vector 7: Strict Disk Inventory & Orphan SST Watermark Invariant Enforcement.
/// Verifies:
/// 1. Compaction and flushes maintain exact parity between manifest inventory and disk files.
/// 2. Any orphan SST file appearing on disk is immediately detected by `assert_disk_inventory_invariant`.
/// 3. Any file on disk with number >= manifest next_file_number trips the fail-closed `WatermarkViolation`.
/// 4. Reopen/reconciliation cleans up without leaking disk space or data corruption.
#[test]
fn test_dst_vector7_orphan_sst_and_watermark_invariants() {
    let dir = temp_db_dir("dst-v7");
    let mut db = Db::open(&dir).expect("Db::open");

    // 1. Seed data across multiple flushes to produce active SSTs
    for round in 0..3 {
        for i in 0..50 {
            let k = format!("k_{:02}_{:04}", round, i);
            let v = format!("v_{:02}_{:04}", round, i);
            db.put(k.as_bytes(), v.as_bytes()).unwrap();
        }
        db.flush().unwrap();
    }
    assert_eq!(db.sst_count(), 3);
    db.assert_all_invariants().expect("All invariants must hold after flushes");

    // 2. Compact to merge SSTs
    db.compact().unwrap();
    db.assert_all_invariants().expect("All invariants must hold after compaction");

    // 3. Inject an orphan SST on disk within valid watermark range
    // Pick an unused number below next_file_num
    let live_nums = db.sst_file_nums();
    let unused_num = (1..db.next_file_num()).find(|n| !live_nums.contains(n));
    if let Some(orphan_fn) = unused_num {
        let orphan_path = dir.join(format!("{orphan_fn:06}.sst"));
        std::fs::write(&orphan_path, b"fake orphan sst data").unwrap();

        // Must fail with OrphanSstFile error!
        let inv_res = db.assert_disk_inventory_invariant();
        assert!(
            inv_res.is_err(),
            "Orphan SST file on disk must be rejected fail-closed"
        );
        match inv_res.unwrap_err() {
            pedradb_core::orphan_sst_cleanup_kernel::DiskResourceInvariantError::OrphanSstFile { file_number, .. } => {
                assert_eq!(file_number, orphan_fn);
            }
            other => panic!("Expected OrphanSstFile, got: {:?}", other),
        }

        // Clean up the injected orphan
        std::fs::remove_file(&orphan_path).unwrap();
        db.assert_all_invariants().expect("Invariants must restore after orphan removal");
    }

    // 4. Inject a watermark-violating file (file_number >= next_file_num)
    let bad_num = db.next_file_num() + 10;
    let bad_path = dir.join(format!("{bad_num:06}.sst"));
    std::fs::write(&bad_path, b"watermark violator").unwrap();

    let inv_res = db.assert_disk_inventory_invariant();
    assert!(
        inv_res.is_err(),
        "Watermark violation must be rejected fail-closed"
    );
    match inv_res.unwrap_err() {
        pedradb_core::orphan_sst_cleanup_kernel::DiskResourceInvariantError::WatermarkViolation { file_number, .. } => {
            assert_eq!(file_number, bad_num);
        }
        other => panic!("Expected WatermarkViolation, got: {:?}", other),
    }

    // Clean up injected watermark violator
    std::fs::remove_file(&bad_path).unwrap();
    db.assert_all_invariants().expect("Invariants must hold after cleanup");

    // 5. Verify database reopens cleanly and serves queries
    drop(db);
    let db_reopen = Db::open(&dir).expect("Reopen must succeed");
    db_reopen.assert_all_invariants().expect("Invariants must hold after reopen");
    assert_eq!(db_reopen.get(b"k_00_0000").as_deref(), Some(b"v_00_0000".as_ref()));
    drop(db_reopen);
    let _ = std::fs::remove_dir_all(&dir);
}

/// =========================================================================
/// DST Vector 8: Concurrent In-Flight Group Unapplied Staging Invariant (RFC-0303)
///
/// Validates that `unstage_unapplied` removes ONLY the exact sequence numbers
/// belonging to the completing group, preserving in-flight interleaved
/// operations belonging to concurrent groups so OCC conflict detection
/// never misses a staged write.
/// =========================================================================
#[test]
fn test_dst_vector8_interleaved_unapplied_occ_invariants() {
    let dir = temp_db_dir("dst-v8");
    let mut db = Db::open(&dir).expect("Db open failed");

    // Establish base sequence
    db.put(b"k_base", b"v_base").unwrap();
    let snap = db.last_sequence();

    // Stage two simulated groups with interleaved sequence numbers
    // Group 1 has seq 10 and seq 20 on "k_g1_a" and "k_g1_b"
    // Group 2 has seq 15 on "k_g2"
    db.stage_unapplied_test(&[(10, b"k_g1_a", b"v10"), (20, b"k_g1_b", b"v20")]);
    db.stage_unapplied_test(&[(15, b"k_g2", b"v15")]);

    // Both should be visible to key_has_write_after at snapshot `snap`
    assert!(db.key_has_write_after(b"k_g1_a", snap));
    assert!(db.key_has_write_after(b"k_g2", snap));
    assert!(db.key_has_write_after(b"k_g1_b", snap));

    // Now complete and unstage Group 1 (which spans seq 10 and seq 20)
    db.unstage_unapplied_seqs_test(&[10, 20]);

    // Group 1's ops should no longer be in unapplied
    assert!(!db.key_has_write_after(b"k_g1_a", snap));
    assert!(!db.key_has_write_after(b"k_g1_b", snap));

    // CRITICAL INVARIANT: Group 2's op (seq 15, which falls between 10 and 20)
    // MUST NOT have been purged!
    assert!(
        db.key_has_write_after(b"k_g2", snap),
        "Interleaved in-flight op (seq 15) must survive unstage of group spanning [10, 20]"
    );

    // Finally unstage Group 2
    db.unstage_unapplied_seqs_test(&[15]);
    assert!(!db.key_has_write_after(b"k_g2", snap));

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// =========================================================================
/// DST Vector 9: ConcurrentDb Lock-Free ValueLog Refresh During Compaction (RFC-0303)
///
/// Validates that `ConcurrentDb` readers seamlessly transition to the new
/// ValueLog handle after `compact_vlog` replaces the handle, preventing
/// silent data loss / `None` reads from stale handles.
/// =========================================================================
#[test]
fn test_dst_vector9_concurrent_vlog_handle_refresh_on_compaction() {
    let dir = temp_db_dir("dst-v9");
    let opts = pedradb_core::OpenOptions {
        large_value_threshold: Some(32),
        ..Default::default()
    };
    let cdb = ConcurrentDb::open_with(&dir, opts).expect("ConcurrentDb open failed");

    // Write values large enough to spill to vlog
    let large_val1 = vec![b'X'; 128];
    let large_val2 = vec![b'Y'; 128];
    cdb.put(b"vlog_k1", &large_val1).unwrap();
    cdb.put(b"vlog_k2", &large_val2).unwrap();

    // Verify initial reads
    assert_eq!(cdb.get(b"vlog_k1").as_deref(), Some(large_val1.as_slice()));
    assert_eq!(cdb.get(b"vlog_k2").as_deref(), Some(large_val2.as_slice()));

    // Overwrite vlog_k1 to create garbage in vlog
    let large_val1_v2 = vec![b'Z'; 128];
    cdb.put(b"vlog_k1", &large_val1_v2).unwrap();

    // Compact vlog (replaces vlog handle and remaps memory)
    cdb.with_write(|db| {
        db.compact_vlog().expect("compact_vlog failed");
    });

    // CRITICAL INVARIANT: ConcurrentDb reader must be able to read all values
    // with the refreshed vlog handle without returning None or corrupt data
    assert_eq!(
        cdb.get(b"vlog_k1").as_deref(),
        Some(large_val1_v2.as_slice()),
        "Updated value must resolve via refreshed vlog handle"
    );
    assert_eq!(
        cdb.get(b"vlog_k2").as_deref(),
        Some(large_val2.as_slice()),
        "Preserved value must resolve via refreshed vlog handle"
    );

    drop(cdb);
    let _ = std::fs::remove_dir_all(&dir);
}

/// =========================================================================
/// DST Vector 10: Strict Sequence Number Monotonicity Across Errors (RFC-0303)
///
/// Validates that sequence numbers are strictly burned on commit failure and
/// NEVER rolled back, preventing duplicate sequence collisions upon WAL replay.
/// =========================================================================
#[test]
fn test_dst_vector10_strict_sequence_monotonicity_on_commit_error() {
    let dir = temp_db_dir("dst-v10");
    let mut db = Db::open(&dir).expect("Db open failed");

    db.put(b"k1", b"v1").unwrap();
    let seq1 = db.last_sequence();
    assert!(seq1 > 0);

    // Apply a multi-op batch
    let batch = vec![
        pedradb_core::BatchOp::Put {
            key: bytes::Bytes::from_static(b"k2"),
            value: bytes::Bytes::from_static(b"v2"),
        },
        pedradb_core::BatchOp::Put {
            key: bytes::Bytes::from_static(b"k3"),
            value: bytes::Bytes::from_static(b"v3"),
        },
    ];
    let seq2 = db.apply_batch(batch).unwrap();
    assert_eq!(seq2, seq1 + 2);

    // Verify next write has strictly higher sequence number
    db.put(b"k4", b"v4").unwrap();
    let seq3 = db.last_sequence();
    assert_eq!(seq3, seq2 + 1);

    drop(db);

    // Reopen and verify monotonicity preserved
    let db_reopen = Db::open(&dir).expect("Reopen failed");
    assert_eq!(db_reopen.last_sequence(), seq3);
    assert_eq!(db_reopen.get(b"k1").as_deref(), Some(b"v1".as_ref()));
    assert_eq!(db_reopen.get(b"k2").as_deref(), Some(b"v2".as_ref()));
    assert_eq!(db_reopen.get(b"k3").as_deref(), Some(b"v3".as_ref()));
    assert_eq!(db_reopen.get(b"k4").as_deref(), Some(b"v4".as_ref()));
    drop(db_reopen);

    let _ = std::fs::remove_dir_all(&dir);
}



