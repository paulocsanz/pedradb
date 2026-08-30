//! RFC-0045 P0.2 — multi-writer probe for the async bypass path.
//!
//! Mirrors the `kvrocks_set_mc50` shape (N barrier-aligned writer threads,
//! skewed keys over a seeded keyspace, 1 KiB values) but in-process on
//! `ConcurrentDb`, so `PEDRA_WRITE_PHASE_STATS=1` can attribute where the
//! writer's nanoseconds go: lock wait vs prepare vs WAL encode/append vs
//! BTree insert vs publish.
//!
//! Usage:
//! ```text
//! PEDRA_WRITE_PHASE_STATS=1 cargo run --release -p pedradb-core --example multiwriter_probe -- [threads_csv] [ops_per_thread] [records]
//! ```
//! Defaults: threads sweep 1,4,12,50; ops 100_000/thread; records 4096.

use pedradb_core::{ConcurrentDb, OpenOptions};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};
use std::time::Instant;

/// 1 GiB write buffer: keeps the measured window flush-free (the parity
/// bench uses 256 MiB for the same reason) so `flush_check` stays a pure
/// counter check and the probe attributes only lock/prepare/wal/mem/publish.
/// Default is 4 MiB — a flush every ~4k 1-KiB ops would swamp the phases.
const FLUSH_BYTES: usize = 1 << 30;

/// Tiny xorshift zipf-substitute: skew toward low indices (hottest first),
/// good enough for a lock-contention probe (documented in the finding).
struct Picker(u64);
impl Picker {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        // 80/20-ish skew: square the uniform sample.
        let u = (self.0 >> 32) as f64 / u64::from(u32::MAX) as f64;
        ((u * u) * n as f64) as usize % n
    }
}

fn run(threads: usize, ops: usize, records: usize) {
    let dir =
        std::env::temp_dir().join(format!("pedra-mwprobe-{}-{}", threads, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = ConcurrentDb::open_with(
        &dir,
        OpenOptions {
            auto_flush_bytes: Some(FLUSH_BYTES),
            ..OpenOptions::default()
        },
    )
    .expect("open");
    db.set_default_write_sync(false); // async bypass — the mc50 column shape

    let val = Arc::new(vec![b'k'; 1024]);
    let barrier = Arc::new(Barrier::new(threads));
    let mut all_lats = Vec::new();

    let t0 = Instant::now();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let db = &db;
                let val = &val;
                let barrier = &barrier;
                s.spawn(move || {
                    let mut rng = Picker(0x9E3779B97F4A7C15 ^ (t as u64 + 1));
                    let mut lats = Vec::with_capacity(ops);
                    barrier.wait();
                    for _ in 0..ops {
                        let i = rng.next(records);
                        let key = format!("k/{i:06}");
                        let t = Instant::now();
                        db.put(key.as_bytes(), val.as_slice()).expect("put");
                        lats.push(t.elapsed().as_nanos() as u64);
                    }
                    lats
                })
            })
            .collect();
        for h in handles {
            all_lats.extend(h.join().expect("writer"));
        }
    });
    let wall = t0.elapsed();
    let total = threads * ops;
    all_lats.sort_unstable();
    let p = |q: f64| all_lats[((q * all_lats.len() as f64) as usize).min(all_lats.len() - 1)];
    let qps = total as f64 / wall.as_secs_f64();
    println!(
        "threads={threads} ops={total} qps={qps:.0} p50={:.2}us p99={:.2}us",
        p(0.50) as f64 / 1000.0,
        p(0.99) as f64 / 1000.0
    );

    if let Some(st) = db.write_phase_stats() {
        let g = |f: &std::sync::atomic::AtomicU64| f.load(Ordering::Relaxed);
        let commits = g(&st.commits).max(1);
        let ns = |f: &std::sync::atomic::AtomicU64| g(f) as f64 / 1000.0 / commits as f64;
        println!(
            "  phases us/commit: lock_wait={:.2} prepare={:.2} wal={:.2} mem={:.2} publish={:.2} flush_check={:.3} (commits={commits})",
            ns(&st.lock_wait_ns),
            ns(&st.prepare_ns),
            ns(&st.wal_ns),
            ns(&st.mem_ns),
            ns(&st.publish_ns),
            ns(&st.flush_check_ns),
        );
    } else {
        println!("  (PEDRA_WRITE_PHASE_STATS unset — no phase attribution)");
    }

    db.close().expect("close");
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads: Vec<usize> = args
        .get(1)
        .map(|v| v.split(',').filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 4, 12, 50]);
    let ops: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let records: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(4096);
    println!("multiwriter_probe ops/thread={ops} records={records} payload=1024B skew=squared-uniform flush=1GiB");
    for t in threads {
        run(t, ops, records);
    }
}
