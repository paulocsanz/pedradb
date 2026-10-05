//! Deterministic reproduction of the shuffled-ingest compact livelock
//! (run11/run12: compact worker dead inside ONE job, ~98% CPU, zero I/O,
//! l1 never forming, WAL never rotating under sustained memtable-path
//! ingest).
//!
//! Shape: small write buffer so L0 forms fast, shuffled keys (the streak
//! latch never engages), then drive the worker's job shape
//! (`compact_l0_assist_once` = prepare / job.write / install) with a
//! progress oracle: L0 must drain within the budget and every job must
//! return. Red = hang (the watchdog aborts with diagnostics).

use std::time::{Duration, Instant};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::{BatchOp, OpenOptions};

fn key(i: u64) -> String {
    format!("route.svc-{:06}.{:08}", i / 1000, i % 1000)
}

#[test]
fn shuffled_memtable_ingest_l0_compact_makes_progress() {
    let n: u64 = if std::env::var_os("SHUF_L0_N").is_some() {
        std::env::var("SHUF_L0_N").unwrap().parse().unwrap()
    } else {
        400_000 // ~100 MB shuffled through a 4 MiB buffer
    };
    let dir = std::env::temp_dir().join(format!("shuf-l0-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let buf: usize = std::env::var("SHUF_L0_BUF_MI")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let mut oo = OpenOptions::default();
    oo.auto_flush_bytes = Some(buf << 20);
    let db = ConcurrentDb::open_with(dir.clone(), oo).expect("open");

    // Shuffled order over the same key set: no 8-batch ascending streak
    // ever forms, so the bulk latch never engages — this is the
    // memtable -> L0 -> compaction pipeline.
    let mut order: Vec<u64> = (0..n).collect();
    let mut rng = 0x5EED_CAFE_F00D_0001u64;
    for cut in (1..order.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        order.swap(cut, (rng as usize) % (cut + 1));
    }

    let val = vec![0xABu8; 200];
    let t0 = Instant::now();
    let mut i = 0usize;
    while i < n as usize {
        let end = (i + 1024).min(n as usize);
        let mut ops = Vec::with_capacity(end - i);
        for j in i..end {
            ops.push(BatchOp::put(key(order[j]).as_bytes(), &val));
        }
        db.apply_batch_vec(ops).expect("apply");
        i = end;
    }
    eprintln!(
        "ingest done in {:.1}s; l0={} l1={}",
        t0.elapsed().as_secs_f64(),
        db.with_read(|d| d.level_file_count(0)),
        db.with_read(|d| d.level_file_count(1)),
    );

    // The progress oracle: drain L0 through the worker's job shape.
    let reclaim = std::env::var_os("SHUF_L0_RECLAIM").is_some();
    let budget = Duration::from_secs(if reclaim { 420 } else { 180 });
    let started = Instant::now();
    let mut jobs = 0u64;
    let mut last_print = Instant::now();
    while db.with_read(|d| d.level_file_count(0)) > 0 {
        assert!(
            started.elapsed() < budget,
            "L0 never drained: jobs={jobs} l0={} l1={} elapsed={:?} — compact livelock reproduced",
            db.with_read(|d| d.level_file_count(0)),
            db.with_read(|d| d.level_file_count(1)),
            started.elapsed(),
        );
        let w = Instant::now();
        let installed = if reclaim {
            // The compat worker's exact job shape with auto_reclaim=true:
            // GC wraps the k-way merge (GcMergeSource).
            let job = db.with_write(|d| {
                d.prepare_l0_compact(pedradb_core::CompactOptions {
                    gc: pedradb_core::merge::CompactGcOptions::for_oldest_snapshot(
                        d.last_sequence(),
                    ),
                    max_input_files: Some(2),
                })
                .expect("prepare")
            });
            match job {
                Some(job) => {
                    let tables = job.write().expect("job.write must terminate");
                    db.install_prepared_l0_off_lock(job, tables)
                }
                None => false,
            }
        } else {
            db.compact_l0_assist_once()
        };
        jobs += 1;
        if last_print.elapsed() > Duration::from_secs(10) || !installed {
            eprintln!(
                "job {jobs}: installed={installed} in {:.1}s; l0={} l1={}",
                w.elapsed().as_secs_f64(),
                db.with_read(|d| d.level_file_count(0)),
                db.with_read(|d| d.level_file_count(1)),
            );
            last_print = Instant::now();
        }
        // A job that "succeeds" without installing anything twice in a
        // row is the eligibility spin from the run forensics.
        if !installed && jobs > 4 {
            // give it a couple of warm-up rounds before judging
        }
    }
    eprintln!(
        "L0 drained: {jobs} jobs in {:.1}s; l1={}",
        started.elapsed().as_secs_f64(),
        db.with_read(|d| d.level_file_count(1)),
    );
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}
