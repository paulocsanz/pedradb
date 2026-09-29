//! Stateright model over the **real** [`pedradb_http::form_kernel`] (F101 / F155).
//!
//! Faithfulness: `+` → space, `%2B` stays plus, and disagreeing query
//! repeats are production `form_decode` / `query_*_conflict`. Path `%HH`
//! is a different caller (RFC 3986).
//!
//! - **Inv-plus-is-space:** raw `+` in a query value is a space (F101).
//! - **Inv-pct2b-literal:** `%2B` remains `+` (`+` before `%HH`).
//! - **Inv-query-conflict:** `rev=1&rev=0` is a conflict (F155).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_http::{
    form_decode, form_decode_as_is, query_u64_conflict, query_u64_conflict_as_is,
    query_values_conflict, query_values_conflict_as_is,
};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    plus_literal: bool,
    pct2b_not_plus: bool,
    first_wins: bool,
    saw_decode: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    DecodePlus,
    DecodePct2B,
    ConflictValues,
    ConflictU64,
}

#[derive(Clone)]
struct FormModel {
    fixed: bool,
}

impl FormModel {
    fn decode(&self, s: &str) -> Vec<u8> {
        if self.fixed {
            form_decode(s)
        } else {
            form_decode_as_is(s)
        }
    }

    fn values_conflict(&self, vs: &[&str]) -> bool {
        if self.fixed {
            query_values_conflict(vs)
        } else {
            query_values_conflict_as_is(vs)
        }
    }

    fn ints_conflict(&self, a: u64, b: u64) -> bool {
        if self.fixed {
            query_u64_conflict(a, b)
        } else {
            query_u64_conflict_as_is(a, b)
        }
    }
}

impl Model for FormModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            plus_literal: false,
            pct2b_not_plus: false,
            first_wins: false,
            saw_decode: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            Act::DecodePlus,
            Act::DecodePct2B,
            Act::ConflictValues,
            Act::ConflictU64,
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::DecodePlus => {
                next.saw_decode = true;
                if self.decode("hello+world") != b"hello world" {
                    next.plus_literal = true;
                }
            }
            Act::DecodePct2B => {
                if self.decode("plus%2Bsign") != b"plus+sign" {
                    next.pct2b_not_plus = true;
                }
            }
            Act::ConflictValues => {
                if !self.values_conflict(&["1", "0"]) {
                    next.first_wins = true;
                }
            }
            Act::ConflictU64 => {
                if !self.ints_conflict(1, 0) {
                    next.first_wins = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-plus-is-space", inv_plus_is_space),
            Property::always("Inv-pct2b-literal", inv_pct2b),
            Property::always("Inv-query-conflict", inv_conflict),
            Property::sometimes("non-vacuity-decode", non_vacuity_decode),
        ]
    }
}

fn inv_plus_is_space(_: &FormModel, s: &St) -> bool {
    !s.plus_literal
}

fn inv_pct2b(_: &FormModel, s: &St) -> bool {
    !s.pct2b_not_plus
}

fn inv_conflict(_: &FormModel, s: &St) -> bool {
    !s.first_wins
}

fn non_vacuity_decode(_: &FormModel, s: &St) -> bool {
    s.saw_decode
}

#[test]
fn fixed_form_holds() {
    let checker = FormModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_keeps_plus_literal() {
    let checker = FormModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-plus-is-space").is_some(),
        "AS-IS hello+world must stay hello+world (F101 teeth)"
    );
}

#[test]
fn as_is_first_query_wins() {
    let checker = FormModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-query-conflict").is_some(),
        "AS-IS must ignore rev=1&rev=0 (F155 teeth)"
    );
}
