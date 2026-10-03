//! RFC-0306 tooling: deterministic per-phase CPU probe for the slipstream
//! shapes. Run on any box (loaded or idle): medians of fixed op counts,
//! fixed seeds, and a counting global allocator so every phase reports
//! **allocations per op** alongside wall time — allocation churn is the
//! engine-side cost the 100M harness cannot attribute.
//!
//! ```text
//! cargo run --release -p rocksdb-compat --example perf_probe
//! ```
//!
//! Phases: latched apply (µs/batch + allocs/op), settled outside-envelope
//! miss (ns/op), inside-envelope miss (ns/op), warm hit (µs/op). Ends with
//! the `HYDRATEDIAG` writer-phase split when `PEDRA_HYDRATE_DIAG=1`.
//! Output is one stable table + one `PERFPROBE_JSON` line for diffing runs
//! (`scripts/perf_calltree.py --diff-json a.json b.json` style A/B).

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use rocksdb_compat::{ColumnFamilyDescriptor, Options, WriteBatch, WriteOptions, DB};

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREES.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(new_size.saturating_sub(layout.size()) as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn key(i: u64) -> String {
    format!("route.svc-{:06}.{:08}", i / 1000, i % 1000)
}

fn value_pool() -> Vec<u8> {
    let mut pool = vec![0u8; 1 << 20];
    let mut x = 0x5EED_5EED_5EED_5EEDu64;
    for chunk in pool.chunks_mut(8) {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        chunk.copy_from_slice(&x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes()[..chunk.len()]);
    }
    pool
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

fn main() {
    let mul: usize = std::env::var("PROBE_MUL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let n_target: u64 = std::env::var("PROBE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(204_800);
    let per_batch: u64 = 1024;
    let batches = (n_target / per_batch).max(1);
    let n = batches * per_batch;
    // Deterministic shape: PROBE_N entries (default 200k ≈ 1-2 chunks;
    // 25M ≈ 92 chunks — the multi-table residency regime of 100M+).
    let pool = value_pool();
    let dir = std::env::temp_dir().join(format!("perf-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut opts = Options::new();
    opts.create_if_missing(true);
    opts.set_sync(false);
    let mut data_opts = Options::default();
    data_opts.set_write_buffer_size(256 << 20);
    let db = DB::open_cf_descriptors(
        &opts,
        &dir,
        [
            ColumnFamilyDescriptor::new("data", data_opts),
            ColumnFamilyDescriptor::new("meta", Options::default()),
        ],
    )
    .unwrap();
    let data = db.cf_handle("data").unwrap();
    let mut wo = WriteOptions::default();
    wo.set_sync(false);

    // --- Phase 1: latched apply (µs/batch, allocs/op) ---
    let mut batch_us = Vec::new();
    let mut i = 0u64;
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let t_all = Instant::now();
    let rss_probe = |tag: &str| {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output();
        if let Ok(o) = out {
            let kb: String = String::from_utf8_lossy(&o.stdout).trim().to_string();
            eprintln!(
                "  RSS[{tag}] = {kb} KB | live_bytes = {:.2} GiB | allocs = {} frees = {}",
                LIVE_BYTES.load(Ordering::Relaxed) as f64 / (1u64 << 30) as f64,
                ALLOCS.load(Ordering::Relaxed),
                FREES.load(Ordering::Relaxed)
            );
        }
    };
    let sample_every = (batches / 8).max(1);
    let mut b_idx = 0u64;
    rss_probe("apply-start");
    for _ in 0..batches {
        let end = i + per_batch;
        let mut wb = WriteBatch::default();
        for j in i..end {
            let off = ((j * 7919) % (pool.len() as u64 - 200)) as usize;
            wb.put_cf(&data, key(j).as_bytes(), &pool[off..off + 200]);
        }
        wb.put_cf(&db.cf_handle("meta").unwrap(), b"cursor", end.to_le_bytes());
        let t = Instant::now();
        db.write_opt_owned(wb, &wo).unwrap();
        batch_us.push(t.elapsed().as_secs_f64() * 1e6);
        i = end;
        if b_idx % sample_every == 0 {
            rss_probe(&format!("apply-{i}"));
        }
        b_idx += 1;
    }
    rss_probe("apply-end");
    let apply_total_s = t_all.elapsed().as_secs_f64();
    let apply_allocs = ALLOCS.load(Ordering::Relaxed) - a0;
    // RFC-0306 attribution: where does the wall live when median ≪ wall?
    // Histogram the batch-time tail (chunk boundaries vs uniform slowness).
    {
        let mut sorted = batch_us.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let sum: f64 = batch_us.iter().sum();
        let n_slow_1ms = batch_us.iter().filter(|t| **t > 1_000.0).count();
        let slow_sum: f64 = batch_us.iter().filter(|t| **t > 1_000.0).sum();
        let p999 = sorted[(sorted.len() as f64 * 0.999) as usize];
        println!(
            "  batch-tail : mean {:.1}µs | p99 {:.1} | p999 {:.1} | max {:.1} | >1ms: {} batches = {:.1}s of {:.1}s total",
            sum / batch_us.len() as f64,
            sorted[(sorted.len() as f64 * 0.99) as usize],
            p999,
            sorted[sorted.len() - 1],
            n_slow_1ms,
            slow_sum / 1e6,
            sum / 1e6
        );
    }

    // --- Settle so the envelope fast path arms ---
    db.flush().unwrap();
    db.compact().unwrap();
    assert!(db.is_settled_sst_only(), "probe requires a settled store");
    let runs = db.sst_run_debug();
    let total: usize = runs.iter().map(|(_, n, _)| n).sum();
    let armed: usize = runs.iter().filter(|(_, _, d)| *d).map(|(_, n, _)| n).sum();
    println!("  runs       : {:?} | tables {} | disjoint-armed {}", runs, total, armed);

    // --- Phase 2: outside-envelope miss (ns/op) ---
    let mut miss_ns = Vec::new();
    let mut miss_keys = Vec::new();
    for p in 0..20_000u64 {
        miss_keys.push(format!("route.svc-9{:06}.{:08}", p % 999_999, p % 1000));
    }
    let a1 = ALLOCS.load(Ordering::Relaxed);
    let t1 = Instant::now();
    for k in &miss_keys {
        let _ = std::hint::black_box(db.get_named("data", k).unwrap());
    }
    let miss_wall = t1.elapsed().as_secs_f64();
    let miss_allocs = ALLOCS.load(Ordering::Relaxed) - a1;
    // second pass for stable per-op medians
    for _ in 0..3 {
        let t = Instant::now();
        for k in &miss_keys {
            let _ = std::hint::black_box(db.get_named("data", k).unwrap());
        }
        miss_ns.push(t.elapsed().as_secs_f64() / miss_keys.len() as f64 * 1e9);
    }

    // --- Phase 3: inside-envelope miss (ns/op) ---
    let inside_keys: Vec<String> = (0..20_000u64)
        .map(|p| format!("route.svc-{:06}.{:08}x", p % (n / 1000), p % 1000))
        .collect();
    let mut inside_ns = Vec::new();
    let a2 = ALLOCS.load(Ordering::Relaxed);
    let in_blocks0 = rocksdb_compat::probe_counters().blocks_decoded;
    let in_tables0 = db.lookup_tables_probed();
    let in_parts0 = pedradb_core::bloom::filter_part_load_count();
    for rep in 0..(3 * mul.max(1)) {
        let t = Instant::now();
        for k in &inside_keys {
            let _ = std::hint::black_box(db.get_named("data", k).unwrap());
        }
        if rep == 0 {
            inside_ns.push(t.elapsed().as_secs_f64() / inside_keys.len() as f64 * 1e9);
        }
    }
    let inside_allocs = ALLOCS.load(Ordering::Relaxed) - a2;
    let in_ops = 20_000u64 * (3 * mul.max(1)) as u64;
    let in_blocks = rocksdb_compat::probe_counters().blocks_decoded - in_blocks0;
    let in_tables = db.lookup_tables_probed() - in_tables0;
    let in_parts = pedradb_core::bloom::filter_part_load_count() - in_parts0;
    println!(
        "  miss-in x  : blocks/get {:.3} | tables/get {:.3} | part-loads/get {:.3}",
        in_blocks as f64 / in_ops as f64,
        in_tables as f64 / in_ops as f64,
        in_parts as f64 / in_ops as f64
    );

    // --- Phase 4: warm hit (µs/op) ---
    let hit_keys: Vec<String> = (0..20_000u64)
        .map(|p| key((p * 7_919) % n))
        .collect();
    let mut hit_us = Vec::new();
    let a3 = ALLOCS.load(Ordering::Relaxed);
    let blocks0 = rocksdb_compat::probe_counters().blocks_decoded;
    let crc0 = rocksdb_compat::probe_counters().block_crc_skipped;
    let tables0 = db.lookup_tables_probed();
    for rep in 0..(3 * mul.max(1)) {
        let t = Instant::now();
        for k in &hit_keys {
            let _ = std::hint::black_box(db.get_named("data", k).unwrap());
        }
        if rep == 0 {
            hit_us.push(t.elapsed().as_secs_f64() / hit_keys.len() as f64 * 1e6);
        }
    }
    let hit_allocs = ALLOCS.load(Ordering::Relaxed) - a3;
    let tables_probed = db.lookup_tables_probed() - tables0;
    let blocks_decoded = rocksdb_compat::probe_counters().blocks_decoded - blocks0;
    let crc_skipped = rocksdb_compat::probe_counters().block_crc_skipped - crc0;
    let gets = 20_000u64 * (3 * mul.max(1)) as u64;

    let per_op = |c: u64, ops: u64| c as f64 / ops as f64;
    println!("PERFPROBE n={n}");
    println!("  apply      : {:>9.1} µs/batch (median) | allocs/op {:>6.2} | total {apply_total_s:.2}s",
        median(&mut batch_us), per_op(apply_allocs, n));
    println!("  miss-out   : {:>9.1} ns/op (min of medians) | allocs/op {:>6.2}",
        miss_ns.iter().cloned().fold(f64::INFINITY, f64::min), per_op(miss_allocs, 20_000 * 4));
    println!("  miss-in    : {:>9.1} ns/op | allocs/op {:>6.2}",
        median(&mut inside_ns), per_op(inside_allocs, 20_000 * 3));
    println!("  hit-warm   : {:>9.2} µs/op | allocs/op {:>6.2} | blocks/get {:.3} | tables/get {:.3} | crc-skip/get {:.3}",
        median(&mut hit_us), per_op(hit_allocs, 20_000 * 3),
        blocks_decoded as f64 / gets as f64, tables_probed as f64 / gets as f64,
        crc_skipped as f64 / gets as f64);
    {
        let bounds = pedradb_core::bulk_boundary_diag_take();
        if !bounds.is_empty() {
            let sorted = bounds.iter().filter(|(s, _)| *s).count();
            let ns: u64 = bounds.iter().map(|(_, n)| n).sum();
            println!(
                "  boundary  : {} chunks | sorted {} | sort+park total {:.1} ms | max {:.1} ms",
                bounds.len(),
                sorted,
                ns as f64 / 1e6,
                bounds.iter().map(|(_, n)| *n).max().unwrap_or(0) as f64 / 1e6
            );
        }
    }
    if std::env::var_os("PEDRA_HYDRATE_DIAG").is_some() {
        println!("  {}", pedradb_core::write_diag_kernel::latched_bulk_diag_line());
    }
    println!(
        "PERFPROBE_JSON {{\"n\":{n},\"apply_us\":{:.1},\"apply_allocs_per_op\":{:.2},\"miss_out_ns\":{:.0},\"miss_in_ns\":{:.0},\"hit_us\":{:.2},\"hit_allocs_per_op\":{:.2},\"blocks_per_get\":{:.3},\"miss_wall_s\":{:.3}}}",
        median(&mut batch_us.clone()),
        per_op(apply_allocs, n),
        miss_ns.iter().cloned().fold(f64::INFINITY, f64::min),
        median(&mut inside_ns),
        median(&mut hit_us),
        per_op(hit_allocs, 20_000 * 3),
        blocks_decoded as f64 / gets as f64,
        miss_wall
    );

    // RFC-0308 Pilar E: cold-reopen attribution — first 100 gets after a
    // fresh open, with part-loads/tables/blocks deltas (PEDRA_PROBE_COLD=1).
    if std::env::var_os("PEDRA_PROBE_COLD").is_some() {
        drop(db);
        let mut opts2 = Options::new();
        opts2.create_if_missing(true);
        let mut data_opts2 = Options::default();
        data_opts2.set_write_buffer_size(256 << 20);
        let db2 = DB::open_cf_descriptors(
            &opts2,
            &dir,
            [
                ColumnFamilyDescriptor::new("data", data_opts2),
                ColumnFamilyDescriptor::new("meta", Options::default()),
            ],
        )
        .unwrap();
        let cold0 = pedradb_core::bloom::filter_part_load_count();
        let ct0 = db2.lookup_tables_probed();
        let cb0 = rocksdb_compat::probe_counters().blocks_decoded;
        let t0 = std::time::Instant::now();
        let mut hit_n = 0usize;
        for p in 0..100u64 {
            let k = key((p * 7_919_927) % n);
            if db2.get_named("data", &k).unwrap().is_some() {
                hit_n += 1;
            }
        }
        let cold_us = t0.elapsed().as_secs_f64() / 100.0 * 1e6;
        println!(
            "  cold-100   : {:.2} us/get | hits {hit_n}/100 | part-loads/get {:.3} | tables/get {:.3} | blocks/get {:.3}",
            cold_us,
            (pedradb_core::bloom::filter_part_load_count() - cold0) as f64 / 100.0,
            (db2.lookup_tables_probed() - ct0) as f64 / 100.0,
            (rocksdb_compat::probe_counters().blocks_decoded - cb0) as f64 / 100.0,
        );
        let _ = db2;
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = Arc::new(0u8); // silence unused-import noise in some toolchains
}
