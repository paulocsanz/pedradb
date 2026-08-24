//! Sub-bench scan profile v2 (RFC-0059 P2.3 follow-up): attribute the
//! SurrealDB scan gap between the plain DB iterator walk and the
//! layered path a SurrealDB read-tx actually drives —
//! `Transaction::raw_iterator_opt` (F183 overlay machinery, staged
//! empty) with `ro.set_snapshot(&tx.snapshot())`, seek + next rows.
//! Same seed/write pattern as sub-bench, same loop body both legs.
//!
//! Usage:
//!   cargo run --release -p rocksdb-compat --example sub_scan_profile2 [dir]
//! Env: SUB_SECONDS (8), SUB_RECORDS (1024).

#![forbid(unsafe_code)]

use rocksdb_compat::{ReadOptions, DB};
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

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pedra-sub-scan-profile2".into());
    let _ = std::fs::remove_dir_all(&dir);
    let n = records();
    let s = seconds();

    let db = DB::open_default(&dir).unwrap();
    for i in 0..n {
        db.put(key(i), val(i)).unwrap();
    }

    // 8s of point_write (same zipf leg) to grow versions, like sub-bench.
    let mut state = 0x2545F4914F6CDD1Du64;
    let v0 = val(0);
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(s);
    let mut wops = 0u64;
    while Instant::now() < deadline {
        db.put(key(zipf_key(&mut state, n)), v0.clone()).unwrap();
        wops += 1;
    }
    println!("point_write ops {wops}");

    let beg = key(0);
    let end = key(n.saturating_sub(1));
    let limit = 1000usize;

    // Warm both.
    let _ = leg_db(&db, &beg, &end, limit, 2.0);
    let _ = leg_tx(&db, &beg, &end, limit, 2.0);

    let r_db = leg_db(&db, &beg, &end, limit, s);
    println!("db  iterator rows/s {r_db:.0}");
    let r_tx = leg_tx(&db, &beg, &end, limit, s);
    println!("txn iterator rows/s {r_tx:.0}");
    println!("db/txn = {:.2}x", r_db / r_tx);
}

/// Plain DB raw iterator, SurrealDB loop body.
fn leg_db(db: &DB, beg: &[u8], end: &[u8], limit: usize, seconds: f64) -> f64 {
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let mut iter = db.raw_iterator_opt(ReadOptions::default());
        iter.seek(beg);
        let mut res: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        while iter.valid() {
            if res.len() < limit {
                if let (Some(k), Some(v)) = (iter.key(), iter.value()) {
                    if k >= beg && k < end {
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
        done += res.len() as u64;
    }
    done as f64 / start.elapsed().as_secs_f64()
}

/// Transaction raw iterator (F183 path; staged empty, like Surreal read-tx).
fn leg_tx(db: &DB, beg: &[u8], end: &[u8], limit: usize, seconds: f64) -> f64 {
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let tx = db.transaction();
        let mut ro = ReadOptions::default();
        ro.set_snapshot(&tx.snapshot());
        let mut iter = tx.raw_iterator_opt(ro);
        iter.seek(beg);
        let mut res: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        while iter.valid() {
            if res.len() < limit {
                if let (Some(k), Some(v)) = (iter.key(), iter.value()) {
                    if k >= beg && k < end {
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
        let _ = tx.rollback();
        done += res.len() as u64;
    }
    done as f64 / start.elapsed().as_secs_f64()
}
