//! Stateright model over the **real** [`pedradb_core::prefix_exclusive_end`] (F57 / F58).
//!
//! Faithfulness: exclusive end and `[prefix, end)` membership are production
//! `prefix` helpers. The range scan is an axiom.
//!
//! - **Inv-ff-kept:** `prefix || 0xff || …` stays in the scan (F57).
//! - **Inv-child:** a normal child is in range.
//! - **Inv-end-exclusive:** the exclusive-end key itself is out.
//! - AS-IS `prefix || 0xff` must drop the 0xff continuation (teeth).

use pedradb_core::{key_in_prefix_range, prefix_exclusive_end, prefix_exclusive_end_as_is};
use stateright::{Checker, Model, Property};

const P: &[u8] = b"/host/h1/";

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    ff_dropped: bool,
    child_missed: bool,
    end_included: bool,
    saw_ff: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Child,
    FfContinuation,
    EndKey,
    EmptyPrefixFf,
}

#[derive(Clone)]
struct PrefixModel {
    fixed: bool,
}

impl PrefixModel {
    fn end(&self, prefix: &[u8]) -> Option<Vec<u8>> {
        if self.fixed {
            prefix_exclusive_end(prefix)
        } else {
            prefix_exclusive_end_as_is(prefix)
        }
    }

    fn in_range(&self, key: &[u8], prefix: &[u8]) -> bool {
        let end = self.end(prefix);
        key_in_prefix_range(key, prefix, end.as_deref())
    }
}

impl Model for PrefixModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            ff_dropped: false,
            child_missed: false,
            end_included: false,
            saw_ff: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            Act::Child,
            Act::FfContinuation,
            Act::EndKey,
            Act::EmptyPrefixFf,
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Child => {
                let mut key = P.to_vec();
                key.extend_from_slice(b"a");
                if !self.in_range(&key, P) {
                    next.child_missed = true;
                }
            }
            Act::FfContinuation => {
                next.saw_ff = true;
                let mut key = P.to_vec();
                key.push(0xff);
                key.extend_from_slice(b"z");
                if !self.in_range(&key, P) {
                    next.ff_dropped = true;
                }
            }
            Act::EndKey => {
                if let Some(end) = prefix_exclusive_end(P) {
                    // The FIXED exclusive end must never be a prefix match.
                    if key_in_prefix_range(&end, P, Some(end.as_slice())) {
                        next.end_included = true;
                    }
                }
            }
            Act::EmptyPrefixFf => {
                let key = [0xff, b'z'];
                if !self.in_range(&key, b"") {
                    next.ff_dropped = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-ff-kept", inv_ff_kept),
            Property::always("Inv-child", inv_child),
            Property::always("Inv-end-exclusive", inv_end_exclusive),
            Property::sometimes("non-vacuity-ff", non_vacuity_ff),
        ]
    }
}

fn inv_ff_kept(_: &PrefixModel, s: &St) -> bool {
    !s.ff_dropped
}

fn inv_child(_: &PrefixModel, s: &St) -> bool {
    !s.child_missed
}

fn inv_end_exclusive(_: &PrefixModel, s: &St) -> bool {
    !s.end_included
}

fn non_vacuity_ff(_: &PrefixModel, s: &St) -> bool {
    s.saw_ff
}

#[test]
fn fixed_prefix_holds() {
    let checker = PrefixModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_drops_ff_continuation() {
    let checker = PrefixModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-ff-kept").is_some(),
        "AS-IS prefix||0xff must drop prefix||0xff||z (F57/F58 teeth)"
    );
}
