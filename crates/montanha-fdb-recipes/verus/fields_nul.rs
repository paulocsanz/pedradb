// Verus proof of F60 field span (RFC-0002 P24).
// Twin of `src/fields_kernel.rs` (Vec encode/decode is caller).
//
//   ./scripts/verus_fields_nul.sh

use vstd::prelude::*;

verus! {

pub open spec fn field_kept_spec(len: u64, _nul_at: u64) -> u64 {
    len
}

pub fn field_kept(len: u64, nul_at: u64) -> (k: u64)
    ensures
        k == field_kept_spec(len, nul_at),
        k == len,
{
    let _ = nul_at;
    len
}

pub open spec fn field_kept_as_is(_len: u64, nul_at: u64) -> u64 {
    nul_at
}

proof fn lemma_as_is_truncates(len: u64, nul_at: u64)
    requires
        nul_at < len,
    ensures
        field_kept_as_is(len, nul_at) < field_kept_spec(len, nul_at),
{
}

proof fn lemma_no_nul_same(len: u64)
    ensures
        field_kept_spec(len, len) == field_kept_as_is(len, len),
{
}

} // verus!
