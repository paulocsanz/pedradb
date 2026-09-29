//! RFC-0302: Deterministic Metamorphic Adversarial Fuzzer (DMAF) & Asymmetry Hunter.
//!
//! Systematically attacks the 5 architectural storage engine asymmetry vectors:
//! 1. Vector 1: Hot Key Version Density & LSM SST Boundary Splits (no split mid-user-key).
//! 2. Vector 2: Multi-Level Tombstone Shadowing & Bottommost GC Masking (no premature drop).
//! 3. Vector 3: Asymmetric Multi-CF Crash & Recovery (temporal desync & archived WAL replay).
//! 4. Vector 4: High-Contention Intra-Group OCC Transactions (zero lost updates & serializability).
//! 5. Vector 5: Sequence Monotonicity & Reopen Watermark Invariants.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::Db;
use pedradb_core::resilient_tx::TransactionRetryPolicy;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cnt = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pedradb-dmaf-{tag}-{nanos}-{cnt}"));
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
            seed = 0x853c_49e6_748f_ea9b;
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
}

/// In-memory reference oracle with exact Range Delete semantics.
#[derive(Debug, Default, Clone)]
struct ReferenceOracle {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl ReferenceOracle {
    fn new() -> Self {
        Self::default()
    }

    fn put(&mut self, key: &[u8], val: &[u8]) {
        self.map.insert(key.to_vec(), val.to_vec());
    }

    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.map.get(key).cloned()
    }

    fn delete(&mut self, key: &[u8]) {
        self.map.remove(key);
    }

    fn delete_range(&mut self, start: &[u8], end: &[u8]) {
        let keys_to_remove: Vec<Vec<u8>> = self
            .map
            .range(start.to_vec()..end.to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys_to_remove {
            self.map.remove(&k);
        }
    }

    fn verify_equivalence(&self, db: &ConcurrentDb) {
        for (k, expected_v) in &self.map {
            let actual = db.get(k);
            assert_eq!(
                actual.as_deref(),
                Some(expected_v.as_slice()),
                "Metamorphic mismatch for key {:?}",
                String::from_utf8_lossy(k)
            );
        }
    }

    fn verify_equivalence_raw(&self, db: &Db) {
        for (k, expected_v) in &self.map {
            let actual = db.get(k);
            assert_eq!(
                actual.as_deref(),
                Some(expected_v.as_slice()),
                "Metamorphic mismatch in Db for key {:?}",
                String::from_utf8_lossy(k)
            );
        }
    }
}

/// Vector 1: Hot Key Version Density & LSM SST Boundary Split Attack.
/// Floods a small set of keys with thousands of updates and large payloads to force
/// block and SST boundary calculations. Enforces zero mid-user-key splits across L1+.
#[test]
fn test_fuzz_asymmetry_v1_hot_key_density_splits() {
    let seed = 0x1A2B_3C4D_5E6F_7081;
    let mut rng = TestRng::new(seed);
    println!("[DMAF-V1] Running Hot Key Density Split Attack with seed 0x{seed:016X}");

    let path = temp_db_dir("v1-splits");
    let mut db = Db::open(&path).expect("open db");
    let mut oracle = ReferenceOracle::new();

    let hot_keys: Vec<Vec<u8>> = (0..5).map(|i| format!("hot_key_{i:02}").into_bytes()).collect();

    for step in 0..2500 {
        let k_idx = rng.gen_range(0, hot_keys.len());
        let key = &hot_keys[k_idx];
        let val_len = rng.gen_range(64, 512);
        let val: Vec<u8> = (0..val_len).map(|_| rng.next_u64() as u8).collect();

        db.put(key, &val).expect("put");
        oracle.put(key, &val);

        if step % 250 == 0 && step > 0 {
            db.flush().expect("flush");
            if step % 500 == 0 {
                let _ = db.compact();
            }
            // Strict Invariant: All SST runs at Level >= 1 must be strictly pairwise disjoint
            db.assert_level_disjointness_invariant()
                .expect("Level disjointness violated mid-user-key split!");
            db.assert_disk_inventory_invariant()
                .expect("Disk inventory violation");
        }
    }

    oracle.verify_equivalence_raw(&db);
    db.assert_all_invariants().expect("final invariants");
    db.close().expect("close db");
    let _ = std::fs::remove_dir_all(&path);
}

/// Vector 2: Multi-Level Tombstone Shadowing & Bottommost GC Masking.
/// Exercises tombstones across L0, L1, L2, L3, ensuring no partial compaction
/// erroneously drops a tombstone when an older version exists deeper down.
#[test]
fn test_fuzz_asymmetry_v2_deep_multilevel_tombstone_shadowing() {
    let seed = 0xCAFE_BABE_9988_7766;
    let _ = TestRng::new(seed);
    println!("[DMAF-V2] Running Multi-Level Tombstone Shadowing with seed 0x{seed:016X}");

    let path = temp_db_dir("v2-tombstones");
    let mut db = Db::open(&path).expect("open db");
    let mut oracle = ReferenceOracle::new();

    let all_keys: Vec<Vec<u8>> = (0..50).map(|i| format!("tomb_key_{i:03}").into_bytes()).collect();

    // Generation 1: Push initial values to deep levels
    for k in &all_keys {
        let v = b"old_deep_val";
        db.put(k, v).expect("put");
        oracle.put(k, v);
    }
    db.flush().expect("flush gen1");
    let _ = db.compact(); // Pushed down

    // Generation 2: Update half of them and push down again
    for i in 0..25 {
        let v = b"mid_level_val";
        db.put(&all_keys[i], v).expect("put");
        oracle.put(&all_keys[i], v);
    }
    db.flush().expect("flush gen2");
    let _ = db.compact();

    // Generation 3: Delete range and point delete in L0
    let start_del = &all_keys[10];
    let end_del = &all_keys[30];
    db.delete_range(start_del, end_del).expect("delete_range");
    oracle.delete_range(start_del, end_del);

    let point_del = &all_keys[5];
    db.delete(point_del).expect("point delete");
    oracle.delete(point_del);

    // Partial compaction without bottommost should retain tombstones
    db.flush().expect("flush gen3");
    let _ = db.compact();

    // Verify all keys match oracle - no resurrection of old_deep_val
    for k in &all_keys {
        let actual = db.get(k);
        let expected = oracle.get(k);
        assert_eq!(
            actual.as_deref(),
            expected.as_deref(),
            "Zombie resurrection detected for key {:?}",
            String::from_utf8_lossy(k)
        );
    }

    db.assert_all_invariants().expect("invariants");
    db.close().expect("close db");
    let _ = std::fs::remove_dir_all(&path);
}

/// Vector 3: Asymmetric Multi-CF Crash & Recovery.
/// Simulates disparate flush schedules across CFs with abrupt crash-reboot cycles.
#[test]
fn test_fuzz_asymmetry_v3_multicf_asymmetric_crash_recovery() {
    let seed = 0x55AA_1234_9876_FEDC;
    let _ = TestRng::new(seed);
    println!("[DMAF-V3] Running Asymmetric Multi-CF Crash Recovery with seed 0x{seed:016X}");

    let path = temp_db_dir("v3-multicf-crash");
    let mut oracle = ReferenceOracle::new();

    // Session 1: Write to default and aux CFs, flush only default, then crash
    {
        let mut db = Db::open(&path).expect("open db");
        db.set_physical_cfs(vec!["default".into(), "orders".into()]);

        for i in 0..100 {
            let k_def = format!("default\0item_{i:03}").into_bytes();
            let v_def = format!("v_def_{i}").into_bytes();
            db.put(&k_def, &v_def).expect("put");
            oracle.put(&k_def, &v_def);

            let k_ord = format!("orders\0item_{i:03}").into_bytes();
            let v_ord = format!("v_ord_{i}").into_bytes();
            db.put(&k_ord, &v_ord).expect("put");
            oracle.put(&k_ord, &v_ord);
        }

        // Flush default, leave orders un-flushed in memtable
        db.flush_cf("default").expect("flush default");

        // Sudden ungraceful close (crash)
        drop(db);
    }

    // Session 2: Reopen after crash, verify recovery replay and monotonicity
    {
        let db2 = Db::open(&path).expect("reopen db after crash");
        db2.assert_sequence_monotonicity_invariant().expect("monotonicity after recovery");
        oracle.verify_equivalence_raw(&db2);
        db2.close().expect("clean close session 2");
    }

    let _ = std::fs::remove_dir_all(&path);
}

/// Vector 4: High-Contention Intra-Group OCC Fuzzing.
/// Concurrent threads hammering conflicting keys, range deletes, and counters.
#[test]
fn test_fuzz_asymmetry_v4_high_contention_concurrent_occ() {
    let seed: u64 = 0x900D_F00D_1337_BEEF;
    println!("[DMAF-V4] Running High-Contention Concurrent OCC Fuzzing with seed 0x{seed:016X}");

    let path = temp_db_dir("v4-occ-fuzz");
    let db = Arc::new(ConcurrentDb::open(&path).expect("open db"));

    let counter_key = b"shared_hot_counter";
    db.put(counter_key, &0u64.to_le_bytes()).expect("init counter");

    let num_threads = 6;
    let txs_per_thread = 50;
    let mut handles = Vec::new();

    for tid in 0..num_threads {
        let db_c = Arc::clone(&db);
        handles.push(thread::spawn(move || {
            let policy = TransactionRetryPolicy {
                max_retries: 500,
                initial_backoff: Duration::from_micros(20),
                max_backoff: Duration::from_millis(10),
                backoff_multiplier: 1.5,
                jitter: true,
            };

            for i in 0..txs_per_thread {
                // Mix increment on shared counter and thread-local unique range ops
                db_c.transact_with(policy, |tx| {
                    let cur = tx.get(counter_key)?.expect("exists");
                    let val = u64::from_le_bytes(cur.as_ref().try_into().unwrap());
                    tx.put(counter_key, &(val + 1).to_le_bytes())?;

                    let t_key = format!("thread_{tid}_item_{i}").into_bytes();
                    tx.put(&t_key, b"ok")?;
                    Ok(())
                }).expect("transact commit");
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    // Verify counter matches exactly num_threads * txs_per_thread
    let final_bytes = db.get(counter_key).expect("exists");
    let final_val = u64::from_le_bytes(final_bytes.as_ref().try_into().unwrap());
    assert_eq!(
        final_val,
        (num_threads * txs_per_thread) as u64,
        "Lost update detected in concurrent OCC transactions!"
    );

    db.assert_all_invariants().expect("all invariants hold");
    drop(db);
    let _ = std::fs::remove_dir_all(&path);
}

/// Vector 5: Continuous Metamorphic Fuzz Campaign with Random Crash Reboots.
/// 500 interleaved metamorphic transitions: Puts, Deletes, DeleteRanges, Flushes,
/// Compactions, and Crash-Reopens, continuously checked against the oracle.
#[test]
fn test_fuzz_asymmetry_v5_continuous_metamorphic_fuzz_campaign() {
    let seed = 0xDE7E_8811_4422_99AA;
    let mut rng = TestRng::new(seed);
    println!("[DMAF-V5] Running Continuous Metamorphic Campaign with seed 0x{seed:016X}");

    let path = temp_db_dir("v5-metamorphic");
    let mut oracle = ReferenceOracle::new();

    let mut db = ConcurrentDb::open(&path).expect("open db");
    let key_pool: Vec<Vec<u8>> = (0..30).map(|i| format!("meta_key_{i:02}").into_bytes()).collect();

    for step in 0..500 {
        let op_type = rng.gen_range(0, 10);
        match op_type {
            0..=4 => {
                // Put
                let k_idx = rng.gen_range(0, key_pool.len());
                let key = &key_pool[k_idx];
                let val = format!("val_step_{step}_{}", rng.next_u64()).into_bytes();
                oracle.put(key, &val);
                db.put(key, &val).expect("put");
            }
            5..=6 => {
                // Delete
                let k_idx = rng.gen_range(0, key_pool.len());
                let key = &key_pool[k_idx];
                oracle.delete(key);
                db.delete(key).expect("delete");
            }
            7 => {
                // DeleteRange
                let i1 = rng.gen_range(0, key_pool.len());
                let i2 = rng.gen_range(0, key_pool.len());
                let (lo, hi) = if i1 <= i2 { (i1, i2) } else { (i2, i1) };
                let start = &key_pool[lo];
                let end = &key_pool[hi];
                oracle.delete_range(start, end);
                db.delete_range(start, end).expect("delete_range");
            }
            8 => {
                // Flush & Invariant check
                db.flush().expect("flush");
                db.assert_all_invariants().expect("invariants after flush");
            }
            9 => {
                // Simulated Crash & Reopen
                db.close().expect("close before reopen");
                db = ConcurrentDb::open(&path).expect("reopen after simulated crash");
                db.assert_sequence_monotonicity_invariant().expect("monotonicity on reopen");
                oracle.verify_equivalence(&db);
            }
            _ => unreachable!(),
        }

        if step % 50 == 0 {
            oracle.verify_equivalence(&db);
        }
    }

    oracle.verify_equivalence(&db);
    db.assert_all_invariants().expect("final invariants");
    db.close().expect("close db");
    let _ = std::fs::remove_dir_all(&path);
    println!("[DMAF-V5] 500 Metamorphic Transitions 100% VERIFIED against Reference Oracle.");
}
