//! Scale ladder: Pedra async (sync=false, write-per-commit) vs fjall/sled
//! **default async**. Same YCSB A/C schedule as alt-engines (zipf θ=0.99).
//!
//! Product Pedra defaults otherwise (4 MiB auto-flush). Cross-engine qps is
//! only comparable inside this durability class (no fsync-per-op).
//!
//! ```text
//! cargo run --release --manifest-path findings/async-scale-ladder/Cargo.toml
//! ```

use std::time::Instant;

fn xorshift(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

fn ykey(i: usize) -> Vec<u8> {
    format!("ycsb/{i:06}").into_bytes()
}

struct Schedule {
    cdf: Vec<f64>,
    rng: u64,
}

impl Schedule {
    fn new(records: usize, seed: u64) -> Self {
        let mut s = 0.0_f64;
        let cdf = (0..records)
            .map(|i| {
                s += 1.0 / (i as f64 + 1.0).powf(0.99);
                s
            })
            .collect::<Vec<_>>();
        let total = cdf.last().copied().unwrap_or(1.0);
        Self {
            cdf: cdf.into_iter().map(|v| v / total).collect(),
            rng: seed,
        }
    }
    fn pick(&mut self, latest: usize, records: usize) -> usize {
        let window = latest.min(records);
        let u = (xorshift(&mut self.rng) >> 11) as f64 / (1u64 << 53) as f64;
        let target = u * self.cdf[window - 1];
        let idx = self.cdf[..window].partition_point(|&c| c < target);
        (records - window) + idx.min(window - 1)
    }
    fn roll(&mut self) -> u64 {
        xorshift(&mut self.rng) % 100
    }
}

trait Adapter {
    fn name(&self) -> &'static str;
    fn get(&mut self, k: &[u8]) -> bool;
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool;
}

fn run_shape<A: Adapter>(
    a: &mut A,
    shape: &str,
    records: usize,
    ops: usize,
    payload: usize,
    read_pct: u64,
) {
    let payload = vec![b'y'; payload];
    let mut sched = Schedule::new(records, 0x5EED_0001);
    let mut latest = records;
    let mut lats = Vec::with_capacity(ops);
    let t0 = Instant::now();
    for _ in 0..ops {
        let t = Instant::now();
        if sched.roll() < read_pct {
            let i = sched.pick(latest, records);
            a.get(&ykey(i));
        } else {
            let k = format!("ycsb/{latest:06}").into_bytes();
            if a.put(&k, &payload) {
                latest += 1;
            }
        }
        lats.push(t.elapsed().as_secs_f64() * 1_000.0);
    }
    let wall = t0.elapsed().as_secs_f64();
    lats.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| lats[(((lats.len() as f64) - 1.0) * p).round() as usize];
    println!(
        "{:22} {:>7} rec={:<7} pay={:<4} qps={:>9.0} p50={:>8.3}ms p99={:>8.3}ms",
        a.name(),
        shape,
        records,
        payload.len(),
        ops as f64 / wall,
        pct(0.50),
        pct(0.99),
    );
}

fn seed<A: Adapter>(a: &mut A, records: usize, payload: &[u8]) {
    for i in 0..records {
        assert!(a.put(&ykey(i), payload), "seed {i}");
    }
}

struct PedraAsync {
    db: pedradb_core::Db<pedradb_core::env::StdEnv>,
    label: &'static str,
}
impl PedraAsync {
    fn open(dir: &str) -> Self {
        Self::open_opts(dir, true)
    }
    fn open_noflush(dir: &str) -> Self {
        Self::open_opts(dir, false)
    }
    fn open_opts(dir: &str, auto_flush: bool) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let mut opts = pedradb_core::OpenOptions {
            sync: false,
            ..pedradb_core::OpenOptions::default()
        };
        if !auto_flush {
            opts.auto_flush_bytes = None;
            opts.auto_compact_sst_count = None;
            opts.auto_compact_sst_bytes = None;
        }
        Self {
            db: pedradb_core::Db::open_with(dir, opts).unwrap(),
            label: if auto_flush {
                "pedra(async)"
            } else {
                "pedra(async,noflush)"
            },
        }
    }
}
impl Adapter for PedraAsync {
    fn name(&self) -> &'static str {
        self.label
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.db.get(k).is_some()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
}

struct FjallDef {
    #[allow(dead_code)]
    db: fjall::Database,
    keys: fjall::Keyspace,
}
impl FjallDef {
    fn open(dir: &str) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let db = fjall::Database::builder(dir).open().unwrap();
        let keys = db
            .keyspace("p", fjall::KeyspaceCreateOptions::default)
            .unwrap();
        Self { db, keys }
    }
}
impl Adapter for FjallDef {
    fn name(&self) -> &'static str {
        "fjall(default)"
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.keys.get(k).is_ok()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.keys.insert(k, v).is_ok()
    }
}

struct SledDef {
    db: sled::Db,
}
impl SledDef {
    fn open(dir: &str) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        Self {
            db: sled::Config::default().path(dir).open().unwrap(),
        }
    }
}
impl Adapter for SledDef {
    fn name(&self) -> &'static str {
        "sled(default)"
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.db.get(k).is_ok()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.db.insert(k.to_vec(), v.to_vec()).is_ok()
    }
}

fn cell<A: Adapter>(
    mut a: A,
    dir: &str,
    records: usize,
    ops: usize,
    payload: usize,
) {
    let val = vec![b'y'; payload];
    let t_seed = Instant::now();
    seed(&mut a, records, &val);
    let seed_s = t_seed.elapsed().as_secs_f64();
    println!(
        "{:22} {:>7} rec={:<7} pay={:<4} seed={:>7.3}s ({:.0} kputs/s)",
        a.name(),
        "load",
        records,
        payload,
        seed_s,
        records as f64 / seed_s / 1000.0
    );
    run_shape(&mut a, "ycsb_a", records, ops, payload, 50);
    run_shape(&mut a, "ycsb_c", records, ops, payload, 100);
    let _ = std::fs::remove_dir_all(dir);
}

fn main() {
    let root = std::env::temp_dir().join(format!("pedra-async-ladder-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let ops = 2000usize;
    // Tiny (alt-engines 4k×1000) through 1M. Payload 1000 only where the
    // working set stays laptop-reasonable.
    let cells: &[(usize, usize)] = &[
        (1_024, 100),
        (4_096, 100),
        (4_096, 1000),
        (16_384, 100),
        (16_384, 1000),
        (65_536, 100),
        (262_144, 100),
        (1_048_576, 100),
    ];
    println!("# pedra async vs fjall/sled default  ops={ops}");
    println!("# Pedra OpenOptions.sync=false, auto_flush=4MiB (product default)");
    for &(records, payload) in cells {
        println!("--- records={records} payload={payload} ---");
        let p = root.join(format!("p-{records}-{payload}"));
        cell(
            PedraAsync::open(p.to_str().unwrap()),
            p.to_str().unwrap(),
            records,
            ops,
            payload,
        );
        if payload >= 1000 {
            let pn = root.join(format!("pn-{records}-{payload}"));
            cell(
                PedraAsync::open_noflush(pn.to_str().unwrap()),
                pn.to_str().unwrap(),
                records,
                ops,
                payload,
            );
        }
        let f = root.join(format!("f-{records}-{payload}"));
        cell(
            FjallDef::open(f.to_str().unwrap()),
            f.to_str().unwrap(),
            records,
            ops,
            payload,
        );
        let s = root.join(format!("s-{records}-{payload}"));
        cell(
            SledDef::open(s.to_str().unwrap()),
            s.to_str().unwrap(),
            records,
            ops,
            payload,
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
