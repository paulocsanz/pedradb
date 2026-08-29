// Verus twin of `src/key.rs::pack_sequence_and_type` (RFC-0150 P1).
// Packed trailer: (seq << 8) | kind. Kind nibble 0/1/2.
//
//   ./scripts/verus_ikey_pack.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

pub open spec fn pack_sequence_and_type_spec(sequence: u64, kind: u8) -> u64 {
    (sequence << 8) | (kind as u64)
}

pub fn pack_sequence_and_type(sequence: u64, kind: u8) -> (p: u64)
    requires
        sequence <= 0x00ff_ffff_ffff_ffffu64,
        kind <= 2,
    ensures
        p == pack_sequence_and_type_spec(sequence, kind),
{
    (sequence << 8) | (kind as u64)
}

pub open spec fn unpack_sequence_spec(packed: u64) -> u64 {
    packed >> 8
}

pub fn unpack_sequence(packed: u64) -> (s: u64)
    ensures
        s == unpack_sequence_spec(packed),
{
    packed >> 8
}

proof fn lemma_pack_spec_is_shift_or(sequence: u64, kind: u8)
    ensures
        pack_sequence_and_type_spec(sequence, kind) == (sequence << 8) | (kind as u64),
{
}

} // verus!
