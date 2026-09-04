//! Lab split of one apply-shaped write (64 CF ops: 32×1KB + 32 small).
//! Not a harness number — isolates CPU vs fdatasync vs ConcurrentDb.
//!
//! Usage: cargo run --release -p rocksdb-parity-bench --example apply_profile [iters]

use pedradb_core::batch::{encode_ops, WriteOp};
use pedradb_core::change_feed::ChangeEntry;
use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::memtable::MemTable;
use pedradb_core::{BatchOp, Db, OpenOptions, WriteOptions};
use std::collections::BTreeMap;
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

fn main() {
    let iters: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let payload = vec![b'p'; 1000];
    let batch = 32usize;

    // 1) BTreeMap insert of unique MVCC keys (growing table, like apply default+write).
    let mut map: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let t = Instant::now();
    for it in 0..iters {
        for i in 0..batch {
            map.insert(cf("default", &mvcc(i, it as u64 + 1)), payload.clone());
            map.insert(cf("write", &mvcc(i, it as u64 + 1)), b"c".to_vec());
        }
    }
    let btree_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "btree_unique keys={} iters={iters} wall_ms={btree_ms:.1} per_iter_us={:.1}",
        map.len(),
        btree_ms * 1000.0 / iters as f64
    );

    // 2) MemTable put of the same keys.
    let mut mt = MemTable::new();
    let t = Instant::now();
    let mut seq = 1u64;
    for it in 0..iters {
        for i in 0..batch {
            mt.put(cf("default", &mvcc(i, it as u64 + 1)), seq, payload.clone());
            seq += 1;
            mt.put(cf("write", &mvcc(i, it as u64 + 1)), seq, b"c".as_slice());
            seq += 1;
        }
    }
    let mem_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "memtable_put entries={} wall_ms={mem_ms:.1} per_iter_us={:.1}",
        mt.len(),
        mem_ms * 1000.0 / iters as f64
    );

    // 3) WAL encode only (64 ops, 32×1KB).
    let mut ops = Vec::new();
    seq = 1;
    for i in 0..batch {
        ops.push(WriteOp::put(
            seq,
            cf("default", &mvcc(i, 1)),
            payload.clone(),
        ));
        seq += 1;
        ops.push(WriteOp::put(seq, cf("write", &mvcc(i, 1)), b"c".as_slice()));
        seq += 1;
    }
    let t = Instant::now();
    let mut buf = Vec::new();
    for _ in 0..iters {
        buf.clear();
        encode_ops(&ops, &mut buf);
    }
    let enc_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "encode_ops bytes={} wall_ms={enc_ms:.1} per_iter_us={:.1}",
        buf.len(),
        enc_ms * 1000.0 / iters as f64
    );

    // 4) ConcurrentDb apply_batch of the apply-pre shape (64 ops, sync).
    let dir = "/tmp/pedra-apply-profile";
    let _ = std::fs::remove_dir_all(dir);
    let db = ConcurrentDb::open(dir).expect("open");
    let t = Instant::now();
    for it in 0..iters {
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            batch_ops.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload.as_slice(),
            ));
        }
        db.apply_batch(batch_ops).expect("pre");
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(
                cf("write", &mvcc(i, it as u64 + 1)),
                b"c".as_slice(),
            ));
            batch_ops.push(BatchOp::delete(cf("lock", &ukey(i))));
        }
        db.apply_batch(batch_ops).expect("com");
    }
    let db_ms = t.elapsed().as_secs_f64() * 1e3;
    let syncs = db.wal_sync_count();
    println!(
        "concurrent_apply pre+com iters={iters} wall_ms={db_ms:.1} per_iter_us={:.1} qps={:.0} wal_syncs={syncs}",
        db_ms * 1000.0 / iters as f64,
        iters as f64 / (db_ms / 1e3)
    );

    // 5) Same batches with no_sync to subtract fd (G1 still on default path above).
    let dir2 = "/tmp/pedra-apply-profile-nosync";
    let _ = std::fs::remove_dir_all(dir2);
    let db2 = ConcurrentDb::open_with(
        dir2,
        pedradb_core::OpenOptions {
            sync: false,
            ..pedradb_core::OpenOptions::default()
        },
    )
    .expect("open nosync");
    let t = Instant::now();
    for it in 0..iters {
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            batch_ops.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload.as_slice(),
            ));
        }
        db2.apply_batch(batch_ops).expect("pre");
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(
                cf("write", &mvcc(i, it as u64 + 1)),
                b"c".as_slice(),
            ));
            batch_ops.push(BatchOp::delete(cf("lock", &ukey(i))));
        }
        db2.apply_batch(batch_ops).expect("com");
    }
    let nosync_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "concurrent_apply_nosync per_iter_us={:.1} qps={:.0} (CPU only; not product path)",
        nosync_ms * 1000.0 / iters as f64,
        iters as f64 / (nosync_ms / 1e3)
    );

    // 6) Isolated ChangeEntry materialization (what interval=0 still does).
    let t = Instant::now();
    let mut feed = Vec::new();
    for _ in 0..iters {
        feed.clear();
        for op in &ops {
            feed.push(ChangeEntry::from_write_op(op));
        }
    }
    let feed_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "change_entry_from_ops n={} wall_ms={feed_ms:.1} per_iter_us={:.1}",
        ops.len(),
        feed_ms * 1000.0 / iters as f64
    );

    // 7) Single-thread Db apply (no ConcurrentDb / write-group).
    let dir3 = "/tmp/pedra-apply-profile-db";
    let _ = std::fs::remove_dir_all(dir3);
    let mut single = Db::open_with(
        dir3,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
    )
    .expect("db");
    let t = Instant::now();
    for it in 0..iters {
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            batch_ops.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload.as_slice(),
            ));
        }
        single
            .apply_batch_with(batch_ops, WriteOptions::sync())
            .expect("pre");
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(
                cf("write", &mvcc(i, it as u64 + 1)),
                b"c".as_slice(),
            ));
            batch_ops.push(BatchOp::delete(cf("lock", &ukey(i))));
        }
        single
            .apply_batch_with(batch_ops, WriteOptions::sync())
            .expect("com");
    }
    let db1_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "db_apply_sync per_iter_us={:.1} qps={:.0} wal_syncs={}",
        db1_ms * 1000.0 / iters as f64,
        iters as f64 / (db1_ms / 1e3),
        single.wal_sync_count()
    );

    let dir4 = "/tmp/pedra-apply-profile-db-nosync";
    let _ = std::fs::remove_dir_all(dir4);
    let mut single2 = Db::open_with(
        dir4,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: false,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
    )
    .expect("db");
    let t = Instant::now();
    for it in 0..iters {
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            batch_ops.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload.as_slice(),
            ));
        }
        single2
            .apply_batch_with(batch_ops, WriteOptions::no_sync())
            .expect("pre");
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(
                cf("write", &mvcc(i, it as u64 + 1)),
                b"c".as_slice(),
            ));
            batch_ops.push(BatchOp::delete(cf("lock", &ukey(i))));
        }
        single2
            .apply_batch_with(batch_ops, WriteOptions::no_sync())
            .expect("com");
    }
    let db1n_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "db_apply_nosync per_iter_us={:.1} qps={:.0}",
        db1n_ms * 1000.0 / iters as f64,
        iters as f64 / (db1n_ms / 1e3)
    );

    // 8) BatchOp construction only (CF encode + Bytes copy).
    let t = Instant::now();
    for it in 0..iters {
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(cf("lock", &ukey(i)), b"l".as_slice()));
            batch_ops.push(BatchOp::put(
                cf("default", &mvcc(i, it as u64 + 1)),
                payload.as_slice(),
            ));
        }
        std::hint::black_box(batch_ops);
        let mut batch_ops = Vec::with_capacity(batch * 2);
        for i in 0..batch {
            batch_ops.push(BatchOp::put(
                cf("write", &mvcc(i, it as u64 + 1)),
                b"c".as_slice(),
            ));
            batch_ops.push(BatchOp::delete(cf("lock", &ukey(i))));
        }
        std::hint::black_box(batch_ops);
    }
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "build_batchops per_iter_us={:.1}",
        build_ms * 1000.0 / iters as f64
    );

    // 9) 4 clients × iters/4 apply-ops, product path (sync), catch-up as env.
    let clients = 4usize;
    let per = (iters / clients).max(1);
    let dir5 = "/tmp/pedra-apply-profile-mc";
    let _ = std::fs::remove_dir_all(dir5);
    let dbmc = ConcurrentDb::open(dir5).expect("mc");
    let window = dbmc.write_group_catchup_window();
    let payload_mc = payload.clone();
    let t = Instant::now();
    std::thread::scope(|s| {
        for c in 0..clients {
            let db = &dbmc;
            let payload = &payload_mc;
            s.spawn(move || {
                for it in 0..per {
                    let ts = (c * per + it) as u64 + 1;
                    let mut pre = Vec::with_capacity(batch * 2);
                    for i in 0..batch {
                        pre.push(BatchOp::put(cf("lock", &ukey(i + c * 32)), b"l".as_slice()));
                        pre.push(BatchOp::put(
                            cf("default", &mvcc(i + c * 32, ts)),
                            payload.as_slice(),
                        ));
                    }
                    db.apply_batch(pre).expect("pre");
                    let mut com = Vec::with_capacity(batch * 2);
                    for i in 0..batch {
                        com.push(BatchOp::put(
                            cf("write", &mvcc(i + c * 32, ts)),
                            b"c".as_slice(),
                        ));
                        com.push(BatchOp::delete(cf("lock", &ukey(i + c * 32))));
                    }
                    db.apply_batch(com).expect("com");
                }
            });
        }
    });
    let mc_ms = t.elapsed().as_secs_f64() * 1e3;
    let total = clients * per;
    let (sub, queued, groups, gops) = dbmc.write_group_stats();
    println!(
        "mc4_apply catchup_us={} ops={total} wall_ms={mc_ms:.1} qps={:.0} per_op_us={:.1} wal_syncs={} avg_group={:.2} queued={queued}/{sub}",
        window.as_micros(),
        total as f64 / (mc_ms / 1e3),
        mc_ms * 1000.0 / total as f64,
        dbmc.wal_sync_count(),
        gops as f64 / groups.max(1) as f64,
    );

    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
    let _ = std::fs::remove_dir_all(dir3);
    let _ = std::fs::remove_dir_all(dir4);
    let _ = std::fs::remove_dir_all(dir5);
}
