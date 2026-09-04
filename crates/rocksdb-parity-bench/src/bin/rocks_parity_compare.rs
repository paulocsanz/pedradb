//! Compare rocksdb-compat bench JSON against the real-RocksDB peer JSON.
//!
//! Usage:
//!   cargo run -q --release -p rocksdb-parity-bench --bin rocks-parity-compare -- \
//!     <compat_bench.json> [out_dir] [peer_bench.json]
//!
//! Peer: 3rd arg or ROCKS_PARITY_PEER=path/to/rocks_parity_bench.json (real
//!   engine run, scripts/rocks_side_ycsb.sh). Gate: ROCKS_PARITY_RATIO_FLOOR=0.8 fails any
//!   real ratio below it ("none"/unset = report-only). Without a peer the
//!   report is template mode (parity.pass null) — CI stays green.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let compat_json = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "findings/rocks-parity-local/compat/rocks_parity_bench.json".into());
    let out = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            PathBuf::from(format!("findings/rocks-parity-compare-{ts}"))
        });
    std::fs::create_dir_all(&out).expect("mkdir");

    let compat_raw = std::fs::read_to_string(&compat_json).unwrap_or_else(|_| "{}".into());
    let compat_metrics = extract_metrics(&compat_raw);
    let compat_sync = extract_bool_field(&compat_raw, "sync");
    let compat_durability = extract_string_field(&compat_raw, "durability");

    let peer_path = std::env::args()
        .nth(3)
        .or_else(|| std::env::var("ROCKS_PARITY_PEER").ok());
    let peer_raw = peer_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let peer_metrics = peer_raw
        .as_ref()
        .map(|s| extract_metrics(s))
        .unwrap_or_default();
    let peer_sync = peer_raw
        .as_deref()
        .and_then(|s| extract_bool_field(s, "sync"));
    let peer_durability = peer_raw
        .as_deref()
        .and_then(|s| extract_string_field(s, "durability"));

    // Official cartaz: Rocks default (sync=false). A blanket sync=true peer
    // is not a win — unless this run is host-default (MyRocks commit-sync /
    // Surreal sync=every), which is the upper DB's own factory setting.
    let peer_policy = peer_raw
        .as_deref()
        .and_then(|s| extract_string_field(s, "peer_policy"));
    let host_default = peer_policy.as_deref() == Some("host-default");
    if peer_sync == Some(true)
        && !host_default
        && std::env::var("ROCKS_PARITY_ALLOW_SYNC_PEER")
            .ok()
            .as_deref()
            != Some("1")
    {
        eprintln!(
            "rocks-parity-compare: peer has sync=true — that is NOT the official Rocks default. \
             Re-run the rocks side with ROCKS_PARITY_SYNC=0. \
             Host-default MyRocks/Surreal sets peer_policy=host-default. \
             Override only with ROCKS_PARITY_ALLOW_SYNC_PEER=1."
        );
        std::process::exit(2);
    }

    // Surface the peer's own status so a stub ("unavailable") is visible in
    // the report instead of reading as a real run.
    let peer_status = match (&peer_raw, peer_path.as_ref()) {
        (Some(raw), Some(_)) => {
            let s = extract_string_field(raw, "status").unwrap_or_else(|| "?".into());
            format!("peer_file:{s}")
        }
        (None, Some(p)) => format!("peer_file_unreadable:{p}"),
        (None, None) => "unavailable".to_string(),
        (Some(_), None) => unreachable!(),
    };

    // Ratio table: compat / rocksdb when both present; else null.
    // Union of both suites — rows absent from a report stay null.
    // RFC-0043: COMPARE_SHAPES only grows. Never delete a shape to lift min_ratio.
    // High-level gate set (ROCKS_PARITY_GATE_SHAPES) is a subset; canaries stay.
    let shapes = rocksdb_parity_bench::COMPARE_SHAPES;
    let parity_floor: Option<f64> = std::env::var("ROCKS_PARITY_RATIO_FLOOR")
        .ok()
        .filter(|s| s != "none")
        .and_then(|s| s.parse().ok());
    // Optional subset the gate looks at (csv). Default = every shape with a ratio.
    // RFC-0031: write shapes already meet 2× same-class; read/iter wait on P1.
    let gate_only: Option<Vec<String>> = std::env::var("ROCKS_PARITY_GATE_SHAPES")
        .ok()
        .filter(|s| !s.is_empty() && s != "all")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        });
    let gated = |name: &str| match &gate_only {
        None => true,
        Some(list) => list.iter().any(|s| s == name),
    };
    let mut real_ratios: Vec<f64> = Vec::new();
    let mut gated_ratios: Vec<f64> = Vec::new();
    let mut ratios = String::from("[\n");
    for (i, shape) in shapes.iter().enumerate() {
        let c_kps = compat_metrics.get(*shape).copied();
        let r_kps = peer_metrics.get(*shape).copied();
        let (ratio, ratio_v) = match (c_kps, r_kps) {
            (Some(a), Some(b)) if b > 0.0 => {
                let v = a / b;
                real_ratios.push(v);
                if gated(shape) {
                    gated_ratios.push(v);
                }
                (format!("{v:.3}"), Some(v))
            }
            _ => ("null".into(), None),
        };
        let meets_floor = match (parity_floor, ratio_v) {
            (Some(floor), Some(v)) => format!("{}", v >= floor),
            _ => "null".into(),
        };
        let c_s = c_kps
            .map(|k| format!("{k:.3}"))
            .unwrap_or_else(|| "null".into());
        let r_s = r_kps
            .map(|k| format!("{k:.3}"))
            .unwrap_or_else(|| "null".into());
        if i > 0 {
            ratios.push_str(",\n");
        }
        ratios.push_str(&format!(
            r#"    {{"shape":"{shape}","compat_keys_per_s":{c_s},"rocksdb_keys_per_s":{r_s},"compat_over_rocksdb":{ratio},"meets_floor":{meets_floor}}}"#
        ));
    }
    ratios.push_str("\n  ]");

    let anomalies = rocksdb_parity_bench::peer_anomalies(&peer_metrics);
    let anomalies_json = if anomalies.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            anomalies
                .iter()
                .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if !anomalies.is_empty() {
        for a in &anomalies {
            eprintln!("rocks-parity-compare: peer anomaly: {a}");
        }
    }

    // Parity summary: only meaningful when a real peer produced ratios.
    let shapes_with_peer = real_ratios.len();
    let parity = if let Some(floor) = parity_floor {
        if gated_ratios.is_empty() {
            format!(
                r#"{{"floor": {floor}, "shapes_with_peer": {shapes_with_peer}, "gated": 0, "min_ratio": null, "pass": null, "note": "floor set but no gated peer ratios — template mode"}}"#
            )
        } else {
            let min_r = gated_ratios.iter().cloned().fold(f64::INFINITY, f64::min);
            let pass = gated_ratios.iter().all(|v| *v >= floor);
            format!(
                r#"{{"floor": {floor}, "shapes_with_peer": {shapes_with_peer}, "gated": {}, "min_ratio": {min_r:.3}, "pass": {pass}}}"#,
                gated_ratios.len()
            )
        }
    } else {
        format!(
            r#"{{"floor": null, "shapes_with_peer": {shapes_with_peer}, "min_ratio": null, "pass": null, "note": "set ROCKS_PARITY_RATIO_FLOOR to gate"}}"#
        )
    };

    let template = peer_template(&compat_metrics);
    let peer_json = peer_path
        .as_ref()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .unwrap_or_else(|| "null".into());
    let sync_opt = |v: Option<bool>| -> String {
        match v {
            Some(true) => "true".into(),
            Some(false) => "false".into(),
            None => "null".into(),
        }
    };
    let cs = sync_opt(compat_sync);
    let rs = sync_opt(peer_sync);
    // Honesty must mirror the column actually measured. The official
    // scoreboard column is G1 (fdatasync before Ok) vs Rocks default;
    // `PEDRA_PARITY_ASYNC=1` legs are the same-class async column (no
    // fdatasync) and must never read as the product's durability claim.
    let honesty = match compat_sync {
        Some(false) => concat!(
            "Single-node lab bench: rocksdb-compat async same-class column ",
            "(PEDRA_PARITY_ASYNC=1; WAL write, NO fdatasync) vs real RocksDB default ",
            "(WriteOptions.sync=false). Same durability class on both sides. ",
            "This column measures engine speed only and is NEVER quoted as ",
            "'we beat Rocks'. The product claim (more durability AND faster) ",
            "is the G1 column (set_sync(true)) vs Rocks default. ",
            "Not a distributed/field claim."
        ),
        _ => concat!(
            "Single-node lab bench: rocksdb-compat (Pedra, fdatasync before Ok, G1) ",
            "vs real RocksDB default (WriteOptions.sync=false). Official peer is Rocks ",
            "**default**, not a matched-sync peer. Pedra keeps the stronger durability ",
            "and still has to beat default Rocks. Not a distributed/field claim."
        ),
    };
    let report = format!(
        r#"{{
  "compare": "rocks-parity-v1",
  "compat_path": {cj:?},
  "compat": {{
    "engine": "compat (rocksdb-compat on pedradb-core, single node, single client)",
    "sync": {cs},
    "durability": {cd},
    "metrics": {compat_metrics_json}
  }},
  "rocksdb": {{
    "status": "{peer_status}",
    "peer_file": {peer_json},
    "sync": {rs},
    "durability": {pd},
    "how_to_fill": "1) cargo run -q --release -p rocksdb-parity-bench --features real --bin rocks-parity-bench -- <out> rocksdb  2) ROCKS_PARITY_PEER=<out>/rocks_parity_bench.json rocks-parity-compare  3) scripts/rocksdb_parity_v0.sh does both. See docs/rocksdb-compat.md"
  }},
  "ratios": {ratios},
  "peer_anomalies": {anomalies_json},
  "parity": {parity},
  "honesty": "{honesty}"
}}
"#,
        cj = compat_json,
        cd = compat_durability
            .map(|s| format!("\"{s}\""))
            .unwrap_or_else(|| "null".into()),
        pd = peer_durability
            .map(|s| format!("\"{s}\""))
            .unwrap_or_else(|| "null".into()),
        compat_metrics_json = metrics_to_json(&compat_metrics),
    );

    let path = out.join("compare_report.json");
    std::fs::write(&path, &report).expect("write compare");
    let tmpl = out.join("rocks_shaped_peer.template.json");
    std::fs::write(&tmpl, &template).expect("write template");
    println!("{report}");
    eprintln!("wrote {}", path.display());
    eprintln!(
        "wrote {} (fill keys_per_s then set ROCKS_PARITY_PEER)",
        tmpl.display()
    );
    // Lab parity gate: floor set + real peer + any ratio below floor → nonzero.
    if let Some(floor) = parity_floor {
        if !gated_ratios.is_empty() && !gated_ratios.iter().all(|v| *v >= floor) {
            eprintln!(
                "parity gate FAILED: floor={floor} min_ratio={:.3} gated={}",
                gated_ratios.iter().cloned().fold(f64::INFINITY, f64::min),
                gated_ratios.len()
            );
            std::process::exit(2);
        }
    }
    if !anomalies.is_empty()
        && std::env::var("ROCKS_PARITY_FAIL_PEER_ANOMALY").as_deref() == Ok("1")
    {
        eprintln!("parity gate FAILED: peer_anomalies={}", anomalies.len());
        std::process::exit(2);
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
    let rest = raw[i + key.len()..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Best-effort extract name → qps (or keys_per_s) from a bench JSON.
fn extract_metrics(raw: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for chunk in raw.split("\"name\"") {
        let Some(name) = json_string_after(chunk, ':') else {
            continue;
        };
        let kps = json_number_field(chunk, "qps")
            .or_else(|| json_number_field(chunk, "keys_per_s"))
            .or_else(|| {
                json_number_field(chunk, "n")
                    .and_then(|n| json_number_field(chunk, "wall_s").map(|w| n / w.max(1e-12)))
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

fn peer_template(compat: &BTreeMap<String, f64>) -> String {
    let mut s = String::from("{\n  \"bench\": \"rocks-side-peer-template\",\n  \"engine\": \"rocksdb\",\n  \"sync\": false,\n  \"durability\": \"async-wal (WriteOptions.sync=false, rocksdb default)\",\n  \"benches\": [\n");
    for (i, (name, ckps)) in compat.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        s.push_str(&format!(
            r#"    {{
      "name": "{name}",
      "qps": null,
      "compat_qps_ref": {ckps:.3},
      "note": "fill qps from real RocksDB same shape"
    }}"#
        ));
    }
    if compat.is_empty() {
        s.push_str(
            r#"    {
      "name": "ycsb_a",
      "qps": null,
      "note": "fill after compat bench"
    }"#,
        );
    }
    s.push_str("\n  ]\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_sync_and_durability() {
        let raw = r#"{"engine":"compat","sync":true,"durability":"fsync-before-ok (pedradb-core WAL)","benches":[]}"#;
        assert_eq!(extract_bool_field(raw, "sync"), Some(true));
        assert_eq!(
            extract_string_field(raw, "durability").as_deref(),
            Some("fsync-before-ok (pedradb-core WAL)")
        );
        assert_eq!(extract_bool_field("{}", "sync"), None);
    }

    #[test]
    fn extract_metrics_prefers_qps() {
        let raw = r#"{"benches":[{"name":"ycsb_a","qps":12.5,"keys_per_s":0.0}]}"#;
        let m = extract_metrics(raw);
        assert!((m.get("ycsb_a").copied().unwrap_or(0.0) - 12.5).abs() < 1e-9);
    }

    #[test]
    fn peer_status_from_stub() {
        let stub = r#"{"status":"unavailable","benches":[{"name":"ycsb_a","qps":null}]}"#;
        assert_eq!(
            extract_string_field(stub, "status").as_deref(),
            Some("unavailable")
        );
    }

    #[test]
    fn peer_anomalies_flag_incoherent_rmw() {
        let mut peer = std::collections::BTreeMap::new();
        peer.insert("surreal_tx_put".into(), 3072.0);
        peer.insert("surreal_tx_rmw".into(), 5009.0);
        let a = rocksdb_parity_bench::peer_anomalies(&peer);
        assert_eq!(a.len(), 1);
        assert!(a[0].contains("rmw ⊃ put"));
    }
}
