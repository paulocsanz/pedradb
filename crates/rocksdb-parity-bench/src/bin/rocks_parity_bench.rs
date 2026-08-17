//! rocksdb-compat vs real RocksDB — YCSB + dependent-shaped parity bench
//! (single node, single client, same op schedule both engines).
//!
//! Usage:
//!   cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- <out_dir> [engine]
//!     engine: compat (default; rocksdb-compat on pedradb-core)
//!            rocksdb (needs --features real; real RocksDB via the rocksdb crate)
//!   suites: ROCKS_PARITY_SUITE (default "ycsb,deps"; csv, "all" = both)
//!
//! Env: ROCKS_YCSB_RECORDS/OPS/PAYLOAD/DIST (uniform|zipfian), ROCKS_DEPS_BATCH
//! (ops per apply commit), ROCKS_PARITY_SYNC (rocksdb engine only; **0 = default
//! Rocks async WAL — official peer**; 1 = sync-per-write, same-class column).
//! write to match Pedra fsync-before-Ok).
//!
//! Writes <out_dir>/rocks_parity_bench.json.

#![forbid(unsafe_code)]

use rocksdb_parity_bench::{report_json, suites_enabled, Cfg, Engine, YcsbRunner};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "findings/rocks-parity".into());
    let engine_sel = std::env::args().nth(2).unwrap_or_else(|| "compat".into());
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("mkdir out");

    let cfg = Cfg::from_env(200);
    let suites = if suites_enabled("ycsb") && suites_enabled("deps") {
        "ycsb,deps"
    } else if suites_enabled("ycsb") {
        "ycsb"
    } else {
        "deps"
    };
    let dbdir = out.join(format!("db-{engine_sel}"));
    let _ = std::fs::remove_dir_all(&dbdir);
    std::fs::create_dir_all(&dbdir).expect("mkdir db");

    eprintln!(
        "[rocks-parity] engine={engine_sel} suites={suites} records={} ops={} payload={} dist={} batch={}",
        cfg.records,
        cfg.ops,
        cfg.payload,
        cfg.dist_label(),
        cfg.batch
    );

    match engine_sel.as_str() {
        "compat" => {
            let e = rocksdb_parity_bench::engines::CompatEngine::open(&dbdir);
            run_and_report(&e, &cfg, suites, &out);
        }
        "concurrent" => {
            if suites_enabled("deps") {
                eprintln!("engine 'concurrent' is ycsb-only (CF ops unimplemented; RFC-0037 P2.2)");
                std::process::exit(1);
            }
            let e = rocksdb_parity_bench::engines::ConcurrentEngine::open(&dbdir);
            run_and_report(&e, &cfg, suites, &out);
        }
        "rocksdb" => {
            #[cfg(feature = "real")]
            {
                let sync = rocksdb_parity_bench::env_usize("ROCKS_PARITY_SYNC", 0) != 0;
                let e = rocksdb_parity_bench::engines::RocksEngine::open(&dbdir, sync);
                run_and_report(&e, &cfg, suites, &out);
            }
            #[cfg(not(feature = "real"))]
            {
                let _ = suites;
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

fn run_and_report<E: Engine + Sync>(e: &E, cfg: &Cfg, suites: &str, out: &Path) {
    let mut r = YcsbRunner::new(cfg.clone());
    let mut benches = Vec::new();
    if suites_enabled("ycsb") {
        let t0 = std::time::Instant::now();
        r.seed(e);
        eprintln!(
            "[rocks-parity] seed {} records in {:.1}s",
            cfg.records,
            t0.elapsed().as_secs_f64()
        );
        benches.extend([
            r.run(e, "ycsb_a", 50, 0, false, false),
            r.run(e, "ycsb_b", 95, 0, false, false),
            r.run(e, "ycsb_c", 100, 0, false, false),
            r.run(e, "ycsb_d", 95, 5, false, false),
            r.run(e, "ycsb_e", 0, 5, false, true),
            r.run(e, "ycsb_f", 50, 0, true, false),
        ]);
        let clients = rocksdb_parity_bench::env_usize("ROCKS_PARITY_CLIENTS", 1);
        if clients >= 2 {
            benches.extend(r.run_clients(e, clients));
        }
    }
    if suites_enabled("deps") {
        benches.extend(r.run_deps(e));
        let clients = rocksdb_parity_bench::env_usize("ROCKS_PARITY_CLIENTS", 1);
        if clients >= 2 {
            benches.extend(r.run_deps_clients(e, clients));
        }
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let report = report_json(e, cfg, &benches, suites);
    let path = out.join("rocks_parity_bench.json");
    std::fs::write(&path, &report).expect("write bench");
    println!("{report}");
    eprintln!("[rocks-parity] wrote {} (ts={ts})", path.display());
}
