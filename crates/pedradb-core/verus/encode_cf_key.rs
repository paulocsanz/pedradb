// Verus twin of `src/cf_kernel.rs::encode_cf_key` (RFC-0150 P0).
// Model domain: `effective_is_empty` / `effective_len` / `key_len` stand in
// for the byte slices; the result is (carries the `cf\0` prefix, out len).
//
//   ./scripts/verus_encode_cf_key.sh
//
// Do not link this into the production crate.

use vstd::prelude::*;

verus! {

/// Model of `encode_cf_key`: raw passthrough when the effective prefix is
/// empty, else `prefix ++ NUL ++ key`.
pub open spec fn encode_cf_key_spec(
    effective_is_empty: bool,
    effective_len: u64,
    key_len: u64,
) -> (bool, u64) {
    if effective_is_empty {
        (false, key_len)
    } else {
        (true, (effective_len + 1 + key_len) as u64)
    }
}

/// AS-IS prefix loss: always raw — a `lock` key and a `default` key encode
/// to the same bytes (cross-CF collision).
pub open spec fn encode_cf_key_as_is_spec(
    _effective_is_empty: bool,
    _effective_len: u64,
    key_len: u64,
) -> (bool, u64) {
    (false, key_len)
}

pub fn encode_cf_key(
    effective_is_empty: bool,
    effective_len: u64,
    key_len: u64,
) -> (d: (bool, u64))
    requires
        // prefix + NUL + key fits (spec math is unbounded; exec is u64)
        effective_len + 1 + key_len <= u64::MAX,
    ensures
        d == encode_cf_key_spec(effective_is_empty, effective_len, key_len),
{
    if effective_is_empty {
        (false, key_len)
    } else {
        (true, effective_len + 1 + key_len)
    }
}

pub fn encode_cf_key_as_is(
    effective_is_empty: bool,
    effective_len: u64,
    key_len: u64,
) -> (d: (bool, u64))
    ensures
        d == encode_cf_key_as_is_spec(effective_is_empty, effective_len, key_len),
{
    let _ = effective_is_empty;
    let _ = effective_len;
    (false, key_len)
}

/// `encode_cf_key("lock", "k")` carries `lock\0`; AS-IS emits raw `k`,
/// colliding with a default-raw `k` — the prefix-loss dente.
proof fn lemma_as_is_prefix_loss_collides_across_cfs()
    ensures
        encode_cf_key_spec(false, 4, 1) == (true, 6),
        encode_cf_key_as_is_spec(false, 4, 1) == (false, 1),
        encode_cf_key_spec(true, 0, 1) == (false, 1),
        encode_cf_key_as_is_spec(false, 4, 1).1 == encode_cf_key_spec(true, 0, 1).1,
{
}

/// The NUL separator is always counted: out len is prefix + 1 + key.
proof fn lemma_nul_separator_counts(
    effective_is_empty: bool,
    effective_len: u64,
    key_len: u64,
)
    requires
        effective_is_empty == false,
        effective_len > 0,
        effective_len + 1 + key_len <= u64::MAX,
    ensures
        encode_cf_key_spec(effective_is_empty, effective_len, key_len).0 == true,
        encode_cf_key_spec(effective_is_empty, effective_len, key_len).1
            == effective_len + 1 + key_len,
{
}

} // verus!
