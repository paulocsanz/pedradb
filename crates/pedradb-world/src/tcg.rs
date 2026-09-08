//! RFC-0079 / R-tcg-guest: native World is not TCG guest coverage.
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_tcg_guest.sh
//!
//! Not a `*_kernel.rs` (RFC-0079: no new TCB file). Guest SSH stays
//! `scripts/tcg_guest_status.sh`.

#![forbid(unsafe_code)]

macro_rules! tcg_guest_admitted_body {
    ($guest_reachable:expr) => {
        $guest_reachable
    };
}

macro_rules! tcg_guest_admitted_as_is_body {
    ($guest_reachable:expr) => {{
        let _ = $guest_reachable;
        true
    }};
}

/// Admit TCG guest coverage (RFC-0079 / R-tcg-guest).
///
/// Native [`crate::World::run`] does not SSH and does not invent a guest, so it
/// passes `guest_reachable = false`. AS-IS treats a green World as TCG.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn tcg_guest_admitted(guest_reachable: bool) -> bool {
    tcg_guest_admitted_body!(guest_reachable)
}

/// AS-IS: native World smoke is rounded to TCG coverage (the 0079 hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn tcg_guest_admitted_as_is(_guest_reachable: bool) -> bool {
    tcg_guest_admitted_as_is_body!(_guest_reachable)
}

/// RFC-0079 P1.2: `world_smoke --claim-tcg` is admitted only when the
/// kernel admits. Native World passes `guest_reachable = false`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn allow_claim_tcg_flag(claim_flag: bool, guest_reachable: bool) -> bool {
    if !claim_flag {
        return true;
    }
    tcg_guest_admitted(guest_reachable)
}

/// AS-IS: `--claim-tcg` on native smoke is treated as TCG coverage.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn allow_claim_tcg_flag_as_is(_claim_flag: bool, _guest_reachable: bool) -> bool {
    true
}

/// RFC-0079 P2.2: native World does not SSH. Guest probe stays the script.
/// Always false.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn world_runs_guest_ssh() -> bool {
    false
}

/// AS-IS: World::run is rounded to an in-process SSH guest probe.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn world_runs_guest_ssh_as_is() -> bool {
    true
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn tcg_guest_admitted_spec(guest_reachable: bool) -> bool {
    guest_reachable
}

pub open spec fn tcg_guest_admitted_as_is_spec(_guest_reachable: bool) -> bool {
    true
}

pub fn tcg_guest_admitted(guest_reachable: bool) -> (ok: bool)
    ensures
        ok == tcg_guest_admitted_spec(guest_reachable),
{
    tcg_guest_admitted_body!(guest_reachable)
}

pub fn tcg_guest_admitted_as_is(_guest_reachable: bool) -> (ok: bool)
    ensures
        ok == tcg_guest_admitted_as_is_spec(_guest_reachable),
{
    tcg_guest_admitted_as_is_body!(_guest_reachable)
}

proof fn lemma_native_world_is_not_tcg()
    ensures
        !tcg_guest_admitted_spec(false),
        tcg_guest_admitted_as_is_spec(false),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    /// Catalog three-teeth plant. Direct `claim_tcg_guest_refused_on_native_world`
    /// is **not** this tooth.
    #[test]
    fn tcg_guest_admitted_on_live_world_is_not_ok() {
        assert!(!tcg_guest_admitted(false));
        assert!(
            tcg_guest_admitted_as_is(false),
            "AS-IS dente: native World would claim TCG guest coverage"
        );
        let parent = crate::temp_parent("tcg-0152");
        let cfg = crate::WorldConfig {
            n_nodes: 3,
            n_ranges: 1,
            schedule_steps: 8,
            parent: parent.clone(),
            mem_storage: true,
            ..Default::default()
        };
        let t = crate::World::new(0x0152_0143, cfg).run().unwrap();
        assert!(!t.events.is_empty(), "World::run must actually schedule");
        assert!(
            !t.claim_tcg_guest(),
            "live native World must refuse TCG guest coverage"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }
}
