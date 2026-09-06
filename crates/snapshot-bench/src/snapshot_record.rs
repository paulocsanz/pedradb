//! Shared on-disk value-record codec for the LSM-backed [`SnapshotStore`]
//! backends (`FjallSnapshot`, `RocksDbSnapshot`, `PedraDbSnapshot`).
//!
//! All three backends store the folded KV state as `key` → `[ver_len:u8][version
//! bytes][value bytes]`. Keeping the codec (and its corruption tests) in one
//! place means the record format cannot drift between backends — a store
//! written by one decodes identically in the other's terms.
//!
//! Ported from beyondoss/slipstream `src/snapshot_record.rs` (MIT), verbatim.
//!
//! [`SnapshotStore`]: crate::snapshot::SnapshotStore

use crate::kv::{KvEntry, VersionToken};
use crate::snapshot::SnapshotError;

/// Encode a stored value as `[ver_len:u8][version bytes][value bytes]` into `buf`.
///
/// `buf` is cleared and refilled (its capacity is reused across a batch). The
/// version is length-prefixed raw bytes so a backend's token (u64 revision,
/// FDB 10-byte versionstamp) must round-trip intact.
///
/// `VersionToken` caps inline storage at 10 bytes, so the `u8` length prefix
/// never truncates today. Checking with `try_from` rather than casting
/// surfaces a format error instead of silently writing a wrong length — which
/// would frame a record `decode_entry` then mis-parses — if a future token
/// ever widens past 255 bytes.
pub(crate) fn encode_value_into(
    buf: &mut Vec<u8>,
    value: &[u8],
    version: &VersionToken,
) -> Result<(), SnapshotError> {
    let vb = version.as_bytes();
    let ver_len = u8::try_from(vb.len()).map_err(|_| {
        SnapshotError::InvalidFormat(format!(
            "version too long: {} bytes (max {})",
            vb.len(),
            u8::MAX
        ))
    })?;
    buf.clear();
    buf.reserve(1 + vb.len() + value.len());
    buf.push(ver_len);
    buf.extend_from_slice(vb);
    buf.extend_from_slice(value);
    Ok(())
}

/// Decode a `[ver_len:u8][version][value]` record back into a [`KvEntry`].
pub(crate) fn decode_entry(key: &str, raw: &[u8]) -> Result<KvEntry, SnapshotError> {
    let ver_len = *raw.first().ok_or_else(|| {
        SnapshotError::InvalidFormat("snapshot value record is empty (no version length)".into())
    })? as usize;
    let value_off = 1 + ver_len;
    if raw.len() < value_off {
        return Err(SnapshotError::InvalidFormat(format!(
            "snapshot value record truncated: need {value_off} bytes for version, have {}",
            raw.len()
        )));
    }
    let version = VersionToken::from_raw(&raw[1..value_off]).ok_or_else(|| {
        SnapshotError::InvalidFormat(format!(
            "version length {ver_len} exceeds version token capacity"
        ))
    })?;
    Ok(KvEntry {
        key: key.to_string(),
        value: raw[value_off..].to_vec(),
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10-byte FDB versionstamp has no `u64` form; the length-prefixed value
    /// format must carry it intact. A `u64`-only field would flatten it to 0
    /// and silently break every later CAS — so this is the load-bearing reason
    /// the record stores a length-prefixed token rather than a fixed 8 bytes.
    #[test]
    fn encode_decode_round_trips_fdb_versionstamp() {
        let vs = VersionToken::from_fdb_versionstamp(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let mut enc = Vec::new();
        encode_value_into(&mut enc, b"payload", &vs).expect("encode");
        let entry = decode_entry("k", &enc).expect("decode");

        assert_eq!(entry.version.as_bytes(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert!(
            entry.version.as_u64().is_none(),
            "a 10-byte token has no u64 form — it must not be flattened"
        );
        assert_eq!(entry.value, b"payload");
    }

    /// An empty value encodes to just the version prefix and decodes back to a
    /// present, empty-valued entry with its version intact.
    #[test]
    fn encode_decode_round_trips_empty_value() {
        let mut enc = Vec::new();
        encode_value_into(&mut enc, b"", &VersionToken::from_u64(7)).expect("encode");
        let entry = decode_entry("k", &enc).expect("decode");

        assert!(entry.value.is_empty());
        assert_eq!(entry.version.as_u64(), Some(7));
    }

    /// A zero-byte record has no version-length byte — corruption, not a valid
    /// record. It must surface as a recoverable `InvalidFormat`, never a panic.
    #[test]
    fn decode_entry_rejects_empty_record() {
        let err = decode_entry("k", &[]).unwrap_err();
        assert!(
            matches!(err, SnapshotError::InvalidFormat(_)),
            "empty record must be a format error, got {err:?}"
        );
    }

    /// A record that claims a longer version than its bytes provide is
    /// truncated on-disk corruption — reject it instead of reading past the
    /// buffer.
    #[test]
    fn decode_entry_rejects_truncated_version() {
        // Claims a 5-byte version, but only 2 bytes follow the length prefix.
        let raw = [5u8, 0xAA, 0xBB];
        let err = decode_entry("k", &raw).unwrap_err();
        assert!(
            matches!(err, SnapshotError::InvalidFormat(_)),
            "truncated version must be a format error, got {err:?}"
        );
    }

    /// A version length beyond `VersionToken`'s 10-byte capacity can't
    /// round-trip; `from_raw` rejects it and `decode_entry` maps that to
    /// `InvalidFormat` rather than silently truncating to a wrong
    /// (CAS-breaking) version.
    #[test]
    fn decode_entry_rejects_oversized_version() {
        // ver_len = 11 with 11 trailing bytes: passes the truncation check,
        // then trips the capacity check inside `VersionToken::from_raw`.
        let mut raw = vec![11u8];
        raw.extend_from_slice(&[0u8; 11]);
        let err = decode_entry("k", &raw).unwrap_err();
        assert!(
            matches!(err, SnapshotError::InvalidFormat(_)),
            "oversized version must be a format error, got {err:?}"
        );
    }
}
