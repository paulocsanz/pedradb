//! Comparative benchmarks for the on-disk [`SnapshotStore`] backends:
//! `FjallSnapshot` vs `RocksDbSnapshot` vs `PedraDbSnapshot`, on the route-fold
//! workload shape the backends are tuned for (clustered
//! `route.svc-NNNNNN.NNNNNNNN` keys, ~200 B values, batched applies, point gets
//! for existing keys, per-service prefix scans).
//!
//! Ported from beyondoss/slipstream `benches/snapshot_backends.rs` (MIT),
//! branch `cursor/pedradb-snapshot-adapter-cb1b` @ `d3bc6a4`, verbatim except
//! for the crate import. Pedra is pinned to the in-tree `rocksdb-compat`.
//!
//! Run with all three backends enabled:
//!
//! ```text
//! cargo bench -p snapshot-bench --bench snapshot_backends --features fjall,rocksdb,pedradb
//! SLIPSTREAM_BENCH_ENTRIES=4000000 cargo bench -p snapshot-bench --bench snapshot_backends --features fjall,rocksdb,pedradb
//! SLIPSTREAM_BENCH_ENTRIES=1000000000 cargo bench -p snapshot-bench --bench snapshot_backends --features fjall,rocksdb,pedradb
//! ```
//!
//! Env knobs: `SLIPSTREAM_BENCH_ENTRIES` (default 1_000_000),
//! `SLIPSTREAM_BENCH_VALUE_BYTES` (default 200),
//! `SLIPSTREAM_BENCH_CACHE_BYTES` (default: each backend's default cache),
//! `SLIPSTREAM_BENCH_BACKENDS` (comma list: `fjall,rocksdb,pedradb`; default all),
//! `SLIPSTREAM_BENCH_SEQUENTIAL=1` (force one-backend-at-a-time; auto-on when
//! entries ≥ 50M so a 1B fold fits on ~250 GiB disks).
//!
//! Note: this harness reads `SLIPSTREAM_BENCH_BACKENDS`. An env var named
//! `SLIPSTREAM_BACKENDS` (set by some historical private campaign drivers) is
//! NOT read by this code — exporting it silently runs every enabled backend
//! in one process.
//!
//! Caveats for honest numbers: `TempDir` honors `TMPDIR` — point it at real
//! NVMe, not tmpfs, when disk behavior matters. Criterion's repeated iterations
//! measure *warm-cache* reads, which is fair for cross-backend comparison;
//! cold-cache latency needs a manual run against a freshly opened store.

use std::hint::black_box;
use std::path::Path;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use snapshot_bench::snapshot::SnapshotStore;
use snapshot_bench::cellcost;
use snapshot_bench::{
    FjallConfig, FjallSnapshot, KvEntry, KvUpdate, PedraDbConfig, PedraDbSnapshot, RocksDbConfig,
    RocksDbReader, RocksDbSnapshot, PedraDbReader, VersionToken, WatchCursor,
};
use tempfile::TempDir;

/// Routes per service: keys cluster as `route.svc-{service:06}.{route:08}`.
const ROUTES_PER_SERVICE: usize = 1000;
/// Updates per `apply` batch — the watch-applied flush-batch shape.
const APPLY_BATCH: usize = 1024;
/// Pseudo-random pool the entry values are sliced from (so compression sees
/// realistic entropy instead of a constant byte).
const VALUE_POOL_BYTES: usize = 1 << 20;
/// Above this size, keep only one backend on disk at a time (1B ≈ 220–320 GiB).
const SEQUENTIAL_ENTRIES: usize = 50_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Fjall,
    RocksDb,
    PedraDb,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Self::Fjall => "fjall",
            Self::RocksDb => "rocksdb",
            Self::PedraDb => "pedradb",
        }
    }
}

fn entries() -> usize {
    std::env::var("SLIPSTREAM_BENCH_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000)
}

fn value_bytes() -> usize {
    std::env::var("SLIPSTREAM_BENCH_VALUE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
}

fn backends() -> Vec<Backend> {
    let raw = std::env::var("SLIPSTREAM_BENCH_BACKENDS").unwrap_or_default();
    if raw.trim().is_empty() {
        return vec![Backend::Fjall, Backend::RocksDb, Backend::PedraDb];
    }
    raw.split(',')
        .filter_map(|s| match s.trim().to_ascii_lowercase().as_str() {
            "fjall" => Some(Backend::Fjall),
            "rocksdb" | "rocks" => Some(Backend::RocksDb),
            "pedradb" | "pedra" => Some(Backend::PedraDb),
            other => {
                eprintln!("snapshot_backends: ignoring unknown backend {other:?}");
                None
            }
        })
        .collect()
}

fn sequential_mode(n: usize) -> bool {
    std::env::var("SLIPSTREAM_BENCH_SEQUENTIAL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || n >= SEQUENTIAL_ENTRIES
}

/// Deterministic xorshift64* step — repeatable key/value choice across runs.
fn next_rand(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn key(i: usize) -> String {
    format!(
        "route.svc-{:06}.{:08}",
        i / ROUTES_PER_SERVICE,
        i % ROUTES_PER_SERVICE
    )
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

/// Fold `n` entries into `store` in `APPLY_BATCH`-sized applies, the way
/// a watch-driven consumer would during hydration.
fn hydrate<S: SnapshotStore>(store: &mut S, n: usize, pool: &[u8], vlen: usize) {
    let mut batch = Vec::with_capacity(APPLY_BATCH);
    let mut i = 0usize;
    let progress_every = if n >= 10_000_000 { 10_000_000 } else { usize::MAX };
    let started = std::time::Instant::now();
    while i < n {
        batch.clear();
        let end = (i + APPLY_BATCH).min(n);
        for j in i..end {
            batch.push(KvUpdate::Put(KvEntry {
                key: key(j),
                value: value_for(pool, j, vlen).to_vec(),
                version: VersionToken::from_u64(j as u64 + 1),
            }));
        }
        store
            .apply(&batch, &WatchCursor::from_u64(end as u64))
            .expect("apply");
        i = end;
        if i % progress_every == 0 || i == n {
            let secs = started.elapsed().as_secs_f64().max(1e-9);
            eprintln!(
                "  hydrate progress: {i}/{n} ({:.1}%, {:.2}M entries/s)",
                100.0 * i as f64 / n as f64,
                i as f64 / secs / 1e6,
            );
        }
    }
}

fn miss_key(i: usize) -> String {
    format!("route.svc-9{:06}.{:08}", i % 999_999, i % 1000)
}

fn probe_percentiles(label: &str, mut op: impl FnMut(usize)) {
    const PROBES: usize = 10_000;
    let n = entries();
    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut lat = Vec::with_capacity(PROBES);
    for _ in 0..PROBES {
        let i = (next_rand(&mut state) % n as u64) as usize;
        let t = std::time::Instant::now();
        op(i);
        lat.push(t.elapsed());
    }
    lat.sort();
    let p = |q: f64| lat[(((lat.len() as f64) * q) as usize).min(lat.len() - 1)];
    eprintln!(
        "{label}: p50 {:.1?} / p90 {:.1?} / p99 {:.1?} / p999 {:.1?} / max {:.1?}",
        p(0.50),
        p(0.90),
        p(0.99),
        p(0.999),
        lat[lat.len() - 1],
    );
}

fn cache_bytes() -> Option<u64> {
    std::env::var("SLIPSTREAM_BENCH_CACHE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
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

fn free_disk_bytes(path: &Path) -> Option<u64> {
    let out = std::process::Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .nth(1)
        .and_then(|l| l.trim().parse().ok())
}

fn open_fjall(path: &Path) -> FjallSnapshot {
    let mut config = FjallConfig::default();
    if let Some(bytes) = cache_bytes() {
        config.cache_size_bytes = bytes;
    }
    FjallSnapshot::open(path, config).expect("open fjall").1
}

fn open_rocksdb(path: &Path) -> RocksDbSnapshot {
    let mut config = RocksDbConfig::default();
    if let Some(bytes) = cache_bytes() {
        config.cache_size_bytes = bytes;
    }
    RocksDbSnapshot::open(path, config).expect("open rocksdb").1
}

fn open_pedradb(path: &Path) -> PedraDbSnapshot {
    let mut config = PedraDbConfig::default();
    if let Some(bytes) = cache_bytes() {
        config.cache_size_bytes = bytes;
    }
    PedraDbSnapshot::open(path, config).expect("open pedradb").1
}

fn maybe_settle(name: &str, dir: &Path, settle: impl FnOnce() -> Result<(), String>) {
    let size = dir_size_bytes(dir);
    let free = free_disk_bytes(dir).unwrap_or(u64::MAX);
    // fjall settle can peak ~2×; require 1.3× free headroom or skip.
    if free < size.saturating_mul(13) / 10 {
        eprintln!(
            "settle/{name}: SKIPPED — need ~{:.1} GiB free for settle headroom, have {:.1} GiB \
             (store is {:.1} GiB)",
            size as f64 * 1.3 / (1u64 << 30) as f64,
            free as f64 / (1u64 << 30) as f64,
            size as f64 / (1u64 << 30) as f64,
        );
        return;
    }
    let started = std::time::Instant::now();
    settle().unwrap_or_else(|e| panic!("settle {name}: {e}"));
    let settled_bytes = dir_size_bytes(dir);
    eprintln!(
        "settle/{name}: {:.1}s; on disk after {:.2} GiB",
        started.elapsed().as_secs_f64(),
        settled_bytes as f64 / (1u64 << 30) as f64,
    );
}

fn bench_apply_hydrate(c: &mut Criterion) {
    let n = entries();
    let vlen = value_bytes();
    let pool = value_pool();
    let selected = backends();

    if n > 8_000_000 {
        eprintln!(
            "apply_hydrate: skipped at {n} entries (>8M); see the one-shot \
             hydration timings printed by the read benchmarks' setup"
        );
        return;
    }

    let mut g = c.benchmark_group("apply_hydrate");
    g.sample_size(10);
    g.throughput(Throughput::Elements(n as u64));

    if selected.contains(&Backend::Fjall) {
        g.bench_function(BenchmarkId::new("fjall", n), |b| {
            b.iter_batched(
                || TempDir::new().unwrap(),
                |dir| {
                    let mut s = open_fjall(&dir.path().join("store"));
                    hydrate(&mut s, n, &pool, vlen);
                    dir
                },
                BatchSize::PerIteration,
            );
        });
    }
    if selected.contains(&Backend::RocksDb) {
        g.bench_function(BenchmarkId::new("rocksdb", n), |b| {
            b.iter_batched(
                || TempDir::new().unwrap(),
                |dir| {
                    let mut s = open_rocksdb(&dir.path().join("store"));
                    hydrate(&mut s, n, &pool, vlen);
                    dir
                },
                BatchSize::PerIteration,
            );
        });
    }
    if selected.contains(&Backend::PedraDb) {
        g.bench_function(BenchmarkId::new("pedradb", n), |b| {
            b.iter_batched(
                || TempDir::new().unwrap(),
                |dir| {
                    let mut s = open_pedradb(&dir.path().join("store"));
                    hydrate(&mut s, n, &pool, vlen);
                    dir
                },
                BatchSize::PerIteration,
            );
        });
    }
    g.finish();
}

fn report_hydrate(name: &str, n: usize, dir: &Path, secs: f64) {
    let bytes = dir_size_bytes(dir);
    eprintln!(
        "hydrate/{name}: {n} entries in {secs:.1}s ({:.2}M entries/s); on disk \
         {:.2} GiB ({:.0} B/entry)",
        n as f64 / secs / 1e6,
        bytes as f64 / (1u64 << 30) as f64,
        bytes as f64 / n as f64,
    );
}

fn bench_one_backend_reads(
    c: &mut Criterion,
    backend: Backend,
    n: usize,
    pool: &[u8],
    vlen: usize,
) {
    let name = backend.name();
    eprintln!("=== backend {name} (n={n}) ===");
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store");

    match backend {
        Backend::Fjall => {
            let mut store = open_fjall(&store_path);
            let started = std::time::Instant::now();
            hydrate(&mut store, n, pool, vlen);
            report_hydrate(name, n, dir.path(), started.elapsed().as_secs_f64());
            maybe_settle(name, dir.path(), || {
                store.settle().map_err(|e| e.to_string())
            });
            probe_percentiles(&format!("probe_hit/{name}"), |i| {
                let _ = black_box(store.get(&key(i)).expect("get"));
            });
            probe_percentiles(&format!("probe_miss/{name}"), |i| {
                let _ = black_box(store.get(&miss_key(i)).expect("get"));
            });
            bench_get_hit(c, name, n, |i| store.get(&key(i)).expect("get"));
            bench_prefix_scan(c, name, n, |prefix, f| {
                store
                    .for_each_in_range(prefix, |e| f(e))
                    .expect("scan");
            });
        }
        Backend::RocksDb => {
            let mut store = open_rocksdb(&store_path);
            let started = std::time::Instant::now();
            hydrate(&mut store, n, pool, vlen);
            report_hydrate(name, n, dir.path(), started.elapsed().as_secs_f64());
            maybe_settle(name, dir.path(), || {
                store.settle().map_err(|e| e.to_string())
            });
            let reader = store.reader();
            probe_percentiles(&format!("probe_hit/{name}"), |i| {
                let _ = black_box(store.get(&key(i)).expect("get"));
            });
            probe_percentiles(&format!("probe_miss/{name}"), |i| {
                let _ = black_box(store.get(&miss_key(i)).expect("get"));
            });
            bench_get_hit(c, name, n, |i| store.get(&key(i)).expect("get"));
            bench_prefix_scan(c, name, n, |prefix, f| {
                store
                    .for_each_in_range(prefix, |e| f(e))
                    .expect("scan");
            });
            bench_lookup_100(c, name, n, &reader);
        }
        Backend::PedraDb => {
            let mut store = open_pedradb(&store_path);
            let started = std::time::Instant::now();
            hydrate(&mut store, n, pool, vlen);
            report_hydrate(name, n, dir.path(), started.elapsed().as_secs_f64());
            maybe_settle(name, dir.path(), || {
                store.settle().map_err(|e| e.to_string())
            });
            let reader = store.reader();
            probe_percentiles(&format!("probe_hit/{name}"), |i| {
                let _ = black_box(store.get(&key(i)).expect("get"));
            });
            probe_percentiles(&format!("probe_miss/{name}"), |i| {
                let _ = black_box(store.get(&miss_key(i)).expect("get"));
            });
            bench_get_hit(c, name, n, |i| store.get(&key(i)).expect("get"));
            bench_prefix_scan(c, name, n, |prefix, f| {
                store
                    .for_each_in_range(prefix, |e| f(e))
                    .expect("scan");
            });
            bench_lookup_100_pedra(c, name, n, &reader);
        }
    }
    eprintln!("=== done {name}; freeing disk ===");
    drop(dir);
}

fn bench_get_hit<F>(c: &mut Criterion, name: &str, n: usize, mut get: F)
where
    F: FnMut(usize) -> Option<KvEntry>,
{
    let mut g = c.benchmark_group("get_hit");
    g.throughput(Throughput::Elements(1));
    // Shorter measurement at huge N — each coldish get can be hundreds of µs.
    if n >= SEQUENTIAL_ENTRIES {
        g.sample_size(30);
        g.warm_up_time(std::time::Duration::from_secs(2));
        g.measurement_time(std::time::Duration::from_secs(10));
    }
    let mut state = 0xDEAD_BEEFu64;
    let _cell = cellcost::Guard::new("get_hit", name);
    g.bench_function(name, |b| {
        b.iter(|| {
            let i = (next_rand(&mut state) % n as u64) as usize;
            black_box(get(i))
        });
    });
    g.finish();
    drop(_cell);
    cellcost::flush_group("get_hit");
}

fn bench_prefix_scan<F>(c: &mut Criterion, name: &str, n: usize, mut scan: F)
where
    F: FnMut(&str, &mut dyn FnMut(KvEntry) -> Result<(), snapshot_bench::snapshot::SnapshotError>),
{
    let mid_service = (n / ROUTES_PER_SERVICE) / 2;
    let prefix = format!("route.svc-{mid_service:06}.");
    let mut g = c.benchmark_group("prefix_scan");
    g.throughput(Throughput::Elements(ROUTES_PER_SERVICE as u64));
    if n >= SEQUENTIAL_ENTRIES {
        g.sample_size(20);
        g.warm_up_time(std::time::Duration::from_secs(2));
        g.measurement_time(std::time::Duration::from_secs(10));
    }
    let _cell = cellcost::Guard::new("prefix_scan", name);
    g.bench_function(name, |b| {
        b.iter(|| {
            let mut count = 0usize;
            scan(&prefix, &mut |e| {
                count += black_box(e.value.len());
                Ok(())
            });
            black_box(count)
        });
    });
    g.finish();
    drop(_cell);
    cellcost::flush_group("prefix_scan");
}

fn bench_lookup_100(c: &mut Criterion, name: &str, n: usize, reader: &RocksDbReader) {
    let make_keys = |state: &mut u64| -> Vec<String> {
        (0..100)
            .map(|_| key((next_rand(state) % n as u64) as usize))
            .collect()
    };
    let mut g = c.benchmark_group("lookup_100");
    g.throughput(Throughput::Elements(100));
    if n >= SEQUENTIAL_ENTRIES {
        g.sample_size(20);
        g.warm_up_time(std::time::Duration::from_secs(2));
        g.measurement_time(std::time::Duration::from_secs(15));
    }
    let mut loop_state = 0xFACE_FEEDu64;
    let get_loop = format!("{name}_get_loop");
    let _cell = cellcost::Guard::new("lookup_100", &get_loop);
    g.bench_function(get_loop.as_str(), |b| {
        b.iter_batched(
            || make_keys(&mut loop_state),
            |keys| {
                for k in &keys {
                    black_box(reader.get(k).expect("get"));
                }
            },
            BatchSize::SmallInput,
        );
    });
    drop(_cell);
    let mut mg_state = 0xBADC_0FFEu64;
    let multi_get = format!("{name}_multi_get");
    let _cell = cellcost::Guard::new("lookup_100", &multi_get);
    g.bench_function(multi_get.as_str(), |b| {
        b.iter_batched(
            || make_keys(&mut mg_state),
            |keys| {
                black_box(
                    reader
                        .multi_get(keys.iter().map(String::as_str))
                        .expect("multi_get"),
                )
            },
            BatchSize::SmallInput,
        );
    });
    drop(_cell);
    g.finish();
    cellcost::flush_group("lookup_100");
}

fn bench_lookup_100_pedra(c: &mut Criterion, name: &str, n: usize, reader: &PedraDbReader) {
    let make_keys = |state: &mut u64| -> Vec<String> {
        (0..100)
            .map(|_| key((next_rand(state) % n as u64) as usize))
            .collect()
    };
    let mut g = c.benchmark_group("lookup_100");
    g.throughput(Throughput::Elements(100));
    if n >= SEQUENTIAL_ENTRIES {
        g.sample_size(20);
        g.warm_up_time(std::time::Duration::from_secs(2));
        g.measurement_time(std::time::Duration::from_secs(15));
    }
    let mut loop_state = 0xC0DE_BEEFu64;
    let get_loop = format!("{name}_get_loop");
    let _cell = cellcost::Guard::new("lookup_100", &get_loop);
    g.bench_function(get_loop.as_str(), |b| {
        b.iter_batched(
            || make_keys(&mut loop_state),
            |keys| {
                for k in &keys {
                    black_box(reader.get(k).expect("get"));
                }
            },
            BatchSize::SmallInput,
        );
    });
    drop(_cell);
    let mut mg_state = 0xFEED_FACEu64;
    let multi_get = format!("{name}_multi_get");
    let _cell = cellcost::Guard::new("lookup_100", &multi_get);
    g.bench_function(multi_get.as_str(), |b| {
        b.iter_batched(
            || make_keys(&mut mg_state),
            |keys| {
                black_box(
                    reader
                        .multi_get(keys.iter().map(String::as_str))
                        .expect("multi_get"),
                )
            },
            BatchSize::SmallInput,
        );
    });
    drop(_cell);
    g.finish();
    cellcost::flush_group("lookup_100");
}

fn bench_reads(c: &mut Criterion) {
    let n = entries();
    let vlen = value_bytes();
    let pool = value_pool();
    let selected = backends();
    let sequential = sequential_mode(n);
    eprintln!(
        "snapshot_backends: n={n} value_bytes={vlen} sequential={sequential} backends={:?}",
        selected.iter().map(|b| b.name()).collect::<Vec<_>>()
    );
    // Always one backend at a time: at ≥50M (or SLIPSTREAM_BENCH_SEQUENTIAL=1)
    // concurrent stores cannot fit; at small N the difference is only page-cache
    // warming order, which already varied by run order before.
    for backend in selected {
        bench_one_backend_reads(c, backend, n, &pool, vlen);
    }
}

criterion_group!(benches, bench_apply_hydrate, bench_reads);
criterion_main!(benches);
