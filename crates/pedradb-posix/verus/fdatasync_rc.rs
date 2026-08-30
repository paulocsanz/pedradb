// Verus twin of `src/lib.rs::fdatasync_rc_ok` (RFC-0073 P2.1).
// Not linked into production. No new `*_kernel.rs`.
//
//   ./scripts/verus_fdatasync_rc.sh

use vstd::prelude::*;

verus! {

pub open spec fn fdatasync_rc_ok_spec(rc: i32) -> bool {
    rc == 0
}

pub open spec fn fdatasync_rc_ok_as_is_spec(_rc: i32) -> bool {
    true
}

pub fn fdatasync_rc_ok(rc: i32) -> (ok: bool)
    ensures
        ok == fdatasync_rc_ok_spec(rc),
{
    rc == 0
}

pub fn fdatasync_rc_ok_as_is(_rc: i32) -> (ok: bool)
    ensures
        ok == fdatasync_rc_ok_as_is_spec(_rc),
{
    true
}

proof fn lemma_nonzero_rc_is_not_ok()
    ensures
        fdatasync_rc_ok_spec(0),
        !fdatasync_rc_ok_spec(-1),
        fdatasync_rc_ok_as_is_spec(-1),
{
}

} // verus!
