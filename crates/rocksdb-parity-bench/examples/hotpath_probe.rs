//! CPU-only hot-path probe (no per-op `Instant`).
//!
//! The official YCSB harness timestamps every op; on GET that timer is ~61% of
//! `sample` hits, so it cannot tell us where the engine is slow. This probe
//! pre-materialises keys, pins one phase, and reports one wall time for the
//! whole window — so `sample` / `xctrace` / a TCG insn plugin see engine work.
//!
//! Requires `PEDRA_PARITY_ASYNC=1` (WAL `write()` at 64 KiB, no `fdatasync`).
//! Not G1, not an official ratio. Lab-only.
//!
//! Usage:
//!   PEDRA_PARITY_ASYNC=1 cargo run --release -p rocksdb-parity-bench \
//!     --example hotpath_probe -- [dir]
//! Env: PHASE=get|set|blob|pipe|all  OPS  RECORDS  SECONDS  PAYLOAD  BLOB  BATCH

use rocksdb_parity_bench::engines::CompatEngine;
use rocksdb_parity_bench::Engine;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn kkey(i: usize) -> Vec<u8> {
    format!("k/{i:06}").into_bytes()
}

fn report(phase: &str, ops: usize, payload: usize, t: std::time::Duration) {
    let ns = t.as_nanos() as f64;
    let qps = if t.as_secs_f64() > 0.0 {
        ops as f64 / t.as_secs_f64()
    } else {
        0.0
    };
    println!(
        r#"{{"phase":"{phase}","ops":{ops},"payload":{payload},"ns":{},"ns_per_op":{:.1},"qps":{:.1}}}"#,
        t.as_nanos(),
        ns / ops.max(1) as f64,
        qps
    );
}

fn run_ops<F: FnMut(usize)>(
    seconds: usize,
    ops: usize,
    mut body: F,
) -> (usize, std::time::Duration) {
    if seconds > 0 {
        let deadline = Instant::now() + std::time::Duration::from_secs(seconds as u64);
        let mut n = 0usize;
        let t0 = Instant::now();
        while Instant::now() < deadline {
            body(n);
            n += 1;
        }
        (n, t0.elapsed())
    } else {
        let t0 = Instant::now();
        for i in 0..ops {
            body(i);
        }
        (ops, t0.elapsed())
    }
}

fn main() {
    if std::env::var("PEDRA_PARITY_ASYNC").as_deref() != Ok("1") {
        eprintln!(
            "hotpath_probe: set PEDRA_PARITY_ASYNC=1 (CPU column; not G1). refusing to mix fdatasync into a profile window."
        );
        std::process::exit(2);
    }

    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pedra-hotpath-probe".into());
    let records = env_usize("RECORDS", 1024).max(64);
    let ops = env_usize("OPS", 2_000_000).max(1);
    let seconds = env_usize("SECONDS", 0);
    let payload_len = env_usize("PAYLOAD", 1000);
    let blob_len = env_usize("BLOB", 16 * 1024);
    let batch = env_usize("BATCH", 32).clamp(2, 512);
    let phase = std::env::var("PHASE").unwrap_or_else(|_| "all".into());

    let _ = std::fs::remove_dir_all(&dir);
    let e = CompatEngine::open(Path::new(&dir));

    let keys: Vec<Vec<u8>> = (0..records + batch + 8).map(kkey).collect();
    let yval = vec![b'k'; payload_len];
    let blob = vec![b'B'; blob_len];

    for i in 0..records {
        assert!(e.put(&keys[i], &yval), "seed {i}");
    }
    for i in 0..records {
        let _ = e.get_probe(&keys[i]);
    }

    let want = |name: &str| phase == "all" || phase == name;
    println!(
        r#"{{"probe":"hotpath","engine":"compat","async":true,"records":{records},"ops":{ops},"seconds":{seconds},"phase":"{phase}"}}"#
    );

    if want("get") {
        let (n, t) = run_ops(seconds, ops, |i| {
            let u = i % records;
            black_box(e.get_probe(&keys[u]).ok());
        });
        report("get", n, payload_len, t);
    }

    if want("set") {
        let (n, t) = run_ops(seconds, ops, |i| {
            let u = i % records;
            black_box(e.put(&keys[u], &yval));
        });
        report("set", n, payload_len, t);
    }

    if want("blob") {
        let blob_n = records.min(256);
        for i in 0..blob_n {
            let _ = e.put(&keys[i], &blob);
        }
        let (n, t) = run_ops(seconds, ops / 8, |i| {
            let u = i % blob_n;
            black_box(e.put(&keys[u], &blob));
        });
        report("blob", n, blob_len, t);
    }

    if want("pipe") {
        let rounds = if seconds > 0 { 16_384 } else { ops.min(20_000) };
        let mut pipe: Vec<Vec<Vec<u8>>> = Vec::with_capacity(rounds);
        for r in 0..rounds {
            let mut ks = Vec::with_capacity(batch);
            for b in 0..batch {
                ks.push(keys[(r + b) % records].clone());
            }
            pipe.push(ks);
        }
        let (n, t) = run_ops(seconds, rounds, |i| {
            let keys = &pipe[i % pipe.len()];
            black_box(e.batch_put_same("default", keys, &yval));
        });
        report("pipe", n, payload_len, t);
    }
}
