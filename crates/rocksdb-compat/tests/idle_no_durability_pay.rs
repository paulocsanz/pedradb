//! Invariant: an idle, fully-published DB must not pay durability points.
//!
//! `rotate_wal_now`'s "settled" gate must be reachable in the default
//! configuration (`changelog_interval = 0`). The pay point
//! (`persist_manifest_durable`: SST fsyncs + MANIFEST rewrite) runs under
//! the Db write lock — if a settled DB keeps paying it on every WAL-rotate
//! tick, every reader's `try_read` fails for the fsync window and falls to
//! the O(candidates) `lookup_published` path (the measured read-decay
//! driver at 25M-100M scales).
//!
//! Oracle: hydrate (the slipstream shape: latched bulk + per-batch meta
//! cursor), settle, then hold a read-only window. `durable_pays_count()`
//! must stay flat across it — after a grace period that lets the idle
//! drain legitimately flush any real pending debt exactly once.

use std::time::{Duration, Instant};

use rocksdb_compat::{ColumnFamilyDescriptor, Options, WriteBatch, WriteOptions, DB};

fn key(i: u64) -> String {
    format!("route.svc-{:06}.{:08}", i / 1000, i % 1000)
}

fn value_pool() -> Vec<u8> {
    let mut pool = vec![0u8; 1 << 20];
    let mut x = 0x5EED_5EED_5EED_5EEDu64;
    for chunk in pool.chunks_mut(8) {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        chunk.copy_from_slice(&x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes()[..chunk.len()]);
    }
    pool
}

#[test]
fn idle_settled_db_never_pays_durability_points() {
    let n: u64 = std::env::var("IDLE_PAY_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(204_800);
    let per_batch: u64 = 1024;
    let pool = value_pool();
    let dir = std::env::temp_dir().join(format!("idle-pay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut opts = Options::new();
    opts.create_if_missing(true);
    opts.set_sync(false);
    let mut data_opts = Options::default();
    data_opts.set_write_buffer_size(256 << 20);
    let db = DB::open_cf_descriptors(
        &opts,
        &dir,
        [
            ColumnFamilyDescriptor::new("data", data_opts),
            ColumnFamilyDescriptor::new("meta", Options::default()),
        ],
    )
    .unwrap();
    let data = db.cf_handle("data").unwrap();
    let meta = db.cf_handle("meta").unwrap();
    let mut wo = WriteOptions::default();
    wo.set_sync(false);

    let mut i = 0u64;
    while i < n {
        let end = i + per_batch;
        let mut wb = WriteBatch::default();
        for j in i..end {
            let off = ((j * 7919) % (pool.len() as u64 - 200)) as usize;
            wb.put_cf(&data, key(j).as_bytes(), &pool[off..off + 200]);
        }
        wb.put_cf(&meta, b"cursor", end.to_le_bytes());
        db.write_opt_owned(wb, &wo).unwrap();
        i = end;
    }

    db.flush().unwrap();
    db.compact().unwrap();
    assert!(db.is_settled_sst_only(), "test requires a settled store");

    // Grace: let the idle drain legitimately pay any real pending debt
    // exactly once (unsynced SSTs from the last installs, manifest dirty).
    std::thread::sleep(Duration::from_millis(750));

    // Read-only window: hits on settled data. The WAL-rotate tick fires on
    // wall-clock (~5 ms), so a 3 s window sees hundreds of ticks regardless
    // of machine load.
    let pays0 = rocksdb_compat::probe_counters().durable_pays;
    let stores0 = rocksdb_compat::probe_counters().manifest_stores;
    let pubsv0 = rocksdb_compat::probe_counters().published_sv;
    let started = Instant::now();
    let mut gets = 0u64;
    let mut p = 0u64;
    while started.elapsed() < Duration::from_secs(3) {
        let k = key((p * 7_919) % n);
        let _ = std::hint::black_box(db.get_named("data", &k).unwrap());
        p = p.wrapping_add(1);
        gets += 1;
    }
    let pays = rocksdb_compat::probe_counters().durable_pays - pays0;
    let stores = rocksdb_compat::probe_counters().manifest_stores - stores0;
    let pubsv = rocksdb_compat::probe_counters().published_sv - pubsv0;
    eprintln!(
        "window: {gets} gets | pubsv {pubsv} ({:.1}%) | pays {pays} | stores {stores} | n {n}",
        pubsv as f64 / gets as f64 * 100.0
    );

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        pays, 0,
        "idle settled DB paid {pays} durability points during a {gets}-get read-only \
         window ({stores} manifest rewrites): the WAL-rotate 'settled' gate is \
         unreachable and every rotate re-fsyncs under the Db write lock"
    );
    assert_eq!(
        pubsv, 0,
        "{pubsv} of {gets} idle gets fell to the published-SV path: a host worker \
         kept queuing the Db write lock during a read-only window on a settled \
         store (WAL-rotate churn)"
    );
}
