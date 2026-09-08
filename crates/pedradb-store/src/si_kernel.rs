//! Pure SI / changelog reader ranking (RFC-0002 P19 / P21 / P30 / F42 / F55 / F84).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_si_reader.sh
//!
//! Production [`crate::StoreCluster::best_applied_reader`] and
//! [`crate::StoreCluster::best_changelog_reader`] fold local peers with
//! [`si_reader_beats`]. Watermark is `applied` (F42) or `last_sequence` (F55).
//! Point `get` / `dcs_get` use [`point_get_prefer_applied`] (F84).

#![forbid(unsafe_code)]

macro_rules! si_reader_beats_body {
    ($c_leader:expr, $c_part:expr, $c_self:expr, $c_applied:expr, $b_leader:expr, $b_part:expr, $b_self:expr, $b_applied:expr) => {{
        let c_live = $c_leader && $c_part;
        let b_live = $b_leader && $b_part;
        if c_live != b_live {
            c_live
        } else if $c_part != $b_part {
            $c_part
        } else if $c_self != $b_self {
            $c_self
        } else {
            $c_applied > $b_applied
        }
    }};
}

macro_rules! si_reader_beats_as_is_body {
    ($c_leader:expr, $c_part:expr, $c_self:expr, $c_applied:expr, $b_leader:expr, $b_part:expr, $b_self:expr, $b_applied:expr) => {{
        let _ = (
            $c_leader, $c_part, $c_self, $c_applied, $b_leader, $b_part, $b_self, $b_applied,
        );
        false
    }};
}

macro_rules! point_get_prefer_applied_body {
    () => {
        true
    };
}

macro_rules! point_get_prefer_applied_as_is_body {
    () => {
        false
    };
}

macro_rules! point_get_watermark_body {
    ($range_applied:expr, $global_seq:expr) => {{
        let _ = $global_seq;
        $range_applied
    }};
}

macro_rules! point_get_watermark_as_is_body {
    ($range_applied:expr, $global_seq:expr) => {{
        let _ = $range_applied;
        $global_seq
    }};
}

/// Whether candidate `c` is a better SI/applied reader than `best`.
///
/// Order: live local leader ≻ participating ≻ self ≻ higher `applied`.
/// Never “first id wins” (F42: partitioned `ids[0]` poisons hist).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[allow(clippy::too_many_arguments)] // arity locked to verus/si_reader.rs
pub fn si_reader_beats(
    c_leader: bool,
    c_part: bool,
    c_self: bool,
    c_applied: u64,
    b_leader: bool,
    b_part: bool,
    b_self: bool,
    b_applied: u64,
) -> bool {
    si_reader_beats_body!(
        c_leader, c_part, c_self, c_applied, b_leader, b_part, b_self, b_applied
    )
}

/// F84: point LocalApplied `get` ranks by per-range `applied`, not Pedra
/// `last_sequence` (a node busy on range A can lag on range B).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn point_get_prefer_applied() -> bool {
    point_get_prefer_applied_body!()
}

/// AS-IS F84: `best_changelog_reader` (max global `last_sequence`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn point_get_prefer_applied_as_is() -> bool {
    point_get_prefer_applied_as_is_body!()
}

/// Watermark folded into [`si_reader_beats`] for a point get (FIXED: range applied).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn point_get_watermark(range_applied: u64, global_seq: u64) -> u64 {
    point_get_watermark_body!(range_applied, global_seq)
}

/// AS-IS F84: global Pedra sequence.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn point_get_watermark_as_is(range_applied: u64, global_seq: u64) -> u64 {
    point_get_watermark_as_is_body!(range_applied, global_seq)
}

/// AS-IS F42: first candidate always stays (ids[0] / first local).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[allow(clippy::too_many_arguments)] // arity locked to verus/si_reader.rs
pub fn si_reader_beats_as_is(
    c_leader: bool,
    c_part: bool,
    c_self: bool,
    c_applied: u64,
    b_leader: bool,
    b_part: bool,
    b_self: bool,
    b_applied: u64,
) -> bool {
    si_reader_beats_as_is_body!(
        c_leader, c_part, c_self, c_applied, b_leader, b_part, b_self, b_applied
    )
}

/// Decision for one snapshot read against the SI GC watermark (F168).
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotRead {
    /// Snapshot predates the GC floor; history for it may be pruned.
    /// Serving a value (or absence) would be fabricated — fail closed.
    TooOld,
    /// Snapshot is at/above the floor entry (`watermark - 1`) and servable
    /// from pruned history.
    Serve,
}

/// F168 kernel: may a read at `snapshot` be served, or must it fail as
/// `TransactionTooOld`?
///
/// `maybe_gc_versions` keeps one **floor** entry at `watermark - 1`, so the
/// smallest servable snapshot is `watermark - 1`; anything strictly older has
/// no covering entry and the old code answered `Ok(None)` — fabricated key
/// absence for committed data. Overflow-free form: `watermark - 1 > snapshot`
/// (`watermark == 0` and `snapshot == u64::MAX` never reject).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn snapshot_read_plan(snapshot: u64, watermark: u64) -> SnapshotRead {
    if watermark.saturating_sub(1) > snapshot {
        SnapshotRead::TooOld
    } else {
        SnapshotRead::Serve
    }
}

/// AS-IS F168: every snapshot is "servable" — pruned history fabricates
/// absence (`Ok(None)`) instead of `TransactionTooOld`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn snapshot_read_plan_as_is(_snapshot: u64, _watermark: u64) -> SnapshotRead {
    SnapshotRead::Serve
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn si_reader_beats_spec(
    c_leader: bool,
    c_part: bool,
    c_self: bool,
    c_applied: u64,
    b_leader: bool,
    b_part: bool,
    b_self: bool,
    b_applied: u64,
) -> bool {
    let c_live = c_leader && c_part;
    let b_live = b_leader && b_part;
    if c_live != b_live {
        c_live
    } else if c_part != b_part {
        c_part
    } else if c_self != b_self {
        c_self
    } else {
        c_applied > b_applied
    }
}

pub fn si_reader_beats(
    c_leader: bool,
    c_part: bool,
    c_self: bool,
    c_applied: u64,
    b_leader: bool,
    b_part: bool,
    b_self: bool,
    b_applied: u64,
) -> (d: bool)
    ensures
        d == si_reader_beats_spec(
            c_leader,
            c_part,
            c_self,
            c_applied,
            b_leader,
            b_part,
            b_self,
            b_applied,
        ),
{
    si_reader_beats_body!(
        c_leader, c_part, c_self, c_applied, b_leader, b_part, b_self, b_applied
    )
}

pub open spec fn si_reader_beats_as_is_spec(
    _c_leader: bool,
    _c_part: bool,
    _c_self: bool,
    _c_applied: u64,
    _b_leader: bool,
    _b_part: bool,
    _b_self: bool,
    _b_applied: u64,
) -> bool {
    false
}

pub fn si_reader_beats_as_is(
    c_leader: bool,
    c_part: bool,
    c_self: bool,
    c_applied: u64,
    b_leader: bool,
    b_part: bool,
    b_self: bool,
    b_applied: u64,
) -> (d: bool)
    ensures
        d == false,
        d == si_reader_beats_as_is_spec(
            c_leader,
            c_part,
            c_self,
            c_applied,
            b_leader,
            b_part,
            b_self,
            b_applied,
        ),
{
    si_reader_beats_as_is_body!(
        c_leader, c_part, c_self, c_applied, b_leader, b_part, b_self, b_applied
    )
}

proof fn lemma_as_is_keeps_first()
    ensures
        !si_reader_beats_as_is_spec(true, true, false, 10, false, false, true, 1),
{
}

proof fn lemma_leader_beats_lagging_first()
    ensures
        si_reader_beats_spec(true, true, false, 10, false, false, true, 1),
{
}

pub fn point_get_prefer_applied() -> (d: bool)
    ensures
        d == true,
{
    point_get_prefer_applied_body!()
}

pub fn point_get_prefer_applied_as_is() -> (d: bool)
    ensures
        d == false,
{
    point_get_prefer_applied_as_is_body!()
}

pub open spec fn point_get_watermark_spec(range_applied: u64, _global_seq: u64) -> u64 {
    range_applied
}

pub open spec fn point_get_watermark_as_is_spec(_range_applied: u64, global_seq: u64) -> u64 {
    global_seq
}

pub fn point_get_watermark(range_applied: u64, global_seq: u64) -> (w: u64)
    ensures
        w == point_get_watermark_spec(range_applied, global_seq),
        w == range_applied,
{
    point_get_watermark_body!(range_applied, global_seq)
}

pub fn point_get_watermark_as_is(range_applied: u64, global_seq: u64) -> (w: u64)
    ensures
        w == point_get_watermark_as_is_spec(range_applied, global_seq),
        w == global_seq,
{
    point_get_watermark_as_is_body!(range_applied, global_seq)
}

proof fn lemma_as_is_ignores_lagging_range(applied: u64, global: u64)
    requires
        applied < global,
    ensures
        point_get_watermark_spec(applied, global) < point_get_watermark_as_is_spec(applied, global),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leader_beats_lagging_first() {
        assert!(si_reader_beats(
            true, true, false, 10, false, false, true, 1
        ));
        assert!(!si_reader_beats_as_is(
            true, true, false, 10, false, false, true, 1
        ));
    }

    #[test]
    fn participating_beats_partitioned() {
        assert!(si_reader_beats(
            false, true, false, 3, false, false, true, 99
        ));
    }

    #[test]
    fn higher_applied_among_peers() {
        assert!(si_reader_beats(
            false, true, false, 8, false, true, false, 3
        ));
        assert!(!si_reader_beats(
            false, true, false, 3, false, true, false, 8
        ));
    }

    #[test]
    fn point_get_uses_range_applied() {
        assert!(point_get_prefer_applied());
        assert!(!point_get_prefer_applied_as_is());
        assert_eq!(point_get_watermark(3, 99), 3);
        assert_eq!(point_get_watermark_as_is(3, 99), 99);
        assert!(point_get_watermark(3, 99) < point_get_watermark_as_is(3, 99));
    }

    #[test]
    fn snapshot_plan_fails_closed_below_floor() {
        use super::SnapshotRead;
        // watermark 7 ⇒ floor at 6: snapshot 5 is TooOld, 6/7 serve.
        assert_eq!(snapshot_read_plan(5, 7), SnapshotRead::TooOld);
        assert_eq!(snapshot_read_plan(6, 7), SnapshotRead::Serve);
        assert_eq!(snapshot_read_plan(7, 7), SnapshotRead::Serve);
        // No GC yet: everything serves, including snapshot 0.
        assert_eq!(snapshot_read_plan(0, 0), SnapshotRead::Serve);
        assert_eq!(snapshot_read_plan(0, 1), SnapshotRead::Serve);
        // Overflow edges: max snapshot / max watermark never reject.
        assert_eq!(snapshot_read_plan(u64::MAX, u64::MAX), SnapshotRead::Serve);
        assert_eq!(snapshot_read_plan(u64::MAX, 0), SnapshotRead::Serve);
        assert_eq!(snapshot_read_plan(0, u64::MAX), SnapshotRead::TooOld);
    }

    /// F168 teeth: AS-IS serves the below-floor snapshot (fabricated absence).
    #[test]
    fn snapshot_plan_as_is_serves_everything() {
        use super::SnapshotRead;
        assert_eq!(snapshot_read_plan_as_is(0, u64::MAX), SnapshotRead::Serve);
    }

    #[test]
    fn theorem_on_bool_domain() {
        let mut n = 0u32;
        for cl in [false, true] {
            for cp in [false, true] {
                for cs in [false, true] {
                    for bl in [false, true] {
                        for bp in [false, true] {
                            for bs in [false, true] {
                                for ca in 0u64..3 {
                                    for ba in 0u64..3 {
                                        let d = si_reader_beats(cl, cp, cs, ca, bl, bp, bs, ba);
                                        let want = (cl && cp, cp, cs, ca) > (bl && bp, bp, bs, ba);
                                        assert_eq!(d, want);
                                        assert!(!si_reader_beats_as_is(
                                            cl, cp, cs, ca, bl, bp, bs, ba,
                                        ));
                                        n += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(n, 2 * 2 * 2 * 2 * 2 * 2 * 3 * 3);
    }
}
