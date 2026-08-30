// Verus twin of `src/cqe_kernel.rs::cqe_res_ok` (RFC-0074 P2.1).
// Not linked into production. Not a ring model (P2.2 / R-uring).
//
//   ./scripts/verus_cqe_res.sh

use vstd::prelude::*;

verus! {

pub open spec fn cqe_res_ok_spec(res: i32) -> bool {
    res >= 0
}

pub open spec fn cqe_res_ok_as_is_spec(_res: i32) -> bool {
    true
}

pub fn cqe_res_ok(res: i32) -> (ok: bool)
    ensures
        ok == cqe_res_ok_spec(res),
{
    res >= 0
}

pub fn cqe_res_ok_as_is(_res: i32) -> (ok: bool)
    ensures
        ok == cqe_res_ok_as_is_spec(_res),
{
    true
}

proof fn lemma_negative_res_is_not_ok()
    ensures
        cqe_res_ok_spec(0i32),
        cqe_res_ok_spec(16i32),
        !cqe_res_ok_spec(-5i32),
        !cqe_res_ok_spec(-1i32),
        cqe_res_ok_as_is_spec(-5i32),
{
}

} // verus!
