// Verus proof of the compaction decision kernel
// (RFC-0056 P0.3 — crash dictionary on the merge/GC path).
//
// Source of truth for production remains `src/compact_kernel.rs`
// (called by `Db::compact_with_ssts_only` and `merge::gc_snapshot_safe`).
// This file is the machine-checked theorem: exec == spec, a version a
// pinned snapshot still reads is kept, a lone tombstone drops only on a
// bottommost rewrite (F177), merges move exactly one level down, and the
// AS-IS mutants (drop-under-pinned-snapshot / ignore-bottommost)
// diverge exactly where the fixed kernel refuses.
//
//   ./scripts/verus_compact_decision.sh
//
// Do not link this into the production crate — it is a twin of the pure kernel.

use vstd::prelude::*;

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
