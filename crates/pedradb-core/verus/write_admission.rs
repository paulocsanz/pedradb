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

/// 0 = Ok, 1 = StallMem, 2 = StallL0 (production `WriteAdmit`).
pub open spec fn write_admit_spec(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> u8 {
    if mem_armed && mem_bytes >= mem_limit {
        1u8
    } else if l0_armed && l0 >= l0_limit {
        2u8
    } else {
        0u8
    }
}

pub fn write_admit(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> (d: u8)
    ensures
        d == write_admit_spec(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit),
{
    if mem_armed && mem_bytes >= mem_limit {
        1u8
    } else if l0_armed && l0 >= l0_limit {
        2u8
    } else {
        0u8
    }
}

pub open spec fn write_admit_as_is_spec(
    _mem_bytes: u64,
    _mem_armed: bool,
    _mem_limit: u64,
    _l0: u64,
    _l0_armed: bool,
    _l0_limit: u64,
) -> u8 {
    0u8
}

pub fn write_admit_as_is(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> (d: u8)
    ensures
        d == write_admit_as_is_spec(mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit),
        d == 0u8,
{
    let _ = (mem_bytes, mem_armed, mem_limit, l0, l0_armed, l0_limit);
    0u8
}

proof fn lemma_as_is_admits_mem_over()
    ensures
        write_admit_spec(100, true, 50, 0, false, 0) == 1u8,
        write_admit_as_is_spec(100, true, 50, 0, false, 0) == 0u8,
{
}

} // verus!
