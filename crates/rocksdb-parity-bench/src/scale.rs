//! Sorted-ingest scale harness: hydrate + settle + point/prefix/lookup.
//!
//! This is the in-tree reproduction of the published 1M–100M table. One
//! backend per process (`SCALE_BACKENDS`, default `pedradb`). Optional
//! peers behind features: `real` (RocksDB) and `fjall`.
//!
//! Same key shape as the slipstream snapshot bench the numbers were
//! taken on: clustered `route.svc-NNNNNN.NNNNNNNN`, ~200 B values,
//! 1024-entry apply batches. Not a Criterion loop — one-shot hydrate
//! and a fixed probe/get sample, so a 100M run is one process, one
//! number, not 10 hydrations.

#![allow(dead_code)]

use std::io::Write;
use std::path::Path;
use std::time::Instant;

const ROUTES_PER_SERVICE: usize = 1000;
const APPLY_BATCH: usize = 1024;
const VALUE_POOL_BYTES: usize = 1 << 20;
const PROBES: usize = 10_000;
const GET_HIT_N: usize = 10_000;
const PREFIX_N: usize = 200;
const LOOKUP_N: usize = 200;

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub fn entries() -> usize {
    env_usize("SCALE_ENTRIES", 1_000_000)
}

pub fn value_bytes() -> usize {
    env_usize("SCALE_VALUE_BYTES", 200)
}

pub fn cache_bytes() -> Option<u64> {
    std::env::var("SCALE_CACHE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
}

pub fn backends() -> Vec<String> {
    std::env::var("SCALE_BACKENDS")
        .unwrap_or_else(|_| "pedradb".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn next_rand(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

pub fn key(i: usize) -> String {
    format!(
        "route.svc-{:06}.{:08}",
        i / ROUTES_PER_SERVICE,
        i % ROUTES_PER_SERVICE
    )
}

fn miss_key(i: usize) -> String {
    format!("route.svc-9{:06}.{:08}", i % 999_999, i % 1000)
}

fn value_pool() -> Vec<u8> {
    let mut pool = vec![0u8; VALUE_POOL_BYTES];
    let mut state = 0x5EED_5EED_5EED_5EEDu64;
    for chunk in pool.chunks_mut(8) {
        let bytes = next_rand(&mut state).to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    pool
}

fn value_for(pool: &[u8], i: usize, len: usize) -> &[u8] {
    let off = (i * 7919) % (pool.len() - len);
    &pool[off..off + len]
}

fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

fn pct_us(lat: &mut [u64], q: f64) -> f64 {
    if lat.is_empty() {
        return 0.0;
    }
    lat.sort_unstable();
    let i = (((lat.len() as f64) * q) as usize).min(lat.len() - 1);
    lat[i] as f64 / 1000.0
}

fn mean_us(lat: &[u64]) -> f64 {
    if lat.is_empty() {
        return 0.0;
    }
    lat.iter().sum::<u64>() as f64 / lat.len() as f64 / 1000.0
}

/// One LSM backend the scale harness can hydrate and read.
pub trait ScaleStore {
    fn label(&self) -> &'static str;
    fn put_batch(&mut self, kvs: &[(&[u8], &[u8])]) -> bool;
    fn get(&self, k: &[u8]) -> Option<Vec<u8>>;
    fn prefix_count(&self, prefix: &[u8]) -> usize;
    fn settle(&mut self) -> bool;
}

fn hydrate(store: &mut dyn ScaleStore, n: usize, pool: &[u8], vlen: usize) {
    let mut batch_owned: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(APPLY_BATCH);
    let mut i = 0usize;
    while i < n {
        batch_owned.clear();
        let end = (i + APPLY_BATCH).min(n);
        for j in i..end {
            batch_owned.push((key(j).into_bytes(), value_for(pool, j, vlen).to_vec()));
        }
        let refs: Vec<(&[u8], &[u8])> = batch_owned
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
            .collect();
        assert!(store.put_batch(&refs), "hydrate put_batch {}", store.label());
        i = end;
    }
}

fn run_one(store: &mut dyn ScaleStore, dir: &Path, n: usize, vlen: usize, pool: &[u8]) {
    let label = store.label();
    let t0 = Instant::now();
    hydrate(store, n, pool, vlen);
    let secs = t0.elapsed().as_secs_f64();
    let bytes = dir_size_bytes(dir);
    eprintln!(
        "hydrate/{label}: {n} entries in {secs:.1}s ({:.2}M entries/s); on disk {:.2} GiB ({:.0} B/entry)",
        n as f64 / secs / 1e6,
        bytes as f64 / (1u64 << 30) as f64,
        bytes as f64 / n as f64,
    );

    let t1 = Instant::now();
    assert!(store.settle(), "settle {label}");
    let settle_s = t1.elapsed().as_secs_f64();
    let settled = dir_size_bytes(dir);
    eprintln!(
        "settle/{label}: {settle_s:.1}s; on disk after {:.2} GiB",
        settled as f64 / (1u64 << 30) as f64,
    );

    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut hit = Vec::with_capacity(PROBES);
    let mut miss = Vec::with_capacity(PROBES);
    for _ in 0..PROBES {
        let i = (next_rand(&mut state) % n as u64) as usize;
        let t = Instant::now();
        let _ = store.get(key(i).as_bytes());
        hit.push(t.elapsed().as_nanos() as u64);
        let t = Instant::now();
        let _ = store.get(miss_key(i).as_bytes());
        miss.push(t.elapsed().as_nanos() as u64);
    }
    eprintln!(
        "probe_hit/{label}: p50 {:.1}µs / p99 {:.1}µs / p999 {:.1}µs / max {:.1}µs",
        pct_us(&mut hit, 0.50),
        pct_us(&mut hit, 0.99),
        pct_us(&mut hit, 0.999),
        hit.iter().copied().max().unwrap_or(0) as f64 / 1000.0,
    );
    eprintln!(
        "probe_miss/{label}: p50 {:.1}µs / p99 {:.1}µs / p999 {:.1}µs / max {:.1}µs",
        pct_us(&mut miss, 0.50),
        pct_us(&mut miss, 0.99),
        pct_us(&mut miss, 0.999),
        miss.iter().copied().max().unwrap_or(0) as f64 / 1000.0,
    );

    let mut gstate = 0xDEAD_BEEFu64;
    let mut gets = Vec::with_capacity(GET_HIT_N);
    for _ in 0..GET_HIT_N {
        let i = (next_rand(&mut gstate) % n as u64) as usize;
        let t = Instant::now();
        let _ = store.get(key(i).as_bytes());
        gets.push(t.elapsed().as_nanos() as u64);
    }
    eprintln!("get_hit/{label}: mean {:.1}µs (n={GET_HIT_N})", mean_us(&gets));

    let mid_service = (n / ROUTES_PER_SERVICE) / 2;
    let prefix = format!("route.svc-{mid_service:06}.");
    let mut pfx = Vec::with_capacity(PREFIX_N);
    for _ in 0..PREFIX_N {
        let t = Instant::now();
        let c = store.prefix_count(prefix.as_bytes());
        pfx.push(t.elapsed().as_nanos() as u64);
        std::hint::black_box(c);
    }
    eprintln!(
        "prefix_scan/{label}: mean {:.1}µs (n={PREFIX_N}, prefix={prefix})",
        mean_us(&pfx)
    );

    let mut lstate = 0xC0DE_BEEFu64;
    let mut loops = Vec::with_capacity(LOOKUP_N);
    for _ in 0..LOOKUP_N {
        let keys: Vec<String> = (0..100)
            .map(|_| key((next_rand(&mut lstate) % n as u64) as usize))
            .collect();
        let t = Instant::now();
        for k in &keys {
            let _ = store.get(k.as_bytes());
        }
        loops.push(t.elapsed().as_nanos() as u64);
    }
    eprintln!(
        "lookup_100/{label}_get_loop: mean {:.1}µs (n={LOOKUP_N})",
        mean_us(&loops)
    );
}

// ── Pedra (always) ──────────────────────────────────────────────────────────

struct PedraScale {
    db: rocksdb_compat::DB,
}

impl PedraScale {
    fn open(path: &Path) -> Self {
        let mut opts = rocksdb_compat::Options::default();
        opts.create_if_missing(true);
        opts.set_sync(false);
        if let Some(b) = cache_bytes() {
            opts.set_block_cache(&rocksdb_compat::Cache::new_lru_cache(b as usize));
        }
        let db = rocksdb_compat::DB::open(&opts, path).expect("pedra open");
        Self { db }
    }
}

impl ScaleStore for PedraScale {
    fn label(&self) -> &'static str {
        "pedradb"
    }
    fn put_batch(&mut self, kvs: &[(&[u8], &[u8])]) -> bool {
        let mut wb = rocksdb_compat::WriteBatch::default();
        for (k, v) in kvs {
            wb.put(k, v);
        }
        self.db.write(&wb).is_ok()
    }
    fn get(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.db.get(k).ok().flatten()
    }
    fn prefix_count(&self, prefix: &[u8]) -> usize {
        let mut end = prefix.to_vec();
        if let Some(last) = end.last_mut() {
            *last = last.saturating_add(1);
        }
        self.db
            .count_named("default", prefix, &end, usize::MAX)
            .unwrap_or(0)
    }
    fn settle(&mut self) -> bool {
        self.db.flush().is_ok() && self.db.compact().is_ok()
    }
}

// ── RocksDB (feature real) ──────────────────────────────────────────────────

#[cfg(feature = "real")]
struct RocksScale {
    db: rocksdb::DB,
}

#[cfg(feature = "real")]
impl RocksScale {
    fn open(path: &Path) -> Self {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        if let Some(b) = cache_bytes() {
            let cache = rocksdb::Cache::new_lru_cache(b as usize);
            let mut bto = rocksdb::BlockBasedOptions::default();
            bto.set_block_cache(&cache);
            opts.set_block_based_table_factory(&bto);
        }
        let db = rocksdb::DB::open(&opts, path).expect("rocks open");
        Self { db }
    }
}

#[cfg(feature = "real")]
impl ScaleStore for RocksScale {
    fn label(&self) -> &'static str {
        "rocksdb"
    }
    fn put_batch(&mut self, kvs: &[(&[u8], &[u8])]) -> bool {
        let mut wb = rocksdb::WriteBatch::default();
        for (k, v) in kvs {
            wb.put(k, v);
        }
        self.db.write(wb).is_ok()
    }
    fn get(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.db.get(k).ok().flatten()
    }
    fn prefix_count(&self, prefix: &[u8]) -> usize {
        let mut n = 0usize;
        let iter = self.db.prefix_iterator(prefix);
        for item in iter {
            let Ok((k, _)) = item else { break };
            if !k.starts_with(prefix) {
                break;
            }
            n += 1;
        }
        n
    }
    fn settle(&mut self) -> bool {
        self.db.flush().is_ok()
            && {
                self.db.compact_range::<&[u8], &[u8]>(None, None);
                true
            }
    }
}

// ── Fjall (feature fjall) ───────────────────────────────────────────────────

#[cfg(feature = "fjall")]
struct FjallScale {
    db: fjall::Database,
    ks: fjall::Keyspace,
}

#[cfg(feature = "fjall")]
impl FjallScale {
    fn open(path: &Path) -> Self {
        let mut b = fjall::Database::builder(path);
        if let Some(n) = cache_bytes() {
            b = b.cache_size(n);
        }
        let db = b.open().expect("fjall open");
        let ks = db
            .keyspace("default", fjall::KeyspaceCreateOptions::default)
            .expect("fjall keyspace");
        Self { db, ks }
    }
}

#[cfg(feature = "fjall")]
impl ScaleStore for FjallScale {
    fn label(&self) -> &'static str {
        "fjall"
    }
    fn put_batch(&mut self, kvs: &[(&[u8], &[u8])]) -> bool {
        let mut wb = self.db.batch();
        for (k, v) in kvs {
            wb.insert(&self.ks, *k, *v);
        }
        wb.commit().is_ok()
    }
    fn get(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.ks.get(k).ok().flatten().map(|v| v.to_vec())
    }
    fn prefix_count(&self, prefix: &[u8]) -> usize {
        self.ks.prefix(prefix).count()
    }
    fn settle(&mut self) -> bool {
        self.db.persist(fjall::PersistMode::SyncAll).is_ok()
    }
}

/// Run every backend in `SCALE_BACKENDS` (one store at a time, sequential).
/// For 25M/100M prefer one backend per process so RSS cannot leak.
pub fn run(out: &Path) {
    let n = entries();
    let vlen = value_bytes();
    let pool = value_pool();
    let list = backends();
    if list.len() > 1 && std::env::var("SCALE_ALLOW_MULTI").as_deref() != Ok("1") {
        eprintln!(
            "SCALE_BACKENDS has {} engines; 25M/100M runs must be one process per backend (allocator high-water). Set SCALE_ALLOW_MULTI=1 to override, or pass a single name.",
            list.len()
        );
        std::process::exit(2);
    }
    std::fs::create_dir_all(out).expect("mkdir");
    eprintln!(
        "[scale] backends={} entries={n} value_bytes={vlen} cache={:?}",
        list.join(","),
        cache_bytes()
    );
    for name in &list {
        let dir = out.join(format!("db-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir db");
        match name.as_str() {
            "pedradb" | "compat" => {
                let mut s = PedraScale::open(&dir);
                run_one(&mut s, &dir, n, vlen, &pool);
            }
            "rocksdb" => {
                #[cfg(feature = "real")]
                {
                    let mut s = RocksScale::open(&dir);
                    run_one(&mut s, &dir, n, vlen, &pool);
                }
                #[cfg(not(feature = "real"))]
                {
                    eprintln!("backend rocksdb needs --features real");
                    std::process::exit(1);
                }
            }
            "fjall" => {
                #[cfg(feature = "fjall")]
                {
                    let mut s = FjallScale::open(&dir);
                    run_one(&mut s, &dir, n, vlen, &pool);
                }
                #[cfg(not(feature = "fjall"))]
                {
                    eprintln!("backend fjall needs --features fjall");
                    std::process::exit(1);
                }
            }
            other => {
                eprintln!("unknown SCALE_BACKENDS entry {other:?} (want pedradb|rocksdb|fjall)");
                std::process::exit(1);
            }
        }
        let _ = writeln!(std::io::stderr(), "[scale] done {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn key_clusters_by_service() {
        assert_eq!(&key(0)[..16], "route.svc-000000");
        assert_eq!(&key(1000)[..16], "route.svc-000001");
        assert_eq!(key(0).len(), key(42).len());
    }

    #[test]
    fn pedra_tiny_hydrate_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut s = PedraScale::open(dir.path());
        let pool = value_pool();
        hydrate(&mut s, 64, &pool, 32);
        assert!(s.get(key(0).as_bytes()).is_some());
        assert!(s.get(key(63).as_bytes()).is_some());
        assert!(s.get(miss_key(0).as_bytes()).is_none());
        assert!(s.prefix_count(b"route.svc-000000.") > 0);
        assert!(s.settle());
    }
}
