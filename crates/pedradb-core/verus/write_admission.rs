// Verus twin of write_admission_kernel.rs (RFC-0170 P2.4).
//
//   ./scripts/verus_write_admission.sh

use vstd::prelude::*;

verus! {

pub open spec fn write_admission_idle_spec(mem: bool, pressure: bool, stall: bool) -> bool {
    !mem && !pressure && !stall
}

pub fn write_admission_idle(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> (d: bool)
    ensures
        d == write_admission_idle_spec(mem_stall, pressure_l0, stall_l0),
{
    !mem_stall && !pressure_l0 && !stall_l0
}

pub open spec fn write_admission_idle_as_is_spec(_mem: bool, _pressure: bool, _stall: bool) -> bool {
    true
}

pub fn write_admission_idle_as_is(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> (d: bool)
    ensures
        d == write_admission_idle_as_is_spec(mem_stall, pressure_l0, stall_l0),
        d == true,
{
    let _ = (mem_stall, pressure_l0, stall_l0);
    true
}

proof fn lemma_as_is_ignores_stall()
    ensures
        !write_admission_idle_spec(true, false, false),
        write_admission_idle_as_is_spec(true, false, false),
{
}

} // verus!
