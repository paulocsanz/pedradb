//! Stateright model over the **real** [`montanha_fdb_recipes::fields_kernel`] (F60).
//!
//! Faithfulness: encode/decode and child suffix are production
//! `fields_kernel`. Tuple pack is an axiom.
//!
//! Zip `9\x000` as a field: length-prefixed keeps it; first-NUL split
//! truncates to `9`. Child id `[a,0,b]` after `pack||0x00` stays whole;
//! last-NUL rsplit keeps only `b`.
//!
//! - **Inv-field-kept:** `field_kept(len, nul_at) == len`.
//! - **Inv-roundtrip:** encode+decode of a NUL-containing field recovers it.
//! - **Inv-child-id:** `child_bytes_after` keeps the full id including NULs.
//! - AS-IS must truncate (teeth).

use montanha_fdb_recipes::{
    child_bytes_after, child_bytes_after_as_is, decode_fields, decode_pair_first_nul,
    encode_fields, encode_fields_as_is, field_kept, field_kept_as_is,
};
use stateright::{Checker, Model, Property};

fn nul_zip() -> &'static [u8] {
    b"9\x000"
}

fn child_start() -> &'static [u8] {
    b"zip\x0090\x00"
}

fn child_id() -> &'static [u8] {
    b"a\x00b"
}

fn child_key() -> Vec<u8> {
    let mut k = child_start().to_vec();
    k.extend_from_slice(child_id());
    k
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    field_truncated: bool,
    payload_truncated: bool,
    child_truncated: bool,
    saw_payload: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    FieldKept { len: u8, nul_at: u8 },
    PayloadNulZip,
    ChildIdWithNul,
}

#[derive(Clone)]
struct FieldsModel {
    fixed: bool,
}

impl FieldsModel {
    fn kept(&self, len: u64, nul_at: u64) -> u64 {
        if self.fixed {
            field_kept(len, nul_at)
        } else {
            field_kept_as_is(len, nul_at)
        }
    }

    fn payload_parts(&self, zip: &[u8], name: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
        if self.fixed {
            let raw = encode_fields(&[zip, name]);
            let got = decode_fields(&raw, 2)?;
            Some((got[0].clone(), got[1].clone()))
        } else {
            let raw = encode_fields_as_is(&[zip, name]);
            Some(decode_pair_first_nul(&raw))
        }
    }

    fn child_rest<'a>(&self, key: &'a [u8], start: &[u8]) -> Option<&'a [u8]> {
        if self.fixed {
            child_bytes_after(key, start)
        } else {
            child_bytes_after_as_is(key, start)
        }
    }
}

impl Model for FieldsModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            field_truncated: false,
            payload_truncated: false,
            child_truncated: false,
            saw_payload: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            Act::FieldKept { len: 3, nul_at: 1 },
            Act::FieldKept { len: 3, nul_at: 0 },
            Act::FieldKept { len: 3, nul_at: 3 },
            Act::FieldKept { len: 0, nul_at: 0 },
            Act::PayloadNulZip,
            Act::ChildIdWithNul,
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::FieldKept { len, nul_at } => {
                if self.kept(u64::from(len), u64::from(nul_at)) != u64::from(len) {
                    next.field_truncated = true;
                }
            }
            Act::PayloadNulZip => {
                next.saw_payload = true;
                match self.payload_parts(nul_zip(), b"alice") {
                    Some((a, b)) if a == nul_zip() && b == b"alice" => {}
                    _ => next.payload_truncated = true,
                }
            }
            Act::ChildIdWithNul => {
                if self.child_rest(&child_key(), child_start()) != Some(child_id()) {
                    next.child_truncated = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-field-kept", inv_field_kept),
            Property::always("Inv-roundtrip", inv_roundtrip),
            Property::always("Inv-child-id", inv_child_id),
            Property::sometimes("non-vacuity-payload", non_vacuity),
        ]
    }
}

fn inv_field_kept(_: &FieldsModel, s: &St) -> bool {
    !s.field_truncated
}

fn inv_roundtrip(_: &FieldsModel, s: &St) -> bool {
    !s.payload_truncated
}

fn inv_child_id(_: &FieldsModel, s: &St) -> bool {
    !s.child_truncated
}

fn non_vacuity(_: &FieldsModel, s: &St) -> bool {
    s.saw_payload
}

#[test]
fn fixed_fields_holds() {
    let checker = FieldsModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_truncates_field() {
    let checker = FieldsModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-field-kept").is_some(),
        "AS-IS field_kept must keep only [0, nul_at) (F60 teeth)"
    );
}

#[test]
fn as_is_truncates_nul_zip() {
    let checker = FieldsModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-roundtrip").is_some(),
        "AS-IS first-NUL split must drop the tail of zip 9\\0 0 (F60 teeth)"
    );
}

#[test]
fn as_is_truncates_child_id() {
    let checker = FieldsModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-child-id").is_some(),
        "AS-IS last-NUL rsplit must keep only the last id segment (F60 teeth)"
    );
}
