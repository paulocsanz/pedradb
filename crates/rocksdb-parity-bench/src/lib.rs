//! Shared YCSB-shaped harness for the rocksdb-compat vs real RocksDB parity bench.
//!
//! One generic runner, two engine adapters (see [`engines`]) — the op schedule
//! (rng seed, zipf CDF, read/insert/scan/RMW mix) is identical for both engines
//! by construction. Same shape semantics as the Montanha `ycsb` suite (FDB
//! benchmark tool workloads): ycsb_a 50/50, b 95/5, c 100r, d 95r/5
//! insert-latest, e 5 insert + short scans, f 50 read-modify-write.
//!
//! The `deps` suite models the access patterns of real RocksDB dependents
//! (TiKV is the reference): raftstore apply batches across CFs, MVCC
//! version-suffix keys with reverse-seek latest read, raft-log appends,
//! cache-style overwrites, and range scans.
//!
//! Opt-in suites grow the catalog (RFC-0043; never replace the official 16):
//! `qs` (Quicksilver-inspired), `kvrocks` (Redis GET/SET/pipeline/SCAN),
//! `myrocks` (sysbench point/range/tx + LinkBench-inspired mix), `rocksapi`
//! (mixgraph / WBWI / compaction filter / ingest). See [`COMPARE_SHAPES`]
//! and `docs/rocksdb-dependents-benchmarks.md`.

#![forbid(unsafe_code)]

pub mod engines;

use std::time::{Duration, Instant};

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// `ROCKS_PARITY_ONLY=csv` — experiment filter. Unset = every shape in the
/// selected suites. Filtered runs change the rng stream; never official tables.
pub fn shape_wanted(name: &str) -> bool {
    shape_wanted_in(name, std::env::var("ROCKS_PARITY_ONLY").ok().as_deref())
}

pub fn shape_wanted_in(name: &str, only: Option<&str>) -> bool {
    match only {
        None => true,
        Some(s) => s.split(',').map(str::trim).any(|x| x == name),
    }
}

/// Extra `File::sync_all` after a Rocks write (`ROCKS_PARITY_FULL_SYNC`).
/// Must be false when `set_write_sync(false)` (untimed ycsb_c_big seed).
#[must_use]
pub fn rocks_full_sync_after_write(full_sync: bool, write_sync: bool) -> bool {
    full_sync && write_sync
}

/// RFC-0163 P1.2: `ROCKS_PARITY_SEED_ASYNC=1` — untimed ycsb seed with the
/// WAL barrier off (same pattern as `run_c_big`'s untimed seed), restored
/// before any timed shape. Without it the G1 column pays one fdatasync
/// per seed put (25M records ≈ hours); the Rocks peer already seeds async
/// by default. Off by default; when on, `report_json` records it.
#[must_use]
pub fn seed_async_enabled() -> bool {
    std::env::var("ROCKS_PARITY_SEED_ASYNC").as_deref() == Ok("1")
}

/// JSON note for the seed barrier (pure — env read once at the call site).
#[must_use]
pub fn seed_async_note(enabled: bool) -> Option<&'static str> {
    enabled.then_some("seed_async=1 (untimed seed WAL-async; timed shapes keep the column sync)")
}

/// Run the untimed `seed` under the async barrier when `enabled` and the
/// engine currently syncs before Ok. Engines already async (the Rocks
/// default peer) pass through untouched.
pub fn seed_under_async_barrier<E: Engine>(e: &E, enabled: bool, seed: impl FnOnce()) {
    let column_sync = e.sync();
    if enabled && column_sync {
        e.set_write_sync(false);
    }
    seed();
    if enabled && column_sync {
        e.set_write_sync(column_sync);
    }
}

/// RFC-0163 P1.4: client-count ladder for the mc shapes. A csv
/// `ROCKS_PARITY_CLIENTS_LADDER` (e.g. "4,16,64") runs every mc shape once
/// per count, ascending, in-process; without it the single
/// `ROCKS_PARITY_CLIENTS` value applies. Counts < 2 are skipped — the
/// 1-client rung is the plain single-client shape of every unfiltered run.
/// Bad entries panic (an official campaign must not silently drop a rung).
#[must_use]
pub fn clients_ladder_from(ladder: Option<&str>, single: usize) -> Vec<usize> {
    let csv = ladder.map(str::trim).filter(|s| !s.is_empty());
    let Some(csv) = csv else {
        return if single >= 2 { vec![single] } else { Vec::new() };
    };
    let mut ns: Vec<usize> = csv
        .split(',')
        .map(|x| {
            let t = x.trim();
            t.parse::<usize>()
                .unwrap_or_else(|_| panic!("ROCKS_PARITY_CLIENTS_LADDER: bad entry {t:?}"))
        })
        .filter(|&n| n >= 2)
        .collect();
    ns.sort_unstable();
    ns.dedup();
    ns
}

/// Env leg of `clients_ladder_from` (thin, like `seed_async_enabled`).
pub fn clients_from_env() -> Vec<usize> {
    clients_ladder_from(
        std::env::var("ROCKS_PARITY_CLIENTS_LADDER").ok().as_deref(),
        env_usize("ROCKS_PARITY_CLIENTS", 1),
    )
}

/// Live Rocks WAL among dirent names (`NNNNNN.log`). Highest number only —
/// syncing every `*.log` double-pays recycled segments and contends the
/// same `F_FULLFSYNC` as Pedra's one fd.
#[must_use]
pub fn live_wal_log_name<'a, I, S>(names: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a S>,
    S: AsRef<str> + 'a + ?Sized,
{
    let mut best: Option<(u64, &'a str)> = None;
    for n in names {
        let n = n.as_ref();
        let Some(stem) = n.strip_suffix(".log") else {
            continue;
        };
        let Ok(num) = stem.parse::<u64>() else {
            continue;
        };
        if best.map_or(true, |(b, _)| num >= b) {
            best = Some((num, n));
        }
    }
    best.map(|(_, n)| n)
}

/// Env knobs: `ROCKS_YCSB_RECORDS/OPS/PAYLOAD/DIST` (uniform|zipfian) and
/// `ROCKS_DEPS_BATCH` (ops per apply commit, deps suite).
#[derive(Clone, Debug)]
pub struct Cfg {
    pub records: usize,
    pub ops: usize,
    pub payload: usize,
    pub zipfian: bool,
    pub batch: usize,
}

impl Cfg {
    pub fn from_env(default_ops: usize) -> Self {
        Self {
            records: env_usize("ROCKS_YCSB_RECORDS", 1024).max(64),
            ops: env_usize("ROCKS_YCSB_OPS", default_ops).max(32),
            payload: env_usize("ROCKS_YCSB_PAYLOAD", 100),
            zipfian: std::env::var("ROCKS_YCSB_DIST")
                .map(|s| s.eq_ignore_ascii_case("zipfian"))
                .unwrap_or(false),
            batch: env_usize("ROCKS_DEPS_BATCH", 32).clamp(2, 512),
        }
    }

    pub fn dist_label(&self) -> &'static str {
        if self.zipfian {
            "zipfian"
        } else {
            "uniform"
        }
    }
}

/// One op of an atomic multi-CF batch (TiKV raftstore apply shape).
#[derive(Clone, Debug)]
pub enum CfWrite {
    Put {
        cf: &'static str,
        k: Vec<u8>,
        v: Vec<u8>,
    },
    Delete {
        cf: &'static str,
        k: Vec<u8>,
    },
}

/// CF layout modeled on TiKV's store: `default` (MVCC values), `write`
/// (commit index), `lock` (prewrite locks), plus `raftlog` (the raftdb
/// instance's own default CF, modeled as a dedicated CF).
pub const DEPS_CFS: &[&str] = &["write", "lock", "raftlog"];

/// Official 16 (RFC-0041) are the prefix. RFC-0043: this array only grows —
/// never delete a shape to lift `min_ratio`. Compare iterates this list.
pub const COMPARE_SHAPES: &[&str] = &[
    // official 16
    "ycsb_a",
    "ycsb_b",
    "ycsb_c",
    "ycsb_d",
    "ycsb_e",
    "ycsb_f",
    "deps_apply_batch",
    "deps_mvcc_latest",
    "deps_scan",
    "deps_raftlog",
    "deps_cache_overwrite",
    "ycsb_a_mc4",
    "ycsb_f_mc4",
    "deps_cache_overwrite_mc4",
    "deps_apply_batch_mc4",
    "deps_raftlog_mc4",
    // RFC-0043 HL₁
    "deps_lock_prewrite",
    "qs_hot_get",
    "qs_neg_lookup",
    "qs_batch_write",
    // RFC-0043 P2.3 — Kvrocks + MyRocks (docs/rocksdb-dependents-benchmarks.md)
    "kvrocks_get",
    "kvrocks_set",
    "kvrocks_pipelined_set",
    "kvrocks_scan",
    "kvrocks_set_mc50",
    "myrocks_point_select",
    "myrocks_read_only",
    "myrocks_write_tx",
    "linkbench_mix",
    // RFC-0043 P2.4 — SurrealDB kv-rocksdb / crud-bench-inspired
    "surreal_tx_get",
    "surreal_tx_put",
    "surreal_tx_rmw",
    "surreal_tx_scan",
    "surreal_tx_batch",
    "surreal_tx_rmw_mc8",
    "kvrocks_blob_set",
    // RFC-0043 P2.6 — expanding catalog (nebula / streaming / ceph / solana /
    // arango / venice / rockstore / oxigraph)
    "nebula_get_neighbors",
    "nebula_insert_edge",
    "flink_window_state",
    "kafka_changelog_flush",
    "bluestore_omap_write",
    "bluestore_omap_read",
    "solana_shred_append",
    "solana_trailing_read",
    "arango_doc_crud",
    "arango_traversal",
    "venice_fanout_get",
    "rockstore_widecol_rw",
    "oxigraph_spo_lookup",
    "oxigraph_triple_put",
    // RFC-0043 P2.7 — blocked-API (mixgraph / WBWI / compaction filter / ingest)
    "mixgraph_like",
    "wbwi_read_your_writes",
    "compaction_filter_drop",
    "ingest_sst",
    // RFC-0059 anti-overindex: uniform (no zipf hot set) + 2^20-key working
    // set — the official shapes' caches and windows must generalize.
    "ycsb_b_unif",
    "ycsb_c_unif",
    "ycsb_c_big",
];

/// Length of the RFC-0041 official prefix of [`COMPARE_SHAPES`].
pub const OFFICIAL_16: usize = 16;

/// Engine adapter. Both sides implement exactly these ops; the runner measures
/// only through this trait so the schedule cannot drift between engines.
pub trait Engine {
    fn label(&self) -> &'static str;
    /// Durability label surfaced in bench + compare JSON (honesty discipline).
    fn durability(&self) -> &'static str;
    fn sync(&self) -> bool;
    fn put(&self, k: &[u8], v: &[u8]) -> bool;
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()>;
    /// Lookup without materializing the value (kvrocks GET canary).
    fn get_probe(&self, k: &[u8]) -> Result<bool, ()> {
        Ok(self.get(k)?.is_some())
    }
    /// Count keys in `[start, end)` up to `cap` (short range scan).
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()>;
    /// Read-modify-write: bump last payload byte based on the current value,
    /// then a single write. Default = client get + one write.
    fn rmw(&self, k: &[u8], v: &[u8]) -> bool {
        let old = self.get(k).ok().flatten();
        let mut nv = v.to_vec();
        if let Some(last) = nv.last_mut() {
            *last = old
                .as_deref()
                .and_then(|o| o.last().copied())
                .unwrap_or(b'x')
                .wrapping_add(1);
        }
        self.put(k, &nv)
    }

    // ── deps suite (multi-CF shapes) ────────────────────────────────────────
    /// Point put into a named CF.
    fn put_cf(&self, cf: &str, k: &[u8], v: &[u8]) -> bool;
    /// Point get from a named CF.
    fn get_cf(&self, cf: &str, k: &[u8]) -> Result<Option<Vec<u8>>, ()>;
    /// One atomic multi-CF WriteBatch (TiKV apply-ready shape).
    fn batch(&self, ops: Vec<CfWrite>) -> bool;
    /// Pipeline of puts of the **same** payload (redis SET pipeline).
    /// Default clones `v` per key; compat interned `Bytes` (refcount).
    fn batch_put_same(&self, cf: &'static str, keys: &[Vec<u8>], v: &[u8]) -> bool {
        let mut wb = Vec::with_capacity(keys.len());
        for k in keys {
            wb.push(CfWrite::Put {
                cf,
                k: k.clone(),
                v: v.to_vec(),
            });
        }
        self.batch(wb)
    }
    /// Latest-version KEY for `prefix` in `cf`: reverse-seek from
    /// `prefix || u64::MAX` and take the first entry still under the prefix
    /// (TiKV MVCC latest read; the returned key carries the version suffix).
    fn latest_cf(&self, cf: &str, prefix: &[u8]) -> Result<Option<Vec<u8>>, ()>;
    /// `scan_count` for a named CF.
    fn scan_count_cf(&self, cf: &str, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()>;
    /// RFC-0035: zero latest/scan counters (compat only; default no-op).
    fn reset_read_probe(&self) {}
    /// RFC-0035: JSON object of counters + LSM shape, or `None`.
    fn read_probe_json(&self) -> Option<String> {
        None
    }
    /// Write-group stats when the engine is ConcurrentDb-backed.
    fn write_group_stats(&self) -> Option<(u64, u64, u64, u64)> {
        None
    }
    /// Live memtable entries (compat). RFC-0054 raftlog attribution.
    fn mem_entries(&self) -> Option<u64> {
        None
    }
    /// `PEDRA_WRITE_PHASE_STATS` dump, or `None`.
    fn write_phase_line(&self) -> Option<String> {
        None
    }
    /// Raw phase counters for per-shape deltas: `[commits, prepare, wal,
    /// mem, publish, flush, lock_wait]` ns (RFC-0054 P0.2).
    fn write_phase_snapshot(&self) -> Option<[u64; 7]> {
        None
    }
    /// Fold memtable tail (no SST). Returns tail length before fold.
    fn fold_mem_tail(&self) -> usize {
        0
    }
    /// Switch the Rocks peer's `WriteOptions.sync` for the next writes.
    /// Compat ignores this (always fdatasync). Used so MyRocks/Surreal
    /// match the **upper DB default**, not a forced async peer.
    fn set_write_sync(&self, _sync: bool) {}
    /// Untimed settle after seed (flush memtable). Default no-op.
    fn flush(&self) -> bool {
        true
    }
    /// Latest key under `prefix` in `latest_cf`, then get that key in `value_cf`.
    /// Default is the two calls; compat uses one mutex.
    fn latest_then_get_cf(
        &self,
        latest_cf: &str,
        prefix: &[u8],
        value_cf: &str,
    ) -> Result<Option<Vec<u8>>, ()> {
        match self.latest_cf(latest_cf, prefix)? {
            Some(k) => self.get_cf(value_cf, &k),
            None => Ok(None),
        }
    }

    /// RFC-0043 P2.7: `WriteBatchWithIndex` overlay then DB (read-your-writes).
    fn wbwi_overlay_get(&self, puts: &[(&[u8], &[u8])], key: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let _ = (puts, key);
        Err(())
    }

    /// RFC-0043 P2.7: `SstFileWriter` + `ingest_external_file`. Keys must be
    /// unique per call; the adapter sorts them.
    fn ingest_kvs(&self, kvs: &[(&[u8], &[u8])]) -> bool {
        let _ = kvs;
        false
    }

    /// RFC-0043 P2.7: compact, dropping keys that start with `prefix`
    /// (compaction-filter contract).
    fn compact_drop_prefix(&self, prefix: &[u8]) -> bool {
        let _ = prefix;
        false
    }
}

/// One optimistic transaction (SurrealDB `kv-rocksdb` / rust-rocksdb shape).
pub trait OccTxn {
    fn get(&mut self, k: &[u8]) -> Result<Option<Vec<u8>>, ()>;
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool;
    fn delete(&mut self, k: &[u8]) -> bool;
    fn scan_count(&mut self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()>;
    fn commit(&mut self) -> bool;
}

/// Engine that can begin an optimistic txn. Same schedule on both adapters.
pub trait OccEngine: Engine {
    fn with_txn<R>(&self, f: impl FnOnce(&mut dyn OccTxn) -> R) -> R;
}

pub struct YcsbRunner {
    cfg: Cfg,
    rng: u64,
    zipf_cdf: Vec<f64>,
}

impl YcsbRunner {
    pub fn new(cfg: Cfg) -> Self {
        // Deterministic rng — same schedule every run, both engines.
        let rng = 0x5EED_0001_u64;
        // Zipfian(theta=0.99) CDF over [0, records).
        let theta = 0.99_f64;
        let records = cfg.records;
        let zipf_cdf = {
            let mut s = 0.0_f64;
            let cdf = (0..records)
                .map(|i| {
                    s += 1.0 / (i as f64 + 1.0).powf(theta);
                    s
                })
                .collect::<Vec<_>>();
            let total = cdf.last().copied().unwrap_or(1.0);
            cdf.into_iter().map(|v| v / total).collect()
        };
        Self { cfg, rng, zipf_cdf }
    }

    fn pick(&self, rng: &mut u64, latest: usize) -> usize {
        let records = self.cfg.records;
        if !self.cfg.zipfian || latest == 0 {
            return (xorshift(rng) % records as u64) as usize;
        }
        // zipf over recency window (ycsb_d/e "latest" style access)
        let window = latest.min(records);
        let u = (xorshift(rng) >> 11) as f64 / (1u64 << 53) as f64;
        let target = u * self.zipf_cdf[window - 1];
        let idx = self.zipf_cdf[..window].partition_point(|&c| c < target);
        (records - window) + idx.min(window - 1)
    }

    /// Dependent-shaped suite (TiKV as reference dependent). Seeds an MVCC
    /// keyspace (2 versions/record, batched) then runs five shapes; returns
    /// bench JSON blocks in a fixed order.
    pub fn run_deps<E: Engine>(&mut self, e: &E) -> Vec<String> {
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'd'; self.cfg.payload];

        // RFC-0054: `ROCKS_PARITY_ONLY` on deps (same contract as kvrocks —
        // filtered runs are experiments, never official tables).
        let only: Option<Vec<String>> = std::env::var("ROCKS_PARITY_ONLY").ok().map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(String::from)
                .collect()
        });
        let want = |name: &str| only.as_ref().map_or(true, |v| v.iter().any(|x| x == name));
        let need_seed = want("deps_apply_batch")
            || want("deps_mvcc_latest")
            || want("deps_scan")
            || want("deps_lock_prewrite");

        // Seed: 2 versions per record via batched commits (not timed) —
        // prewrite rows (lock + default) then commit rows (write, lock del).
        let mut vers: Vec<u64> = vec![0; records];
        if need_seed {
            let t0 = Instant::now();
            for round in 0..2u64 {
                let mut i = 0usize;
                while i < records {
                    let take = (records - i).min(64);
                    let mut pre = Vec::with_capacity(take * 2);
                    let mut com = Vec::with_capacity(take * 2);
                    for j in 0..take {
                        let ts = round * records as u64 + (i + j) as u64 + 1;
                        vers[i + j] = ts;
                        pre.push(CfWrite::Put {
                            cf: "lock",
                            k: ukey(i + j),
                            v: b"l".to_vec(),
                        });
                        pre.push(CfWrite::Put {
                            cf: "default",
                            k: mvcc(i + j, ts),
                            v: yval.clone(),
                        });
                        com.push(CfWrite::Put {
                            cf: "write",
                            k: mvcc(i + j, ts),
                            v: b"c".to_vec(),
                        });
                        com.push(CfWrite::Delete {
                            cf: "lock",
                            k: ukey(i + j),
                        });
                    }
                    assert!(e.batch(std::mem::take(&mut pre)), "seed prewrite");
                    assert!(e.batch(std::mem::take(&mut com)), "seed commit");
                    i += take;
                }
            }
            eprintln!(
                "[rocks-parity] deps seed {records}×2 versions in {:.1}s",
                t0.elapsed().as_secs_f64()
            );
        } else {
            eprintln!("[rocks-parity] deps seed skipped (ROCKS_PARITY_ONLY)");
        }

        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(5);

        // 1. deps_apply_batch — raftstore apply: per logical op one ready =
        //    prewrite batch + commit batch (batch txns each).
        if want("deps_apply_batch") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut txns, mut errors) = (0u64, 0u64);
            let phase0 = e.write_phase_snapshot();
            let mut build_ns = Vec::with_capacity(cfg_ops);
            let t0 = Instant::now();
            for _ in 0..cfg_ops {
                let t = Instant::now();
                let mut picks = Vec::with_capacity(batch);
                for _ in 0..batch {
                    let u = self.pick(&mut rng, records);
                    vers[u] = vers[u].saturating_add(1);
                    picks.push((u, vers[u]));
                }
                let mut pre = Vec::with_capacity(batch * 2);
                let mut com = Vec::with_capacity(batch * 2);
                for &(u, ts) in &picks {
                    pre.push(CfWrite::Put {
                        cf: "lock",
                        k: ukey(u),
                        v: b"l".to_vec(),
                    });
                    pre.push(CfWrite::Put {
                        cf: "default",
                        k: mvcc(u, ts),
                        v: yval.clone(),
                    });
                    com.push(CfWrite::Put {
                        cf: "write",
                        k: mvcc(u, ts),
                        v: b"c".to_vec(),
                    });
                    com.push(CfWrite::Delete {
                        cf: "lock",
                        k: ukey(u),
                    });
                }
                let t_build = t.elapsed();
                let ok = e.batch(std::mem::take(&mut pre)) && e.batch(std::mem::take(&mut com));
                if ok {
                    txns += batch as u64;
                } else {
                    errors += 1;
                }
                build_ns.push(t_build.as_nanos());
                lats.push(ms(t));
            }
            build_ns.sort_unstable();
            let bp50 = build_ns[build_ns.len() / 2] as f64 / 1000.0;
            eprintln!("[rocks-parity] deps_apply_batch split p50 build={bp50:.2}µs");
            if let (Some(a), Some(b)) = (phase0, e.write_phase_snapshot()) {
                let n = b[0].saturating_sub(a[0]).max(1);
                let us = |d: u64| d as f64 / n as f64 / 1000.0;
                eprintln!(
                "[rocks-parity] deps_apply_batch phasesΔ (per commit, 2/op) prepare={:.2}µs wal={:.2}µs mem={:.2}µs publish={:.2}µs flsh={:.2}µs lock_wait={:.2}µs n={n}",
                us(b[1].saturating_sub(a[1])),
                us(b[2].saturating_sub(a[2])),
                us(b[3].saturating_sub(a[3])),
                us(b[4].saturating_sub(a[4])),
                us(b[5].saturating_sub(a[5])),
                us(b[6].saturating_sub(a[6])),
            );
            }
            blocks.push(summarize(
                "deps_apply_batch",
                cfg_ops,
                t0.elapsed(),
                &mut lats,
            ));
            eprintln!("[rocks-parity] deps_apply_batch done txns={txns} errors={errors}");
        }

        // 2. deps_mvcc_latest — point read of the latest version: reverse-seek
        //    write CF for the user prefix, then fetch the value in default.
        if want("deps_mvcc_latest") {
            e.reset_read_probe();
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut reads, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for _ in 0..cfg_ops {
                let t = Instant::now();
                let u = self.pick(&mut rng, records);
                match e.latest_then_get_cf("write", &ukey(u), "default") {
                    Ok(Some(_)) => reads += 1,
                    Ok(None) | Err(()) => errors += 1,
                }
                lats.push(ms(t));
            }
            blocks.push(summarize(
                "deps_mvcc_latest",
                cfg_ops,
                t0.elapsed(),
                &mut lats,
            ));
            let probe = e.read_probe_json().unwrap_or_else(|| "null".into());
            blocks.push(format!(
                r#"{{
    "name": "deps_mvcc_latest_split",
    "combined": true,
    "probe": {probe}
  }}"#
            ));
            eprintln!("[rocks-parity] deps_mvcc_latest done reads={reads} errors={errors}");
        }

        // 3. deps_scan — short range scan over user keys in the write CF
        //    (coprocessor / GC range shape).
        if want("deps_scan") {
            e.reset_read_probe();
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut scans, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for _ in 0..cfg_ops {
                let t = Instant::now();
                let u = self.pick(&mut rng, records);
                match e.scan_count_cf("write", &ukey(u), &ukey(u + 25), 25) {
                    Ok(_) => scans += 1,
                    Err(_) => errors += 1,
                }
                lats.push(ms(t));
            }
            blocks.push(summarize("deps_scan", cfg_ops, t0.elapsed(), &mut lats));
            let probe = e.read_probe_json().unwrap_or_else(|| "null".into());
            blocks.push(format!(
                r#"{{
    "name": "deps_scan_probe",
    "probe": {probe}
  }}"#
            ));
            eprintln!("[rocks-parity] deps_scan done scans={scans} errors={errors}");
        }

        // 4. deps_raftlog — raftdb append shape: batched sequential appends to
        //    the raftlog CF; every 8th op also reads the previous entry.
        if want("deps_raftlog") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut appends, mut reads, mut errors) = (0u64, 0u64, 0u64);
            let mut idx = 0u64;
            let mut build_ns = Vec::with_capacity(cfg_ops);
            let mut batch_ns = Vec::with_capacity(cfg_ops);
            eprintln!(
                "[rocks-parity] deps_raftlog enter mem_entries={}",
                e.mem_entries()
                    .map_or_else(|| "?".into(), |n| n.to_string())
            );
            // RFC-0054 P0.2 discriminator: untimed fold of the active tail
            // before the loop (does an empty tail recover isolated p50?).
            if std::env::var_os("ROCKS_DEPS_FOLD_TAIL").is_some() {
                let t = e.fold_mem_tail();
                eprintln!(
                    "[rocks-parity] deps_raftlog pre-fold tail={t} mem_entries={}",
                    e.mem_entries()
                        .map_or_else(|| "?".into(), |n| n.to_string())
                );
            }
            let phase0 = e.write_phase_snapshot();
            let t0 = Instant::now();
            for op in 0..cfg_ops {
                let t = Instant::now();
                let mut wb = Vec::with_capacity(16);
                for _ in 0..16 {
                    idx += 1;
                    wb.push(CfWrite::Put {
                        cf: "raftlog",
                        k: format!("raftlog/{idx:08}").into_bytes(),
                        v: yval.clone(),
                    });
                }
                let t_build = t.elapsed();
                let t_b = Instant::now();
                let ok = e.batch(std::mem::take(&mut wb));
                let t_batch = t_b.elapsed();
                if ok {
                    appends += 16;
                } else {
                    errors += 1;
                }
                if op % 8 == 0 && idx > 1 {
                    match e.get_cf("raftlog", format!("raftlog/{:08}", idx - 1).as_bytes()) {
                        Ok(_) => reads += 1,
                        Err(_) => errors += 1,
                    }
                }
                build_ns.push(t_build.as_nanos());
                batch_ns.push(t_batch.as_nanos());
                lats.push(ms(t));
            }
            blocks.push(summarize("deps_raftlog", cfg_ops, t0.elapsed(), &mut lats));
            build_ns.sort_unstable();
            batch_ns.sort_unstable();
            let p50 = |v: &[u128]| v[v.len() / 2] as f64 / 1000.0;
            eprintln!(
                "[rocks-parity] deps_raftlog split p50 build={:.2}µs batch={:.2}µs mem_after={}",
                p50(&build_ns),
                p50(&batch_ns),
                e.mem_entries()
                    .map_or_else(|| "?".into(), |n| n.to_string())
            );
            if let (Some(a), Some(b)) = (phase0, e.write_phase_snapshot()) {
                let n = b[0].saturating_sub(a[0]).max(1);
                let us = |d: u64| d as f64 / n as f64 / 1000.0;
                eprintln!(
                "[rocks-parity] deps_raftlog phasesΔ prepare={:.2}µs wal={:.2}µs mem={:.2}µs publish={:.2}µs flsh={:.2}µs lock_wait={:.2}µs n={n}",
                us(b[1].saturating_sub(a[1])),
                us(b[2].saturating_sub(a[2])),
                us(b[3].saturating_sub(a[3])),
                us(b[4].saturating_sub(a[4])),
                us(b[5].saturating_sub(a[5])),
                us(b[6].saturating_sub(a[6])),
            );
            }
            if let Some(line) = e.write_phase_line() {
                eprintln!("[rocks-parity] deps_raftlog phases {line}");
            }
            eprintln!(
                "[rocks-parity] deps_raftlog done appends={appends} reads={reads} errors={errors}"
            );
        }

        // 5. deps_cache_overwrite — unbatched zipf overwrite of a fixed
        //    keyspace (cache-style dependent; compat worst case).
        if want("deps_cache_overwrite") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut writes, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for _ in 0..cfg_ops {
                let t = Instant::now();
                let u = self.pick(&mut rng, records);
                if e.put(&format!("c/{u:06}").as_bytes(), &yval) {
                    writes += 1;
                } else {
                    errors += 1;
                }
                lats.push(ms(t));
            }
            blocks.push(summarize(
                "deps_cache_overwrite",
                cfg_ops,
                t0.elapsed(),
                &mut lats,
            ));
            eprintln!("[rocks-parity] deps_cache_overwrite done writes={writes} errors={errors}");
        }

        // 6. RFC-0043: TiKV prewrite-only ready (lock+default WriteBatch, no
        //    commit). Batched — HL, not a 1-op canary.
        if want("deps_lock_prewrite") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut txns, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for _ in 0..cfg_ops {
                let t = Instant::now();
                let mut pre = Vec::with_capacity(batch * 2);
                for _ in 0..batch {
                    let u = self.pick(&mut rng, records);
                    vers[u] = vers[u].saturating_add(1);
                    pre.push(CfWrite::Put {
                        cf: "lock",
                        k: ukey(u),
                        v: b"L".to_vec(),
                    });
                    pre.push(CfWrite::Put {
                        cf: "default",
                        k: mvcc(u, vers[u]),
                        v: yval.clone(),
                    });
                }
                if e.batch(std::mem::take(&mut pre)) {
                    txns += batch as u64;
                } else {
                    errors += 1;
                }
                lats.push(ms(t));
            }
            blocks.push(summarize(
                "deps_lock_prewrite",
                cfg_ops,
                t0.elapsed(),
                &mut lats,
            ));
            eprintln!("[rocks-parity] deps_lock_prewrite done txns={txns} errors={errors}");
            // Untimed: drop leftover prewrite locks so later assertions / reopen
            // see the same lock CF as before this shape existed.
            let cleanup: Vec<CfWrite> = (0..records)
                .map(|u| CfWrite::Delete {
                    cf: "lock",
                    k: ukey(u),
                })
                .collect();
            let _ = e.batch(cleanup);
        }

        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.1 — Quicksilver-inspired mixes (published numbers, not a
    /// production replay): hot working set, negative lookups, batched writes.
    pub fn run_qs<E: Engine>(&mut self, e: &E) -> Vec<String> {
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'q'; self.cfg.payload];
        let hot = records.div_ceil(10).max(16).min(records);
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(3);

        // qs_hot_get — 99% get on the hot 10%, 1% WriteBatch ≥32 on that set.
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut writes, mut errors) = (0u64, 0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            if i % 100 == 0 {
                let mut wb = Vec::with_capacity(batch);
                for _ in 0..batch {
                    let u = (xorshift(&mut rng) as usize) % hot;
                    wb.push(CfWrite::Put {
                        cf: "default",
                        k: ykey(u),
                        v: yval.clone(),
                    });
                }
                if e.batch(std::mem::take(&mut wb)) {
                    writes += batch as u64;
                } else {
                    errors += 1;
                }
            } else {
                let u = (xorshift(&mut rng) as usize) % hot;
                match e.get(&ykey(u)) {
                    Ok(_) => gets += 1,
                    Err(()) => errors += 1,
                }
            }
            lats.push(ms(t));
        }
        blocks.push(summarize("qs_hot_get", cfg_ops, t0.elapsed(), &mut lats));
        eprintln!("[rocks-parity] qs_hot_get done gets={gets} writes={writes} errors={errors}");

        // qs_neg_lookup — gets past the keyspace (QS: ~10× more misses).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut misses, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = records + (xorshift(&mut rng) as usize % records.max(1));
            match e.get(&ykey(u)) {
                Ok(None) => misses += 1,
                Ok(Some(_)) => {}
                Err(()) => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize("qs_neg_lookup", cfg_ops, t0.elapsed(), &mut lats));
        eprintln!("[rocks-parity] qs_neg_lookup done misses={misses} errors={errors}");

        // qs_batch_write — every op is one batched put (QS root write).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(batch);
            for _ in 0..batch {
                let u = (xorshift(&mut rng) as usize) % records;
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: ykey(u),
                    v: yval.clone(),
                });
            }
            if e.batch(std::mem::take(&mut wb)) {
                puts += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "qs_batch_write",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] qs_batch_write done puts={puts} errors={errors}");

        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.3 — Apache Kvrocks (Redis protocol on RocksDB).
    /// GET/SET = redis-benchmark 1-op (canary). Pipeline = N SETs as one
    /// WriteBatch (HL). SCAN = short prefix walk (HL). Provenance:
    /// `docs/rocksdb-dependents-benchmarks.md` §Kvrocks; apache/kvrocks#389
    /// + kvrocks.apache.org/blog/how-we-use-rocksdb-in-kvrocks/.
    pub fn run_kvrocks<E: Engine + Sync>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("kvrocks"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'k'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(4);

        // Untimed Redis-string keyspace (independent of ycsb/).
        let ktab: Vec<Vec<u8>> = (0..records + 25).map(kkey).collect();
        for i in 0..records {
            assert!(e.put(&ktab[i], &yval), "kvrocks seed {i}");
        }
        // Warm the read path (seed already auto-flushes at 4 MiB). An extra
        // `flush()` here sent every later SET to a fresh mem+SST and
        // tanked kvrocks_set / pipeline.
        for i in 0..records {
            let _ = e.get(&ktab[i]);
        }

        // RFC-0044 P0.5 A/B tool: `ROCKS_PARITY_ONLY=csv` runs a subset of
        // shapes. Filtering changes the rng stream and db state, so filtered
        // runs are for experiments only — never official tables.
        let only: Option<Vec<String>> = std::env::var("ROCKS_PARITY_ONLY").ok().map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(String::from)
                .collect()
        });
        let want = |name: &str| only.as_ref().map_or(true, |v| v.iter().any(|x| x == name));

        // Materialise the zipf stream before the timed window so the ratio
        // is LSM/CPU, not `format!` (RFC-0044 P1 — same schedule both engines).
        let get_idx: Vec<usize> = if want("kvrocks_get") {
            (0..cfg_ops).map(|_| self.pick(&mut rng, records)).collect()
        } else {
            Vec::new()
        };
        let set_idx: Vec<usize> = if want("kvrocks_set") {
            (0..cfg_ops).map(|_| self.pick(&mut rng, records)).collect()
        } else {
            Vec::new()
        };
        let mut pipe: Vec<Vec<Vec<u8>>> = Vec::with_capacity(cfg_ops);
        if want("kvrocks_pipelined_set") {
            for _ in 0..cfg_ops {
                let mut keys = Vec::with_capacity(batch);
                for _ in 0..batch {
                    keys.push(ktab[self.pick(&mut rng, records)].clone());
                }
                pipe.push(keys);
            }
        }
        let scan_idx: Vec<usize> = if want("kvrocks_scan") {
            (0..cfg_ops).map(|_| self.pick(&mut rng, records)).collect()
        } else {
            Vec::new()
        };

        // kvrocks_get — redis-benchmark GET (1-op canary).
        if want("kvrocks_get") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut gets, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for &u in &get_idx {
                let t = Instant::now();
                match e.get_probe(&ktab[u]) {
                    Ok(_) => gets += 1,
                    Err(()) => errors += 1,
                }
                lats.push(ms(t));
            }
            blocks.push(summarize("kvrocks_get", cfg_ops, t0.elapsed(), &mut lats));
            eprintln!("[rocks-parity] kvrocks_get done gets={gets} errors={errors}");
        }

        // kvrocks_set — redis-benchmark SET (1-op canary).
        if want("kvrocks_set") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut sets, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for &u in &set_idx {
                let t = Instant::now();
                if e.put(&ktab[u], &yval) {
                    sets += 1;
                } else {
                    errors += 1;
                }
                lats.push(ms(t));
            }
            blocks.push(summarize("kvrocks_set", cfg_ops, t0.elapsed(), &mut lats));
            eprintln!("[rocks-parity] kvrocks_set done sets={sets} errors={errors}");
        }

        // kvrocks_pipelined_set — pipeline of `batch` SETs → 1 WriteBatch (HL).
        if want("kvrocks_pipelined_set") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut puts, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for keys in &pipe {
                let t = Instant::now();
                if e.batch_put_same("default", keys, &yval) {
                    puts += batch as u64;
                } else {
                    errors += 1;
                }
                lats.push(ms(t));
            }
            blocks.push(summarize(
                "kvrocks_pipelined_set",
                cfg_ops,
                t0.elapsed(),
                &mut lats,
            ));
            eprintln!("[rocks-parity] kvrocks_pipelined_set done puts={puts} errors={errors}");
        }

        // kvrocks_scan — Redis SCAN COUNT=25 over a window (HL).
        if want("kvrocks_scan") {
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut scans, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for &u in &scan_idx {
                let t = Instant::now();
                match e.scan_count(&ktab[u], &ktab[u + 25], 25) {
                    Ok(_) => scans += 1,
                    Err(_) => errors += 1,
                }
                lats.push(ms(t));
            }
            blocks.push(summarize("kvrocks_scan", cfg_ops, t0.elapsed(), &mut lats));
            eprintln!("[rocks-parity] kvrocks_scan done scans={scans} errors={errors}");
        }

        // kvrocks_blob_set — Kvrocks BlobDB-sized value (16 KiB; their post
        // cites 10–50 KB). Independent keyspace so the 1 KB SET path stays
        // the redis-benchmark canary.
        if want("kvrocks_blob_set") {
            let blob = vec![b'B'; 16 * 1024];
            let blob_n = records.min(256);
            for i in 0..blob_n {
                let _ = e.put(&bkey(i), &blob);
            }
            // `pick(blob_n)` is zipf over the last `blob_n` of `records` (same
            // as before pregen) — not `0..blob_n`.
            let blob_keys: Vec<Vec<u8>> = (0..cfg_ops)
                .map(|_| bkey(self.pick(&mut rng, blob_n)))
                .collect();
            let mut lats = Vec::with_capacity(cfg_ops);
            let (mut sets, mut errors) = (0u64, 0u64);
            let t0 = Instant::now();
            for k in &blob_keys {
                let t = Instant::now();
                if e.put(k, &blob) {
                    sets += 1;
                } else {
                    errors += 1;
                }
                lats.push(ms(t));
            }
            blocks.push(summarize(
                "kvrocks_blob_set",
                cfg_ops,
                t0.elapsed(),
                &mut lats,
            ));
            eprintln!("[rocks-parity] kvrocks_blob_set done sets={sets} errors={errors}");
        }

        self.rng = rng;
        // kvrocks_set_mc50 — see comment below the shapes.
        if want("kvrocks_set_mc50") {
            // redis-benchmark default concurrency: 50 SET connections, no
            // pipeline. The 1-client kvrocks_set above is the G1 canary (1 fd per
            // SET — its ratio vs the async peer is durability-skewed by physics).
            // Here the write group shares one fdatasync across waiting writers.
            blocks.extend(self.run_kvrocks_set_clients(e, 50));
        }
        blocks
    }

    /// `kvrocks_set_mc{clients}` — redis-benchmark `-c clients -t set` shape:
    /// barrier-aligned client threads, each running `cfg.ops` zipf SETs over
    /// the seeded Redis-string keyspace. Aggregate block, per-op latencies.
    pub fn run_kvrocks_set_clients<E: Engine + Sync>(&self, e: &E, clients: usize) -> Vec<String> {
        assert!(clients >= 2, "multi-client harness needs >= 2 clients");
        let cfg_ops = self.cfg.ops;
        let records = self.cfg.records;
        let yval = std::sync::Arc::new(vec![b'k'; self.cfg.payload]);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(clients));
        let t0 = Instant::now();
        let mut lats = Vec::with_capacity(cfg_ops * clients);
        let mut errors = 0u64;
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..clients)
                .map(|c| {
                    let barrier = barrier.clone();
                    let yval = yval.clone();
                    s.spawn(move || {
                        let mut rng =
                            0x5EED_0001_u64.wrapping_mul((c as u64) + 0x9E37) ^ (c as u64);
                        let mut lats = Vec::with_capacity(cfg_ops);
                        let mut errors = 0u64;
                        barrier.wait();
                        for _ in 0..cfg_ops {
                            let t = Instant::now();
                            let u = self.pick(&mut rng, records);
                            if !e.put(&kkey(u), &yval) {
                                errors += 1;
                            }
                            lats.push(ms(t));
                        }
                        (lats, errors)
                    })
                })
                .collect();
            for h in handles {
                let (mut l, err) = h.join().expect("kvrocks set client");
                errors += err;
                lats.append(&mut l);
            }
        });
        let name = format!("kvrocks_set_mc{clients}");
        let block = summarize_mc(
            &name,
            cfg_ops * clients,
            t0.elapsed(),
            &mut lats,
            clients,
            errors,
        );
        eprintln!(
            "[rocks-parity] {name} done ops={} errors={errors}",
            cfg_ops * clients
        );
        if let Some((sub, queued, groups, gops)) = e.write_group_stats() {
            let avg = if groups == 0 {
                0.0
            } else {
                gops as f64 / groups as f64
            };
            eprintln!(
                "[rocks-parity] write_group submits={sub} queued={queued} groups={groups} ops={gops} avg_group={avg:.2}"
            );
        }
        vec![block]
    }

    /// RFC-0043 P2.3 — MyRocks (MySQL + RocksDB) + LinkBench-inspired mix.
    /// `myrocks_point_select` = sysbench oltp_point_select (canary).
    /// `myrocks_read_only` = short PK range (oltp_read_only).
    /// `myrocks_write_tx` = one OLTP tx = N row updates as WriteBatch (HL).
    /// `linkbench_mix` = Facebook social-graph mix (scan-heavy GET_LINKS_LIST).
    /// Provenance: Percona sysbench-tpcc / Small Datum / VLDB MyRocks;
    /// `docs/rocksdb-dependents-benchmarks.md` §MyRocks.
    pub fn run_myrocks<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("myrocks"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'm'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(4);

        // Untimed: one node + 4 outgoing links per record (LinkBench seed).
        for i in 0..records {
            assert!(e.put(&nkey(i), &yval), "myrocks node seed {i}");
            for d in 1..=4 {
                let dst = (i + d) % records;
                assert!(e.put(&lkey(i, dst), &yval), "myrocks link seed");
            }
        }

        // myrocks_point_select — sysbench oltp_point_select (1-op canary).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            match e.get(&nkey(u)) {
                Ok(_) => gets += 1,
                Err(()) => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "myrocks_point_select",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] myrocks_point_select done gets={gets} errors={errors}");

        // myrocks_read_only — sysbench oltp_read_only short PK range (HL).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut scans, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            match e.scan_count(&nkey(u), &nkey(u + 25), 25) {
                Ok(_) => scans += 1,
                Err(_) => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "myrocks_read_only",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] myrocks_read_only done scans={scans} errors={errors}");

        // myrocks_write_tx — one OLTP tx = `batch` row updates, one WriteBatch.
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut rows, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(batch);
            for _ in 0..batch {
                let u = self.pick(&mut rng, records);
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: nkey(u),
                    v: yval.clone(),
                });
            }
            if e.batch(std::mem::take(&mut wb)) {
                rows += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "myrocks_write_tx",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] myrocks_write_tx done rows={rows} errors={errors}");

        // linkbench_mix — inspired by LinkBench proportions, not a replay:
        //   55% GET_LINKS_LIST (prefix scan), 15% GET_NODE,
        //   25% ADD/UPDATE_LINK as a 4-put batch, 5% DELETE_LINK.
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut scans, mut gets, mut writes, mut deletes, mut errors) =
            (0u64, 0u64, 0u64, 0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let roll = xorshift(&mut rng) % 100;
            let u = self.pick(&mut rng, records);
            let ok = if roll < 55 {
                match e.scan_count(&lprefix(u), &lprefix(u + 1), 8) {
                    Ok(_) => {
                        scans += 1;
                        true
                    }
                    Err(_) => false,
                }
            } else if roll < 70 {
                match e.get(&nkey(u)) {
                    Ok(_) => {
                        gets += 1;
                        true
                    }
                    Err(()) => false,
                }
            } else if roll < 95 {
                let mut wb = Vec::with_capacity(4);
                for _ in 0..4 {
                    let dst = self.pick(&mut rng, records);
                    wb.push(CfWrite::Put {
                        cf: "default",
                        k: lkey(u, dst),
                        v: yval.clone(),
                    });
                }
                if e.batch(std::mem::take(&mut wb)) {
                    writes += 4;
                    true
                } else {
                    false
                }
            } else {
                let dst = self.pick(&mut rng, records);
                if e.batch(vec![CfWrite::Delete {
                    cf: "default",
                    k: lkey(u, dst),
                }]) {
                    deletes += 1;
                    true
                } else {
                    false
                }
            };
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize("linkbench_mix", cfg_ops, t0.elapsed(), &mut lats));
        eprintln!(
            "[rocks-parity] linkbench_mix done scans={scans} gets={gets} writes={writes} deletes={deletes} errors={errors}"
        );

        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.4 — SurrealDB `kv-rocksdb` / crud-bench-inspired.
    /// Every op is one optimistic txn (`transaction_opt` + snapshot + commit).
    /// Provenance: surrealdb/core/src/kvs/rocksdb/mod.rs; crud-bench.
    pub fn run_surreal<E: OccEngine + Sync>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("surreal"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b's'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(5);

        for i in 0..records {
            assert!(e.put(&skey(i), &yval), "surreal seed {i}");
        }
        for i in 0..records {
            let _ = e.get(&skey(i));
        }

        // surreal_tx_get — read-only txn (snapshot get + commit). HL.
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let ok = e.with_txn(|tx| match tx.get(&skey(u)) {
                Ok(_) => {
                    gets += 1;
                    tx.commit()
                }
                Err(()) => false,
            });
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "surreal_tx_get",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] surreal_tx_get done gets={gets} errors={errors}");

        // surreal_tx_put — 1 put + commit (canary: one fd/Ok).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let ok = e.with_txn(|tx| {
                if tx.put(&skey(u), &yval) {
                    puts += 1;
                    tx.commit()
                } else {
                    false
                }
            });
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "surreal_tx_put",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] surreal_tx_put done puts={puts} errors={errors}");

        // surreal_tx_rmw — SurrealQL UPDATE: get + put + one commit (HL).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut rmws, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let ok = e.with_txn(|tx| {
                let old = tx.get(&skey(u)).ok().flatten();
                let mut nv = yval.clone();
                if let Some(last) = nv.last_mut() {
                    *last = old
                        .as_deref()
                        .and_then(|o| o.last().copied())
                        .unwrap_or(b's')
                        .wrapping_add(1);
                }
                if tx.put(&skey(u), &nv) {
                    rmws += 1;
                    tx.commit()
                } else {
                    false
                }
            });
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "surreal_tx_rmw",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] surreal_tx_rmw done rmws={rmws} errors={errors}");

        // surreal_tx_scan — snapshot range + commit (HL).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut scans, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let ok = e.with_txn(|tx| match tx.scan_count(&skey(u), &skey(u + 25), 25) {
                Ok(_) => {
                    scans += 1;
                    tx.commit()
                }
                Err(()) => false,
            });
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "surreal_tx_scan",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] surreal_tx_scan done scans={scans} errors={errors}");

        // surreal_tx_batch — crud-bench insert: N puts, one commit (HL).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut rows, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let ok = e.with_txn(|tx| {
                for _ in 0..batch {
                    let u = self.pick(&mut rng, records);
                    if !tx.put(&skey(u), &yval) {
                        return false;
                    }
                    rows += 1;
                }
                tx.commit()
            });
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "surreal_tx_batch",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] surreal_tx_batch done rows={rows} errors={errors}");

        self.rng = rng;
        // crud-bench is concurrent; 1c rmw misses OCC conflict + group commit.
        blocks.extend(self.run_surreal_rmw_clients(e, 8));
        blocks
    }

    /// `surreal_tx_rmw_mc{clients}` — concurrent UPDATE with Busy retry.
    /// Each client runs `cfg.ops` zipf RMWs; commit false → retry up to 8×.
    pub fn run_surreal_rmw_clients<E: OccEngine + Sync>(
        &self,
        e: &E,
        clients: usize,
    ) -> Vec<String> {
        assert!(clients >= 2, "multi-client harness needs >= 2 clients");
        let cfg_ops = self.cfg.ops;
        let records = self.cfg.records;
        let yval = std::sync::Arc::new(vec![b's'; self.cfg.payload]);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(clients));
        let t0 = Instant::now();
        let mut lats = Vec::with_capacity(cfg_ops * clients);
        let mut errors = 0u64;
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..clients)
                .map(|c| {
                    let barrier = barrier.clone();
                    let yval = yval.clone();
                    s.spawn(move || {
                        let mut rng =
                            0x51C8_0001_u64.wrapping_mul((c as u64) + 0x9E37) ^ (c as u64);
                        let mut lats = Vec::with_capacity(cfg_ops);
                        let mut errors = 0u64;
                        barrier.wait();
                        for _ in 0..cfg_ops {
                            let t = Instant::now();
                            let u = self.pick(&mut rng, records);
                            let mut ok = false;
                            for _ in 0..32 {
                                ok = e.with_txn(|tx| {
                                    let old = tx.get(&skey(u)).ok().flatten();
                                    let mut nv = yval.as_ref().clone();
                                    if let Some(last) = nv.last_mut() {
                                        *last = old
                                            .as_deref()
                                            .and_then(|o| o.last().copied())
                                            .unwrap_or(b's')
                                            .wrapping_add(1);
                                    }
                                    tx.put(&skey(u), &nv) && tx.commit()
                                });
                                if ok {
                                    break;
                                }
                            }
                            if !ok {
                                errors += 1;
                            }
                            lats.push(ms(t));
                        }
                        (lats, errors)
                    })
                })
                .collect();
            for h in handles {
                let (mut l, err) = h.join().expect("surreal rmw client");
                errors += err;
                lats.append(&mut l);
            }
        });
        let name = format!("surreal_tx_rmw_mc{clients}");
        let block = summarize_mc(
            &name,
            cfg_ops * clients,
            t0.elapsed(),
            &mut lats,
            clients,
            errors,
        );
        eprintln!(
            "[rocks-parity] {name} done ops={} errors={errors}",
            cfg_ops * clients
        );
        vec![block]
    }

    /// RFC-0043 P2.6 — NebulaGraph storage: vertex prefix-scan (GO 1-hop)
    /// + edge insert batch (LDBC/nebula-bench inspired).
    pub fn run_nebula<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("nebula"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'n'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        for i in 0..records {
            for d in 1..=4 {
                let dst = (i + d) % records;
                assert!(e.put(&ekey(i, dst), &yval), "nebula seed edge");
            }
        }
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut scans, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            match e.scan_count(&eprefix(u), &eprefix(u + 1), 25) {
                Ok(_) => scans += 1,
                Err(_) => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "nebula_get_neighbors",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] nebula_get_neighbors done scans={scans} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(batch);
            for _ in 0..batch {
                let src = self.pick(&mut rng, records);
                let dst = self.pick(&mut rng, records);
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: ekey(src, dst),
                    v: yval.clone(),
                });
            }
            if e.batch(std::mem::take(&mut wb)) {
                puts += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "nebula_insert_edge",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] nebula_insert_edge done puts={puts} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.6 — Flink window state + Kafka Streams changelog flush
    /// (Nexmark-inspired; not a JVM replay).
    pub fn run_streaming<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("streaming"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'f'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        let windows = (records / 16).max(1);
        for i in 0..records {
            let win = i % windows;
            assert!(e.put(&wkey(win, i), &yval), "flink seed {i}");
        }
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut ops, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let win = u % windows;
            let ok = e.put(&wkey(win, records + i), &yval)
                && e.scan_count(&wprefix(win), &wprefix(win + 1), 25).is_ok();
            if ok {
                ops += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "flink_window_state",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] flink_window_state done ops={ops} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(batch);
            for j in 0..batch {
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: format!("ch/{i:06}/{j:04}").into_bytes(),
                    v: yval.clone(),
                });
            }
            let ok = e.batch(std::mem::take(&mut wb)) && e.flush();
            if ok {
                puts += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "kafka_changelog_flush",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] kafka_changelog_flush done puts={puts} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.6 — Ceph BlueStore omap (the published kv bottleneck).
    /// Host default = durable metadata (sync on commit).
    pub fn run_ceph<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("ceph"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'c'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        for i in 0..records {
            assert!(e.put(&okey(i), &yval), "ceph omap seed {i}");
        }
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(batch);
            for _ in 0..batch {
                let u = self.pick(&mut rng, records);
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: okey(u),
                    v: yval.clone(),
                });
            }
            if e.batch(std::mem::take(&mut wb)) {
                puts += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "bluestore_omap_write",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] bluestore_omap_write done puts={puts} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let ok = e.get(&okey(u)).is_ok()
                && e.scan_count(&okey(u), &okey(u.saturating_add(8)), 8)
                    .is_ok();
            if ok {
                gets += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "bluestore_omap_read",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] bluestore_omap_read done gets={gets} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.6 — Solana/Agave blockstore: shred append + trailing slot read.
    pub fn run_solana<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("solana"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let yval = vec![b'S'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        for i in 0..records {
            assert!(e.put(&shred(i), &yval), "solana shred seed {i}");
        }
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        let mut idx = records as u64;
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(16);
            for _ in 0..16 {
                idx += 1;
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: shred(idx as usize),
                    v: yval.clone(),
                });
            }
            if e.batch(std::mem::take(&mut wb)) {
                puts += 16;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "solana_shred_append",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] solana_shred_append done puts={puts} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut scans, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records).max(25);
            match e.scan_count(&shred(u - 25), &shred(u + 1), 25) {
                Ok(_) => scans += 1,
                Err(_) => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "solana_trailing_read",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] solana_trailing_read done scans={scans} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.6 — ArangoDB document CRUD + k-hop traversal (scan chain).
    pub fn run_arango<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("arango"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let yval = vec![b'a'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        for i in 0..records {
            assert!(e.put(&dkey(i), &yval), "arango doc seed {i}");
            for d in 1..=3 {
                let dst = (i + d) % records;
                let _ = e.put(&ekey(i, dst), &yval);
            }
        }
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut ops, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let r = xorshift(&mut rng) % 100;
            let ok = if r < 50 {
                e.get(&dkey(u)).is_ok()
            } else if r < 80 {
                e.put(&dkey(u), &yval)
            } else {
                e.scan_count(&dkey(u), &dkey(u.saturating_add(5)), 5)
                    .is_ok()
            };
            if ok {
                ops += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "arango_doc_crud",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] arango_doc_crud done ops={ops} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut hops, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records).min(records.saturating_sub(2));
            // 2-hop: neighbors of u, then of u+1. No wrap: wrap made
            // start>end and BTreeMap panics on the range.
            let ok = e.scan_count(&eprefix(u), &eprefix(u + 1), 8).is_ok()
                && e.scan_count(&eprefix(u + 1), &eprefix(u + 2), 8).is_ok();
            if ok {
                hops += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "arango_traversal",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] arango_traversal done hops={hops} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.6 — Venice fanout get (scaled: 32 point-gets / op, not 5k)
    /// + Pinterest Rockstore wide-column (row+col+ts).
    pub fn run_venice<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("venice"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let yval = vec![b'V'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        for i in 0..records {
            assert!(e.put(&dkey(i), &yval), "venice seed {i}");
            for col in 0..4u16 {
                let _ = e.put(&colkey(i, col, 1), &yval);
            }
        }
        const FANOUT: usize = 32;
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut ok = true;
            for _ in 0..FANOUT {
                let u = self.pick(&mut rng, records);
                if e.get(&dkey(u)).is_err() {
                    ok = false;
                    break;
                }
                gets += 1;
            }
            if !ok {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "venice_fanout_get",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] venice_fanout_get done gets={gets} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut ops, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            let row = self.pick(&mut rng, records);
            let col = (xorshift(&mut rng) % 4) as u16;
            let ok = if i % 2 == 0 {
                e.put(&colkey(row, col, (i as u64) + 2), &yval)
            } else {
                e.scan_count(&colprefix(row, col), &colprefix(row, col + 1), 8)
                    .is_ok()
            };
            if ok {
                ops += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "rockstore_widecol_rw",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] rockstore_widecol_rw done ops={ops} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.6 — Oxigraph still ships RocksDB (feature `rocksdb`, 0.5.x
    /// 2026). Shape is SPO point lookup + triple insert batch, not SPARQL.
    pub fn run_oxigraph<E: Engine>(&mut self, e: &E) -> Vec<String> {
        e.set_write_sync(write_sync_for_suite("oxigraph"));
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let batch = self.cfg.batch;
        let yval = vec![b'x'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(2);
        for i in 0..records {
            assert!(e.put(&tkey(i, 0, i), &yval), "oxigraph seed {i}");
        }
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            match e.get(&tkey(u, 0, u)) {
                Ok(_) => gets += 1,
                Err(()) => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "oxigraph_spo_lookup",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] oxigraph_spo_lookup done gets={gets} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut puts, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let mut wb = Vec::with_capacity(batch);
            for p in 0..batch {
                let s = self.pick(&mut rng, records);
                let o = self.pick(&mut rng, records);
                wb.push(CfWrite::Put {
                    cf: "default",
                    k: tkey(s, (p % 8) as u16, o),
                    v: yval.clone(),
                });
            }
            if e.batch(std::mem::take(&mut wb)) {
                puts += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "oxigraph_triple_put",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] oxigraph_triple_put done puts={puts} errors={errors}");
        self.rng = rng;
        blocks
    }

    /// RFC-0043 P2.7 — rust-rocksdb APIs that were catalog-`blocked`:
    /// mixgraph-like put/get/seek mix, `WriteBatchWithIndex`, compaction
    /// filter, `SstFileWriter`+ingest. Opt-in `ROCKS_PARITY_SUITE=rocksapi`.
    pub fn run_rocksapi<E: Engine>(&mut self, e: &E) -> Vec<String> {
        let records = self.cfg.records;
        let cfg_ops = self.cfg.ops;
        let yval = vec![b'r'; self.cfg.payload];
        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(4);

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut ops_ok, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let u = self.pick(&mut rng, records);
            let k = ykey(u);
            let k2 = ykey((u + 1) % records);
            let ok_put = e.put(&k, &yval);
            let ok_g1 = e.get(&k).is_ok();
            let ok_g2 = e.get(&k2).is_ok();
            let end = {
                let mut e = k.clone();
                e.push(0xff);
                e
            };
            let ok_s = e.scan_count(&k, &end, 8).is_ok();
            if ok_put && ok_g1 && ok_g2 && ok_s {
                ops_ok += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize("mixgraph_like", cfg_ops, t0.elapsed(), &mut lats));
        eprintln!("[rocks-parity] mixgraph_like done ops={ops_ok} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut gets, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            let k = format!("wbwi/{i:08}").into_bytes();
            let overlay = [(&k[..], yval.as_slice())];
            match e.wbwi_overlay_get(&overlay, &k) {
                Ok(Some(v)) if v == yval => gets += 1,
                _ => errors += 1,
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "wbwi_read_your_writes",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] wbwi_read_your_writes done gets={gets} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut drops, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            let keep = format!("keep/{i:08}").into_bytes();
            let drop = format!("drop/{i:08}").into_bytes();
            let put_ok = e.put(&keep, &yval) && e.put(&drop, &yval);
            let compact_ok = e.flush() && e.compact_drop_prefix(b"drop/");
            let kept = e.get(&keep).ok().flatten().as_deref() == Some(yval.as_slice());
            let gone = matches!(e.get(&drop), Ok(None));
            if put_ok && compact_ok && kept && gone {
                drops += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "compaction_filter_drop",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] compaction_filter_drop done drops={drops} errors={errors}");

        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut ingest, mut errors) = (0u64, 0u64);
        let t0 = Instant::now();
        for i in 0..cfg_ops {
            let t = Instant::now();
            let k = format!("ing/{i:08}").into_bytes();
            let pair = [(&k[..], yval.as_slice())];
            if e.ingest_kvs(&pair) && e.get(&k).ok().flatten().as_deref() == Some(yval.as_slice()) {
                ingest += 1;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize("ingest_sst", cfg_ops, t0.elapsed(), &mut lats));
        eprintln!("[rocks-parity] ingest_sst done ingest={ingest} errors={errors}");

        self.rng = rng;
        blocks
    }

    pub fn seed<E: Engine>(&mut self, e: &E) {
        let val = vec![b'y'; self.cfg.payload];
        for i in 0..self.cfg.records {
            assert!(e.put(&ykey(i), &val), "seed put {i}");
        }
        // Same as kvrocks: warm TLS / point-cache before the timed window.
        for i in 0..self.cfg.records {
            let _ = e.get(&ykey(i));
        }
    }

    /// Run one workload; returns the bench JSON block (same schema as the
    /// Montanha fdb-bench summarize).
    #[allow(clippy::too_many_arguments)]
    pub fn run<E: Engine>(
        &mut self,
        e: &E,
        name: &str,
        read_pct: u64,
        insert_pct: u64,
        rmw: bool,
        scans: bool,
    ) -> String {
        let cfg_ops = self.cfg.ops;
        let records = self.cfg.records;
        let payload = self.cfg.payload;
        let yval = vec![b'y'; payload];
        let ytab = YKeys::new(records + cfg_ops + 25);
        let mut lats = Vec::with_capacity(cfg_ops);
        let mut updates = 0u64;
        let mut inserts = 0u64;
        let mut scan_ops = 0u64;
        let mut errors = 0u64;
        let mut latest = records;
        let mut rng = std::mem::take(&mut self.rng);
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let roll = xorshift(&mut rng) % 100;
            if roll < read_pct {
                let i = self.pick(&mut rng, latest);
                if e.get_probe(ytab.key(i)).is_err() {
                    errors += 1;
                }
            } else if roll < read_pct + insert_pct {
                // insert (new key) → read-latest window grows
                if e.put(ytab.key(latest), &yval) {
                    latest += 1;
                    inserts += 1;
                } else {
                    errors += 1;
                }
            } else if scans {
                // short range scan: [key(i), key(i)+25) window, capped at 25
                let i = self.pick(&mut rng, latest);
                let start = ytab.key(i);
                let mut end = ytab.key(i + 25).to_vec();
                end.pop();
                end.push(b'~');
                match e.scan_count(start, &end, 25) {
                    Ok(_) => scan_ops += 1,
                    Err(_) => errors += 1,
                }
            } else if rmw {
                let i = self.pick(&mut rng, latest);
                if e.rmw(ytab.key(i), &yval) {
                    updates += 1;
                } else {
                    errors += 1;
                }
            } else {
                // update
                let i = self.pick(&mut rng, latest);
                if e.put(ytab.key(i), &yval) {
                    updates += 1;
                } else {
                    errors += 1;
                }
            }
            lats.push(ms(t));
        }
        self.rng = rng;
        let wall = t0.elapsed();
        let block = summarize(name, cfg_ops, wall, &mut lats);
        eprintln!(
            "[rocks-parity] {name} done ops={cfg_ops} updates={updates} inserts={inserts} scans={scan_ops} errors={errors}"
        );
        block
    }

    /// RFC-0059 anti-overindex: same mix as [`Self::run`] but with the
    /// distribution forced (uniform = no zipf hot set — the TLS/point caches
    /// cannot lean on a hot working set). Existing callers stay zipf-default.
    pub fn run_dist<E: Engine>(
        &mut self,
        e: &E,
        name: &str,
        read_pct: u64,
        insert_pct: u64,
        rmw: bool,
        scans: bool,
        uniform: bool,
    ) -> String {
        let saved = self.cfg.zipfian;
        self.cfg.zipfian = !uniform;
        let block = self.run(e, name, read_pct, insert_pct, rmw, scans);
        self.cfg.zipfian = saved;
        block
    }

    /// RFC-0059 anti-overindex: 100% uniform GETs over a 2^20-key working
    /// set (≈1000× the official `records=1024`) — cache sizing and read path
    /// must generalize past the official hot windows. Seed is untimed; the
    /// measured loop is `cfg.ops` uniform point reads. Skip with
    /// `ROCKS_PARITY_BIG=0`.
    pub fn run_c_big<E: Engine>(&mut self, e: &E) -> Option<String> {
        if std::env::var("ROCKS_PARITY_BIG").as_deref() == Ok("0") {
            return None;
        }
        let cfg_ops = self.cfg.ops;
        let big: usize = 1 << 20;
        let payload = self.cfg.payload;
        let yval = vec![b'y'; payload];
        // Untimed 2^20-put seed: under the G1 column each put would
        // F_FULLFSYNC (~70 min of Darwin setup for a read-only measured
        // loop). Seed async — symmetric with the Rocks peer, whose global
        // default seeds the same keyspace async — then restore the
        // battery column before the timed loop (pure point reads; every
        // timed write shape elsewhere keeps its per-op sync).
        let column_sync = e.sync();
        e.set_write_sync(false);
        let t0 = std::time::Instant::now();
        for i in 0..big {
            let _ = e.put(&ykey(i), &yval);
        }
        e.set_write_sync(column_sync);
        eprintln!(
            "[rocks-parity] ycsb_c_big seed {big} keys in {:.1}s (untimed)",
            t0.elapsed().as_secs_f64()
        );
        let mut rng = std::mem::take(&mut self.rng);
        let mut lats = Vec::with_capacity(cfg_ops);
        let mut errors = 0u64;
        let t0 = Instant::now();
        for _ in 0..cfg_ops {
            let t = Instant::now();
            let i = (xorshift(&mut rng) as usize) % big;
            if e.get_probe(&ykey(i)).is_err() {
                errors += 1;
            }
            lats.push(ms(t));
        }
        self.rng = rng;
        let block = summarize("ycsb_c_big", cfg_ops, t0.elapsed(), &mut lats);
        eprintln!("[rocks-parity] ycsb_c_big done (uniform 2^20 keyspace) errors={errors}");
        Some(block)
    }

    /// RFC-0037 P2.2: multi-client A/F/overwrite shapes over a fixed seeded
    /// keyspace (no inserts — the window growth would be racy). `clients`
    /// threads, independent per-client schedules from the same zipf CDF,
    /// barrier-aligned start. Each client runs `cfg.ops` ops; block `n` is
    /// the aggregate. Same op mix as the single-client rows so the two are
    /// directly comparable.
    pub fn run_clients<E: Engine + Sync>(&self, e: &E, clients: usize) -> Vec<String> {
        assert!(clients >= 2, "multi-client harness needs >= 2 clients");
        let cfg_ops = self.cfg.ops;
        let records = self.cfg.records;
        let payload = self.cfg.payload;
        let yval = std::sync::Arc::new(vec![b'y'; payload]);
        let ytab = std::sync::Arc::new(YKeys::new(records));
        let mut blocks = Vec::new();
        // (name, read_pct, rmw, overwrite) — mirrors run()/run_deps mixes.
        // RFC-0163 P1.4: ycsb_b (95% get / 5% put) is the read-dominated
        // rung of the concurrency ladder.
        let shapes: [(&str, u64, bool, bool); 4] = [
            ("ycsb_a", 50, false, false),
            ("ycsb_b", 95, false, false),
            ("ycsb_f", 50, true, false),
            ("deps_cache_overwrite", 0, false, true),
        ];
        for (name, read_pct, rmw, overwrite) in shapes {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(clients));
            let t0 = Instant::now();
            let mut lats = Vec::with_capacity(cfg_ops * clients);
            let mut errors = 0u64;
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..clients)
                    .map(|c| {
                        let barrier = barrier.clone();
                        let yval = yval.clone();
                        let ytab = ytab.clone();
                        s.spawn(move || {
                            let mut rng =
                                0x5EED_0001_u64.wrapping_mul((c as u64) + 0x9E37) ^ (c as u64);
                            let mut lats = Vec::with_capacity(cfg_ops);
                            let mut errors = 0u64;
                            barrier.wait();
                            for _ in 0..cfg_ops {
                                let t = Instant::now();
                                let u = self.pick(&mut rng, records);
                                let ok = if overwrite {
                                    e.put(format!("c/{u:06}").as_bytes(), &yval)
                                } else if xorshift(&mut rng) % 100 < read_pct {
                                    e.get_probe(ytab.key(u)).is_ok()
                                } else if rmw {
                                    e.rmw(ytab.key(u), &yval)
                                } else {
                                    e.put(ytab.key(u), &yval)
                                };
                                if !ok {
                                    errors += 1;
                                }
                                lats.push(ms(t));
                            }
                            (lats, errors)
                        })
                    })
                    .collect();
                for h in handles {
                    let (mut l, err) = h.join().expect("client thread");
                    errors += err;
                    lats.append(&mut l);
                }
            });
            let wall = t0.elapsed();
            let block = summarize_mc(
                &format!("{name}_mc{clients}"),
                cfg_ops * clients,
                wall,
                &mut lats,
                clients,
                errors,
            );
            eprintln!(
                "[rocks-parity] {name} mc{clients} done ops={} errors={errors}",
                cfg_ops * clients
            );
            blocks.push(block);
        }
        blocks
    }

    /// RFC-0040 P1.1: multi-client apply + raftlog (group-commit vs Rocks async).
    ///
    /// Per-client schedules, barrier start. Apply versions use a shared
    /// `AtomicU64` so (user, ts) stays unique. Raftlog keys are
    /// `raftlog/{client}/{idx}` so clients do not collide.
    pub fn run_deps_clients<E: Engine + Sync>(&self, e: &E, clients: usize) -> Vec<String> {
        assert!(clients >= 2, "multi-client harness needs >= 2 clients");
        let cfg_ops = self.cfg.ops;
        let records = self.cfg.records;
        let batch = self.cfg.batch;
        let yval = std::sync::Arc::new(vec![b'd'; self.cfg.payload]);
        let ts_src = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000_000));
        let mut blocks = Vec::with_capacity(2);

        // deps_apply_batch_mcN
        {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(clients));
            let t0 = Instant::now();
            let mut lats = Vec::with_capacity(cfg_ops * clients);
            let mut errors = 0u64;
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..clients)
                    .map(|c| {
                        let barrier = barrier.clone();
                        let yval = yval.clone();
                        let ts_src = ts_src.clone();
                        s.spawn(move || {
                            let mut rng =
                                0xA11A_0001_u64.wrapping_mul((c as u64) + 0x9E37) ^ (c as u64);
                            let mut lats = Vec::with_capacity(cfg_ops);
                            let mut errors = 0u64;
                            barrier.wait();
                            for _ in 0..cfg_ops {
                                let t = Instant::now();
                                let mut pre = Vec::with_capacity(batch * 2);
                                let mut com = Vec::with_capacity(batch * 2);
                                for _ in 0..batch {
                                    let u = self.pick(&mut rng, records);
                                    let ts =
                                        ts_src.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    pre.push(CfWrite::Put {
                                        cf: "lock",
                                        k: ukey(u),
                                        v: b"l".to_vec(),
                                    });
                                    pre.push(CfWrite::Put {
                                        cf: "default",
                                        k: mvcc(u, ts),
                                        v: yval.as_ref().clone(),
                                    });
                                    com.push(CfWrite::Put {
                                        cf: "write",
                                        k: mvcc(u, ts),
                                        v: b"c".to_vec(),
                                    });
                                    com.push(CfWrite::Delete {
                                        cf: "lock",
                                        k: ukey(u),
                                    });
                                }
                                let ok = e.batch(pre) && e.batch(com);
                                if !ok {
                                    errors += 1;
                                }
                                lats.push(ms(t));
                            }
                            (lats, errors)
                        })
                    })
                    .collect();
                for h in handles {
                    let (mut l, err) = h.join().expect("apply client");
                    errors += err;
                    lats.append(&mut l);
                }
            });
            let name = format!("deps_apply_batch_mc{clients}");
            blocks.push(summarize_mc(
                &name,
                cfg_ops * clients,
                t0.elapsed(),
                &mut lats,
                clients,
                errors,
            ));
            eprintln!(
                "[rocks-parity] deps_apply_batch mc{clients} done ops={} errors={errors}",
                cfg_ops * clients
            );
            if let Some((sub, queued, groups, gops)) = e.write_group_stats() {
                let avg = if groups == 0 {
                    0.0
                } else {
                    gops as f64 / groups as f64
                };
                eprintln!(
                    "[rocks-parity] write_group submits={sub} queued={queued} groups={groups} ops={gops} avg_group={avg:.2}"
                );
            }
        }

        // deps_raftlog_mcN
        {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(clients));
            let t0 = Instant::now();
            let mut lats = Vec::with_capacity(cfg_ops * clients);
            let mut errors = 0u64;
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..clients)
                    .map(|c| {
                        let barrier = barrier.clone();
                        let yval = yval.clone();
                        s.spawn(move || {
                            let mut lats = Vec::with_capacity(cfg_ops);
                            let mut errors = 0u64;
                            let mut idx = 0u64;
                            barrier.wait();
                            for op in 0..cfg_ops {
                                let t = Instant::now();
                                let mut wb = Vec::with_capacity(16);
                                for _ in 0..16 {
                                    idx += 1;
                                    wb.push(CfWrite::Put {
                                        cf: "raftlog",
                                        k: format!("raftlog/{c}/{idx:08}").into_bytes(),
                                        v: yval.as_ref().clone(),
                                    });
                                }
                                let ok = e.batch(wb);
                                if !ok {
                                    errors += 1;
                                }
                                if op % 8 == 0 && idx > 1 {
                                    let k = format!("raftlog/{c}/{:08}", idx - 1);
                                    if e.get_cf("raftlog", k.as_bytes()).is_err() {
                                        errors += 1;
                                    }
                                }
                                lats.push(ms(t));
                            }
                            (lats, errors)
                        })
                    })
                    .collect();
                for h in handles {
                    let (mut l, err) = h.join().expect("raftlog client");
                    errors += err;
                    lats.append(&mut l);
                }
            });
            let name = format!("deps_raftlog_mc{clients}");
            blocks.push(summarize_mc(
                &name,
                cfg_ops * clients,
                t0.elapsed(),
                &mut lats,
                clients,
                errors,
            ));
            eprintln!(
                "[rocks-parity] deps_raftlog mc{clients} done ops={} errors={errors}",
                cfg_ops * clients
            );
        }
        blocks
    }
}

pub fn ykey(i: usize) -> Vec<u8> {
    format!("ycsb/{i:06}").into_bytes()
}

/// Flat ycsb keyspace (RFC-0163 P1.2 relaunch): the same `ykey` bytes in
/// one buffer plus an end-offset table. At 25M records a
/// `Vec<Vec<u8>>` keyspace costs ~1.4 GiB of anon and OOM-killed the
/// compat leg (exit 137) when the mc phase built its table next to the
/// resident engine under the 3.9 GiB guest cgroup; flat, the same keys
/// cost ~0.4 GiB. Key bytes are identical — no dataset or rng change.
pub struct YKeys {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

impl YKeys {
    pub fn new(n: usize) -> Self {
        let mut bytes = Vec::with_capacity(n.saturating_mul(13));
        let mut ends = Vec::with_capacity(n);
        for i in 0..n {
            bytes.extend_from_slice(&ykey(i));
            ends.push(u32::try_from(bytes.len()).expect("keyspace < 4 GiB"));
        }
        Self { bytes, ends }
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub fn key(&self, i: usize) -> &[u8] {
        let hi = self.ends[i] as usize;
        let lo = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.bytes[lo..hi]
    }
}

/// Kvrocks Redis-string key (independent of the YCSB keyspace).
pub fn kkey(i: usize) -> Vec<u8> {
    format!("k/{i:06}").into_bytes()
}

/// MyRocks / LinkBench node (PK) key.
pub fn nkey(i: usize) -> Vec<u8> {
    format!("n/{i:06}").into_bytes()
}

/// LinkBench edge key: outgoing links from `src` sort as a prefix.
pub fn lkey(src: usize, dst: usize) -> Vec<u8> {
    format!("l/{src:06}/{dst:06}").into_bytes()
}

/// Exclusive-end prefix for `scan_count` over `src`'s outgoing links.
pub fn lprefix(src: usize) -> Vec<u8> {
    format!("l/{src:06}/").into_bytes()
}

/// SurrealDB / crud-bench document key (independent of YCSB).
pub fn skey(i: usize) -> Vec<u8> {
    format!("s/{i:06}").into_bytes()
}

/// Kvrocks BlobDB-sized value key (independent of the 1 KB SET canary).
pub fn bkey(i: usize) -> Vec<u8> {
    format!("b/{i:06}").into_bytes()
}

/// Nebula / Arango edge key (outgoing from `src` sort as a prefix).
pub fn ekey(src: usize, dst: usize) -> Vec<u8> {
    format!("e/{src:06}/{dst:06}").into_bytes()
}

/// Exclusive-end prefix for `scan_count` over `src`'s outgoing edges.
pub fn eprefix(src: usize) -> Vec<u8> {
    format!("e/{src:06}/").into_bytes()
}

/// Flink window-state key.
pub fn wkey(win: usize, seq: usize) -> Vec<u8> {
    format!("w/{win:06}/{seq:06}").into_bytes()
}

pub fn wprefix(win: usize) -> Vec<u8> {
    format!("w/{win:06}/").into_bytes()
}

/// Ceph BlueStore omap key.
pub fn okey(i: usize) -> Vec<u8> {
    format!("o/{i:06}").into_bytes()
}

/// Solana shred / slot key (zero-padded so trailing scans are ordered).
pub fn shred(i: usize) -> Vec<u8> {
    format!("sh/{i:08}").into_bytes()
}

/// Arango / Venice document key.
pub fn dkey(i: usize) -> Vec<u8> {
    format!("d/{i:06}").into_bytes()
}

/// Pinterest Rockstore wide-column: row + col + ts (latest sorts last).
pub fn colkey(row: usize, col: u16, ts: u64) -> Vec<u8> {
    format!("c/{row:06}/{col:02}/{ts:08}").into_bytes()
}

pub fn colprefix(row: usize, col: u16) -> Vec<u8> {
    format!("c/{row:06}/{col:02}/").into_bytes()
}

/// Oxigraph SPO triple key (not a SPARQL algebra).
pub fn tkey(s: usize, p: u16, o: usize) -> Vec<u8> {
    format!("t/{s:06}/{p:02}/{o:06}").into_bytes()
}

/// MVCC user key (deps suite).
pub fn ukey(i: usize) -> Vec<u8> {
    format!("u/{i:06}").into_bytes()
}

/// MVCC row key: user key + big-endian ascending version suffix (latest
/// version sorts last under the prefix — reverse seek finds it first).
pub fn mvcc(i: usize, ts: u64) -> Vec<u8> {
    let mut k = ukey(i);
    k.extend_from_slice(&ts.to_be_bytes());
    k
}

fn xorshift(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize(name: &str, n: usize, wall: Duration, lats_ms: &mut [f64]) -> String {
    lats_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let wall_s = wall.as_secs_f64().max(1e-12);
    let qps = n as f64 / wall_s;
    format!(
        r#"{{
    "name": "{name}",
    "n": {n},
    "qps": {qps:.3},
    "p50_ms": {p50:.4},
    "p95_ms": {p95:.4},
    "p99_ms": {p99:.4},
    "p999_ms": {p999:.4},
    "max_ms": {max:.4},
    "wall_s": {wall_s:.4}
  }}"#,
        p50 = pct(lats_ms, 50.0),
        p95 = pct(lats_ms, 95.0),
        p99 = pct(lats_ms, 99.0),
        p999 = pct(lats_ms, 99.9),
        max = lats_ms.last().copied().unwrap_or(0.0),
    )
}

/// `summarize` plus `"clients"` and `"errors"` fields (RFC-0037 P2.2
/// multi-client blocks).
fn summarize_mc(
    name: &str,
    n: usize,
    wall: Duration,
    lats_ms: &mut [f64],
    clients: usize,
    errors: u64,
) -> String {
    let block = summarize(name, n, wall, lats_ms);
    block.replace(
        "\"wall_s\"",
        &format!("\"clients\": {clients},\n    \"errors\": {errors},\n    \"wall_s\""),
    )
}

/// Suite selector: `ROCKS_PARITY_SUITE` csv (default "ycsb,deps"; "all" =
/// every suite). RFC-0043: `qs`/`kvrocks`/`myrocks`/`surreal`/`rocksapi`
/// are opt-in so the 16-shape official set stays comparable.
pub fn suites_enabled(want: &str) -> bool {
    let s = std::env::var("ROCKS_PARITY_SUITE").unwrap_or_else(|_| "ycsb,deps".into());
    let s = s.to_lowercase();
    if s.split(',').any(|x| x.trim() == "all") {
        return true;
    }
    s.split(',').any(|x| x.trim() == want)
}

/// Per-suite Rocks `WriteOptions.sync` = default of the DB **on top**.
///
/// - ycsb / deps / qs / kvrocks / nebula / streaming / solana / arango /
///   venice / oxigraph: Rocks default = async (`false`)
/// - myrocks: `rocksdb_flush_log_at_trx_commit=1` = sync on commit
/// - surreal: `SURREAL_DATASTORE_SYNC_DATA=every` = fsync after txn
/// - ceph: BlueStore omap is durable metadata (sync on commit)
///
/// `ROCKS_PARITY_SYNC=1` forces every suite sync (same-class column).
/// `=0` does **not** flatten MyRocks/Surreal/Ceph to async (host default wins).
pub fn write_sync_for_suite(suite: &str) -> bool {
    if std::env::var("ROCKS_PARITY_SYNC").as_deref() == Ok("1") {
        return true;
    }
    matches!(suite, "myrocks" | "surreal" | "ceph")
}

/// JSON `peer_policy`: host-default if a host-sync suite ran.
pub fn peer_policy(suites: &str) -> &'static str {
    if std::env::var("ROCKS_PARITY_SYNC").as_deref() == Ok("1") {
        return "forced-sync";
    }
    let host = suites.split(',').any(|s| {
        let s = s.trim();
        s == "myrocks" || s == "surreal" || s == "ceph" || s == "all"
    });
    if host {
        "host-default"
    } else {
        "rocks-default"
    }
}

/// Top-level JSON `sync` for the Rocks peer: true if any enabled suite
/// writes with host/forced sync.
pub fn peer_reports_sync() -> bool {
    [
        "ycsb",
        "deps",
        "qs",
        "kvrocks",
        "myrocks",
        "surreal",
        "nebula",
        "streaming",
        "ceph",
        "solana",
        "arango",
        "venice",
        "oxigraph",
        "rocksapi",
    ]
    .iter()
    .any(|s| suites_enabled(s) && write_sync_for_suite(s))
}

/// Physical incoherence in the **peer** metrics (not Pedra). Dirty-box
/// example: Rocks `surreal_tx_rmw` qps > its own `surreal_tx_put` — rmw
/// does strictly more work (get+put+commit). 5% slack for noise.
///
/// Report-only by default; `ROCKS_PARITY_FAIL_PEER_ANOMALY=1` makes
/// compare exit 2.
pub fn peer_anomalies(peer: &std::collections::BTreeMap<String, f64>) -> Vec<String> {
    let mut out = Vec::new();
    if let (Some(&put), Some(&rmw)) = (peer.get("surreal_tx_put"), peer.get("surreal_tx_rmw")) {
        if put > 0.0 && rmw > put * 1.05 {
            out.push(format!(
                "surreal_tx_rmw peer {rmw:.0} qps > put {put:.0} (rmw ⊃ put; run dirty/invalid)"
            ));
        }
    }
    out
}

/// Assemble the bench report file content for one engine run.
pub fn report_json<E: Engine>(e: &E, cfg: &Cfg, benches: &[String], suites: &str) -> String {
    let mut notes = vec![
        format!(
            "ycsb records={} ops={} payload={} dist={}",
            cfg.records,
            cfg.ops,
            cfg.payload,
            cfg.dist_label()
        ),
        format!("engine={} durability={}", e.label(), e.durability()),
        format!("suites: {suites}"),
    ];
    notes.push("seed: one put per record (not timed)".to_string());
    if let Some(note) = seed_async_note(seed_async_enabled()) {
        notes.push(note.to_string());
    }
    let clients = clients_from_env();
    if !clients.is_empty() {
        notes.push(format!("clients ladder: {:?}", clients));
    }
    notes.push(format!(
        "cfs: default + {} (TiKV store shape; raftlog = raftdb)",
        DEPS_CFS.join(", ")
    ));
    notes.push(format!("deps apply batch txns/commit: {}", cfg.batch));
    let changelog_interval = env_usize("PEDRA_CHANGELOG_INTERVAL", 0);
    notes.push(format!("changelog_interval={changelog_interval}"));
    let policy = peer_policy(suites);
    notes.push(format!("peer_policy={policy}"));
    notes.push(format!(
        "host-default write sync: kvrocks={} myrocks={} surreal={}",
        write_sync_for_suite("kvrocks"),
        write_sync_for_suite("myrocks"),
        write_sync_for_suite("surreal")
    ));
    let notes_json = notes
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"{{
  "bench": "rocks-parity-v1",
  "engine": "{engine}",
  "sync": {sync},
  "peer_policy": "{policy}",
  "durability": "{durability}",
  "changelog_interval": {changelog_interval},
  "status": "ok",
  "notes": [{notes_json}],
  "benches": [
{benches}
  ]
}}
"#,
        engine = e.label(),
        sync = e.sync(),
        durability = e.durability(),
        benches = benches.join(",\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// Minimal Engine that records `set_write_sync` calls (RFC-0163 P1.2
    /// seed-barrier unit — no real DB behind it).
    #[derive(Default)]
    struct BarrierMock {
        sync: Cell<bool>,
        toggles: RefCell<Vec<bool>>,
        seeded: Cell<bool>,
    }

    impl BarrierMock {
        fn new(sync: bool) -> Self {
            Self {
                sync: Cell::new(sync),
                ..Self::default()
            }
        }
    }

    impl Engine for BarrierMock {
        fn label(&self) -> &'static str {
            "mock"
        }
        fn durability(&self) -> &'static str {
            "mock"
        }
        fn sync(&self) -> bool {
            self.sync.get()
        }
        fn set_write_sync(&self, sync: bool) {
            self.sync.set(sync);
            self.toggles.borrow_mut().push(sync);
        }
        fn put(&self, _: &[u8], _: &[u8]) -> bool {
            true
        }
        fn get(&self, _: &[u8]) -> Result<Option<Vec<u8>>, ()> {
            Ok(None)
        }
        fn scan_count(&self, _: &[u8], _: &[u8], _: usize) -> Result<usize, ()> {
            Ok(0)
        }
        fn put_cf(&self, _: &str, _: &[u8], _: &[u8]) -> bool {
            true
        }
        fn get_cf(&self, _: &str, _: &[u8]) -> Result<Option<Vec<u8>>, ()> {
            Ok(None)
        }
        fn batch(&self, _: Vec<CfWrite>) -> bool {
            true
        }
        fn latest_cf(&self, _: &str, _: &[u8]) -> Result<Option<Vec<u8>>, ()> {
            Ok(None)
        }
        fn scan_count_cf(&self, _: &str, _: &[u8], _: &[u8], _: usize) -> Result<usize, ()> {
            Ok(0)
        }
    }

    #[test]
    fn seed_barrier_toggles_only_for_sync_engine_when_enabled() {
        // Enabled + sync column: WAL barrier off before the untimed seed,
        // column sync restored after — exactly the run_c_big pattern.
        let e = BarrierMock::new(true);
        seed_under_async_barrier(&e, true, || e.seeded.set(true));
        assert!(e.seeded.get(), "seed ran");
        assert_eq!(*e.toggles.borrow(), vec![false, true], "off then restore");
        assert!(e.sync(), "column sync restored");

        // Disabled (default): the column is never touched.
        let e = BarrierMock::new(true);
        seed_under_async_barrier(&e, false, || e.seeded.set(true));
        assert!(e.seeded.get());
        assert!(e.toggles.borrow().is_empty(), "no toggles when disabled");

        // Async engine (the Rocks default peer): pass-through, no toggles.
        let e = BarrierMock::new(false);
        seed_under_async_barrier(&e, true, || e.seeded.set(true));
        assert!(e.seeded.get());
        assert!(e.toggles.borrow().is_empty(), "async peer untouched");
    }

    #[test]
    fn seed_async_note_marks_the_report_only_when_enabled() {
        assert!(seed_async_note(false).is_none());
        let note = seed_async_note(true).expect("note when enabled");
        assert!(
            note.starts_with("seed_async=1") && note.contains("column sync"),
            "{note}"
        );
    }

    /// RFC-0163 P1.4: the ladder csv wins over the single value, canonical
    /// ascending, counts < 2 dropped (free single-client rung), duplicates
    /// collapse; unset ladder falls back to `ROCKS_PARITY_CLIENTS`.
    #[test]
    fn clients_ladder_from_semantics() {
        assert!(clients_ladder_from(None, 1).is_empty());
        assert_eq!(clients_ladder_from(None, 4), vec![4]);
        assert_eq!(clients_ladder_from(Some("4,16,64"), 99), vec![4, 16, 64]);
        assert_eq!(clients_ladder_from(Some("64, 4,16,4"), 1), vec![4, 16, 64]);
        assert_eq!(clients_ladder_from(Some("1,4"), 1), vec![4]);
        assert_eq!(clients_ladder_from(Some(""), 8), vec![8]);
        assert_eq!(clients_ladder_from(Some("   "), 8), vec![8]);
        let _ = std::panic::catch_unwind(|| clients_ladder_from(Some("4,6O"), 1));
    }

    /// RFC-0163 P1.2 relaunch: the flat keyspace returns byte-identical
    /// `ykey`s across the 6/7/8-digit index boundaries at the real campaign
    /// size (25M records + ops + 25 scan window).
    #[test]
    fn ykeys_flat_matches_ykey_bytes() {
        let n = 25_000_000 + 100_000 + 25;
        let yk = YKeys::new(n);
        assert_eq!(yk.len(), n);
        assert!(!yk.is_empty());
        for i in [0usize, 1, 999_999, 1_000_000, 9_999_999, 10_000_000, n - 1] {
            assert_eq!(yk.key(i), ykey(i), "key({i})");
        }
    }

    #[test]
    fn shape_wanted_in_unset_keeps_every_shape() {
        assert!(shape_wanted_in("ycsb_a", None));
        assert!(shape_wanted_in("ycsb_c_big", None));
        assert!(shape_wanted_in("deps_raftlog", None));
    }

    /// RFC-0163 P1.1: every block reports the full tail ladder
    /// p50 ≤ p95 ≤ p99 ≤ p999 ≤ max, and `p999_ms` is a first-class
    /// field (the tail is a cell criterion, not an afterthought).
    #[test]
    fn summarize_schema_has_p999_and_monotonic_tail() {
        // 1000 samples 1..=1000 ms: sorted, pct(99.9) must pick the
        // top-decile-of-a-percent value, not clamp to max.
        let mut lats: Vec<f64> = (1..=1000).map(f64::from).collect();
        // shuffle so summarize's internal sort is exercised
        lats.reverse();
        let block = summarize("unit_tail", 1000, Duration::from_secs(10), &mut lats);
        for key in ["\"p50_ms\"", "\"p95_ms\"", "\"p99_ms\"", "\"p999_ms\"", "\"max_ms\""] {
            assert!(block.contains(key), "missing {key} in:\n{block}");
        }
        let val = |k: &str| {
            block
                .lines()
                .find(|l| l.contains(k))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|s| s.trim().trim_end_matches(',').parse::<f64>().ok())
                .unwrap_or_else(|| panic!("parse {k} in:\n{block}"))
        };
        let (p50, p99, p999, max) = (val("\"p50_ms\""), val("\"p99_ms\""), val("\"p999_ms\""), val("\"max_ms\""));
        // pct index = round(p/100 * (len-1)); len=1000 → idx 500/989/998
        assert_eq!(p50, 501.0, "p50 of 1..=1000");
        assert_eq!(p99, 990.0, "p99 of 1..=1000");
        assert_eq!(p999, 999.0, "p999 of 1..=1000");
        assert_eq!(max, 1000.0);
        let mc = summarize_mc("unit_mc", 10, Duration::from_secs(1), &mut vec![1.0; 10], 4, 0);
        assert!(mc.contains("\"clients\": 4") && mc.contains("\"p999_ms\""), "mc block keeps tail:\n{mc}");
    }

    #[test]
    fn shape_wanted_in_csv_is_exact_names() {
        let only = Some("ycsb_a,deps_raftlog");
        assert!(shape_wanted_in("ycsb_a", only));
        assert!(shape_wanted_in("deps_raftlog", only));
        assert!(!shape_wanted_in("ycsb_b", only));
        assert!(!shape_wanted_in("ycsb_c_big", only));
    }

    #[test]
    fn full_sync_follows_write_sync_flag() {
        assert!(rocks_full_sync_after_write(true, true));
        assert!(!rocks_full_sync_after_write(true, false));
        assert!(!rocks_full_sync_after_write(false, true));
        assert!(!rocks_full_sync_after_write(false, false));
    }

    #[test]
    fn live_wal_log_picks_highest_numbered_segment() {
        let names = [
            "LOG",
            "CURRENT",
            "000003.log",
            "000012.log",
            "MANIFEST-000011",
        ];
        assert_eq!(live_wal_log_name(names.iter().copied()), Some("000012.log"));
        assert_eq!(live_wal_log_name(["OPTIONS-000007"].iter().copied()), None);
    }

    #[test]
    fn runner_schedule_is_deterministic() {
        // Same cfg → same rng stream: pick() sequence must repeat exactly.
        let cfg = Cfg {
            records: 256,
            ops: 64,
            payload: 8,
            zipfian: true,
            batch: 32,
        };
        let a = YcsbRunner::new(cfg.clone());
        let b = YcsbRunner::new(cfg);
        let mut ra = a.rng;
        let mut rb = b.rng;
        for _ in 0..100 {
            let latest = 200;
            assert_eq!(a.pick(&mut ra, latest), b.pick(&mut rb, latest));
        }
    }

    #[test]
    fn host_default_write_sync() {
        // No ROCKS_PARITY_SYNC=1 in this process: MyRocks/Surreal sync, Kvrocks not.
        assert!(!write_sync_for_suite("kvrocks"));
        assert!(!write_sync_for_suite("ycsb"));
        assert!(!write_sync_for_suite("nebula"));
        assert!(write_sync_for_suite("myrocks"));
        assert!(write_sync_for_suite("surreal"));
        assert!(write_sync_for_suite("ceph"));
        assert_eq!(peer_policy("kvrocks"), "rocks-default");
        assert_eq!(peer_policy("myrocks,surreal"), "host-default");
        assert_eq!(peer_policy("ceph"), "host-default");
    }

    #[test]
    fn cfg_defaults_sane() {
        let c = Cfg {
            records: 64,
            ops: 32,
            payload: 1,
            zipfian: false,
            batch: 4,
        };
        assert_eq!(c.dist_label(), "uniform");
    }

    #[test]
    fn mvcc_key_orders_latest_last() {
        let lo = mvcc(3, 1);
        let hi = mvcc(3, 2);
        assert!(lo < hi, "same-user later version must sort after earlier");
        assert!(
            ukey(3) < lo,
            "versioned row must sort after its user prefix"
        );
        // Prefix containment: both versions share the user prefix.
        assert!(lo.starts_with(&ukey(3)) && hi.starts_with(&ukey(3)));
    }

    // Deps suite end-to-end on the compat engine (seed + five shapes run,
    // MVCC latest read finds the newest version, value reachable in default).
    /// RFC-0037 P2.2: multi-client blocks run on both the compat engine and
    /// the ConcurrentDb (group commit) engine with zero errors, and every
    /// written key stays readable afterwards.
    #[test]
    fn multi_client_shapes_on_compat_and_concurrent() {
        let cfg = Cfg {
            records: 64,
            ops: 32,
            payload: 16,
            zipfian: false,
            batch: 8,
        };
        for dir in [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()] {
            let compat = crate::engines::CompatEngine::open(dir.path());
            let mut r = YcsbRunner::new(cfg.clone());
            r.seed(&compat);
            let blocks = r.run_clients(&compat, 3);
            assert_eq!(
                blocks
                    .iter()
                    .map(|b| b
                        .split("\"name\": \"")
                        .nth(1)
                        .and_then(|s| s.split('"').next()))
                    .collect::<Vec<_>>(),
                vec![
                    Some("ycsb_a_mc3"),
                    Some("ycsb_b_mc3"),
                    Some("ycsb_f_mc3"),
                    Some("deps_cache_overwrite_mc3")
                ]
            );
            for b in &blocks {
                assert!(b.contains("\"clients\": 3"), "{b}");
                assert!(b.contains("\"errors\": 0"), "{b}");
            }
            assert!(compat.get(&ykey(0)).unwrap().is_some());

            let cdir = tempfile::tempdir().unwrap();
            let conc = crate::engines::ConcurrentEngine::open(cdir.path());
            let mut r2 = YcsbRunner::new(cfg.clone());
            r2.seed(&conc);
            let blocks2 = r2.run_clients(&conc, 3);
            assert_eq!(blocks2.len(), 4);
            assert!(conc.get(&ykey(0)).is_ok());
            assert!(conc.get(b"c/000000").is_ok());
        }
    }

    /// RFC-0040 P1.1: apply + raftlog MC on compat (group commit / ConcurrentDb).
    #[test]
    fn multi_client_deps_apply_raftlog_on_compat() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let cfg = Cfg {
            records: 64,
            ops: 24,
            payload: 16,
            zipfian: false,
            batch: 4,
        };
        let mut r = YcsbRunner::new(cfg);
        let _ = r.run_deps(&e);
        let blocks = r.run_deps_clients(&e, 3);
        let names: Vec<_> = blocks
            .iter()
            .map(|b| {
                b.split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
            })
            .collect();
        assert_eq!(
            names,
            vec![Some("deps_apply_batch_mc3"), Some("deps_raftlog_mc3")]
        );
        for b in &blocks {
            assert!(b.contains("\"clients\": 3"), "{b}");
            assert!(b.contains("\"errors\": 0"), "{b}");
        }
        assert!(e
            .get_cf("raftlog", b"raftlog/0/00000001")
            .unwrap()
            .is_some());
    }

    #[test]
    fn deps_suite_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let cfg = Cfg {
            records: 64,
            ops: 48,
            payload: 16,
            zipfian: false,
            batch: 8,
        };
        let mut r = YcsbRunner::new(cfg);
        let blocks = r.run_deps(&e);
        assert_eq!(
            blocks
                .iter()
                .map(|b| b
                    .split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next()))
                .collect::<Vec<_>>(),
            vec![
                Some("deps_apply_batch"),
                Some("deps_mvcc_latest"),
                Some("deps_mvcc_latest_split"),
                Some("deps_scan"),
                Some("deps_scan_probe"),
                Some("deps_raftlog"),
                Some("deps_cache_overwrite"),
                Some("deps_lock_prewrite"),
            ]
        );
        // Latest version of user 0: seed round 2 wrote ts = records + 1; the
        // apply shape then advanced it further. Latest must be a versioned row
        // of user 0 with ts >= records + 1 (apply-batch writes visible).
        let latest = e.latest_cf("write", &ukey(0)).unwrap().unwrap();
        assert!(latest.starts_with(&ukey(0)), "{latest:?}");
        let ts = u64::from_be_bytes(latest[ukey(0).len()..].try_into().unwrap());
        assert!(ts >= 65, "latest ts {ts} must be at least seed round 2");
        assert!(e.get_cf("default", &latest).unwrap().is_some());
        // Locks were released by the commit batches (and lock_prewrite cleanup).
        assert!(e.get_cf("lock", &ukey(0)).unwrap().is_none());
        // Raftlog appends are readable back.
        assert!(e.get_cf("raftlog", b"raftlog/00000001").unwrap().is_some());
    }

    #[test]
    fn qs_suite_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let cfg = Cfg {
            records: 64,
            ops: 100,
            payload: 16,
            zipfian: false,
            batch: 8,
        };
        let mut r = YcsbRunner::new(cfg);
        r.seed(&e);
        let blocks = r.run_qs(&e);
        let names: Vec<_> = blocks
            .iter()
            .map(|b| {
                b.split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
            })
            .collect();
        assert_eq!(
            names,
            vec![
                Some("qs_hot_get"),
                Some("qs_neg_lookup"),
                Some("qs_batch_write"),
            ]
        );
        for b in &blocks {
            assert!(b.contains("\"p50_ms\""), "{b}");
        }
    }

    #[test]
    fn compare_shapes_keep_official_16_prefix() {
        assert!(COMPARE_SHAPES.len() >= OFFICIAL_16);
        assert_eq!(
            &COMPARE_SHAPES[..OFFICIAL_16],
            &[
                "ycsb_a",
                "ycsb_b",
                "ycsb_c",
                "ycsb_d",
                "ycsb_e",
                "ycsb_f",
                "deps_apply_batch",
                "deps_mvcc_latest",
                "deps_scan",
                "deps_raftlog",
                "deps_cache_overwrite",
                "ycsb_a_mc4",
                "ycsb_f_mc4",
                "deps_cache_overwrite_mc4",
                "deps_apply_batch_mc4",
                "deps_raftlog_mc4",
            ]
        );
        for name in [
            "deps_lock_prewrite",
            "qs_hot_get",
            "kvrocks_pipelined_set",
            "kvrocks_scan",
            "myrocks_write_tx",
            "linkbench_mix",
            "surreal_tx_rmw",
            "surreal_tx_batch",
            "surreal_tx_rmw_mc8",
            "kvrocks_blob_set",
            "nebula_get_neighbors",
            "flink_window_state",
            "bluestore_omap_write",
            "solana_shred_append",
            "arango_traversal",
            "venice_fanout_get",
            "oxigraph_spo_lookup",
            "mixgraph_like",
            "wbwi_read_your_writes",
            "compaction_filter_drop",
            "ingest_sst",
        ] {
            assert!(
                COMPARE_SHAPES.contains(&name),
                "catalog must contain {name}"
            );
        }
    }

    #[test]
    fn kvrocks_suite_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let cfg = Cfg {
            records: 64,
            ops: 48,
            payload: 16,
            zipfian: false,
            batch: 8,
        };
        let mut r = YcsbRunner::new(cfg);
        let blocks = r.run_kvrocks(&e);
        let names: Vec<_> = blocks
            .iter()
            .map(|b| {
                b.split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
            })
            .collect();
        assert_eq!(
            names,
            vec![
                Some("kvrocks_get"),
                Some("kvrocks_set"),
                Some("kvrocks_pipelined_set"),
                Some("kvrocks_scan"),
                Some("kvrocks_blob_set"),
                Some("kvrocks_set_mc50"),
            ]
        );
        assert!(e.get(&kkey(0)).unwrap().is_some());
    }

    #[test]
    fn myrocks_suite_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let cfg = Cfg {
            records: 64,
            ops: 80,
            payload: 16,
            zipfian: false,
            batch: 8,
        };
        let mut r = YcsbRunner::new(cfg);
        let blocks = r.run_myrocks(&e);
        let names: Vec<_> = blocks
            .iter()
            .map(|b| {
                b.split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
            })
            .collect();
        assert_eq!(
            names,
            vec![
                Some("myrocks_point_select"),
                Some("myrocks_read_only"),
                Some("myrocks_write_tx"),
                Some("linkbench_mix"),
            ]
        );
        assert!(e.get(&nkey(0)).unwrap().is_some());
        // Seeded outgoing edge 0 → 1 survives unless the mix deleted it;
        // prefix scan of node 0 must still be well-defined either way.
        assert!(e.scan_count(&lprefix(0), &lprefix(1), 8).unwrap() <= 8);
        // Link keys sort as a prefix of lprefix(src).
        assert!(lkey(3, 7).starts_with(&lprefix(3)));
        assert!(lprefix(3) < lprefix(4));
    }

    #[test]
    fn surreal_suite_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let cfg = Cfg {
            records: 64,
            ops: 48,
            payload: 16,
            zipfian: false,
            batch: 8,
        };
        let mut r = YcsbRunner::new(cfg);
        let blocks = r.run_surreal(&e);
        let names: Vec<_> = blocks
            .iter()
            .map(|b| {
                b.split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
            })
            .collect();
        assert_eq!(
            names,
            vec![
                Some("surreal_tx_get"),
                Some("surreal_tx_put"),
                Some("surreal_tx_rmw"),
                Some("surreal_tx_scan"),
                Some("surreal_tx_batch"),
                Some("surreal_tx_rmw_mc8"),
            ]
        );
        assert!(e.get(&skey(0)).unwrap().is_some());
    }

    #[test]
    fn peer_rmw_faster_than_put_is_anomaly() {
        let mut peer = std::collections::BTreeMap::new();
        peer.insert("surreal_tx_put".into(), 3000.0);
        peer.insert("surreal_tx_rmw".into(), 5000.0);
        assert!(!peer_anomalies(&peer).is_empty());
        peer.insert("surreal_tx_rmw".into(), 2900.0);
        assert!(peer_anomalies(&peer).is_empty());
    }

    fn tiny_cfg() -> Cfg {
        Cfg {
            records: 64,
            ops: 24,
            payload: 16,
            zipfian: false,
            batch: 4,
        }
    }

    fn block_names(blocks: &[String]) -> Vec<Option<&str>> {
        blocks
            .iter()
            .map(|b| {
                b.split("\"name\": \"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
            })
            .collect()
    }

    #[test]
    fn expanding_suites_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let mut r = YcsbRunner::new(tiny_cfg());
        assert_eq!(
            block_names(&r.run_nebula(&e)),
            vec![Some("nebula_get_neighbors"), Some("nebula_insert_edge"),]
        );
        assert_eq!(
            block_names(&r.run_streaming(&e)),
            vec![Some("flink_window_state"), Some("kafka_changelog_flush"),]
        );
        assert_eq!(
            block_names(&r.run_ceph(&e)),
            vec![Some("bluestore_omap_write"), Some("bluestore_omap_read"),]
        );
        assert_eq!(
            block_names(&r.run_solana(&e)),
            vec![Some("solana_shred_append"), Some("solana_trailing_read"),]
        );
        assert_eq!(
            block_names(&r.run_arango(&e)),
            vec![Some("arango_doc_crud"), Some("arango_traversal")]
        );
        assert_eq!(
            block_names(&r.run_venice(&e)),
            vec![Some("venice_fanout_get"), Some("rockstore_widecol_rw"),]
        );
        assert_eq!(
            block_names(&r.run_oxigraph(&e)),
            vec![Some("oxigraph_spo_lookup"), Some("oxigraph_triple_put"),]
        );
        assert!(ekey(3, 7).starts_with(&eprefix(3)));
        assert!(tkey(1, 0, 1).starts_with(b"t/"));
    }

    #[test]
    fn rocksapi_suite_on_compat_engine() {
        let dir = tempfile::tempdir().unwrap();
        let e = crate::engines::CompatEngine::open(dir.path());
        let mut r = YcsbRunner::new(tiny_cfg());
        assert_eq!(
            block_names(&r.run_rocksapi(&e)),
            vec![
                Some("mixgraph_like"),
                Some("wbwi_read_your_writes"),
                Some("compaction_filter_drop"),
                Some("ingest_sst"),
            ]
        );
        let k = b"wbwi-probe";
        let v = b"overlay";
        assert_eq!(
            e.wbwi_overlay_get(&[(k.as_slice(), v.as_slice())], k)
                .unwrap()
                .as_deref(),
            Some(&b"overlay"[..])
        );
        assert!(e.ingest_kvs(&[(b"ing-k".as_slice(), b"ing-v".as_slice())]));
        assert_eq!(e.get(b"ing-k").unwrap().as_deref(), Some(&b"ing-v"[..]));
        assert!(e.put(b"keep/z", b"1"));
        assert!(e.put(b"drop/z", b"2"));
        assert!(e.flush());
        assert!(e.compact_drop_prefix(b"drop/"));
        assert!(e.get(b"drop/z").unwrap().is_none());
        assert_eq!(e.get(b"keep/z").unwrap().as_deref(), Some(&b"1"[..]));
    }
}
