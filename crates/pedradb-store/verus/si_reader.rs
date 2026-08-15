// Verus proof of SI reader ranking (RFC-0002 P19 / F42).
// Twin of `src/si_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_si_reader.sh

use vstd::prelude::*;

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

pub open spec fn si_reader_beats_as_is(
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

proof fn lemma_as_is_keeps_first()
    ensures
        !si_reader_beats_as_is(true, true, false, 10, false, false, true, 1),
{
}

proof fn lemma_leader_beats_lagging_first()
    ensures
        si_reader_beats_spec(true, true, false, 10, false, false, true, 1),
{
}

pub open spec fn point_get_watermark_spec(range_applied: u64, _global_seq: u64) -> u64 {
    range_applied
}

pub open spec fn point_get_watermark_as_is_spec(_range_applied: u64, global_seq: u64) -> u64 {
    global_seq
}

/// F84: point get ranks by per-range applied, not Pedra last_sequence.
pub fn point_get_watermark(range_applied: u64, global_seq: u64) -> (w: u64)
    ensures
        w == point_get_watermark_spec(range_applied, global_seq),
        w == range_applied,
{
    let _ = global_seq;
    range_applied
}

pub fn point_get_watermark_as_is(range_applied: u64, global_seq: u64) -> (w: u64)
    ensures
        w == point_get_watermark_as_is_spec(range_applied, global_seq),
        w == global_seq,
{
    let _ = range_applied;
    global_seq
}

/// A node busy on another range (global_seq > applied) is ranked by applied.
proof fn lemma_as_is_ignores_lagging_range(applied: u64, global: u64)
    requires
        applied < global,
    ensures
        point_get_watermark_spec(applied, global) < point_get_watermark_as_is_spec(applied, global),
{
}

} // verus!
