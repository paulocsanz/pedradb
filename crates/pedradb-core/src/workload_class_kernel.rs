//! Endure workload class (RFC-0235): \(w=(z_0,z_1,q,W)\) → one D4 action.
//! Integer unique-max; no I/O, no env pin, no autotune \(T\).
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_workload_class.sh`).
//!
//!   ./scripts/aeneas_workload_class.sh --required
//!
//! \(z_0\) = point empty (miss), \(z_1\) = point nonempty (hit),
//! \(q\) = short range/count, \(W\) = writes. Unique strict max picks the
//! class; empty or a tie is `Mixed`. Sequential ingest is the bulk latch
//! (RFC-0159), not this 4-tuple.
//!
//! AS-IS always `Mixed` — the pre-0235 shape where every fire rediscovers
//! a knob as if one LSM fit every mix.

#![forbid(unsafe_code)]

/// One Endure class. `Sequential` is not produced by the 4-tuple
/// (bulk latch); it exists so the action table in RFC-0235 is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadClass {
    /// Point lookup that missed (\(z_0\) unique max).
    PointMiss,
    /// Point lookup that hit (\(z_1\) unique max).
    PointHit,
    /// Range/count (\(q\) unique max).
    ShortRange,
    /// Sorted ingest (not from the 4-tuple).
    Sequential,
    /// Writes (\(W\) unique max).
    WriteBurst,
    /// Empty window or a tie — Endure robust default (leveling).
    Mixed,
}

/// Unique strict max among \((z_0,z_1,q,W)\). Ties and the empty
/// window return [`WorkloadClass::Mixed`].
#[must_use]
pub fn workload_class(z0: u64, z1: u64, q: u64, w: u64) -> WorkloadClass {
    if q > z0 && q > z1 && q > w {
        return WorkloadClass::ShortRange;
    }
    if w > z0 && w > z1 && w > q {
        return WorkloadClass::WriteBurst;
    }
    if z0 > z1 && z0 > q && z0 > w {
        return WorkloadClass::PointMiss;
    }
    if z1 > z0 && z1 > q && z1 > w {
        return WorkloadClass::PointHit;
    }
    WorkloadClass::Mixed
}

/// AS-IS: every mix is `Mixed` — no class, every fire picks a knob.
#[must_use]
pub fn workload_class_as_is(_z0: u64, _z1: u64, _q: u64, _w: u64) -> WorkloadClass {
    WorkloadClass::Mixed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_class_scan_window_is_short_range() {
        assert_eq!(workload_class(0, 0, 8, 0), WorkloadClass::ShortRange);
        assert_eq!(workload_class_as_is(0, 0, 8, 0), WorkloadClass::Mixed);
    }

    #[test]
    fn workload_class_overwrite_window_is_write_burst() {
        assert_eq!(workload_class(0, 0, 0, 8), WorkloadClass::WriteBurst);
        assert_eq!(workload_class_as_is(0, 0, 0, 8), WorkloadClass::Mixed);
    }

    #[test]
    fn workload_class_on_live_scan_window_is_not_ok() {
        assert_eq!(workload_class(0, 0, 10, 0), WorkloadClass::ShortRange);
        assert_eq!(
            workload_class_as_is(0, 0, 10, 0),
            WorkloadClass::Mixed,
            "AS-IS tooth: scan window still Mixed"
        );
    }

    #[test]
    fn workload_class_tie_and_empty_are_mixed() {
        assert_eq!(workload_class(0, 0, 0, 0), WorkloadClass::Mixed);
        assert_eq!(workload_class(3, 3, 1, 0), WorkloadClass::Mixed);
        assert_eq!(workload_class(5, 0, 0, 0), WorkloadClass::PointMiss);
        assert_eq!(workload_class(0, 5, 0, 0), WorkloadClass::PointHit);
    }
}
