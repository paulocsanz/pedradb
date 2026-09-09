//! FDB-shaped microbenchmarks on Montanha — find substrate limits (not YCSB field peer).
//!
//! Usage:
//!   cargo run -p pedradb-store --release --bin montanha-fdb-bench -- [out_dir]
//!
//! Env knobs (all optional):
//!   MONTANHA_BENCH_N          ops per micro-bench (default 200)
//!   MONTANHA_BENCH_PAYLOAD    value bytes (default 64)
//!   MONTANHA_BENCH_TX_KEYS    keys per multi-key TX (default 8)
//!   MONTANHA_BENCH_RANGES     range count for multi-range suite (default 4)
//!   MONTANHA_BENCH_WARMUP     warmup ops discarded (default 20)
//!   MONTANHA_BENCH_THREADS    concurrent client threads for C/T suites (default 4)
//!   MONTANHA_BENCH_SUITE      comma list: core,threads,tcp,mini-bt,scale,all
//!     (scale includes S1 sequential + S2 multi-client put + S3 multi-client PutBatch)
//!   MONTANHA_WRITE_BACKPRESSURE=1  Pedra L0 pressure/stall defaults on open
//!
//! Writes `fdb_shaped_bench.json` + human summary. Compare to FDB using the same
//! workload shapes (see docs/montanha-vs-fdb-bench.md). Not a claim of field parity.

#![forbid(unsafe_code)]

use pedradb_store::{
    client_dcs_create, client_dcs_get, client_get, client_status, client_tick, FdbDatabase,
    StoreCluster, StoreOpenOptions, TcpClusterClient,
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn write_backpressure_enabled() -> bool {
    std::env::var("MONTANHA_WRITE_BACKPRESSURE").ok().as_deref() == Some("1")
}

fn store_opts() -> StoreOpenOptions {
    let mut opts = StoreOpenOptions::default();
    if write_backpressure_enabled() {
        opts = opts.with_write_backpressure();
    }
    opts
}

fn open_cluster(dir: &Path, n_nodes: u64, n_ranges: u64) -> StoreCluster {
    StoreCluster::open_with_options_lab_direct(dir, n_nodes, n_ranges, store_opts())
        .expect("open cluster")
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize(name: &str, n: usize, wall: Duration, lats_ms: &mut [f64]) -> String {
    lats_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let wall_s = wall.as_secs_f64().max(1e-12);
    let qps = n as f64 / wall_s;
    format!(
        r#"{{
    "name": "{name}",
    "n": {n},
    "qps": {qps:.3},
    "p50_ms": {p50:.4},
    "p95_ms": {p95:.4},
    "p99_ms": {p99:.4},
    "max_ms": {max:.4},
    "wall_s": {wall_s:.4}
  }}"#,
        p50 = pct(lats_ms, 50.0),
        p95 = pct(lats_ms, 95.0),
        p99 = pct(lats_ms, 99.0),
        max = lats_ms.last().copied().unwrap_or(0.0),
    )
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn xorshift_bench(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

fn suite_enabled(want: &str) -> bool {
    let s =
        std::env::var("MONTANHA_BENCH_SUITE").unwrap_or_else(|_| "core,threads,mini-bt,tcp".into());
    let s = s.to_lowercase();
    if s.split(',').any(|x| x.trim() == "all") {
        return true;
    }
    s.split(',').any(|x| x.trim() == want)
}

fn find_montanha_tcp() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_montanha-tcp") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let candidates = [
        PathBuf::from("target/release/montanha-tcp"),
        PathBuf::from("target/debug/montanha-tcp"),
    ];
    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    // Next to this binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("montanha-tcp");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

struct TcpNode {
    child: Child,
    addr: SocketAddr,
    id: u64,
}

impl Drop for TcpNode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_tcp_cluster(bin: &Path, tmp: &Path, n_ranges: u64) -> Vec<TcpNode> {
    let ports: Vec<u16> = (0..3).map(|_| free_port()).collect();
    let peers: Vec<(u64, SocketAddr)> = ports
        .iter()
        .enumerate()
        .map(|(i, &p)| ((i as u64) + 1, format!("127.0.0.1:{p}").parse().unwrap()))
        .collect();
    let peer_flags: Vec<String> = peers
        .iter()
        .flat_map(|(id, a)| vec!["--peer".into(), format!("{id}={a}")])
        .collect();
    let mut nodes = Vec::new();
    for (id, addr) in &peers {
        let data = tmp.join(format!("n{id}"));
        std::fs::create_dir_all(&data).unwrap();
        let child = Command::new(bin)
            .arg("node")
            .arg("--id")
            .arg(id.to_string())
            .arg("--data")
            .arg(&data)
            .arg("--bind")
            .arg(addr.to_string())
            .arg("--ranges")
            .arg(n_ranges.to_string())
            .args(&peer_flags)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn montanha-tcp");
        nodes.push(TcpNode {
            child,
            addr: *addr,
            id: *id,
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    for n in &nodes {
        loop {
            if client_status(n.addr.to_string()).is_ok() {
                break;
            }
            if Instant::now() > deadline {
                panic!("tcp node {} not up", n.id);
            }
            thread::sleep(Duration::from_millis(40));
        }
    }
    // elect-wait: every r*:leader set (multi-range-safe).
    let peer_flags: Vec<String> = nodes
        .iter()
        .flat_map(|n| vec!["--peer".into(), format!("{}={}", n.id, n.addr)])
        .collect();
    let st = Command::new(bin)
        .arg("elect-wait")
        .args(&peer_flags)
        .status()
        .expect("spawn elect-wait");
    if !st.success() {
        panic!("elect-wait failed for {n_ranges} ranges");
    }
    // Brief settle after leaders appear (HB + apply catch-up).
    for _ in 0..12 {
        for n in &nodes {
            let _ = client_tick(n.addr.to_string(), 2);
        }
        thread::sleep(Duration::from_millis(20));
    }
    nodes
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
            PathBuf::from(format!("findings/fdb-bench-{ts}"))
        });
    std::fs::create_dir_all(&out).expect("mkdir out");

    let n = env_usize("MONTANHA_BENCH_N", 200);
    let payload = env_usize("MONTANHA_BENCH_PAYLOAD", 64);
    let tx_keys = env_usize("MONTANHA_BENCH_TX_KEYS", 8).max(2);
    let n_ranges = env_usize("MONTANHA_BENCH_RANGES", 4).max(1) as u64;
    let warmup = env_usize("MONTANHA_BENCH_WARMUP", 20);
    let n_threads = env_usize("MONTANHA_BENCH_THREADS", 4).max(1);
    let val = vec![b'x'; payload];

    let mut benches = Vec::new();
    let mut notes = Vec::new();
    // Aggregate Pedra L0/mem admission after core suites (WriteAdmissionSnap).
    let mut admission_core_a: Option<String> = None;
    let mut admission_core_b: Option<String> = None;

    macro_rules! progress {
        ($($t:tt)*) => {{
            eprintln!("[fdb-bench] {}", format!($($t)*));
        }};
    }

    // ── Suite A: single-range majority (baseline substrate) ───────────────
    if suite_enabled("core") {
        let dir = out.join("db-r1");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        progress!("A open 3-node 1-range…");
        let mut c = open_cluster(&dir, 3, 1);
        c.elect_all(150).expect("elect r1");
        progress!("A elect ok; warmup {warmup}");

        // Warmup puts.
        for i in 0..warmup {
            let k = format!("warm-{i:06}").into_bytes();
            c.put(&k, &val).expect("warm");
        }
        progress!("A warmup done; n={n}");

        // A1 raw put
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for i in 0..n {
            let k = format!("rawp-{i:06}").into_bytes();
            let t = Instant::now();
            c.put(&k, &val).expect("put");
            lats.push(ms(t));
        }
        benches.push(summarize("A1_raw_put_1range", n, t0.elapsed(), &mut lats));
        progress!("A1 raw put done");

        // A1b: same-range put_batch of batch_sz keys (amortize Raft+WAL) — RFC-0025
        let batch_sz = 16usize.min(n.max(1));
        let batches = (n / batch_sz).max(1);
        let mut lats = Vec::with_capacity(batches);
        let t0 = Instant::now();
        let mut keys_written = 0usize;
        for b in 0..batches {
            let mut pairs = Vec::with_capacity(batch_sz);
            for j in 0..batch_sz {
                let k = format!("bat-{b:04}-{j:02}").into_bytes();
                pairs.push((k, val.clone()));
            }
            let t = Instant::now();
            c.put_batch(pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
                .expect("put_batch");
            lats.push(ms(t));
            keys_written += batch_sz;
        }
        let wall = t0.elapsed();
        let key_qps = keys_written as f64 / wall.as_secs_f64().max(1e-12);
        lats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        benches.push(format!(
            r#"{{
    "name": "A1b_put_batch_{batch_sz}",
    "batches": {batches},
    "keys": {keys_written},
    "keys_per_s": {key_qps:.3},
    "batch_p50_ms": {p50:.4},
    "batch_p99_ms": {p99:.4},
    "wall_s": {ws:.4},
    "note": "keys/s vs A1 qps shows amortization of one Raft entry per batch"
  }}"#,
            p50 = pct(&lats, 50.0),
            p99 = pct(&lats, 99.0),
            ws = wall.as_secs_f64(),
        ));
        progress!("A1b put_batch done keys_per_s={key_qps:.1}");

        // A1c: put_many (same as batch when one range)
        let many_n = n.min(64);
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..many_n)
            .map(|i| (format!("many-{i:04}").into_bytes(), val.clone()))
            .collect();
        let t0 = Instant::now();
        c.put_many(pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
            .expect("put_many");
        let wall = t0.elapsed();
        benches.push(format!(
            r#"{{
    "name": "A1c_put_many_{many_n}",
    "keys": {many_n},
    "keys_per_s": {kps:.3},
    "wall_s": {ws:.4}
  }}"#,
            kps = many_n as f64 / wall.as_secs_f64().max(1e-12),
            ws = wall.as_secs_f64(),
        ));
        progress!("A1c put_many done");
        let _ = pairs;

        // A1d: put_buffered + flush (P1.1 group-commit style API)
        let buf_n = n.min(64);
        let t0 = Instant::now();
        for i in 0..buf_n {
            c.put_buffered(format!("buf-{i:04}").into_bytes(), &val)
                .expect("buf");
        }
        c.flush_writes().expect("flush");
        let wall = t0.elapsed();
        benches.push(format!(
            r#"{{
    "name": "A1d_put_buffered_flush_{buf_n}",
    "keys": {buf_n},
    "keys_per_s": {kps:.3},
    "wall_s": {ws:.4}
  }}"#,
            kps = buf_n as f64 / wall.as_secs_f64().max(1e-12),
            ws = wall.as_secs_f64(),
        ));
        progress!("A1d buffered flush done");

        // A2 raw get
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for i in 0..n {
            let k = format!("rawp-{:06}", i % n.max(1)).into_bytes();
            let t = Instant::now();
            let _ = c.get(&k).expect("get");
            lats.push(ms(t));
        }
        benches.push(summarize("A2_raw_get_1range", n, t0.elapsed(), &mut lats));
        progress!("A2 raw get done");

        // A3 FDB face set+commit (single key)
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for i in 0..n {
            let k = format!("fdbp-{i:06}").into_bytes();
            let t = Instant::now();
            let mut db = FdbDatabase::open(&mut c);
            let mut tr = db.create_transaction();
            tr.set(&k, &val).expect("set");
            db.commit(tr).expect("commit face");
            lats.push(ms(t));
        }
        benches.push(summarize(
            "A3_fdb_face_set_commit_1k",
            n,
            t0.elapsed(),
            &mut lats,
        ));
        progress!("A3 fdb face done");

        // A4 snapshot TX multi-key (same range)
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for i in 0..n {
            let t = Instant::now();
            let mut tr = c.begin();
            for j in 0..tx_keys {
                let k = format!("txm-{i:05}-{j:02}").into_bytes();
                tr.set(&k, &val).expect("set");
            }
            tr.commit(&mut c).expect("tx multi");
            lats.push(ms(t));
        }
        benches.push(summarize(
            &format!("A4_snapshot_tx_{tx_keys}keys_1range"),
            n,
            t0.elapsed(),
            &mut lats,
        ));
        progress!("A4 multi-key tx done");

        // A5 get_range over pre-filled prefix
        for i in 0..n.min(100) {
            let k = format!("rng/{i:04}").into_bytes();
            c.put(&k, &val).expect("seed range");
        }
        let range_n = n.min(100);
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for _ in 0..n {
            let t = Instant::now();
            let mut tr = c.begin();
            let pairs = tr.get_range(&c, b"rng/", b"rng0").expect("range");
            assert!(pairs.len() >= range_n / 2, "range thin");
            lats.push(ms(t));
        }
        benches.push(summarize("A5_get_range_prefix", n, t0.elapsed(), &mut lats));
        progress!("A5 get_range done");

        // A6 clear_range cost: seed once outside the timer, measure clear+commit only.
        let clear_iters = n.min(20);
        for i in 0..clear_iters {
            for j in 0..4u32 {
                let k = format!("clr{i:03}/{j}").into_bytes();
                c.put(&k, &val).expect("seed clear");
            }
        }
        progress!("A6 seed done; timing clear_range×{clear_iters}");
        let mut lats = Vec::with_capacity(clear_iters);
        let t0 = Instant::now();
        for i in 0..clear_iters {
            let t = Instant::now();
            let mut tr = c.begin();
            let start = format!("clr{i:03}/").into_bytes();
            let end = format!("clr{i:03}0").into_bytes();
            tr.clear_range(&c, &start, &end).expect("clear_range");
            // clear_range of empty range after first pass still ok; re-seed one key so TX not empty
            if tr.is_empty() {
                let k = format!("clr{i:03}/0").into_bytes();
                // already cleared — stage a no-op set/clear pair so commit has work
                tr.set(&k, b"t").expect("touch");
                tr.clear(&k).expect("clear touch");
            }
            tr.commit(&mut c).expect("commit clear");
            lats.push(ms(t));
        }
        benches.push(summarize(
            "A6_clear_range_4keys",
            clear_iters,
            t0.elapsed(),
            &mut lats,
        ));
        progress!("A6 clear_range done");

        // A7 hot-key contention: two TXs on same key, measure abort rate + success latency
        let cont_n = n.min(40);
        let mut ok_lats = Vec::new();
        let mut aborts = 0u64;
        let mut ok = 0u64;
        let t0 = Instant::now();
        for i in 0..cont_n {
            let mut t1 = c.begin();
            let mut t2 = c.begin();
            let k = b"hot/key";
            let _ = t1.get(&c, k);
            let _ = t2.get(&c, k);
            t1.set(k, format!("a{i}").as_bytes()).unwrap();
            t2.set(k, format!("b{i}").as_bytes()).unwrap();
            let t = Instant::now();
            let r1 = t1.commit(&mut c);
            let r2 = t2.commit(&mut c);
            match (r1.is_ok(), r2.is_ok()) {
                (true, false) | (false, true) => {
                    ok += 1;
                    ok_lats.push(ms(t));
                    aborts += 1;
                }
                (true, true) => {
                    // Should not both succeed on same key WW — count as anomaly
                    ok += 2;
                    ok_lats.push(ms(t));
                    notes.push(format!("A7 anomaly: both commits ok on hot key iter {i}"));
                }
                (false, false) => {
                    aborts += 2;
                }
            }
        }
        let wall = t0.elapsed();
        ok_lats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        benches.push(format!(
            r#"{{
    "name": "A7_hot_key_ww_conflict",
    "pairs": {cont_n},
    "success_commits": {ok},
    "abort_commits": {aborts},
    "abort_ratio": {ar:.4},
    "success_p50_ms": {p50:.4},
    "success_p99_ms": {p99:.4},
    "wall_s": {ws:.4},
    "note": "exactly one of two concurrent WW should win per pair"
  }}"#,
            ar = aborts as f64 / ((cont_n * 2) as f64).max(1.0),
            p50 = pct(&ok_lats, 50.0),
            p99 = pct(&ok_lats, 99.0),
            ws = wall.as_secs_f64(),
        ));
        progress!("A7 hot-key done ok={ok} aborts={aborts}");

        // A8 record-shaped: row + secondary index in one TX
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for i in 0..n {
            let pk = format!("pk{i:06}").into_bytes();
            let email = format!("u{i}@x").into_bytes();
            let t = Instant::now();
            let mut tr = c.begin();
            let mut rk = b"rec/r/".to_vec();
            rk.extend_from_slice(&pk);
            let mut ik = b"rec/i/email/".to_vec();
            ik.extend_from_slice(&email);
            ik.push(0);
            ik.extend_from_slice(&pk);
            tr.set(&rk, &email).unwrap();
            tr.set(&ik, b"\x01").unwrap();
            tr.commit(&mut c).unwrap();
            lats.push(ms(t));
        }
        benches.push(summarize(
            "A8_record_row_plus_index_tx",
            n,
            t0.elapsed(),
            &mut lats,
        ));
        progress!("A8 record index done");

        admission_core_a = Some(c.write_admission_snap().to_json_object());
        drop(c);
    }

    // ── Suite B: multi-range (N writers potential / cross-range 2PC cost) ─
    if suite_enabled("core") {
        let dir = out.join(format!("db-r{n_ranges}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        progress!("B open 3-node {n_ranges}-range…");
        let mut c = open_cluster(&dir, 3, n_ranges);
        c.elect_all(200).expect("elect multi");
        progress!("B elect ok");

        // One key per range for disjoint puts (leading byte = range start).
        let metas = c.range_metas();
        let mut range_keys: Vec<Vec<u8>> = metas
            .iter()
            .map(|r| {
                if r.start.is_empty() {
                    vec![0x00, b'k']
                } else {
                    let mut k = r.start.clone();
                    k.push(b'k');
                    k
                }
            })
            .collect();
        if range_keys.is_empty() {
            range_keys.push(b"k".to_vec());
        }

        // B1 disjoint single-key puts (should fan across leaders)
        let mut lats = Vec::with_capacity(n);
        let t0 = Instant::now();
        for i in 0..n {
            let base = &range_keys[i % range_keys.len()];
            let mut k = base.clone();
            k.extend_from_slice(format!("-{i:06}").as_bytes());
            let t = Instant::now();
            c.put(&k, &val).expect("put multi");
            lats.push(ms(t));
        }
        benches.push(summarize(
            &format!("B1_disjoint_put_{n_ranges}ranges"),
            n,
            t0.elapsed(),
            &mut lats,
        ));
        progress!("B1 disjoint put done");

        // B2 cross-range TX: one key in each of first min(4, ranges).
        // Cap low: 2PC+log is the cliff — enough samples for p50/p99, not soak.
        let take = range_keys.len().clamp(2, 4);
        let iters = n.min(12);
        let mut lats = Vec::with_capacity(iters);
        let mut fails = 0u64;
        let t0 = Instant::now();
        for i in 0..iters {
            let t = Instant::now();
            let mut tr = c.begin();
            for (j, base) in range_keys.iter().take(take).enumerate() {
                let mut k = base.clone();
                k.extend_from_slice(format!("-x{i:04}-{j}").as_bytes());
                tr.set(&k, &val).unwrap();
            }
            match tr.commit(&mut c) {
                Ok(_) => lats.push(ms(t)),
                Err(_) => {
                    fails += 1;
                    lats.push(ms(t));
                }
            }
            if (i + 1) % 3 == 0 {
                progress!("B2 progress {}/{iters}", i + 1);
            }
        }
        let wall = t0.elapsed();
        lats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        benches.push(format!(
            r#"{{
    "name": "B2_cross_range_tx_{take}keys",
    "n": {iters},
    "fails": {fails},
    "qps": {qps:.3},
    "p50_ms": {p50:.4},
    "p99_ms": {p99:.4},
    "wall_s": {ws:.4},
    "note": "2PC multi-range cliff; expected >> A4 same-range; small n by design"
  }}"#,
            qps = iters as f64 / wall.as_secs_f64().max(1e-12),
            p50 = pct(&lats, 50.0),
            p99 = pct(&lats, 99.0),
            ws = wall.as_secs_f64(),
        ));
        progress!("B2 cross-range tx done fails={fails}");

        // B4: strong vs fast replica read (RFC-0025 P2.3)
        let k = range_keys[0].clone();
        let mut kk = k.clone();
        kk.extend_from_slice(b"-read");
        c.put(&kk, b"rv").expect("seed read");
        let mut lats_s = Vec::with_capacity(n.min(40));
        let t0 = Instant::now();
        for _ in 0..n.min(40) {
            let t = Instant::now();
            let _ = c.get_strong(&kk).expect("strong");
            lats_s.push(ms(t));
        }
        benches.push(summarize(
            "B4_get_strong",
            n.min(40),
            t0.elapsed(),
            &mut lats_s,
        ));
        let mut lats_f = Vec::with_capacity(n.min(40));
        let t0 = Instant::now();
        for _ in 0..n.min(40) {
            let t = Instant::now();
            let _ = c.get_fast_replica(&kk).expect("fast");
            lats_f.push(ms(t));
        }
        benches.push(summarize(
            "B4_get_fast_replica",
            n.min(40),
            t0.elapsed(),
            &mut lats_f,
        ));
        progress!("B4 strong vs fast replica done");

        admission_core_b = Some(c.write_admission_snap().to_json_object());
        drop(c);
    }

    // ── Suite ycsb: FDB benchmark tool shapes (ycsb_a..ycsb_f) ────────────
    if suite_enabled("ycsb") {
        let records = env_usize("MONTANHA_YCSB_RECORDS", 1024).max(64);
        let ycsb_ops = env_usize("MONTANHA_YCSB_OPS", n).max(32);
        let ycsb_payload = env_usize("MONTANHA_YCSB_PAYLOAD", 100);
        let zipfian = std::env::var("MONTANHA_YCSB_DIST")
            .map(|s| s.eq_ignore_ascii_case("zipfian"))
            .unwrap_or(false);
        let dir = out.join("db-ycsb");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        progress!(
            "YCSB open 3-node 1-range records={records} ops={ycsb_ops} dist={}",
            if zipfian { "zipfian" } else { "uniform" }
        );
        let mut c = open_cluster(&dir, 3, 1);
        c.elect_all(200).expect("elect ycsb");

        // Deterministic rng (same schedule every run; FDB side uses its own).
        let mut rng = 0x5EED_0001_u64;
        // Zipfian(theta=0.99) CDF over [0, records): sample via binary search.
        let theta = 0.99_f64;
        let zipf_cdf: Vec<f64> = {
            let mut s = 0.0_f64;
            let cdf = (0..records)
                .map(|i| {
                    s += 1.0 / (i as f64 + 1.0).powf(theta);
                    s
                })
                .collect::<Vec<_>>();
            let total = cdf.last().copied().unwrap_or(1.0);
            cdf.into_iter().map(|v| v / total).collect()
        };
        let ykey = |i: usize| format!("ycsb/{i:06}").into_bytes();
        let pick = |rng: &mut u64, latest: usize| -> usize {
            if !zipfian || latest == 0 {
                return (xorshift_bench(rng) % records as u64) as usize;
            }
            // zipf over recency window (D/E use "latest" style access)
            let window = latest.min(records);
            let u = (xorshift_bench(rng) >> 11) as f64 / (1u64 << 53) as f64;
            let target = u * zipf_cdf[window - 1];
            let idx = zipf_cdf[..window].partition_point(|&c| c < target);
            (records - window) + idx.min(window - 1)
        };

        let yval = vec![b'y'; ycsb_payload];
        // Seed the keyspace in batches (put_batch amortizes Raft+WAL).
        let t0 = Instant::now();
        let mut seeded = 0usize;
        while seeded < records {
            let take = (records - seeded).min(64);
            let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..take)
                .map(|j| (ykey(seeded + j), yval.clone()))
                .collect();
            c.put_batch(pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
                .expect("seed batch");
            seeded += take;
        }
        progress!("YCSB seed {records} in {:.1}s", t0.elapsed().as_secs_f64());

        // Per-workload runner: returns (latencies, updates, inserts, scans, errors).
        let mut run_workload =
            |name: &str,
             read_pct: u64,
             insert_pct: u64,
             rmw: bool,
             scans: bool,
             c: &mut pedradb_store::StoreCluster| {
                let mut lats = Vec::with_capacity(ycsb_ops);
                let mut updates = 0u64;
                let mut inserts = 0u64;
                let mut scan_ops = 0u64;
                let mut errors = 0u64;
                let mut latest = records;
                let t0 = Instant::now();
                for _ in 0..ycsb_ops {
                    let t = Instant::now();
                    let roll = xorshift_bench(&mut rng) % 100;
                    if roll < read_pct {
                        // read
                        let i = pick(&mut rng, latest);
                        if c.get(&ykey(i)).is_err() {
                            errors += 1;
                        }
                    } else if roll < read_pct + insert_pct {
                        // insert (new key) → read-latest window grows
                        let k = format!("ycsb/{latest:06}").into_bytes();
                        if c.put(&k, &yval).is_ok() {
                            latest += 1;
                            inserts += 1;
                        } else {
                            errors += 1;
                        }
                    } else if scans {
                        // short range scan: [key(i), key(i+25)) window
                        let i = pick(&mut rng, latest);
                        let start = ykey(i);
                        let mut end = ykey(i + 25);
                        end.pop();
                        end.push(b'~');
                        let mut tr = c.begin();
                        match tr.get_range(c, start.as_slice(), end.as_slice()) {
                            Ok(_) => scan_ops += 1,
                            Err(_) => errors += 1,
                        }
                    } else if rmw {
                        // read-modify-write in one TX
                        let i = pick(&mut rng, latest);
                        let k = ykey(i);
                        let mut tr = c.begin();
                        let got = tr.get(c, k.as_slice()).ok().flatten();
                        let mut nv = yval.clone();
                        if let Some(old) = &got {
                            let last = nv.last_mut().unwrap();
                            *last = old.last().copied().unwrap_or(b'x').wrapping_add(1);
                        }
                        tr.set(k.as_slice(), nv.as_slice()).expect("set rmw");
                        match tr.commit(c) {
                            Ok(_) => updates += 1,
                            Err(_) => errors += 1,
                        }
                    } else {
                        // update
                        let i = pick(&mut rng, latest);
                        if c.put(&ykey(i), &yval).is_ok() {
                            updates += 1;
                        } else {
                            errors += 1;
                        }
                    }
                    lats.push(ms(t));
                }
                let wall = t0.elapsed();
                benches.push(summarize(name, ycsb_ops, wall, &mut lats));
                progress!(
                "{name} done ops={ycsb_ops} updates={updates} inserts={inserts} scans={scan_ops} errors={errors}"
            );
                (updates, inserts, scan_ops, errors)
            };

        // FDB benchmark shapes: ycsb_a 50/50, b 95/5, c 100r, d 95r/5i(latest),
        // e 95 scan/5i (zipfian), f 50 rmw.
        run_workload("ycsb_a", 50, 0, false, false, &mut c);
        run_workload("ycsb_b", 95, 0, false, false, &mut c);
        run_workload("ycsb_c", 100, 0, false, false, &mut c);
        run_workload("ycsb_d", 95, 5, false, false, &mut c);
        run_workload("ycsb_e", 0, 5, false, true, &mut c);
        run_workload("ycsb_f", 50, 0, true, false, &mut c);

        notes.push(format!(
            "ycsb records={records} ops={ycsb_ops} payload={ycsb_payload} dist={}",
            if zipfian { "zipfian" } else { "uniform" }
        ));
        drop(c);
    }

    // ── Suite scale: disjoint put QPS vs range count (RFC-0025 P2.2) ──────
    if suite_enabled("scale") {
        progress!("S scale multi-range put probe…");
        let scale_n = n.clamp(8, 24);
        for &nr in &[1u64, 2, 4, 8] {
            let dir = out.join(format!("db-scale-r{nr}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mut c = open_cluster(&dir, 3, nr);
            c.elect_all(200).expect("elect scale");
            let metas = c.range_metas();
            let bases: Vec<Vec<u8>> = metas
                .iter()
                .map(|r| {
                    if r.start.is_empty() {
                        vec![0x00, b's']
                    } else {
                        let mut k = r.start.clone();
                        k.push(b's');
                        k
                    }
                })
                .collect();
            let t0 = Instant::now();
            for i in 0..scale_n {
                let base = &bases[i % bases.len().max(1)];
                let mut k = base.clone();
                k.extend_from_slice(format!("-{i:04}").as_bytes());
                c.put(&k, &val).expect("scale put");
            }
            let wall = t0.elapsed();
            let kps = scale_n as f64 / wall.as_secs_f64().max(1e-12);
            benches.push(format!(
                r#"{{
    "name": "S1_disjoint_put_r{nr}",
    "ranges": {nr},
    "keys": {scale_n},
    "keys_per_s": {kps:.3},
    "wall_s": {ws:.4},
    "note": "RFC-0025 P2.2 / 0021 scale option A"
  }}"#,
                ws = wall.as_secs_f64(),
            ));
            progress!("S1 ranges={nr} keys_per_s={kps:.2}");
            drop(c);
        }

        // S2: multi-client multi-range (TCP) — true option-A scale probe.
        // thr ≥ min(nr,8) so r8 is not under-threaded; map thr→preferred-node range.
        if let Some(bin) = find_montanha_tcp() {
            progress!("S2 multi-client multi-range TCP…");
            let range_for = |tid: usize, nr: u64| -> u64 {
                if nr <= 1 {
                    return 1;
                }
                let n_nodes = 3u64.min(nr);
                let pref = ((tid as u64) % n_nodes) + 1;
                let cycle = (tid as u64) / n_nodes;
                let rid = pref + cycle * n_nodes;
                if rid <= nr {
                    rid
                } else {
                    ((tid as u64) % nr) + 1
                }
            };
            for &nr in &[1u64, 4, 8] {
                let tmp = out.join(format!("tcp-scale-r{nr}"));
                let _ = std::fs::remove_dir_all(&tmp);
                std::fs::create_dir_all(&tmp).unwrap();
                let nodes = start_tcp_cluster(&bin, &tmp, nr);
                // Cover ranges: at least one writer per range up to 8.
                let thr = n_threads.max(1).max((nr as usize).min(8));
                let per = (n.min(32) / thr.max(1)).max(2);
                let peers: Vec<(u64, String)> =
                    nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
                let step = (256u64 / nr.max(1)) as u8;
                let peers_a = Arc::new(peers);
                let val_a = Arc::new(val.clone());
                // Brief settle so rebalance_local can shed colocated leaders.
                thread::sleep(Duration::from_millis(500));
                let t0 = Instant::now();
                let mut handles = Vec::new();
                for tid in 0..thr {
                    let peers = (*peers_a).clone();
                    let v = Arc::clone(&val_a);
                    let range_id = range_for(tid, nr);
                    let range_i = range_id - 1;
                    let start_b = if range_i == 0 {
                        0u8
                    } else {
                        (range_i as u8).saturating_mul(step)
                    };
                    handles.push(thread::spawn(move || {
                        let mut cli = TcpClusterClient::new(peers)
                            .with_max_attempts(8)
                            .with_active_range(range_id);
                        cli.warm_leaders();
                        let deadline = Instant::now() + Duration::from_secs(45);
                        let mut ok = 0u64;
                        for i in 0..per {
                            if Instant::now() >= deadline {
                                break;
                            }
                            let mut k = vec![start_b, b't'];
                            k.extend_from_slice(format!("-{tid:02}-{i:04}").as_bytes());
                            for _ in 0..6 {
                                if Instant::now() >= deadline {
                                    break;
                                }
                                if cli.put(&k, &v).is_ok() {
                                    ok += 1;
                                    break;
                                }
                                thread::sleep(Duration::from_millis(10));
                            }
                        }
                        ok
                    }));
                }
                let mut total_ok = 0u64;
                for h in handles {
                    total_ok += h.join().unwrap();
                }
                let wall = t0.elapsed();
                let kps = total_ok as f64 / wall.as_secs_f64().max(1e-12);
                benches.push(format!(
                    r#"{{
    "name": "S2_tcp_mt_put_r{nr}_t{thr}",
    "ranges": {nr},
    "threads": {thr},
    "keys_ok": {total_ok},
    "keys_per_s": {kps:.3},
    "wall_s": {ws:.4},
    "note": "thr≥min(nr,8); preferred-node map; rebalance settle; 45s wall"
  }}"#,
                    ws = wall.as_secs_f64(),
                ));
                progress!("S2 ranges={nr} thr={thr} ok={total_ok} keys_per_s={kps:.2}");
                drop(nodes);
            }

            // S3 PutBatch: r1+r4+r8 with thr≥min(nr,8) and 60s walls.
            progress!("S3 multi-client multi-range TCP PutBatch…");
            let batch_sz = 8usize;
            for &nr in &[1u64, 4, 8] {
                let tmp = out.join(format!("tcp-scale-batch-r{nr}"));
                let _ = std::fs::remove_dir_all(&tmp);
                std::fs::create_dir_all(&tmp).unwrap();
                let nodes = start_tcp_cluster(&bin, &tmp, nr);
                let thr = n_threads.max(1).max((nr as usize).min(8));
                let batches_per = (n.min(32) / thr.max(1)).max(2);
                let peers: Vec<(u64, String)> =
                    nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
                let step = (256u64 / nr.max(1)) as u8;
                let peers_a = Arc::new(peers);
                let val_a = Arc::new(val.clone());
                thread::sleep(Duration::from_millis(500));
                let t0 = Instant::now();
                let mut handles = Vec::new();
                for tid in 0..thr {
                    let peers = (*peers_a).clone();
                    let v = Arc::clone(&val_a);
                    let range_id = range_for(tid, nr);
                    let range_i = range_id - 1;
                    let start_b = if range_i == 0 {
                        0u8
                    } else {
                        (range_i as u8).saturating_mul(step)
                    };
                    handles.push(thread::spawn(move || {
                        let mut cli = TcpClusterClient::new(peers)
                            .with_max_attempts(8)
                            .with_active_range(range_id);
                        cli.warm_leaders();
                        let deadline = Instant::now() + Duration::from_secs(60);
                        let mut ok_keys = 0u64;
                        let mut ok_batches = 0u64;
                        for b in 0..batches_per {
                            if Instant::now() >= deadline {
                                break;
                            }
                            let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..batch_sz)
                                .map(|i| {
                                    let mut k = vec![start_b, b't', b'b'];
                                    k.extend_from_slice(
                                        format!("-{tid:02}-{b:03}-{i:02}").as_bytes(),
                                    );
                                    (k, v.as_slice().to_vec())
                                })
                                .collect();
                            for _ in 0..6 {
                                if Instant::now() >= deadline {
                                    break;
                                }
                                if cli.put_batch(&pairs).is_ok() {
                                    ok_keys += batch_sz as u64;
                                    ok_batches += 1;
                                    break;
                                }
                                thread::sleep(Duration::from_millis(12));
                            }
                        }
                        (ok_keys, ok_batches)
                    }));
                }
                let mut total_ok = 0u64;
                let mut total_batches = 0u64;
                for h in handles {
                    let (k, b) = h.join().unwrap();
                    total_ok += k;
                    total_batches += b;
                }
                let wall = t0.elapsed();
                let kps = total_ok as f64 / wall.as_secs_f64().max(1e-12);
                let bps = total_batches as f64 / wall.as_secs_f64().max(1e-12);
                benches.push(format!(
                    r#"{{
    "name": "S3_tcp_mt_put_batch_r{nr}_t{thr}_b{batch_sz}",
    "ranges": {nr},
    "threads": {thr},
    "batch_sz": {batch_sz},
    "keys_ok": {total_ok},
    "batches_ok": {total_batches},
    "keys_per_s": {kps:.3},
    "batches_per_s": {bps:.3},
    "wall_s": {ws:.4},
    "note": "PutBatch thr≥min(nr,8); rebalance settle; 60s wall"
  }}"#,
                    ws = wall.as_secs_f64(),
                ));
                progress!(
                    "S3 ranges={nr} thr={thr} batch={batch_sz} ok_keys={total_ok} keys_per_s={kps:.2} batches_per_s={bps:.2}"
                );
                drop(nodes);
            }
        } else {
            notes.push("S2/S3 skipped: montanha-tcp not found".into());
            progress!("S2/S3 skip: no montanha-tcp");
        }
    }

    // ── Suite C: multi-thread needs Send StoreCluster — use TCP suite D4 ─
    // StoreCluster holds SeedRng (Rc) and is !Send; concurrent clients = suite tcp.
    if suite_enabled("threads") && !suite_enabled("tcp") {
        notes.push(
            "threads suite: in-process StoreCluster is !Send; enabling TCP multi-thread path"
                .into(),
        );
    }

    // ── Suite D: real TCP cluster (network path + multi-thread clients) ─
    // Also runs when suite includes "threads" (honest concurrent clients).
    if suite_enabled("tcp") || suite_enabled("threads") {
        match find_montanha_tcp() {
            None => {
                notes.push("tcp suite skipped: montanha-tcp binary not found".into());
                progress!("D skip: no montanha-tcp");
            }
            Some(bin) => {
                progress!("D TCP suite with {}", bin.display());
                let tmp = out.join("tcp-cluster");
                let _ = std::fs::remove_dir_all(&tmp);
                std::fs::create_dir_all(&tmp).unwrap();
                let nodes = start_tcp_cluster(&bin, &tmp, 1);
                let addrs: Vec<String> = nodes.iter().map(|n| n.addr.to_string()).collect();
                let peers: Vec<(u64, String)> =
                    nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();

                let mut client = TcpClusterClient::new(peers.clone()).with_max_attempts(48);
                let mut lats = Vec::with_capacity(n.min(40));
                let d1n = n.min(40);
                let t0 = Instant::now();
                let mut ok = 0usize;
                for i in 0..d1n {
                    let k = format!("tcp-p-{i:05}").into_bytes();
                    let t = Instant::now();
                    if client.put(&k, &val).is_ok() {
                        ok += 1;
                        lats.push(ms(t));
                    }
                }
                let wall = t0.elapsed();
                if lats.is_empty() {
                    notes.push("D1 tcp put: zero successes".into());
                } else {
                    benches.push(summarize("D1_tcp_put", ok, wall, &mut lats));
                }
                progress!("D1 tcp put ok={ok}/{d1n}");

                let mut lats = Vec::with_capacity(ok);
                let t0 = Instant::now();
                let mut gok = 0usize;
                for i in 0..ok {
                    let k = format!("tcp-p-{i:05}").into_bytes();
                    let t = Instant::now();
                    for a in &addrs {
                        if client_get(a, &k).ok().flatten().is_some() {
                            gok += 1;
                            lats.push(ms(t));
                            break;
                        }
                    }
                }
                if !lats.is_empty() {
                    benches.push(summarize("D2_tcp_get", gok, t0.elapsed(), &mut lats));
                }
                progress!("D2 tcp get ok={gok}");

                let d3n = n.min(15);
                let mut lats = Vec::with_capacity(d3n);
                let t0 = Instant::now();
                let mut cok = 0usize;
                for i in 0..d3n {
                    let pairs = vec![
                        (format!("tcp-tx-{i:04}-a").into_bytes(), val.clone()),
                        (format!("tcp-tx-{i:04}-b").into_bytes(), val.clone()),
                    ];
                    let t = Instant::now();
                    // Retries: CommitTx under TCP can NotLeader / busy.
                    for _ in 0..12 {
                        if client.commit_tx(&pairs).is_ok() {
                            cok += 1;
                            lats.push(ms(t));
                            break;
                        }
                        thread::sleep(Duration::from_millis(30));
                    }
                }
                if !lats.is_empty() {
                    benches.push(summarize(
                        "D3_tcp_commit_tx_2k",
                        cok,
                        t0.elapsed(),
                        &mut lats,
                    ));
                } else {
                    notes.push(format!("D3 tcp commit_tx: zero successes ({d3n} tries)"));
                }
                progress!("D3 tcp commit_tx ok={cok}/{d3n}");

                let per = (n.min(40) / n_threads).max(3);
                let peers_a = Arc::new(peers.clone());
                let val_a = Arc::new(val.clone());
                let t0 = Instant::now();
                let mut handles = Vec::new();
                for tid in 0..n_threads {
                    let peers = (*peers_a).clone();
                    let v = Arc::clone(&val_a);
                    handles.push(thread::spawn(move || {
                        // One TcpClusterClient per thread (NotLeader retry).
                        let mut cli = TcpClusterClient::new(peers).with_max_attempts(64);
                        let mut lats = Vec::new();
                        let mut ok = 0u64;
                        for i in 0..per {
                            let k = format!("tcp-mt-{tid:02}-{i:04}").into_bytes();
                            let t = Instant::now();
                            // brief backoff storms under multi-thread dial
                            for attempt in 0..8 {
                                if cli.put(&k, &v).is_ok() {
                                    ok += 1;
                                    lats.push(ms(t));
                                    break;
                                }
                                thread::sleep(Duration::from_millis(10 + attempt * 5));
                            }
                        }
                        (ok, lats)
                    }));
                }
                let mut all = Vec::new();
                let mut total_ok = 0u64;
                for h in handles {
                    let (ok, l) = h.join().unwrap();
                    total_ok += ok;
                    all.extend(l);
                }
                if !all.is_empty() {
                    benches.push(summarize(
                        &format!("D4_tcp_mt_put_{n_threads}thr"),
                        total_ok as usize,
                        t0.elapsed(),
                        &mut all,
                    ));
                } else {
                    notes.push(format!(
                        "D4 tcp multi-thread put: zero successes (threads={n_threads})"
                    ));
                }
                progress!("D4 tcp multi-thread put ok={total_ok}");

                // Extra ticks so DCS range has a stable leader after put storm.
                for _ in 0..40 {
                    for a in &addrs {
                        let _ = client_tick(a, 2);
                    }
                    thread::sleep(Duration::from_millis(15));
                }
                let t0 = Instant::now();
                let mut rev = 0u64;
                let key = b"m/bench/lock";
                let deadline = Instant::now() + Duration::from_secs(12);
                while Instant::now() < deadline && rev == 0 {
                    for a in &addrs {
                        if let Ok(r) = client_dcs_create(a, key, b"holder") {
                            rev = r;
                            break;
                        }
                    }
                    if rev == 0 {
                        thread::sleep(Duration::from_millis(50));
                    }
                }
                let create_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let mut exclusive_fail = false;
                if rev > 0 {
                    for a in &addrs {
                        if client_dcs_create(a, key, b"other").is_err() {
                            exclusive_fail = true;
                            break;
                        }
                    }
                }
                let mut seen = 0u32;
                let deadline = Instant::now() + Duration::from_secs(8);
                while Instant::now() < deadline && seen < 2 {
                    seen = 0;
                    for a in &addrs {
                        if client_dcs_get(a, key).ok().flatten().as_deref()
                            == Some(b"holder".as_ref())
                        {
                            seen += 1;
                        }
                    }
                    if seen < 2 {
                        thread::sleep(Duration::from_millis(40));
                    }
                }
                benches.push(format!(
                    r#"{{
    "name": "D5_tcp_dcs_create_get",
    "rev": {rev},
    "exclusive_fail": {exclusive_fail},
    "majority_seen": {seen},
    "create_ms": {create_ms:.4},
    "note": "etcd-need over real TCP"
  }}"#
                ));
                progress!("D5 tcp dcs rev={rev} exclusive_fail={exclusive_fail} seen={seen}");

                // D6: PutBatch one RTT multi-key (RFC-0025 P1.3)
                let batch_sz = 16usize.min(n.max(1));
                let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..batch_sz)
                    .map(|i| (format!("tpb-{i:03}").into_bytes(), val.clone()))
                    .collect();
                let t0 = Instant::now();
                let mut ok = false;
                for _ in 0..20 {
                    if client.put_batch(&pairs).is_ok() {
                        ok = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(30));
                }
                let wall = t0.elapsed();
                if ok {
                    benches.push(format!(
                        r#"{{
    "name": "D6_tcp_put_batch_{batch_sz}",
    "keys": {batch_sz},
    "keys_per_s": {kps:.3},
    "wall_s": {ws:.4},
    "note": "one RTT PutBatch vs N× D1 put"
  }}"#,
                        kps = batch_sz as f64 / wall.as_secs_f64().max(1e-12),
                        ws = wall.as_secs_f64(),
                    ));
                    progress!(
                        "D6 tcp put_batch ok keys_per_s={:.1}",
                        batch_sz as f64 / wall.as_secs_f64().max(1e-12)
                    );
                } else {
                    notes.push("D6 tcp put_batch failed".into());
                    progress!("D6 tcp put_batch FAILED");
                }
                drop(nodes);
            }
        }
    }

    // ── Suite E: mini-bindingtester random soak ──────────────────────────
    if suite_enabled("mini-bt") {
        progress!("E1 mini-bindingtester soak…");
        let dir = out.join("db-minib");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = open_cluster(&dir, 3, 1);
        c.elect_all(100).expect("elect mini");
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let ops = n.max(50) * 3;
        let mut mismatches = 0u64;
        let mut commits_ok = 0u64;
        let mut commits_err = 0u64;
        let mut multi_ok = 0u64;
        let mut ww_ok = 0u64;
        let mut rng = 0x000F_DBB1_u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let t0 = Instant::now();
        for i in 0..ops {
            let op = next() % 7;
            let k = format!("mb/{}", next() % 32).into_bytes();
            match op {
                0 | 1 => {
                    let v = format!("v{}", next() % 1000).into_bytes();
                    let mut tr = c.begin();
                    tr.set(&k, &v).unwrap();
                    match tr.commit(&mut c) {
                        Ok(_) => {
                            model.insert(k, v);
                            commits_ok += 1;
                        }
                        Err(_) => commits_err += 1,
                    }
                }
                2 => {
                    let mut tr = c.begin();
                    tr.clear(&k).unwrap();
                    match tr.commit(&mut c) {
                        Ok(_) => {
                            model.remove(&k);
                            commits_ok += 1;
                        }
                        Err(_) => commits_err += 1,
                    }
                }
                3 => {
                    let mut tr = c.begin();
                    let got = tr.get(&c, &k).unwrap();
                    let exp = model.get(&k).cloned();
                    if got != exp {
                        mismatches += 1;
                        if mismatches < 5 {
                            notes.push(format!(
                                "mini-bt mismatch i={i} key={}",
                                String::from_utf8_lossy(&k)
                            ));
                        }
                    }
                }
                4 => {
                    // multi-key TX (2 keys)
                    let k2 = format!("mb/{}", next() % 32).into_bytes();
                    let v1 = format!("m{}", next() % 1000).into_bytes();
                    let v2 = format!("m{}", next() % 1000).into_bytes();
                    let mut tr = c.begin();
                    tr.set(&k, &v1).unwrap();
                    tr.set(&k2, &v2).unwrap();
                    match tr.commit(&mut c) {
                        Ok(_) => {
                            model.insert(k, v1);
                            model.insert(k2, v2);
                            commits_ok += 1;
                            multi_ok += 1;
                        }
                        Err(_) => commits_err += 1,
                    }
                }
                5 => {
                    // intentional WW: two TX same key — exactly one should win
                    let mut t1 = c.begin();
                    let mut t2 = c.begin();
                    let _ = t1.get(&c, &k);
                    let _ = t2.get(&c, &k);
                    let va = format!("a{}", next() % 100).into_bytes();
                    let vb = format!("b{}", next() % 100).into_bytes();
                    t1.set(&k, &va).unwrap();
                    t2.set(&k, &vb).unwrap();
                    let r1 = t1.commit(&mut c);
                    let r2 = t2.commit(&mut c);
                    let wins = r1.is_ok() as u8 + r2.is_ok() as u8;
                    if wins != 1 {
                        mismatches += 1;
                        notes.push(format!("mini-bt WW wins={wins} i={i}"));
                    } else {
                        ww_ok += 1;
                        if r1.is_ok() {
                            model.insert(k, va);
                        } else {
                            model.insert(k, vb);
                        }
                        commits_ok += 1;
                        commits_err += 1; // the loser
                    }
                }
                _ => {
                    let mut tr = c.begin();
                    let pairs = tr.get_range(&c, b"mb/", b"mb0").unwrap();
                    for (pk, pv) in &pairs {
                        if let Some(ev) = model.get(pk) {
                            if ev != pv {
                                mismatches += 1;
                            }
                        }
                    }
                    for (mk, mv) in &model {
                        if mk.starts_with(b"mb/") && !pairs.iter().any(|(k, v)| k == mk && v == mv)
                        {
                            mismatches += 1;
                        }
                    }
                }
            }
        }
        let wall = t0.elapsed();
        let ops_s = ops as f64 / wall.as_secs_f64().max(1e-12);
        benches.push(format!(
            r#"{{
    "name": "E1_mini_bindingtester_soak",
    "ops": {ops},
    "ops_per_s": {ops_s:.2},
    "commits_ok": {commits_ok},
    "commits_err": {commits_err},
    "multi_key_ok": {multi_ok},
    "ww_pairs_ok": {ww_ok},
    "mismatches": {mismatches},
    "wall_s": {ws:.4},
    "pass": {pass}
  }}"#,
            ws = wall.as_secs_f64(),
            pass = pedradb_core::write_admission_kernel::batch_is_empty(mismatches),
        ));
        progress!(
            "E1 mini-bt ops={ops} mismatches={mismatches} ok_commit={commits_ok} multi={multi_ok} ww={ww_ok}"
        );
        if !pedradb_core::write_admission_kernel::batch_is_empty(mismatches) {
            notes.push(format!("mini-bt FAILED mismatches={mismatches}"));
        }
        drop(c);

        // E2: TCP multi-client — partitioned keys (no model race), final global verify
        if suite_enabled("tcp") || suite_enabled("threads") || suite_enabled("all") {
            match find_montanha_tcp() {
                None => notes.push("E2 skipped: no montanha-tcp".into()),
                Some(bin) => {
                    progress!("E2 TCP multi-client mini-bt ({n_threads} thr)…");
                    let tmp = out.join("tcp-minib");
                    let _ = std::fs::remove_dir_all(&tmp);
                    std::fs::create_dir_all(&tmp).unwrap();
                    let nodes = start_tcp_cluster(&bin, &tmp, 1);
                    let peers: Vec<(u64, String)> =
                        nodes.iter().map(|n| (n.id, n.addr.to_string())).collect();
                    let addrs: Vec<String> = nodes.iter().map(|n| n.addr.to_string()).collect();
                    let per = (n.max(20) / n_threads).max(8);
                    let peers_a = Arc::new(peers);
                    let mut handles = Vec::new();
                    let t0 = Instant::now();
                    for tid in 0..n_threads {
                        let peers = (*peers_a).clone();
                        handles.push(thread::spawn(move || {
                            let mut cli = TcpClusterClient::new(peers).with_max_attempts(64);
                            let mut local: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
                            let mut ok = 0u64;
                            let mut err = 0u64;
                            let mut rng = 0xBEEF_u64 ^ (tid as u64).wrapping_mul(0x9E37);
                            let mut next = || {
                                rng ^= rng << 13;
                                rng ^= rng >> 7;
                                rng ^= rng << 17;
                                rng
                            };
                            for i in 0..per {
                                let slot = next() % 16;
                                let k = format!("e2/{tid}/{slot}").into_bytes();
                                let op = next() % 3;
                                match op {
                                    0 => {
                                        let v = format!("t{tid}-i{i}").into_bytes();
                                        let mut done = false;
                                        for _ in 0..10 {
                                            if cli.put(&k, &v).is_ok() {
                                                local.insert(k.clone(), v.clone());
                                                ok += 1;
                                                done = true;
                                                break;
                                            }
                                            thread::sleep(Duration::from_millis(15));
                                        }
                                        if !done {
                                            err += 1;
                                        }
                                    }
                                    1 => {
                                        // 2-key TX in same partition
                                        let k2 =
                                            format!("e2/{tid}/{}", (slot + 1) % 16).into_bytes();
                                        let v1 = format!("a{i}").into_bytes();
                                        let v2 = format!("b{i}").into_bytes();
                                        let pairs =
                                            vec![(k.clone(), v1.clone()), (k2.clone(), v2.clone())];
                                        let mut done = false;
                                        for _ in 0..12 {
                                            if cli.commit_tx(&pairs).is_ok() {
                                                local.insert(k.clone(), v1.clone());
                                                local.insert(k2, v2);
                                                ok += 1;
                                                done = true;
                                                break;
                                            }
                                            thread::sleep(Duration::from_millis(20));
                                        }
                                        if !done {
                                            err += 1;
                                        }
                                    }
                                    _ => {
                                        // read own key — best effort (may lag)
                                        let _ = cli; // use get via any peer in verify pass
                                    }
                                }
                            }
                            (ok, err, local)
                        }));
                    }
                    let mut merged = BTreeMap::new();
                    let mut ok_all = 0u64;
                    let mut err_all = 0u64;
                    for h in handles {
                        let (ok, err, local) = h.join().unwrap();
                        ok_all += ok;
                        err_all += err;
                        for (k, v) in local {
                            merged.insert(k, v);
                        }
                    }
                    // Global verify: every merged key visible on majority
                    let mut mismatches = 0u64;
                    let mut verified = 0u64;
                    for (k, v) in &merged {
                        let mut seen = 0u32;
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while Instant::now() < deadline && seen < 2 {
                            seen = 0;
                            for a in &addrs {
                                if client_get(a, k).ok().flatten().as_deref() == Some(v.as_slice())
                                {
                                    seen += 1;
                                }
                            }
                            if seen < 2 {
                                thread::sleep(Duration::from_millis(30));
                            }
                        }
                        if seen >= 2 {
                            verified += 1;
                        } else {
                            mismatches += 1;
                            if mismatches < 5 {
                                notes.push(format!(
                                    "E2 missing majority key={}",
                                    String::from_utf8_lossy(k)
                                ));
                            }
                        }
                    }
                    let wall = t0.elapsed();
                    benches.push(format!(
                        r#"{{
    "name": "E2_tcp_multiclient_mini_bt",
    "threads": {n_threads},
    "ops_ok": {ok_all},
    "ops_err": {err_all},
    "model_keys": {mk},
    "verified_majority": {verified},
    "mismatches": {mismatches},
    "wall_s": {ws:.4},
    "pass": {pass}
  }}"#,
                        mk = merged.len(),
                        ws = wall.as_secs_f64(),
                        pass = pedradb_core::write_admission_kernel::batch_is_empty(mismatches)
                            && err_all < ok_all.saturating_add(1),
                    ));
                    progress!(
                        "E2 tcp multi-bt ok={ok_all} err={err_all} verified={verified}/{} mismatches={mismatches}",
                        merged.len()
                    );
                    if !pedradb_core::write_admission_kernel::batch_is_empty(mismatches) {
                        notes.push(format!("E2 FAILED mismatches={mismatches}"));
                    }
                    drop(nodes);
                }
            }
        }
    }

    let limitations = r#"[
    "In-process core suite: no real NIC (suite tcp adds real localhost TCP).",
    "C1 Mutex multi-thread serializes StoreCluster — concurrent waiters, not parallel leaders.",
    "D4 multi-thread TCP is the honest concurrent client path on one range.",
    "FDB face path (A3) includes OCC bookkeeping; compare to A1 raw put for face overhead.",
    "Multi-range TX (B2) exercises cross-range 2PC; expect cliff vs A4.",
    "E1 mini-bt: model-checked random soak + multi-key + WW pairs (not full bindingtester).",
    "E2 TCP multi-client: partitioned keys per thread + majority verify (true concurrent writers).",
    "Not YCSB / not production FDB — see docs/montanha-vs-fdb-bench.md."
  ]"#;

    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".into());
    let suite =
        std::env::var("MONTANHA_BENCH_SUITE").unwrap_or_else(|_| "core,threads,mini-bt".into());
    let write_bp = write_backpressure_enabled();
    if write_bp {
        notes.push("write_backpressure=1".into());
    }
    let admission_a_json = admission_core_a.unwrap_or_else(|| "null".into());
    let admission_b_json = admission_core_b.unwrap_or_else(|| "null".into());
    let notes_json = if notes.is_empty() {
        "[]".into()
    } else {
        format!(
            "[{}]",
            notes
                .iter()
                .map(|s| format!("\"{}\"", s.replace('\"', "'")))
                .collect::<Vec<_>>()
                .join(",")
        )
    };

    let report = format!(
        r#"{{
  "bench": "montanha-fdb-shaped-v1",
  "host": "{host}",
  "suite": "{suite}",
  "write_backpressure": {write_bp},
  "admission_core_a": {admission_a_json},
  "admission_core_b": {admission_b_json},
  "nodes": 3,
  "payload_bytes": {payload},
  "n_default": {n},
  "tx_keys": {tx_keys},
  "multi_ranges": {n_ranges},
  "threads": {n_threads},
  "warmup": {warmup},
  "benches": [
    {body}
  ],
  "limitations": {limitations},
  "anomalies": {notes_json},
  "vs_fdb": {{
    "claim": "not field peer",
    "how_to_compare": "docs/montanha-vs-fdb-bench.md",
    "fdb_workloads": [
      "single key set/get (A1/A3/D1)",
      "multi-key transaction (A4/D3)",
      "get_range (A5)",
      "clear_range (A6)",
      "hot key conflicts (A7)",
      "index+row TX (A8)",
      "multi-shard / multi-range TX (B2)",
      "multi-client TCP (D4)",
      "random soak (E1)",
      "TCP multi-client mini-bt (E2)"
    ]
  }}
}}
"#,
        body = benches.join(",\n    "),
    );

    let path = out.join("fdb_shaped_bench.json");
    std::fs::write(&path, &report).expect("write");
    println!("{report}");
    println!("wrote {}", path.display());

    eprintln!("--- montanha-fdb-bench done ---");
    eprintln!("report: {}", path.display());
    eprintln!("Suites: core | threads | tcp | mini-bt | all");
    eprintln!("Compare A1 vs A3 (face), A4 vs B2 (2PC), A1 vs D1 (TCP tax), C1 vs D4 (mutex vs real concurrent).");
}
