//! Write-admission idle gate (RFC-0170 P2.4).
//! kernel: write_admission — stall knobs off ⇒ skip CF-family collection.
//!
//! Production [`crate::db::Db::write_admission_idle`] calls this. AS-IS
//! never stalls, so a live stall knob is ignored (writes keep the idle path).

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
}
