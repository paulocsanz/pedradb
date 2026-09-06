// Verus twin of `src/cf_kernel.rs::cf_family_of` (RFC-0150 P0).
// Model domain: the NUL position stands in for `&[u8]`;
// `prefix_is_default` stands in for the prefix-vs-`"default"` byte compare.
//
//   ./scripts/verus_cf_family_of.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `cf_family_of`: the family is `"default"` when the key is raw
/// (no NUL), has a leading NUL, or its prefix is exactly `"default"`.
pub open spec fn cf_family_of_spec(nul_pos: Option<u64>, prefix_is_default: bool) -> bool {
    match nul_pos {
        None | Some(0) => true,
        Some(_) => prefix_is_default,
    }
}

/// AS-IS family loss: every key reports `"default"`.
pub open spec fn cf_family_of_as_is_spec(
    _nul_pos: Option<u64>,
    _prefix_is_default: bool,
) -> bool {
    true
}

pub fn cf_family_of(nul_pos: Option<u64>, prefix_is_default: bool) -> (d: bool)
    ensures
        d == cf_family_of_spec(nul_pos, prefix_is_default),
{
    match nul_pos {
        None | Some(0) => true,
        Some(_) => prefix_is_default,
    }
}

pub fn cf_family_of_as_is(nul_pos: Option<u64>, prefix_is_default: bool) -> (d: bool)
    ensures
        d == true,
{
    let _ = nul_pos;
    let _ = prefix_is_default;
    true
}

/// A named prefix (`lock\0k`: NUL at 4, prefix not default) keeps its own
/// family. AS-IS reports `default` — the family-loss dente.
proof fn lemma_as_is_loses_named_family()
    ensures
        cf_family_of_spec(Some(4), false) == false,
        cf_family_of_as_is_spec(Some(4), false) == true,
{
}

/// Raw keys, leading-NUL keys, and `default\0…` keys are `default`.
proof fn lemma_default_for_raw_and_prefixed()
    ensures
        cf_family_of_spec(None, false) == true,
        cf_family_of_spec(Some(0), false) == true,
        cf_family_of_spec(Some(7), true) == true,
{
}

/// The named-prefix branch only applies when the prefix is non-empty.
proof fn lemma_named_prefix_requires_positive_len(
    len: u64,
    prefix_is_default: bool,
)
    requires
        len > 0,
    ensures
        cf_family_of_spec(Some(len), prefix_is_default) == prefix_is_default,
{
}

} // verus!
