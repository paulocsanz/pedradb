// Verus twin of `src/cf_kernel.rs::infer_sst_cf` (RFC-0150 P0).
// Model domain: each bound is `None` (no bound) or `Some(is_default)`
// (bound present, family-is-default stand-in); the result is
// `None` = empty tag (mixed / no bounds) or `Some(is_default)` = family.
//
//   ./scripts/verus_infer_sst_cf.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `infer_sst_cf`: agreeing bounds tag their family; disagreeing
/// bounds tag empty (mixed); a one-sided bound tags its own family.
pub open spec fn infer_sst_cf_spec(
    smallest: Option<bool>,
    largest: Option<bool>,
) -> Option<bool> {
    match (smallest, largest) {
        (Some(a), Some(b)) => if a == b { Some(a) } else { None },
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

/// AS-IS tag loss: every SST tags `default`.
pub open spec fn infer_sst_cf_as_is_spec(
    _smallest: Option<bool>,
    _largest: Option<bool>,
) -> Option<bool> {
    Some(true)
}

pub fn infer_sst_cf(smallest: Option<bool>, largest: Option<bool>) -> (d: Option<bool>)
    ensures
        d == infer_sst_cf_spec(smallest, largest),
{
    match (smallest, largest) {
        (Some(a), Some(b)) => if a == b { Some(a) } else { None },
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

pub fn infer_sst_cf_as_is(smallest: Option<bool>, largest: Option<bool>) -> (d: Option<bool>)
    ensures
        d == Some(true),
{
    let _ = smallest;
    let _ = largest;
    Some(true)
}

/// Mixed bounds (a `lock` and a raw `default` key) tag empty; AS-IS tags
/// `default` — the tag-loss dente (mixed file compacted as default).
proof fn lemma_as_is_tags_mixed_file_default()
    ensures
        infer_sst_cf_spec(Some(false), Some(true)) == None,
        infer_sst_cf_as_is_spec(Some(false), Some(true)) == Some(true),
{
}

/// Agreeing named bounds tag the named family; one-sided bounds tag their
/// own family; a lock-tagged file is not default.
proof fn lemma_agree_and_one_sided()
    ensures
        infer_sst_cf_spec(Some(false), Some(false)) == Some(false),
        infer_sst_cf_spec(None, Some(false)) == Some(false),
        infer_sst_cf_spec(Some(true), Some(true)) == Some(true),
        infer_sst_cf_spec(None, None) == None,
{
}

} // verus!
