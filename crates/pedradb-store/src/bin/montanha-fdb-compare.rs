//! RFC-0025 P2.1 — compare Montanha FDB-shaped bench JSON to an optional FDB side.
//!
//! Usage:
//!   cargo run -p pedradb-store --release --bin montanha-fdb-compare -- \
//!     findings/fdb-bench-scale-s10/fdb_shaped_bench.json [out_dir]
//!
//! Optional peer file (pre-filled FDB measurements):
//!   MONTANHA_FDB_PEER=path/to/fdb_shaped_peer.json
//!
//! If `FDB_CLUSTER_FILE` is set and `fdbcli` is on PATH, attempts a minimal
//! fdbcli probe. Otherwise writes a fill-in template so CI stays green without
//! FoundationDB installed.
//!
//! Not a claim of field parity — fills the comparison skeleton.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() {
    let montanha_json = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "findings/fdb-bench-scale-s10/fdb_shaped_bench.json".into());
    let out = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            PathBuf::from(format!("findings/fdb-compare-{ts}"))
        });
    std::fs::create_dir_all(&out).expect("mkdir");

    let montanha_raw = std::fs::read_to_string(&montanha_json).unwrap_or_else(|_| "{}".into());
    let extracted = extract_montanha_metrics(&montanha_raw);

    let peer_path = std::env::var("MONTANHA_FDB_PEER").ok();
    let peer_raw = peer_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let fdb_extracted = peer_raw
        .as_ref()
        .map(|s| extract_montanha_metrics(s))
        .unwrap_or_default();

    let fdb_cluster = std::env::var("FDB_CLUSTER_FILE").ok();
    let fdbcli = which("fdbcli");
    let mut fdb_status = if peer_raw.is_some() {
        "peer_file".to_string()
    } else {
        "unavailable".to_string()
    };
    let mut fdb_probe = "null".to_string();

    if let (Some(cluster), Some(cli)) = (fdb_cluster.as_ref(), fdbcli.as_ref()) {
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

    // Ratio table: montanha / fdb when both present; else null fdb.
    let mut ratios = String::from("[\n");
    // Prefer multi-range scale rows when present (option A).
    let shapes = [
        ("A1", "A1"),
        ("A3", "A3"),
        ("A4", "A4"),
        ("B2", "B2"),
        ("D1", "D1"),
        ("D6", "D6"),
        ("S2", "S2_tcp_mt_put_r4"),
        ("S3", "S3_tcp_mt_put_batch_r4"),
        ("S3_r8", "S3_tcp_mt_put_batch_r8"),
        ("A1b", "A1b"),
    ];
    for (i, (shape, prefer)) in shapes.iter().enumerate() {
        let m = extracted
            .iter()
            .find(|(n, _)| n.contains(prefer))
            .or_else(|| extracted.iter().find(|(n, _)| n.contains(shape)));
        let f = fdb_extracted
            .iter()
            .find(|(n, _)| n.contains(prefer))
            .or_else(|| fdb_extracted.iter().find(|(n, _)| n.contains(shape)));
        let m_kps = m.map(|(_, k)| *k);
        let f_kps = f.map(|(_, k)| *k);
        let ratio = match (m_kps, f_kps) {
            (Some(a), Some(b)) if b > 0.0 => format!("{:.3}", a / b),
            _ => "null".into(),
        };
        let m_s = m_kps
            .map(|k| format!("{k:.3}"))
            .unwrap_or_else(|| "null".into());
        let f_s = f_kps
            .map(|k| format!("{k:.3}"))
            .unwrap_or_else(|| "null".into());
        let m_name = m.map(|(n, _)| n.as_str()).unwrap_or("");
        if i > 0 {
            ratios.push_str(",\n");
        }
        ratios.push_str(&format!(
            r#"    {{"shape":"{shape}","montanha_name":"{m_name}","montanha_keys_per_s":{m_s},"fdb_keys_per_s":{f_s},"montanha_over_fdb":{ratio}}}"#
        ));
    }
    ratios.push_str("\n  ]");

    let template = fdb_peer_template(&extracted);

    let cf_json = fdb_cluster
        .as_ref()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .unwrap_or_else(|| "null".into());
    let cli_json = fdbcli
        .as_ref()
        .map(|p| format!("\"{}\"", p.display()))
        .unwrap_or_else(|| "null".into());
    let peer_json = peer_path
        .as_ref()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .unwrap_or_else(|| "null".into());

    let montanha_metrics_json = metrics_to_json(&extracted);
    let report = format!(
        r#"{{
  "compare": "montanha-fdb-shaped-v1",
  "montanha_path": {mj:?},
  "montanha_metrics": {montanha_metrics_json},
  "fdb": {{
    "status": "{fdb_status}",
    "cluster_file": {cf_json},
    "fdbcli": {cli_json},
    "peer_file": {peer_json},
    "probe": {fdb_probe},
    "how_to_fill": "1) Lab fdbserver. 2) scripts/fdb_side_shapes.sh or Python binding. 3) Write fdb_shaped_peer.json with same bench names + keys_per_s. 4) MONTANHA_FDB_PEER=... montanha-fdb-compare. See docs/montanha-vs-fdb-bench.md"
  }},
  "ratios": {ratios},
  "honesty": "Montanha numbers alone are not field parity. FDB side optional. Topology/durability must be labeled."
}}
"#,
        mj = montanha_json,
    );

    let path = out.join("compare_report.json");
    std::fs::write(&path, &report).expect("write compare");
    let tmpl = out.join("fdb_shaped_peer.template.json");
    std::fs::write(&tmpl, &template).expect("write template");
    println!("{report}");
    eprintln!("wrote {}", path.display());
    eprintln!("wrote {} (fill keys_per_s then set MONTANHA_FDB_PEER)", tmpl.display());
}

/// Best-effort extract name → keys_per_s (or qps) from fdb_shaped_bench JSON.
fn extract_montanha_metrics(raw: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    // Walk benches array objects without a full JSON crate.
    for chunk in raw.split("\"name\"") {
        let Some(name) = json_string_after(chunk, ':') else {
            continue;
        };
        let kps = json_number_field(chunk, "keys_per_s")
            .or_else(|| json_number_field(chunk, "qps"))
            .or_else(|| json_number_field(chunk, "keys_ok").and_then(|k| {
                json_number_field(chunk, "wall_s").map(|w| k / w.max(1e-12))
            }));
        if let Some(v) = kps {
            out.insert(name, v);
        }
    }
    out
}

fn json_string_after(s: &str, after: char) -> Option<String> {
    let i = s.find(after)?;
    let rest = s[i + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn json_number_field(s: &str, field: &str) -> Option<f64> {
    let key = format!("\"{field}\"");
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == 'e' || *c == 'E')
        .collect();
    num.parse().ok()
}

fn metrics_to_json(m: &BTreeMap<String, f64>) -> String {
    let mut s = String::from("{\n");
    for (i, (k, v)) in m.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        s.push_str(&format!("    \"{k}\": {v:.3}"));
    }
    s.push_str("\n  }");
    s
}

fn fdb_peer_template(montanha: &BTreeMap<String, f64>) -> String {
    let mut s = String::from("{\n  \"bench\": \"fdb-side-peer-template\",\n  \"topology\": \"TODO: label fdbserver layout\",\n  \"durability\": \"TODO: sync/storage class\",\n  \"benches\": [\n");
    for (i, (name, mkps)) in montanha.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        s.push_str(&format!(
            r#"    {{
      "name": "{name}",
      "keys_per_s": null,
      "montanha_keys_per_s_ref": {mkps:.3},
      "note": "fill keys_per_s from FDB same shape"
    }}"#
        ));
    }
    if montanha.is_empty() {
        s.push_str(
            r#"    {
      "name": "A1_raw_put",
      "keys_per_s": null,
      "note": "fill after montanha bench"
    }"#,
        );
    }
    s.push_str("\n  ]\n}\n");
    s
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

#[cfg(test)]
mod tests {
    // Binary crate — run via `cargo run`; parse helpers tested by integration use.
}
