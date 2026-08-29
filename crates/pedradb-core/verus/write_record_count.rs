// Verus twin of `src/batch.rs::write_record_count_ok` (RFC-0150 P2a).
// Decode Ok ⇒ ops.len() == encoded count. AS-IS accepts a silent prefix.
//
//   ./scripts/verus_write_record_count.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

pub open spec fn write_record_count_ok_spec(count: u32, decoded_len: u32) -> bool {
    decoded_len == count
}

pub open spec fn write_record_count_ok_as_is_spec(_count: u32, _decoded_len: u32) -> bool {
    true
}

pub fn write_record_count_ok(count: u32, decoded_len: u32) -> (ok: bool)
    ensures
        ok == write_record_count_ok_spec(count, decoded_len),
        ok ==> decoded_len == count,
{
    decoded_len == count
}

pub fn write_record_count_ok_as_is(_count: u32, _decoded_len: u32) -> (ok: bool)
    ensures
        ok == true,
{
    true
}

proof fn lemma_prefix_is_not_ok()
    ensures
        write_record_count_ok_spec(3, 3),
        !write_record_count_ok_spec(3, 2),
        write_record_count_ok_as_is_spec(3, 2),
{
}

} // verus!
