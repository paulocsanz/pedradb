//! Pure compaction decisions (RFC-0056 P0.3 — crash dictionary on the
//! merge/GC path).
//!
//! Production `Db::compact_with_ssts_only` and `merge::gc_snapshot_safe`
//! route their decisions through this kernel: which levels to merge, the
//! snapshot-safe fate of each point version, and the F177 bottommost
//! guard for lone tombstones. Merging bytes, writing SSTs, installing and
//! deleting files stay caller + axiom.
//!
//! Named decisions (the ones that were `SilentWrong` when inverted):
//! - **A version a pinned snapshot still reads is kept** — an older
//!   version drops only when its newer sibling is visible to every open
//!   snapshot (`newer_seq <= oldest_snapshot`).
//! - **F177: a lone tombstone drops only on a bottommost rewrite** — in a
//!   partial compaction an older version of the key can live in a file
//!   outside the input; dropping the tombstone there resurrects that
//!   version (durably, after reopen).
//! - **Merge moves exactly one level down** — the trigger picks the
//!   lowest non-empty level below max; GC-only rewrite of the max level
//!   happens only when requested.
//!
//! Verus twin: `crates/pedradb-core/verus/compact_decision.rs`.
//! Spec page: `docs/formal/crash-dictionary.md` (compaction section).

#![forbid(unsafe_code)]

/// What one compaction run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompactPlan {
    /// Merge the lowest non-empty level into the next one down.
    Merge {
        /// Source level (the lowest one holding files).
        from: u32,
        /// Destination level (always `from + 1`).
        to: u32,
    },
    /// Files exist only at the max level and GC was requested: rewrite
    /// them all in place.
    GcRewriteMax,
    /// Nothing to do.
    NoOp,
}

/// Pure rule for the compaction trigger / level choice.
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   plan == Merge{from, to} ==> (from < max_level && to == from + 1)
///   plan == GcRewriteMax    ==> (lowest_level_with_files == None
///                                && gc_requested && files_at_max_level)
///   plan == NoOp            ==> (lowest_level_with_files == None
///                                && !(gc_requested && files_at_max_level))
/// ```
///
/// Finite-domain check: [`tests::theorem_compact_pick_on_finite_domain`].
#[must_use]
pub fn compact_pick(
    lowest_level_with_files: Option<u32>,
    files_at_max_level: bool,
    gc_requested: bool,
    max_level: u32,
) -> CompactPlan {
    // `max_level` is part of the caller contract (GcRewriteMax targets it);
    // the decision itself does not need its value.
    let _ = max_level;
    match lowest_level_with_files {
        // Caller invariant: `lowest_level_with_files < max_level` (the
        // trigger loop only scans levels below max).
        Some(l) => CompactPlan::Merge { from: l, to: l + 1 },
        None => {
            if gc_requested && files_at_max_level {
                CompactPlan::GcRewriteMax
            } else {
                CompactPlan::NoOp
            }
        }
    }
}

/// AS-IS: never compact (acked versions pile in L0 forever; or the inverse
/// hole — skip the merge that would drop a live pin).
#[must_use]
pub fn compact_pick_as_is(
    _lowest_level_with_files: Option<u32>,
    _files_at_max_level: bool,
    _gc_requested: bool,
    _max_level: u32,
) -> CompactPlan {
    CompactPlan::NoOp
}

/// What happens to one version of a user key during GC compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionFate {
    /// A live snapshot can still read this version.
    Keep,
    /// Every snapshot sees a newer sibling: safe to drop.
    Drop,
}

/// Pure snapshot-safe retention rule for one non-newest point version.
///
/// `newer_kept_seq` is the sequence of the immediately-newer sibling that
/// was kept (`None` ⇒ this is the newest version of the key, always kept).
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   fate == Drop ==> newer_kept_seq is Some(n) && n <= oldest_snapshot
///     // every open snapshot (all >= oldest_snapshot) sees the newer
///     // sibling, never this version
///   newer_kept_seq is Some(n) && n > oldest_snapshot ==> fate == Keep
///     // a snapshot pinned between this version and n still reads it
/// ```
///
/// Finite-domain check: [`tests::theorem_point_version_fate_on_finite_domain`].
#[must_use]
pub fn point_version_fate(
    this_seq: u64,
    newer_kept_seq: Option<u64>,
    oldest_snapshot: u64,
) -> VersionFate {
    // `this_seq` identifies the version under decision; the fate itself
    // depends only on the newer sibling and the oldest snapshot.
    let _ = this_seq;
    match newer_kept_seq {
        // Newest version of the key is always kept.
        None => VersionFate::Keep,
        Some(newer_seq) => {
            if newer_seq <= oldest_snapshot {
                VersionFate::Drop
            } else {
                VersionFate::Keep
            }
        }
    }
}

/// AS-IS silent-wrong: drop a version by **its own** sequence instead of
/// the newer sibling's — every version below the watermark vanishes even
/// though a snapshot pinned between it and the newer sibling still reads
/// it ("compacts over a pinned snapshot"). Mutant must fail every theorem
/// above.
#[must_use]
pub fn point_version_fate_as_is_drop_under_snapshot(
    this_seq: u64,
    newer_kept_seq: Option<u64>,
    oldest_snapshot: u64,
) -> VersionFate {
    match newer_kept_seq {
        None => VersionFate::Keep,
        Some(_) => {
            if this_seq <= oldest_snapshot {
                VersionFate::Drop
            } else {
                VersionFate::Keep
            }
        }
    }
}

/// Pure F177 rule for the lone newest tombstone of a key.
///
/// `lone_newest_tombstone`: after retention, the only kept version of the
/// key is its tombstone (nothing older survived the input). It may be
/// dropped only when the compaction input covered every live SST down to
/// the bottom level — otherwise an older version in a file outside the
/// input resurrects after the merge.
///
/// # Post-condition (theorem-ready)
///
/// ```text
/// ensures
///   fate == Drop ==> (bottommost && lone_newest_tombstone)
///   !bottommost  ==> fate == Keep   // F177: partial compaction keeps it
/// ```
///
/// Finite-domain check: [`tests::theorem_lone_tombstone_on_finite_domain`].
#[must_use]
pub fn lone_tombstone_fate(bottommost: bool, lone_newest_tombstone: bool) -> VersionFate {
    if bottommost && lone_newest_tombstone {
        VersionFate::Drop
    } else {
        VersionFate::Keep
    }
}

/// AS-IS F177 violation: drop the lone tombstone regardless of
/// bottommost — the older version living in a file outside the partial
/// compaction input resurrects (durably, after reopen).
#[must_use]
pub fn lone_tombstone_fate_as_is_ignore_bottommost(
    _bottommost: bool,
    lone_newest_tombstone: bool,
) -> VersionFate {
    if lone_newest_tombstone {
        VersionFate::Drop
    } else {
        VersionFate::Keep
    }
}

/// Snapshot-safe GC floor from the oldest live [`crate::db::SnapshotPin`].
///
/// No pin ⇒ cap at `last_seq.min(visible_seq)` (unpublished writes must not
/// raise the watermark). AS-IS ignores the pin and always uses that cap —
/// compact-over-snapshot.
#[must_use]
pub fn gc_oldest_from_pin(oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> u64 {
    match oldest_pin {
        Some(p) => p,
        None => last_seq.min(visible_seq),
    }
}

/// AS-IS: ignore the pin (compact over a live snapshot).
#[must_use]
pub fn gc_oldest_from_pin_as_is(_oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> u64 {
    last_seq.min(visible_seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_version_always_kept() {
        for oldest in 0..5 {
            assert_eq!(
                point_version_fate(7, None, oldest),
                VersionFate::Keep,
                "newest kept under oldest={oldest}"
            );
        }
    }

    #[test]
    fn drops_only_when_newer_visible_to_all_snapshots() {
        // newer sibling at 3; oldest open snapshot at 3 sees it ⇒ drop ok.
        assert_eq!(point_version_fate(1, Some(3), 3), VersionFate::Drop);
        // oldest at 2 < newer 3: a snapshot pinned at 2 still reads seq 1.
        assert_eq!(point_version_fate(1, Some(3), 2), VersionFate::Keep);
    }

    #[test]
    fn lone_tombstone_needs_bottommost() {
        assert_eq!(lone_tombstone_fate(true, true), VersionFate::Drop);
        for lone in [false, true] {
            assert_eq!(
                lone_tombstone_fate(false, lone),
                VersionFate::Keep,
                "partial compaction keeps tombstones (F177)"
            );
        }
        assert_eq!(lone_tombstone_fate(true, false), VersionFate::Keep);
    }

    #[test]
    fn pick_merges_lowest_and_moves_one_level() {
        assert_eq!(
            compact_pick(Some(0), false, false, 3),
            CompactPlan::Merge { from: 0, to: 1 }
        );
        assert_eq!(
            compact_pick(Some(2), true, false, 3),
            CompactPlan::Merge { from: 2, to: 3 }
        );
        assert_eq!(compact_pick(None, true, true, 3), CompactPlan::GcRewriteMax);
        assert_eq!(compact_pick(None, true, false, 3), CompactPlan::NoOp);
        assert_eq!(compact_pick(None, false, true, 3), CompactPlan::NoOp);
    }

    /// Finite-domain theorem (seq space 0..=4): a drop always has the newer
    /// sibling visible to every open snapshot; a version pinned between
    /// itself and the newer sibling is kept; the AS-IS mutant drops exactly
    /// those pinned versions.
    #[test]
    fn theorem_point_version_fate_on_finite_domain() {
        for this in 0..=4u64 {
            for newer in 0..=4u64 {
                for oldest in 0..=4u64 {
                    if newer <= this {
                        continue; // siblings are strictly newer
                    }
                    let f = point_version_fate(this, Some(newer), oldest);
                    match f {
                        VersionFate::Drop => {
                            assert!(newer <= oldest, "drop requires newer visible to all snaps");
                        }
                        VersionFate::Keep => {}
                    }
                    if newer > oldest {
                        // snapshot pinned at `oldest` (>= this here?) reads the
                        // newest version <= oldest; if that is `this`, keep.
                        if this <= oldest {
                            assert_eq!(f, VersionFate::Keep, "pinned snapshot reads this");
                        }
                        let m =
                            point_version_fate_as_is_drop_under_snapshot(this, Some(newer), oldest);
                        if this <= oldest {
                            assert_eq!(m, VersionFate::Drop, "AS-IS must drop the pinned one");
                            assert_ne!(m, f, "mutant must differ from fixed");
                        }
                    }
                }
            }
        }
    }

    /// Finite-domain theorem (2×2): lone tombstones drop only on a
    /// bottommost rewrite; the AS-IS mutant resurrects on every partial
    /// compaction.
    #[test]
    fn theorem_lone_tombstone_on_finite_domain() {
        for bottommost in [false, true] {
            for lone in [false, true] {
                let f = lone_tombstone_fate(bottommost, lone);
                if f == VersionFate::Drop {
                    assert!(bottommost && lone, "drop needs bottommost + lone");
                }
                if !bottommost && lone {
                    let m = lone_tombstone_fate_as_is_ignore_bottommost(bottommost, lone);
                    assert_eq!(m, VersionFate::Drop, "AS-IS must resurrect (F177)");
                    assert_ne!(m, f, "mutant must differ from fixed");
                }
            }
        }
    }

    /// Finite-domain theorem: Merge always moves exactly one level down;
    /// GcRewriteMax only when nothing lives below max and GC was asked.
    #[test]
    fn theorem_compact_pick_on_finite_domain() {
        let max = 3;
        for lowest in [None, Some(0), Some(1), Some(2)] {
            for files_at_max in [false, true] {
                for gc in [false, true] {
                    let p = compact_pick(lowest, files_at_max, gc, max);
                    match p {
                        CompactPlan::Merge { from, to } => {
                            assert!(from < max);
                            assert_eq!(to, from + 1, "one level down");
                            assert_eq!(lowest, Some(from));
                        }
                        CompactPlan::GcRewriteMax => {
                            assert!(lowest.is_none() && gc && files_at_max);
                        }
                        CompactPlan::NoOp => {
                            assert!(lowest.is_none());
                            assert!(!(gc && files_at_max));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn pin_is_oldest_snapshot_for_point_version_fate() {
        let pin = 5u64;
        let last = 10u64;
        let vis = 9u64;
        let oldest = gc_oldest_from_pin(Some(pin), last, vis);
        assert_eq!(oldest, pin);
        // newer sibling at 8; pin at 5 still reads seq 1.
        assert_eq!(point_version_fate(1, Some(8), oldest), VersionFate::Keep);
        let as_is = gc_oldest_from_pin_as_is(Some(pin), last, vis);
        assert_eq!(as_is, vis.min(last));
        assert_eq!(
            point_version_fate(1, Some(8), as_is),
            VersionFate::Drop,
            "AS-IS dente: ignore pin ⇒ drop the pinned version"
        );
        assert_eq!(gc_oldest_from_pin(None, last, vis), last.min(vis));
    }

    #[test]
    fn gc_oldest_from_pin_on_live_reclaim_is_not_ok() {
        let pin = 5u64;
        let oldest = gc_oldest_from_pin(Some(pin), 10, 9);
        assert_eq!(oldest, pin);
        assert_eq!(point_version_fate(1, Some(8), oldest), VersionFate::Keep);
        assert_eq!(
            point_version_fate(1, Some(8), gc_oldest_from_pin_as_is(Some(pin), 10, 9)),
            VersionFate::Drop,
            "AS-IS dente: compact over pin"
        );
    }

    #[test]
    fn compact_pick_on_live_merge_is_not_ok() {
        assert!(matches!(
            compact_pick(Some(0), false, false, 3),
            CompactPlan::Merge { from: 0, to: 1 }
        ));
        assert_eq!(
            compact_pick_as_is(Some(0), false, false, 3),
            CompactPlan::NoOp,
            "AS-IS dente: skip merge"
        );
    }

    #[test]
    fn point_version_fate_on_live_snapshot_is_not_ok() {
        // newer sibling at 8; pin at 5 still reads seq 1.
        assert_eq!(point_version_fate(1, Some(8), 5), VersionFate::Keep);
        assert_eq!(
            point_version_fate_as_is_drop_under_snapshot(1, Some(8), 5),
            VersionFate::Drop,
            "AS-IS dente: drop a version a snapshot still reads"
        );
    }
}
