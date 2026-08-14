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
//!
//! Writes `fdb_shaped_bench.json` + human summary. Compare to FDB using the same
//! workload shapes (see docs/montanha-vs-fdb-bench.md). Not a claim of field parity.

#![forbid(unsafe_code)]

use pedradb_store::{FdbDatabase, StoreCluster};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
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
    let val = vec![b'x'; payload];

    let mut benches = Vec::new();
    let mut notes = Vec::new();

    macro_rules! progress {
        ($($t:tt)*) => {{
            eprintln!("[fdb-bench] {}", format!($($t)*));
        }};
    }

    // ── Suite A: single-range majority (baseline substrate) ───────────────
    {
        let dir = out.join("db-r1");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        progress!("A open 3-node 1-range…");
        let mut c = StoreCluster::open(&dir, 3, 1).expect("open r1");
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
        benches.push(summarize(
            "A5_get_range_prefix",
            n,
            t0.elapsed(),
            &mut lats,
        ));
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
                    notes.push(format!(
                        "A7 anomaly: both commits ok on hot key iter {i}"
                    ));
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

        drop(c);
    }

    // ── Suite B: multi-range (N writers potential / cross-range 2PC cost) ─
    {
        let dir = out.join(format!("db-r{n_ranges}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        progress!("B open 3-node {n_ranges}-range…");
        let mut c = StoreCluster::open(&dir, 3, n_ranges).expect("open multi");
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
        let take = range_keys.len().min(4).max(2);
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

        drop(c);
    }

    // ── Limitation heuristics (ratios of thumb from this run) ─────────────
    let limitations = r#"[
    "In-process 3-node majority: no real NIC/kernel TCP in this binary (use montanha-tcp benches separately).",
    "Single-threaded client: not max cluster aggregate; measures serial substrate cost.",
    "FDB face path (A3) includes OCC bookkeeping; compare to A1 raw put for face overhead.",
    "Multi-range TX (B2) exercises cross-range commit; expect >> same-range A4 if 2PC dominates.",
    "Hot-key A7 measures conflict correctness under sequential dual-TX, not parallel OS threads.",
    "Not YCSB / not fdb_latency / not production FDB cluster — use docs/montanha-vs-fdb-bench.md for FDB side.",
    "Disk/fsync policy is Pedra lab default on this machine — label sync mode when comparing to FDB."
  ]"#;

    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".into());
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
  "bench": "montanha-fdb-shaped-v0",
  "host": "{host}",
  "nodes": 3,
  "payload_bytes": {payload},
  "n_default": {n},
  "tx_keys": {tx_keys},
  "multi_ranges": {n_ranges},
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
      "single key set/get (A1/A3)",
      "multi-key transaction (A4)",
      "get_range (A5)",
      "clear_range (A6)",
      "hot key conflicts (A7)",
      "index+row TX (A8)",
      "multi-shard / multi-range TX (B2)"
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

    // Human ratio hints if we can parse qps from names — optional stderr summary
    eprintln!("--- montanha-fdb-bench done ---");
    eprintln!("report: {}", path.display());
    eprintln!("Compare A1 vs A3 (face overhead), A4 vs B2 (cross-range tax), A5/A6 range costs.");
}
