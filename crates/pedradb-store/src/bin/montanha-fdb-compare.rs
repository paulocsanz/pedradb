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
    let montanha_bp = extract_bool_field(&montanha_raw, "write_backpressure");

    let peer_path = std::env::var("MONTANHA_FDB_PEER").ok();
    let peer_raw = peer_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let fdb_extracted = peer_raw
        .as_ref()
        .map(|s| extract_montanha_metrics(s))
        .unwrap_or_default();
    let fdb_bp = peer_raw
        .as_ref()
        .and_then(|s| extract_bool_field(s, "write_backpressure"));

    let fdb_cluster = std::env::var("FDB_CLUSTER_FILE").ok();
    let fdbcli = which("fdbcli");
    let mut fdb_status = if peer_raw.is_some() {
        "peer_file".to_string()
    } else {
        "unavailable".to_string()
    };
    // Surface the peer's own status so a stub ("unavailable") is visible in the
    // report instead of reading as a real lab run.
    if let Some(peer_status) = peer_raw.as_deref().and_then(|s| extract_string_field(s, "status")) {
        fdb_status = format!("peer_file:{peer_status}");
    }
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
        ("ycsb_a", "ycsb_a"),
        ("ycsb_b", "ycsb_b"),
        ("ycsb_c", "ycsb_c"),
        ("ycsb_d", "ycsb_d"),
        ("ycsb_e", "ycsb_e"),
        ("ycsb_f", "ycsb_f"),
    ];
    // Optional parity gate: MONTANHA_PARITY_RATIO_FLOOR=0.5 fails any real
    // ratio below it (lab mode; CI template mode leaves it unset).
    let parity_floor: Option<f64> = std::env::var("MONTANHA_PARITY_RATIO_FLOOR")
        .ok()
        .and_then(|s| s.parse().ok());
    let mut real_ratios: Vec<f64> = Vec::new();
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
        let (ratio, ratio_v) = match (m_kps, f_kps) {
            (Some(a), Some(b)) if b > 0.0 => {
                let v = a / b;
                real_ratios.push(v);
                (format!("{v:.3}"), Some(v))
            }
            _ => ("null".into(), None),
        };
        let meets_floor = match (parity_floor, ratio_v) {
            (Some(floor), Some(v)) => format!("{}", v >= floor),
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
            r#"    {{"shape":"{shape}","montanha_name":"{m_name}","montanha_keys_per_s":{m_s},"fdb_keys_per_s":{f_s},"montanha_over_fdb":{ratio},"meets_floor":{meets_floor}}}"#
        ));
    }
    ratios.push_str("\n  ]");

    // Parity summary: only meaningful when a real peer produced ratios.
    let shapes_with_peer = real_ratios.len();
    let parity = if let Some(floor) = parity_floor {
        if real_ratios.is_empty() {
            format!(
                r#"{{"floor": {floor}, "shapes_with_peer": 0, "min_ratio": null, "pass": null, "note": "floor set but no peer ratios — template mode"}}"#
            )
        } else {
            let min_r = real_ratios.iter().cloned().fold(f64::INFINITY, f64::min);
            let pass = real_ratios.iter().all(|v| *v >= floor);
            format!(
                r#"{{"floor": {floor}, "shapes_with_peer": {shapes_with_peer}, "min_ratio": {min_r:.3}, "pass": {pass}}}"#
            )
        }
    } else {
        format!(
            r#"{{"floor": null, "shapes_with_peer": {shapes_with_peer}, "min_ratio": null, "pass": null, "note": "set MONTANHA_PARITY_RATIO_FLOOR to gate"}}"#
        )
    };

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
    let montanha_bp_json = bool_opt_json(montanha_bp);
    let fdb_bp_json = bool_opt_json(fdb_bp);
    let report = format!(
        r#"{{
  "compare": "montanha-fdb-shaped-v1",
  "montanha_path": {mj:?},
  "montanha_write_backpressure": {montanha_bp_json},
  "montanha_metrics": {montanha_metrics_json},
  "fdb": {{
    "status": "{fdb_status}",
    "cluster_file": {cf_json},
    "fdbcli": {cli_json},
    "peer_file": {peer_json},
    "write_backpressure": {fdb_bp_json},
    "probe": {fdb_probe},
    "how_to_fill": "1) Lab fdbserver. 2) scripts/fdb_side_shapes.sh or Python binding. 3) Write fdb_shaped_peer.json with same bench names + keys_per_s (+ optional write_backpressure). 4) MONTANHA_FDB_PEER=... montanha-fdb-compare. See docs/montanha-vs-fdb-bench.md"
  }},
  "ratios": {ratios},
  "parity": {parity},
  "honesty": "Montanha numbers alone are not field parity. FDB side optional. Topology/durability must be labeled. write_backpressure is pass-through from Montanha/peer JSON (MONTANHA_WRITE_BACKPRESSURE lab flag)."
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
    eprintln!(
        "wrote {} (fill keys_per_s then set MONTANHA_FDB_PEER)",
        tmpl.display()
    );
    // Lab parity gate: floor set + real peer + any ratio below floor → nonzero.
    if let Some(floor) = parity_floor {
        if !real_ratios.is_empty() && !real_ratios.iter().all(|v| *v >= floor) {
            eprintln!(
                "parity gate FAILED: floor={floor} min_ratio={:.3} shapes={}",
                real_ratios.iter().cloned().fold(f64::INFINITY, f64::min),
                real_ratios.len()
            );
            std::process::exit(2);
        }
    }
}

fn extract_bool_field(raw: &str, field: &str) -> Option<bool> {
    let key = format!("\"{field}\"");
    let i = raw.find(&key)?;
    let rest = &raw[i + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_string_field(raw: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let i = raw.find(&key)?;
    let rest = raw[i + key.len()..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn bool_opt_json(v: Option<bool>) -> String {
    match v {
        Some(true) => "true".into(),
        Some(false) => "false".into(),
        None => "null".into(),
    }
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
            .or_else(|| {
                json_number_field(chunk, "keys_ok")
                    .and_then(|k| json_number_field(chunk, "wall_s").map(|w| k / w.max(1e-12)))
            });
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
    use super::*;

    #[test]
    fn extract_bool_write_backpressure() {
        let raw = r#"{"bench":"x","write_backpressure":true,"benches":[]}"#;
        assert_eq!(extract_bool_field(raw, "write_backpressure"), Some(true));
        let raw2 = r#"{"write_backpressure": false}"#;
        assert_eq!(extract_bool_field(raw2, "write_backpressure"), Some(false));
        assert_eq!(extract_bool_field("{}", "write_backpressure"), None);
    }

    #[test]
    fn extract_metrics_still_works() {
        let raw = r#"{"benches":[{"name":"A1_raw_put","keys_per_s":12.5}]}"#;
        let m = extract_montanha_metrics(raw);
        assert!((m.get("A1_raw_put").copied().unwrap_or(0.0) - 12.5).abs() < 1e-9);
    }
}
