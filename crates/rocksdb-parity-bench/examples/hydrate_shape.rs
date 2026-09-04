//! Sorted-ingest hydrate shape (slipstream apply batches).
//!
//! ```text
//! cargo run --release -p rocksdb-parity-bench --example hydrate_shape
//! ROCKS=1 cargo run --release -p rocksdb-parity-bench --features real --example hydrate_shape
//! ```

use rocksdb_compat::{Options, DB};
use std::time::Instant;

fn key(i: u64) -> Vec<u8> {
    format!("data\0route.svc-{:06}.{:08}", i / 1000, i % 1000).into_bytes()
}

fn pedra(n: u64, payload: usize) {
    let dir = std::env::temp_dir().join(format!("hydrate-pedra-{}-{}", n, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut opts = Options::new();
    opts.create_if_missing(true);
    opts.set_sync(false);
    opts.set_write_buffer_size(256 * 1024 * 1024);
    let db = DB::open_cf(&opts, &dir, &["data", "meta"]).expect("open");
    let val = vec![b'v'; payload];
    let t0 = Instant::now();
    let mut i = 0u64;
    while i < n {
        let end = (i + 1024).min(n);
        let mut puts = Vec::with_capacity((end - i) as usize + 1);
        for j in i..end {
            puts.push(("data", key(j), val.clone()));
        }
        puts.push(("meta", b"meta\0cursor".to_vec(), i.to_le_bytes().to_vec()));
        db.write_cf_owned(puts, Vec::new()).unwrap();
        i = end;
    }
    let s = t0.elapsed().as_secs_f64();
    db.flush().ok();
    println!(
        "pedra  n={n} pay={payload} hydrate={s:.3}s ({:.2} M/s)",
        n as f64 / s / 1e6
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "real")]
fn rocks(n: u64, payload: usize) {
    use rocksdb::{ColumnFamilyDescriptor, Options as Ro, WriteBatch, WriteOptions, DB as RDB};
    let dir = std::env::temp_dir().join(format!("hydrate-rocks-{}-{}", n, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut opts = Ro::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    opts.set_write_buffer_size(256 * 1024 * 1024);
    let cfs = ["data", "meta"].map(|n| ColumnFamilyDescriptor::new(n, Ro::default()));
    let db = RDB::open_cf_descriptors(&opts, &dir, cfs).expect("rocks open");
    let data = db.cf_handle("data").unwrap();
    let meta = db.cf_handle("meta").unwrap();
    let mut wo = WriteOptions::default();
    wo.set_sync(false);
    let val = vec![b'v'; payload];
    let t0 = Instant::now();
    let mut i = 0u64;
    while i < n {
        let end = (i + 1024).min(n);
        let mut wb = WriteBatch::default();
        for j in i..end {
            wb.put_cf(&data, key(j), &val);
        }
        wb.put_cf(&meta, b"cursor", i.to_le_bytes());
        db.write_opt(wb, &wo).unwrap();
        i = end;
    }
    let s = t0.elapsed().as_secs_f64();
    db.flush().ok();
    println!(
        "rocks  n={n} pay={payload} hydrate={s:.3}s ({:.2} M/s)",
        n as f64 / s / 1e6
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let ns: Vec<u64> = std::env::var("HYDRATE_N")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
        .filter(|v: &Vec<u64>| !v.is_empty())
        .unwrap_or_else(|| vec![1_000_000, 4_000_000]);
    let payload = std::env::var("HYDRATE_PAY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    for n in ns {
        pedra(n, payload);
        #[cfg(feature = "real")]
        rocks(n, payload);
    }
}
