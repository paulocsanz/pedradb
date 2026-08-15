//! Stateright model over the **real** [`montanha_fdb_recipes::children_kernel`] (F59).
//!
//! Faithfulness: exclusive end of packed children is production
//! `packed_children_end`. Tuple `pack` is an axiom.
//!
//! Zip `900` sorts inside `[pack(90), pack(90)||0xff)` but not inside
//! `[pack(90)||0x00, pack(90)||0x01)`.
//!
//! - **Inv-no-sibling-900:** `pack("900")` is outside children of `pack("90")`.
//! - **Inv-child:** a `0x00`-separated child is inside.
//! - AS-IS `||0xff` must leak the sibling (teeth).

use montanha_fdb_recipes::{
    key_in_half_open, next_byte_in_packed_children, next_byte_in_packed_children_as_is,
    packed_children_end, packed_children_end_as_is, packed_children_start, PACKED_CHILD_SEP,
};
use stateright::{Checker, Model, Property};

fn pack90() -> &'static [u8] {
    b"zip\x0090"
}

fn child_short() -> Vec<u8> {
    let mut k = packed_children_start(pack90());
    k.extend_from_slice(b"short");
    k
}

fn sibling_900() -> Vec<u8> {
    let mut k = pack90().to_vec();
    k.push(b'0');
    k.extend_from_slice(b"\x00long");
    k
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    sibling_leaked: bool,
    child_missed: bool,
    saw_sibling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Child,
    Sibling900,
    NextByte0,
    NextByteDigit,
}

#[derive(Clone)]
struct ChildrenModel {
    fixed: bool,
}

impl ChildrenModel {
    fn in_range(&self, key: &[u8]) -> bool {
        let p = pack90();
        let (start, end) = if self.fixed {
            (packed_children_start(p), packed_children_end(p))
        } else {
            (p.to_vec(), packed_children_end_as_is(p))
        };
        key_in_half_open(key, &start, &end)
    }

    fn next_is_child(&self, b: u8) -> bool {
        if self.fixed {
            next_byte_in_packed_children(b)
        } else {
            next_byte_in_packed_children_as_is(b)
        }
    }
}

impl Model for ChildrenModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            sibling_leaked: false,
            child_missed: false,
            saw_sibling: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            Act::Child,
            Act::Sibling900,
            Act::NextByte0,
            Act::NextByteDigit,
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::Child => {
                if !self.in_range(&child_short()) {
                    next.child_missed = true;
                }
            }
            Act::Sibling900 => {
                next.saw_sibling = true;
                if self.in_range(&sibling_900()) {
                    next.sibling_leaked = true;
                }
            }
            Act::NextByte0 => {
                if !self.next_is_child(PACKED_CHILD_SEP) {
                    next.child_missed = true;
                }
            }
            Act::NextByteDigit => {
                if self.next_is_child(b'0') {
                    next.sibling_leaked = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-sibling-900", inv_no_sib),
            Property::always("Inv-child", inv_child),
            Property::sometimes("non-vacuity-sib", non_vacuity),
        ]
    }
}

fn inv_no_sib(_: &ChildrenModel, s: &St) -> bool {
    !s.sibling_leaked
}

fn inv_child(_: &ChildrenModel, s: &St) -> bool {
    !s.child_missed
}

fn non_vacuity(_: &ChildrenModel, s: &St) -> bool {
    s.saw_sibling
}

#[test]
fn fixed_children_holds() {
    let checker = ChildrenModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_leaks_zip_900() {
    let checker = ChildrenModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-sibling-900").is_some(),
        "AS-IS pack||0xff must include zip 900 (F59 teeth)"
    );
}
