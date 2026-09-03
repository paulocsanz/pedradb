//! RFC-0042 P0.2 — lone-writer 1-op put split under YCSB-like conditions.
//!
//! Runs sequential (single-client) 1-op puts of a 1 KB payload on a real
//! directory, then prints the measured phase split from
//! [`ConcurrentDb::lone_path_split`]: `start` (write lock + prepare + WAL
//! encode), `apply` (memtable), `io` (WAL `write` + `fdatasync`),
//! `publish`. An isolated `write(1 KiB) + fdatasync` microbench on a raw
//! file grounds the fd floor on the same box/run.
//!
//! Usage: `cargo run --release -p pedradb-core --example lone_split [iters]`

use pedradb_core::{ConcurrentDb, OpenOptions};
use std::fs;
use std::io::Write;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() {
    let iters: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    let records: usize = 4096;
    let value = vec![b'v'; 1000];

    let dir = std::env::temp_dir().join(format!(
        "pedra-lone-split-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let db = ConcurrentDb::open_with(
        &dir,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
    )
    .expect("open");

    // Seed the keyspace first (append-only), like the YCSB bench.
    for i in 0..records {
        db.put(format!("user{i:08}").as_bytes(), &value).unwrap();
    }
    let (n0, s0) = db.lone_path_split();

    // Measured pass 1: sequential appends (unique keys, worst-case BTree).
    let t0 = Instant::now();
    for i in 0..iters {
        db.put(format!("user{:08}", records + i).as_bytes(), &value)
            .unwrap();
    }
    let append_wall = t0.elapsed();
    let (n1, s1) = db.lone_path_split();

    // Measured pass 2: hot overwrite of a small key window (overwrite shape).
    let hot = 512;
    let t1 = Instant::now();
    for i in 0..iters {
        db.put(format!("user{:08}", i % hot).as_bytes(), &value)
            .unwrap();
    }
    let overwrite_wall = t1.elapsed();
    let (n2, s2) = db.lone_path_split();
    let fd_ema = db.wal_fd_ema();
    drop(db);
    let _ = fs::remove_dir_all(&dir);

    let delta = |a: &[u64; 4], b: &[u64; 4]| {
        [
            a[0].saturating_sub(b[0]),
            a[1].saturating_sub(b[1]),
            a[2].saturating_sub(b[2]),
            a[3].saturating_sub(b[3]),
        ]
    };
    print_pass(
        "append (unique keys)",
        n1 - n0,
        &delta(&s1, &s0),
        append_wall,
        iters,
    );
    print_pass(
        "overwrite (512 hot keys)",
        n2 - n1,
        &delta(&s2, &s1),
        overwrite_wall,
        iters,
    );
    println!(
        "wal fd ema after run: {ema:.1} µs",
        ema = fd_ema.as_secs_f64() * 1e6
    );

    // Isolated fd floor: 1 KiB write + fdatasync on a raw file, same box.
    let raw_dir = std::env::temp_dir().join(format!(
        "pedra-lone-raw-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&raw_dir).unwrap();
    let path = raw_dir.join("wal.bin");
    let mut f = fs::File::create(&path).unwrap();
    let mut samples = Vec::with_capacity(2000);
    let buf = vec![b'w'; 1000];
    for _ in 0..2000 {
        let t = Instant::now();
        f.write_all(&buf).unwrap();
        pedradb_core::env::fdatasync_file(&f).unwrap();
        samples.push(t.elapsed().as_nanos() as u64);
    }
    drop(f);
    let _ = fs::remove_dir_all(&raw_dir);
    samples.sort_unstable();
    let p = |q: usize| samples[(samples.len() * q / 100).min(samples.len() - 1)] as f64 / 1000.0;
    let mean = samples.iter().sum::<u64>() as f64 / samples.len() as f64 / 1000.0;
    println!(
        "raw write(1 KiB)+fdatasync: p50 {p50:.1} µs  p90 {p90:.1} µs  p99 {p99:.1} µs  mean {mean:.1} µs  (n={n})",
        p50 = p(50),
        p90 = p(90),
        p99 = p(99),
        n = samples.len()
    );
}

fn print_pass(label: &str, n: u64, split: &[u64; 4], wall: std::time::Duration, iters: usize) {
    let d = n.max(1) as f64;
    let start = split[0] as f64 / 1000.0 / d;
    let apply = split[1] as f64 / 1000.0 / d;
    let io = split[2] as f64 / 1000.0 / d;
    let publish = split[3] as f64 / 1000.0 / d;
    let total = start + apply + io + publish;
    let wall_us = wall.as_secs_f64() * 1e6 / iters as f64;
    println!("lone split — {label} (n={n}):");
    println!(
        "  start   (lock+prepare+encode) {start:8.2} µs  {p1:5.1}%",
        p1 = 100.0 * start / total
    );
    println!(
        "  apply   (memtable)            {apply:8.2} µs  {p2:5.1}%",
        p2 = 100.0 * apply / total
    );
    println!(
        "  io      (write+fdatasync)    {io:8.2} µs  {p3:5.1}%",
        p3 = 100.0 * io / total
    );
    println!(
        "  publish (visibility)         {publish:8.2} µs  {p4:5.1}%",
        p4 = 100.0 * publish / total
    );
    println!(
        "  total/op {total:8.2} µs  (wall {wall_us:.2} µs)  → ceiling {ceiling:.0} qps",
        ceiling = 1e6 / total
    );
}
