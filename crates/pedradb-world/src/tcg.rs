//! RFC-0079 / R-tcg-guest: native World is not TCG guest coverage.
//!
//! Not a `*_kernel.rs` (RFC-0079: no new TCB file). Guest SSH stays
//! `scripts/tcg_guest_status.sh`.

#![forbid(unsafe_code)]

/// Admit TCG guest coverage (RFC-0079 / R-tcg-guest).
///
/// Native [`crate::World::run`] does not SSH and does not invent a guest, so it
/// passes `guest_reachable = false`. AS-IS treats a green World as TCG.
#[must_use]
pub fn tcg_guest_admitted(guest_reachable: bool) -> bool {
    guest_reachable
}

/// AS-IS: native World smoke is rounded to TCG coverage (the 0079 hole).
#[must_use]
pub fn tcg_guest_admitted_as_is(_guest_reachable: bool) -> bool {
    true
}

/// RFC-0079 P1.2: `world_smoke --claim-tcg` is admitted only when the
/// kernel admits. Native World passes `guest_reachable = false`.
#[must_use]
pub fn allow_claim_tcg_flag(claim_flag: bool, guest_reachable: bool) -> bool {
    if !claim_flag {
        return true;
    }
    tcg_guest_admitted(guest_reachable)
}

/// AS-IS: `--claim-tcg` on native smoke is treated as TCG coverage.
#[must_use]
pub fn allow_claim_tcg_flag_as_is(_claim_flag: bool, _guest_reachable: bool) -> bool {
    true
}

/// RFC-0079 P2.2: native World does not SSH. Guest probe stays the script.
/// Always false.
#[must_use]
pub fn world_runs_guest_ssh() -> bool {
    false
}

/// AS-IS: World::run is rounded to an in-process SSH guest probe.
#[must_use]
pub fn world_runs_guest_ssh_as_is() -> bool {
    true
}
