//! Differential Oracle Testing Harness (Barreira 1).
//!
//! Validates `rocksdb_compat::DB` and `ConcurrentDb` against an in-memory
//! reference oracle (`CfModelStore`) over randomized sequences of operations:
//! - Point Puts, Gets, and Deletes across multiple Column Families.
//! - Range Deletions (`delete_range_cf`) across memtable, flushed SSTs, and compactions.
//! - Append Merge Operands (`merge` / `merge_cf`).
//! - MultiGet batch evaluations.
//! - Forward and Reverse full-range scans.
//! - Prefix iteration.
//! - Disk resource invariant validation (Barreira 2) after transitions.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use rocksdb_compat::{
    ColumnFamily, IteratorMode, Options, DB,
};

/// Deterministic PRNG for reproducible test sequences.
struct TestRng(u64);
impl TestRng {
    fn new(seed: u64) -> Self {
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
    fn gen_bool(&mut self, numerator: usize, denominator: usize) -> bool {
        self.gen_range(0, denominator) < numerator
    }
}

#[derive(Debug, Default, Clone)]
struct CfModelStore {
    cfs: BTreeMap<String, BTreeMap<Vec<u8>, Vec<u8>>>,
    history: BTreeMap<(String, Vec<u8>), Vec<String>>,
}

impl CfModelStore {
    fn new(cf_names: &[&str]) -> Self {
        let mut cfs = BTreeMap::new();
        cfs.insert("default".to_string(), BTreeMap::new());
        for &name in cf_names {
            cfs.insert(name.to_string(), BTreeMap::new());
        }
        Self {
            cfs,
            history: BTreeMap::new(),
        }
    }

    fn put(&mut self, step: usize, cf: &str, key: &[u8], val: &[u8]) {
        self.history
            .entry((cf.to_string(), key.to_vec()))
            .or_default()
            .push(format!("step {step}: PUT val={}", String::from_utf8_lossy(val)));
        self.cfs
            .entry(cf.to_string())
            .or_default()
            .insert(key.to_vec(), val.to_vec());
    }

    fn get(&self, cf: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.cfs.get(cf).and_then(|m| m.get(key).cloned())
    }

    fn delete(&mut self, step: usize, cf: &str, key: &[u8]) {
        self.history
            .entry((cf.to_string(), key.to_vec()))
            .or_default()
            .push(format!("step {step}: DELETE"));
        if let Some(m) = self.cfs.get_mut(cf) {
            m.remove(key);
        }
    }

    fn delete_range(&mut self, step: usize, cf: &str, start: &[u8], end: &[u8]) {
        if let Some(m) = self.cfs.get_mut(cf) {
            let to_remove: Vec<Vec<u8>> = m
                .range(start.to_vec()..end.to_vec())
                .map(|(k, _)| k.clone())
                .collect();
            for k in to_remove {
                self.history
                    .entry((cf.to_string(), k.clone()))
                    .or_default()
                    .push(format!("step {step}: DELETE_RANGE [{}, {})", String::from_utf8_lossy(start), String::from_utf8_lossy(end)));
                m.remove(&k);
            }
        }
    }

    fn merge(&mut self, step: usize, cf: &str, key: &[u8], operand: &[u8]) {
        self.history
            .entry((cf.to_string(), key.to_vec()))
            .or_default()
            .push(format!("step {step}: MERGE operand={:?}", operand));
        let m = self.cfs.entry(cf.to_string()).or_default();
        let entry = m.entry(key.to_vec()).or_default();
        entry.extend_from_slice(operand);
    }

    fn scan_forward(&self, cf: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.cfs
            .get(cf)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    fn scan_reverse(&self, cf: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.cfs
            .get(cf)
            .map(|m| m.iter().rev().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    fn scan_prefix(&self, cf: &str, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.cfs
            .get(cf)
            .map(|m| {
                m.range(prefix.to_vec()..)
                    .take_while(|(k, _)| k.starts_with(prefix))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_tmp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cnt = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pedradb-diff-oracle-{tag}-{nanos}-{cnt}"));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn differential_oracle_randomized_workload() {
    let dir = unique_tmp_dir("random-cf");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    // RFC-0306 triage: PEDRA_ORACLE_NO_BG=1 runs without background
    // flush/compact workers — if the stale read survives, the bug is in
    // the synchronous path; if it vanishes, it is a worker race.
    if std::env::var_os("PEDRA_ORACLE_NO_BG").is_some() {
        opts.set_max_background_jobs(0);
    }
    opts.set_merge_operator_associative("concat", |_k, exist, ops| {
        let mut out = exist.unwrap_or(&[]).to_vec();
        for o in ops.iter() {
            out.extend_from_slice(o);
        }
        Some(out)
    });

    let extra_cfs = ["cf_alpha", "cf_beta"];
    let db = DB::open_cf(&opts, &dir, &extra_cfs).unwrap();

    let mut model = CfModelStore::new(&extra_cfs);
    let mut rng = TestRng::new(0xFEED_FACE_1234_5678);

    let all_cfs = vec!["default", "cf_alpha", "cf_beta"];

    let get_cf_handle = |name: &str| -> Option<ColumnFamily> {
        if name == "default" {
            None
        } else {
            Some(db.cf_handle(name).expect("CF must exist"))
        }
    };

    let key_pool: Vec<Vec<u8>> = (0..60)
        .map(|i| format!("key_{i:04}").into_bytes())
        .collect();

    // Run 1,000 randomized operations verifying equivalence after each step
    for step in 0..1000 {
        let cf_idx = rng.gen_range(0, all_cfs.len());
        let cf = all_cfs[cf_idx];
        let cf_handle = get_cf_handle(cf);

        let op_type = rng.gen_range(0, 10);
        match op_type {
            0..=2 => {
                // Put
                let k_idx = rng.gen_range(0, key_pool.len());
                let key = &key_pool[k_idx];
                let val = format!("val_step_{step}_{}", rng.next_u64()).into_bytes();

                model.put(step, cf, key, &val);
                if let Some(ref h) = cf_handle {
                    db.put_cf(h, key, &val).unwrap();
                } else {
                    db.put(key, &val).unwrap();
                }
            }
            3 => {
                // Delete
                let k_idx = rng.gen_range(0, key_pool.len());
                let key = &key_pool[k_idx];

                model.delete(step, cf, key);
                if let Some(ref h) = cf_handle {
                    db.delete_cf(h, key).unwrap();
                } else {
                    db.delete(key).unwrap();
                }
            }
            4 => {
                // DeleteRange
                let k1 = rng.gen_range(0, key_pool.len());
                let k2 = rng.gen_range(0, key_pool.len());
                let (lo, hi) = if k1 <= k2 { (k1, k2) } else { (k2, k1) };
                let start = &key_pool[lo];
                let end = &key_pool[hi];

                model.delete_range(step, cf, start, end);
                if let Some(ref h) = cf_handle {
                    db.delete_range_cf(h, start, end).unwrap();
                } else {
                    let h_default = db.cf_handle("default").expect("default CF exists");
                    db.delete_range_cf(&h_default, start, end).unwrap();
                }
            }
            5 => {
                // Merge (append)
                let k_idx = rng.gen_range(0, key_pool.len());
                let key = &key_pool[k_idx];
                let operand = b"+op";

                // RFC-0306 triage: capture the pre-merge read. If it is
                // already stale, the bug is a plain GET (no merge needed).
                let pre_expected = model.get(cf, key);
                let pre_actual = if let Some(ref h) = cf_handle {
                    db.get_cf(h, key).unwrap()
                } else {
                    db.get(key).unwrap()
                };
                if pre_actual != pre_expected {
                    eprintln!(
                        "=== PRE-MERGE STALE READ step={step} cf={cf} key={} actual={:?} expected={:?} ===",
                        String::from_utf8_lossy(key),
                        pre_actual.as_ref().map(|x| String::from_utf8_lossy(x)),
                        pre_expected.as_ref().map(|x| String::from_utf8_lossy(x))
                    );
                    let enc = db.debug_encode(cf, key);
                    eprintln!("  LAYER TRACE (encoded {}B):\n{}", enc.len(), db.debug_lookup_trace_encoded(&enc));
                }
                model.merge(step, cf, key, operand);
                if let Some(ref h) = cf_handle {
                    db.merge_cf(h, key, operand).unwrap();
                } else {
                    db.merge(key, operand).unwrap();
                }
                let post_expected = model.get(cf, key);
                let post_actual = if let Some(ref h) = cf_handle {
                    db.get_cf(h, key).unwrap()
                } else {
                    db.get(key).unwrap()
                };
                if post_actual != post_expected {
                    eprintln!("=== MISMATCH IMMEDIATELY AFTER MERGE step={step} cf={cf} key={} actual={:?} expected={:?} ===", String::from_utf8_lossy(key), post_actual.as_ref().map(|x| String::from_utf8_lossy(x)), post_expected.as_ref().map(|x| String::from_utf8_lossy(x)));
                    if let Some(hist) = model.history.get(&(cf.to_string(), key.to_vec())) {
                        eprintln!("  KEY HISTORY: {:#?}", hist);
                    }
                    // RFC-0306 triage: does an explicit drain heal the read?
                    // Heals => read-side race (value exists); stays => the
                    // write or a background compaction dropped it.
                    let runs = if cf == "default" {
                        db.sst_run_debug()
                    } else {
                        Vec::new()
                    };
                    eprintln!("  RUNS at mismatch: {runs:?}");
                    let _ = db.flush();
                    let _ = db.compact();
                    let healed = if cf == "default" {
                        db.get(key).unwrap()
                    } else {
                        db.get_cf(&db.cf_handle(cf).unwrap(), key).unwrap()
                    };
                    eprintln!(
                        "  AFTER flush+compact: {:?} (healed={})",
                        healed.as_ref().map(|x| String::from_utf8_lossy(x)),
                        healed == post_expected
                    );
                }
                assert_eq!(post_actual, post_expected, "Mismatch immediately after merge! step={step}");
            }
            6 => {
                // Point Get check
                let k_idx = rng.gen_range(0, key_pool.len());
                let key = &key_pool[k_idx];

                let expected = model.get(cf, key);
                let actual = if let Some(ref h) = cf_handle {
                    db.get_cf(h, key).unwrap()
                } else {
                    db.get(key).unwrap()
                };

                if actual != expected {
                    eprintln!("=== GET MISMATCH step={step} cf={cf} key={} actual={:?} expected={:?} ===", String::from_utf8_lossy(key), actual.as_ref().map(|x| String::from_utf8_lossy(x)), expected.as_ref().map(|x| String::from_utf8_lossy(x)));
                    if let Some(hist) = model.history.get(&(cf.to_string(), key.to_vec())) {
                        eprintln!("  KEY HISTORY: {:#?}", hist);
                    }
                }
                assert_eq!(
                    actual, expected,
                    "Differential mismatch on get! step={step}, cf={cf}, key={:?}",
                    String::from_utf8_lossy(key)
                );
            }
            7 => {
                // MultiGet check
                let count = rng.gen_range(1, 8);
                let mut test_keys = Vec::new();
                for _ in 0..count {
                    test_keys.push(&key_pool[rng.gen_range(0, key_pool.len())][..]);
                }

                let actuals = if let Some(ref h) = cf_handle {
                    db.multi_get_cf(test_keys.iter().map(|k| (h, *k)))
                } else {
                    db.multi_get(&test_keys)
                };

                assert_eq!(actuals.len(), test_keys.len());
                for (k, actual_res) in test_keys.iter().zip(actuals.iter()) {
                    let expected = model.get(cf, k);
                    let actual = actual_res.as_ref().unwrap().clone();
                    assert_eq!(
                        actual, expected,
                        "Differential mismatch on multi_get! step={step}, cf={cf}, key={:?}",
                        String::from_utf8_lossy(k)
                    );
                }
            }
            8 => {
                // Random Flush or Compact
                if rng.gen_bool(1, 2) {
                    db.flush().unwrap();
                } else {
                    db.compact().unwrap();
                }
                // Verify physical resource invariant (Barreira 2)
                db.assert_disk_inventory_invariant()
                    .expect("Disk inventory invariant must hold after flush/compact");
            }
            9 => {
                // Full Scan comparison (Forward or Reverse)
                if rng.gen_bool(1, 2) {
                    // Forward
                    let expected = model.scan_forward(cf);
                    let mut actual = Vec::new();
                    let mut iter = if let Some(ref h) = cf_handle {
                        db.iterator_cf(h, IteratorMode::Start).unwrap()
                    } else {
                        db.iterator(IteratorMode::Start).unwrap()
                    };
                    while iter.valid() {
                        actual.push((iter.key().to_vec(), iter.value().to_vec()));
                        iter.next();
                    }
                    if actual != expected {
                        eprintln!("=== MISMATCH FORWARD ITERATOR step={step} cf={cf} ===");
                        for (k, v) in &actual {
                            let exp_v = model.get(cf, k);
                            if exp_v.as_ref() != Some(v) {
                                eprintln!("KEY DIFF: {} actual={:?} expected={:?}", String::from_utf8_lossy(k), String::from_utf8_lossy(v), exp_v.as_ref().map(|x| String::from_utf8_lossy(x)));
                                if let Some(hist) = model.history.get(&(cf.to_string(), k.clone())) {
                                    eprintln!("  HISTORY: {:#?}", hist);
                                }
                            }
                        }
                        for (k, exp_v) in &expected {
                            let act_v = actual.iter().find(|(ak, _)| ak == k).map(|(_, v)| v);
                            if act_v != Some(exp_v) {
                                eprintln!("MISSING/DIFF KEY: {} actual={:?} expected={:?}", String::from_utf8_lossy(k), act_v.map(|x| String::from_utf8_lossy(x)), String::from_utf8_lossy(exp_v));
                                if let Some(hist) = model.history.get(&(cf.to_string(), k.clone())) {
                                    eprintln!("  HISTORY: {:#?}", hist);
                                }
                            }
                        }
                    }
                    assert_eq!(
                        actual, expected,
                        "Differential mismatch on forward iterator! step={step}, cf={cf}"
                    );
                } else {
                    // Reverse
                    let expected = model.scan_reverse(cf);
                    let mut actual = Vec::new();
                    let mut iter = if let Some(ref h) = cf_handle {
                        db.iterator_cf(h, IteratorMode::End).unwrap()
                    } else {
                        db.iterator(IteratorMode::End).unwrap()
                    };
                    while iter.valid() {
                        actual.push((iter.key().to_vec(), iter.value().to_vec()));
                        iter.next();
                    }
                    if actual != expected {
                        eprintln!("=== MISMATCH REVERSE ITERATOR step={step} cf={cf} ===");
                        for (k, v) in &actual {
                            let exp_v = model.get(cf, k);
                            if exp_v.as_ref() != Some(v) {
                                eprintln!("KEY DIFF: {} actual={:?} expected={:?}", String::from_utf8_lossy(k), String::from_utf8_lossy(v), exp_v.as_ref().map(|x| String::from_utf8_lossy(x)));
                                if let Some(hist) = model.history.get(&(cf.to_string(), k.clone())) {
                                    eprintln!("  HISTORY: {:#?}", hist);
                                }
                            }
                        }
                        for (k, exp_v) in &expected {
                            let act_v = actual.iter().find(|(ak, _)| ak == k).map(|(_, v)| v);
                            if act_v != Some(exp_v) {
                                eprintln!("MISSING/DIFF KEY: {} actual={:?} expected={:?}", String::from_utf8_lossy(k), act_v.map(|x| String::from_utf8_lossy(x)), String::from_utf8_lossy(exp_v));
                                if let Some(hist) = model.history.get(&(cf.to_string(), k.clone())) {
                                    eprintln!("  HISTORY: {:#?}", hist);
                                }
                            }
                        }
                    }
                    assert_eq!(
                        actual, expected,
                        "Differential mismatch on reverse iterator! step={step}, cf={cf}"
                    );
                }
            }
            _ => unreachable!(),
        }
    }

    // Final comprehensive cross-check across all CFs
    for cf in &all_cfs {
        let cf_handle = get_cf_handle(cf);

        // 1. Forward scan match
        let expected_fwd = model.scan_forward(cf);
        let mut actual_fwd = Vec::new();
        let mut iter = if let Some(ref h) = cf_handle {
            db.iterator_cf(h, IteratorMode::Start).unwrap()
        } else {
            db.iterator(IteratorMode::Start).unwrap()
        };
        while iter.valid() {
            actual_fwd.push((iter.key().to_vec(), iter.value().to_vec()));
            iter.next();
        }
        assert_eq!(actual_fwd, expected_fwd, "Final forward scan mismatch on cf={cf}");

        // 2. Reverse scan match
        let expected_rev = model.scan_reverse(cf);
        let mut actual_rev = Vec::new();
        let mut riter = if let Some(ref h) = cf_handle {
            db.iterator_cf(h, IteratorMode::End).unwrap()
        } else {
            db.iterator(IteratorMode::End).unwrap()
        };
        while riter.valid() {
            actual_rev.push((riter.key().to_vec(), riter.value().to_vec()));
            riter.next();
        }
        assert_eq!(actual_rev, expected_rev, "Final reverse scan mismatch on cf={cf}");

        // 3. Prefix scan match on "key_001"
        let prefix = b"key_001";
        let expected_prefix = model.scan_prefix(cf, prefix);
        let mut actual_prefix = Vec::new();
        let mut piter = if let Some(ref h) = cf_handle {
            db.prefix_iterator_cf(h, prefix).unwrap()
        } else {
            db.prefix_iterator(prefix).unwrap()
        };
        while piter.valid() {
            actual_prefix.push((piter.key().to_vec(), piter.value().to_vec()));
            piter.next();
        }
        assert_eq!(actual_prefix, expected_prefix, "Final prefix scan mismatch on cf={cf}");
    }

    // Final disk invariant check
    db.assert_disk_inventory_invariant()
        .expect("Final disk inventory invariant must hold");

    let _ = std::fs::remove_dir_all(&dir);
}
