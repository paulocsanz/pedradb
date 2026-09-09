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

//! **Single artifact (pair `compact_decision`):** this file is what
//! `rustc` links *and* what Verus proves (`cfg(verus_keep_ghost)`).
//! Pairs `compact_retention` / `pin_gc` keep twins until their turns.
//!
//!   ./scripts/verus_compact_decision.sh
//!
//! rustc `compact_pick` stays last-wins (4-arg, includes `max_level`).
//! Verus stand-in is the 3-arg closed form (same Merge/Gc/NoOp arms).

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {
/// Mirrors `CompactPlan` in compact_kernel.rs.
pub enum CompactPlan {
    Merge { from: u32, to: u32 },
    GcRewriteMax,
    NoOp,
}

/// Closed-form spec — same arms as `compact_kernel::compact_pick`.
pub open spec fn compact_pick_spec(
    lowest_level_with_files: Option<u32>,
    files_at_max_level: bool,
    gc_requested: bool,
) -> CompactPlan {
    match lowest_level_with_files {
        Option::Some(l) => CompactPlan::Merge { from: l, to: (l + 1) as u32 },
        Option::None => {
            if gc_requested && files_at_max_level {
                CompactPlan::GcRewriteMax
            } else {
                CompactPlan::NoOp
            }
        },
    }
}

/// Executable decision — must match `pedradb_core::compact_kernel::
/// compact_pick` bit-for-bit. Caller invariant: a found level is below
/// max, so `from + 1` cannot overflow.
#[verifier::when_used_as_spec(compact_pick_spec)]
pub fn compact_pick(
    lowest_level_with_files: Option<u32>,
    files_at_max_level: bool,
    gc_requested: bool,
) -> (p: CompactPlan)
    requires
        match lowest_level_with_files {
            Option::Some(l) => l < 0xffff_ffffu32,
            Option::None => true,
        },
    ensures
        p == compact_pick_spec(lowest_level_with_files, files_at_max_level, gc_requested),
        match p {
            CompactPlan::Merge { from: f, to: t } => t == f + 1,
            _ => true,
        },
        p == CompactPlan::GcRewriteMax
            ==> (lowest_level_with_files.is_none() && gc_requested && files_at_max_level),
        p == CompactPlan::NoOp
            ==> (lowest_level_with_files.is_none() && !(gc_requested && files_at_max_level)),
{
    match lowest_level_with_files {
        Option::Some(l) => CompactPlan::Merge { from: l, to: (l + 1) as u32 },
        Option::None => {
            if gc_requested && files_at_max_level {
                CompactPlan::GcRewriteMax
            } else {
                CompactPlan::NoOp
            }
        },
    }
}

/// Mirrors `VersionFate` in compact_kernel.rs.
pub enum VersionFate {
    Keep,
    Drop,
}

/// Closed-form spec — same arms as `compact_kernel::point_version_fate`.
pub open spec fn point_version_spec(
    this_seq: u64,
    newer_kept_seq: Option<u64>,
    oldest_snapshot: u64,
) -> VersionFate {
    match newer_kept_seq {
        Option::None => VersionFate::Keep,
        Option::Some(n) => {
            if n <= oldest_snapshot {
                VersionFate::Drop
            } else {
                VersionFate::Keep
            }
        },
    }
}

/// AS-IS silent-wrong: drop a version by its own sequence (below the
/// watermark), ignoring the newer sibling — compacts over a pinned
/// snapshot.
pub open spec fn point_version_as_is(
    this_seq: u64,
    newer_kept_seq: Option<u64>,
    oldest_snapshot: u64,
) -> VersionFate {
    match newer_kept_seq {
        Option::None => VersionFate::Keep,
        Option::Some(_) => {
            if this_seq <= oldest_snapshot {
                VersionFate::Drop
            } else {
                VersionFate::Keep
            }
        },
    }
}

/// Executable decision — must match `pedradb_core::compact_kernel::
/// point_version_fate` bit-for-bit.
#[verifier::when_used_as_spec(point_version_spec)]
pub fn point_version_fate(
    this_seq: u64,
    newer_kept_seq: Option<u64>,
    oldest_snapshot: u64,
) -> (f: VersionFate)
    ensures
        f == point_version_spec(this_seq, newer_kept_seq, oldest_snapshot),
        f == VersionFate::Keep
            ==> (newer_kept_seq.is_none()
                || (match newer_kept_seq {
                    Option::Some(n) => n > oldest_snapshot,
                    Option::None => true,
                })),
        f == VersionFate::Drop
            ==> (newer_kept_seq.is_some()
                && (match newer_kept_seq {
                    Option::Some(n) => n <= oldest_snapshot,
                    Option::None => false,
                })),
        (newer_kept_seq.is_some()
            && (match newer_kept_seq {
                Option::Some(n) => n > oldest_snapshot,
                Option::None => false,
            })) ==> f == VersionFate::Keep,
{
    match newer_kept_seq {
        Option::None => VersionFate::Keep,
        Option::Some(n) => {
            if n <= oldest_snapshot {
                VersionFate::Drop
            } else {
                VersionFate::Keep
            }
        },
    }
}

/// Closed-form spec — same arms as `compact_kernel::lone_tombstone_fate`.
pub open spec fn lone_tombstone_spec(bottommost: bool, lone_newest_tombstone: bool) -> VersionFate {
    if bottommost && lone_newest_tombstone {
        VersionFate::Drop
    } else {
        VersionFate::Keep
    }
}

/// AS-IS F177 violation: drop the lone tombstone regardless of bottommost.
pub open spec fn lone_tombstone_as_is(bottommost: bool, lone_newest_tombstone: bool) -> VersionFate {
    if lone_newest_tombstone {
        VersionFate::Drop
    } else {
        VersionFate::Keep
    }
}

#[verifier::when_used_as_spec(lone_tombstone_spec)]
pub fn lone_tombstone_fate(bottommost: bool, lone_newest_tombstone: bool) -> (f: VersionFate)
    ensures
        f == lone_tombstone_spec(bottommost, lone_newest_tombstone),
        f == VersionFate::Drop ==> (bottommost && lone_newest_tombstone),
        !bottommost ==> f == VersionFate::Keep,
{
    if bottommost && lone_newest_tombstone {
        VersionFate::Drop
    } else {
        VersionFate::Keep
    }
}

/// P0.3 named lemma (crash dictionary): a drop only happens when the newer
/// sibling is visible to every open snapshot — no open snapshot can ever
/// read the dropped version.
proof fn lemma_drop_needs_newer_visible_to_all_snaps(
    this_seq: u64,
    newer_seq: u64,
    oldest_snapshot: u64,
)
    requires
        this_seq < newer_seq,
    ensures
        point_version_fate(this_seq, Option::Some(newer_seq), oldest_snapshot)
            == VersionFate::Drop
            ==> newer_seq <= oldest_snapshot,
        point_version_fate(this_seq, Option::Some(newer_seq), oldest_snapshot)
            == VersionFate::Keep
            ==> newer_seq > oldest_snapshot,
{
}

/// P0.3 named lemma (pinned snapshot): when the newest version ≤ oldest is
/// still needed — the newer sibling is NOT visible to the oldest open
/// snapshot — the version is kept (compaction never walks over a pin).
proof fn lemma_snapshot_between_versions_keeps(
    this_seq: u64,
    newer_seq: u64,
    oldest_snapshot: u64,
)
    requires
        this_seq <= oldest_snapshot,
        this_seq < newer_seq,
        newer_seq > oldest_snapshot,
    ensures
        point_version_fate(this_seq, Option::Some(newer_seq), oldest_snapshot)
            == VersionFate::Keep,
{
}

/// P0.3 named lemma: the newest version of a key is always kept.
proof fn lemma_newest_version_always_kept(this_seq: u64, oldest_snapshot: u64)
    ensures
        point_version_fate(this_seq, Option::None, oldest_snapshot) == VersionFate::Keep,
{
}

/// P0.3 named lemma (F177): a partial compaction (not bottommost) never
/// drops a lone tombstone — the older version outside the input would
/// resurrect.
proof fn lemma_partial_compact_keeps_tombstone(lone_newest_tombstone: bool)
    ensures
        lone_tombstone_fate(false, lone_newest_tombstone) == VersionFate::Keep,
{
}

/// P0.3 named lemma (trigger): a merge moves exactly one level down.
proof fn lemma_merge_moves_one_level_down(from: u32, files_at_max: bool, gc: bool)
    requires
        from < 0xffff_ffff,
    ensures
        match compact_pick(Option::Some(from), files_at_max, gc) {
            CompactPlan::Merge { from: f, to: t } => t == f + 1,
            _ => true,
        },
{
}

/// Teeth: the AS-IS drop-under-snapshot mutant drops exactly the version a
/// snapshot pinned at `oldest_snapshot` still reads (this ≤ oldest <
/// newer) — the fixed kernel keeps it.
proof fn lemma_mutant_drops_pinned_version(
    this_seq: u64,
    newer_seq: u64,
    oldest_snapshot: u64,
)
    requires
        this_seq <= oldest_snapshot,
        this_seq < newer_seq,
        newer_seq > oldest_snapshot,
    ensures
        point_version_fate(this_seq, Option::Some(newer_seq), oldest_snapshot)
            == VersionFate::Keep,
        point_version_as_is(this_seq, Option::Some(newer_seq), oldest_snapshot)
            == VersionFate::Drop,
{
}

/// Teeth: the AS-IS ignore-bottommost mutant drops the lone tombstone in a
/// partial compaction — the resurrection the fixed kernel refuses (F177).
proof fn lemma_mutant_resurrects_over_partial_compact()
    ensures
        lone_tombstone_fate(false, true) == VersionFate::Keep,
        lone_tombstone_as_is(false, true) == VersionFate::Drop,
{
}

pub open spec fn gc_oldest_from_pin_spec(oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> u64 {
    match oldest_pin {
        Some(p) => p,
        None => if last_seq < visible_seq { last_seq } else { visible_seq },
    }
}

pub open spec fn gc_oldest_from_pin_as_is_spec(_oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> u64 {
    if last_seq < visible_seq { last_seq } else { visible_seq }
}

pub fn gc_oldest_from_pin(oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> (o: u64)
    ensures
        o == gc_oldest_from_pin_spec(oldest_pin, last_seq, visible_seq),
{
    match oldest_pin {
        Some(p) => p,
        None => last_seq.min(visible_seq),
    }
}

pub fn gc_oldest_from_pin_as_is(_oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> (o: u64)
    ensures
        o == gc_oldest_from_pin_as_is_spec(_oldest_pin, last_seq, visible_seq),
{
    last_seq.min(visible_seq)
}

/// RFC-0150 P2b: a live pin is the oldest_snapshot bound; a version the pin
/// still reads is Keep. AS-IS ignores the pin and Drops it.
proof fn lemma_pin_keeps_version_as_is_drops(
    this_seq: u64,
    newer_seq: u64,
    pin: u64,
    last_seq: u64,
    visible_seq: u64,
)
    requires
        this_seq < newer_seq,
        this_seq <= pin,
        newer_seq > pin,
        visible_seq >= newer_seq,
        last_seq >= visible_seq,
    ensures
        gc_oldest_from_pin_spec(Some(pin), last_seq, visible_seq) == pin,
        point_version_spec(this_seq, Some(newer_seq), pin) == VersionFate::Keep,
        point_version_spec(
            this_seq,
            Some(newer_seq),
            gc_oldest_from_pin_as_is_spec(Some(pin), last_seq, visible_seq),
        ) == VersionFate::Drop,
{
}
} // verus!

/// Target size of one merged compaction output SST (the Rocks
/// `target_file_size_base` role). The SST writer buffers one output
/// file's compressed bytes in memory before the final write, so merging
/// every input into a single file puts the whole dataset in RAM: a full
/// compact OOMed a 4 GiB guest at 2M entries (620 MB dataset, +1.1 GB
/// during compact) and only survived on the 128 GiB host as a ~10 GB
/// in-memory file. Splitting the sorted merge stream at this bound keeps
/// the writer's buffer bounded and gives compaction file granularity.
/// Splits fall between user keys, so every output file holds a disjoint
/// contiguous key range.
#[cfg(not(verus_keep_ghost))]
pub const COMPACT_TARGET_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Whether a merged-output chunk that has accumulated `written_bytes`
/// should split before the next entry at `target` bytes (pure policy twin
/// for the kernel test; the streaming split also waits for a user-key
/// boundary).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn compact_should_split_at(written_bytes: u64, target: u64) -> bool {
    written_bytes >= target
}

/// [`compact_should_split_at`] at [`COMPACT_TARGET_FILE_BYTES`].
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn compact_should_split(written_bytes: u64) -> bool {
    compact_should_split_at(written_bytes, COMPACT_TARGET_FILE_BYTES)
}

/// What one compaction run does.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn compact_pick_as_is(
    _lowest_level_with_files: Option<u32>,
    _files_at_max_level: bool,
    _gc_requested: bool,
    _max_level: u32,
) -> CompactPlan {
    CompactPlan::NoOp
}

/// What happens to one version of a user key during GC compaction.
#[cfg(not(verus_keep_ghost))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn gc_oldest_from_pin(oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> u64 {
    match oldest_pin {
        Some(p) => p,
        None => last_seq.min(visible_seq),
    }
}

/// AS-IS: ignore the pin (compact over a live snapshot).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn gc_oldest_from_pin_as_is(_oldest_pin: Option<u64>, last_seq: u64, visible_seq: u64) -> u64 {
    last_seq.min(visible_seq)
}

#[cfg(not(verus_keep_ghost))]
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

    #[test]
    fn compact_should_split_bounds_one_output_file() {
        assert!(!compact_should_split(0), "empty chunk never splits");
        assert!(
            !compact_should_split(COMPACT_TARGET_FILE_BYTES - 1),
            "below target keeps merging"
        );
        assert!(
            compact_should_split(COMPACT_TARGET_FILE_BYTES),
            "at target the next entry starts a new file"
        );
        assert_eq!(COMPACT_TARGET_FILE_BYTES, 256 * 1024 * 1024);
        assert!(compact_should_split_at(1_024, 1_024));
        assert!(!compact_should_split_at(1_023, 1_024));
    }
}
