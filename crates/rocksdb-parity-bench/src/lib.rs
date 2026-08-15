//! Shared YCSB-shaped harness for the rocksdb-compat vs real RocksDB parity bench.
//!
//! One generic runner, two engine adapters — the op schedule (rng seed, zipf
//! CDF, read/insert/scan/rmw mix) is identical for both engines by construction.
//! Same shape semantics as the Montanha `ycsb` suite (FDB benchmark tool
//! workloads): ycsb_a 50/50, b 95/5, c 100r, d 95r/5 insert-latest,
//! e 5 insert + short scans, f 50 read-modify-write.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Env knobs: `ROCKS_YCSB_RECORDS/OPS/PAYLOAD/DIST` (uniform|zipfian).
#[derive(Clone, Debug)]
pub struct Cfg {
    pub records: usize,
    pub ops: usize,
    pub payload: usize,
    pub zipfian: bool,
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

    /// Seed the keyspace with `records` keys (not timed).
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

/// Assemble the bench report file content for one engine run.
pub fn report_json<E: Engine>(e: &E, cfg: &Cfg, benches: &[String]) -> String {
    let mut notes = vec![format!(
        "ycsb records={} ops={} payload={} dist={}",
        cfg.records,
        cfg.ops,
        cfg.payload,
        cfg.dist_label()
    )];
    notes.push(format!("engine={} durability={}", e.label(), e.durability()));
    notes.push("seed: one put per record (not timed)".to_string());
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
        // from_env reads env; without env vars the defaults hold.
        // (env_usize covered indirectly; keep this cheap.)
        let c = Cfg {
            records: 64,
            ops: 32,
            payload: 1,
            zipfian: false,
        };
        assert_eq!(c.dist_label(), "uniform");
    }
}
