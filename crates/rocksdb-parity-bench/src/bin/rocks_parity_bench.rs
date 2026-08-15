//! rocksdb-compat vs real RocksDB — YCSB-shaped parity bench (single node,
//! single client, same op schedule both engines).
//!
//! Usage:
//!   cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- <out_dir> [engine]
//!     engine: compat (default; rocksdb-compat on pedradb-core)
//!            rocksdb (needs --features real; real RocksDB via the rocksdb crate)
//!
//! Env: ROCKS_YCSB_RECORDS/OPS/PAYLOAD/DIST (uniform|zipfian), ROCKS_PARITY_SYNC
//! (rocksdb engine only; 1 = sync per write to match Pedra fsync-before-Ok).
//!
//! Writes <out_dir>/rocks_parity_bench.json.

#![forbid(unsafe_code)]

use rocksdb_parity_bench::{report_json, Cfg, Engine, YcsbRunner};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

// ── compat engine: rocksdb-compat on pedradb-core (always available) ────────

struct CompatEngine {
    db: rocksdb_compat::DB,
}

impl CompatEngine {
    fn open(path: &Path) -> Self {
        let mut opts = rocksdb_compat::Options::default();
        opts.create_if_missing(true);
        let db = rocksdb_compat::DB::open(&opts, path).expect("compat open");
        Self { db }
    }
}

impl Engine for CompatEngine {
    fn label(&self) -> &'static str {
        "compat"
    }
    fn durability(&self) -> &'static str {
        "fsync-before-ok (pedradb-core WAL)"
    }
    fn sync(&self) -> bool {
        true
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get(k).map_err(|_| ())
    }
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        let mut it = self
            .db
            .iterator(rocksdb_compat::IteratorMode::From(
                start,
                rocksdb_compat::Direction::Forward,
            ))
            .map_err(|_| ())?;
        let mut n = 0;
        while it.valid() && n < cap && it.key() < end {
            n += 1;
            it.next();
        }
        Ok(n)
    }
}

// ── real engine: RocksDB via the rocksdb crate (feature "real") ─────────────

#[cfg(feature = "real")]
mod real {
    use super::Engine;

    pub struct RocksEngine {
        db: rocksdb::DB,
        wopts: rocksdb::WriteOptions,
        sync: bool,
    }

    impl RocksEngine {
        pub fn open(path: &std::path::Path, sync: bool) -> Self {
            let db = rocksdb::DB::open_default(path).expect("rocksdb open");
            let mut wopts = rocksdb::WriteOptions::default();
            wopts.set_sync(sync);
            Self { db, wopts, sync }
        }
    }

    impl Engine for RocksEngine {
        fn label(&self) -> &'static str {
            "rocksdb"
        }
        fn durability(&self) -> &'static str {
            if self.sync {
                "sync-per-write (WriteOptions.sync=true)"
            } else {
                "async-wal (WriteOptions.sync=false, rocksdb default)"
            }
        }
        fn sync(&self) -> bool {
            self.sync
        }
        fn put(&self, k: &[u8], v: &[u8]) -> bool {
            self.db.put_opt(k, v, &self.wopts).is_ok()
        }
        fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
            self.db.get(k).map_err(|_| ())
        }
        fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
            Ok(self
                .db
                .iterator(rocksdb::IteratorMode::From(
                    start,
                    rocksdb::Direction::Forward,
                ))
                .map_while(|r| r.ok())
                .take(cap)
                .take_while(|(k, _)| k.as_ref() < end)
                .count())
        }
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "findings/rocks-parity".into());
    let engine_sel = std::env::args().nth(2).unwrap_or_else(|| "compat".into());
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("mkdir out");

    let cfg = Cfg::from_env(200);
    let dbdir = out.join(format!("db-{engine_sel}"));
    let _ = std::fs::remove_dir_all(&dbdir);
    std::fs::create_dir_all(&dbdir).expect("mkdir db");

    eprintln!(
        "[rocks-parity] engine={engine_sel} records={} ops={} payload={} dist={}",
        cfg.records,
        cfg.ops,
        cfg.payload,
        cfg.dist_label()
    );

    match engine_sel.as_str() {
        "compat" => {
            let e = CompatEngine::open(&dbdir);
            let benches = run_all(&e, &cfg);
            write_report(&e, &cfg, &benches, &out);
        }
        "rocksdb" => {
            #[cfg(feature = "real")]
            {
                let sync = rocksdb_parity_bench::env_usize("ROCKS_PARITY_SYNC", 1) != 0;
                let e = real::RocksEngine::open(&dbdir, sync);
                let benches = run_all(&e, &cfg);
                write_report(&e, &cfg, &benches, &out);
            }
            #[cfg(not(feature = "real"))]
            {
                eprintln!("engine 'rocksdb' needs --features real (builds real RocksDB via the rocksdb crate)");
                std::process::exit(1);
            }
        }
        other => {
            eprintln!("unknown engine {other:?} (want compat|rocksdb)");
            std::process::exit(1);
        }
    }
}

fn run_all<E: Engine>(e: &E, cfg: &Cfg) -> Vec<String> {
    let t0 = std::time::Instant::now();
    let mut r = YcsbRunner::new(cfg.clone());
    r.seed(e);
    eprintln!(
        "[rocks-parity] seed {} records in {:.1}s",
        cfg.records,
        t0.elapsed().as_secs_f64()
    );
    vec![
        r.run(e, "ycsb_a", 50, 0, false, false),
        r.run(e, "ycsb_b", 95, 0, false, false),
        r.run(e, "ycsb_c", 100, 0, false, false),
        r.run(e, "ycsb_d", 95, 5, false, false),
        r.run(e, "ycsb_e", 0, 5, false, true),
        r.run(e, "ycsb_f", 50, 0, true, false),
    ]
}

fn write_report<E: Engine>(e: &E, cfg: &Cfg, benches: &[String], out: &Path) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let report = report_json(e, cfg, benches);
    let path = out.join("rocks_parity_bench.json");
    std::fs::write(&path, &report).expect("write bench");
    println!("{report}");
    eprintln!("[rocks-parity] wrote {} (ts={ts})", path.display());
}
