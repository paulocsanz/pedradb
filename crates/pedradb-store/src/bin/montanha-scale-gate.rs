//! RFC-0021 / 0025 — scale option-A gate (in-process, CI-friendly).
//!
//! Checks:
//! 1. Multi-range elect diversifies leaders (≥2 distinct leader nodes @ 4 ranges).
//! 2. Same-range `put_batch` works after multi-range elect.
//! 3. Disjoint multi-range puts complete (capacity path sanity).
//!
//! Usage:
//!   cargo run -p pedradb-store --release --bin montanha-scale-gate -- [out_dir]
//!
//! Env:
//!   MONTANHA_SCALE_KEYS  puts per range probe (default 8)
//!   MONTANHA_SCALE_BATCH batch size for put_batch (default 8)
//!   MONTANHA_WRITE_BACKPRESSURE=1  enable Pedra L0 pressure/stall defaults
//!
//! Exit 0 only if all checks pass; writes `scale_report.json`.

#![forbid(unsafe_code)]

use pedradb_store::{StoreCluster, StoreOpenOptions};
use std::path::PathBuf;
use std::process;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn store_opts() -> StoreOpenOptions {
    let mut opts = StoreOpenOptions::default();
    if std::env::var("MONTANHA_WRITE_BACKPRESSURE").ok().as_deref() == Some("1") {
        opts = opts.with_write_backpressure();
    }
    opts
}

fn open_cluster(dir: &std::path::Path, n_nodes: u64, n_ranges: u64) -> StoreCluster {
    StoreCluster::open_with_options(dir, n_nodes, n_ranges, store_opts()).expect("open cluster")
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
            PathBuf::from(format!("findings/scale-gate-{ts}"))
        });
    std::fs::create_dir_all(&out).expect("mkdir out");

    let n_keys: usize = std::env::var("MONTANHA_SCALE_KEYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let batch_sz: usize = std::env::var("MONTANHA_SCALE_BATCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let write_bp = std::env::var("MONTANHA_WRITE_BACKPRESSURE").ok().as_deref() == Some("1");

    let mut failures: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if write_bp {
        notes.push("write_backpressure=1".into());
    }

    // ── Check 1: leader diversity @ 4 ranges ─────────────────────────────
    let dir4 = out.join("db-r4");
    let _ = std::fs::remove_dir_all(&dir4);
    std::fs::create_dir_all(&dir4).unwrap();
    let mut c4 = open_cluster(&dir4, 3, 4);
    c4.elect_all(200).expect("elect 4-range");
    let leaders = c4.leader_nodes();
    let n_leaders = leaders.len();
    if n_leaders < 2 {
        failures.push(format!(
            "leader diversity: expected ≥2 leader nodes @ 4 ranges, got {leaders:?}"
        ));
    } else {
        notes.push(format!("leader_nodes={leaders:?}"));
    }

    // ── Check 2: put_batch same-range after multi-range elect ─────────────
    let metas = c4.range_metas();
    let base = if metas[0].start.is_empty() {
        vec![0u8, b'b']
    } else {
        let mut k = metas[0].start.clone();
        k.push(b'b');
        k
    };
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..batch_sz)
        .map(|i| {
            let mut k = base.clone();
            k.extend_from_slice(format!("-{i:03}").as_bytes());
            (k, vec![b'v'; 32])
        })
        .collect();
    let t_batch = Instant::now();
    match c4.put_batch(pairs.iter().map(|(k, v)| (k.as_slice(), v.as_slice()))) {
        Ok(()) => notes.push(format!(
            "put_batch_{batch_sz} ok wall_ms={:.1}",
            t_batch.elapsed().as_secs_f64() * 1000.0
        )),
        Err(e) => failures.push(format!("put_batch failed: {e}")),
    }

    // ── Check 3: disjoint multi-range puts (one key per range) ───────────
    let t_mr = Instant::now();
    let mut mr_ok = 0u64;
    let range_keys: Vec<(u64, Vec<u8>)> = c4
        .range_metas()
        .iter()
        .map(|r| {
            let mut k = if r.start.is_empty() {
                vec![0u8, b'm']
            } else {
                let mut k = r.start.clone();
                k.push(b'm');
                k
            };
            k.extend_from_slice(b"-gate");
            (r.id, k)
        })
        .collect();
    for (rid, k) in &range_keys {
        match c4.put(k, b"mr") {
            Ok(()) => mr_ok += 1,
            Err(e) => failures.push(format!("multi-range put rid={rid} err={e}")),
        }
    }
    let mr_wall = t_mr.elapsed().as_secs_f64();
    let mr_kps = mr_ok as f64 / mr_wall.max(1e-12);
    if mr_ok < 4 {
        failures.push(format!("multi-range puts: expected 4 ok, got {mr_ok}"));
    } else {
        notes.push(format!("disjoint_put_4ranges keys_per_s={mr_kps:.3}"));
    }

    // ── Check 4: 1-range baseline still elects + puts ─────────────────────
    let dir1 = out.join("db-r1");
    let _ = std::fs::remove_dir_all(&dir1);
    std::fs::create_dir_all(&dir1).unwrap();
    let mut c1 = open_cluster(&dir1, 3, 1);
    c1.elect_all(120).expect("elect 1-range");
    let t1 = Instant::now();
    let mut ok1 = 0u64;
    for i in 0..n_keys {
        let k = format!("s1-{i:04}").into_bytes();
        if c1.put(&k, b"x").is_ok() {
            ok1 += 1;
        }
    }
    let kps1 = ok1 as f64 / t1.elapsed().as_secs_f64().max(1e-12);
    if ok1 != n_keys as u64 {
        failures.push(format!("1-range put: expected {n_keys} ok, got {ok1}"));
    } else {
        notes.push(format!("single_range_put keys_per_s={kps1:.3}"));
    }

    // Pedra L0 / stall counters after load (ops honesty + structured A/B).
    let adm_r4 = c4.write_admission_snap();
    let adm_r1 = c1.write_admission_snap();
    let status_r4 = c4.status_text();
    let status_r1 = c1.status_text();
    notes.push(format!("status_r4={status_r4}"));
    notes.push(format!("status_r1={status_r1}"));
    notes.push(format!("admission_r4={}", adm_r4.to_json_object()));
    notes.push(format!("admission_r1={}", adm_r1.to_json_object()));
    if write_bp {
        if adm_r4.write_stall_l0 == 0 && adm_r1.write_stall_l0 == 0 {
            failures.push("write_backpressure=1 but write_stall_l0 config is 0".into());
        }
    }

    drop(c4);
    drop(c1);

    let pass = failures.is_empty();
    let adm_r4_json = adm_r4.to_json_object();
    let adm_r1_json = adm_r1.to_json_object();
    let report = format!(
        r#"{{
  "gate": "rfc0021-0025-scale-option-a-v0",
  "pass": {pass},
  "write_backpressure": {write_bp},
  "leader_nodes_r4": {n_leaders},
  "multi_range_puts_ok": {mr_ok},
  "multi_range_keys_per_s": {mr_kps:.3},
  "single_range_keys_per_s": {kps1:.3},
  "put_batch_sz": {batch_sz},
  "admission_r4": {adm_r4_json},
  "admission_r1": {adm_r1_json},
  "notes": {notes:?},
  "failures": {failures:?},
  "note": "in-process; TCP multi-client remains montanha-fdb-bench suite scale; MONTANHA_WRITE_BACKPRESSURE=1 opts into Pedra L0 admission; admission_* are WriteAdmissionSnap aggregates"
}}
"#
    );
    let path = out.join("scale_report.json");
    std::fs::write(&path, &report).expect("write report");
    println!("{report}");
    println!("wrote {}", path.display());
    if !pass {
        eprintln!("scale gate FAILED: {failures:?}");
        process::exit(1);
    }
    println!("scale gate OK (option A hygiene)");
}
