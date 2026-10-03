//! Pure DCS apply-pipeline decision (RFC-0002 P12 / F12 / F22).
//!
//! **Single artifact:** this file is what `rustc` links *and* what Verus
//! proves (`cfg(verus_keep_ghost)`). No twin-cópia.
//!
//!   ./scripts/verus_dcs_apply.sh
//!
//! At apply time, `CasFailed` is a no-op (idempotent / racy create). Only
//! hard errors (I/O, corrupt, lease) may fail the loop. Raft and store
//! must call **this** function.

#![forbid(unsafe_code)]

macro_rules! dcs_apply_should_advance_body {
    ($ok:expr, $cas_failed:expr) => {
        $ok || $cas_failed
    };
}

macro_rules! dcs_apply_should_advance_as_is_body {
    ($ok:expr, $cas_failed:expr) => {{
        let _ = $cas_failed;
        $ok
    }};
}

#[cfg(not(verus_keep_ghost))]
use crate::{DcsError, Result};

/// Whether apply may advance `last_applied` / `applied`.
///
/// `ok` — `apply_dcs_command` returned `Ok`.
/// `cas_failed` — the error was `DcsError::CasFailed`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn dcs_apply_should_advance(ok: bool, cas_failed: bool) -> bool {
    dcs_apply_should_advance_body!(ok, cas_failed)
}

/// Map a live `apply_dcs_command` result onto [`dcs_apply_should_advance`].
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn dcs_apply_should_advance_result<T>(r: &Result<T>) -> bool {
    match r {
        Ok(_) => dcs_apply_should_advance(true, false),
        Err(DcsError::CasFailed(_)) => dcs_apply_should_advance(false, true),
        Err(_) => dcs_apply_should_advance(false, false),
    }
}

/// AS-IS F12/F22: only `Ok` advances — `CasFailed` freezes the pipeline.
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn dcs_apply_should_advance_as_is(ok: bool, cas_failed: bool) -> bool {
    dcs_apply_should_advance_as_is_body!(ok, cas_failed)
}

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn dcs_apply_should_advance_spec(ok: bool, cas_failed: bool) -> bool {
    ok || cas_failed
}

pub fn dcs_apply_should_advance(ok: bool, cas_failed: bool) -> (d: bool)
    ensures
        d == dcs_apply_should_advance_spec(ok, cas_failed),
        d == (ok || cas_failed),
{
    dcs_apply_should_advance_body!(ok, cas_failed)
}

pub open spec fn dcs_apply_should_advance_as_is_spec(ok: bool, _cas_failed: bool) -> bool {
    ok
}

pub fn dcs_apply_should_advance_as_is(ok: bool, cas_failed: bool) -> (d: bool)
    ensures
        d == ok,
        d == dcs_apply_should_advance_as_is_spec(ok, cas_failed),
{
    dcs_apply_should_advance_as_is_body!(ok, cas_failed)
}

proof fn lemma_as_is_freezes_cas()
    ensures
        dcs_apply_should_advance_spec(false, true),
        !dcs_apply_should_advance_as_is_spec(false, true),
{
}

proof fn lemma_hard_fail_does_not_advance()
    ensures
        !dcs_apply_should_advance_spec(false, false),
{
}

} // verus!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_and_cas_advance() {
        assert!(dcs_apply_should_advance(true, false));
        assert!(dcs_apply_should_advance(false, true));
        assert!(!dcs_apply_should_advance(false, false));
    }

    #[test]
    fn as_is_freezes_on_cas() {
        assert!(!dcs_apply_should_advance_as_is(false, true));
        assert_ne!(
            dcs_apply_should_advance(false, true),
            dcs_apply_should_advance_as_is(false, true)
        );
    }

    #[test]
    fn result_wrapper_matches() {
        let ok: Result<u64> = Ok(0);
        let cas: Result<u64> = Err(DcsError::CasFailed("key exists"));
        let hard: Result<u64> = Err(DcsError::Corrupt("x".into()));
        assert!(dcs_apply_should_advance_result(&ok));
        assert!(dcs_apply_should_advance_result(&cas));
        assert!(!dcs_apply_should_advance_result(&hard));
    }

    #[test]
    fn theorem_on_bool_domain() {
        for ok in [false, true] {
            for cas in [false, true] {
                let d = dcs_apply_should_advance(ok, cas);
                assert_eq!(d, ok || cas);
                assert_eq!(dcs_apply_should_advance_as_is(ok, cas), ok);
            }
        }
    }
}
