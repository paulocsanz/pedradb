//! Pure changelog SST-rebuild gate (RFC-0002 P22 / F53).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_changelog_rebuild.sh
//!
//! Production [`crate::db::Db::maybe_rebuild_feed_from_live`] calls this.
//! Scan of MemTable ∪ SSTs and persist of `CHANGELOG` are caller + axiom.

#![forbid(unsafe_code)]

macro_rules! changelog_needs_sst_rebuild_body {
    ($feed_empty:expr, $last_sequence:expr) => {
        $feed_empty && $last_sequence > 0
    };
}

macro_rules! changelog_needs_sst_rebuild_as_is_body {
    ($feed_empty:expr, $last_sequence:expr) => {{
        let _ = ($feed_empty, $last_sequence);
        false
    }};
}

macro_rules! changelog_should_store_body {
    ($commits_since:expr, $interval:expr) => {
        $interval > 0 && $commits_since >= $interval
    };
}

macro_rules! changelog_should_store_as_is_body {
    ($commits_since:expr, $interval:expr) => {{
        let _ = $interval;
        $commits_since >= 1
    }};
}

macro_rules! changelog_rebuild_within_budget_body {
    ($live_entries:expr, $budget_entries:expr) => {
        $live_entries <= $budget_entries
    };
}

macro_rules! changelog_rebuild_within_budget_as_is_body {
    ($live_entries:expr, $budget_entries:expr) => {{
        let _ = ($live_entries, $budget_entries);
        true
    }};
}

/// Rebuild a last-per-key feed from MemTable ∪ SSTs when the loaded+WAL
/// changelog is empty but the DB already has a durable sequence.
///
/// After `flush` the WAL is truncated; a missing `CHANGELOG` must not leave
/// fold/journal with `changes_after(0) == []` while SST keys are live.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_needs_sst_rebuild(feed_empty: bool, last_sequence: u64) -> bool {
    changelog_needs_sst_rebuild_body!(feed_empty, last_sequence)
}

/// AS-IS F53: WAL-only rebuild — never consult SST/Mem even when the feed
/// is empty after a truncated WAL.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_needs_sst_rebuild_as_is(feed_empty: bool, last_sequence: u64) -> bool {
    changelog_needs_sst_rebuild_as_is_body!(feed_empty, last_sequence)
}

/// Default durable-commit interval between CHANGELOG cache stores (RFC-0031).
pub const DEFAULT_CHANGELOG_INTERVAL: u64 = 64;

/// Whether the commit path should persist the CHANGELOG cache (RFC-0031 P0.1).
///
/// The on-disk CHANGELOG is a cache rebuilt from WAL (RFC-0019). Persisting
/// it is never a durability gate. `interval == 0` means never on the commit
/// path (flush / close / checkpoint still force a store). `interval >= 1`
/// persists when `commits_since >= interval`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_should_store(commits_since: u64, interval: u64) -> bool {
    changelog_should_store_body!(commits_since, interval)
}

/// AS-IS RFC-0031: every durable commit stores (pre-debounce).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_should_store_as_is(commits_since: u64, interval: u64) -> bool {
    changelog_should_store_as_is_body!(commits_since, interval)
}

/// Default lazy-feed rebuild budget in entries. Above it the explicit
/// flush/checkpoint/close store leaves the CHANGELOG cache stale instead of
/// materializing MemTable ∪ SSTs (~3 live-set copies: BTreeMap + sorted Vec
/// + encode buffer). The on-disk CHANGELOG is a cache (RFC-0019) — the feed
/// is rebuilt from WAL / live on demand, so this is a memory bound, not a
/// durability gate.
pub const DEFAULT_CHANGELOG_REBUILD_BUDGET_ENTRIES: u64 = 100_000;

/// Lazy-feed rebuild gate: materialize the live set into the CHANGELOG cache
/// only while the live entry count stays within `budget_entries`
/// (RFC-0039 P0.3 / RFC-0041 P1.1 — flush stays O(write buffer), not
/// O(live set)).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_rebuild_within_budget(live_entries: u64, budget_entries: u64) -> bool {
    changelog_rebuild_within_budget_body!(live_entries, budget_entries)
}

/// AS-IS: always materialize — the 25M OOM dente (guest settle flush held
/// ~3× live set; killed at 3.3 GB for a 0.61 GiB store).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_rebuild_within_budget_as_is(live_entries: u64, budget_entries: u64) -> bool {
    changelog_rebuild_within_budget_as_is_body!(live_entries, budget_entries)
}

// ── RFC-0217 P1.1: amortized explicit-flush store (WAL archive) ──────────
//
// The lazy feed (interval 0) made every explicit flush an O(live-set)
// CHANGELOG rebuild + full-file rewrite (kafka_changelog_flush 0.036×).
// Amortization: while the on-disk cache lags, a WAL rotate **archives** the
// segment (`WAL.archNNNN`) instead of truncating it, so reopen can extend
// the feed from the archives (same WAL-frame replay, feed-only). The full
// store then fires only on the debounce/cap gate below — flush stays
// O(write buffer) on every path, and feed content after a crash is exactly
// what a synchronous store would have written.

/// Explicit flushes between forced CHANGELOG stores when the feed is lazy
/// (interval 0). Between stores the rotated WAL segments are archived as
/// the crash rebuild source, so this bounds cache staleness, not
/// durability (the archive is the source).
pub const DEFAULT_CHANGELOG_FLUSH_DEBOUNCE_FLUSHES: u64 = 64;

/// Archived WAL segments kept before a rotate forces a synchronous store.
/// Bounds crash-recovery replay cost and archived bytes on disk.
pub const DEFAULT_WAL_ARCHIVE_SEGMENT_CAP: u64 = 64;

/// Archived segments one store point may unlink (RFC-0217 P1.1). The gate
/// already pays the publish + store; the chain's unlink cost drains over
/// the following stores instead of landing as one burst.
pub const WAL_ARCHIVE_UNLINK_BUDGET: u64 = 4;

macro_rules! changelog_flush_store_now_body {
    ($disk_behind:expr, $flushes_since_store:expr, $debounce_flushes:expr, $archives:expr, $archive_cap:expr) => {
        $disk_behind
            && ($flushes_since_store >= $debounce_flushes || $archives >= $archive_cap)
    };
}

macro_rules! changelog_flush_store_now_as_is_body {
    ($disk_behind:expr, $flushes_since_store:expr, $debounce_flushes:expr, $archives:expr, $archive_cap:expr) => {{
        let _ = ($flushes_since_store, $debounce_flushes, $archives, $archive_cap);
        $disk_behind
    }};
}

macro_rules! wal_rotate_archives_body {
    ($disk_behind:expr, $archives:expr, $archive_cap:expr) => {
        $disk_behind && $archives < $archive_cap
    };
}

macro_rules! wal_rotate_archives_as_is_body {
    ($disk_behind:expr, $archives:expr, $archive_cap:expr) => {{
        let _ = ($disk_behind, $archives, $archive_cap);
        false
    }};
}

/// Whether an explicit flush must store the CHANGELOG cache now (lazy
/// feed). `disk_behind` is `changelog_disk_watermark < last_sequence`.
/// Stores at the flush debounce or when the archive chain is full — both
/// bounds are amortization knobs; the archived segments already carry the
/// feed content through a crash.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_flush_store_now(
    disk_behind: bool,
    flushes_since_store: u64,
    debounce_flushes: u64,
    archives: u64,
    archive_cap: u64,
) -> bool {
    changelog_flush_store_now_body!(
        disk_behind,
        flushes_since_store,
        debounce_flushes,
        archives,
        archive_cap
    )
}

/// AS-IS F212: every behind explicit flush stores synchronously.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_flush_store_now_as_is(
    disk_behind: bool,
    flushes_since_store: u64,
    debounce_flushes: u64,
    archives: u64,
    archive_cap: u64,
) -> bool {
    changelog_flush_store_now_as_is_body!(
        disk_behind,
        flushes_since_store,
        debounce_flushes,
        archives,
        archive_cap
    )
}

/// Whether a WAL rotate (feed lazy, on-disk cache behind) archives the
/// segment instead of truncating. False at the cap: the caller then stores
/// the CHANGELOG synchronously (covering every archived segment), deletes
/// the archives and truncates.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn wal_rotate_archives(disk_behind: bool, archives: u64, archive_cap: u64) -> bool {
    wal_rotate_archives_body!(disk_behind, archives, archive_cap)
}

/// AS-IS: rotate always truncates — the rebuild source is dropped and the
/// flush must pay the synchronous store (kafka_changelog_flush 0.036×).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn wal_rotate_archives_as_is(disk_behind: bool, archives: u64, archive_cap: u64) -> bool {
    wal_rotate_archives_as_is_body!(disk_behind, archives, archive_cap)
}

// RFC-0217 P1.1: the archived-segment file names (`WAL.archNNNN`) live in
// db.rs (`wal_archive_slot_name`/`wal_archive_slot_of`) — they are I/O
// naming, not decision logic, and `str::pattern`/`format!` machinery is
// outside what the aeneas/charon extraction lane translates.

// ── RFC-0219 P0.1: durable-commit CHANGELOG fate (trampoline pull) ──────
//
// `commit_ops_with` used to resolve inline whether a finished WAL commit
// was made durable and only then count it toward the CHANGELOG debounce.
// That fate is now this named kernel — the write-admission sync
// resolution (client flag wins, else the DB default) decided HERE; the
// store itself is trampoline I/O. The db.rs caller `match`es the plan.

/// Fate of the CHANGELOG debounce for one finished WAL commit.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangelogCommitFate {
    /// Durable commit: count it toward the debounce (may store the cache).
    Count,
    /// No barrier: skip the count — the cache lags and reopen rebuilds
    /// the feed from the WAL (the CHANGELOG is a cache, RFC-0019).
    Skip,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_durable_commit_fate(
    client_set: bool,
    client_sync: bool,
    db_sync: bool,
) -> ChangelogCommitFate {
    if client_set {
        if client_sync {
            ChangelogCommitFate::Count
        } else {
            ChangelogCommitFate::Skip
        }
    } else if db_sync {
        ChangelogCommitFate::Count
    } else {
        ChangelogCommitFate::Skip
    }
}

/// AS-IS: durable commits never count — the cache only ever stores at
/// flush/close, so every crash pays the full WAL replay (dente).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_durable_commit_fate_as_is(
    _client_set: bool,
    _client_sync: bool,
    _db_sync: bool,
) -> ChangelogCommitFate {
    ChangelogCommitFate::Skip
}

/// RFC-0219 P1.4: fate of the synchronous store point (RFC-0217 P1.1).
/// The deferred MANIFEST publish must cover the archived window before
/// the store may delete those segments.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangelogStorePlan {
    /// Durable MANIFEST publish covered the window — store the feed.
    StoreFeed,
    /// Publish failed/deferred — the archived segments are the only
    /// durable copy; skip the store.
    SkipStorePublishHolds,
}

#[cfg(not(verus_keep_ghost))]
/// Store the feed EXACTLY when the durable manifest publish is ok.
#[must_use]
pub fn changelog_store_plan(publish_ok: bool) -> ChangelogStorePlan {
    if publish_ok {
        ChangelogStorePlan::StoreFeed
    } else {
        ChangelogStorePlan::SkipStorePublishHolds
    }
}

/// AS-IS: stores even when the publish failed — the store deletes
/// archived segments no published MANIFEST covers (dente).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn changelog_store_plan_as_is(_publish_ok: bool) -> ChangelogStorePlan {
    ChangelogStorePlan::StoreFeed
}

/// RFC-0219 P0.2: fate of the archived WAL chain at a delete point.
/// A CHANGELOG watermark only proves the *cache* is current — while the
/// deferred MANIFEST publish lags the archives, the segments above
/// `manifest_published_seq` are the only durable copy of their window,
/// so they are kept; a publish that covers the chain frees the delete.
#[cfg(not(verus_keep_ghost))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalArchiveDelete {
    /// Manifest publish lags the archives — keep every segment.
    KeepUntilPublished,
    /// Publish covers the chain — delete (budgeted by the caller).
    DeleteCovered,
}

#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn wal_archive_delete_plan(
    manifest_published_seq: u64,
    wal_archive_max_seq: u64,
) -> WalArchiveDelete {
    if manifest_published_seq < wal_archive_max_seq {
        WalArchiveDelete::KeepUntilPublished
    } else {
        WalArchiveDelete::DeleteCovered
    }
}

/// AS-IS: delete covered-or-not — the un-published window's only durable
/// copy is unlinked (data-loss window dente).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn wal_archive_delete_plan_as_is(
    _manifest_published_seq: u64,
    _wal_archive_max_seq: u64,
) -> WalArchiveDelete {
    WalArchiveDelete::DeleteCovered
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn changelog_needs_sst_rebuild_spec(feed_empty: bool, last_sequence: u64) -> bool {
    feed_empty && last_sequence > 0
}

pub fn changelog_needs_sst_rebuild(feed_empty: bool, last_sequence: u64) -> (d: bool)
    ensures
        d == changelog_needs_sst_rebuild_spec(feed_empty, last_sequence),
        d ==> feed_empty,
        d ==> last_sequence > 0,
{
    changelog_needs_sst_rebuild_body!(feed_empty, last_sequence)
}

pub open spec fn changelog_needs_sst_rebuild_as_is_spec(_feed_empty: bool, _last_sequence: u64) -> bool {
    false
}

pub fn changelog_needs_sst_rebuild_as_is(feed_empty: bool, last_sequence: u64) -> (d: bool)
    ensures
        d == false,
        d == changelog_needs_sst_rebuild_as_is_spec(feed_empty, last_sequence),
{
    changelog_needs_sst_rebuild_as_is_body!(feed_empty, last_sequence)
}

proof fn lemma_as_is_misses_empty_feed_with_seq()
    ensures
        changelog_needs_sst_rebuild_spec(true, 1),
        !changelog_needs_sst_rebuild_as_is_spec(true, 1),
{
}

proof fn lemma_fresh_db_no_rebuild()
    ensures
        !changelog_needs_sst_rebuild_spec(true, 0),
{
}

proof fn lemma_live_feed_no_rebuild()
    ensures
        !changelog_needs_sst_rebuild_spec(false, 99),
{
}

pub fn changelog_should_store(commits_since: u64, interval: u64) -> (d: bool)
    ensures
        d == (interval > 0 && commits_since >= interval),
{
    changelog_should_store_body!(commits_since, interval)
}

pub fn changelog_should_store_as_is(commits_since: u64, interval: u64) -> (d: bool)
    ensures
        d == (commits_since >= 1),
{
    changelog_should_store_as_is_body!(commits_since, interval)
}

pub fn changelog_rebuild_within_budget(live_entries: u64, budget_entries: u64) -> (d: bool)
    ensures
        d == (live_entries <= budget_entries),
{
    changelog_rebuild_within_budget_body!(live_entries, budget_entries)
}

pub fn changelog_rebuild_within_budget_as_is(live_entries: u64, budget_entries: u64) -> (d: bool)
    ensures
        d == true,
{
    changelog_rebuild_within_budget_as_is_body!(live_entries, budget_entries)
}

pub open spec fn changelog_flush_store_now_spec(
    disk_behind: bool, flushes_since_store: u64, debounce_flushes: u64,
    archives: u64, archive_cap: u64,
) -> bool {
    disk_behind
        && (flushes_since_store >= debounce_flushes || archives >= archive_cap)
}

pub open spec fn wal_rotate_archives_spec(
    disk_behind: bool, archives: u64, archive_cap: u64,
) -> bool {
    disk_behind && archives < archive_cap
}

pub fn changelog_flush_store_now(
    disk_behind: bool, flushes_since_store: u64, debounce_flushes: u64,
    archives: u64, archive_cap: u64,
) -> (d: bool)
    ensures
        d == changelog_flush_store_now_spec(
            disk_behind, flushes_since_store, debounce_flushes, archives, archive_cap
        ),
        d ==> disk_behind,
{
    changelog_flush_store_now_body!(
        disk_behind, flushes_since_store, debounce_flushes, archives, archive_cap
    )
}

pub fn wal_rotate_archives(disk_behind: bool, archives: u64, archive_cap: u64) -> (d: bool)
    ensures
        d == wal_rotate_archives_spec(disk_behind, archives, archive_cap),
        d ==> disk_behind,
        d ==> archives < archive_cap,
{
    wal_rotate_archives_body!(disk_behind, archives, archive_cap)
}

proof fn lemma_archive_chain_forces_store()
    ensures
        changelog_flush_store_now_spec(true, 0, 64, 64, 64),
        changelog_flush_store_now_spec(true, 64, 64, 0, 64),
        !changelog_flush_store_now_spec(true, 63, 64, 63, 64),
        !changelog_flush_store_now_spec(false, u64::MAX, 64, u64::MAX, 64),
{
}

proof fn lemma_full_chain_never_archives()
    ensures
        !wal_rotate_archives_spec(true, 64, 64),
        wal_rotate_archives_spec(true, 63, 64),
        !wal_rotate_archives_spec(false, 0, 64),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_when_feed_empty_and_seq_live() {
        assert!(changelog_needs_sst_rebuild(true, 1));
        assert!(changelog_needs_sst_rebuild(true, u64::MAX));
        assert!(!changelog_needs_sst_rebuild_as_is(true, 1));
    }

    #[test]
    fn skip_fresh_db() {
        assert!(!changelog_needs_sst_rebuild(true, 0));
        assert!(!changelog_needs_sst_rebuild(false, 0));
    }

    #[test]
    fn skip_when_feed_already_has_entries() {
        assert!(!changelog_needs_sst_rebuild(false, 5));
        assert!(!changelog_needs_sst_rebuild(false, u64::MAX));
    }

    #[test]
    fn theorem_on_small_domain() {
        let mut n = 0u32;
        for feed_empty in [false, true] {
            for last in 0u64..8 {
                let d = changelog_needs_sst_rebuild(feed_empty, last);
                assert_eq!(d, feed_empty && last > 0);
                assert!(!changelog_needs_sst_rebuild_as_is(feed_empty, last));
                if d {
                    assert!(feed_empty);
                    assert!(last > 0);
                    assert_ne!(d, changelog_needs_sst_rebuild_as_is(feed_empty, last));
                }
                n += 1;
            }
        }
        assert_eq!(n, 2 * 8);
    }

    #[test]
    fn rebuild_budget_bounds_materialization() {
        assert!(changelog_rebuild_within_budget(0, 100));
        assert!(changelog_rebuild_within_budget(100, 100));
        assert!(!changelog_rebuild_within_budget(101, 100));
        assert!(!changelog_rebuild_within_budget(
            25_000_000,
            DEFAULT_CHANGELOG_REBUILD_BUDGET_ENTRIES
        ));
    }

    #[test]
    fn rebuild_budget_as_is_always_materializes() {
        assert!(changelog_rebuild_within_budget_as_is(
            25_000_000,
            DEFAULT_CHANGELOG_REBUILD_BUDGET_ENTRIES
        ));
    }

    #[test]
    fn flush_store_debounces_until_gate() {
        // Fresh disk cache: nothing to store even at the debounce.
        assert!(!changelog_flush_store_now(false, 100, 64, 0, 64));
        // Behind, below both bounds: defer (the archive carries the feed).
        assert!(!changelog_flush_store_now(true, 63, 64, 63, 64));
        // Debounce hit.
        assert!(changelog_flush_store_now(true, 64, 64, 0, 64));
        // Archive chain full forces the store even at flush 1.
        assert!(changelog_flush_store_now(true, 1, 64, 64, 64));
    }

    #[test]
    fn flush_store_as_is_stores_every_behind_flush() {
        assert!(changelog_flush_store_now_as_is(true, 1, 64, 0, 64));
        assert!(!changelog_flush_store_now_as_is(false, 99, 64, 63, 64));
    }

    #[test]
    fn rotate_archives_only_while_chain_has_room() {
        assert!(wal_rotate_archives(true, 0, 64));
        assert!(wal_rotate_archives(true, 63, 64));
        assert!(!wal_rotate_archives(true, 64, 64));
        assert!(!wal_rotate_archives(false, 0, 64));
        assert!(!wal_rotate_archives_as_is(true, 0, 64));
    }

    #[test]
    fn wal_rotate_and_store_gate_decisions() {
        // Naming helpers live in db.rs (I/O naming, not kernel decisions —
        // RFC-0217 P1.1); this pins the decisions that use them.
        for i in [0u64, 1, 9, 63, 999, 9999] {
            assert!(wal_rotate_archives(true, i, 64) == (i < 64));
            assert!(changelog_flush_store_now(true, 63, 64, i, 64) == (i >= 64));
        }
    }

    #[test]
    fn flush_store_theorem_on_small_domain() {
        for disk_behind in [false, true] {
            for flushes in 0u64..4 {
                for debounce in 1u64..4 {
                    for archives in 0u64..4 {
                        for cap in 1u64..4 {
                            let d = changelog_flush_store_now(
                                disk_behind, flushes, debounce, archives, cap,
                            );
                            assert_eq!(
                                d,
                                disk_behind
                                    && (flushes >= debounce || archives >= cap)
                            );
                            if d {
                                assert!(disk_behind);
                            }
                            let a = wal_rotate_archives(disk_behind, archives, cap);
                            assert_eq!(a, disk_behind && archives < cap);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn debounce_interval_zero_never_on_commit_path() {
        assert!(!changelog_should_store(0, 0));
        assert!(!changelog_should_store(1, 0));
        assert!(!changelog_should_store(u64::MAX, 0));
    }

    #[test]
    fn debounce_interval_one_is_every_durable_commit() {
        assert!(!changelog_should_store(0, 1));
        assert!(changelog_should_store(1, 1));
        assert!(changelog_should_store(2, 1));
    }

    #[test]
    fn debounce_default_fires_at_n() {
        assert!(!changelog_should_store(63, DEFAULT_CHANGELOG_INTERVAL));
        assert!(changelog_should_store(64, DEFAULT_CHANGELOG_INTERVAL));
        assert!(changelog_should_store(65, DEFAULT_CHANGELOG_INTERVAL));
    }

    #[test]
    fn debounce_as_is_ignores_interval() {
        assert!(!changelog_should_store_as_is(0, 64));
        assert!(changelog_should_store_as_is(1, 64));
        assert!(changelog_should_store_as_is(1, 0));
    }

    #[test]
    fn debounce_theorem_on_small_domain() {
        let mut n = 0u32;
        for interval in 0u64..8 {
            for since in 0u64..16 {
                let d = changelog_should_store(since, interval);
                assert_eq!(d, interval > 0 && since >= interval);
                let as_is = changelog_should_store_as_is(since, interval);
                assert_eq!(as_is, since >= 1);
                if interval == 0 {
                    assert!(!d);
                }
                n += 1;
            }
        }
        assert_eq!(n, 8 * 16);
    }

    const B_OPEN: u8 = 123;
    const B_CLOSE: u8 = 125;

    fn named_fn_src(src: &str, name: &str) -> Option<String> {
        let needle = format!("fn {}(", name);
        let start = src.find(&needle)?;
        let rest = &src[start..];
        let bytes = rest.as_bytes();
        let brace = bytes.iter().position(|&b| b == B_OPEN)?;
        let mut depth = 0i32;
        for (i, &b) in bytes[brace..].iter().enumerate() {
            if b == B_OPEN {
                depth += 1;
            } else if b == B_CLOSE {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[brace..=brace + i].to_string());
                }
            }
        }
        None
    }

    #[test]
    fn changelog_durable_commit_fate_on_live_client_sync_counts() {
        // RFC-0219 P0.1: client flag wins — explicit sync counts, explicit
        // async skips even when the DB default would sync.
        assert_eq!(
            changelog_durable_commit_fate(true, true, false),
            ChangelogCommitFate::Count
        );
        assert_eq!(
            changelog_durable_commit_fate(true, false, true),
            ChangelogCommitFate::Skip
        );
        // No client flag: the DB default decides.
        assert_eq!(
            changelog_durable_commit_fate(false, false, true),
            ChangelogCommitFate::Count
        );
        assert_eq!(
            changelog_durable_commit_fate(false, true, false),
            ChangelogCommitFate::Skip
        );
        // AS-IS dente: durable commits never count — every crash pays the
        // full WAL replay.
        assert_eq!(
            changelog_durable_commit_fate_as_is(true, true, true),
            ChangelogCommitFate::Skip
        );
        // Live: commit_ops_with matches the kernel plan; the changelog
        // debounce gate is no longer an inline sync-resolution if (the
        // do_sync resolution feeding wal_commit_plan stays — that is the
        // write-admission family's own call).
        let coc = named_fn_src(include_str!("db_kernel.rs"), "commit_ops_with").expect("commit_ops_with");
        assert!(
            coc.contains("match crate::changelog_kernel::changelog_durable_commit_fate("),
            "commit_ops_with must match changelog_durable_commit_fate"
        );
        assert!(
            coc.contains("ChangelogCommitFate::Count =>"),
            "the debounce count must live in the Count arm"
        );
        assert_eq!(
            coc.matches("maybe_persist_changelog_after_durable_commit").count(),
            1,
            "exactly one debounce call, inside the kernel arm"
        );
    }

    #[test]
    fn changelog_store_plan_on_live_failed_publish_skips() {
        // RFC-0219 P1.4: the store may delete archived segments only
        // after the durable MANIFEST publish covers them; a failed
        // publish holds the store (AS-IS deletes the only durable copy).
        assert_eq!(changelog_store_plan(true), ChangelogStorePlan::StoreFeed);
        assert_eq!(
            changelog_store_plan(false),
            ChangelogStorePlan::SkipStorePublishHolds
        );
        assert_eq!(
            changelog_store_plan_as_is(false),
            ChangelogStorePlan::StoreFeed,
            "AS-IS dente: stores with the publish failed"
        );
        let csp = named_fn_src(include_str!("db_kernel.rs"), "changelog_store_point")
            .expect("changelog_store_point");
        assert!(
            csp.contains("match crate::changelog_kernel::changelog_store_plan("),
            "changelog_store_point must match changelog_store_plan"
        );
        assert!(
            !csp.contains("persist_manifest_durable().is_ok() {"),
            "the raw publish gate left the trampoline"
        );
        // RFC-0219 P2.1 drain: the group_apply debounce matches the
        // same kernel fate as commit_ops_with (P0.1), no raw
        // wal_sync_required gate left in the trampoline.
        let ga = named_fn_src(include_str!("db_kernel.rs"), "group_apply").expect("group_apply");
        assert!(
            ga.contains("match crate::changelog_kernel::changelog_durable_commit_fate("),
            "group_apply matches changelog_durable_commit_fate"
        );
        assert!(
            !ga.contains("wal_sync_required(true, any_sync, false)"),
            "the raw debounce gate left the group_apply trampoline"
        );
    }

    #[test]
    fn wal_archive_delete_plan_on_live_unpublished_window_keeps() {
        // RFC-0219 P0.2: an unpublished archive window is the only durable
        // copy of its range — kept until the MANIFEST publish covers it.
        assert_eq!(
            wal_archive_delete_plan(3, 7),
            WalArchiveDelete::KeepUntilPublished
        );
        assert_eq!(
            wal_archive_delete_plan(7, 7),
            WalArchiveDelete::DeleteCovered
        );
        assert_eq!(
            wal_archive_delete_plan(9, 7),
            WalArchiveDelete::DeleteCovered
        );
        // AS-IS dente: deletes the un-published window's only durable copy.
        assert_eq!(
            wal_archive_delete_plan_as_is(3, 7),
            WalArchiveDelete::DeleteCovered
        );
        // Live: delete_wal_archives matches the kernel plan; the raw
        // seq comparison left the trampoline.
        let dwa = named_fn_src(include_str!("db_kernel.rs"), "delete_wal_archives")
            .expect("delete_wal_archives");
        assert!(
            dwa.contains("match crate::changelog_kernel::wal_archive_delete_plan("),
            "delete_wal_archives must match wal_archive_delete_plan"
        );
        assert!(
            !dwa.contains("manifest_published_seq < self.wal_archive_max_seq"),
            "delete_wal_archives must not keep the raw seq comparison inline"
        );
    }
}
