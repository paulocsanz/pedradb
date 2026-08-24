//! Sub-bench scan profile (RFC-0059 P2.3 follow-up): replicate the exact
//! SurrealDB scan pattern through the compat raw iterator —
//! `raw_iterator_opt(ro)` + `seek` + `next()` loop with per-row
//! `k >= beg && k < end` and two `to_vec` — to attribute where the row
//! cost lives (window refill fixed cost vs MVCC version walk).
//!
//! Legs: scan0 (fresh DB, no overwrites) → point_write 8s → scan1
//! (version-heavy). Prints rows/s per leg plus layer stats.
//!
//! Usage:
//!   cargo run --release -p rocksdb-compat --example sub_scan_profile [dir]
//! Env: SUB_SECONDS (8), SUB_RECORDS (1024).

#![forbid(unsafe_code)]

use rocksdb_compat::{Options, ReadOptions, DB};
use std::time::{Duration, Instant};

fn records() -> usize {
    std::env::var("SUB_RECORDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024)
}

fn seconds() -> f64 {
    std::env::var("SUB_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8.0)
}

fn key(i: usize) -> Vec<u8> {
    format!("bench/key/{i:08}").into_bytes()
}

fn val(i: usize) -> Vec<u8> {
    format!("{{\"id\":{i},\"name\":\"record-{i}\",\"tags\":[\"a\",\"b\"],\"count\":{i}}}")
        .into_bytes()
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn zipf_key(state: &mut u64, n: usize) -> usize {
    let a = (xorshift(state) % n as u64) as f64;
    let b = (xorshift(state) % n as u64) as f64;
    ((a * b).sqrt() as u64 % n as u64) as usize
}

/// SurrealDB `scan` loop, byte-for-byte (kvs/rocksdb/mod.rs::scan).
fn scan_leg(db: &DB, n: usize, seconds: f64, limit: usize) -> f64 {
    let beg = key(0);
    let end = key(n.saturating_sub(1));
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        // Latest-sequence raw iterator: same window machinery the pinned
        // variant drives (ensure_snapshot_readable + scan_at_raw); the OCC
        // pin itself is not what we are attributing here.
        let mut iter = db.raw_iterator_opt(ReadOptions::default());
        iter.seek(&beg);
        let mut res: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        while iter.valid() {
            if res.len() < limit {
                let (k, v) = (iter.key(), iter.value());
                if let (Some(k), Some(v)) = (k, v) {
                    if k.as_ref() >= beg.as_slice() && k.as_ref() < end.as_slice() {
                        res.push((k.to_vec(), v.to_vec()));
                        iter.next();
                        continue;
                    }
                }
            }
            break;
        }
        if res.is_empty() {
            panic!("scan empty");
        }
        drop(iter);
        done += res.len() as u64;
    }
    done as f64 / start.elapsed().as_secs_f64()
}

fn write_leg(db: &DB, n: usize, seconds: f64) -> u64 {
    let mut state = 0x2545F4914F6CDD1Du64;
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    let v0 = val(0);
    while Instant::now() < deadline {
        let k = key(zipf_key(&mut state, n));
        db.put(k, v0.clone()).unwrap();
        done += 1;
    }
    done
}

fn stats(db: &DB, tag: &str) {
    let get = |name: &str| {
        db.property_int_value(name)
            .ok()
            .flatten()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into())
    };
    println!(
        "[{tag}] est-keys={} total-sst={} all-mem={} pend-compaction={}",
        get(rocksdb_compat::properties::ESTIMATE_NUM_KEYS),
        get(rocksdb_compat::properties::TOTAL_SST_FILES_SIZE),
        get(rocksdb_compat::properties::CUR_SIZE_ALL_MEM_TABLES),
        get(rocksdb_compat::properties::COMPACTION_PENDING),
    );
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pedra-sub-scan-profile".into());
    let _ = std::fs::remove_dir_all(&dir);
    let n = records();
    let s = seconds();

    let db = DB::open_default(&dir).unwrap();
    let opts = Options::default();
    let _ = opts;
    for i in 0..n {
        db.put(key(i), val(i)).unwrap();
    }
    stats(&db, "seeded");

    let warm = 2.0;
    let _ = scan_leg(&db, n, warm, 1000);
    let r0 = scan_leg(&db, n, s, 1000);
    println!("scan0 (fresh)     rows/s {r0:.0}");
    stats(&db, "scan0");

    let w = write_leg(&db, n, s);
    println!("point_write       ops   {w}");
    stats(&db, "wrote");

    let _ = scan_leg(&db, n, warm, 1000);
    let r1 = scan_leg(&db, n, s, 1000);
    println!("scan1 (versions)  rows/s {r1:.0}");
    stats(&db, "scan1");
    println!(
        "scan0/scan1 = {:.2}x (1.0 → cost is fixed per-row; <1 → version walk dominates)",
        r0 / r1
    );
}
