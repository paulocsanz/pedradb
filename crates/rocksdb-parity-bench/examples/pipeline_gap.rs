//! RFC-0055 P0 — do the three missing Rocks write-pipeline knobs show up
//! on shapes we actually ship?
//!
//! Prints two tables:
//!   1. 1 / 12 / 50 async writers (bypass). Official ycsb/deps/kvrocks are 1c.
//!   2. apply 4 MiB vs 64 MiB vs no-flush (the product default vs Rocks vs
//!      the official bench pin).
//!
//! ```text
//! PEDRA_WRITE_PHASE_STATS=1 cargo run --release -p rocksdb-parity-bench \
//!     --example pipeline_gap
//! ```

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::{BatchOp, OpenOptions};
use std::sync::{Arc, Barrier};
use std::time::Instant;

fn ukey(i: usize) -> Vec<u8> {
    format!("u/{i:06}").into_bytes()
}
fn mvcc(i: usize, ts: u64) -> Vec<u8> {
    let mut k = ukey(i);
    k.extend_from_slice(&ts.to_be_bytes());
    k
}
fn cf(name: &str, key: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(name.len() + 1 + key.len());
    v.extend_from_slice(name.as_bytes());
    v.push(0);
    v.extend_from_slice(key);
    v
}

fn open(tag: &str, flush: Option<usize>, sync: bool) -> ConcurrentDb {
    let dir = std::env::temp_dir().join(format!("pedra-pgap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = ConcurrentDb::open_with(
        &dir,
        OpenOptions {
            auto_flush_bytes: flush,
            sync,
            ..OpenOptions::default()
        },
    )
    .expect("open");
    db.set_default_write_sync(sync);
    db
}

fn writers(n: usize, ops: usize) {
    // 1 GiB buffer: flush-free, so this table is lock/WAL/mem only.
    let db = open(&format!("w{n}"), Some(1 << 30), false);
    let val = Arc::new(vec![b'k'; 1024]);
    let barrier = Arc::new(Barrier::new(n));
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for t in 0..n {
            let db = &db;
            let val = &val;
            let barrier = &barrier;
            s.spawn(move || {
                barrier.wait();
                for i in 0..ops {
                    let k = format!("k/{t}/{i:06}");
                    db.put(k.as_bytes(), val.as_slice()).expect("put");
                }
            });
        }
    });
    let wall = t0.elapsed().as_secs_f64();
    let total = (n * ops) as f64;
    println!(
        "  writers={n:>2} ops/thread={ops} qps={:.0} wall_ms={:.0}",
        total / wall,
        wall * 1e3
    );
}

fn apply(label: &str, flush: Option<usize>, iters: usize) {
    let db = open(label, flush, false);
    let payload = vec![b'p'; 1000];
    let batch = 32usize;
    let t0 = Instant::now();
    for it in 0..iters {
        let mut pre = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            pre.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            pre.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload.as_slice(),
            ));
        }
        db.apply_batch(pre).expect("pre");
        let mut com = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            com.push(BatchOp::put(
                cf("write", &mvcc(i, it as u64 + 1)),
                b"c".as_slice(),
            ));
            com.push(BatchOp::delete(cf("lock", &ukey(i))));
        }
        db.apply_batch(com).expect("com");
        if flush.is_some() && it % 16 == 0 {
            while db.drain_imm_once() {}
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    let st = db.stats();
    println!(
        "  {label:12} qps={:.0} wall_ms={:.0} sst={} mem={}",
        iters as f64 / wall,
        wall * 1e3,
        st.sst_count,
        st.mem_approx_bytes
    );
}

fn main() {
    let ops: usize = std::env::var("PGAP_OPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4_000);
    let apply_n: usize = std::env::var("PGAP_APPLY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);

    println!("RFC-0055 pipeline_gap (async WAL, flush-free writers)");
    println!("table 1 — concurrent writers (official 1c never hits this)");
    for n in [1usize, 12, 50] {
        writers(n, ops);
    }

    println!("table 2 — apply drain 4MiB vs 64MiB vs none (product vs Rocks vs bench pin)");
    apply("none", None, apply_n);
    apply("4MiB", Some(4 * 1024 * 1024), apply_n);
    apply("64MiB", Some(64 * 1024 * 1024), apply_n);
}
