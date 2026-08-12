//! Append-only value log for large values (RFC-0014 P2.2 / RFC-0016 P0.1).
//!
//! # Layout (`VALUES.vlog`)
//! ```text
//! magic "PDBVLOG1" (8)
//! // records:
//! //   len u32 LE | crc32c(data) u32 LE | data[len]
//! ```
//!
//! SST/mem store a compact [`VLOG_VALUE_PREFIX`] pointer instead of the payload.
//! [`rewrite_live`] builds a new log with only live records (GC).

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use bytes::Bytes;

use crate::env::{Env, EnvFile};
use crate::error::{CoreError, Result};

/// Primary value-log file name inside the DB directory.
pub const VLOG_FILE_NAME: &str = "VALUES.vlog";
/// Side file written during GC; preferred on open only after adopt marker (crash recovery).
pub const VLOG_NEW_NAME: &str = "VALUES.vlog.new";
/// Marker written after MANIFEST points at remapped SSTs; open may then prefer `.new`.
pub const VLOG_ADOPT_NAME: &str = "VALUES.vlog.adopt";

const MAGIC: &[u8; 8] = b"PDBVLOG1";

/// Inline value marker: `VLG1` + offset `u64` + len `u32` + data CRC `u32`.
pub const VLOG_VALUE_PREFIX: &[u8; 4] = b"VLG1";

/// Encode a pointer to a vlog record as a mem/SST value.
#[must_use]
pub fn encode_vlog_ref(offset: u64, len: u32, data_crc: u32) -> Bytes {
    let mut v = Vec::with_capacity(4 + 8 + 4 + 4);
    v.extend_from_slice(VLOG_VALUE_PREFIX);
    v.extend_from_slice(&offset.to_le_bytes());
    v.extend_from_slice(&len.to_le_bytes());
    v.extend_from_slice(&data_crc.to_le_bytes());
    Bytes::from(v)
}

/// Decode a vlog pointer; `None` if the value is not a vlog ref.
#[must_use]
pub fn decode_vlog_ref(value: &[u8]) -> Option<(u64, u32, u32)> {
    if value.len() != 4 + 8 + 4 + 4 || &value[0..4] != VLOG_VALUE_PREFIX {
        return None;
    }
    let offset = u64::from_le_bytes(value[4..12].try_into().ok()?);
    let len = u32::from_le_bytes(value[12..16].try_into().ok()?);
    let crc = u32::from_le_bytes(value[16..20].try_into().ok()?);
    Some((offset, len, crc))
}

/// Result of rewriting the value log with only live records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VlogRewriteStats {
    /// Bytes in the old log file before rewrite.
    pub bytes_before: u64,
    /// Bytes in the new log file after rewrite.
    pub bytes_after: u64,
    /// Number of live records copied.
    pub live_records: u64,
}

/// Open or create the value log and append/read records.
#[derive(Debug)]
pub struct ValueLog<F: EnvFile> {
    path: PathBuf,
    file: F,
    /// Next append offset (file length).
    next_offset: u64,
}

impl<F: EnvFile> ValueLog<F> {
    /// Path of the vlog file currently open.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Current file length / next append offset.
    #[must_use]
    pub fn len_bytes(&self) -> u64 {
        self.next_offset
    }

    /// Resolve which path to open given MANIFEST `vlog_use_new` (crash-safe GC).
    ///
    /// When `use_new` is true (MANIFEST records remapped SST offsets), open
    /// `VALUES.vlog.new` if present; if missing after promote rename, fall back to
    /// primary. When `use_new` is false, always use the primary (orphan `.new` is
    /// ignored so mid-GC before MANIFEST cannot mis-open).
    #[must_use]
    pub fn resolve_path<E: Env>(env: &E, dir: &Path, use_new: bool) -> PathBuf {
        let newp = dir.join(VLOG_NEW_NAME);
        let main = dir.join(VLOG_FILE_NAME);
        if use_new {
            if env.exists(&newp) {
                newp
            } else {
                // Promote finished rename but MANIFEST not yet cleared, or only main.
                main
            }
        } else {
            main
        }
    }

    /// Create or open appendable vlog via `env` (primary file; `use_new = false`).
    ///
    /// # Errors
    /// I/O.
    pub fn open_on<E: Env<File = F>>(env: &E, dir: &Path) -> Result<Self> {
        Self::open_with_flag(env, dir, false)
    }

    /// Open vlog using MANIFEST `vlog_use_new` flag.
    ///
    /// # Errors
    /// I/O.
    pub fn open_with_flag<E: Env<File = F>>(env: &E, dir: &Path, use_new: bool) -> Result<Self> {
        let path = Self::resolve_path(env, dir, use_new);
        if !env.exists(&path) {
            // Create primary only when not expecting a staged `.new`.
            let main = dir.join(VLOG_FILE_NAME);
            if use_new && env.exists(&dir.join(VLOG_NEW_NAME)) {
                return Self::open_path(env, dir.join(VLOG_NEW_NAME));
            }
            let mut f = env.create(&main)?;
            Write::write_all(&mut f, MAGIC)?;
            f.sync_all()?;
            drop(f);
            return Self::open_path(env, main);
        }
        Self::open_path(env, path)
    }

    fn open_path<E: Env<File = F>>(env: &E, path: PathBuf) -> Result<Self> {
        let mut file = env.open_append(&path)?;
        let len = file.len()?;
        if len < MAGIC.len() as u64 {
            return Err(CoreError::Internal(format!(
                "vlog too short: {}",
                path.display()
            )));
        }
        {
            let mut r = env.open_read(&path)?;
            let mut mag = [0u8; 8];
            r.read_exact(&mut mag)?;
            if &mag != MAGIC {
                return Err(CoreError::Internal(format!(
                    "bad vlog magic in {}",
                    path.display()
                )));
            }
        }
        Ok(Self {
            path,
            file,
            next_offset: len,
        })
    }

    /// Append `data`, fsync, return `(offset, len, crc)`.
    ///
    /// # Errors
    /// I/O.
    pub fn append(&mut self, data: &[u8]) -> Result<(u64, u32, u32)> {
        let len = u32::try_from(data.len()).map_err(|_| {
            CoreError::Internal("vlog value too large".into())
        })?;
        let crc = crc32c::crc32c(data);
        let offset = self.next_offset;
        Write::write_all(&mut self.file, &len.to_le_bytes())?;
        Write::write_all(&mut self.file, &crc.to_le_bytes())?;
        Write::write_all(&mut self.file, data)?;
        self.file.sync_all()?;
        self.next_offset = offset
            .checked_add(8)
            .and_then(|o| o.checked_add(u64::from(len)))
            .ok_or_else(|| CoreError::Internal("vlog offset overflow".into()))?;
        Ok((offset, len, crc))
    }

    /// Read a record at `offset` via a separate read handle.
    ///
    /// # Errors
    /// I/O or CRC mismatch.
    pub fn read_at_on<E: Env>(
        &self,
        env: &E,
        offset: u64,
        len: u32,
        expect_crc: u32,
    ) -> Result<Bytes> {
        read_record_at(env, &self.path, offset, len, expect_crc)
    }

    /// Rewrite `live` payloads into `VALUES.vlog.new`; returns stats + old→new ref map.
    ///
    /// `live` maps **old offset** → payload bytes (must match stored CRC when read).
    ///
    /// # Errors
    /// I/O.
    pub fn rewrite_live_to_new<E: Env<File = F>>(
        env: &E,
        dir: &Path,
        live: &[(u64, Bytes)],
    ) -> Result<(VlogRewriteStats, std::collections::HashMap<u64, Bytes>)> {
        let main = dir.join(VLOG_FILE_NAME);
        let bytes_before = if env.exists(&main) {
            env.metadata_len(&main).unwrap_or(0)
        } else {
            0
        };
        let newp = dir.join(VLOG_NEW_NAME);
        if env.exists(&newp) {
            env.remove_file(&newp)?;
        }
        let mut body = Vec::new();
        body.extend_from_slice(MAGIC);
        let mut remap = std::collections::HashMap::new();
        let mut next = MAGIC.len() as u64;
        for (old_off, data) in live {
            let len = u32::try_from(data.len()).map_err(|_| {
                CoreError::Internal("vlog value too large".into())
            })?;
            let crc = crc32c::crc32c(data);
            let new_off = next;
            body.extend_from_slice(&len.to_le_bytes());
            body.extend_from_slice(&crc.to_le_bytes());
            body.extend_from_slice(data);
            next = next
                .checked_add(8 + u64::from(len))
                .ok_or_else(|| CoreError::Internal("vlog rewrite overflow".into()))?;
            remap.insert(*old_off, encode_vlog_ref(new_off, len, crc));
        }
        {
            let mut f = env.create(&newp)?;
            Write::write_all(&mut f, &body)?;
            f.sync_all()?;
        }
        if let Ok(()) = env.sync_dir(dir) {
            // best-effort dir sync
        }
        let stats = VlogRewriteStats {
            bytes_before,
            bytes_after: body.len() as u64,
            live_records: live.len() as u64,
        };
        Ok((stats, remap))
    }

    /// After MANIFEST records remapped SSTs (`vlog_use_new`): promote `.new` → primary.
    ///
    /// # Errors
    /// I/O.
    pub fn promote_new_and_reopen<E: Env<File = F>>(env: &E, dir: &Path) -> Result<Self> {
        let newp = dir.join(VLOG_NEW_NAME);
        let main = dir.join(VLOG_FILE_NAME);
        if !env.exists(&newp) {
            // Already promoted or never staged.
            return Self::open_on(env, dir);
        }
        if env.exists(&main) {
            env.remove_file(&main)?;
        }
        env.rename(&newp, &main)?;
        let _ = env.sync_dir(dir);
        // Best-effort clear legacy adopt marker from older builds.
        let adopt = dir.join(VLOG_ADOPT_NAME);
        if env.exists(&adopt) {
            let _ = env.remove_file(&adopt);
        }
        Self::open_path(env, main)
    }
}

fn read_record_at<E: Env>(
    env: &E,
    path: &Path,
    offset: u64,
    len: u32,
    expect_crc: u32,
) -> Result<Bytes> {
    let mut f = env.open_read(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut hdr = [0u8; 8];
    f.read_exact(&mut hdr)?;
    let stored_len = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let stored_crc = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    if stored_len != len {
        return Err(CoreError::Internal(format!(
            "vlog len mismatch at {offset}: stored {stored_len} expect {len}"
        )));
    }
    if stored_crc != expect_crc {
        return Err(CoreError::Internal(format!(
            "vlog crc mismatch at {offset}"
        )));
    }
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?;
    let got = crc32c::crc32c(&buf);
    if got != expect_crc {
        return Err(CoreError::Internal(format!(
            "vlog data crc mismatch at {offset}"
        )));
    }
    Ok(Bytes::from(buf))
}

/// Resolve a stored value: either inline or vlog pointer.
///
/// # Errors
/// Vlog I/O / CRC when the value is a ref.
pub fn resolve_value_on<E: Env, F: EnvFile>(
    env: &E,
    vlog: Option<&ValueLog<F>>,
    stored: Bytes,
) -> Result<Bytes> {
    if let Some((off, len, crc)) = decode_vlog_ref(stored.as_ref()) {
        let log = vlog.ok_or_else(|| {
            CoreError::Internal("vlog ref present but value log not open".into())
        })?;
        log.read_at_on(env, off, len, crc)
    } else {
        Ok(stored)
    }
}

/// Remap a stored value if it is a `VLG1` pointer present in `remap` (old offset → new ref).
#[must_use]
pub fn remap_stored_value<S: std::hash::BuildHasher>(
    stored: &Bytes,
    remap: &std::collections::HashMap<u64, Bytes, S>,
) -> Bytes {
    if let Some((off, _, _)) = decode_vlog_ref(stored.as_ref()) {
        if let Some(new_ref) = remap.get(&off) {
            return new_ref.clone();
        }
    }
    stored.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;
    use std::fs;

    #[test]
    fn append_read_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log = ValueLog::open_on(&env, &dir).unwrap();
        let data = vec![7u8; 10_000];
        let (off, len, crc) = log.append(&data).unwrap();
        let got = log.read_at_on(&env, off, len, crc).unwrap();
        assert_eq!(got.as_ref(), data.as_slice());
        let ptr = encode_vlog_ref(off, len, crc);
        assert!(decode_vlog_ref(ptr.as_ref()).is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewrite_live_shrinks_file() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-gc-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log = ValueLog::open_on(&env, &dir).unwrap();
        let (o1, _, _) = log.append(&vec![1u8; 1000]).unwrap();
        let (_o2, _, _) = log.append(&vec![2u8; 1000]).unwrap();
        let before = log.len_bytes();
        drop(log);
        // Keep only first record live.
        let live = vec![(o1, Bytes::from(vec![1u8; 1000]))];
        let (stats, remap) = ValueLog::<std::fs::File>::rewrite_live_to_new(&env, &dir, &live)
            .unwrap();
        assert!(stats.bytes_after < before);
        assert_eq!(stats.live_records, 1);
        assert!(remap.contains_key(&o1));
        // Without MANIFEST flag, open still uses primary (crash-safe: SST not remapped).
        let log = ValueLog::open_with_flag(&env, &dir, false).unwrap();
        assert!(log.path().ends_with(VLOG_FILE_NAME));
        // MANIFEST would set use_new after SST remap; open staged file.
        let log = ValueLog::open_with_flag(&env, &dir, true).unwrap();
        assert!(log.path().ends_with(VLOG_NEW_NAME));
        let log = ValueLog::promote_new_and_reopen(&env, &dir).unwrap();
        assert!(log.path().ends_with(VLOG_FILE_NAME));
        assert!(!env.exists(&dir.join(VLOG_NEW_NAME)));
        let _ = fs::remove_dir_all(&dir);
    }
}
