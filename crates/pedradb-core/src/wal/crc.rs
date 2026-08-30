//! CRC32C masking, compatible with RocksDB's log format.
//!
//! RocksDB does not store the raw CRC32C in a record header; it applies a
//! reversible mask so that data containing pre-existing valid checksums is
//! not accidentally accepted. We replicate the exact transform so a PedraDB
//! WAL is byte-compatible with RocksDB's expectations, and so the oracle
//! harness can cross-check our checksumming.
//!
//! Reference: RocksDB `util/crc32c.h` — `Mask` / `Unmask`.

/// Constant added during masking (same value as RocksDB's `kMaskDelta`).
pub const MASK_DELTA: u32 = 0xa282_ead8;

/// Mask a raw CRC32C value the way RocksDB does, for on-disk storage.
///
/// `masked = rotate_right_15(crc) + MASK_DELTA`
#[must_use]
pub fn mask(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(MASK_DELTA)
}

/// Reverse [`mask`]. Used when validating a record read back from disk.
#[must_use]
pub fn unmask(masked_crc: u32) -> u32 {
    masked_crc.wrapping_sub(MASK_DELTA).rotate_left(15)
}

/// Compute a raw (unmasked) CRC32C over `data`, matching the Castagnoli
/// polynomial used by both the `crc32c` crate and RocksDB.
#[must_use]
pub fn crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

/// Compute the masked CRC over **length (LE u16) + type byte + payload**.
///
/// PedraDB deliberately includes the length field (unlike classic RocksDB,
/// which checksums only `{type, data}`). A flipped length mid-file otherwise
/// looks like a clean torn tail (`Ok(None)`), silently dropping later durable
/// WAL records (F4). Pre-release: not byte-compatible with RocksDB WAL CRCs.
#[must_use]
pub fn record_checksum(record_type: u8, length: u16, data: &[u8]) -> u32 {
    let crc = crc32c::crc32c_append(0, &length.to_le_bytes());
    let crc = crc32c::crc32c_append(crc, &[record_type]);
    let crc = crc32c::crc32c_append(crc, data);
    mask(crc)
}

/// Admit a stored checksum against the computed one (RFC-0076 / R-hardware).
/// Mismatch is never Ok — never serve corruption as a valid record.
#[must_use]
pub fn crc_match_ok(stored: u32, computed: u32) -> bool {
    stored == computed
}

/// AS-IS: any checksum matches (the 0076 hole — silent-wrong record).
#[must_use]
pub fn crc_match_ok_as_is(_stored: u32, _computed: u32) -> bool {
    true
}

/// RFC-0076 P2.2 / R-crc: CRC32C collision-freedom as a Pedra theorem.
/// Always false. `crc_match_ok` is equality of two u32s, not a collision proof.
#[must_use]
pub fn crc_collision_admitted() -> bool {
    false
}

/// AS-IS: matching checksums look collision-free (the 0076 P2.2 hole).
#[must_use]
pub fn crc_collision_admitted_as_is() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_round_trips() {
        for &v in &[0u32, 1, 0xdead_beef, u32::MAX, 0x1234_5678] {
            assert_eq!(unmask(mask(v)), v, "round-trip failed for {v:#x}");
        }
    }

    #[test]
    fn known_mask_value() {
        // mask(0) == rotate + delta; sanity anchor independent of polynomial.
        assert_eq!(mask(0), MASK_DELTA);
    }

    #[test]
    fn crc_mismatch_is_not_ok() {
        assert!(crc_match_ok(1, 1));
        assert!(!crc_match_ok(1, 2));
        assert!(crc_match_ok_as_is(1, 2), "AS-IS dente: ignore mismatch");
    }

    /// RFC-0076 P2.2: equality of checksums is not a collision theorem.
    #[test]
    fn crc_collision_axiom_remains() {
        assert!(!crc_collision_admitted());
        assert!(
            crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }
}
