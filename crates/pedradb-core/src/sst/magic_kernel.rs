//! Pure SST magic admission (RFC-0186 P2.2).
//!
//! **Term:** this file is what `rustc` links. A u64/bool fingerprint of
//! the rustc `&[u8]` prefix compare is a model twin — not last-wins
//! (deleted).
//!
//! A file is a Pedra SST iff it carries at least the 8-byte `PEDRSST\0`
//! prefix. The C++ BlockBasedTable puts its magic in the *footer*, so a
//! Rocks `.sst` starts with block bytes and must not be admitted — the
//! on-disk drop-in lie. `SstTable::decode` and the ops `DirKind` classifier
//! both call this one predicate (no inline `!= SST_MAGIC` duplicates).

use super::table::SST_MAGIC;

/// Header is a Pedra SST iff it carries at least the 8-byte `PEDRSST\0`
/// prefix. A C++ Rocks `.sst` (magic in the footer) and any truncated
/// header are not admitted (RFC-0186: not drop-in on disk).
#[must_use]
pub fn sst_magic_is_pedra(header: &[u8]) -> bool {
    header.len() >= SST_MAGIC.len() && header[..SST_MAGIC.len()] == SST_MAGIC[..]
}

/// AS-IS RFC-0186: every header is a Pedra SST (the on-disk drop-in lie).
#[must_use]
pub fn sst_magic_is_pedra_as_is(_header: &[u8]) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::sst_magic_is_pedra;

    #[test]
    fn magic_kernel_has_no_verus_cartoon() {
        let src = include_str!("magic_kernel.rs");
        let block = concat!("verus", "!", " {");
        let cfg = concat!("cfg(", "verus", "_keep", "_ghost)");
        assert!(
            !src.contains(block),
            "u64 fingerprint is not last-wins of rustc &[u8] SST magic"
        );
        assert!(
            !src.contains(cfg),
            "cfg split hides rustc types from the prover"
        );
    }

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
