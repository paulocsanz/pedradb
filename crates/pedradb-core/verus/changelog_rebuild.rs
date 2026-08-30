// Verus proof of the changelog SST-rebuild gate (RFC-0002 P22 / F53).
// Twin of `changelog_kernel::changelog_needs_sst_rebuild`.
//
//   ./scripts/verus_changelog_rebuild.sh

use vstd::prelude::*;

verus! {

pub open spec fn changelog_needs_sst_rebuild_spec(feed_empty: bool, last_sequence: u64) -> bool {
    feed_empty && last_sequence > 0
}

/// Rebuild last-per-key from Mem ∪ SST iff the loaded+WAL feed is empty
/// and the DB already has a durable sequence.
pub fn changelog_needs_sst_rebuild(feed_empty: bool, last_sequence: u64) -> (d: bool)
    ensures
        d == changelog_needs_sst_rebuild_spec(feed_empty, last_sequence),
        d ==> feed_empty,
        d ==> last_sequence > 0,
{
    feed_empty && last_sequence > 0
}

/// AS-IS F53: WAL-only — never rebuild from SST/Mem.
pub open spec fn changelog_needs_sst_rebuild_as_is(_feed_empty: bool, _last_sequence: u64) -> bool {
    false
}

/// The REAL: empty feed + live sequence rebuilds in FIXED, not in AS-IS.
proof fn lemma_as_is_misses_empty_feed_with_seq()
    ensures
        changelog_needs_sst_rebuild_spec(true, 1),
        !changelog_needs_sst_rebuild_as_is(true, 1),
{
}

/// Fresh DB (no durable sequence) never rebuilds.
proof fn lemma_fresh_db_no_rebuild()
    ensures
        !changelog_needs_sst_rebuild_spec(true, 0),
{
}

/// A live feed is not replaced by an SST last-per-key rebuild.
proof fn lemma_live_feed_no_rebuild()
    ensures
        !changelog_needs_sst_rebuild_spec(false, 99),
{
}

} // verus!
