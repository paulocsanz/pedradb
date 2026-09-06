//! Write-admission idle gate + hard stall (RFC-0170 P2.4).
//! kernel: write_admission — stall knobs off ⇒ skip CF-family collection;
//! hard admit after the handler has measured (and optionally drained).
//!
//! Production [`crate::db::Db::write_admission_idle`] / `ensure_write_admitted_for`
//! call these. AS-IS never stalls.

#![forbid(unsafe_code)]

/// True iff no write-stall knob is armed.
#[must_use]
pub fn write_admission_idle(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> bool {
    !mem_stall && !pressure_l0 && !stall_l0
}

/// AS-IS: always idle — stall knobs are decorative.
#[must_use]
pub fn write_admission_idle_as_is(_mem_stall: bool, _pressure_l0: bool, _stall_l0: bool) -> bool {
    true
}

/// Hard-admit verdict after the handler measured mem/L0 (drain is glue).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteAdmit {
    /// Write may proceed.
    Ok,
    /// Mem bytes still at/above the armed mem stall limit.
    StallMem,
    /// L0 file count still at/above the armed L0 stall limit.
    StallL0,
}

/// Mem axis first (same order as `Db::ensure_write_admitted_for`), then L0.
#[must_use]
pub fn write_admit(
    mem_bytes: u64,
    mem_armed: bool,
    mem_limit: u64,
    l0: u64,
    l0_armed: bool,
    l0_limit: u64,
) -> WriteAdmit {
    if mem_armed && mem_bytes >= mem_limit {
        return WriteAdmit::StallMem;
    }
    if l0_armed && l0 >= l0_limit {
        return WriteAdmit::StallL0;
    }
    WriteAdmit::Ok
}

/// AS-IS: always admit — stall knobs are decorative.
#[must_use]
pub fn write_admit_as_is(
    _mem_bytes: u64,
    _mem_armed: bool,
    _mem_limit: u64,
    _l0: u64,
    _l0_armed: bool,
    _l0_limit: u64,
) -> WriteAdmit {
    WriteAdmit::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_admission_idle_on_live_stall_is_not_ok() {
        assert!(write_admission_idle(false, false, false));
        assert!(!write_admission_idle(true, false, false));
        assert!(
            write_admission_idle_as_is(true, true, true),
            "AS-IS dente: stall knobs ignored"
        );
    }

    #[test]
    fn write_admit_on_live_mem_over_is_not_ok() {
        assert_eq!(
            write_admit(100, true, 50, 0, false, 0),
            WriteAdmit::StallMem
        );
        assert_eq!(
            write_admit_as_is(100, true, 50, 0, false, 0),
            WriteAdmit::Ok,
            "AS-IS dente: mem over still admits"
        );
        assert_eq!(write_admit(10, true, 50, 8, true, 4), WriteAdmit::StallL0);
        assert_eq!(write_admit(10, true, 50, 2, true, 4), WriteAdmit::Ok);
        assert_eq!(
            write_admit(100, true, 50, 8, true, 4),
            WriteAdmit::StallMem,
            "mem axis wins when both over"
        );
    }
}
