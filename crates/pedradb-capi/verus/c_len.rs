// Verus twin of `src/handles.rs::c_len_admitted` (RFC-0075 P2.1).
// Not linked into production. Not a free-table proof (P2.2).
//
//   ./scripts/verus_c_len.sh

use vstd::prelude::*;

verus! {

pub open spec fn c_len_admitted_spec(len: usize, max: usize) -> bool {
    len <= max
}

pub open spec fn c_len_admitted_as_is_spec(_len: usize, _max: usize) -> bool {
    true
}

pub fn c_len_admitted(len: usize, max: usize) -> (ok: bool)
    ensures
        ok == c_len_admitted_spec(len, max),
{
    len <= max
}

pub fn c_len_admitted_as_is(_len: usize, _max: usize) -> (ok: bool)
    ensures
        ok == c_len_admitted_as_is_spec(_len, _max),
{
    true
}

proof fn lemma_oversize_len_is_not_admitted()
    ensures
        c_len_admitted_spec(0, 10),
        c_len_admitted_spec(10, 10),
        !c_len_admitted_spec(11, 10),
        c_len_admitted_as_is_spec(11, 10),
{
}

} // verus!
