// Verus twin of `src/cf_kernel.rs::decode_cf_key` (RFC-0150 P0).
// Model domain: `effective_is_empty` / `effective_len` / `enc_len` stand in
// for the byte slices; the result is (prefix stripped, user-key len).
//
//   ./scripts/verus_decode_cf_key.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `decode_cf_key`: raw passthrough when the effective prefix is
/// empty; else strip `prefix + NUL`, with no user bytes for a truncated
/// tag (the kernel's `unwrap_or(&[])`).
pub open spec fn decode_cf_key_spec(
    effective_is_empty: bool,
    effective_len: u64,
    enc_len: u64,
) -> (bool, u64) {
    if effective_is_empty {
        (false, enc_len)
    } else if enc_len > effective_len + 1 {
        (true, (enc_len - (effective_len + 1)) as u64)
    } else {
        (true, 0)
    }
}

/// AS-IS strip loss: the `cf\0` prefix is returned to the caller as part
/// of the user key.
pub open spec fn decode_cf_key_as_is_spec(
    _effective_is_empty: bool,
    _effective_len: u64,
    enc_len: u64,
) -> (bool, u64) {
    (false, enc_len)
}

pub fn decode_cf_key(
    effective_is_empty: bool,
    effective_len: u64,
    enc_len: u64,
) -> (d: (bool, u64))
    requires
        // prefix + NUL fits (spec math is unbounded; exec is u64)
        effective_len < u64::MAX,
    ensures
        d == decode_cf_key_spec(effective_is_empty, effective_len, enc_len),
{
    if effective_is_empty {
        (false, enc_len)
    } else if enc_len > effective_len + 1 {
        (true, enc_len - (effective_len + 1))
    } else {
        (true, 0)
    }
}

pub fn decode_cf_key_as_is(
    effective_is_empty: bool,
    effective_len: u64,
    enc_len: u64,
) -> (d: (bool, u64))
    ensures
        d == decode_cf_key_as_is_spec(effective_is_empty, effective_len, enc_len),
{
    let _ = effective_is_empty;
    let _ = effective_len;
    (false, enc_len)
}

/// Decoding `lock\0k` yields user key `k`; AS-IS yields the full 6 bytes
/// with the prefix — the strip-loss dente.
proof fn lemma_as_is_leaks_prefix_into_user_key()
    ensures
        decode_cf_key_spec(false, 4, 6) == (true, 1),
        decode_cf_key_as_is_spec(false, 4, 6) == (false, 6),
{
}

/// Raw-default passthrough keeps the stored bytes untouched; a truncated
/// tag yields no user bytes.
proof fn lemma_raw_passthrough_and_truncated_tag()
    ensures
        decode_cf_key_spec(true, 0, 5) == (false, 5),
        decode_cf_key_spec(false, 4, 5) == (true, 0),
{
}

} // verus!
