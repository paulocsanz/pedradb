//! RFC-0185 column A drop-in gate.
//!
//! G_A is the 22-shape must-win set. Pass = **min of 3 quiet rounds > 1.0**
//! on every G_A name — not a median that hides a round ≤1.0. Catalog
//! [`crate::COMPARE_SHAPES`] only grows; this set is a subset.

use std::collections::BTreeMap;

/// RFC-0185 G_A — column A must-win (official 16 ∪ P04 17 1c extras).
pub const COLUMN_A_SHAPES: &[&str] = &[
    "ycsb_a",
    "ycsb_b",
    "ycsb_c",
    "ycsb_d",
    "ycsb_e",
    "ycsb_f",
    "deps_apply_batch",
    "deps_mvcc_latest",
    "deps_scan",
    "deps_raftlog",
    "deps_cache_overwrite",
    "deps_lock_prewrite",
    "ycsb_a_mc4",
    "ycsb_f_mc4",
    "deps_cache_overwrite_mc4",
    "deps_apply_batch_mc4",
    "deps_raftlog_mc4",
    "kvrocks_get",
    "kvrocks_set",
    "kvrocks_scan",
    "kvrocks_pipelined_set",
    "kvrocks_blob_set",
];

/// Strict floor: `min(r1,r2,r3) > 1.0` (not `>=`, not median).
pub const COLUMN_A_FLOOR: f64 = 1.0;

/// Quiet-host Rocks overwrite_mc4 band (RFC-0185). Below this is not
/// automatically collapsed — collapsed is the documented 90–157 k band.
pub const OVERWRITE_MC4_QUIET_ROCKS_QPS: f64 = 260_000.0;

/// Upper bound of the documented collapsed Rocks overwrite_mc4 band
/// (90–157 k). A peer at or below this is not a Pedra win.
pub const OVERWRITE_MC4_COLLAPSED_ROCKS_QPS: f64 = 157_000.0;

pub const OVERWRITE_MC4_SHAPE: &str = "deps_cache_overwrite_mc4";

/// One G_A gate result. `pass` is false if any G_A shape is missing, any
/// per-shape min ≤ 1.0, or the Rocks overwrite_mc4 peer is collapsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnAVerdict {
    pub pass: bool,
    pub min_ratio: Option<f64>,
    /// `(shape, min_of_rounds)` — 0.0 means missing in at least one round.
    pub fails: Vec<(String, f64)>,
    pub anomalies: Vec<String>,
    pub gated: usize,
}

impl ColumnAVerdict {
    pub fn fail_text(&self) -> String {
        if self.pass {
            return format!(
                "COLUMN_A_PASS min_ratio={:.3} gated={}",
                self.min_ratio.unwrap_or(f64::NAN),
                self.gated
            );
        }
        let fail_shapes: Vec<String> = self
            .fails
            .iter()
            .map(|(s, v)| format!("{s}={v:.3}"))
            .collect();
        format!(
            "COLUMN_A_FAIL min_ratio={} fail={} anomalies={}",
            self.min_ratio
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "null".into()),
            fail_shapes.join(","),
            self.anomalies.len()
        )
    }
}

/// `ROCKS_PARITY_COLUMN_A=1` or `ROCKS_PARITY_GATE_SHAPES=column_a`.
#[must_use]
pub fn column_a_gate_requested() -> bool {
    column_a_gate_requested_from(
        std::env::var("ROCKS_PARITY_COLUMN_A").ok().as_deref(),
        std::env::var("ROCKS_PARITY_GATE_SHAPES").ok().as_deref(),
    )
}

#[must_use]
pub fn column_a_gate_requested_from(column_a: Option<&str>, gate_shapes: Option<&str>) -> bool {
    column_a == Some("1") || gate_shapes == Some("column_a")
}

/// Ratio beats the column-A floor (`> 1.0`, not `>=`).
#[must_use]
pub fn column_a_ratio_passes(ratio: f64) -> bool {
    ratio.is_finite() && ratio > COLUMN_A_FLOOR
}

/// Rocks overwrite_mc4 in the documented collapsed band (not a Pedra win).
#[must_use]
pub fn overwrite_mc4_peer_collapsed(qps: f64) -> bool {
    qps > 0.0 && qps <= OVERWRITE_MC4_COLLAPSED_ROCKS_QPS
}

/// Anomaly string for a collapsed overwrite_mc4 peer (same text as
/// [`crate::peer_anomalies`]).
#[must_use]
pub fn overwrite_mc4_collapsed_anomaly(qps: f64) -> String {
    format!(
        "deps_cache_overwrite_mc4 peer {qps:.0} qps collapsed (≤157k; quiet band ≳260k; not a Pedra win)"
    )
}

/// Per-shape min across `rounds`. `None` = shape missing in at least one
/// round (unpaid — cannot claim 3/3).
#[must_use]
pub fn column_a_shape_mins(rounds: &[BTreeMap<String, f64>]) -> BTreeMap<String, Option<f64>> {
    let mut out = BTreeMap::new();
    for shape in COLUMN_A_SHAPES {
        let mut min_v: Option<f64> = None;
        let mut n = 0usize;
        for r in rounds {
            if let Some(&v) = r.get(*shape) {
                if v.is_finite() && v > 0.0 {
                    n += 1;
                    min_v = Some(min_v.map_or(v, |m| m.min(v)));
                }
            }
        }
        out.insert(
            (*shape).to_string(),
            if n == rounds.len() && n > 0 {
                min_v
            } else {
                None
            },
        );
    }
    out
}

fn verdict_from_mins(
    mins: &BTreeMap<String, Option<f64>>,
    anomalies: Vec<String>,
) -> ColumnAVerdict {
    let mut fails = Vec::new();
    let mut present = Vec::new();
    for shape in COLUMN_A_SHAPES {
        match mins.get(*shape).copied().flatten() {
            Some(v) if column_a_ratio_passes(v) => present.push(v),
            Some(v) => fails.push(((*shape).to_string(), v)),
            None => fails.push(((*shape).to_string(), 0.0)),
        }
    }
    let min_ratio = present
        .iter()
        .copied()
        .chain(fails.iter().filter(|(_, v)| *v > 0.0).map(|(_, v)| *v))
        .fold(None, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |m| m.min(v)))
        });
    let pass = fails.is_empty() && anomalies.is_empty() && present.len() == COLUMN_A_SHAPES.len();
    ColumnAVerdict {
        pass,
        min_ratio,
        fails,
        anomalies,
        gated: COLUMN_A_SHAPES.len(),
    }
}

fn collapsed_anomalies(peer_overwrite_qps: &[Option<f64>]) -> Vec<String> {
    let mut out = Vec::new();
    for qps in peer_overwrite_qps {
        if let Some(q) = *qps {
            if overwrite_mc4_peer_collapsed(q) {
                let a = overwrite_mc4_collapsed_anomaly(q);
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
    }
    out
}

/// One-round G_A check (compare binary). Every G_A shape must be present
/// and `> 1.0`. Collapsed Rocks overwrite_mc4 is an anomaly, not a win.
#[must_use]
pub fn column_a_one_round(
    ratios: &BTreeMap<String, f64>,
    peer_overwrite_mc4_qps: Option<f64>,
) -> ColumnAVerdict {
    let mins = column_a_shape_mins(std::slice::from_ref(ratios));
    verdict_from_mins(&mins, collapsed_anomalies(&[peer_overwrite_mc4_qps]))
}

/// RFC-0185 cartaz: min of exactly 3 rounds > 1.0 on every G_A shape.
/// Median-hiding (1.002 / 0.816 / 1.032) fails.
#[must_use]
pub fn column_a_three_round_min(
    rounds: &[BTreeMap<String, f64>; 3],
    peer_overwrite_mc4_qps: [Option<f64>; 3],
) -> ColumnAVerdict {
    let mins = column_a_shape_mins(rounds);
    verdict_from_mins(&mins, collapsed_anomalies(&peer_overwrite_mc4_qps))
}

/// Slice form used by the aggregator bin (must be exactly 3).
#[must_use]
pub fn column_a_three_round_min_slice(
    rounds: &[BTreeMap<String, f64>],
    peer_overwrite_mc4_qps: &[Option<f64>],
) -> ColumnAVerdict {
    if rounds.len() != 3 {
        return ColumnAVerdict {
            pass: false,
            min_ratio: None,
            fails: vec![("rounds".into(), rounds.len() as f64)],
            anomalies: vec![format!("need 3 rounds, got {}", rounds.len())],
            gated: COLUMN_A_SHAPES.len(),
        };
    }
    let mut arr: [BTreeMap<String, f64>; 3] = [BTreeMap::new(), BTreeMap::new(), BTreeMap::new()];
    for (i, r) in rounds.iter().enumerate() {
        arr[i] = r.clone();
    }
    let mut qps = [None; 3];
    for (i, q) in peer_overwrite_mc4_qps.iter().take(3).enumerate() {
        qps[i] = *q;
    }
    column_a_three_round_min(&arr, qps)
}

/// Best-effort `name` → qps from a bench JSON (`qps` / `keys_per_s` / n/wall).
#[must_use]
pub fn extract_bench_qps(raw: &str) -> BTreeMap<String, f64> {
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
            if v.is_finite() && v > 0.0 {
                out.insert(name, v);
            }
        }
    }
    out
}

/// Ratios from a `rocks-parity-compare` report (`compat_over_rocksdb`).
#[must_use]
pub fn extract_compare_ratios(raw: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for chunk in raw.split("\"shape\"") {
        let Some(name) = json_string_after(chunk, ':') else {
            continue;
        };
        if let Some(v) = json_number_field(chunk, "compat_over_rocksdb") {
            if v.is_finite() && v > 0.0 {
                out.insert(name, v);
            }
        }
    }
    out
}

/// Peer overwrite_mc4 qps from a compare report (`rocksdb_keys_per_s` on
/// that shape row).
#[must_use]
pub fn extract_compare_peer_overwrite_qps(raw: &str) -> Option<f64> {
    for chunk in raw.split("\"shape\"") {
        let Some(name) = json_string_after(chunk, ':') else {
            continue;
        };
        if name != OVERWRITE_MC4_SHAPE {
            continue;
        }
        return json_number_field(chunk, "rocksdb_keys_per_s");
    }
    None
}

/// compat/rocks qps → per-shape ratio (rocks > 0).
#[must_use]
pub fn ratios_from_qps(
    compat: &BTreeMap<String, f64>,
    peer: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for (name, &c) in compat {
        if let Some(&r) = peer.get(name) {
            if r > 0.0 && c.is_finite() {
                out.insert(name.clone(), c / r);
            }
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
    if rest.starts_with("null") {
        return None;
    }
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == 'e' || *c == 'E')
        .collect();
    num.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_ga(ratio: f64) -> BTreeMap<String, f64> {
        COLUMN_A_SHAPES
            .iter()
            .map(|s| ((*s).to_string(), ratio))
            .collect()
    }

    fn with_overwrite(mut m: BTreeMap<String, f64>, v: f64) -> BTreeMap<String, f64> {
        m.insert(OVERWRITE_MC4_SHAPE.into(), v);
        m
    }

    #[test]
    fn rfc0185_column_a_shapes_are_the_gate() {
        assert_eq!(COLUMN_A_SHAPES.len(), 22);
        assert_eq!(
            COLUMN_A_SHAPES,
            &[
                "ycsb_a",
                "ycsb_b",
                "ycsb_c",
                "ycsb_d",
                "ycsb_e",
                "ycsb_f",
                "deps_apply_batch",
                "deps_mvcc_latest",
                "deps_scan",
                "deps_raftlog",
                "deps_cache_overwrite",
                "deps_lock_prewrite",
                "ycsb_a_mc4",
                "ycsb_f_mc4",
                "deps_cache_overwrite_mc4",
                "deps_apply_batch_mc4",
                "deps_raftlog_mc4",
                "kvrocks_get",
                "kvrocks_set",
                "kvrocks_scan",
                "kvrocks_pipelined_set",
                "kvrocks_blob_set",
            ]
        );
        for name in COLUMN_A_SHAPES {
            assert!(
                crate::COMPARE_SHAPES.contains(name),
                "G_A {name} must stay a subset of COMPARE_SHAPES (catalog only grows)"
            );
        }
        assert_eq!(
            &crate::COMPARE_SHAPES[..crate::OFFICIAL_16],
            &[
                "ycsb_a",
                "ycsb_b",
                "ycsb_c",
                "ycsb_d",
                "ycsb_e",
                "ycsb_f",
                "deps_apply_batch",
                "deps_mvcc_latest",
                "deps_scan",
                "deps_raftlog",
                "deps_cache_overwrite",
                "ycsb_a_mc4",
                "ycsb_f_mc4",
                "deps_cache_overwrite_mc4",
                "deps_apply_batch_mc4",
                "deps_raftlog_mc4",
            ]
        );
    }

    #[test]
    fn median_hide_1_002_0_816_1_032_fails() {
        // RFC-0180 P0.9 Darwin: median 1.002 with named loss 0.816.
        let r1 = with_overwrite(full_ga(1.2), 1.002);
        let r2 = with_overwrite(full_ga(1.2), 0.816);
        let r3 = with_overwrite(full_ga(1.2), 1.032);
        let v = column_a_three_round_min(&[r1, r2, r3], [Some(270_000.0); 3]);
        assert!(!v.pass, "{}", v.fail_text());
        assert!(
            v.fails
                .iter()
                .any(|(s, r)| s == OVERWRITE_MC4_SHAPE && (*r - 0.816).abs() < 1e-9),
            "{:?}",
            v.fails
        );
        let median = {
            let mut xs = [1.002_f64, 0.816, 1.032];
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            xs[1]
        };
        assert!(
            (median - 1.002).abs() < 1e-9,
            "fixture is the median-hide case"
        );
        assert!(median > COLUMN_A_FLOOR);
    }

    #[test]
    fn all_three_above_one_passes() {
        let r1 = full_ga(1.10);
        let r2 = full_ga(1.05);
        let r3 = full_ga(1.20);
        let v = column_a_three_round_min(&[r1, r2, r3], [Some(270_000.0); 3]);
        assert!(v.pass, "{}", v.fail_text());
        assert!((v.min_ratio.unwrap() - 1.05).abs() < 1e-9);
        assert!(v.fails.is_empty());
    }

    #[test]
    fn exactly_one_is_not_a_pass() {
        let v = column_a_one_round(&full_ga(1.0), Some(270_000.0));
        assert!(!v.pass, "floor is strict >: 1.0 must fail");
    }

    #[test]
    fn missing_ga_shape_fails() {
        let mut r = full_ga(1.2);
        r.remove("deps_raftlog");
        let v = column_a_one_round(&r, Some(270_000.0));
        assert!(!v.pass);
        assert!(
            v.fails
                .iter()
                .any(|(s, r)| s == "deps_raftlog" && *r == 0.0),
            "{:?}",
            v.fails
        );
    }

    #[test]
    fn collapsed_overwrite_peer_is_anomaly_not_win() {
        let v = column_a_one_round(&full_ga(1.2), Some(120_000.0));
        assert!(!v.pass, "{}", v.fail_text());
        assert!(
            v.anomalies.iter().any(|a| a.contains("collapsed")),
            "{:?}",
            v.anomalies
        );
    }

    #[test]
    fn quiet_overwrite_peer_is_not_collapsed() {
        assert!(!overwrite_mc4_peer_collapsed(270_000.0));
        assert!(overwrite_mc4_peer_collapsed(157_000.0));
        assert!(overwrite_mc4_peer_collapsed(90_000.0));
        assert!(!overwrite_mc4_peer_collapsed(0.0));
    }

    #[test]
    fn extract_compare_ratios_skips_null() {
        let raw = r#"{"ratios":[
            {"shape":"ycsb_a","compat_over_rocksdb":1.25,"rocksdb_keys_per_s":400000},
            {"shape":"ycsb_b","compat_over_rocksdb":null}
        ]}"#;
        let m = extract_compare_ratios(raw);
        assert!((m.get("ycsb_a").copied().unwrap() - 1.25).abs() < 1e-9);
        assert!(!m.contains_key("ycsb_b"));
        assert_eq!(extract_compare_peer_overwrite_qps(raw), None);
        let ow = r#"{"ratios":[{"shape":"deps_cache_overwrite_mc4","compat_over_rocksdb":0.55,"rocksdb_keys_per_s":120000}]}"#;
        assert!((extract_compare_peer_overwrite_qps(ow).unwrap() - 120_000.0).abs() < 1e-6);
    }

    #[test]
    fn gate_env_column_a() {
        assert!(column_a_gate_requested_from(Some("1"), None));
        assert!(column_a_gate_requested_from(None, Some("column_a")));
        assert!(!column_a_gate_requested_from(None, Some("ycsb_a,ycsb_b")));
        assert!(!column_a_gate_requested_from(None, None));
    }
}
