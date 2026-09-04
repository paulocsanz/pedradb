//! RFC-0054 P0.1 — deps_raftlog write-core vs bench-shape, same process.
//!
//! Official battery is 2000 × 16 puts, async WAL, 256 MiB memtable (no flush).
//! In-process write-core is ~4 µs; the official leg is 13–15 µs. This probe
//! prints both in one process so the next cut has a number, not a theory.
//!
//! ```text
//! PEDRA_WRITE_PHASE_STATS=1 cargo run --release -p rocksdb-parity-bench \
//!     --example raftlog_submit_probe
//! ```

use rocksdb_compat::{Options, DB};
use std::time::Instant;

fn main() {
    let batches: usize = std::env::var("RAFTLOG_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let per_batch = 16usize;
    let val = vec![b'r'; 100];

    let dir = std::env::temp_dir().join(format!("pedra-rlsubmit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut opts = Options::new();
    opts.create_if_missing(true);
    opts.set_sync(false);
    opts.set_write_buffer_size(256 * 1024 * 1024);
    let db = DB::open_cf(&opts, &dir, &["raftlog"]).expect("open");

    // Write-core: construction outside the timer (same as raftlog_tail_probe).
    let mut idx = 0u64;
    let mut core: Vec<u128> = Vec::with_capacity(batches);
    for _ in 0..batches {
        let mut puts = Vec::with_capacity(per_batch);
        for _ in 0..per_batch {
            idx += 1;
            puts.push((
                "raftlog",
                format!("raftlog/{idx:08}").into_bytes(),
                val.clone(),
            ));
        }
        let t = Instant::now();
        db.write_cf_owned(puts, Vec::new()).unwrap();
        core.push(t.elapsed().as_nanos());
    }
    core.sort_unstable();
    let q = |v: &[u128], p: f64| v[((v.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
    println!(
        "raftlog write-core: {batches} x {per_batch}: p50={:.2}µs p95={:.2}µs",
        q(&core, 0.50),
        q(&core, 0.95)
    );

    // Bench-shape: construction + Instant + every-8th get inside the timer.
    let cf = db.cf_handle("raftlog").expect("cf");
    let mut shape: Vec<u128> = Vec::with_capacity(batches);
    let t_all = Instant::now();
    for op in 0..batches {
        let t = Instant::now();
        let mut wb = Vec::with_capacity(per_batch);
        for _ in 0..per_batch {
            idx += 1;
            wb.push((
                "raftlog",
                format!("raftlog/{idx:08}").into_bytes(),
                val.clone(),
            ));
        }
        db.write_cf_owned(wb, Vec::new()).unwrap();
        if op % 8 == 0 && idx > 1 {
            let _ = db.get_cf(&cf, format!("raftlog/{:08}", idx - 1).as_bytes());
        }
        shape.push(t.elapsed().as_nanos());
    }
    let wall = t_all.elapsed();
    shape.sort_unstable();
    println!(
        "raftlog bench-shape: {batches} x {per_batch}: p50={:.2}µs p95={:.2}µs mean={:.2}µs (leg {:.4}s, {:.0} batches/s)",
        q(&shape, 0.50),
        q(&shape, 0.95),
        wall.as_nanos() as f64 / batches as f64 / 1000.0,
        wall.as_secs_f64(),
        batches as f64 / wall.as_secs_f64()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
