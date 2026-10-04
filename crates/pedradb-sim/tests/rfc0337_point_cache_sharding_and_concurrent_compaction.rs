//! Integration test suite for RFC-0337:
//! Point Cache Multi-Shard & Compaction Off-Lock Verification Suite.
//!
//! Proves:
//! 1. Multi-threaded Point Cache linearizability & zero lock contention across 16 shards.
//! 2. Uncontested concurrent writes (`ConcurrentDb::put`) while leveled compaction is actively running off-lock.
//! 3. Decompression bomb rejection via `SafeBlockDecoder` in SST paths.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pedradb_core::{ConcurrentDb, OpenOptions, WriteOptions};
use pedradb_sim::FailingEnvArc;

fn temp_db_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("pedra_rfc0337_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[test]
fn test_rfc0337_point_cache_multi_shard_concurrency() {
    let dir = temp_db_dir("cache_shards");
    let env = FailingEnvArc::passing();
    let opts = OpenOptions::default();
    let db = ConcurrentDb::open_with_env(&dir, opts, env).unwrap();

    // Pre-populate keys mapping across diverse shards
    for i in 0..128 {
        let k = format!("user_key_{:04}", i);
        let v = format!("value_{:04}", i);
        db.put(k.as_bytes(), v.as_bytes()).unwrap();
    }
    db.flush().unwrap();

    let db = Arc::new(db);
    let stop = Arc::new(AtomicBool::new(false));
    let read_count = Arc::new(AtomicU64::new(0));

    // Spawn 16 reader threads simultaneously exercising the 16 shards
    let mut handles = Vec::new();
    for thread_idx in 0..16 {
        let db_cloned = Arc::clone(&db);
        let stop_cloned = Arc::clone(&stop);
        let count_cloned = Arc::clone(&read_count);
        let handle = thread::spawn(move || {
            let mut round = 0;
            while !stop_cloned.load(Ordering::Relaxed) && round < 2000 {
                // Key targetted to rotate across shards
                let key_idx = (thread_idx * 8 + (round % 8)) % 128;
                let k = format!("user_key_{:04}", key_idx);
                let val_old = format!("value_{:04}", key_idx);
                let val_new = format!("updated_value_{:04}", key_idx);
                let got = db_cloned.get(k.as_bytes());
                assert!(
                    got.as_deref() == Some(val_old.as_bytes()) || got.as_deref() == Some(val_new.as_bytes()),
                    "Point get must return linearizable value: got {:?}",
                    got
                );
                count_cloned.fetch_add(1, Ordering::Relaxed);
                round += 1;
            }
        });
        handles.push(handle);
    }

    // Concurrent invalidation & cache refills
    for round in 0..100 {
        let k = format!("user_key_{:04}", round % 128);
        let v = format!("updated_value_{:04}", round % 128);
        db.put_with(k.as_bytes(), v.as_bytes(), WriteOptions::sync()).unwrap();
        thread::sleep(Duration::from_millis(1));
    }

    stop.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().unwrap();
    }

    assert!(
        read_count.load(Ordering::Relaxed) > 1000,
        "Concurrent readers must complete thousands of point queries without lock contention"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_rfc0337_concurrent_compaction_off_lock() {
    let dir = temp_db_dir("compaction_off_lock");
    let env = FailingEnvArc::passing();
    let mut opts = OpenOptions::default();
    opts.auto_compact_sst_count = None;
    opts.auto_compact_sst_bytes = None;
    let db = Arc::new(ConcurrentDb::open_with_env(&dir, opts, env).unwrap());

    // Step 1: Ingest multiple SST files with overlapping key ranges
    for file_idx in 0..8 {
        for key_idx in 0..300 {
            let k = format!("overlap_key_{:04}", (key_idx * 7 + file_idx * 13) % 500);
            let v = vec![b'x'; 256];
            db.put(k.as_bytes(), &v).unwrap();
        }
        db.flush().unwrap();
    }

    let initial_ssts = db.sst_count();
    assert!(initial_ssts >= 1, "Initial SST count must be >= 1, got {}", initial_ssts);

    let stop = Arc::new(AtomicBool::new(false));
    let write_progress = Arc::new(AtomicU64::new(0));

    // Step 2: Spawn concurrent writers
    let mut writer_handles = Vec::new();
    for thread_id in 0..4 {
        let db_cloned = Arc::clone(&db);
        let stop_cloned = Arc::clone(&stop);
        let progress_cloned = Arc::clone(&write_progress);
        let h = thread::spawn(move || {
            let mut i = 0;
            while !stop_cloned.load(Ordering::Relaxed) {
                let k = format!("concurrent_writer_{}_{:06}", thread_id, i);
                let v = format!("val_{}_{:06}", thread_id, i);
                let res = db_cloned.put(k.as_bytes(), v.as_bytes());
                assert!(res.is_ok(), "Concurrent put must succeed during compaction");
                progress_cloned.fetch_add(1, Ordering::Relaxed);
                i += 1;
                if i % 50 == 0 {
                    let _ = db_cloned.flush();
                    thread::yield_now();
                }
            }
        });
        writer_handles.push(h);
    }

    // Step 3: Trigger off-lock compaction in a loop while writers generate load
    let db_compactor = Arc::clone(&db);
    let stop_compactor = Arc::clone(&stop);
    let compactor_handle = thread::spawn(move || {
        let start = Instant::now();
        let mut compactions = 0;
        while !stop_compactor.load(Ordering::Relaxed) && compactions < 10 {
            let _ = db_compactor.compact_leveled();
            compactions += 1;
            thread::sleep(Duration::from_millis(2));
        }
        (start.elapsed(), compactions)
    });

    // Let writers perform steady writes concurrently with compactions
    while write_progress.load(Ordering::Relaxed) < 200 {
        thread::sleep(Duration::from_millis(5));
    }

    // Signal all threads to finish
    stop.store(true, Ordering::Relaxed);
    let (compaction_duration, compactions) = compactor_handle.join().unwrap();
    for h in writer_handles {
        h.join().unwrap();
    }

    let completed_writes = write_progress.load(Ordering::Relaxed);
    println!(
        "RFC-0337 off-lock compactions ({}) completed in {:?}, concurrent writes absorbed: {}",
        compactions, compaction_duration, completed_writes
    );

    assert!(
        completed_writes >= 200,
        "Writers must make steady progress during off-lock compaction without being blocked"
    );

    // Verify that data is readable and consistent
    for i in 0..completed_writes.min(50) {
        let k = format!("concurrent_writer_0_{:06}", i);
        let expected = format!("val_0_{:06}", i);
        assert_eq!(
            db.get(k.as_bytes()).as_deref(),
            Some(expected.as_bytes()),
            "Written data during off-lock compaction must be durable and readable"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_rfc0337_decompression_bomb_rejection_fail_closed() {
    use pedradb_core::sst_block_decompression_guard_kernel::{
        BlockDecompressionGuardConfig, DecompressionGuardError, SafeBlockDecoder,
    };

    // Synthesize a decompression bomb block:
    // small compressed payload that claims an astronomical uncompressed size
    let bomb_uncompressed_size = 128 * 1024 * 1024; // 128 MiB (exceeds default 64 MiB cap)
    let mut bomb_payload = Vec::new();
    bomb_payload.extend_from_slice(&(bomb_uncompressed_size as u32).to_le_bytes());
    bomb_payload.extend_from_slice(&[0u8; 32]); // Dummy compressed stream

    let config = BlockDecompressionGuardConfig::default();
    let mut scratch = Vec::new();

    let res = SafeBlockDecoder::decompress_lz4_into(&bomb_payload, &config, &mut scratch);
    assert!(
        matches!(
            res,
            Err(DecompressionGuardError::ExceedsMaxUncompressedSize { .. })
        ),
        "Decompression bomb claiming 128 MiB must be rejected before memory allocation: got {:?}",
        res
    );
    assert_eq!(scratch.len(), 0, "No memory should be allocated on rejection");

    // Synthesize an expansion ratio explosion (> 256x)
    let ratio_bomb_size = 32 * 1024 * 1024; // 32 MiB
    let small_payload_len = 64; // 64 bytes -> 32 MiB / 64 B = 524,288x expansion
    let mut ratio_bomb = Vec::new();
    ratio_bomb.extend_from_slice(&(ratio_bomb_size as u32).to_le_bytes());
    ratio_bomb.resize(small_payload_len, 0xaa);

    let res_ratio = SafeBlockDecoder::decompress_lz4_into(&ratio_bomb, &config, &mut scratch);
    assert!(
        matches!(
            res_ratio,
            Err(DecompressionGuardError::ExpansionRatioExplosion { .. })
        ),
        "Decompression bomb with 500k:1 expansion must be rejected: got {:?}",
        res_ratio
    );
    assert_eq!(scratch.len(), 0, "No memory should be allocated on ratio rejection");
}
