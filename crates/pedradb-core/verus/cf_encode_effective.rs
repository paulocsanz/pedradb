// Verus twin of `src/cf_kernel.rs::cf_encode_effective` (RFC-0150 P0).
// Model domain: `cf_is_default` stands in for `cf == "default"`; the result
// models "effective prefix is empty" (raw on-disk layout).
//
//   ./scripts/verus_cf_encode_effective.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `cf_encode_effective`: the prefix is empty only for `default`
/// in raw-default mode.
pub open spec fn cf_encode_effective_spec(cf_is_default: bool, default_raw: bool) -> bool {
    cf_is_default && default_raw
}

/// AS-IS raw-mode loss: the prefix is never empty, so `default` keys are
/// always stored prefixed and the raw layout contract is lost.
pub open spec fn cf_encode_effective_as_is_spec(
    _cf_is_default: bool,
    _default_raw: bool,
) -> bool {
    false
}

pub fn cf_encode_effective(cf_is_default: bool, default_raw: bool) -> (d: bool)
    ensures
        d == cf_encode_effective_spec(cf_is_default, default_raw),
{
    cf_is_default && default_raw
}

pub fn cf_encode_effective_as_is(cf_is_default: bool, default_raw: bool) -> (d: bool)
    ensures
        d == false,
{
    let _ = cf_is_default;
    let _ = default_raw;
    false
}

/// `default` in raw mode is stored raw; AS-IS still prefixes it — the
/// raw-mode-loss dente.
proof fn lemma_as_is_never_raw()
    ensures
        cf_encode_effective_spec(true, true) == true,
        cf_encode_effective_as_is_spec(true, true) == false,
{
}

/// Named CFs always carry their prefix; `default` carries none when
/// raw mode is off.
proof fn lemma_named_cf_and_non_raw_default()
    ensures
        cf_encode_effective_spec(false, true) == false,
        cf_encode_effective_spec(false, false) == false,
        cf_encode_effective_spec(true, false) == false,
{
}

} // verus!
