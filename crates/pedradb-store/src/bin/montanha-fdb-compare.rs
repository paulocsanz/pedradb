//! RFC-0025 P2.1 — compare Montanha FDB-shaped bench JSON to an optional FDB side.
//!
//! Usage:
//!   cargo run -p pedradb-store --release --bin montanha-fdb-compare -- \
//!     findings/fdb-bench-p0/fdb_shaped_bench.json [out_dir]
//!
//! If `FDB_CLUSTER_FILE` is set and `fdbcli` is on PATH, attempts a minimal
//! fdbcli latency probe and records it. Otherwise writes a template with
//! `fdb_status: unavailable` so CI stays green without Apple FDB installed.
//!
//! Not a claim of field parity — fills the comparison skeleton.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() {
    let montanha_json = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "findings/fdb-bench-p0/fdb_shaped_bench.json".into());
    let out = std::env::args().nth(2).map(PathBuf::from).unwrap_or_else(|| {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        PathBuf::from(format!("findings/fdb-compare-{ts}"))
    });
    std::fs::create_dir_all(&out).expect("mkdir");

    let montanha_raw =
        std::fs::read_to_string(&montanha_json).unwrap_or_else(|_| "{}".into());

    let fdb_cluster = std::env::var("FDB_CLUSTER_FILE").ok();
    let fdbcli = which("fdbcli");
    let mut fdb_status = "unavailable".to_string();
    let mut fdb_probe = "null".to_string();

    if let (Some(cluster), Some(cli)) = (fdb_cluster.as_ref(), fdbcli.as_ref()) {
        // Minimal: time `status` — not a full workload; documents hook for real probes.
        let t0 = Instant::now();
        let st = Command::new(cli)
            .args(["-C", cluster, "--exec", "status minimal"])
            .output();
        let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
        match st {
            Ok(o) if o.status.success() => {
                fdb_status = "ok".into();
                fdb_probe = format!(
                    r#"{{"op":"status_minimal","wall_ms":{wall_ms:.3},"stdout_len":{}}}"#,
                    o.stdout.len()
                );
            }
            Ok(o) => {
                fdb_status = "error".into();
                let err = String::from_utf8_lossy(&o.stderr);
                fdb_probe = format!(
                    r#"{{"op":"status_minimal","wall_ms":{wall_ms:.3},"stderr":"{}"}}"#,
                    err.chars().take(200).collect::<String>().replace('"', "'")
                );
            }
            Err(e) => {
                fdb_status = "error".into();
                fdb_probe = format!(r#"{{"error":"{e}"}}"#);
            }
        }
    }

    let cf_json = fdb_cluster
        .as_ref()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .unwrap_or_else(|| "null".into());
    let cli_json = fdbcli
        .as_ref()
        .map(|p| format!("\"{}\"", p.display()))
        .unwrap_or_else(|| "null".into());
    let report = format!(
        r#"{{
  "compare": "montanha-fdb-shaped-v0",
  "montanha_path": {mj:?},
  "montanha": {montanha},
  "fdb": {{
    "status": "{fdb_status}",
    "cluster_file": {cf_json},
    "fdbcli": {cli_json},
    "probe": {fdb_probe},
    "how_to_fill": "Run matching shapes with fdbcli/Python bindings; paste keys/s + p50 into a fdb_shaped_bench.json peer file. See docs/montanha-vs-fdb-bench.md"
  }},
  "honesty": "Montanha numbers alone are not field parity. FDB side optional."
}}
"#,
        mj = montanha_json,
        montanha = if montanha_raw.trim().is_empty() {
            "{}".into()
        } else {
            montanha_raw
        },
    );

    let path = out.join("compare_report.json");
    std::fs::write(&path, &report).expect("write");
    println!("{report}");
    eprintln!("wrote {}", path.display());
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(bin);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
