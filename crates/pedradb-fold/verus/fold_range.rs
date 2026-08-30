// Verus proof of fold range-tombstone coverage (F169).
// Twin of `src/fold_kernel.rs::fold_event_hides_key`. Model domain: u64 keys.
//
//   ./scripts/verus_fold_range.sh

use vstd::prelude::*;

verus! {

pub open spec fn fold_event_hides_key_spec(
    is_range: bool,
    start: u64,
    end: u64,
    key: u64,
) -> bool {
    if is_range {
        key >= start && key < end
    } else {
        key == start
    }
}

/// F169 kernel twin: a range event hides `[start, end)`; a point event
/// hides only the exact key.
pub fn fold_event_hides_key(is_range: bool, start: u64, end: u64, key: u64) -> (r: bool)
    ensures
        r == fold_event_hides_key_spec(is_range, start, end, key),
        is_range && key >= start && key < end ==> r,
        is_range && (key < start || key >= end) ==> !r,
        !is_range ==> r == (key == start),
{
    if is_range {
        key >= start && key < end
    } else {
        key == start
    }
}

/// AS-IS F169: range delete hides only the start key.
pub fn fold_event_hides_key_as_is(_is_range: bool, start: u64, _end: u64, key: u64) -> (r: bool)
    ensures
        r == (key == start),
{
    key == start
}

/// Teeth: covered key 3 in `[2, 4)` is hidden by the kernel, not by AS-IS.
proof fn lemma_as_is_misses_cover() {
    assert(fold_event_hides_key_spec(true, 2, 4, 3));
    assert(!fold_event_hides_key_spec(true, 2, 4, 4));
    assert(fold_event_hides_key_spec(true, 2, 4, 2));
}

fn main() {}
}
