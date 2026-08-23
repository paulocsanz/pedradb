//! RFC-0026 P0.2 — load large values, Zipf overwrite, time `compact_vlog`.
//!
//! Shape measurement on a laptop-sized DB (not HashKV's 40 GiB). Reports
//! live-ratio collapse and rewrite cost (vlog + SST remap), not a WA-vs-Rocks claim.
//!
//! ```text
//! cargo bench -p pedradb-core --bench vlog_zipf -- --keys 2000 --val 4096
//! ```

use std::env;
use std::time::Instant;

use pedradb_core::{CompactOptions, Db, OpenOptions, Rng, SeedRng, WriteOptions};

fn parse_u64(flag: &str, default: u64) -> u64 {
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        if a == flag {
            if let Some(v) = args.next() {
                return v.parse().unwrap_or(default);
            }
        }
    }
    default
}

fn zipf_cdf(n: usize, s: f64) -> Vec<f64> {
    let mut acc = 0.0;
    let mut h = Vec::with_capacity(n);
    for i in 1..=n {
        acc += (i as f64).powf(-s);
        h.push(acc);
    }
    if acc > 0.0 {
        for x in &mut h {
            *x /= acc;
        }
    }
    h
}

fn zipf_sample(cdf: &[f64], u: f64) -> usize {
    cdf.partition_point(|&p| p < u)
        .min(cdf.len().saturating_sub(1))
}

fn key_of(i: u64) -> [u8; 16] {
    let mut k = [b'k'; 16];
    k[8..].copy_from_slice(&i.to_be_bytes());
    k
}

fn unit(u: f64) -> f64 {
    (u / (u64::MAX as f64)).clamp(0.0, 0.999_999)
}

fn main() {
    let n_keys = parse_u64("--keys", 2_000);
    let val_len = parse_u64("--val", 4_096) as usize;
    let passes = parse_u64("--passes", 3) as usize;
    let seed = parse_u64("--seed", 42);
    let zipf_s = 0.99;

    let dir = std::env::temp_dir().join(format!("pedradb-vlog-zipf-{seed}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");

    let mut db = Db::open_with(
        &dir,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: false,
            auto_flush_bytes: Some(256 * 1024),
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: Some(1024),
        },
    )
    .expect("open");

    let payload = vec![0xABu8; val_len];
    let t_load = Instant::now();
    for i in 0..n_keys {
        db.put_with(&key_of(i), &payload, WriteOptions::no_sync())
            .expect("load put");
    }
    db.flush().expect("load flush");
    db.sync().expect("load sync");
    let load_ms = t_load.elapsed().as_secs_f64() * 1000.0;
    let after_load = db.stats();

    let cdf = zipf_cdf(n_keys as usize, zipf_s);
    let rng = SeedRng::new(seed);
    let mut phases = Vec::new();

    for p in 1..=passes {
        let t_up = Instant::now();
        let mut over = payload.clone();
        over[0] = u8::try_from(p).unwrap_or(0xFF);
        for _ in 0..n_keys {
            let u = unit(rng.next_u64() as f64);
            let idx = zipf_sample(&cdf, u) as u64;
            db.put_with(&key_of(idx), &over, WriteOptions::no_sync())
                .expect("zipf put");
        }
        db.flush().expect("phase flush");
        let update_ms = t_up.elapsed().as_secs_f64() * 1000.0;
        // Old vlog records stay live until SST latest_only drops their pointers
        // (same rule as `usage.md`). Measure the garbage that rewrite can actually drop.
        db.compact_with(CompactOptions::latest_only())
            .expect("latest_only");
        let before_gc = db.stats();

        let t_gc = Instant::now();
        let rw = db.compact_vlog().expect("compact_vlog");
        let gc_ms = t_gc.elapsed().as_secs_f64() * 1000.0;
        let after_gc = db.stats();

        phases.push(serde_lite(
            p,
            update_ms,
            gc_ms,
            &before_gc,
            &after_gc,
            rw.bytes_before,
            rw.bytes_after,
            rw.live_records,
        ));
        eprintln!(
            "pass {p}: update={update_ms:.1}ms gc={gc_ms:.1}ms before={} after={}",
            before_gc.vlog_line(),
            after_gc.vlog_line()
        );
    }

    let out = format!(
        "{{\n  \"rfc\": \"0026\",\n  \"seed\": {seed},\n  \"keys\": {n_keys},\n  \"val_bytes\": {val_len},\n  \"zipf_s\": {zipf_s},\n  \"load_ms\": {load_ms:.3},\n  \"after_load\": {},\n  \"phases\": [\n{}  ]\n}}\n",
        stats_json(&after_load),
        phases.join(",\n")
    );
    print!("{out}");
    let _ = db.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn stats_json(s: &pedradb_core::DbStats) -> String {
    format!(
        "{{\"vlog_bytes\":{},\"vlog_live_bytes\":{},\"vlog_live_records\":{},\"vlog_live_ratio\":{:.6},\"vlog_gc_count\":{},\"sst_count\":{},\"sst_bytes\":{},\"bytes_written_sst\":{}}}",
        s.vlog_bytes,
        s.vlog_live_bytes,
        s.vlog_live_records,
        s.vlog_live_ratio(),
        s.vlog_gc_count,
        s.sst_count,
        s.sst_bytes,
        s.bytes_written_sst
    )
}

#[allow(clippy::too_many_arguments)]
fn serde_lite(
    pass: usize,
    update_ms: f64,
    gc_ms: f64,
    before: &pedradb_core::DbStats,
    after: &pedradb_core::DbStats,
    rw_before: u64,
    rw_after: u64,
    live_records: u64,
) -> String {
    format!(
        "    {{\"pass\":{pass},\"update_ms\":{update_ms:.3},\"gc_ms\":{gc_ms:.3},\"rewrite_bytes_before\":{rw_before},\"rewrite_bytes_after\":{rw_after},\"rewrite_live_records\":{live_records},\"before\":{},\"after\":{}}}",
        stats_json(before),
        stats_json(after)
    )
}
