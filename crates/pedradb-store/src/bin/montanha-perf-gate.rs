//! RFC-0021 P0.3 — perf gate v0: measured put/get/commit_tx on in-process 3-node majority.
//!
//! Usage:
//!   cargo run -p pedradb-store --bin montanha-perf-gate -- [out_dir]
//!
//! Writes `perf_report.json` under out_dir (default findings/perf-<utc>).

#![forbid(unsafe_code)]

use pedradb_store::StoreCluster;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            PathBuf::from(format!("findings/perf-{ts}"))
        });
    std::fs::create_dir_all(&out).expect("mkdir out");

    let n_put = std::env::var("MONTANHA_PERF_PUTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200usize);
    let n_get = std::env::var("MONTANHA_PERF_GETS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200usize);
    let n_tx = std::env::var("MONTANHA_PERF_TX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50usize);
    let payload: usize = std::env::var("MONTANHA_PERF_PAYLOAD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);

    let dir = out.join("db");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut c = StoreCluster::open(&dir, 3, 1).expect("open");
    c.elect_all(120).expect("elect");

    let val = vec![b'x'; payload];
    let mut put_lat = Vec::with_capacity(n_put);
    let t0 = Instant::now();
    for i in 0..n_put {
        let key = format!("pk-{i:06}").into_bytes();
        let t = Instant::now();
        c.put(&key, &val).expect("put");
        put_lat.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let put_wall = t0.elapsed();

    let mut get_lat = Vec::with_capacity(n_get);
    let t0 = Instant::now();
    for i in 0..n_get {
        let key = format!("pk-{:06}", i % n_put.max(1)).into_bytes();
        let t = Instant::now();
        let _ = c.get(&key).expect("get");
        get_lat.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let get_wall = t0.elapsed();

    let mut tx_lat = Vec::with_capacity(n_tx);
    let t0 = Instant::now();
    for i in 0..n_tx {
        let mut tx = c.tx_begin();
        let k1 = format!("tx-{i:04}-a").into_bytes();
        let k2 = format!("tx-{i:04}-b").into_bytes();
        tx.set(&k1, &val).expect("set");
        tx.set(&k2, &val).expect("set");
        let t = Instant::now();
        tx.commit(&mut c).expect("commit");
        tx_lat.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let tx_wall = t0.elapsed();

    put_lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    get_lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    tx_lat.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let put_qps = n_put as f64 / put_wall.as_secs_f64().max(1e-9);
    let get_qps = n_get as f64 / get_wall.as_secs_f64().max(1e-9);
    let tx_qps = n_tx as f64 / tx_wall.as_secs_f64().max(1e-9);

    let report = format!(
        r#"{{
  "gate": "rfc0021-p0.3-perf-v0",
  "nodes": 3,
  "ranges": 1,
  "payload_bytes": {payload},
  "put": {{
    "n": {n_put},
    "qps": {put_qps:.3},
    "p50_ms": {p50p:.4},
    "p99_ms": {p99p:.4},
    "wall_s": {pw:.4}
  }},
  "get": {{
    "n": {n_get},
    "qps": {get_qps:.3},
    "p50_ms": {p50g:.4},
    "p99_ms": {p99g:.4},
    "wall_s": {gw:.4}
  }},
  "commit_tx_pending": {{
    "n": {n_tx},
    "keys_per_tx": 2,
    "qps": {tx_qps:.3},
    "p50_ms": {p50t:.4},
    "p99_ms": {p99t:.4},
    "wall_s": {tw:.4}
  }},
  "note": "in-process 3-node majority; not field peer; not YCSB"
}}
"#,
        p50p = pct(&put_lat, 50.0),
        p99p = pct(&put_lat, 99.0),
        pw = put_wall.as_secs_f64(),
        p50g = pct(&get_lat, 50.0),
        p99g = pct(&get_lat, 99.0),
        gw = get_wall.as_secs_f64(),
        p50t = pct(&tx_lat, 50.0),
        p99t = pct(&tx_lat, 99.0),
        tw = tx_wall.as_secs_f64(),
    );

    let path = out.join("perf_report.json");
    std::fs::write(&path, &report).expect("write report");
    println!("{report}");
    println!("wrote {}", path.display());
    // Keep process short; drop cluster
    drop(c);
    let _ = Duration::from_millis(1);
}
