// Verus twin of `src/cf_kernel.rs::key_in_cf_family` (RFC-0150 P0).
// Model domain: NUL position / prefix equality stand in for `&[u8]`.
//
//   ./scripts/verus_cf_family.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `key_in_cf_family`.
///
/// `nul_pos`: `None` = no NUL (raw key); `Some(0)` = leading NUL;
/// `Some(i)` with i > 0 = named prefix of length i.
/// `family_is_default` / `prefix_is_default` / `prefix_is_family` stand in
/// for the byte compares; `key_len` / `family_len` for the named-CF length check.
pub open spec fn key_in_cf_family_spec(
    nul_pos: Option<u64>,
    family_is_default: bool,
    prefix_is_default: bool,
    prefix_is_family: bool,
    key_len: u64,
    family_len: u64,
) -> bool {
    if family_is_default {
        match nul_pos {
            None | Some(0) => true,
            Some(_i) => prefix_is_default,
        }
    } else {
        key_len > family_len && prefix_is_family
    }
}

/// AS-IS scan leak: every key is in-family.
pub open spec fn key_in_cf_family_as_is_spec(
    _nul_pos: Option<u64>,
    _family_is_default: bool,
    _prefix_is_default: bool,
    _prefix_is_family: bool,
    _key_len: u64,
    _family_len: u64,
) -> bool {
    true
}

pub fn key_in_cf_family(
    nul_pos: Option<u64>,
    family_is_default: bool,
    prefix_is_default: bool,
    prefix_is_family: bool,
    key_len: u64,
    family_len: u64,
) -> (d: bool)
    ensures
        d == key_in_cf_family_spec(
            nul_pos,
            family_is_default,
            prefix_is_default,
            prefix_is_family,
            key_len,
            family_len,
        ),
{
    if family_is_default {
        match nul_pos {
            None | Some(0) => true,
            Some(_i) => prefix_is_default,
        }
    } else {
        key_len > family_len && prefix_is_family
    }
}

pub fn key_in_cf_family_as_is(
    _nul_pos: Option<u64>,
    _family_is_default: bool,
    _prefix_is_default: bool,
    _prefix_is_family: bool,
    _key_len: u64,
    _family_len: u64,
) -> (d: bool)
    ensures
        d == true,
{
    true
}

/// A `lock\0k` key (`nul_pos = Some(4)`, prefix not default) is not in
/// `default`. AS-IS admits it (scan leak).
proof fn lemma_as_is_admits_foreign_cf()
    ensures
        !key_in_cf_family_spec(Some(4), true, false, false, 6, 7),
        key_in_cf_family_as_is_spec(Some(4), true, false, false, 6, 7),
{
}

/// Raw and `default\0…` keys are in `default`.
proof fn lemma_default_matches_raw_and_prefixed()
    ensures
        key_in_cf_family_spec(None, true, false, false, 3, 7),
        key_in_cf_family_spec(Some(0), true, false, false, 2, 7),
        key_in_cf_family_spec(Some(7), true, true, false, 9, 7),
{
}

} // verus!
