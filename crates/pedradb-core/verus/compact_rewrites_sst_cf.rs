// Verus twin of `src/cf_kernel.rs::compact_rewrites_sst_cf` (RFC-0150 P0).
// Model domain: `sst_cf_is_empty` stands in for the empty tag (mixed /
// legacy file); `same_family` stands in for the representative encoded-key
// membership `key_in_cf_family(encode_cf_key(sst_cf, "", false), family)`.
//
//   ./scripts/verus_compact_rewrites_sst_cf.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `compact_rewrites_sst_cf`: only a tagged file whose family
/// matches the compact's family is rewritten.
pub open spec fn compact_rewrites_sst_cf_spec(
    sst_cf_is_empty: bool,
    same_family: bool,
) -> bool {
    !sst_cf_is_empty && same_family
}

/// AS-IS compact leak: every SST is rewritten.
pub open spec fn compact_rewrites_sst_cf_as_is_spec(
    _sst_cf_is_empty: bool,
    _same_family: bool,
) -> bool {
    true
}

pub fn compact_rewrites_sst_cf(sst_cf_is_empty: bool, same_family: bool) -> (d: bool)
    ensures
        d == compact_rewrites_sst_cf_spec(sst_cf_is_empty, same_family),
{
    if sst_cf_is_empty {
        return false;
    }
    same_family
}

pub fn compact_rewrites_sst_cf_as_is(sst_cf_is_empty: bool, same_family: bool) -> (d: bool)
    ensures
        d == true,
{
    let _ = sst_cf_is_empty;
    let _ = same_family;
    true
}

/// A `default`-tagged file under a `lock` compact is left alone; AS-IS
/// rewrites it — the compact-leak dente (lock compact walks default).
proof fn lemma_as_is_compact_walks_foreign_cf()
    ensures
        compact_rewrites_sst_cf_spec(false, false) == false,
        compact_rewrites_sst_cf_as_is_spec(false, false) == true,
{
}

/// Mixed/legacy files (empty tag) are never rewritten, even in-family.
proof fn lemma_empty_tag_left_alone()
    ensures
        compact_rewrites_sst_cf_spec(true, true) == false,
        compact_rewrites_sst_cf_spec(true, false) == false,
        compact_rewrites_sst_cf_spec(false, true) == true,
{
}

} // verus!
