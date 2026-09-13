//! RFC-0185 3-round column-A aggregator.
//!
//! Usage:
//!   rocks-parity-column-a <compare_r1.json> <compare_r2.json> <compare_r3.json>
//!
//! Each file is a `rocks-parity-compare` report. Pass = min of the three
//! rounds > 1.0 on every G_A shape (not median). Exit 2 on fail.
//!
//! Alternate (bench JSON pairs, 6 files):
//!   rocks-parity-column-a --qps \
//!     compat1.json peer1.json compat2.json peer2.json compat3.json peer3.json

#![forbid(unsafe_code)]

use rocksdb_parity_bench::column_a::{
    column_a_three_round_min_slice, extract_bench_qps, extract_compare_peer_overwrite_qps,
    extract_compare_ratios, ratios_from_qps, ColumnAVerdict,
};
use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (rounds, peer_ow, sources) = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "rocks-parity-column-a: {e}\n\
                 usage: rocks-parity-column-a <compare_r1.json> <compare_r2.json> <compare_r3.json>\n\
                 or:    rocks-parity-column-a --qps c1.json p1.json c2.json p2.json c3.json p3.json"
            );
            std::process::exit(2);
        }
    };
    let v = column_a_three_round_min_slice(&rounds, &peer_ow);
    print_verdict(&v, &sources);
    if !v.pass {
        eprintln!("parity gate FAILED: {}", v.fail_text());
        std::process::exit(2);
    }
}

fn parse_args(
    args: &[String],
) -> Result<(Vec<BTreeMap<String, f64>>, Vec<Option<f64>>, Vec<String>), String> {
    if args.first().map(String::as_str) == Some("--qps") {
        let files = &args[1..];
        if files.len() != 6 {
            return Err(format!("--qps needs 6 JSON files, got {}", files.len()));
        }
        let mut rounds = Vec::new();
        let mut peer_ow = Vec::new();
        let mut sources = Vec::new();
        for pair in files.chunks(2) {
            let c_raw = std::fs::read_to_string(&pair[0])
                .map_err(|e| format!("read {}: {e}", pair[0]))?;
            let p_raw = std::fs::read_to_string(&pair[1])
                .map_err(|e| format!("read {}: {e}", pair[1]))?;
            let compat = extract_bench_qps(&c_raw);
            let peer = extract_bench_qps(&p_raw);
            peer_ow.push(peer.get("deps_cache_overwrite_mc4").copied());
            rounds.push(ratios_from_qps(&compat, &peer));
            sources.push(format!("{} vs {}", pair[0], pair[1]));
        }
        return Ok((rounds, peer_ow, sources));
    }
    if args.len() != 3 {
        return Err(format!("need 3 compare reports, got {}", args.len()));
    }
    let mut rounds = Vec::new();
    let mut peer_ow = Vec::new();
    let mut sources = Vec::new();
    for p in args {
        if !Path::new(p).is_file() {
            return Err(format!("not a file: {p}"));
        }
        let raw = std::fs::read_to_string(p).map_err(|e| format!("read {p}: {e}"))?;
        rounds.push(extract_compare_ratios(&raw));
        peer_ow.push(extract_compare_peer_overwrite_qps(&raw));
        sources.push(p.clone());
    }
    Ok((rounds, peer_ow, sources))
}

fn print_verdict(v: &ColumnAVerdict, sources: &[String]) {
    let min_s = v
        .min_ratio
        .map(|m| format!("{m:.3}"))
        .unwrap_or_else(|| "null".into());
    let fail_json = if v.fails.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            v.fails
                .iter()
                .map(|(s, r)| format!("\"{s}={r:.3}\""))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let anom_json = if v.anomalies.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            v.anomalies
                .iter()
                .map(|s| format!("\"{}\"", s.replace('"', "'")))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let src = sources
        .iter()
        .map(|s| format!("\"{}\"", s.replace('"', "'")))
        .collect::<Vec<_>>()
        .join(",");
    let report = format!(
        r#"{{
  "compare": "rfc0185-column-a-3round",
  "rule": "min_of_3_gt_1.0_on_every_G_A",
  "sources": [{src}],
  "parity": {{
    "floor": 1.0,
    "gated": {},
    "min_ratio": {min_s},
    "pass": {},
    "fails": {fail_json},
    "anomalies": {anom_json}
  }}
}}
"#,
        v.gated, v.pass
    );
    print!("{report}");
}
