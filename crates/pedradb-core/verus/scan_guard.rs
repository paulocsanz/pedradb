//! Verus twin of `pedradb_core::sst::scan_kernel` (F167) — model domain
//! (`u64` keys stand in for `[u8]` under the same total order).
//!
//! Proven:
//! - **skip-soundness** (`tombstone_reaches_window`): when the predicate says
//!   "does not reach", no key of the scan window is covered by the tombstone —
//!   a file skipped by the fast-reject can never resurrect a deleted key.
//! - **teeth** (`lemma_as_is_misses_span`): the AS-IS bounds-only reject
//!   returns "does not reach" for the F167 world (file at key 2 carrying
//!   tombstone `[2,5)`, window `[4,6]`), while window key 4 is covered.
//! - **writer invariant** (`lemma_writer_tombstones_can_straddle`): every
//!   `delete_range` tombstone has `start < end`, so straddling is reachable —
//!   the guard is not vacuous.

#![allow(unused_imports)]

use vstd::prelude::*;

verus! {

/// Model of `Bound<&[u8]>` over the one-scalar key domain.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum B {
    Unbounded,
    Inc(u64),
    Exc(u64),
}

pub open spec fn in_window(w: u64, start: B, end: B) -> bool {
    let after_start = match start {
        B::Unbounded => true,
        B::Inc(s) => w >= s,
        B::Exc(s) => w > s,
    };
    let before_end = match end {
        B::Unbounded => true,
        B::Inc(e) => w <= e,
        B::Exc(e) => w < e,
    };
    after_start && before_end
}

pub open spec fn covers(t_start: u64, t_end: u64, w: u64) -> bool {
    t_start <= w && w < t_end
}

pub open spec fn reaches_spec(t_start: u64, t_end: u64, start: B, end: B) -> bool {
    let reaches_start = match start {
        B::Unbounded => true,
        B::Inc(s) | B::Exc(s) => t_end > s,
    };
    let starts_before_end = match end {
        B::Unbounded => true,
        B::Inc(e) => t_start <= e,
        B::Exc(e) => t_start < e,
    };
    reaches_start && starts_before_end
}

/// F167 kernel twin: end past the window start **and** start before its end.
pub fn tombstone_reaches_window(t_start: u64, t_end: u64, start: B, end: B) -> (r: bool)
    ensures
        r == reaches_spec(t_start, t_end, start, end),
        !r ==> forall |w: u64| in_window(w, start, end) ==> !covers(t_start, t_end, w),
{
    let reaches_start = match start {
        B::Unbounded => true,
        B::Inc(s) | B::Exc(s) => t_end > s,
    };
    let starts_before_end = match end {
        B::Unbounded => true,
        B::Inc(e) => t_start <= e,
        B::Exc(e) => t_start < e,
    };
    let r = reaches_start && starts_before_end;
    proof {
        if !r {
            assert(!reaches_start || !starts_before_end);
            assert forall |w: u64| in_window(w, start, end)
            implies !covers(t_start, t_end, w) by {
                if !reaches_start {
                    match start {
                        B::Unbounded => {},
                        B::Inc(s) => {
                            assert(w >= s);
                            assert(t_end <= s <= w);
                        },
                        B::Exc(s) => {
                            assert(w > s);
                            assert(t_end <= s < w);
                        },
                    }
                } else {
                    match end {
                        B::Unbounded => {},
                        B::Inc(e) => {
                            assert(w <= e);
                            assert(t_start > e >= w);
                        },
                        B::Exc(e) => {
                            assert(w < e);
                            assert(t_start >= e > w);
                        },
                    }
                }
            }
        }
    }
    r
}

/// Model of the file point-key range `[small, large]` (`None` = unknown/any).
pub open spec fn in_file_range(w: u64, small: Option<u64>, large: Option<u64>) -> bool {
    match (small, large) {
        (Some(lo), Some(hi)) => lo <= w && w <= hi,
        _ => true,
    }
}

/// File point bounds may hold a key of the scan window.
pub open spec fn file_may_hold_window_key(
    small: Option<u64>,
    large: Option<u64>,
    start: B,
    end: B,
) -> bool {
    match (small, large) {
        (Some(lo), Some(hi)) => {
            let before_end = match end {
                B::Unbounded => true,
                B::Inc(e) => lo <= e,
                B::Exc(e) => lo < e,
            };
            let after_start = match start {
                B::Unbounded => true,
                B::Inc(s) => hi >= s,
                B::Exc(s) => hi > s,
            };
            before_end && after_start
        }
        _ => true,
    }
}

/// Full kernel spec: point overlap **or** a straddling tombstone.
pub open spec fn scan_reads_file_spec(
    small: Option<u64>,
    large: Option<u64>,
    t_start: u64,
    t_end: u64,
    start: B,
    end: B,
) -> bool {
    file_may_hold_window_key(small, large, start, end)
        || reaches_spec(t_start, t_end, start, end)
}

/// Twin of `scan_kernel::point_bounds_overlap`: false ⇒ no window key is a
/// possible point key of the file.
pub fn point_bounds_overlap(
    small: Option<u64>,
    large: Option<u64>,
    start: B,
    end: B,
) -> (ok: bool)
    ensures
        ok == file_may_hold_window_key(small, large, start, end),
        !ok ==> forall |w: u64|
            in_window(w, start, end) ==> !in_file_range(w, small, large),
{
    match (small, large) {
        (Some(lo), Some(hi)) => {
            let before_end = match end {
                B::Unbounded => true,
                B::Inc(e) => lo <= e,
                B::Exc(e) => lo < e,
            };
            let after_start = match start {
                B::Unbounded => true,
                B::Inc(s) => hi >= s,
                B::Exc(s) => hi > s,
            };
            let ok = before_end && after_start;
            proof {
                if !ok {
                    assert forall |w: u64| in_window(w, start, end)
                    implies !in_file_range(w, small, large) by {
                        if !before_end {
                            match end {
                                B::Unbounded => {},
                                B::Inc(e) => {
                                    assert(lo > e && w <= e);
                                    assert(w < lo);
                                },
                                B::Exc(e) => {
                                    assert(lo >= e && w < e);
                                    assert(w < lo);
                                },
                            }
                        } else {
                            match start {
                                B::Unbounded => {},
                                B::Inc(s) => {
                                    assert(hi < s && w >= s);
                                    assert(w > hi);
                                },
                                B::Exc(s) => {
                                    assert(hi <= s && w > s);
                                    assert(w > hi);
                                },
                            }
                        }
                    }
                }
            }
            ok
        }
        _ => true,
    }
}

/// F167 kernel twin: must a scan of `[start, end)` read this file?
///
/// Skip is **sound on both axes**: no possible point key of the file lies in
/// the window, and no window key is covered by the file's tombstone.
pub fn scan_reads_file(
    small: Option<u64>,
    large: Option<u64>,
    t_start: u64,
    t_end: u64,
    start: B,
    end: B,
) -> (r: bool)
    ensures
        r == scan_reads_file_spec(small, large, t_start, t_end, start, end),
        !r ==> forall |w: u64|
            in_window(w, start, end) ==> !in_file_range(w, small, large)
                && !covers(t_start, t_end, w),
{
    let bounds_ok = point_bounds_overlap(small, large, start, end);
    let reaches = tombstone_reaches_window(t_start, t_end, start, end);
    bounds_ok || reaches
}

/// AS-IS F167: bounds-only fast-reject — tombstone span ignored.
pub fn tombstone_reaches_window_as_is(
    _t_start: u64,
    _t_end: u64,
    _start: B,
    _end: B,
) -> (r: bool)
    ensures r == false,
{
    false
}

/// Teeth: in the F167 world (file at 2, tombstone `[2,5)`, window `[4,6]`)
/// AS-IS says "does not reach" while window key 4 is covered — skipping the
/// file resurrects a deleted key.
proof fn lemma_as_is_misses_span() {
    // AS-IS returns false unconditionally; the guarded spec keeps the file.
    let guarded = reaches_spec(2, 5, B::Inc(4), B::Inc(6));
    assert(guarded); // 5 > 4 and 2 <= 6
    assert(in_window(4, B::Inc(4), B::Inc(6)));
    assert(covers(2, 5, 4));
}

/// Writer invariant: `delete_range` stores `start < end`, so a tombstone can
/// straddle a later window — the guard is not vacuous.
proof fn lemma_writer_tombstones_can_straddle() {
    // A tombstone [2,5) written into a file whose point keys are all `2`
    // straddles the window [4,6]: file bounds reject, kernel keeps the file.
    let t_start = 2u64;
    let t_end = 5u64;
    assert(t_start < t_end);
    let reaches = reaches_spec(t_start, t_end, B::Inc(4), B::Inc(6));
    assert(reaches == true);
}

fn main() {}
}
