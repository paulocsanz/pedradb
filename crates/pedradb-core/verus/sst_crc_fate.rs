// Verus twin of `src/sst/scan_kernel.rs::sst_crc_fate` (RFC-0077 P2.1).
// Not linked into production. scan_guard.rs stays the F167 model twin.
// Not a zero-glue theorem (P2.2 / R-glue).
//
//   ./scripts/verus_sst_crc_fate.sh

use vstd::prelude::*;

verus! {

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SstCrcFate {
    StripTrailer,
    WholeBuffer,
    Reject,
}

pub open spec fn sst_crc_fate_spec(stored: u32, computed: u32, buf_len: usize) -> SstCrcFate {
    if stored == computed {
        SstCrcFate::StripTrailer
    } else if buf_len < 32 {
        SstCrcFate::WholeBuffer
    } else {
        SstCrcFate::Reject
    }
}

pub open spec fn sst_crc_fate_as_is_spec(_stored: u32, _computed: u32, _buf_len: usize) -> SstCrcFate {
    SstCrcFate::StripTrailer
}

pub fn sst_crc_fate(stored: u32, computed: u32, buf_len: usize) -> (f: SstCrcFate)
    ensures
        f == sst_crc_fate_spec(stored, computed, buf_len),
{
    if stored == computed {
        SstCrcFate::StripTrailer
    } else if buf_len < 32 {
        SstCrcFate::WholeBuffer
    } else {
        SstCrcFate::Reject
    }
}

pub fn sst_crc_fate_as_is(_stored: u32, _computed: u32, _buf_len: usize) -> (f: SstCrcFate)
    ensures
        f == sst_crc_fate_as_is_spec(_stored, _computed, _buf_len),
{
    SstCrcFate::StripTrailer
}

proof fn lemma_mismatch_on_modern_file_is_reject()
    ensures
        sst_crc_fate_spec(1, 1, 100) == SstCrcFate::StripTrailer,
        sst_crc_fate_spec(1, 2, 100) == SstCrcFate::Reject,
        sst_crc_fate_spec(1, 2, 16) == SstCrcFate::WholeBuffer,
        sst_crc_fate_as_is_spec(1, 2, 100) == SstCrcFate::StripTrailer,
{
}

/// RFC-0155 P2.1 / R-glue: zero remaining glue is not a theorem.
pub open spec fn zero_glue_admitted_spec() -> bool {
    false
}

pub open spec fn zero_glue_admitted_as_is_spec() -> bool {
    true
}

pub fn zero_glue_admitted() -> (ok: bool)
    ensures
        ok == zero_glue_admitted_spec(),
{
    false
}

pub fn zero_glue_admitted_as_is() -> (ok: bool)
    ensures
        ok == zero_glue_admitted_as_is_spec(),
{
    true
}

} // verus!
