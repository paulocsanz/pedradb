//! Stateright model over the **real** [`montanha_fdb_recipes::pack_cut_tag`] (F62).
//!
//! Faithfulness: the cut tag is production `pack_kernel`. Concat of
//! `0x00 || tag || part` is the caller. AS-IS omits the length (raw
//! `0x00 || part`) — `pack([a\\0b, c])` equals `pack([a, b\\0c])`.
//!
//! - **Inv-no-collide:** the F62 witness pair encodes differently.
//! - **Inv-tag-injective:** distinct lengths get distinct tags.
//! - AS-IS must collide (teeth).

use montanha_fdb_recipes::{pack_cut_tag, pack_cut_tag_as_is, PACK_CUT_SEP};
use stateright::{Checker, Model, Property};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    collided: bool,
    tag_clash: bool,
    saw_witness: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    WitnessPair,
    DistinctLens { a: u8, b: u8 },
}

#[derive(Clone)]
struct PackModel {
    fixed: bool,
}

impl PackModel {
    fn tag(&self, len: u32) -> u32 {
        if self.fixed {
            pack_cut_tag(len)
        } else {
            pack_cut_tag_as_is(len)
        }
    }

    fn pack(&self, parts: &[&[u8]]) -> Vec<u8> {
        let mut buf = Vec::new();
        for part in parts {
            buf.push(PACK_CUT_SEP);
            let n = self.tag(u32::try_from(part.len()).unwrap_or(u32::MAX));
            if self.fixed {
                buf.extend_from_slice(&n.to_be_bytes());
            }
            buf.extend_from_slice(part);
        }
        buf
    }
}

impl Model for PackModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            collided: false,
            tag_clash: false,
            saw_witness: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.push(Act::WitnessPair);
        for a in 1u8..=4 {
            for b in 1u8..=4 {
                actions.push(Act::DistinctLens { a, b });
            }
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::WitnessPair => {
                next.saw_witness = true;
                let left = self.pack(&[b"a\x00b", b"c"]);
                let right = self.pack(&[b"a", b"b\x00c"]);
                if left == right {
                    next.collided = true;
                }
            }
            Act::DistinctLens { a, b } => {
                if a != b && self.tag(u32::from(a)) == self.tag(u32::from(b)) {
                    next.tag_clash = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-collide", inv_no_collide),
            Property::always("Inv-tag-injective", inv_tag),
            Property::sometimes("non-vacuity-witness", non_vacuity),
        ]
    }
}

fn inv_no_collide(_: &PackModel, s: &St) -> bool {
    !s.collided
}

fn inv_tag(_: &PackModel, s: &St) -> bool {
    !s.tag_clash
}

fn non_vacuity(_: &PackModel, s: &St) -> bool {
    s.saw_witness
}

#[test]
fn fixed_pack_holds() {
    let checker = PackModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_collides_nul_cut() {
    let checker = PackModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-collide").is_some(),
        "AS-IS pack([a\\0b,c]) must equal pack([a,b\\0c]) (F62 teeth)"
    );
}

#[test]
fn as_is_tags_clash() {
    let checker = PackModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-tag-injective").is_some(),
        "AS-IS pack_cut_tag must be 0 for every length (F62 teeth)"
    );
}
