// Verus twin of `src/wal/crc.rs::crc_match_ok` (RFC-0076 P2.1).
// Not linked into production. Not a collision theorem (P2.2 / R-crc).
//
//   ./scripts/verus_crc_match.sh

use vstd::prelude::*;

verus! {

pub open spec fn crc_match_ok_spec(stored: u32, computed: u32) -> bool {
    stored == computed
}

pub open spec fn crc_match_ok_as_is_spec(_stored: u32, _computed: u32) -> bool {
    true
}

pub fn crc_match_ok(stored: u32, computed: u32) -> (ok: bool)
    ensures
        ok == crc_match_ok_spec(stored, computed),
{
    stored == computed
}

pub fn crc_match_ok_as_is(_stored: u32, _computed: u32) -> (ok: bool)
    ensures
        ok == crc_match_ok_as_is_spec(_stored, _computed),
{
    true
}

proof fn lemma_mismatch_is_not_ok()
    ensures
        crc_match_ok_spec(1, 1),
        !crc_match_ok_spec(1, 2),
        crc_match_ok_as_is_spec(1, 2),
{
}

} // verus!
