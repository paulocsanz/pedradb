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
//!
//! RFC-0029: optional numbered blob files (`000001.blob`) with [`VLOG_BLOB_PREFIX`]
//! pointers (`file_num`, offset). File 0 remains `VALUES.vlog` / `VLG1`.

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
/// Blob pointer: `VLG3` + `file_num` `u32` + offset `u64` + len `u32` + crc `u32`.
pub const VLOG_BLOB_PREFIX: &[u8; 4] = b"VLG3";
/// Sealed / active blob file suffix (`000001.blob`).
pub const BLOB_SUFFIX: &str = ".blob";

/// Decoded value-log pointer (file 0 = [`VLOG_FILE_NAME`] / `VLG1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VlogPtr {
    /// Blob generation (`0` = legacy `VALUES.vlog`).
    pub file_num: u32,
    /// Byte offset of the record header in that file.
    pub offset: u64,
    /// Payload length.
    pub len: u32,
    /// CRC32C of the payload.
    pub crc: u32,
}

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

/// Decode `VLG1` or `VLG3`.
#[must_use]
pub fn decode_vlog_ptr(value: &[u8]) -> Option<VlogPtr> {
    if value.len() == 4 + 8 + 4 + 4 && value.starts_with(VLOG_VALUE_PREFIX) {
        let (offset, len, crc) = decode_vlog_ref(value)?;
        return Some(VlogPtr {
            file_num: 0,
            offset,
            len,
            crc,
        });
    }
    if value.len() == 4 + 4 + 8 + 4 + 4 && value.starts_with(VLOG_BLOB_PREFIX) {
        let file_num = u32::from_le_bytes(value[4..8].try_into().ok()?);
        let offset = u64::from_le_bytes(value[8..16].try_into().ok()?);
        let len = u32::from_le_bytes(value[16..20].try_into().ok()?);
        let crc = u32::from_le_bytes(value[20..24].try_into().ok()?);
        return Some(VlogPtr {
            file_num,
            offset,
            len,
            crc,
        });
    }
    None
}

/// Encode a pointer (`VLG1` when `file_num == 0`, else `VLG3`).
#[must_use]
pub fn encode_vlog_ptr(ptr: VlogPtr) -> Bytes {
    if ptr.file_num == 0 {
        return encode_vlog_ref(ptr.offset, ptr.len, ptr.crc);
    }
    let mut v = Vec::with_capacity(24);
    v.extend_from_slice(VLOG_BLOB_PREFIX);
    v.extend_from_slice(&ptr.file_num.to_le_bytes());
    v.extend_from_slice(&ptr.offset.to_le_bytes());
    v.extend_from_slice(&ptr.len.to_le_bytes());
    v.extend_from_slice(&ptr.crc.to_le_bytes());
    Bytes::from(v)
}

/// Path of blob generation `num` (`000001.blob`).
#[must_use]
pub fn blob_path(dir: &Path, num: u32) -> PathBuf {
    dir.join(format!("{num:06}{BLOB_SUFFIX}"))
}

/// Parse `000001.blob` → `1`.
#[must_use]
pub fn parse_blob_name(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(BLOB_SUFFIX)?;
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// Discover sealed/active blob generations under `dir` (sorted).
#[must_use]
pub fn list_blob_nums<E: Env>(env: &E, dir: &Path) -> Vec<u32> {
    let Ok(names) = env.read_dir_names(dir) else {
        return Vec::new();
    };
    let mut nums: Vec<u32> = names.iter().filter_map(|n| parse_blob_name(n)).collect();
    nums.sort_unstable();
    nums.dedup();
    nums
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

/// Userspace buffer before `write()`, same size as Rocks
/// `writable_file_max_buffer_size`. A process crash can lose the tail
/// (&lt; 64 KiB) when the caller does not `sync_pending`; the Db commit
/// paths flush per commit, so acked values do not sit here.
pub const ASYNC_VLOG_BUFFER: usize = 64 * 1024;

/// Payload size that skips the contiguous pending memcpy (RFC-0149 P2.1
/// `kvrocks_blob_set` is 16 KiB). Held as interned `Bytes` until the
/// next flush concatenates once.
const LARGE_PENDING: usize = 4096;

/// Same chunk as the WAL: delayed allocation must not hit the Ok path.
const VLOG_PREALLOC_CHUNK: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
struct PendingLarge {
    offset: u64,
    hdr: [u8; 8],
    data: Bytes,
}

/// Open or create the value log and append/read records.
#[derive(Debug)]
pub struct ValueLog<F: EnvFile> {
    path: PathBuf,
    file: F,
    /// Next append offset (logical: on-disk + pending).
    next_offset: u64,
    /// File offset of `pending[0]`. `pending_start + pending.len() == next_offset`.
    pending_start: u64,
    /// Unwritten tail of small records (headers + payloads).
    pending: Vec<u8>,
    /// Large records held by `Bytes` (no payload memcpy) until flush.
    pending_large: Vec<PendingLarge>,
    /// Sum of `8 + data.len()` over [`Self::pending_large`].
    pending_large_bytes: usize,
    /// Logical offset covered by space reservation.
    prealloc_to: u64,
    /// Bytes have been `write()`n since the last [`Self::sync_pending`].
    /// G1 must not `fdatasync` an empty vlog on every small put (RFC-0062
    /// P1.1: the parity bench enables blob, ycsb_a stays under the
    /// threshold, and a second barrier per Ok was the leftover Linux tax).
    needs_sync: bool,
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

    /// Open numbered blob `{num:06}.blob` for append (creates if missing).
    ///
    /// # Errors
    /// I/O.
    pub fn open_blob<E: Env<File = F>>(env: &E, dir: &Path, num: u32) -> Result<Self> {
        if num == 0 {
            return Self::open_on(env, dir);
        }
        let path = blob_path(dir, num);
        if !env.exists(&path) {
            let mut f = env.create(&path)?;
            Write::write_all(&mut f, MAGIC)?;
            f.sync_all()?;
            drop(f);
            // F4: the dirent must be durable before WAL pointers to this
            // generation can be acked — a swallowed failure here can lose the
            // *filename* while the payload was synced.
            env.sync_dir(dir)?;
        }
        Self::open_path(env, path)
    }

    /// Open vlog using MANIFEST `vlog_use_new` flag.
    ///
    /// # Errors
    /// I/O.
    pub fn open_with_flag<E: Env<File = F>>(env: &E, dir: &Path, use_new: bool) -> Result<Self> {
        let path = Self::resolve_path(env, dir, use_new);
        if !env.exists(&path) {
            let main = dir.join(VLOG_FILE_NAME);
            let newp = dir.join(VLOG_NEW_NAME);
            if use_new && env.exists(&newp) {
                return Self::open_path(env, newp);
            }
            // F51: MANIFEST `vlog_use_new` means SST pointers target the staged
            // layout. Inventing an empty primary would make large values vanish.
            if use_new && !env.exists(&main) && !env.exists(&newp) {
                return Err(CoreError::Internal(
                    "vlog missing under vlog_use_new (mid-promote?). refuse empty create".into(),
                ));
            }
            // Fresh DB / first large put: create empty primary.
            let mut f = env.create(&main)?;
            Write::write_all(&mut f, MAGIC)?;
            f.sync_all()?;
            drop(f);
            // F4: same dirent-durability rule as blob creation above.
            env.sync_dir(dir)?;
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
            pending_start: len,
            pending: Vec::new(),
            pending_large: Vec::new(),
            pending_large_bytes: 0,
            prealloc_to: len,
            needs_sync: false,
        })
    }

    /// Append `data`, fsync, return `(offset, len, crc)`.
    ///
    /// Durable on return (tests / operator tools). The put path uses
    /// [`Self::append_pending`] so G1 can fsync once per commit and async
    /// can `write()` without `fdatasync` (RFC-0044 same-class).
    ///
    /// # Errors
    /// I/O.
    pub fn append(&mut self, data: &[u8]) -> Result<(u64, u32, u32)> {
        let rec = self.append_pending(data)?;
        self.sync_pending()?;
        Ok(rec)
    }

    /// Stage `data` in the 64 KiB buffer. `write()`s when the buffer fills.
    /// Does **not** fsync. Get can still read the record from `pending`.
    ///
    /// # Errors
    /// I/O on a buffer flush.
    pub fn append_pending(&mut self, data: &[u8]) -> Result<(u64, u32, u32)> {
        if data.len() >= LARGE_PENDING {
            self.append_pending_bytes(Bytes::copy_from_slice(data))
        } else {
            self.append_pending_small(data)
        }
    }

    /// [`Self::append_pending`] taking an already-owned payload so a 16 KiB
    /// blob is not memcpy'd into the userspace tail (RFC-0149 P2.1).
    pub(crate) fn append_pending_bytes(&mut self, data: Bytes) -> Result<(u64, u32, u32)> {
        if data.len() < LARGE_PENDING {
            return self.append_pending_small(data.as_ref());
        }
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::BEFORE_VLOG_APPEND)?;
        let len = u32::try_from(data.len())
            .map_err(|_| CoreError::Internal("vlog value too large".into()))?;
        let crc = crc32c::crc32c(data.as_ref());
        let rec_len = 8usize
            .checked_add(data.len())
            .ok_or_else(|| CoreError::Internal("vlog record overflow".into()))?;
        if self.staged_len() >= ASYNC_VLOG_BUFFER
            || self.staged_len().saturating_add(rec_len) > ASYNC_VLOG_BUFFER
                && self.staged_len() > 0
        {
            self.flush_pending()?;
        }
        let offset = self.next_offset;
        let mut hdr = [0u8; 8];
        hdr[..4].copy_from_slice(&len.to_le_bytes());
        hdr[4..].copy_from_slice(&crc.to_le_bytes());
        self.pending_large.push(PendingLarge { offset, hdr, data });
        self.pending_large_bytes = self.pending_large_bytes.saturating_add(rec_len);
        self.next_offset = offset
            .checked_add(rec_len as u64)
            .ok_or_else(|| CoreError::Internal("vlog offset overflow".into()))?;
        if self.staged_len() >= ASYNC_VLOG_BUFFER {
            self.flush_pending()?;
        }
        Ok((offset, len, crc))
    }

    fn append_pending_small(&mut self, data: &[u8]) -> Result<(u64, u32, u32)> {
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::BEFORE_VLOG_APPEND)?;
        let len = u32::try_from(data.len())
            .map_err(|_| CoreError::Internal("vlog value too large".into()))?;
        let crc = crc32c::crc32c(data);
        let rec_len = 8usize
            .checked_add(data.len())
            .ok_or_else(|| CoreError::Internal("vlog record overflow".into()))?;
        if self.staged_len() >= ASYNC_VLOG_BUFFER
            || self.staged_len().saturating_add(rec_len) > ASYNC_VLOG_BUFFER
                && self.staged_len() > 0
        {
            self.flush_pending()?;
        }
        let offset = self.next_offset;
        self.pending.extend_from_slice(&len.to_le_bytes());
        self.pending.extend_from_slice(&crc.to_le_bytes());
        self.pending.extend_from_slice(data);
        self.next_offset = offset
            .checked_add(rec_len as u64)
            .ok_or_else(|| CoreError::Internal("vlog offset overflow".into()))?;
        if self.staged_len() >= ASYNC_VLOG_BUFFER {
            self.flush_pending()?;
        }
        Ok((offset, len, crc))
    }

    fn staged_len(&self) -> usize {
        self.pending.len().saturating_add(self.pending_large_bytes)
    }

    fn reserve_space(&mut self, upcoming: u64) {
        let pos = self.pending_start;
        self.prealloc_to = self.prealloc_to.max(pos);
        let need = pos
            .saturating_add(upcoming)
            .saturating_add(VLOG_PREALLOC_CHUNK);
        while self.prealloc_to < need {
            if self.file.preallocate(VLOG_PREALLOC_CHUNK).is_err() {
                break;
            }
            self.prealloc_to = self.prealloc_to.saturating_add(VLOG_PREALLOC_CHUNK);
        }
    }

    /// `write()` the userspace tail. No fsync.
    ///
    /// # Errors
    /// I/O.
    pub fn flush_pending(&mut self) -> Result<()> {
        if self.pending.is_empty() && self.pending_large.is_empty() {
            return Ok(());
        }
        self.reserve_space(self.staged_len() as u64);
        if !self.pending.is_empty() {
            Write::write_all(&mut self.file, &self.pending)?;
            self.pending.clear();
        }
        if !self.pending_large.is_empty() {
            // One contiguous write — `writev` on an O_APPEND handle was
            // dropping the payload (get read UnexpectedEof). Concat is
            // once per 64 KiB, not once per 16 KiB put.
            let mut buf = Vec::with_capacity(self.pending_large_bytes);
            for rec in &self.pending_large {
                buf.extend_from_slice(&rec.hdr);
                buf.extend_from_slice(rec.data.as_ref());
            }
            Write::write_all(&mut self.file, &buf)?;
            self.pending_large.clear();
            self.pending_large_bytes = 0;
        }
        self.pending_start = self.next_offset;
        self.needs_sync = true;
        Ok(())
    }

    /// Flush the tail and barrier at the **same class as WAL G1**
    /// (`sync_data_strong`). Pointers in the WAL must not become durable
    /// before this returns. Not `sync_all`: that was a full metadata
    /// `fsync` / uring wait on Linux (RFC-0062 P1.1 blob_set).
    ///
    /// No-op when nothing is staged and the file is already durable —
    /// `vlog_prepare_wal(true)` runs on every G1 commit, including
    /// 100-byte ycsb puts that never spilled.
    ///
    /// # Errors
    /// I/O.
    pub fn sync_pending(&mut self) -> Result<()> {
        if self.pending.is_empty() && self.pending_large.is_empty() && !self.needs_sync {
            return Ok(());
        }
        self.flush_pending()?;
        self.file.sync_data_strong()?;
        self.needs_sync = false;
        Ok(())
    }

    /// Whether a G1 `sync_pending` would issue a barrier (tests / probes).
    #[must_use]
    pub fn needs_barrier(&self) -> bool {
        !self.pending.is_empty() || !self.pending_large.is_empty() || self.needs_sync
    }

    /// Bytes staged in userspace (tests / probes).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.staged_len()
    }

    /// Path that holds `ptr`.
    ///
    /// File 0 is `VALUES.vlog` / `.new`. After rotation the open handle is a
    /// numbered blob — using `self.path` for file 0 would read the wrong file
    /// (`VLG1` get → None). When the handle is not the legacy log, honor
    /// `use_new` the same way [`Self::resolve_path`] does.
    #[must_use]
    pub fn path_for_ptr<E: Env>(
        env: &E,
        dir: &Path,
        ptr: VlogPtr,
        use_new: bool,
        open_path: &Path,
    ) -> PathBuf {
        if ptr.file_num == 0 {
            let name = open_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == VLOG_FILE_NAME || name == VLOG_NEW_NAME {
                return open_path.to_path_buf();
            }
            return Self::resolve_path(env, dir, use_new);
        }
        blob_path(dir, ptr.file_num)
    }

    /// Read a [`VlogPtr`], opening a sealed blob by `file_num` when needed.
    ///
    /// `use_new` selects `VALUES.vlog.new` for file 0 when the handle is a
    /// numbered blob (mid-GC after MANIFEST, RFC-0029 mixed mode).
    ///
    /// # Errors
    /// I/O or CRC.
    pub fn read_ptr_on<E: Env>(
        &self,
        env: &E,
        dir: &Path,
        ptr: VlogPtr,
        use_new: bool,
    ) -> Result<Bytes> {
        if let Some(bytes) = self.read_pending(ptr.offset, ptr.len, ptr.crc)? {
            return Ok(bytes);
        }
        let path = Self::path_for_ptr(env, dir, ptr, use_new, &self.path);
        read_record_at(env, &path, ptr.offset, ptr.len, ptr.crc)
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
        if let Some(bytes) = self.read_pending(offset, len, expect_crc)? {
            return Ok(bytes);
        }
        read_record_at(env, &self.path, offset, len, expect_crc)
    }

    fn read_pending(&self, offset: u64, len: u32, expect_crc: u32) -> Result<Option<Bytes>> {
        for rec in &self.pending_large {
            if rec.offset != offset {
                continue;
            }
            let stored_len = u32::from_le_bytes(rec.hdr[0..4].try_into().unwrap());
            let stored_crc = u32::from_le_bytes(rec.hdr[4..8].try_into().unwrap());
            if stored_len != len {
                return Err(CoreError::CorruptValue(format!(
                    "len mismatch in vlog pending at {offset}: stored {stored_len} expect {len}"
                )));
            }
            if !crate::wal::crc::crc_match_ok(stored_crc, expect_crc) {
                return Err(CoreError::CorruptValue(format!(
                    "crc mismatch in vlog pending at {offset}"
                )));
            }
            let crc = crc32c::crc32c(rec.data.as_ref());
            if !crate::wal::crc::crc_match_ok(crc, expect_crc) {
                return Err(CoreError::CorruptValue(format!(
                    "data crc mismatch in vlog pending at {offset}"
                )));
            }
            return Ok(Some(rec.data.clone()));
        }
        if self.pending.is_empty() {
            return Ok(None);
        }
        let rec_len = 8u64.saturating_add(u64::from(len));
        let end = offset.saturating_add(rec_len);
        if offset < self.pending_start || end > self.next_offset {
            return Ok(None);
        }
        let idx = usize::try_from(offset - self.pending_start)
            .map_err(|_| CoreError::Internal("vlog pending offset exceeds usize".into()))?;
        let need = usize::try_from(rec_len)
            .map_err(|_| CoreError::Internal("vlog pending record exceeds usize".into()))?;
        if idx.saturating_add(need) > self.pending.len() {
            return Ok(None);
        }
        let hdr = &self.pending[idx..idx + 8];
        let stored_len = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let stored_crc = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
        if stored_len != len {
            return Err(CoreError::CorruptValue(format!(
                "len mismatch in vlog pending at {offset}: stored {stored_len} expect {len}"
            )));
        }
        if !crate::wal::crc::crc_match_ok(stored_crc, expect_crc) {
            return Err(CoreError::CorruptValue(format!(
                "crc mismatch in vlog pending at {offset}"
            )));
        }
        let payload = &self.pending[idx + 8..idx + need];
        let crc = crc32c::crc32c(payload);
        if !crate::wal::crc::crc_match_ok(crc, expect_crc) {
            return Err(CoreError::CorruptValue(format!(
                "data crc mismatch in vlog pending at {offset}"
            )));
        }
        Ok(Some(Bytes::copy_from_slice(payload)))
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
        // F3: never `remove_file` the staging path first — `create` truncates
        // in place (same contract `Wal::create_on` relies on), so there is no
        // window with no `.new` under a live `vlog_use_new`. Callers guarantee
        // the file is not live (un-promoted rounds are promoted before this).
        let mut body = Vec::new();
        body.extend_from_slice(MAGIC);
        let mut remap = std::collections::HashMap::new();
        let mut next = MAGIC.len() as u64;
        for (old_off, data) in live {
            let len = u32::try_from(data.len())
                .map_err(|_| CoreError::Internal("vlog value too large".into()))?;
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
        // F4: the rewritten `.new` dirent must be durable before MANIFEST can
        // commit `vlog_use_new` — propagate instead of best-effort.
        env.sync_dir(dir)?;
        let stats = VlogRewriteStats {
            bytes_before,
            bytes_after: body.len() as u64,
            live_records: live.len() as u64,
        };
        Ok((stats, remap))
    }

    /// Rewrite `live` records into a **new** blob file `dest_num` (does not delete source).
    ///
    /// `live` is `(old_offset, payload)` from one source generation.
    /// Remap keys are old offsets; values are [`encode_vlog_ptr`] for `dest_num`.
    ///
    /// # Errors
    /// I/O.
    pub fn rewrite_live_to_blob<E: Env<File = F>>(
        env: &E,
        dir: &Path,
        dest_num: u32,
        live: &[(u64, Bytes)],
        bytes_before: u64,
    ) -> Result<(VlogRewriteStats, std::collections::HashMap<u64, Bytes>)> {
        if dest_num == 0 {
            return Err(CoreError::Internal(
                "rewrite_live_to_blob dest must be a numbered blob".into(),
            ));
        }
        let dest = blob_path(dir, dest_num);
        if env.exists(&dest) {
            return Err(CoreError::Internal(format!(
                "blob dest exists: {}",
                dest.display()
            )));
        }
        let mut body = Vec::new();
        body.extend_from_slice(MAGIC);
        let mut remap = std::collections::HashMap::new();
        let mut next = MAGIC.len() as u64;
        for (old_off, data) in live {
            let len = u32::try_from(data.len())
                .map_err(|_| CoreError::Internal("vlog value too large".into()))?;
            let crc = crc32c::crc32c(data);
            let new_off = next;
            body.extend_from_slice(&len.to_le_bytes());
            body.extend_from_slice(&crc.to_le_bytes());
            body.extend_from_slice(data);
            next = next
                .checked_add(8 + u64::from(len))
                .ok_or_else(|| CoreError::Internal("blob rewrite overflow".into()))?;
            remap.insert(
                *old_off,
                encode_vlog_ptr(VlogPtr {
                    file_num: dest_num,
                    offset: new_off,
                    len,
                    crc,
                }),
            );
        }
        {
            let mut f = env.create(&dest)?;
            Write::write_all(&mut f, &body)?;
            f.sync_all()?;
        }
        // F4: propagate — the new blob generation's dirent precedes the
        // MANIFEST commit that makes its pointers authoritative.
        env.sync_dir(dir)?;
        Ok((
            VlogRewriteStats {
                bytes_before,
                bytes_after: body.len() as u64,
                live_records: live.len() as u64,
            },
            remap,
        ))
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
        // F51: do **not** remove `main` before rename. POSIX rename replaces the
        // destination atomically; remove-then-rename left a window with no vlog
        // file (and a failed rename after remove lost the primary).
        env.rename(&newp, &main)?;
        // F4: propagate — a promote whose rename is not yet durable must not
        // report success (MANIFEST clear follows; reopen reconciles either way).
        env.sync_dir(dir)?;
        // Best-effort clear legacy adopt marker from older builds.
        let adopt = dir.join(VLOG_ADOPT_NAME);
        if env.exists(&adopt) {
            let _ = env.remove_file(&adopt);
        }
        Self::open_path(env, main)
    }
}

/// RFC-0081 P2.1: live blob/vlog disk read is a catalog caller of
/// `crc_match_ok` (pair `crc_match`; twin `verus/crc_match.rs`).
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
        return Err(CoreError::CorruptValue(format!(
            "len mismatch at {offset}: stored {stored_len} expect {len}"
        )));
    }
    if !crate::wal::crc::crc_match_ok(stored_crc, expect_crc) {
        return Err(CoreError::CorruptValue(format!("crc mismatch at {offset}")));
    }
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?;
    let got = crc32c::crc32c(&buf);
    if !crate::wal::crc::crc_match_ok(got, expect_crc) {
        return Err(CoreError::CorruptValue(format!(
            "data crc mismatch at {offset}"
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
    if let Some(ptr) = decode_vlog_ptr(stored.as_ref()) {
        let log = vlog
            .ok_or_else(|| CoreError::Internal("vlog ref present but value log not open".into()))?;
        let dir = log.path.parent().unwrap_or_else(|| Path::new("."));
        log.read_ptr_on(env, dir, ptr, false)
    } else {
        Ok(stored)
    }
}

/// Remap a stored value if it is a `VLG1` pointer present in `remap` (old offset → new ref).
///
/// `VLG3` is left untouched: file-0 rewrite offsets collide with blob offsets
/// (both start at 8) and must not steal numbered-blob pointers.
#[must_use]
pub fn remap_stored_value<S: std::hash::BuildHasher>(
    stored: &Bytes,
    remap: &std::collections::HashMap<u64, Bytes, S>,
) -> Bytes {
    if let Some(ptr) = decode_vlog_ptr(stored.as_ref()) {
        if ptr.file_num != 0 {
            return stored.clone();
        }
        if let Some(new_ref) = remap.get(&ptr.offset) {
            return new_ref.clone();
        }
    }
    stored.clone()
}

/// Remap only pointers that match `file_num` (RFC-0029 one-blob GC).
#[must_use]
pub fn remap_stored_blob<S: std::hash::BuildHasher>(
    stored: &Bytes,
    file_num: u32,
    remap: &std::collections::HashMap<u64, Bytes, S>,
) -> Bytes {
    if let Some(ptr) = decode_vlog_ptr(stored.as_ref()) {
        if ptr.file_num == file_num {
            if let Some(new_ref) = remap.get(&ptr.offset) {
                return new_ref.clone();
            }
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
        let blob = encode_vlog_ptr(VlogPtr {
            file_num: 3,
            offset: off,
            len,
            crc,
        });
        let decoded = decode_vlog_ptr(&blob).unwrap();
        assert_eq!(decoded.file_num, 3);
        assert_eq!(decoded.offset, off);
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0081 P0: production append+read; a flipped payload is
    /// CorruptValue, never a value. AS-IS `crc_match_ok` would accept it.
    #[test]
    fn crc_mismatch_on_live_vlog_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(crate::wal::crc::crc_match_ok_as_is(1, 2));
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-crc-0081-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let payload = b"vlog-crc-payload-0081";
        let (off, len, crc) = {
            let mut log = ValueLog::open_on(&env, &dir).unwrap();
            log.append(payload).unwrap()
        };
        let path = dir.join(VLOG_FILE_NAME);
        let mut bytes = fs::read(&path).unwrap();
        let pos = usize::try_from(off).unwrap().saturating_add(8);
        assert!(
            pos < bytes.len(),
            "payload must be on disk after append+sync"
        );
        bytes[pos] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let log = ValueLog::open_on(&env, &dir).unwrap();
        let err = log.read_at_on(&env, off, len, crc).unwrap_err();
        let _ = fs::remove_dir_all(&dir);
        let msg = err.to_string();
        assert!(
            msg.contains("crc"),
            "flipped vlog must not serve a value; got {err:?}"
        );
    }

    /// RFC-0081 P2.2: vlog `crc_match_ok` is not a CRC32C collision theorem.
    #[test]
    fn vlog_crc_collision_axiom_remains() {
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

    /// RFC-0081 P1.1: numbered `*.blob` reads use `crc_match_ok` (same gate
    /// as `VALUES.vlog`). AS-IS would serve the flipped payload.
    #[test]
    fn crc_mismatch_on_live_blob_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any blob crc would match"
        );
        let dir = std::env::temp_dir().join(format!(
            "pedradb-blob-crc-0081-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let payload = b"blob-crc-payload-0081";
        let ptr = {
            let mut log = ValueLog::open_blob(&env, &dir, 1).unwrap();
            let (offset, len, crc) = log.append(payload).unwrap();
            VlogPtr {
                file_num: 1,
                offset,
                len,
                crc,
            }
        };
        let path = blob_path(&dir, 1);
        assert!(
            path.extension().and_then(|s| s.to_str()) == Some("blob"),
            "P1.1 tooth is a numbered blob file, not VALUES.vlog"
        );
        let mut bytes = fs::read(&path).unwrap();
        let pos = usize::try_from(ptr.offset).unwrap().saturating_add(8);
        assert!(
            pos < bytes.len(),
            "payload must be on disk after append+sync"
        );
        bytes[pos] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let log = ValueLog::open_on(&env, &dir).unwrap();
        let err = log
            .read_ptr_on(&env, &dir, ptr, false)
            .expect_err("flipped blob must not serve a value");
        let _ = fs::remove_dir_all(&dir);
        let msg = err.to_string();
        assert!(
            msg.contains("crc"),
            "must fail on crc_match_ok, not a parse; got {err:?}"
        );
    }

    #[test]
    fn append_pending_large_readable_before_flush() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-large-pending-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log = ValueLog::open_on(&env, &dir).unwrap();
        let data = Bytes::from(vec![7u8; 16 * 1024]);
        let (off, len, crc) = log.append_pending_bytes(data.clone()).unwrap();
        assert!(log.pending_len() > 0, "large record stays in userspace");
        let on_disk = fs::metadata(dir.join(VLOG_FILE_NAME)).unwrap().len();
        assert_eq!(on_disk, 8, "only magic on disk until flush");
        let got = log.read_at_on(&env, off, len, crc).unwrap();
        assert_eq!(got.as_ref(), data.as_ref());
        log.flush_pending().unwrap();
        assert_eq!(log.pending_len(), 0);
        let got = log.read_at_on(&env, off, len, crc).unwrap();
        assert_eq!(got.as_ref(), data.as_ref());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_pending_readable_before_flush() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-pending-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log = ValueLog::open_on(&env, &dir).unwrap();
        let data = vec![9u8; 100];
        let (off, len, crc) = log.append_pending(&data).unwrap();
        assert!(log.pending_len() > 0, "small record stays in userspace");
        let on_disk = fs::metadata(dir.join(VLOG_FILE_NAME)).unwrap().len();
        assert_eq!(on_disk, 8, "only magic on disk until flush");
        let got = log.read_at_on(&env, off, len, crc).unwrap();
        assert_eq!(got.as_ref(), data.as_slice());
        log.flush_pending().unwrap();
        assert_eq!(log.pending_len(), 0);
        let got = log.read_at_on(&env, off, len, crc).unwrap();
        assert_eq!(got.as_ref(), data.as_slice());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_pending_flushes_at_buffer() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-buf-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log = ValueLog::open_on(&env, &dir).unwrap();
        let chunk = vec![1u8; 16 * 1024];
        let rec = 8 + chunk.len();
        for _ in 0..4 {
            log.append_pending(&chunk).unwrap();
        }
        // 3 records = 49 176 < 64KiB; the 4th would overflow so we flush 3
        // then keep the 4th in userspace.
        assert_eq!(log.pending_len(), rec);
        let on_disk = fs::metadata(dir.join(VLOG_FILE_NAME)).unwrap().len();
        assert_eq!(on_disk, 8 + 3 * rec as u64, "magic + 3 flushed records");
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0062 P1.1: G1 `vlog_prepare_wal(true)` must not `fdatasync` when
    /// the handle has no unsynced bytes (small puts under the blob
    /// threshold). A spilled record still takes exactly one strong barrier.
    #[test]
    fn sync_pending_skips_empty_and_barriers_once_when_dirty() {
        use crate::env::EnvFile;
        use std::cell::Cell;
        use std::io::{self, Read, Seek, SeekFrom, Write};
        use std::rc::Rc;

        struct CountFile {
            inner: std::fs::File,
            strong: Rc<Cell<u64>>,
        }
        impl Read for CountFile {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buf)
            }
        }
        impl Write for CountFile {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.inner.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }
        }
        impl Seek for CountFile {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.inner.seek(pos)
            }
        }
        impl EnvFile for CountFile {
            fn sync_data(&mut self) -> io::Result<()> {
                self.inner.sync_data()
            }
            fn sync_data_strong(&mut self) -> io::Result<()> {
                self.strong.set(self.strong.get() + 1);
                self.inner.sync_data_strong()
            }
            fn sync_all(&mut self) -> io::Result<()> {
                self.inner.sync_all()
            }
            fn set_len(&mut self, len: u64) -> io::Result<()> {
                self.inner.set_len(len)
            }
            fn len(&mut self) -> io::Result<u64> {
                self.inner.len()
            }
        }

        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-skip-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(VLOG_FILE_NAME);
        {
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(MAGIC).unwrap();
            f.sync_all().unwrap();
        }
        let strong = Rc::new(Cell::new(0));
        let file = CountFile {
            inner: fs::OpenOptions::new()
                .read(true)
                .append(true)
                .open(&path)
                .unwrap(),
            strong: Rc::clone(&strong),
        };
        let mut log = ValueLog {
            path: path.clone(),
            file,
            next_offset: MAGIC.len() as u64,
            pending_start: MAGIC.len() as u64,
            pending: Vec::new(),
            pending_large: Vec::new(),
            pending_large_bytes: 0,
            prealloc_to: MAGIC.len() as u64,
            needs_sync: false,
        };
        assert!(!log.needs_barrier());
        log.sync_pending().unwrap();
        log.sync_pending().unwrap();
        assert_eq!(strong.get(), 0, "empty vlog must not fsync");

        log.append_pending(&[1u8; 32]).unwrap();
        assert!(log.needs_barrier());
        log.sync_pending().unwrap();
        assert_eq!(strong.get(), 1, "one spill, one G1 barrier");
        assert!(!log.needs_barrier());
        log.sync_pending().unwrap();
        assert_eq!(strong.get(), 1, "already durable: skip");
        let _ = fs::remove_dir_all(&dir);
    }

    /// After the handle moves to a numbered blob, file-0 reads must still
    /// hit `VALUES.vlog` (not the active `.blob`).
    #[test]
    fn read_ptr_on_file0_after_blob_handle() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-ptr-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        let mut log0 = ValueLog::open_on(&env, &dir).unwrap();
        let legacy = vec![0x11u8; 64];
        let (off0, len0, crc0) = log0.append(&legacy).unwrap();
        drop(log0);
        let mut blob = ValueLog::open_blob(&env, &dir, 1).unwrap();
        let later = vec![0x22u8; 64];
        let (off1, len1, crc1) = blob.append(&later).unwrap();
        let got0 = blob
            .read_ptr_on(
                &env,
                &dir,
                VlogPtr {
                    file_num: 0,
                    offset: off0,
                    len: len0,
                    crc: crc0,
                },
                false,
            )
            .unwrap();
        assert_eq!(got0.as_ref(), legacy.as_slice());
        let got1 = blob
            .read_ptr_on(
                &env,
                &dir,
                VlogPtr {
                    file_num: 1,
                    offset: off1,
                    len: len1,
                    crc: crc1,
                },
                false,
            )
            .unwrap();
        assert_eq!(got1.as_ref(), later.as_slice());
        let mut remap = std::collections::HashMap::new();
        remap.insert(off0, encode_vlog_ref(8, 1, 0));
        let vlg3 = encode_vlog_ptr(VlogPtr {
            file_num: 1,
            offset: off0,
            len: len0,
            crc: crc0,
        });
        assert_eq!(
            remap_stored_value(&vlg3, &remap),
            vlg3,
            "file-0 remap must not rewrite VLG3"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// F51: promote uses atomic rename over primary (no remove-before-rename).
    #[test]
    fn promote_atomic_rename_keeps_primary() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-vlog-promote-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let env = StdEnv;
        {
            let mut vl = ValueLog::open_on(&env, &dir).unwrap();
            vl.append(&[0xAAu8; 64]).unwrap();
        }
        let main = dir.join(VLOG_FILE_NAME);
        let newp = dir.join(VLOG_NEW_NAME);
        // Stage a larger .new (rewrite path).
        let live = vec![(8u64, Bytes::from(vec![0xAAu8; 64]))];
        ValueLog::<std::fs::File>::rewrite_live_to_new(&env, &dir, &live).unwrap();
        assert!(env.exists(&newp));
        let before = env.metadata_len(&newp).unwrap();
        ValueLog::promote_new_and_reopen(&env, &dir).unwrap();
        assert!(env.exists(&main), "primary must exist after promote");
        assert!(!env.exists(&newp), ".new must be consumed");
        let after = env.metadata_len(&main).unwrap();
        assert_eq!(after, before, "promoted primary keeps staged bytes");
        // use_new with neither file must fail-stop, not invent empty log.
        let _ = env.remove_file(&main);
        let err = ValueLog::<std::fs::File>::open_with_flag(&env, &dir, true).unwrap_err();
        assert!(
            err.to_string().contains("vlog missing") || err.to_string().contains("refuse"),
            "got {err}"
        );
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
        let (stats, remap) =
            ValueLog::<std::fs::File>::rewrite_live_to_new(&env, &dir, &live).unwrap();
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
