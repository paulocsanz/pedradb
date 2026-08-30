// Verus proof of the fail-closed bloom header bound (F166).
// Twin of `pedradb-core/src/bloom.rs` `bloom_header_ok` (same u32/u64 domain).
//
//   ./scripts/verus_bloom_header.sh

use vstd::prelude::*;

verus! {

pub struct Params {
    pub nbits: u32,
    pub k: u32,
    pub nbytes: u32,
    pub residual: u64,
}

pub open spec fn bloom_header_ok_spec(p: Params) -> bool {
    &&& p.k >= 1
    &&& p.k <= 30
    &&& p.nbytes as u64 >= (p.nbits as u64 + 7) / 8
    &&& p.nbytes as u64 <= p.residual
}

pub fn bloom_header_ok(nbits: u32, k: u32, nbytes: u32, residual: u64) -> (ok: bool)
    ensures
        ok == bloom_header_ok_spec(Params { nbits: nbits, k: k, nbytes: nbytes, residual: residual }),
        // F166: accepting bounds the per-lookup probe loop (`may_contain`
        // iterates k times) and the bits slice it indexes.
        ok ==> 1 <= k <= 30,
        ok ==> nbytes as u64 * 8 >= nbits as u64,
        ok ==> nbytes as u64 <= residual,
{
    k >= 1
        && k <= 30
        && nbytes as u64 >= (nbits as u64 + 7) / 8
        && nbytes as u64 <= residual
}

/// AS-IS F166: no probe-count bound.
pub open spec fn bloom_header_ok_as_is_spec(p: Params) -> bool {
    &&& p.nbytes as u64 >= (p.nbits as u64 + 7) / 8
    &&& p.nbytes as u64 <= p.residual
}

proof fn lemma_as_is_accepts_hostile_k(nbits: u32, nbytes: u32, residual: u64)
    requires
        nbytes as u64 >= (nbits as u64 + 7) / 8,
        nbytes as u64 <= residual,
    ensures
        bloom_header_ok_spec(Params { nbits: nbits, k: 31, nbytes: nbytes, residual: residual })
            == false,
        bloom_header_ok_as_is_spec(Params {
            nbits: nbits,
            k: u32::MAX,
            nbytes: nbytes,
            residual: residual,
        }),
        bloom_header_ok_spec(Params { nbits: nbits, k: u32::MAX, nbytes: nbytes, residual: residual })
            == false,
{
}

/// Any filter the writer can build passes the check (no false rejection of
/// self-produced headers): `with_capacity` clamps k into [1, 30].
proof fn lemma_writer_headers_pass(nbits: u32, k: u32, nbytes: u32)
    requires
        1 <= k <= 30,
        nbytes as u64 == (nbits as u64 + 7) / 8,
    ensures
        bloom_header_ok_spec(Params { nbits: nbits, k: k, nbytes: nbytes, residual: nbytes as u64 }),
{
}

} // verus!
