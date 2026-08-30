// Verus proof of F91/F92 origin-form routing (RFC-0002 P34).
// Twin of `path_kernel::strip_authority_for_routing` (string slice is caller).
//
//   ./scripts/verus_origin_path.sh

use vstd::prelude::*;

verus! {

pub open spec fn strip_authority_for_routing_spec(is_authority_form: bool) -> bool {
    is_authority_form
}

pub open spec fn strip_authority_for_routing_as_is_spec(_is_authority_form: bool) -> bool {
    false
}

/// F91/F92: authority-form targets are stripped to origin-form before routing.
pub fn strip_authority_for_routing(is_authority_form: bool) -> (d: bool)
    ensures
        d == strip_authority_for_routing_spec(is_authority_form),
        d == is_authority_form,
{
    is_authority_form
}

/// AS-IS: `strip_prefix("/kv/")` on the raw target (scheme/authority stay).
pub fn strip_authority_for_routing_as_is(is_authority_form: bool) -> (d: bool)
    ensures
        d == strip_authority_for_routing_as_is_spec(is_authority_form),
        !d,
{
    let _ = is_authority_form;
    false
}

proof fn lemma_as_is_keeps_authority()
    ensures
        strip_authority_for_routing_spec(true),
        !strip_authority_for_routing_as_is_spec(true),
{
}

proof fn lemma_origin_form_untouched()
    ensures
        !strip_authority_for_routing_spec(false),
{
}

} // verus!
