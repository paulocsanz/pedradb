//! RFC-0037 P2.2 group-commit sizing (lab-only): N client threads × OPS puts
//! through `ConcurrentDb` (sync WAL — fdatasync before each reply), then
//! report wall, qps, WAL fsync count (real group size = ops / fsyncs),
//! write-group diagnostics and per-op latency percentiles.
//!
//! Usage: cargo run --release -p rocksdb-parity-bench --example group_profile [clients] [ops] [dir]

use pedradb_core::concurrent::ConcurrentDb;
use std::time::Instant;

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    let clients: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let ops: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4000);
    let dir = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "/tmp/pedra-group-profile".into());
    let payload_len: usize = std::env::args()
        .nth(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let catchup_us: Option<u64> = std::env::args().nth(5).and_then(|s| s.parse().ok());
    let _ = std::fs::remove_dir_all(&dir);
    let db = ConcurrentDb::open(&dir).expect("open");
    if let Some(us) = catchup_us {
        db.set_write_group_catchup_window(std::time::Duration::from_micros(us));
    }
    let window_us = db.write_group_catchup_window().as_micros();
    let payload = vec![b'g'; payload_len];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(clients));
    let mut all_latencies: Vec<u64> = Vec::with_capacity(clients * ops);
    let t0 = Instant::now();
    std::thread::scope(|s| {
        let latencies = std::sync::Arc::new(std::sync::Mutex::new(&mut all_latencies));
        for c in 0..clients {
            let barrier = barrier.clone();
            let db = &db;
            let payload = &payload;
            let latencies = latencies.clone();
            s.spawn(move || {
                barrier.wait();
                let mut mine = Vec::with_capacity(ops);
                for i in 0..ops {
                    let k = format!("g/{c:02}/{i:08}");
                    let t = Instant::now();
                    assert!(db.put(k.as_bytes(), payload).is_ok());
                    mine.push(t.elapsed().as_nanos() as u64);
                }
                latencies.lock().unwrap().extend(mine);
            });
        }
    });
    let wall = t0.elapsed();
    let total = clients * ops;
    let syncs = db.wal_sync_count();
    let (submits, queued, groups, group_ops) = db.write_group_stats();
    all_latencies.sort_unstable();
    let avg = all_latencies.iter().sum::<u64>() / total.max(1) as u64;
    println!(
        "group_profile clients={clients} ops={total} payload={payload_len}B catchup={window_us}us wall={:.3}s qps={:.0} wal_syncs={syncs} group_size={:.2}",
        wall.as_secs_f64(),
        total as f64 / wall.as_secs_f64(),
        total as f64 / syncs.max(1) as f64
    );
    println!(
        "  diag submits={submits} queued_behind_leader={queued} ({:.0}%) groups={groups} ops_in_groups={group_ops} avg_group={:.2}",
        100.0 * queued as f64 / submits.max(1) as f64,
        group_ops as f64 / groups.max(1) as f64,
    );
    println!(
        "  lat_ns avg={avg} p50={} p90={} p99={} max={}",
        pct(&all_latencies, 0.50),
        pct(&all_latencies, 0.90),
        pct(&all_latencies, 0.99),
        all_latencies.last().copied().unwrap_or(0),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
