//! Stateright model over the **real** [`pedradb_http::path_kernel`] (F91 / F92 / F156).
//!
//! Faithfulness: absolute-form / network-path / fragment stripping are
//! production `origin_form_path`. Routing `starts_with("/kv/")` is the
//! caller check this kernel exists for.
//!
//! - **Inv-origin-kv:** `http(s)://` and `//host` route as `/kv/…` (F91/F92).
//! - **Inv-no-fragment:** `#frag` is not a path segment (F156).
//! - **Inv-strip-flag:** authority-form is stripped before routing.
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_http::{
    origin_form_path, origin_form_path_as_is, strip_authority_for_routing,
    strip_authority_for_routing_as_is,
};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    missed_kv: bool,
    fragment_leaked: bool,
    strip_skipped: bool,
    saw_abs: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Absolute,
    Network,
    HttpsCase,
    Origin,
    Fragment,
    StripFlag { authority: bool },
}

#[derive(Clone)]
struct PathModel {
    fixed: bool,
}

impl PathModel {
    fn origin<'a>(&self, t: &'a str) -> &'a str {
        if self.fixed {
            origin_form_path(t)
        } else {
            origin_form_path_as_is(t)
        }
    }

    fn strip(&self, authority: bool) -> bool {
        if self.fixed {
            strip_authority_for_routing(authority)
        } else {
            strip_authority_for_routing_as_is(authority)
        }
    }
}

impl Model for PathModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            missed_kv: false,
            fragment_leaked: false,
            strip_skipped: false,
            saw_abs: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            Act::Absolute,
            Act::Network,
            Act::HttpsCase,
            Act::Origin,
            Act::Fragment,
            Act::StripFlag { authority: true },
            Act::StripFlag { authority: false },
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Absolute => {
                next.saw_abs = true;
                let p = self.origin("http://127.0.0.1:9/kv/x");
                if !p.starts_with("/kv/") {
                    next.missed_kv = true;
                }
            }
            Act::Network => {
                let p = self.origin("//h/kv/x");
                if !p.starts_with("/kv/") {
                    next.missed_kv = true;
                }
            }
            Act::HttpsCase => {
                let p = self.origin("HTTP://H/kv/x");
                if !p.starts_with("/kv/") {
                    next.missed_kv = true;
                }
            }
            Act::Origin => {
                let p = self.origin("/kv/x?y=1");
                if p != "/kv/x" {
                    next.missed_kv = true;
                }
            }
            Act::Fragment => {
                let p = self.origin("/kv/x#frag");
                if p.contains('#') {
                    next.fragment_leaked = true;
                }
            }
            Act::StripFlag { authority } => {
                if authority && !self.strip(true) {
                    next.strip_skipped = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-origin-kv", inv_origin_kv),
            Property::always("Inv-no-fragment", inv_no_fragment),
            Property::always("Inv-strip-flag", inv_strip_flag),
            Property::sometimes("non-vacuity-abs", non_vacuity_abs),
        ]
    }
}

fn inv_origin_kv(_: &PathModel, s: &St) -> bool {
    !s.missed_kv
}

fn inv_no_fragment(_: &PathModel, s: &St) -> bool {
    !s.fragment_leaked
}

fn inv_strip_flag(_: &PathModel, s: &St) -> bool {
    !s.strip_skipped
}

fn non_vacuity_abs(_: &PathModel, s: &St) -> bool {
    s.saw_abs
}

#[test]
fn fixed_path_holds() {
    let checker = PathModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_keeps_authority() {
    let checker = PathModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-origin-kv").is_some(),
        "AS-IS http://host/kv/x must miss /kv/ (F91/F92 teeth)"
    );
}

#[test]
fn as_is_leaks_fragment() {
    let checker = PathModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-fragment").is_some(),
        "AS-IS must keep #frag in the path (F156 teeth)"
    );
}

#[test]
fn as_is_never_strips() {
    let checker = PathModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-strip-flag").is_some(),
        "AS-IS strip_authority_for_routing must stay false (F91 teeth)"
    );
}
