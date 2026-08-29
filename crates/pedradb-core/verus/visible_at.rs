// Verus twin of `src/merge.rs::visible_at` (RFC-0150 P1).
// Newest version with seq <= snapshot is a candidate; Value + not
// range-hidden is live. F30 AS-IS only matches the range start.
//
//   ./scripts/verus_visible_at.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

pub enum ValueType {
    Deletion,
    Value,
    RangeDeletion,
}

pub open spec fn visible_at_spec(kind: ValueType, range_hidden: bool) -> bool {
    match kind {
        ValueType::Value => !range_hidden,
        ValueType::Deletion | ValueType::RangeDeletion => false,
    }
}

/// F30-class leak: never hide (deleted keys scan as live).
pub open spec fn visible_at_as_is_spec(_kind: ValueType, _range_hidden: bool) -> bool {
    true
}

pub fn visible_at(kind: ValueType, range_hidden: bool) -> (d: bool)
    ensures
        d == visible_at_spec(kind, range_hidden),
{
    match kind {
        ValueType::Value => !range_hidden,
        ValueType::Deletion | ValueType::RangeDeletion => false,
    }
}

pub fn visible_at_as_is(_kind: ValueType, _range_hidden: bool) -> (d: bool)
    ensures
        d == true,
{
    true
}

/// Half-open cover (F30) — same tokens as `merge::range_tombstone_covers`.
pub open spec fn range_covers_spec(start: u64, end: u64, key: u64) -> bool {
    key >= start && key < end
}

pub fn range_tombstone_covers(start: u64, end: u64, key: u64) -> (d: bool)
    ensures
        d == range_covers_spec(start, end, key),
        d == (key >= start && key < end),
{
    key >= start && key < end
}

/// AS-IS F30: only the range start conflicts.
pub open spec fn range_tombstone_covers_as_is_spec(start: u64, _end: u64, key: u64) -> bool {
    key == start
}

pub fn range_tombstone_covers_as_is(start: u64, _end: u64, key: u64) -> (d: bool)
    ensures
        d == range_tombstone_covers_as_is_spec(start, _end, key),
        d == (key == start),
{
    key == start
}

proof fn lemma_deletion_is_hidden()
    ensures
        !visible_at_spec(ValueType::Deletion, false),
        visible_at_as_is_spec(ValueType::Deletion, false),
{
}

proof fn lemma_range_hidden_value_is_hidden()
    ensures
        !visible_at_spec(ValueType::Value, true),
        visible_at_spec(ValueType::Value, false),
{
}

/// Mid-range key is covered; AS-IS only matches the start (F30 tooth).
proof fn lemma_f30_as_is_misses_interior(start: u64, end: u64, key: u64)
    requires
        start < key,
        key < end,
    ensures
        range_covers_spec(start, end, key),
        !range_tombstone_covers_as_is_spec(start, end, key),
{
}

} // verus!
