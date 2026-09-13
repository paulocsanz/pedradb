//! Same-class ratio curve (RFC-0197). Integer nanoseconds, no I/O, no float.
//!
//! Answers "how does the async-vs-async ratio grow with scale" BEFORE the
//! meter: `cycle_hat(cold_permille)` is an integer least-squares fit over
//! **dated** anchor measurements of the write family
//! (`deps_cache_overwrite_mc4`, 4 clients, payload 100, peer RocksDB default
//! `sync=false`), the cold fraction reuses [`crate::scale_kernel`]'s
//! `warm_cap`, and `cut_to_cross` decomposes the per-scale deficit into the
//! disk term (scale-owned) and the hot-base gap (engine-owned, covered by the
//! [`crate::write_cycle_kernel`] slice ranking in the post-0193/post-0190
//! view). The AS-IS twin is the flat curve with no scale term — the exact
//! blindness that let the smoke battery pass 15/15 at 1024 records while the
//! same family lost 0.557× at 25M.
//!
//! Hats are hats: every extrapolated ratio is labeled by the caller as a
//! prediction to be metered, never as a cell number (RFC-0197 Out of scope).

#![forbid(unsafe_code)]

use crate::scale_kernel::{warm_cap_bytes, SCALE_BYTES_PER_ENTRY};
use crate::write_cycle_kernel::{qps_hat_error_permille, WritePhaseNs};

/// One dated same-class measurement at one scale (RFC-0197 Background).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleAnchor {
    /// Dataset records of the cell.
    pub records: u64,
    /// Measured Pedra ops/s (labeled DIAG or cartaz by the caller).
    pub pedra_qps: u64,
    /// Measured RocksDB-default (`sync=false`) ops/s on the same leg.
    pub rocks_qps: u64,
}

/// Write-family ladder, `deps_cache_overwrite_mc4`, 4 clients, payload 100.
/// 100k/2M/15M are Darwin DIAG (otimizar mapa, floor-cut/leftover findings);
/// 25M is the Linux cartaz min-of-3 r3 (2026-09-10,
/// `findings/2026-09-10-take-all-reversal-takecell.md`). Cross-box mix is
/// declared: P1.2 re-fits same-box when the gate opens.
pub const WRITE_FAMILY_ANCHORS_2026_09_10: [ScaleAnchor; 4] = [
    ScaleAnchor {
        records: 100_000,
        pedra_qps: 236_000,
        rocks_qps: 266_000,
    },
    ScaleAnchor {
        records: 2_000_000,
        pedra_qps: 209_000,
        rocks_qps: 263_000,
    },
    ScaleAnchor {
        records: 15_000_000,
        pedra_qps: 174_000,
        rocks_qps: 264_000,
    },
    ScaleAnchor {
        records: 25_000_000,
        pedra_qps: 142_959,
        rocks_qps: 256_780,
    },
];

/// Declared cross-box tolerance of the 4-anchor fit (max residual is 93‰ on
/// the 100k DIAG point). A residual above this means the anchors changed and
/// the fit must be re-run, not clamped.
pub const FIT_TOLERANCE_PERMILLE: i64 = 100;

/// One dated GET-side anchor (RFC-0197 P2.1). Ratio is `rocks_ns / pedra_ns`
/// in permille, so > 1000 means Pedra is faster on that leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GetSideAnchor {
    /// Benchmark leg (`prefix_scan`, `get_hit`, `lookup_100_get_loop`, ...).
    pub leg: &'static str,
    /// Dataset records of the leg.
    pub records: u64,
    /// `rocks_ns / pedra_ns` in permille (rounded).
    pub ratio_permille: u64,
    /// Dated provenance + grade (cartaz / DIAG / contrast).
    pub label: &'static str,
}

/// GET-side anchors registered 2026-09-10 (P2.1). The GET ladder covers 1
/// of the 5 write-family scales (100M) on 2 boxes: on the 4 GiB box the
/// point-gets are paid (1484–1576‰, 3-run min-of-3 medians of the
/// `win-probe-prefix` guest harness, vlen=200, DIAG) while the sequential
/// scan is the 700‰ cartaz; the 1050‰ row is the RAM-fits contrast that
/// attributes that deficit to I/O, not compute (RFC-0195). 10k/2M/15M/25M
/// GET legs are unmeasured same-class — named deferral, re-anchored by the
/// P1.2/P1.3 meters when the gate opens.
pub const GET_SIDE_ANCHORS_2026_09_10: [GetSideAnchor; 6] = [
    GetSideAnchor {
        leg: "prefix_scan",
        records: 100_000_000,
        ratio_permille: 700,
        label: "cartaz 2026-09-10 (rocks-parity-compare, payload 100, 4 GiB)",
    },
    GetSideAnchor {
        leg: "prefix_scan",
        records: 100_000_000,
        ratio_permille: 1050,
        label: "contrast big-guest 2026-09-10 (RAM fits; RFC-0195 decomposição)",
    },
    GetSideAnchor {
        leg: "get_hit",
        records: 100_000_000,
        ratio_permille: 1576,
        label: "DIAG 2026-09-05 (win-probe-prefix 3-run min, vlen=200, 4 GiB guest)",
    },
    GetSideAnchor {
        leg: "lookup_100_get_loop",
        records: 100_000_000,
        ratio_permille: 1484,
        label: "DIAG 2026-09-05 (win-probe-prefix 3-run min, vlen=200, 4 GiB guest)",
    },
    GetSideAnchor {
        leg: "lookup_100_multi_get",
        records: 100_000_000,
        ratio_permille: 1562,
        label: "DIAG 2026-09-05 (win-probe-prefix 3-run min, vlen=200, 4 GiB guest)",
    },
    GetSideAnchor {
        leg: "prefix_scan_vlen200",
        records: 100_000_000,
        ratio_permille: 1309,
        label: "DIAG 2026-09-05 (win-probe-prefix 3-run min, vlen=200, 4 GiB guest)",
    },
];

/// Scales of the write-family ladder that carry at least one dated GET
/// anchor — the ladder-completeness check behind P2.1's deferral.
#[must_use]
pub fn get_side_ladder_scales(anchors: &[GetSideAnchor], ladder: &[u64]) -> Vec<u64> {
    ladder
        .iter()
        .copied()
        .filter(|&s| anchors.iter().any(|a| a.records == s))
        .collect()
}

/// Cold fraction of the store in permille: 0 when everything fits the warm
/// cap, up to 1000 when nothing does. Same cap logic as the GET clock.
#[must_use]
pub fn cold_permille(store_bytes: u64, warm_cap: u64) -> u64 {
    if store_bytes <= warm_cap || store_bytes == 0 {
        return 0;
    }
    ((store_bytes - warm_cap) * 1000 / store_bytes).min(1000)
}

/// Rounded integer division for permille ratios (`(n + d/2) / d`).
#[must_use]
fn permille_div(num: u64, den: u64) -> u64 {
    if den == 0 {
        return 0;
    }
    (num + den / 2) / den
}

/// Integer least-squares fit `cycle_ns = (base_num + slope_num * cold) / den`
/// over the anchors' measured Pedra cycles. Degenerate ladders (single cold
/// value) collapse to a flat fit with slope 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurveFit {
    /// Numerator of the hot-base cycle (ns).
    pub base_num: u64,
    /// Numerator of the per-cold-permille slope (ns).
    pub slope_num: u64,
    /// Common denominator (`n * b_den`).
    pub den: u64,
}

impl CurveFit {
    /// Predicted cycle at a cold fraction. Truncating like the 0192 kernel.
    #[must_use]
    pub fn cycle_hat_ns(&self, cold: u64) -> u64 {
        if self.den == 0 {
            return 0;
        }
        (self.base_num + self.slope_num.saturating_mul(cold.min(1000))) / self.den
    }

    /// AS-IS: the flat curve — the scale term does not exist, so every scale
    /// predicts the hot ratio. This is the smoke-battery blindness.
    #[must_use]
    pub fn cycle_hat_as_is_ns(&self, _cold: u64) -> u64 {
        self.cycle_hat_ns(0)
    }
}

/// Cycle in ns from a measured ops/s (truncating, like `qps_from_cycle_ns`).
#[must_use]
fn cycle_from_qps(qps: u64) -> u64 {
    if qps == 0 {
        0
    } else {
        1_000_000_000 / qps
    }
}

/// Fit the write-family curve from dated anchors.
#[must_use]
pub fn fit_write_family_curve(
    anchors: &[ScaleAnchor],
    bytes_per_entry: u64,
    warm_cap: u64,
) -> CurveFit {
    let n = anchors.len() as u64;
    if n == 0 {
        return CurveFit {
            base_num: 0,
            slope_num: 0,
            den: 1,
        };
    }
    let mut sc = 0u64;
    let mut sy = 0u64;
    let mut sxy = 0u64;
    let mut sxx = 0u64;
    for a in anchors {
        let cold = cold_permille(a.records.saturating_mul(bytes_per_entry), warm_cap);
        let cycle = cycle_from_qps(a.pedra_qps);
        sc = sc.saturating_add(cold);
        sy = sy.saturating_add(cycle);
        sxy = sxy.saturating_add(cold.saturating_mul(cycle));
        sxx = sxx.saturating_add(cold.saturating_mul(cold));
    }
    let b_num = n.saturating_mul(sxy).saturating_sub(sc.saturating_mul(sy));
    let b_den = n.saturating_mul(sxx).saturating_sub(sc.saturating_mul(sc));
    if b_den == 0 {
        // Flat ladder: no scale information — slope 0, base = mean cycle.
        return CurveFit {
            base_num: sy,
            slope_num: 0,
            den: n,
        };
    }
    let base_num = sy
        .saturating_mul(b_den)
        .saturating_sub(b_num.saturating_mul(sc));
    let den = n.saturating_mul(b_den);
    CurveFit {
        base_num,
        slope_num: b_num.saturating_mul(n),
        den,
    }
}

/// Rocks cycle modeled as one dated flat constant: the mean of the measured
/// per-op cycles (3 759–3 894 ns across 100k–25M). Refutable at the 100M
/// meter — the Rocks kernel-page LRU may curve.
#[must_use]
pub fn rocks_cycle_ns(anchors: &[ScaleAnchor]) -> u64 {
    let n = anchors.len() as u64;
    if n == 0 {
        return 0;
    }
    anchors
        .iter()
        .map(|a| cycle_from_qps(a.rocks_qps))
        .sum::<u64>()
        / n
}

/// Spread of the measured Rocks cycles (max − min), ns.
#[must_use]
pub fn rocks_cycle_spread_ns(anchors: &[ScaleAnchor]) -> u64 {
    let mut it = anchors.iter().map(|a| cycle_from_qps(a.rocks_qps));
    let Some(first) = it.next() else {
        return 0;
    };
    let (mut min, mut max) = (first, first);
    for c in it {
        if c < min {
            min = c;
        }
        if c > max {
            max = c;
        }
    }
    max - min
}

/// Measured leg of one scale, with the model error on the Pedra side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeasuredPoint {
    pub pedra_qps: u64,
    pub rocks_qps: u64,
    /// `pedra_qps / rocks_qps` in permille (rounded).
    pub ratio_permille: u64,
    /// `qps_hat_error_permille(qps_hat, pedra_qps)` — named, never clamped.
    pub error_permille: Option<i64>,
}

/// One scale of the curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RatioPoint {
    pub records: u64,
    pub store_bytes: u64,
    pub cold_permille: u64,
    pub cycle_hat_ns: u64,
    pub qps_hat: u64,
    /// `rocks_cycle / cycle_hat` in permille (rounded). A prediction.
    pub ratio_hat_permille: u64,
    /// The dated measurement when this scale is an anchor; `None` means the
    /// row is pure extrapolation and must be labeled hat.
    pub measured: Option<MeasuredPoint>,
}

/// Predicted ratio in permille of a cycle against the Rocks constant.
#[must_use]
pub fn ratio_hat_permille(cycle_hat: u64, rocks_cycle: u64) -> u64 {
    if cycle_hat == 0 {
        return 0;
    }
    permille_div(rocks_cycle * 1000, cycle_hat)
}

/// Build the curve over `scales` (records), attaching the dated anchor legs.
#[must_use]
pub fn ratio_curve(
    fit: CurveFit,
    rocks_cycle: u64,
    anchors: &[ScaleAnchor],
    scales: &[u64],
    bytes_per_entry: u64,
    warm_cap: u64,
) -> Vec<RatioPoint> {
    scales
        .iter()
        .map(|&records| {
            let store = records.saturating_mul(bytes_per_entry);
            let cold = cold_permille(store, warm_cap);
            let cycle = fit.cycle_hat_ns(cold);
            let qps = crate::write_cycle_kernel::qps_from_cycle_ns(cycle);
            let measured = anchors.iter().find(|a| a.records == records).map(|a| {
                let error = qps_hat_error_permille(qps, a.pedra_qps);
                MeasuredPoint {
                    pedra_qps: a.pedra_qps,
                    rocks_qps: a.rocks_qps,
                    ratio_permille: permille_div(a.pedra_qps * 1000, a.rocks_qps),
                    error_permille: error,
                }
            });
            RatioPoint {
                records,
                store_bytes: store,
                cold_permille: cold,
                cycle_hat_ns: cycle,
                qps_hat: qps,
                ratio_hat_permille: ratio_hat_permille(cycle, rocks_cycle),
                measured,
            }
        })
        .collect()
}

/// Max |error_permille| across the anchor legs of a curve (the fit residual).
#[must_use]
pub fn fit_residual_permille(curve: &[RatioPoint]) -> Option<i64> {
    curve
        .iter()
        .filter_map(|p| p.measured.and_then(|m| m.error_permille))
        .map(|e| e.abs())
        .max()
}

/// What it takes to cross 1.0× at one scale, decomposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutToCross {
    /// `cycle_hat − rocks_cycle` (0 when already ≥ 1.0×).
    pub deficit_ns: u64,
    /// Scale-owned part: `cycle_hat(cold) − cycle_hat(0)`.
    pub disk_ns: u64,
    /// Engine-owned part: `cycle_hat(0) − rocks_cycle` (saturating).
    pub base_gap_ns: u64,
    /// Ranked post-0193/post-0190 slices covering `base_gap_ns`.
    pub covering: Vec<(&'static str, u64)>,
    /// Whether the covering sums to at least the base gap.
    pub covered: bool,
}

/// Named post-ticket slices that can cover the hot-base gap: `wal_encode`,
/// `wal_write` (off the wal mutex since 0193) and `mem_guard` (out of the
/// leader cycle since 0190 P0.2) are excluded by design; what remains is
/// what the next cuts attack.
#[must_use]
fn ranked_ticket_slices(p: WritePhaseNs) -> [(&'static str, u64); 4] {
    let mut slices = [
        ("mem_lock", p.mem_lock),
        ("mem_insert", p.mem_insert),
        ("publish", p.publish),
        ("grp", p.grp),
    ];
    slices.sort_by(|a, b| b.1.cmp(&a.1));
    slices
}

/// Decompose the crossing deficit at one scale and rank the slices that can
/// pay the engine part. The disk part is owned by the bounded-cache cuts
/// (0194 leftover advise / 0195 scan WILLNEED), not by CS slices.
#[must_use]
pub fn cut_to_cross(fit: CurveFit, cold: u64, rocks_cycle: u64, p: WritePhaseNs) -> CutToCross {
    let cycle = fit.cycle_hat_ns(cold);
    let hot = fit.cycle_hat_ns(0);
    let deficit = cycle.saturating_sub(rocks_cycle);
    let disk = cycle.saturating_sub(hot);
    let base_gap = hot.saturating_sub(rocks_cycle);
    let ranked = ranked_ticket_slices(p);
    let mut covering = Vec::new();
    let mut sum = 0u64;
    let mut covered = base_gap == 0;
    if base_gap > 0 {
        for (name, ns) in ranked {
            if ns == 0 {
                break;
            }
            sum = sum.saturating_add(ns);
            covering.push((name, ns));
            if sum >= base_gap {
                covered = true;
                break;
            }
        }
    }
    CutToCross {
        deficit_ns: deficit,
        disk_ns: disk,
        base_gap_ns: base_gap,
        covering,
        covered,
    }
}

/// The whole RFC-0197 table for one RAM ceiling: fit, curve over `scales`,
/// and per-scale crossing decomposition from the pinned write-phase fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RatioCurveTable {
    pub ram_bytes: u64,
    pub warm_cap: u64,
    pub bytes_per_entry: u64,
    pub fit: CurveFit,
    pub rocks_cycle: u64,
    pub rocks_spread: u64,
    pub points: Vec<RatioPoint>,
    /// Crossing decomposition per scale (parallel to `points`).
    pub cuts: Vec<CutToCross>,
    /// Flat AS-IS ratio (permille) — identical at every scale.
    pub as_is_ratio_permille: u64,
    /// Max |error| across anchor legs, if any.
    pub residual_permille: Option<i64>,
}

/// Compose the table from dated anchors and a scale sweep.
#[must_use]
pub fn ratio_curve_table(
    anchors: &[ScaleAnchor],
    scales: &[u64],
    ram_bytes: u64,
    p: WritePhaseNs,
) -> RatioCurveTable {
    let warm_cap = warm_cap_bytes(ram_bytes);
    let fit = fit_write_family_curve(anchors, SCALE_BYTES_PER_ENTRY, warm_cap);
    let rocks_cycle = rocks_cycle_ns(anchors);
    let points = ratio_curve(
        fit,
        rocks_cycle,
        anchors,
        scales,
        SCALE_BYTES_PER_ENTRY,
        warm_cap,
    );
    let cuts = points
        .iter()
        .map(|pt| cut_to_cross(fit, pt.cold_permille, rocks_cycle, p))
        .collect();
    let as_is_cycle = fit.cycle_hat_as_is_ns(0);
    RatioCurveTable {
        ram_bytes,
        warm_cap,
        bytes_per_entry: SCALE_BYTES_PER_ENTRY,
        fit,
        rocks_cycle,
        rocks_spread: rocks_cycle_spread_ns(anchors),
        as_is_ratio_permille: ratio_hat_permille(as_is_cycle, rocks_cycle),
        residual_permille: fit_residual_permille(&points),
        points,
        cuts,
    }
}

impl RatioCurveTable {
    /// CLI dump. `pedra scale-model ratio` prints this verbatim.
    #[must_use]
    pub fn render(&self) -> String {
        let slope_permille = if self.fit.den == 0 {
            0
        } else {
            self.fit.slope_num * 1000 / self.fit.den
        };
        let mut out = format!(
            "ratio-curve family=overwrite_mc4 ram={} warm_cap={} bytes_per_entry={}\n\
             fit base_ns={} slope_ns_per_1000_cold={} residual_permille={}\n\
             rocks_cycle_ns={} spread_ns={}\n",
            self.ram_bytes,
            self.warm_cap,
            self.bytes_per_entry,
            self.fit.cycle_hat_ns(0),
            slope_permille,
            match self.residual_permille {
                Some(r) => r.to_string(),
                None => "-".to_string(),
            },
            self.rocks_cycle,
            self.rocks_spread,
        );
        for (pt, cut) in self.points.iter().zip(&self.cuts) {
            let measured = match pt.measured {
                Some(m) => format!(
                    "measured_ratio_permille={} error_permille={}",
                    m.ratio_permille,
                    match m.error_permille {
                        Some(e) => e.to_string(),
                        None => "-".to_string(),
                    }
                ),
                None => "measured=- (hat; METER)".to_string(),
            };
            out.push_str(&format!(
                "scale={} cold_permille={} cycle_hat_ns={} qps_hat={} ratio_hat_permille={} {}\n",
                pt.records,
                pt.cold_permille,
                pt.cycle_hat_ns,
                pt.qps_hat,
                pt.ratio_hat_permille,
                measured
            ));
            if cut.deficit_ns > 0 {
                let cover = if cut.covering.is_empty() {
                    "-".to_string()
                } else {
                    cut.covering
                        .iter()
                        .map(|(n, v)| format!("{n}:{v}"))
                        .collect::<Vec<_>>()
                        .join("+")
                };
                out.push_str(&format!(
                    "cut_to_cross scale={} deficit_ns={} disk_ns={} base_gap_ns={} covering={} covered={}\n",
                    pt.records,
                    cut.deficit_ns,
                    cut.disk_ns,
                    cut.base_gap_ns,
                    cover,
                    cut.covered
                ));
            }
        }
        out.push_str(&format!(
            "as_is_ratio_permille={} (flat; a cegueira do smoke 15/15)\n",
            self.as_is_ratio_permille
        ));
        out
    }
}

/// CLI dump of the GET-side anchors (RFC-0197 P2.1): registered rows with
/// their dated labels — the ladder states its own incompleteness instead of
/// hiding the missing scales behind silence.
#[must_use]
pub fn render_get_side_anchors(anchors: &[GetSideAnchor]) -> String {
    let mut out = String::from("get-side anchors (RFC-0197 P2.1; escada 100M em 2 caixas — 10k/2M/15M/25M SEM medição datada)\n");
    for a in anchors {
        out.push_str(&format!(
            "get_anchor leg={} records={} ratio_permille={} label={}\n",
            a.leg, a.records, a.ratio_permille, a.label
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_cycle_kernel::LINUX_QUIET_0189_P01;

    const RAM_4GIB: u64 = 4 * (1 << 30);
    const SCALES: [u64; 5] = [100_000, 2_000_000, 15_000_000, 25_000_000, 100_000_000];

    fn table() -> RatioCurveTable {
        // Post-0190/post-0193 view of the pinned fixture: the guard is out of
        // the leader cycle and encode+write are off the wal mutex.
        let mut p = LINUX_QUIET_0189_P01;
        p.mem_guard = 0;
        ratio_curve_table(&WRITE_FAMILY_ANCHORS_2026_09_10, &SCALES, RAM_4GIB, p)
    }

    #[test]
    fn rfc0197_cold_permille_zero_hot_and_monotone_bounded() {
        let cap = crate::scale_kernel::warm_cap_bytes(RAM_4GIB);
        assert_eq!(cold_permille(100_000 * SCALE_BYTES_PER_ENTRY, cap), 0);
        assert_eq!(cold_permille(2_000_000 * SCALE_BYTES_PER_ENTRY, cap), 0);
        let c15 = cold_permille(15_000_000 * SCALE_BYTES_PER_ENTRY, cap);
        let c25 = cold_permille(25_000_000 * SCALE_BYTES_PER_ENTRY, cap);
        let c100 = cold_permille(100_000_000 * SCALE_BYTES_PER_ENTRY, cap);
        assert_eq!((c15, c25, c100), (123, 474, 868));
        assert!(c15 < c25 && c25 < c100 && c100 <= 1000);
    }

    #[test]
    fn rfc0197_fit_reproduces_anchors_within_declared_residual() {
        let t = table();
        // Anchored fit: base 4 673 ns, slope ≈ 5.14 ns per cold-permille.
        assert_eq!(t.fit.cycle_hat_ns(0), 4_673);
        assert_eq!(t.fit.cycle_hat_ns(474), 7_109);
        assert_eq!(t.fit.cycle_hat_ns(868), 9_134);
        let residual = t.residual_permille.expect("anchors carry errors");
        assert_eq!(residual, 93);
        assert!(residual <= FIT_TOLERANCE_PERMILLE);
    }

    #[test]
    fn rfc0197_extrapolation_100m_is_417_hat() {
        let t = table();
        let p100 = t.points.last().copied().expect("100M present");
        assert_eq!(p100.records, 100_000_000);
        assert!(p100.measured.is_none());
        assert_eq!(p100.ratio_hat_permille, 417);
        // The anchored scales stay near their dated labels (887/795/659/557).
        let hats: Vec<u64> = t.points.iter().map(|p| p.ratio_hat_permille).collect();
        assert_eq!(hats, vec![815, 815, 718, 536, 417]);
    }

    #[test]
    fn rfc0197_rocks_cycle_const_is_dated_flat() {
        let t = table();
        assert_eq!(t.rocks_cycle, 3_810);
        assert_eq!(t.rocks_spread, 135);
        let measured: Vec<u64> = t
            .points
            .iter()
            .filter_map(|p| p.measured.map(|m| m.ratio_permille))
            .collect();
        assert_eq!(measured, vec![887, 795, 659, 557]);
    }

    #[test]
    fn rfc0197_cut_to_cross_decomposes_disk_and_base() {
        let t = table();
        let cut = t.cuts.last().cloned().expect("100M cut");
        assert_eq!(cut.deficit_ns, 5_324);
        assert_eq!(cut.disk_ns, 4_461);
        assert_eq!(cut.base_gap_ns, 863);
        assert!(cut.covered);
        let names: Vec<&str> = cut.covering.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["publish", "mem_insert"]);
        // 83.7% of the 100M deficit is the disk term (84% rounded in the
        // RFC) — the bounded-cache cuts own it, not the CS slices.
        assert_eq!(cut.disk_ns * 1000 / cut.deficit_ns, 837);
    }

    #[test]
    fn rfc0197_as_is_curve_is_flat() {
        let t = table();
        let flat = t.fit.cycle_hat_as_is_ns(868);
        assert_eq!(flat, t.fit.cycle_hat_ns(0));
        assert_eq!(t.as_is_ratio_permille, 815);
        // Every scale predicts the hot ratio under AS-IS: 25M would be
        // "fine" at 815 while the measured leg is 557 — the blindness.
        assert!(t.as_is_ratio_permille > 557);
    }

    #[test]
    fn rfc0197_render_carries_hat_and_measured_lines() {
        let out = table().render();
        assert!(out.contains("ratio-curve family=overwrite_mc4"));
        assert!(out.contains("measured=- (hat; METER)"));
        assert!(out.contains("measured_ratio_permille=557"));
        assert!(out.contains("ratio_hat_permille=417"));
        assert!(out.contains("as_is_ratio_permille=815"));
        assert!(out.contains("covering=publish:820+mem_insert:580"));
    }

    #[test]
    fn rfc0197_hot_scale_has_no_disk_deficit_only_base() {
        let t = table();
        let cut = t.cuts.first().cloned().expect("100k cut");
        assert_eq!(cut.disk_ns, 0);
        assert_eq!(cut.deficit_ns, cut.base_gap_ns);
    }

    #[test]
    fn rfc0197_refit_at_wrong_ram_breaks_tolerance() {
        // The anchors were measured on a 4 GiB box: their 15M/25M legs were
        // COLD. Re-classifying them hot under a 64 GiB cap contradicts the
        // measurements themselves — the degenerate flat fit (slope 0) and a
        // 285‰ residual must be rejected, not clamped.
        let mut p = LINUX_QUIET_0189_P01;
        p.mem_guard = 0;
        let t = ratio_curve_table(&WRITE_FAMILY_ANCHORS_2026_09_10, &SCALES, 64 << 30, p);
        assert_eq!(t.fit.slope_num, 0);
        let residual = t.residual_permille.expect("anchors carry errors");
        assert_eq!(residual, 285);
        assert!(residual > FIT_TOLERANCE_PERMILLE);
    }

    #[test]
    fn rfc0197_p21_get_side_anchors_pin_dated_numbers() {
        // Primary sources: the cartaz row is the rocks-parity-compare prefix
        // cell; the DIAG rows are the 3-run min-of-3 medians recomputed from
        // findings/2026-09-04-win-probe-prefix/serial.win9{,.rerun2,.rerun3}.log
        // (100M, vlen=200, 4 GiB guest): e.g. get_hit 2765.7/1749.8 = 1581,
        // 2920.4/1853.0 = 1576, 2845.1/1725.4 = 1649 → min 1576.
        let by_leg = |leg: &str| -> Vec<u64> {
            GET_SIDE_ANCHORS_2026_09_10
                .iter()
                .filter(|a| a.leg == leg)
                .map(|a| a.ratio_permille)
                .collect()
        };
        assert_eq!(by_leg("prefix_scan"), vec![700, 1050]);
        assert_eq!(by_leg("get_hit"), vec![1576]);
        assert_eq!(by_leg("lookup_100_get_loop"), vec![1484]);
        assert_eq!(by_leg("lookup_100_multi_get"), vec![1562]);
        assert_eq!(by_leg("prefix_scan_vlen200"), vec![1309]);
        for a in GET_SIDE_ANCHORS_2026_09_10 {
            assert!(a.records == 100_000_000);
            assert!(a.label.contains("2026-09-"));
        }
    }

    #[test]
    fn rfc0197_p21_get_ladder_states_its_own_incompleteness() {
        let covered = get_side_ladder_scales(&GET_SIDE_ANCHORS_2026_09_10, &SCALES);
        // 1 of 5 write-family scales carries a dated GET anchor: the
        // deferral is structural, not an omission.
        assert_eq!(covered, vec![100_000_000]);
        let render = render_get_side_anchors(&GET_SIDE_ANCHORS_2026_09_10);
        assert!(render.contains("10k/2M/15M/25M SEM medição datada"));
        assert!(render.contains("get_anchor leg=prefix_scan records=100000000 ratio_permille=700"));
        assert!(render.contains("ratio_permille=1050"));
        assert!(render.contains("ratio_permille=1576"));
    }

    #[test]
    fn rfc0197_p21_point_gets_paid_scan_hole_is_disk_pattern() {
        // The 100M GET story in one assertion set: point paths are paid
        // (>= 1400‰ on every DIAG leg) while the sequential scan at the same
        // scale is the 700‰ cartaz — and the RAM-fits contrast (1050‰)
        // attributes that hole to the bounded-cache I/O pattern, not compute.
        // 0195's landed WILLNEED window is the named owner of the gap.
        let point: Vec<u64> = GET_SIDE_ANCHORS_2026_09_10
            .iter()
            .filter(|a| !a.leg.starts_with("prefix_scan"))
            .map(|a| a.ratio_permille)
            .collect();
        assert!(point.iter().all(|&r| r >= 1400));
        assert!(point.iter().all(|&r| r <= 1600));
        let cartaz = GET_SIDE_ANCHORS_2026_09_10[0].ratio_permille;
        let big_guest = GET_SIDE_ANCHORS_2026_09_10[1].ratio_permille;
        assert_eq!((cartaz, big_guest), (700, 1050));
        assert!(cartaz < big_guest && big_guest >= 1000);
    }
}
