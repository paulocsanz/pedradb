//! Pure SST magic admission (RFC-0186 P2.2).
//!
//! **Single artifact (pair `sst_magic`):** this file is what `rustc` links
//! *and* what Verus proves (`cfg(verus_keep_ghost)`).
//!
//!   ./scripts/verus_sst_magic.sh
//!
//! A file is a Pedra SST iff it carries at least the 8-byte `PEDRSST\0`
//! prefix. The C++ BlockBasedTable puts its magic in the *footer*, so a
//! Rocks `.sst` starts with block bytes and must not be admitted — the
//! on-disk drop-in lie. `SstTable::decode` and the ops `DirKind` classifier
//! both call this one predicate (no inline `!= SST_MAGIC` duplicates).

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// Model of rustc `sst_magic_is_pedra` (Vest-class: the 8-byte prefix and
/// the header length stand in for the slice compare).
pub open spec fn sst_magic_is_pedra_spec(hdr_len: u64, first8_is_magic: bool) -> bool {
    hdr_len >= 8 && first8_is_magic
}

/// AS-IS drop-in lie: every header is a Pedra SST (the C++ footer-magic
/// file would be decoded — silent-wrong or a lie about what was opened).
pub open spec fn sst_magic_is_pedra_as_is_spec(_hdr_len: u64, _first8_is_magic: bool) -> bool {
    true
}

pub fn sst_magic_is_pedra(hdr_len: u64, first8_is_magic: bool) -> (d: bool)
    ensures
        d == sst_magic_is_pedra_spec(hdr_len, first8_is_magic),
{
    hdr_len >= 8 && first8_is_magic
}

pub fn sst_magic_is_pedra_as_is(_hdr_len: u64, _first8_is_magic: bool) -> (d: bool)
    ensures
        d == true,
{
    true
}

/// A C++ header (8 bytes, footer magic) is not admitted; AS-IS blesses it.
proof fn lemma_as_is_admits_cpp_header()
    ensures
        !sst_magic_is_pedra_spec(8, false),
        sst_magic_is_pedra_as_is_spec(8, false),
{
}

/// A truncated 7-byte prefix of the magic itself is not admitted.
proof fn lemma_truncated_magic_is_not_pedra()
    ensures
        !sst_magic_is_pedra_spec(7, true),
{
}

} // verus!

use super::table::SST_MAGIC;

/// Header is a Pedra SST iff it carries at least the 8-byte `PEDRSST\0`
/// prefix. A C++ Rocks `.sst` (magic in the footer) and any truncated
/// header are not admitted (RFC-0186: not drop-in on disk).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn sst_magic_is_pedra(header: &[u8]) -> bool {
    header.len() >= SST_MAGIC.len() && header[..SST_MAGIC.len()] == SST_MAGIC[..]
}

/// AS-IS RFC-0186: every header is a Pedra SST (the on-disk drop-in lie).
#[cfg(not(verus_keep_ghost))]
#[must_use]
pub fn sst_magic_is_pedra_as_is(_header: &[u8]) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::sst_magic_is_pedra;

    #[test]
    fn admits_only_pedra_prefix() {
        assert!(sst_magic_is_pedra(b"PEDRSST\0"));
        assert!(sst_magic_is_pedra(b"PEDRSST\0extra-bytes"));
        // A C++ BlockBasedTable starts with block data, not our magic.
        assert!(!sst_magic_is_pedra(&[0u8; 8]));
        assert!(!sst_magic_is_pedra(b"\0\x00\x00\x00\x00\x00\x00\x00"));
        // Truncated header — even a real magic prefix of 7 bytes — refuses.
        assert!(!sst_magic_is_pedra(b"PEDRSST"));
        assert!(!sst_magic_is_pedra(b""));
    }

    /// RFC-0186 P2.2 dente: the AS-IS twin would admit the C++ header.
    #[test]
    fn sst_magic_as_is_admits_cpp_header() {
        let cpp = [0u8; 8];
        assert!(!sst_magic_is_pedra(&cpp));
        assert!(
            super::sst_magic_is_pedra_as_is(&cpp),
            "AS-IS dente: any header is Pedra (on-disk drop-in lie)"
        );
    }
}
