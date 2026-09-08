//! Sorted-ingest scale harness: hydrate + settle + point/prefix/lookup.
//!
//! This is the in-tree reproduction of the published 1M–100M table
//! (ladder also has 50M — RFC-0168 P2.2). One
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
use std::path::{Path, PathBuf};
use std::time::Instant;

const ROUTES_PER_SERVICE: usize = 1000;
const APPLY_BATCH: usize = 1024;
/// Official scale ladder. 50M (RFC-0168 P2.2) sits between 25M and 100M
/// so get_loop/prefix curvature is visible instead of a 4× jump. On the
/// 4 GiB box the 3 GiB warm floor already skips 25M+; on a large host
/// `max(3 GiB, 3/4 RAM)` can still warm 50M.
pub const SCALE_LADDER: &[u64] = &[1_000_000, 10_000_000, 25_000_000, 50_000_000, 100_000_000];
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

/// Official scale cache (256 MiB). `pedra scale` applies this when neither
/// `--cache` nor `SCALE_CACHE_BYTES` is set.
pub const DEFAULT_SCALE_CACHE_BYTES: u64 = 268_435_456;

/// Parsed `pedra scale` invocation (RFC-0178 P0.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PedraScaleCli {
    /// Store directory (one process, one n).
    pub dir: PathBuf,
    /// `SCALE_ENTRIES` override.
    pub entries: Option<u64>,
    /// `SCALE_CACHE_BYTES` override.
    pub cache: Option<u64>,
    /// `SCALE_BACKENDS` override.
    pub backends: Option<String>,
}

const PEDRA_SCALE_USAGE: &str =
    "usage: pedra scale [--entries N] [--cache BYTES] [--backends NAME] [dir]";

/// Parse `["scale", ...flags, dir]`. Flags win over env at [`apply_pedra_scale`].
pub fn parse_pedra_scale(args: &[String]) -> Result<PedraScaleCli, String> {
    if args.first().map(String::as_str) != Some("scale") {
        return Err(PEDRA_SCALE_USAGE.into());
    }
    let mut entries = None;
    let mut cache = None;
    let mut backends = None;
    let mut dir = None;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--entries" => {
                let v = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| PEDRA_SCALE_USAGE.to_string())?;
                entries = Some(v);
                i += 2;
            }
            "--cache" => {
                let v = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| PEDRA_SCALE_USAGE.to_string())?;
                cache = Some(v);
                i += 2;
            }
            "--backends" => {
                let v = args
                    .get(i + 1)
                    .cloned()
                    .ok_or_else(|| PEDRA_SCALE_USAGE.to_string())?;
                backends = Some(v);
                i += 2;
            }
            "-h" | "--help" => return Err(PEDRA_SCALE_USAGE.into()),
            s if s.starts_with('-') => {
                return Err(format!("unknown flag: {s}\n{PEDRA_SCALE_USAGE}"));
            }
            s => {
                if dir.is_some() {
                    return Err(format!("extra argument: {s}\n{PEDRA_SCALE_USAGE}"));
                }
                dir = Some(PathBuf::from(s));
                i += 1;
            }
        }
    }
    Ok(PedraScaleCli {
        dir: dir.unwrap_or_else(|| PathBuf::from("/tmp/pedra-scale")),
        entries,
        cache,
        backends,
    })
}

/// Push CLI overrides into `SCALE_*`. Default cache is 256 MiB when unset.
pub fn apply_pedra_scale(cli: &PedraScaleCli) {
    if let Some(n) = cli.entries {
        std::env::set_var("SCALE_ENTRIES", n.to_string());
    }
    if let Some(c) = cli.cache {
        std::env::set_var("SCALE_CACHE_BYTES", c.to_string());
    } else if std::env::var_os("SCALE_CACHE_BYTES").is_none() {
        std::env::set_var("SCALE_CACHE_BYTES", DEFAULT_SCALE_CACHE_BYTES.to_string());
    }
    if let Some(b) = &cli.backends {
        std::env::set_var("SCALE_BACKENDS", b);
    }
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

fn dir_sst_count(path: &Path) -> usize {
    std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "sst"))
                .count()
        })
        .unwrap_or(0)
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
    /// Drain leftover ingest into SSTs **inside the hydrate timer**.
    ///
    /// Fjall persist-on-commit already did this work during `put_batch`;
    /// Pedra's bulk path parks a <chunk tail until an explicit flush.
    /// RFC-0168 P1.3: move that tail into hydrate so settle is
    /// compact-of-quiet (≤ Fjall's persist no-op), not a 64 MiB encode.
    fn finish_hydrate(&mut self) -> bool {
        true
    }
    fn settle(&mut self) -> bool;
    /// RFC-0178 P2.1: `hot` or `bounded-cache` after settle. Peers: None.
    fn ram_mode(&self) -> Option<&'static str> {
        None
    }
    /// RFC-0184 P2.22: WRITEPHASE snapshot (`None` unless env at open).
    fn write_phase_snapshot(&self) -> Option<[u64; 7]> {
        None
    }
}

/// RFC-0178 P2.1: map `pedra.ram-pressure` to the published regime name.
#[must_use]
pub fn ram_mode_label(pressure: u64) -> &'static str {
    if pressure == 1 {
        "bounded-cache"
    } else {
        "hot"
    }
}

fn mode_suffix(store: &dyn ScaleStore) -> String {
    match store.ram_mode() {
        Some(m) => format!(" mode={m}"),
        None => String::new(),
    }
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
        assert!(
            store.put_batch(&refs),
            "hydrate put_batch {}",
            store.label()
        );
        i = end;
    }
    assert!(store.finish_hydrate(), "finish_hydrate {}", store.label());
}

fn run_one(store: &mut dyn ScaleStore, dir: &Path, n: usize, vlen: usize, pool: &[u8]) {
    let label = store.label();
    let phase0 = store.write_phase_snapshot();
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
    if let (Some(a), Some(b)) = (phase0, store.write_phase_snapshot()) {
        let batches = ((n + APPLY_BATCH - 1) / APPLY_BATCH).max(1);
        let p50_ms = (secs / batches as f64) * 1000.0;
        let d = super::diagnose_from_phases(p50_ms, a, b, 1, 0.0, 0);
        super::eprint_write_diagnose(&format!("hydrate/{label}"), &d);
    }

    let phase0 = store.write_phase_snapshot();
    let t1 = Instant::now();
    assert!(store.settle(), "settle {label}");
    let settle_s = t1.elapsed().as_secs_f64();
    let settled = dir_size_bytes(dir);
    eprintln!(
        "settle/{label}: {settle_s:.3}s; on disk after {:.2} GiB{}",
        settled as f64 / (1u64 << 30) as f64,
        mode_suffix(store),
    );
    if let (Some(a), Some(b)) = (phase0, store.write_phase_snapshot()) {
        let d = super::diagnose_from_phases(settle_s * 1000.0, a, b, 1, 0.0, 0);
        super::eprint_write_diagnose(&format!("settle/{label}"), &d);
    }

    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut hit = Vec::with_capacity(PROBES);
    let mut miss = Vec::with_capacity(PROBES);
    let cost_on = pedradb_core::cost::enabled();
    let mut miss_considered = 0u64;
    let mut miss_ops = 0u64;
    for _ in 0..PROBES {
        let i = (next_rand(&mut state) % n as u64) as usize;
        let t = Instant::now();
        let _ = store.get(key(i).as_bytes());
        hit.push(t.elapsed().as_nanos() as u64);
        let c0 = cost_on.then(pedradb_core::cost::read);
        let t = Instant::now();
        let _ = store.get(miss_key(i).as_bytes());
        miss.push(t.elapsed().as_nanos() as u64);
        if let Some(c0) = c0 {
            let d = pedradb_core::cost::read().since(&c0);
            miss_considered = miss_considered.saturating_add(d.point_sst_considered);
            miss_ops = miss_ops.saturating_add(d.point_ops);
        }
    }
    eprintln!(
        "probe_hit/{label}: p50 {:.1}µs / p99 {:.1}µs / p999 {:.1}µs / max {:.1}µs",
        pct_us(&mut hit, 0.50),
        pct_us(&mut hit, 0.99),
        pct_us(&mut hit, 0.999),
        hit.iter().copied().max().unwrap_or(0) as f64 / 1000.0,
    );
    {
        // RFC-0184 P2.21: p50 of the hit arm vs the 0176 clock (get_hit
        // is a mean of a later random sample; this is the published p50).
        let measured_ns = if hit.is_empty() {
            0
        } else {
            hit[(((hit.len() as f64) * 0.50) as usize).min(hit.len() - 1)]
        };
        let ram = cache_bytes().unwrap_or(64 << 30);
        let f = pedradb_core::scale_kernel::scale_forecast(n as u64, ram);
        let as_is = pedradb_core::scale_kernel::scale_forecast_as_is(n as u64, ram);
        let class = pedradb_core::classify_get(
            measured_ns,
            f.best_ns,
            f.happy_ns,
            f.worst_ns,
            as_is.best_ns,
        );
        eprintln!(
            "diagnose get probe_hit/{label} measured_ns={measured_ns} (p50) best={} happy={} worst={} as_is={} class={}",
            f.best_ns,
            f.happy_ns,
            f.worst_ns,
            as_is.best_ns,
            class.token()
        );
    }
    eprintln!(
        "probe_miss/{label}: mean {:.1}µs (n={PROBES})",
        mean_us(&miss)
    );
    if cost_on {
        let per_get = if miss_ops == 0 {
            0
        } else {
            miss_considered / miss_ops
        };
        let f =
            pedradb_core::scale_kernel::scale_forecast(n as u64, cache_bytes().unwrap_or(64 << 30));
        let class = pedradb_core::classify_probes(per_get, f.p_best);
        eprintln!(
            "diagnose probes probe_miss/{label} per_get={per_get} p_best={} class={}",
            f.p_best,
            class.token()
        );
    }

    let mut gstate = 0xDEAD_BEEFu64;
    let mut gets = Vec::with_capacity(GET_HIT_N);
    let cost0 = pedradb_core::cost::read();
    for _ in 0..GET_HIT_N {
        let i = (next_rand(&mut gstate) % n as u64) as usize;
        let t = Instant::now();
        let _ = store.get(key(i).as_bytes());
        gets.push(t.elapsed().as_nanos() as u64);
    }
    let cost_hit = pedradb_core::cost::read().since(&cost0);
    eprintln!(
        "get_hit/{label}: mean {:.1}µs (n={GET_HIT_N}){}",
        mean_us(&gets),
        mode_suffix(store),
    );
    {
        let measured_ns = if gets.is_empty() {
            0
        } else {
            gets.iter().sum::<u64>() / gets.len() as u64
        };
        let ram = cache_bytes().unwrap_or(64 << 30);
        let f = pedradb_core::scale_kernel::scale_forecast(n as u64, ram);
        let as_is = pedradb_core::scale_kernel::scale_forecast_as_is(n as u64, ram);
        let class = pedradb_core::classify_get(
            measured_ns,
            f.best_ns,
            f.happy_ns,
            f.worst_ns,
            as_is.best_ns,
        );
        eprintln!(
            "diagnose get get_hit/{label} measured_ns={measured_ns} best={} happy={} worst={} as_is={} class={}",
            f.best_ns,
            f.happy_ns,
            f.worst_ns,
            as_is.best_ns,
            class.token()
        );
    }
    if pedradb_core::cost::enabled() {
        let n_sst = dir_sst_count(dir);
        eprintln!(
            "cost/get_hit/{label}: {} n_sst={n_sst} mean_us={:.1} probes/op={:.2} file/op={:.2} pread_us/file={:.1} image_us/op={:.2}",
            cost_hit.line(),
            mean_us(&gets),
            if cost_hit.point_ops == 0 { 0.0 } else { cost_hit.point_sst_considered as f64 / cost_hit.point_ops as f64 },
            if cost_hit.point_ops == 0 { 0.0 } else { cost_hit.point_block_file as f64 / cost_hit.point_ops as f64 },
            if cost_hit.point_block_file == 0 { 0.0 } else { cost_hit.point_pread_ns as f64 / cost_hit.point_block_file as f64 / 1000.0 },
            if cost_hit.point_ops == 0 { 0.0 } else { cost_hit.point_image_ns as f64 / cost_hit.point_ops as f64 / 1000.0 },
        );
    }

    let mid_service = (n / ROUTES_PER_SERVICE) / 2;
    let prefix = format!("route.svc-{mid_service:06}.");
    let mut pfx = Vec::with_capacity(PREFIX_N);
    let cost_pfx0 = pedradb_core::cost::read();
    for _ in 0..PREFIX_N {
        let t = Instant::now();
        let c = store.prefix_count(prefix.as_bytes());
        pfx.push(t.elapsed().as_nanos() as u64);
        std::hint::black_box(c);
    }
    let cost_pfx = pedradb_core::cost::read().since(&cost_pfx0);
    eprintln!(
        "prefix_scan/{label}: mean {:.1}µs (n={PREFIX_N}, prefix={prefix}){}",
        mean_us(&pfx),
        mode_suffix(store),
    );
    if pedradb_core::cost::enabled() {
        let per_scan = if cost_pfx.scan_ops == 0 {
            0
        } else {
            cost_pfx.scan_sst_probed / cost_pfx.scan_ops
        };
        let f =
            pedradb_core::scale_kernel::scale_forecast(n as u64, cache_bytes().unwrap_or(64 << 30));
        let class = pedradb_core::classify_probes(per_scan, f.p_best);
        eprintln!(
            "diagnose probes prefix_scan/{label} per_scan={per_scan} p_best={} class={}",
            f.p_best,
            class.token()
        );
    }

    let mut lstate = 0xC0DE_BEEFu64;
    let mut loops = Vec::with_capacity(LOOKUP_N);
    let cost_loop0 = pedradb_core::cost::read();
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
    let cost_loop = pedradb_core::cost::read().since(&cost_loop0);
    eprintln!(
        "lookup_100/{label}_get_loop: mean {:.1}µs (n={LOOKUP_N}){}",
        mean_us(&loops),
        mode_suffix(store),
    );
    {
        let loop_ns = if loops.is_empty() {
            0
        } else {
            loops.iter().sum::<u64>() / loops.len() as u64
        };
        let measured_ns = loop_ns / 100;
        let ram = cache_bytes().unwrap_or(64 << 30);
        let f = pedradb_core::scale_kernel::scale_forecast(n as u64, ram);
        let as_is = pedradb_core::scale_kernel::scale_forecast_as_is(n as u64, ram);
        let class = pedradb_core::classify_get(
            measured_ns,
            f.best_ns,
            f.happy_ns,
            f.worst_ns,
            as_is.best_ns,
        );
        eprintln!(
            "diagnose get lookup_100/{label} measured_ns={measured_ns} (loop/100) best={} happy={} worst={} as_is={} class={}",
            f.best_ns,
            f.happy_ns,
            f.worst_ns,
            as_is.best_ns,
            class.token()
        );
    }
    if pedradb_core::cost::enabled() {
        eprintln!(
            "cost/get_loop/{label}: {} mean_us={:.1} probes/op={:.2} file/op={:.2} pread_us/file={:.1}",
            cost_loop.line(),
            mean_us(&loops),
            if cost_loop.point_ops == 0 { 0.0 } else { cost_loop.point_sst_considered as f64 / cost_loop.point_ops as f64 },
            if cost_loop.point_ops == 0 { 0.0 } else { cost_loop.point_block_file as f64 / cost_loop.point_ops as f64 },
            if cost_loop.point_block_file == 0 { 0.0 } else { cost_loop.point_pread_ns as f64 / cost_loop.point_block_file as f64 / 1000.0 },
        );
    }
}

// ── Pedra (always) ──────────────────────────────────────────────────────────

struct PedraScale {
    db: rocksdb_compat::DB,
    ram_mode: Option<&'static str>,
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
        Self { db, ram_mode: None }
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
        // Measurement fidelity (RFC-0168 P1.2): `count_named` serves the
        // thread-local count cache after the first call (~ns) while the
        // rocks/fjall legs iterate for real. Walk the window and
        // materialize entries so every backend does the same work.
        let Some(cf) = self.db.cf_handle("default") else {
            return 0;
        };
        let Ok(iter) = self.db.prefix_iterator_cf(&cf, prefix) else {
            return 0;
        };
        let mut n = 0usize;
        for item in iter {
            let Ok((k, _)) = item else { break };
            if !k.starts_with(prefix) {
                break;
            }
            n += 1;
        }
        n
    }
    fn finish_hydrate(&mut self) -> bool {
        // Leftover open BulkRun is < bulk_chunk_cap (default 64 MiB) and
        // is otherwise the whole settle cell vs Fjall. Flush it here so
        // settle's compact_leveled sees a disjoint max-level run set.
        // No compact-worker notify: a worker job.write() held compact_gate
        // ~90 s @100M before settle compact() (compact_ns stayed 0.001).
        self.db.flush_no_notify().is_ok()
    }
    fn settle(&mut self) -> bool {
        // compact() already flushes. A second flush re-ran WARM / waited
        // on the compact worker (87 s @100M with settle_parts compact=0.002).
        // finish_hydrate already flushed. A second flush waited on
        // flush_lock (~90 s @100M, compact_ns stayed 0.001).
        let t_c = Instant::now();
        let ok = self.db.compact_no_flush().is_ok();
        eprintln!("settle_compact_call={:.3}s", t_c.elapsed().as_secs_f64());
        // One stats() snapshot: seven property_int_value calls each ran
        // ConcurrentDb::stats() (read lock + vlog_size_stats). r8: compact
        // 1.745 s, settle 82 s — the leftover was this.
        let t_s = Instant::now();
        let s = self.db.stats();
        eprintln!("settle_stats_wall={:.3}s", t_s.elapsed().as_secs_f64());
        let compact_ns = s.settle_compact_ns;
        let warm_ns = s.settle_warm_ns;
        let warm_bytes = s.settle_warm_bytes;
        eprintln!(
            "settle_parts/pedradb: compact={:.3}s warm={:.3}s warm_bytes={warm_bytes}",
            compact_ns as f64 / 1e9,
            warm_ns as f64 / 1e9,
        );
        let pressure = s.ram_pressure;
        self.ram_mode = Some(ram_mode_label(pressure));
        let sst = s.sst_bytes;
        let cap = s.ram_ceiling_bytes;
        let skip = s.ram_warm_skipped;
        let mode = self.ram_mode.unwrap_or("hot");
        eprintln!("ram_mode/pedradb: mode={mode} sst_bytes={sst} cap={cap} warm_skipped={skip}");
        if pressure == 1 {
            eprintln!(
                "ram_pressure/pedradb: sst_bytes={sst} cap={cap} warm_skipped={skip} mode=bounded-cache (page cache dropped; grow RAM/cgroup)"
            );
        }
        ok
    }
    fn ram_mode(&self) -> Option<&'static str> {
        self.ram_mode
    }
    fn write_phase_snapshot(&self) -> Option<[u64; 7]> {
        let st = self.db.write_phase_stats()?;
        let r = std::sync::atomic::Ordering::Relaxed;
        Some([
            st.commits.load(r),
            st.prepare_ns.load(r),
            st.wal_ns.load(r),
            st.mem_ns.load(r),
            st.publish_ns.load(r),
            st.flush_check_ns.load(r),
            st.lock_wait_ns.load(r),
        ])
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
        self.db.flush().is_ok() && {
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
    fn rfc0168_ladder_includes_50m() {
        assert_eq!(
            SCALE_LADDER,
            &[1_000_000, 10_000_000, 25_000_000, 50_000_000, 100_000_000]
        );
        assert!(SCALE_LADDER.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn rfc0178_ram_mode_label_two_states() {
        assert_eq!(ram_mode_label(0), "hot");
        assert_eq!(ram_mode_label(1), "bounded-cache");
        assert_eq!(ram_mode_label(2), "hot");
    }

    #[test]
    fn rfc0178_pedra_scale_parses_entries_and_dir() {
        let c = parse_pedra_scale(&[
            "scale".into(),
            "--entries".into(),
            "100000000".into(),
            "--cache".into(),
            "268435456".into(),
            "/tmp/x".into(),
        ])
        .unwrap();
        assert_eq!(c.entries, Some(100_000_000));
        assert_eq!(c.cache, Some(268_435_456));
        assert_eq!(c.dir, PathBuf::from("/tmp/x"));
        assert!(c.backends.is_none());
    }

    #[test]
    fn rfc0178_pedra_scale_rejects_unknown_flag() {
        let e = parse_pedra_scale(&["scale".into(), "--nope".into()]).unwrap_err();
        assert!(e.contains("unknown flag"), "{e}");
    }

    /// RFC-0184 P2.21: probe_hit p50 at RAM clock is not as-is walk.
    #[test]
    fn rfc0184_p221_probe_hit_p50_classifies_vs_0176() {
        let n = 1_000_000u64;
        let ram = 64u64 << 30;
        let f = pedradb_core::scale_kernel::scale_forecast(n, ram);
        let as_is = pedradb_core::scale_kernel::scale_forecast_as_is(n, ram);
        let class =
            pedradb_core::classify_get(1_100, f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns);
        assert_ne!(
            class,
            pedradb_core::GetClass::AsIsWalk,
            "probe_hit p50 @ τ_ram is not walk-all: {}",
            class.token()
        );
        assert!(
            matches!(
                class,
                pedradb_core::GetClass::Best
                    | pedradb_core::GetClass::Happy
                    | pedradb_core::GetClass::FasterThanModel
            ),
            "got {}",
            class.token()
        );
    }

    /// RFC-0184 P2.24: settle WRITEPHASE carries a write lever.
    #[test]
    fn rfc0184_p224_settle_writephase_has_lever() {
        std::env::set_var("PEDRA_WRITE_PHASE_STATS", "1");
        let dir = TempDir::new().unwrap();
        let mut s = PedraScale::open(dir.path());
        let pool = value_pool();
        hydrate(&mut s, 64, &pool, 32);
        let a = s.write_phase_snapshot().expect("WRITEPHASE before settle");
        assert!(s.settle());
        let b = s.write_phase_snapshot().expect("WRITEPHASE after settle");
        let d = crate::diagnose_from_phases(1.0, a, b, 1, 0.0, 0);
        assert!(
            d.json_object().contains("\"lever\":"),
            "settle diagnose.lever: {}",
            d.json_object()
        );
        assert_ne!(
            d.lever.token(),
            "get_path",
            "settle is compact/ingest, not get: {}",
            d.line()
        );
    }

    /// RFC-0184 P2.22: hydrate WRITEPHASE is a write lever, not get_path.
    #[test]
    fn rfc0184_p222_hydrate_writephase_has_lever() {
        std::env::set_var("PEDRA_WRITE_PHASE_STATS", "1");
        let dir = TempDir::new().unwrap();
        let mut s = PedraScale::open(dir.path());
        let a = s.write_phase_snapshot().expect("WRITEPHASE at open");
        let pool = value_pool();
        hydrate(&mut s, 64, &pool, 32);
        let b = s.write_phase_snapshot().expect("WRITEPHASE after hydrate");
        let d = crate::diagnose_from_phases(1.0, a, b, 1, 0.0, 0);
        assert_ne!(
            d.lever.token(),
            "get_path",
            "hydrate is ingest: {}",
            d.line()
        );
        assert!(
            d.json_object().contains("\"lever\":"),
            "{}",
            d.json_object()
        );
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
        assert_eq!(s.ram_mode(), Some("hot"));
    }
}
