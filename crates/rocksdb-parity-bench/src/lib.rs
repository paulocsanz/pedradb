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

#![forbid(unsafe_code)]

pub mod engines;

use std::time::{Duration, Instant};

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
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

/// Engine adapter. Both sides implement exactly these ops; the runner measures
/// only through this trait so the schedule cannot drift between engines.
pub trait Engine {
    fn label(&self) -> &'static str;
    /// Durability label surfaced in bench + compare JSON (honesty discipline).
    fn durability(&self) -> &'static str;
    fn sync(&self) -> bool;
    fn put(&self, k: &[u8], v: &[u8]) -> bool;
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()>;
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

        // Seed: 2 versions per record via batched commits (not timed) —
        // prewrite rows (lock + default) then commit rows (write, lock del).
        let t0 = Instant::now();
        let mut vers: Vec<u64> = vec![0; records];
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

        let mut rng = std::mem::take(&mut self.rng);
        let mut blocks = Vec::with_capacity(5);

        // 1. deps_apply_batch — raftstore apply: per logical op one ready =
        //    prewrite batch + commit batch (batch txns each).
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut txns, mut errors) = (0u64, 0u64);
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
            let ok = e.batch(std::mem::take(&mut pre)) && e.batch(std::mem::take(&mut com));
            if ok {
                txns += batch as u64;
            } else {
                errors += 1;
            }
            lats.push(ms(t));
        }
        blocks.push(summarize(
            "deps_apply_batch",
            cfg_ops,
            t0.elapsed(),
            &mut lats,
        ));
        eprintln!("[rocks-parity] deps_apply_batch done txns={txns} errors={errors}");

        // 2. deps_mvcc_latest — point read of the latest version: reverse-seek
        //    write CF for the user prefix, then fetch the value in default.
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

        // 3. deps_scan — short range scan over user keys in the write CF
        //    (coprocessor / GC range shape).
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

        // 4. deps_raftlog — raftdb append shape: batched sequential appends to
        //    the raftlog CF; every 8th op also reads the previous entry.
        let mut lats = Vec::with_capacity(cfg_ops);
        let (mut appends, mut reads, mut errors) = (0u64, 0u64, 0u64);
        let mut idx = 0u64;
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
            let ok = e.batch(std::mem::take(&mut wb));
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
            lats.push(ms(t));
        }
        blocks.push(summarize("deps_raftlog", cfg_ops, t0.elapsed(), &mut lats));
        eprintln!(
            "[rocks-parity] deps_raftlog done appends={appends} reads={reads} errors={errors}"
        );

        // 5. deps_cache_overwrite — unbatched zipf overwrite of a fixed
        //    keyspace (cache-style dependent; compat worst case).
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

        self.rng = rng;
        blocks
    }

    pub fn seed<E: Engine>(&mut self, e: &E) {
        let val = vec![b'y'; self.cfg.payload];
        for i in 0..self.cfg.records {
            assert!(e.put(&ykey(i), &val), "seed put {i}");
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
                if e.get(&ykey(i)).is_err() {
                    errors += 1;
                }
            } else if roll < read_pct + insert_pct {
                // insert (new key) → read-latest window grows
                let k = format!("ycsb/{latest:06}").into_bytes();
                if e.put(&k, &yval) {
                    latest += 1;
                    inserts += 1;
                } else {
                    errors += 1;
                }
            } else if scans {
                // short range scan: [key(i), key(i)+25) window, capped at 25
                let i = self.pick(&mut rng, latest);
                let start = ykey(i);
                let mut end = ykey(i + 25);
                end.pop();
                end.push(b'~');
                match e.scan_count(&start, &end, 25) {
                    Ok(_) => scan_ops += 1,
                    Err(_) => errors += 1,
                }
            } else if rmw {
                let i = self.pick(&mut rng, latest);
                if e.rmw(&ykey(i), &yval) {
                    updates += 1;
                } else {
                    errors += 1;
                }
            } else {
                // update
                let i = self.pick(&mut rng, latest);
                if e.put(&ykey(i), &yval) {
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
}

pub fn ykey(i: usize) -> Vec<u8> {
    format!("ycsb/{i:06}").into_bytes()
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
    "max_ms": {max:.4},
    "wall_s": {wall_s:.4}
  }}"#,
        p50 = pct(lats_ms, 50.0),
        p95 = pct(lats_ms, 95.0),
        p99 = pct(lats_ms, 99.0),
        max = lats_ms.last().copied().unwrap_or(0.0),
    )
}

/// Suite selector: `ROCKS_PARITY_SUITE` csv (default "ycsb,deps"; "all" = both).
pub fn suites_enabled(want: &str) -> bool {
    let s = std::env::var("ROCKS_PARITY_SUITE").unwrap_or_else(|_| "ycsb,deps".into());
    let s = s.to_lowercase();
    if s.split(',').any(|x| x.trim() == "all") {
        return true;
    }
    s.split(',').any(|x| x.trim() == want)
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
    notes.push(format!(
        "cfs: default + {} (TiKV store shape; raftlog = raftdb)",
        DEPS_CFS.join(", ")
    ));
    notes.push(format!("deps apply batch txns/commit: {}", cfg.batch));
    let changelog_interval = env_usize("PEDRA_CHANGELOG_INTERVAL", 0);
    notes.push(format!("changelog_interval={changelog_interval}"));
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
        // Locks were released by the commit batches.
        assert!(e.get_cf("lock", &ukey(0)).unwrap().is_none());
        // Raftlog appends are readable back.
        assert!(e.get_cf("raftlog", b"raftlog/00000001").unwrap().is_some());
    }
}
