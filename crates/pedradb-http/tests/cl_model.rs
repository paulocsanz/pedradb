//! Stateright model over the **real** [`pedradb_http::cl_kernel`] (F86 / F87 / F88 / F146).
//!
//! Faithfulness: missing CL, invalid CL, repeated CL, and short body vs
//! declared length are production `cl_kernel`. Socket read / `parse` are axioms.
//!
//! - **Inv-keep-without-cl:** no CL keeps the already-read body (F86).
//! - **Inv-invalid-not-zero:** unparseable CL is not length 0 (F87).
//! - **Inv-repeat-equal:** a second CL must equal the first (F88).
//! - **Inv-short-is-error:** EOF before CL bytes is a framing error (F146).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_http::{
    content_length_repeat_ok, content_length_repeat_ok_as_is, invalid_cl_as_zero,
    invalid_cl_as_zero_as_is, keep_body_without_cl, keep_body_without_cl_as_is,
    short_body_vs_cl_is_error, short_body_vs_cl_is_error_as_is,
};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    truncated_empty: bool,
    invalid_as_zero: bool,
    last_wins: bool,
    short_stored: bool,
    saw_missing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    /// Bytes already past the header break, no Content-Length.
    MissingCl {
        arrived: u8,
    },
    InvalidCl,
    /// First CL then a second, possibly different.
    RepeatCl {
        first: u8,
        next: u8,
    },
    /// EOF after `got` bytes, header said `declared`.
    ShortBody {
        got: u8,
        declared: u8,
    },
}

#[derive(Clone)]
struct ClModel {
    fixed: bool,
}

impl ClModel {
    fn keep(&self) -> bool {
        if self.fixed {
            keep_body_without_cl()
        } else {
            keep_body_without_cl_as_is()
        }
    }

    fn invalid_zero(&self) -> bool {
        if self.fixed {
            invalid_cl_as_zero()
        } else {
            invalid_cl_as_zero_as_is()
        }
    }

    fn repeat_ok(&self, first: u64, next: u64) -> bool {
        if self.fixed {
            content_length_repeat_ok(first, next)
        } else {
            content_length_repeat_ok_as_is(first, next)
        }
    }

    fn short_err(&self, got: u64, declared: u64) -> bool {
        if self.fixed {
            short_body_vs_cl_is_error(got, declared)
        } else {
            short_body_vs_cl_is_error_as_is(got, declared)
        }
    }
}

impl Model for ClModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            truncated_empty: false,
            invalid_as_zero: false,
            last_wins: false,
            short_stored: false,
            saw_missing: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        for arrived in 1u8..=3 {
            actions.push(Act::MissingCl { arrived });
        }
        actions.push(Act::InvalidCl);
        for first in 0u8..=2 {
            for next in 0u8..=2 {
                actions.push(Act::RepeatCl { first, next });
            }
        }
        for got in 0u8..=2 {
            for declared in 1u8..=3 {
                actions.push(Act::ShortBody { got, declared });
            }
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::MissingCl { arrived } => {
                next.saw_missing = true;
                if !self.keep() && arrived > 0 {
                    next.truncated_empty = true;
                }
            }
            Act::InvalidCl => {
                if self.invalid_zero() {
                    next.invalid_as_zero = true;
                }
            }
            Act::RepeatCl {
                first,
                next: second,
            } => {
                if first != second && self.repeat_ok(u64::from(first), u64::from(second)) {
                    next.last_wins = true;
                }
            }
            Act::ShortBody { got, declared } => {
                if !self.short_err(u64::from(got), u64::from(declared)) && got < declared {
                    next.short_stored = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-keep-without-cl", inv_keep),
            Property::always("Inv-invalid-not-zero", inv_invalid),
            Property::always("Inv-repeat-equal", inv_repeat),
            Property::always("Inv-short-is-error", inv_short),
            Property::sometimes("non-vacuity-missing", non_vacuity_missing),
        ]
    }
}

fn inv_keep(_: &ClModel, s: &St) -> bool {
    !s.truncated_empty
}

fn inv_invalid(_: &ClModel, s: &St) -> bool {
    !s.invalid_as_zero
}

fn inv_repeat(_: &ClModel, s: &St) -> bool {
    !s.last_wins
}

fn inv_short(_: &ClModel, s: &St) -> bool {
    !s.short_stored
}

fn non_vacuity_missing(_: &ClModel, s: &St) -> bool {
    s.saw_missing
}

#[test]
fn fixed_cl_holds() {
    let checker = ClModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_truncates_without_cl() {
    let checker = ClModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-keep-without-cl").is_some(),
        "AS-IS missing CL must truncate to empty (F86 teeth)"
    );
}

#[test]
fn as_is_invalid_cl_is_zero() {
    let checker = ClModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-invalid-not-zero").is_some(),
        "AS-IS invalid CL must become 0 (F87 teeth)"
    );
}

#[test]
fn as_is_repeat_last_wins() {
    let checker = ClModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-repeat-equal").is_some(),
        "AS-IS conflicting CL must last-win (F88 teeth)"
    );
}

#[test]
fn as_is_stores_short_body() {
    let checker = ClModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-short-is-error").is_some(),
        "AS-IS short body vs CL must store the prefix (F146 teeth)"
    );
}
