//! rocksdb-compat vs real RocksDB — YCSB + dependent-shaped parity bench
//! (single node, single client, same op schedule both engines).
//!
//! Usage:
//!   cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-bench -- <out_dir> [engine]
//!     engine: compat (default; rocksdb-compat on pedradb-core)
//!            rocksdb (needs --features real; real RocksDB via the rocksdb crate)
//!   suites: ROCKS_PARITY_SUITE (default "ycsb,deps"; csv; "all" = every
//!            suite). Opt-in: qs, kvrocks, myrocks, surreal, nebula,
//!            streaming, ceph, solana, arango, venice, oxigraph, rocksapi (RFC-0043).
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
    let suites = {
        let mut v = Vec::new();
        if suites_enabled("ycsb") {
            v.push("ycsb");
        }
        if suites_enabled("deps") {
            v.push("deps");
        }
        if suites_enabled("qs") {
            v.push("qs");
        }
        if suites_enabled("kvrocks") {
            v.push("kvrocks");
        }
        if suites_enabled("myrocks") {
            v.push("myrocks");
        }
        if suites_enabled("surreal") {
            v.push("surreal");
        }
        if suites_enabled("nebula") {
            v.push("nebula");
        }
        if suites_enabled("streaming") {
            v.push("streaming");
        }
        if suites_enabled("ceph") {
            v.push("ceph");
        }
        if suites_enabled("solana") {
            v.push("solana");
        }
        if suites_enabled("arango") {
            v.push("arango");
        }
        if suites_enabled("venice") {
            v.push("venice");
        }
        if suites_enabled("oxigraph") {
            v.push("oxigraph");
        }
        if suites_enabled("rocksapi") {
            v.push("rocksapi");
        }
        if v.is_empty() {
            "ycsb,deps".to_string()
        } else {
            v.join(",")
        }
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
            run_and_report_occ(&e, &cfg, suites.as_str(), &out);
        }
        "compatv" => {
            // RFC-0058 P1.2: the verified profile column (StdEnv + lone
            // commit) on the same suites/opts — only the profile differs.
            let e =
                rocksdb_parity_bench::engines::CompatEngine::<pedradb_core::StdEnv>::open_verified(
                    &dbdir,
                );
            run_and_report_occ(&e, &cfg, suites.as_str(), &out);
        }
        "concurrent" => {
            if suites_enabled("deps") {
                eprintln!("engine 'concurrent' is ycsb-only (CF ops unimplemented; RFC-0037 P2.2)");
                std::process::exit(1);
            }
            let e = rocksdb_parity_bench::engines::ConcurrentEngine::open(&dbdir);
            run_and_report(&e, &cfg, suites.as_str(), &out);
        }
        "rocksdb" => {
            #[cfg(feature = "real")]
            {
                let sync = rocksdb_parity_bench::env_usize("ROCKS_PARITY_SYNC", 0) != 0;
                if suites_enabled("surreal") {
                    // OptimisticTransactionDB peer (SurrealDB kv-rocksdb shape).
                    // Official 16 stay on plain DB unless surreal is in the suite.
                    let e = rocksdb_parity_bench::engines::RocksOccEngine::open(&dbdir, sync);
                    run_and_report_occ(&e, &cfg, suites.as_str(), &out);
                } else {
                    let e = rocksdb_parity_bench::engines::RocksEngine::open(&dbdir, sync);
                    run_and_report(&e, &cfg, suites.as_str(), &out);
                }
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

fn push_ycsb<E: Engine>(r: &mut YcsbRunner, e: &E, records: usize, benches: &mut Vec<String>) {
    use rocksdb_parity_bench::shape_wanted;
    let t0 = std::time::Instant::now();
    // RFC-0163 P1.2: untimed seed may run WAL-async (env-gated) so the G1
    // column doesn't pay per-put fdatasync at 25M records; every timed
    // write shape keeps the column sync.
    rocksdb_parity_bench::seed_under_async_barrier(
        e,
        rocksdb_parity_bench::seed_async_enabled(),
        || r.seed(e),
    );
    eprintln!(
        "[rocks-parity] seed {records} records in {:.1}s",
        t0.elapsed().as_secs_f64()
    );
    if shape_wanted("ycsb_a") {
        benches.push(r.run(e, "ycsb_a", 50, 0, false, false));
    }
    if shape_wanted("ycsb_b") {
        benches.push(r.run(e, "ycsb_b", 95, 0, false, false));
    }
    if shape_wanted("ycsb_c") {
        benches.push(r.run(e, "ycsb_c", 100, 0, false, false));
    }
    if shape_wanted("ycsb_d") {
        benches.push(r.run(e, "ycsb_d", 95, 5, false, false));
    }
    if shape_wanted("ycsb_e") {
        benches.push(r.run(e, "ycsb_e", 0, 5, false, true));
    }
    if shape_wanted("ycsb_f") {
        benches.push(r.run(e, "ycsb_f", 50, 0, true, false));
    }
    if shape_wanted("ycsb_b_unif") {
        benches.push(r.run_dist(e, "ycsb_b_unif", 95, 0, false, false, true));
    }
    if shape_wanted("ycsb_c_unif") {
        benches.push(r.run_dist(e, "ycsb_c_unif", 100, 0, false, false, true));
    }
    if shape_wanted("ycsb_c_big") {
        if let Some(b) = r.run_c_big(e) {
            benches.push(b);
        }
    }
}

fn run_and_report<E: Engine + Sync>(e: &E, cfg: &Cfg, suites: &str, out: &Path) {
    let mut r = YcsbRunner::new(cfg.clone());
    let mut benches = Vec::new();
    if suites_enabled("ycsb") {
        push_ycsb(&mut r, e, cfg.records, &mut benches);
        for clients in rocksdb_parity_bench::clients_from_env() {
            benches.extend(r.run_clients(e, clients));
        }
    }
    if suites_enabled("deps") {
        benches.extend(r.run_deps(e));
        for clients in rocksdb_parity_bench::clients_from_env() {
            benches.extend(r.run_deps_clients(e, clients));
        }
    }
    if suites_enabled("qs") {
        if !suites_enabled("ycsb") {
            r.seed(e);
        }
        benches.extend(r.run_qs(e));
    }
    if suites_enabled("kvrocks") {
        benches.extend(r.run_kvrocks(e));
    }
    if suites_enabled("myrocks") {
        benches.extend(r.run_myrocks(e));
    }
    if suites_enabled("nebula") {
        benches.extend(r.run_nebula(e));
    }
    if suites_enabled("streaming") {
        benches.extend(r.run_streaming(e));
    }
    if suites_enabled("ceph") {
        benches.extend(r.run_ceph(e));
    }
    if suites_enabled("solana") {
        benches.extend(r.run_solana(e));
    }
    if suites_enabled("arango") {
        benches.extend(r.run_arango(e));
    }
    if suites_enabled("venice") {
        benches.extend(r.run_venice(e));
    }
    if suites_enabled("oxigraph") {
        benches.extend(r.run_oxigraph(e));
    }
    if suites_enabled("rocksapi") {
        benches.extend(r.run_rocksapi(e));
    }
    if suites_enabled("surreal") {
        eprintln!(
            "[rocks-parity] engine {} cannot run surreal (need OccEngine / OptimisticTransactionDB)",
            e.label()
        );
        std::process::exit(1);
    }

    finish_report(e, cfg, suites, out, benches);
}

fn run_and_report_occ<E: rocksdb_parity_bench::OccEngine + Sync>(
    e: &E,
    cfg: &Cfg,
    suites: &str,
    out: &Path,
) {
    let mut r = YcsbRunner::new(cfg.clone());
    let mut benches = Vec::new();
    if suites_enabled("ycsb") {
        push_ycsb(&mut r, e, cfg.records, &mut benches);
        for clients in rocksdb_parity_bench::clients_from_env() {
            benches.extend(r.run_clients(e, clients));
        }
    }
    if suites_enabled("deps") {
        benches.extend(r.run_deps(e));
        for clients in rocksdb_parity_bench::clients_from_env() {
            benches.extend(r.run_deps_clients(e, clients));
        }
    }
    if suites_enabled("qs") {
        if !suites_enabled("ycsb") {
            r.seed(e);
        }
        benches.extend(r.run_qs(e));
    }
    if suites_enabled("kvrocks") {
        benches.extend(r.run_kvrocks(e));
    }
    if suites_enabled("myrocks") {
        benches.extend(r.run_myrocks(e));
    }
    if suites_enabled("nebula") {
        benches.extend(r.run_nebula(e));
    }
    if suites_enabled("streaming") {
        benches.extend(r.run_streaming(e));
    }
    if suites_enabled("ceph") {
        benches.extend(r.run_ceph(e));
    }
    if suites_enabled("solana") {
        benches.extend(r.run_solana(e));
    }
    if suites_enabled("arango") {
        benches.extend(r.run_arango(e));
    }
    if suites_enabled("venice") {
        benches.extend(r.run_venice(e));
    }
    if suites_enabled("oxigraph") {
        benches.extend(r.run_oxigraph(e));
    }
    if suites_enabled("rocksapi") {
        benches.extend(r.run_rocksapi(e));
    }
    if suites_enabled("surreal") {
        benches.extend(r.run_surreal(e));
    }

    finish_report(e, cfg, suites, out, benches);
}

fn finish_report<E: Engine>(e: &E, cfg: &Cfg, suites: &str, out: &Path, benches: Vec<String>) {
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
