//! SurrealDB kv-rocksdb substitution bench (RFC-0059).
//!
//! Identical SurrealDB `Datastore`/`Transaction` code paths on both sides;
//! the storage crate is swapped by `[patch.crates-io]` in the workspace
//! manifest. A = surrealdb-core with Pedra (`rocksdb` shim), B =
//! surrealdb-core with real RocksDB (crates.io 0.21).
//!
//! Workloads (SurrealDB kvs ops, not raw KV): point get/set, scan, and a
//! document-shaped txn (get + set + commit). JSON to stdout for the ratio
//! table on the Linux gate.

use std::time::{Duration, Instant};

use surrealdb_core::kvs::{Datastore, LockType, TransactionType};

struct Leg {
    name: &'static str,
    qps: f64,
}

fn ops() -> u32 {
    std::env::var("SUB_BENCH_OPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

fn records() -> u32 {
    std::env::var("SUB_BENCH_RECORDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024)
}

fn key(i: u32) -> Vec<u8> {
    format!("bench/key/{i:08}").into_bytes()
}

fn val(i: u32) -> Vec<u8> {
    // ~120B document-ish payload, deterministic.
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

fn zipf_key(state: &mut u64, n: u32) -> u32 {
    // Cheap zipf-ish skew: square of uniform biases small keys.
    let a = (xorshift(state) % n as u64) as f64;
    let b = (xorshift(state) % n as u64) as f64;
    ((a * b).sqrt() as u64 % n as u64) as u32
}

async fn seed(ds: &Datastore) {
    let n = records();
    for i in 0..n {
        let mut tx = ds
            .transaction(TransactionType::Write, LockType::Optimistic)
            .await
            .unwrap();
        tx.set(key(i), val(i)).await.unwrap();
        tx.commit().await.unwrap();
    }
}

async fn leg_point_read(ds: &Datastore, seconds: f64) -> f64 {
    let n = records();
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let k = key(zipf_key(&mut state, n));
        let mut tx = ds
            .transaction(TransactionType::Read, LockType::Optimistic)
            .await
            .unwrap();
        let v = tx.get(k).await.unwrap();
        if v.is_none() {
            panic!("seed miss");
        }
        tx.cancel().await.unwrap();
        done += 1;
    }
    done as f64 / start.elapsed().as_secs_f64()
}

async fn leg_point_write(ds: &Datastore, seconds: f64) -> f64 {
    let n = records();
    let mut state = 0x2545F4914F6CDD1Du64;
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let k = key(zipf_key(&mut state, n));
        let mut tx = ds
            .transaction(TransactionType::Write, LockType::Optimistic)
            .await
            .unwrap();
        tx.set(k, val(0)).await.unwrap();
        tx.commit().await.unwrap();
        done += 1;
    }
    done as f64 / start.elapsed().as_secs_f64()
}

async fn leg_scan(ds: &Datastore, seconds: f64) -> f64 {
    let n = records();
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let mut tx = ds
            .transaction(TransactionType::Read, LockType::Optimistic)
            .await
            .unwrap();
        let rows = tx
            .scan(key(0)..key(n.saturating_sub(1)), 1000)
            .await
            .unwrap();
        if rows.is_empty() {
            panic!("scan empty");
        }
        tx.cancel().await.unwrap();
        done += rows.len() as u64;
    }
    done as f64 / start.elapsed().as_secs_f64()
}

async fn leg_doc_txn(ds: &Datastore, seconds: f64) -> f64 {
    // SurrealDB's put (insert-if-absent) + set + get-commit read-modify-write
    // shape — the OCC validation path.
    let n = records();
    let mut state = 0x853C49E6748FEA9Bu64;
    let mut done = 0u64;
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let i = zipf_key(&mut state, n);
        let mut tx = ds
            .transaction(TransactionType::Write, LockType::Optimistic)
            .await
            .unwrap();
        let cur = tx.get(key(i)).await.unwrap();
        let v = match cur {
            Some(v) => {
                let mut s = String::from_utf8(v).unwrap();
                s.push('!');
                s
            }
            None => String::from("new"),
        };
        tx.set(key(i), v.into_bytes()).await.unwrap();
        tx.commit().await.unwrap();
        done += 1;
    }
    done as f64 / start.elapsed().as_secs_f64()
}

#[tokio::main]
async fn main() {
    let dir = std::env::var("SUB_BENCH_DIR").unwrap_or_else(|_| {
        let d = std::env::temp_dir().join(format!("sub-bench-{}", std::process::id()));
        d.to_string_lossy().into_owned()
    });
    let _ = std::fs::remove_dir_all(&dir);
    let seconds = std::env::var("SUB_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8.0);
    let warmup = std::env::var("SUB_BENCH_WARMUP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0);

    println!("sub-bench ops={} records={} dir={}", ops(), records(), dir);
    let ds = Datastore::new(&format!("file:{dir}")).await.unwrap();

    let t0 = Instant::now();
    seed(&ds).await;
    println!("seed {} records in {:.1}s", records(), t0.elapsed().as_secs_f64());

    // Warmup each path once, untimed.
    let _ = leg_point_read(&ds, warmup).await;
    let _ = leg_point_write(&ds, warmup).await;

    let legs = vec![
        Leg { name: "point_read", qps: leg_point_read(&ds, seconds).await },
        Leg { name: "point_write", qps: leg_point_write(&ds, seconds).await },
        Leg { name: "scan", qps: leg_scan(&ds, seconds).await },
        Leg { name: "doc_txn", qps: leg_doc_txn(&ds, seconds).await },
    ];
    for l in &legs {
        println!("leg {} qps {:.0}", l.name, l.qps);
    }
    let out = serde_json::json!({
        "engine": std::env::var("SUB_BENCH_ENGINE").unwrap_or_else(|_| "unknown".into()),
        "ops": ops(),
        "records": records(),
        "seconds": seconds,
        "legs": legs.iter().map(|l| serde_json::json!({"name": l.name, "qps": l.qps})).collect::<Vec<_>>(),
    });
    println!("SUB_BENCH_JSON {}", out);
    let _ = std::fs::remove_dir_all(&dir);
}
