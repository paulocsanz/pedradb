//! Isolated apply 2000×(pre+com) under flush policies. Not a harness number.
//!
//! cargo run --release -p rocksdb-parity-bench --example apply_flush_probe

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::{BatchOp, OpenOptions};
use std::time::Instant;

fn ukey(i: usize) -> Vec<u8> {
    format!("u/{i:06}").into_bytes()
}
fn mvcc(i: usize, ts: u64) -> Vec<u8> {
    let mut k = ukey(i);
    k.extend_from_slice(&ts.to_be_bytes());
    k
}
fn cf(cf: &str, key: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(cf.len() + 1 + key.len());
    v.extend_from_slice(cf.as_bytes());
    v.push(0);
    v.extend_from_slice(key);
    v
}

fn run_apply(db: &ConcurrentDb, iters: usize, batch: usize, payload: &[u8]) -> (f64, f64, f64) {
    let mut first = 0.0;
    let mut last = 0.0;
    let t_all = Instant::now();
    for it in 0..iters {
        let t = Instant::now();
        let mut pre = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            pre.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            pre.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload,
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
        let us = t.elapsed().as_secs_f64() * 1e6;
        if it < 100 {
            first += us;
        }
        if it >= iters.saturating_sub(100) {
            last += us;
        }
    }
    let wall = t_all.elapsed().as_secs_f64();
    (wall, first / 100.0, last / 100.0)
}

fn open(dir: &str, auto_flush: Option<usize>, defer: bool) -> ConcurrentDb {
    let _ = std::fs::remove_dir_all(dir);
    let db = ConcurrentDb::open_with(
        dir,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: auto_flush,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
    )
    .expect("open");
    if defer {
        db.set_defer_auto_compact(true);
    }
    db
}

fn report(label: &str, db: &ConcurrentDb, wall: f64, first: f64, last: f64, iters: usize) {
    let qps = iters as f64 / wall;
    let st = db.stats();
    let (imm, l0) = db.with_read(|d| (d.has_imm(), d.level_file_count(0)));
    println!(
        "{label:28} qps={qps:7.0} wall_ms={:.0} first100_us={first:.0} last100_us={last:.0} mem={} entries={} imm={imm} sst={} l0={l0} wal_syncs={}",
        wall * 1e3,
        st.mem_approx_bytes,
        st.mem_entries,
        st.sst_count,
        db.wal_sync_count()
    );
}

fn main() {
    let iters = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let batch = 32usize;
    let payload = vec![b'p'; 1000];
    println!("iters={iters} batch={batch} payload=1000");

    // A: no auto-flush (mem grows)
    {
        let db = open("/tmp/pedra-afp-none", None, true);
        let (w, f, l) = run_apply(&db, iters, batch, &payload);
        report("defer+no_flush", &db, w, f, l, iters);
        let _ = std::fs::remove_dir_all("/tmp/pedra-afp-none");
    }

    // B: 64 MiB, defer, no worker drain (stage at 64MiB, imm sits)
    {
        let db = open("/tmp/pedra-afp-64stage", Some(64 * 1024 * 1024), true);
        let (w, f, l) = run_apply(&db, iters, batch, &payload);
        report("defer+64MiB_stage_only", &db, w, f, l, iters);
        let _ = std::fs::remove_dir_all("/tmp/pedra-afp-64stage");
    }

    // C: 64 MiB, defer, drain every 16 iters (worker-like)
    {
        let db = open("/tmp/pedra-afp-64drain", Some(64 * 1024 * 1024), true);
        let mut first = 0.0;
        let mut last = 0.0;
        let t_all = Instant::now();
        for it in 0..iters {
            let t = Instant::now();
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
            if it % 16 == 0 {
                while db.drain_imm_once() {}
            }
            let us = t.elapsed().as_secs_f64() * 1e6;
            if it < 100 {
                first += us;
            }
            if it >= iters - 100 {
                last += us;
            }
        }
        report(
            "defer+64MiB_drain16",
            &db,
            t_all.elapsed().as_secs_f64(),
            first / 100.0,
            last / 100.0,
            iters,
        );
        let _ = std::fs::remove_dir_all("/tmp/pedra-afp-64drain");
    }

    // D: 4 MiB default, defer, drain every 16
    {
        let db = open("/tmp/pedra-afp-4drain", Some(4 * 1024 * 1024), true);
        let mut first = 0.0;
        let mut last = 0.0;
        let t_all = Instant::now();
        for it in 0..iters {
            let t = Instant::now();
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
            if it % 16 == 0 {
                while db.drain_imm_once() {}
            }
            let us = t.elapsed().as_secs_f64() * 1e6;
            if it < 100 {
                first += us;
            }
            if it >= iters - 100 {
                last += us;
            }
        }
        report(
            "defer+4MiB_drain16",
            &db,
            t_all.elapsed().as_secs_f64(),
            first / 100.0,
            last / 100.0,
            iters,
        );
        let _ = std::fs::remove_dir_all("/tmp/pedra-afp-4drain");
    }

    // E: 8 MiB, defer, drain every 16
    {
        let db = open("/tmp/pedra-afp-8drain", Some(8 * 1024 * 1024), true);
        let mut first = 0.0;
        let mut last = 0.0;
        let t_all = Instant::now();
        for it in 0..iters {
            let t = Instant::now();
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
            if it % 16 == 0 {
                while db.drain_imm_once() {}
            }
            let us = t.elapsed().as_secs_f64() * 1e6;
            if it < 100 {
                first += us;
            }
            if it >= iters - 100 {
                last += us;
            }
        }
        report(
            "defer+8MiB_drain16",
            &db,
            t_all.elapsed().as_secs_f64(),
            first / 100.0,
            last / 100.0,
            iters,
        );
        let _ = std::fs::remove_dir_all("/tmp/pedra-afp-8drain");
    }
}
