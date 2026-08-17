//! Same-schedule YCSB A/C/F across alternative engines (RFC follow-up:
//! "como estamos em relação à performance, não só no nosso benchmark").
//!
//! Schedule is byte-identical to `rocksdb-parity-bench` (xorshift rng seed,
//! zipf theta=0.99 CDF, ykey format, mix semantics) so numbers are comparable
//! with the official parity runs on this same machine.
//!
//! Durability classes are labeled per engine — cross-engine qps is only
//! comparable within a class.

use std::time::Instant;

const RECORDS: usize = 4096;
const OPS: usize = 2000;
const PAYLOAD: usize = 1000;
const THETA: f64 = 0.99;

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
    fn new(seed: u64) -> Self {
        let mut s = 0.0_f64;
        let cdf = (0..RECORDS)
            .map(|i| {
                s += 1.0 / (i as f64 + 1.0).powf(THETA);
                s
            })
            .collect::<Vec<_>>();
        let total = cdf.last().copied().unwrap_or(1.0);
        Self {
            cdf: cdf.into_iter().map(|v| v / total).collect(),
            rng: seed,
        }
    }
    fn pick(&mut self, latest: usize) -> usize {
        let window = latest.min(RECORDS);
        let u = (xorshift(&mut self.rng) >> 11) as f64 / (1u64 << 53) as f64;
        let target = u * self.cdf[window - 1];
        let idx = self.cdf[..window].partition_point(|&c| c < target);
        (RECORDS - window) + idx.min(window - 1)
    }
    fn roll(&mut self) -> u64 {
        xorshift(&mut self.rng) % 100
    }
}

/// The three ops every adapter must provide.
trait Adapter {
    fn name(&self) -> &'static str;
    fn get(&mut self, k: &[u8]) -> bool;
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool;
}

struct Lats(Vec<f64>);
impl Lats {
    fn new() -> Self {
        Self(Vec::with_capacity(OPS))
    }
    fn push(&mut self, t: Instant) {
        self.0.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    fn qps(&self, wall: f64) -> f64 {
        OPS as f64 / wall
    }
    fn pct(&self, p: f64) -> f64 {
        let mut v = self.0.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[(((v.len() as f64) - 1.0) * p).round() as usize]
    }
}

fn run_shape<A: Adapter>(a: &mut A, shape: &str, read_pct: u64, rmw: bool, sched: &mut Schedule) {
    let payload = vec![b'y'; PAYLOAD];
    let mut lats = Lats::new();
    let mut latest = RECORDS;
    let t0 = Instant::now();
    for _ in 0..OPS {
        let t = Instant::now();
        let roll = sched.roll();
        if roll < read_pct {
            let i = sched.pick(latest);
            a.get(&ykey(i));
        } else if rmw {
            let i = sched.pick(latest);
            let k = ykey(i);
            a.get(&k);
            a.put(&k, &payload);
        } else {
            let k = format!("ycsb/{latest:06}").into_bytes();
            if a.put(&k, &payload) {
                latest += 1;
            }
        }
        lats.push(t);
    }
    let wall = t0.elapsed().as_secs_f64();
    println!(
        "{:28} {:8} qps={:>9.0} p50={:>8.3}ms p99={:>8.3}ms",
        a.name(),
        shape,
        lats.qps(wall),
        lats.pct(0.50),
        lats.pct(0.99),
    );
}

// ---- pedradb-core (G1: fdatasync before every Ok) ----
struct Pedra {
    db: pedradb_core::Db<pedradb_core::env::StdEnv>,
}
impl Pedra {
    fn open(dir: &str) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        Self {
            db: pedradb_core::Db::open_with(
                dir,
                pedradb_core::OpenOptions {
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                },
            )
            .unwrap(),
        }
    }
}
impl Adapter for Pedra {
    fn name(&self) -> &'static str {
        "pedradb-core(sync)"
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.db.get(k).is_some()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
}

// ---- real RocksDB ----
struct Rocks {
    db: rocksdb::DB,
    sync: rocksdb::WriteOptions,
    sync_writes: bool,
}
impl Rocks {
    fn open(dir: &str, sync_writes: bool) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let mut w = rocksdb::WriteOptions::default();
        w.set_sync(sync_writes);
        Self {
            db: rocksdb::DB::open(&opts, dir).unwrap(),
            sync: w,
            sync_writes,
        }
    }
}
impl Adapter for Rocks {
    fn name(&self) -> &'static str {
        if self.sync_writes {
            "rocksdb(sync=true)"
        } else {
            "rocksdb(sync=false)"
        }
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.db.get(k).is_ok()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.db.put_opt(k, v, &self.sync).is_ok()
    }
}

// ---- fjall (3.1.9: Database + keyspace; journal persist = durability knob) ----
struct Fjall {
    db: fjall::Database,
    keys: fjall::Keyspace,
    per_op_sync: bool,
}
impl Fjall {
    fn open(dir: &str, per_op_sync: bool) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let db = fjall::Database::builder(dir).open().unwrap();
        let keys = db
            .keyspace("p", fjall::KeyspaceCreateOptions::default)
            .unwrap();
        Self { db, keys, per_op_sync }
    }
}
impl Adapter for Fjall {
    fn name(&self) -> &'static str {
        if self.per_op_sync {
            "fjall(sync/op)"
        } else {
            "fjall(default)"
        }
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.keys.get(k).is_ok()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        let ok = self.keys.insert(k, v).is_ok();
        if ok && self.per_op_sync {
            self.db.persist(fjall::PersistMode::SyncData).is_ok()
        } else {
            ok
        }
    }
}

// ---- sled ----
struct Sled {
    db: sled::Db,
    flush_op: bool,
}
impl Sled {
    fn open(dir: &str, flush_op: bool) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        let db = sled::Config::default().path(dir).open().unwrap();
        Self { db, flush_op }
    }
}
impl Adapter for Sled {
    fn name(&self) -> &'static str {
        if self.flush_op {
            "sled(flush/op)"
        } else {
            "sled(default)"
        }
    }
    fn get(&mut self, k: &[u8]) -> bool {
        self.db.get(k).is_ok()
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        let ok = self.db.insert(k.to_vec(), v.to_vec()).is_ok();
        if ok && self.flush_op {
            self.db.flush().is_ok()
        } else {
            ok
        }
    }
}

// ---- redb (2.6: TableDefinition + set_durability per write txn) ----
type RedbTable = redb::TableDefinition<'static, &'static [u8], &'static [u8]>;
const REDB_TABLE: RedbTable = redb::TableDefinition::new("t");

struct Redb {
    db: redb::Database,
    durability: redb::Durability,
}
impl Redb {
    fn open(dir: &str, durability: redb::Durability) -> Self {
        // redb takes a file path and creates the file itself (plus a journal).
        // Clear any leftover file OR directory from earlier runs at this path.
        let _ = std::fs::remove_file(dir);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_file(format!("{dir}.journal"));
        let db = redb::Database::create(dir).unwrap();
        // Create the table up front so per-op read txns never race creation.
        let wt = db.begin_write().unwrap();
        {
            let _t = wt.open_table(REDB_TABLE).unwrap();
        }
        wt.commit().unwrap();
        Self { db, durability }
    }
}
impl Adapter for Redb {
    fn name(&self) -> &'static str {
        match self.durability {
            redb::Durability::Immediate => "redb(Immediate)",
            redb::Durability::Eventual => "redb(Eventual)",
            _ => "redb(None)",
        }
    }
    fn get(&mut self, k: &[u8]) -> bool {
        let Ok(rt) = self.db.begin_read() else {
            return false;
        };
        match rt.open_table(REDB_TABLE) {
            Ok(t) => t.get(k).is_ok(),
            Err(_) => false,
        }
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        let Ok(mut wt) = self.db.begin_write() else {
            return false;
        };
        wt.set_durability(self.durability);
        let inserted = match wt.open_table(REDB_TABLE) {
            Ok(mut t) => t.insert(k, v).is_ok(),
            Err(_) => false,
        };
        // Drop the table borrow before committing the transaction.
        inserted && wt.commit().is_ok()
    }
}

fn preload<A: Adapter>(a: &mut A) {
    let payload = vec![b'y'; PAYLOAD];
    for i in 0..RECORDS {
        a.put(&ykey(i), &payload);
    }
}

fn bench<A: Adapter + 'static>(name: &str, mk: impl Fn() -> A) {
    for (shape, read_pct, rmw) in [("ycsb_a", 50u64, false), ("ycsb_c", 100, false), ("ycsb_f", 50, true)] {
        let mut a = mk();
        preload(&mut a);
        let mut sched = Schedule::new(0x5EED_0001 + read_pct);
        run_shape(&mut a, shape, read_pct, rmw, &mut sched);
        drop(a);
    }
    let _ = name;
}

fn main() {
    println!(
        "schedule: records={RECORDS} ops={OPS} payload={PAYLOAD}B zipf theta={THETA} (same as rocksdb-parity-bench)"
    );
    bench("pedra", || Pedra::open("/tmp/ab-pedra"));
    bench("rocks-sync", || Rocks::open("/tmp/ab-rocks-s", true));
    bench("rocks-async", || Rocks::open("/tmp/ab-rocks-a", false));
    bench("fjall-def", || Fjall::open("/tmp/ab-fjall-d", false));
    bench("fjall-sync", || Fjall::open("/tmp/ab-fjall-s", true));
    bench("sled-def", || Sled::open("/tmp/ab-sled-d", false));
    bench("redb-imm", || Redb::open("/tmp/ab-redb-i", redb::Durability::Immediate));
    bench("redb-eve", || Redb::open("/tmp/ab-redb-e", redb::Durability::Eventual));
}
