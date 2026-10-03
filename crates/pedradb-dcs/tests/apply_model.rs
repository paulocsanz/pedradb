//! Stateright model over the **real** [`pedradb_dcs::apply_kernel`] (F12 / F22).
//!
//! Faithfulness: whether `last_applied` advances is production
//! [`dcs_apply_should_advance_result`]. Persist of the DCS command is an axiom.
//!
//! - **Inv-cas-advances:** `CasFailed` is a no-op and must not freeze apply (F12).
//! - **Inv-hard-stops:** I/O / corrupt / lease errors do not advance.
//! - AS-IS must freeze on `CasFailed` (teeth).

use pedradb_dcs::{
    dcs_apply_should_advance_as_is, dcs_apply_should_advance_result, DcsError, Result as DcsResult,
};
use stateright::{Checker, Model, Property};

const APPLIED_CAP: u8 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    applied: u8,
    frozen: bool,
    advanced_on_hard: bool,
    saw_cas: bool,
    saw_ok: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Ok,
    Cas,
    Hard,
}

#[derive(Clone)]
struct ApplyModel {
    fixed: bool,
}

impl ApplyModel {
    fn advance(&self, r: &DcsResult<u64>) -> bool {
        if self.fixed {
            dcs_apply_should_advance_result(r)
        } else {
            match r {
                Ok(_) => dcs_apply_should_advance_as_is(true, false),
                Err(DcsError::CasFailed(_)) => dcs_apply_should_advance_as_is(false, true),
                Err(_) => dcs_apply_should_advance_as_is(false, false),
            }
        }
    }
}

impl Model for ApplyModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            applied: 0,
            frozen: false,
            advanced_on_hard: false,
            saw_cas: false,
            saw_ok: false,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.applied >= APPLIED_CAP || state.frozen {
            return;
        }
        actions.extend([Act::Ok, Act::Cas, Act::Hard]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        if state.applied >= APPLIED_CAP || state.frozen {
            return None;
        }
        let mut next = state.clone();
        let r: DcsResult<u64> = match action {
            Act::Ok => Ok(0),
            Act::Cas => Err(DcsError::CasFailed("key exists")),
            Act::Hard => Err(DcsError::Corrupt("x".into())),
        };
        match action {
            Act::Ok => next.saw_ok = true,
            Act::Cas => next.saw_cas = true,
            Act::Hard => {}
        }
        if self.advance(&r) {
            if matches!(action, Act::Hard) {
                next.advanced_on_hard = true;
            }
            next.applied = state.applied.saturating_add(1);
        } else if matches!(action, Act::Cas) {
            next.frozen = true;
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-cas-advances", inv_cas_advances),
            Property::always("Inv-hard-stops", inv_hard_stops),
            Property::sometimes("non-vacuity-ok", non_vacuity_ok),
            Property::sometimes("non-vacuity-cas", non_vacuity_cas),
        ]
    }
}

fn inv_cas_advances(_: &ApplyModel, s: &St) -> bool {
    !s.frozen
}

fn inv_hard_stops(_: &ApplyModel, s: &St) -> bool {
    !s.advanced_on_hard
}

fn non_vacuity_ok(_: &ApplyModel, s: &St) -> bool {
    s.saw_ok && s.applied > 0
}

fn non_vacuity_cas(_: &ApplyModel, s: &St) -> bool {
    s.saw_cas
}

#[test]
fn fixed_apply_holds() {
    let checker = ApplyModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_freezes_on_cas() {
    let checker = ApplyModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-cas-advances").is_some(),
        "AS-IS must freeze last_applied on CasFailed (F12/F22 teeth)"
    );
}
