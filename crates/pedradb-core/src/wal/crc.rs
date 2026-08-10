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

/// Compute the masked CRC over the record **type byte + payload**, which is
/// exactly the region RocksDB checksums for a physical log record.
#[must_use]
pub fn record_checksum(record_type: u8, data: &[u8]) -> u32 {
    // RocksDB extends the CRC over { type, data } in that order. We feed them
    // as a contiguous run without allocating.
    let crc = crc32c::crc32c_append(0, &[record_type]);
    let crc = crc32c::crc32c_append(crc, data);
    mask(crc)
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
}
