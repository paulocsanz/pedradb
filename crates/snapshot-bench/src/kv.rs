//! KV entry/update types and the opaque version/cursor tokens.
//!
//! Ported from beyondoss/slipstream `src/kv.rs` (MIT), trimmed to the types
//! the `SnapshotStore` backends and the `snapshot_backends` bench use; the
//! async NATS reader/writer/watcher traits are not part of this harness.

use std::fmt;

/// Opaque position in a watch stream for resuming after disconnect.
///
/// Backends store whatever they need to resume. Callers should treat this as
/// opaque and only pass it back to the store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchCursor(VersionToken);

impl WatchCursor {
    /// No cursor — forces a full fold on next open.
    pub fn none() -> Self {
        Self(VersionToken::unknown())
    }

    /// Returns true if this cursor has no position.
    pub fn is_none(&self) -> bool {
        self.0.is_unknown()
    }

    /// Create a cursor from a version token.
    pub fn from_version(token: VersionToken) -> Self {
        Self(token)
    }

    /// Create a cursor from a u64 revision.
    pub fn from_u64(rev: u64) -> Self {
        Self(VersionToken::from_u64(rev))
    }

    /// Try to extract as u64 revision.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        self.0.as_u64()
    }

    /// Access the underlying version token.
    pub(crate) fn version(&self) -> &VersionToken {
        &self.0
    }
}

/// Opaque version token that abstracts store-specific versioning.
///
/// Different stores use different versioning schemes:
///
/// - NATS: 8-byte u64 revision
/// - FDB: 10-byte versionstamp
///
/// Stored inline (no heap allocation) — fits up to 10 bytes, which covers
/// every current backend.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct VersionToken {
    len: u8,
    buf: [u8; 10],
}

impl fmt::Debug for VersionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.as_bytes();
        if let Some(v) = self.as_u64() {
            write!(f, "VersionToken(u64: {v})")
        } else if bytes.is_empty() {
            write!(f, "VersionToken(unknown)")
        } else {
            write!(f, "VersionToken({bytes:?})")
        }
    }
}

impl VersionToken {
    /// Create an empty/unknown version (for entries without version info).
    pub fn unknown() -> Self {
        Self::default()
    }

    /// Check if this is an unknown/empty version.
    pub fn is_unknown(&self) -> bool {
        self.len == 0
    }

    /// Create from a u64 revision.
    pub fn from_u64(rev: u64) -> Self {
        let mut buf = [0u8; 10];
        buf[..8].copy_from_slice(&rev.to_be_bytes());
        Self { len: 8, buf }
    }

    /// Create from FDB versionstamp (10 bytes).
    #[cfg(test)]
    pub(crate) fn from_fdb_versionstamp(vs: &[u8; 10]) -> Self {
        Self { len: 10, buf: *vs }
    }

    /// Try to extract as u64.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        if self.len == 8 {
            Some(u64::from_be_bytes(self.buf[..8].try_into().unwrap_or_else(
                |_| unreachable!("len == 8 guarantees an 8-byte slice"),
            )))
        } else {
            None
        }
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }

    /// Create from raw bytes (crate-internal, e.g. store deserialization).
    ///
    /// Returns `None` if `bytes` exceeds the 10-byte inline capacity. Silently
    /// truncating instead would store a version that differs from the real
    /// revision — so an oversized token is rejected at the boundary rather
    /// than absorbed.
    #[must_use]
    pub(crate) fn from_raw(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 10 {
            return None;
        }
        let len = bytes.len() as u8;
        let mut buf = [0u8; 10];
        buf[..len as usize].copy_from_slice(bytes);
        Some(Self { len, buf })
    }
}

/// A single key-value entry with metadata.
#[derive(Debug, Clone)]
pub struct KvEntry {
    pub key: String,
    pub value: Vec<u8>,
    pub version: VersionToken,
}

/// Update event applied to a fold.
#[derive(Debug, Clone)]
pub enum KvUpdate {
    /// Key was created or updated.
    Put(KvEntry),
    /// Key was deleted.
    Delete { key: String, version: VersionToken },
    /// Key was purged (all history removed).
    ///
    /// Stores without purge semantics map this to Delete.
    Purge { key: String, version: VersionToken },
}

impl KvUpdate {
    /// Get the key affected by this update.
    pub fn key(&self) -> &str {
        match self {
            KvUpdate::Put(e) => &e.key,
            KvUpdate::Delete { key, .. } => key,
            KvUpdate::Purge { key, .. } => key,
        }
    }

    /// Get the version of this update.
    pub fn version(&self) -> &VersionToken {
        match self {
            KvUpdate::Put(e) => &e.version,
            KvUpdate::Delete { version, .. } => version,
            KvUpdate::Purge { version, .. } => version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_roundtrips_within_capacity() {
        // The largest token any backend uses is a 10-byte FDB versionstamp.
        let bytes = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let token = VersionToken::from_raw(&bytes).expect("10 bytes is within capacity");
        assert_eq!(token.as_bytes(), &bytes);

        // An 8-byte token is still interpretable as a u64 revision.
        let rev = 0x0102_0304_0506_0708u64;
        let token = VersionToken::from_raw(&rev.to_be_bytes()).expect("8 bytes is within capacity");
        assert_eq!(token.as_u64(), Some(rev));

        // Empty input is the "unknown" token.
        assert!(
            VersionToken::from_raw(&[])
                .expect("empty is within capacity")
                .is_unknown()
        );
    }

    #[test]
    fn from_raw_rejects_above_capacity() {
        // 11 bytes exceeds the 10-byte inline buffer; `from_raw` rejects it
        // instead of silently truncating into a wrong revision.
        assert!(VersionToken::from_raw(&[0u8; 11]).is_none());
    }
}
