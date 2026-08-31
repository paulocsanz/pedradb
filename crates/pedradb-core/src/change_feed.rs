//! Post-commit change feed for watch/CDC layers (RFC-0019 P0.3).
//!
//! Durable append-only `CHANGELOG` next to the DB; rebuilt/extended on open from
//! the file and any WAL ops not yet flushed into it. No ghost seq beyond the
//! durable last sequence of the live DB.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bytes::Bytes;

use crate::batch::WriteOp;
use crate::env::{Env, EnvFile};
use crate::error::{CoreError, Result};
use crate::key::{SequenceNumber, ValueType};

/// On-disk changelog file name inside the DB directory.
pub const CHANGELOG_FILE_NAME: &str = "CHANGELOG";
/// F33 quarantine of a poison `CHANGELOG` (WAL rebuild is source of truth).
pub const CHANGELOG_CORRUPT_FILE_NAME: &str = "CHANGELOG.corrupt";

const MAGIC: &[u8; 8] = b"PDBCHLG1";

fn le_u32(raw: &[u8]) -> Result<u32> {
    let bytes: [u8; 4] = raw
        .try_into()
        .map_err(|_| CoreError::Internal("changelog truncated u32".into()))?;
    Ok(u32::from_le_bytes(bytes))
}

fn le_u32_at(buf: &[u8], off: usize) -> Result<u32> {
    le_u32(
        buf.get(off..off + 4)
            .ok_or_else(|| CoreError::Internal("changelog truncated u32".into()))?,
    )
}

fn le_u64_at(buf: &[u8], off: usize) -> Result<u64> {
    let bytes: [u8; 8] = buf
        .get(off..off + 8)
        .ok_or_else(|| CoreError::Internal("changelog truncated u64".into()))?
        .try_into()
        .map_err(|_| CoreError::Internal("changelog truncated u64".into()))?;
    Ok(u64::from_le_bytes(bytes))
}

/// Kind of a logical change visible to subscribers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Put / value at `sequence`.
    Put,
    /// Point delete tombstone.
    Delete,
    /// Range delete `[key, value)` where value holds the exclusive end.
    DeleteRange,
}

impl ChangeKind {
    fn from_value_type(k: ValueType) -> Self {
        match k {
            ValueType::Value => Self::Put,
            ValueType::Deletion => Self::Delete,
            ValueType::RangeDeletion => Self::DeleteRange,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::Put => 1,
            Self::Delete => 0,
            Self::DeleteRange => 2,
        }
    }

    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Delete),
            1 => Some(Self::Put),
            2 => Some(Self::DeleteRange),
            _ => None,
        }
    }
}

/// One durable logical change (one sequence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEntry {
    /// Sequence of this change (matches WAL/MemTable assignment).
    pub sequence: SequenceNumber,
    /// User key (start key for range deletes).
    pub key: Bytes,
    /// Put / delete / range-delete.
    pub kind: ChangeKind,
    /// Value for puts; empty for point deletes; exclusive end for range deletes.
    pub value: Bytes,
}

impl ChangeEntry {
    /// Build from a WAL [`WriteOp`] (value may be a vlog pointer — feed stores as written).
    #[must_use]
    pub fn from_write_op(op: &WriteOp) -> Self {
        Self {
            sequence: op.sequence,
            key: op.key.clone(),
            kind: ChangeKind::from_value_type(op.kind),
            value: op.value.clone(),
        }
    }
}

/// In-memory + durable changelog.
#[derive(Debug, Default, Clone)]
pub struct ChangeLog {
    entries: Vec<ChangeEntry>,
}

impl ChangeLog {
    /// Empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace contents with `entries` sorted by sequence (SST last-per-key rebuild).
    pub fn replace_sorted(&mut self, mut entries: Vec<ChangeEntry>) {
        entries.sort_by_key(|e| e.sequence);
        self.entries = entries;
    }

    /// Number of recorded changes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Highest sequence in the log, if any.
    #[must_use]
    pub fn max_sequence(&self) -> Option<SequenceNumber> {
        self.entries.last().map(|e| e.sequence)
    }

    /// Append entries (must be non-decreasing by sequence).
    pub fn extend(&mut self, new: impl IntoIterator<Item = ChangeEntry>) {
        for e in new {
            if let Some(max) = self.max_sequence() {
                debug_assert!(e.sequence > max);
            }
            self.entries.push(e);
        }
    }

    /// Changes with `from_seq < sequence <= to_seq` (exclusive lower, inclusive upper).
    #[must_use]
    pub fn changes_in(&self, from_seq: SequenceNumber, to_seq: SequenceNumber) -> Vec<ChangeEntry> {
        self.entries
            .iter()
            .filter(|e| e.sequence > from_seq && e.sequence <= to_seq)
            .cloned()
            .collect()
    }

    /// All changes with `sequence > from_seq` (tail).
    #[must_use]
    pub fn changes_after(&self, from_seq: SequenceNumber) -> Vec<ChangeEntry> {
        self.entries
            .iter()
            .filter(|e| e.sequence > from_seq)
            .cloned()
            .collect()
    }

    /// Load from `CHANGELOG` if present.
    ///
    /// On-disk feed is a **cache** (WAL rebuild fills gaps). Corrupt / truncated
    /// files must not brick DB open (F33): treat as empty and let open rebuild
    /// from WAL when possible.
    ///
    /// # Errors
    /// I/O reading the file (not decode errors).
    pub fn load_on(env: &impl Env, dir: &Path) -> Result<Self> {
        let path = dir.join(CHANGELOG_FILE_NAME);
        if !env.exists(&path) {
            return Ok(Self::new());
        }
        let mut f = env.open_read(&path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        match decode_changelog(&buf) {
            Ok(log) => Ok(log),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "CHANGELOG corrupt or unreadable; treating as empty cache (F33)"
                );
                // Best-effort quarantine so a later rewrite does not keep re-reading poison.
                let bad = dir.join(CHANGELOG_CORRUPT_FILE_NAME);
                let _ = env.rename(&path, &bad);
                Ok(Self::new())
            }
        }
    }

    /// Persist full log to `CHANGELOG` (rewrite) and fsync.
    ///
    /// Uses atomic rename over the destination (POSIX replaces in place). Does
    /// **not** `remove_file` the live `CHANGELOG` first — that window permanently
    /// loses the feed after WAL truncate/flush (F31).
    ///
    /// # Errors
    /// I/O.
    pub fn store_on(&self, env: &impl Env, dir: &Path) -> Result<()> {
        let path = dir.join(CHANGELOG_FILE_NAME);
        let tmp = dir.join(format!("{CHANGELOG_FILE_NAME}.tmp"));
        let body = encode_changelog(self)?;
        {
            let mut f = env.create(&tmp)?;
            f.write_all(&body)?;
            // Cache (RFC-0019): same barrier class as WAL, not Apple F_FULLFSYNC.
            f.sync_data()?;
        }
        // Atomic replace: rename overwrites existing path on the same filesystem.
        env.rename(&tmp, &path)?;
        let _ = env.sync_dir(dir);
        Ok(())
    }

    /// Path helper.
    #[must_use]
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(CHANGELOG_FILE_NAME)
    }
}

fn encode_changelog(log: &ChangeLog) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    let n = u32::try_from(log.entries.len())
        .map_err(|_| CoreError::Internal("changelog too large".into()))?;
    buf.extend_from_slice(&n.to_le_bytes());
    for e in &log.entries {
        buf.extend_from_slice(&e.sequence.to_le_bytes());
        let klen = u32::try_from(e.key.len())
            .map_err(|_| CoreError::Internal("changelog key too large".into()))?;
        buf.extend_from_slice(&klen.to_le_bytes());
        buf.extend_from_slice(&e.key);
        buf.push(e.kind.to_u8());
        let vlen = u32::try_from(e.value.len())
            .map_err(|_| CoreError::Internal("changelog value too large".into()))?;
        buf.extend_from_slice(&vlen.to_le_bytes());
        buf.extend_from_slice(&e.value);
    }
    let crc = crc32c::crc32c(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());
    Ok(buf)
}

/// Decode a CHANGELOG payload (for open + codec fuzz smoke, RFC-0020 P0.5).
/// RFC-0083 P1.2 / RFC-0085 P0: trailer CRC is `crc_match_ok`.
///
/// # Errors
/// Truncation, bad magic, CRC mismatch, or corrupt entries.
pub fn decode_changelog(buf: &[u8]) -> Result<ChangeLog> {
    // F32: never `with_capacity(n)` from an untrusted header alone (F2-class OOM).
    // Minimum entry wire size: seq(8)+klen(4)+kind(1)+vlen(4) = 17 (empty key/value).
    const MIN_ENTRY_BYTES: usize = 17;
    if buf.len() < 8 + 4 + 4 {
        return Err(CoreError::Internal("changelog too short".into()));
    }
    let (payload, crc_bytes) = buf.split_at(buf.len() - 4);
    let stored = le_u32(crc_bytes)?;
    let got = crc32c::crc32c(payload);
    if !crate::wal::crc::crc_match_ok(stored, got) {
        return Err(CoreError::Internal(format!(
            "changelog CRC mismatch: {stored:#x} vs {got:#x}"
        )));
    }
    if &payload[0..8] != MAGIC {
        return Err(CoreError::Internal("bad changelog magic".into()));
    }
    let n = le_u32_at(payload, 8)? as usize;
    let max_possible = payload.len().saturating_sub(12) / MIN_ENTRY_BYTES;
    if n > max_possible {
        return Err(CoreError::Internal(format!(
            "changelog entry count {n} exceeds file residual (max {max_possible})"
        )));
    }
    let mut off = 12;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        if off + 8 + 4 > payload.len() {
            return Err(CoreError::Internal("changelog truncated entry".into()));
        }
        let sequence = le_u64_at(payload, off)?;
        off += 8;
        let klen = le_u32_at(payload, off)? as usize;
        off += 4;
        if off + klen + 1 + 4 > payload.len() {
            return Err(CoreError::Internal("changelog truncated key".into()));
        }
        let key = Bytes::copy_from_slice(&payload[off..off + klen]);
        off += klen;
        let kind = ChangeKind::from_u8(payload[off])
            .ok_or_else(|| CoreError::Internal("bad changelog kind".into()))?;
        off += 1;
        let vlen = le_u32_at(payload, off)? as usize;
        off += 4;
        if off + vlen > payload.len() {
            return Err(CoreError::Internal("changelog truncated value".into()));
        }
        let value = Bytes::copy_from_slice(&payload[off..off + vlen]);
        off += vlen;
        entries.push(ChangeEntry {
            sequence,
            key,
            kind,
            value,
        });
    }
    if off != payload.len() {
        return Err(CoreError::Internal("changelog trailing garbage".into()));
    }
    Ok(ChangeLog { entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;
    use std::fs;

    #[test]
    fn encode_decode_round_trip() {
        let mut log = ChangeLog::new();
        log.extend([
            ChangeEntry {
                sequence: 1,
                key: Bytes::from_static(b"a"),
                kind: ChangeKind::Put,
                value: Bytes::from_static(b"1"),
            },
            ChangeEntry {
                sequence: 2,
                key: Bytes::from_static(b"a"),
                kind: ChangeKind::Delete,
                value: Bytes::new(),
            },
        ]);
        let raw = encode_changelog(&log).unwrap();
        let got = decode_changelog(&raw).unwrap();
        assert_eq!(got.entries, log.entries);
        assert_eq!(got.changes_in(0, 1).len(), 1);
        assert_eq!(got.changes_after(1).len(), 1);
    }

    #[test]
    fn store_load_file() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-changelog-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log = ChangeLog::new();
        log.extend([ChangeEntry {
            sequence: 3,
            key: Bytes::from_static(b"k"),
            kind: ChangeKind::Put,
            value: Bytes::from_static(b"v"),
        }]);
        log.store_on(&env, &dir).unwrap();
        let loaded = ChangeLog::load_on(&env, &dir).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].sequence, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    /// F53: missing CHANGELOG after flush rebuilds last-per-key from SST (not empty).
    #[test]
    fn changelog_missing_post_flush_rebuilds_feed_from_sst() {
        use crate::db::{Db, OpenOptions};
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedradb-chlog-loss-{n}"));
        let _ = fs::remove_dir_all(&dir);
        let opts = OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        };
        {
            let mut db = Db::open_with(&dir, opts).unwrap();
            for i in 0..5u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
            }
            assert_eq!(db.changes_after(0).len(), 5);
            db.flush().unwrap();
            drop(db);
        }
        let path = dir.join(CHANGELOG_FILE_NAME);
        assert!(path.exists());
        fs::remove_file(&path).unwrap();
        let db = Db::open_with(&dir, opts).unwrap();
        let feed = db.changes_after(0);
        assert!(
            db.get(&[b'k', 0]).is_some(),
            "data plane must still see SST keys"
        );
        assert_eq!(
            feed.len(),
            5,
            "SST last-per-key rebuild must restore feed, got {feed:?}"
        );
        for i in 0..5u8 {
            assert!(
                feed.iter().any(|e| e.key.as_ref() == [b'k', i]),
                "missing k{i} in rebuilt feed"
            );
        }
        assert!(db.last_sequence() >= 5);
        let _ = fs::remove_dir_all(&dir);
    }

    /// F32: huge entry count must fail-stop, not allocate multi-GiB.
    #[test]
    fn decode_rejects_huge_entry_count() {
        let mut payload = Vec::new();
        payload.extend_from_slice(MAGIC);
        payload.extend_from_slice(&0x0FFF_FFFFu32.to_le_bytes()); // huge n, no entries
        let crc = crc32c::crc32c(&payload);
        payload.extend_from_slice(&crc.to_le_bytes());
        let err = decode_changelog(&payload).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("exceeds") || msg.contains("count"),
            "expected count bound error, got {msg}"
        );
    }

    /// RFC-0085 P0 / RFC-0083 P1.2: production `put`+`close` writes CHANGELOG;
    /// XOR only the trailer CRC (payload intact). `decode_changelog` is crc
    /// mismatch. `Db::open` still succeeds (F33). AS-IS would decode Ok.
    #[test]
    fn crc_mismatch_on_live_changelog_is_not_ok() {
        use crate::db::Db;
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any changelog crc would match"
        );
        let dir = std::env::temp_dir().join(format!(
            "pedradb-chlog-crc-0085-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.close().unwrap();
        }
        let path = dir.join(CHANGELOG_FILE_NAME);
        let mut bytes = fs::read(&path).unwrap();
        assert!(bytes.len() >= 12, "CHANGELOG must have payload + trailer");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let err = decode_changelog(&bytes).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on crc_match_ok, not a payload parse; got {msg}"
        );
        let db = Db::open(&dir).expect("F33: trailer lie must not brick open");
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0085 P2.1: changelog `crc_match_ok` is not a CRC32C collision theorem.
    #[test]
    fn changelog_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
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

    /// RFC-0085 P2.2: F33 quarantine stays fail-open. Trailer lie is renamed
    /// to CHANGELOG.corrupt; `Db::open` is Ok; WAL rebuild serves k.
    #[test]
    fn changelog_crc_mismatch_open_still_quarantines_f33() {
        use crate::db::Db;
        let dir = std::env::temp_dir().join(format!(
            "pedradb-chlog-f33-0085-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        {
            let mut db = Db::open(&dir).unwrap();
            db.put(b"k", b"v").unwrap();
            db.close().unwrap();
        }
        let path = dir.join(CHANGELOG_FILE_NAME);
        let mut bytes = fs::read(&path).unwrap();
        assert!(bytes.len() >= 12, "CHANGELOG must have payload + trailer");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let db = Db::open(&dir).expect("F33: trailer lie must not brick open");
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        let quarantined = dir.join(CHANGELOG_CORRUPT_FILE_NAME);
        assert!(
            quarantined.exists(),
            "F33 must rename poison CHANGELOG to CHANGELOG.corrupt"
        );
        let q = fs::read(&quarantined).unwrap();
        assert_eq!(q, bytes, "quarantine must keep the trailer lie");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// Corrupt CHANGELOG should not brick open of durable SST data (feed is a cache).
    #[test]
    fn corrupt_changelog_does_not_block_open() {
        use crate::db::{Db, OpenOptions};
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedradb-chlog-corrupt-{n}"));
        let _ = fs::remove_dir_all(&dir);
        let opts = OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        };
        {
            let mut db = Db::open_with(&dir, opts).unwrap();
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            drop(db);
        }
        let path = dir.join(CHANGELOG_FILE_NAME);
        let mut bytes = fs::read(&path).unwrap();
        // Flip a payload byte (not just CRC) so CRC mismatches.
        if bytes.len() > 20 {
            bytes[12] ^= 0xff;
        }
        fs::write(&path, &bytes).unwrap();
        match Db::open_with(&dir, opts) {
            Ok(db) => {
                assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
                // feed may be empty after WAL rotate — data plane must work
                let _ = db.changes_after(0);
                let _ = fs::remove_dir_all(&dir);
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&dir);
                panic!("BUG: corrupt CHANGELOG blocks open of SST data: {e}");
            }
        }
    }
}
