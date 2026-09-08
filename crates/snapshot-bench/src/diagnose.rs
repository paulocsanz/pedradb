//! RFC-0182 P2.1: snapshot-bench point-get vs the RFC-0176 clock.
//!
//! Same `classify_get` line as `pedra scale` get_hit (0184 P2.6). Criterion
//! medians land in `estimates.json` after `BenchmarkGroup::finish`; we
//! classify that number, not a second stopwatch.

use pedradb_core::{classify_get, scale_kernel, GetClass};

/// `(class, best, happy, worst, as_is)` ns against the 0176 forecast.
#[must_use]
pub fn classify_measured(n: u64, ram: u64, measured_ns: u64) -> (GetClass, u64, u64, u64, u64) {
    let f = scale_kernel::scale_forecast(n, ram);
    let as_is = scale_kernel::scale_forecast_as_is(n, ram);
    let class = classify_get(
        measured_ns,
        f.best_ns,
        f.happy_ns,
        f.worst_ns,
        as_is.best_ns,
    );
    (class, f.best_ns, f.happy_ns, f.worst_ns, as_is.best_ns)
}

/// One harness line: `diagnose get <cell>/<label> … class=…`.
#[must_use]
pub fn diagnose_line(cell: &str, label: &str, n: u64, ram: u64, measured_ns: u64) -> String {
    let (class, best, happy, worst, as_is) = classify_measured(n, ram, measured_ns);
    format!(
        "diagnose get {cell}/{label} measured_ns={measured_ns} best={best} happy={happy} worst={worst} as_is={as_is} class={}",
        class.token()
    )
}

/// Print [`diagnose_line`] to stderr (same stream as hydrate/probe_hit).
pub fn eprint_get(cell: &str, label: &str, n: u64, ram: u64, measured_ns: u64) {
    eprintln!("{}", diagnose_line(cell, label, n, ram, measured_ns));
}

/// After criterion `finish()`, classify `group/id` from the median estimate.
pub fn eprint_get_from_criterion(group: &str, id: &str, n: u64, ram: u64) {
    let Some(median) = crate::cellcost::median_ns(group, id) else {
        return;
    };
    if !(median.is_finite() && median > 0.0) {
        return;
    }
    eprint_get(group, id, n, ram, median.round() as u64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::GetClass;

    /// RFC-0182 cites snapshot 1M get_hit 1,38 µs vs Rocks 2,58 µs — RAM clock.
    #[test]
    fn rfc0182_p21_snapshot_get_hit_classifies_vs_0176() {
        let n = 1_000_000u64;
        let ram = 64u64 << 30;
        let (class, best, happy, worst, as_is) = classify_measured(n, ram, 1_380);
        assert_ne!(
            class,
            GetClass::AsIsWalk,
            "1M get_hit 1.38µs is not walk-all best={best} happy={happy} worst={worst} as_is={as_is}"
        );
        assert!(
            matches!(
                class,
                GetClass::Best | GetClass::Happy | GetClass::FasterThanModel
            ),
            "got {} best={best} happy={happy} worst={worst} as_is={as_is}",
            class.token()
        );
        let printed = diagnose_line("get_hit", "pedradb", n, ram, 1_380);
        assert!(
            printed.starts_with("diagnose get get_hit/pedradb"),
            "{printed}"
        );
        assert!(printed.contains("class="), "{printed}");
        assert!(!printed.contains("as_is_walk"), "{printed}");
    }

    #[test]
    fn rfc0182_p21_as_is_walk_is_not_best() {
        let n = 1_000_000_000u64;
        let ram = 64u64 << 30;
        let as_is = scale_kernel::scale_forecast_as_is(n, ram);
        let (class, ..) = classify_measured(n, ram, as_is.best_ns);
        assert_eq!(class, GetClass::AsIsWalk, "got {}", class.token());
    }
}
