//! On-disk version inventory (RFC-0009 P2.1).
//!
//! Layout (LevelDB-inspired, simplified full rewrite — not an append-only edit log):
//!
//! - `CURRENT` — one line: active manifest file name (`MANIFEST-000001`)
//! - `MANIFEST-NNNNNN` — binary inventory of live SST file numbers + next file num
//!
//! Updates are crash-safe: write `MANIFEST-*.tmp` → rename → write `CURRENT.tmp` → rename.
//! A crash mid-update leaves the previous `CURRENT` pointer valid; orphans are GC'd on open.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::env::{Env, EnvFile};
use crate::error::{CoreError, Result};

/// Pointer file naming the active MANIFEST.
pub const CURRENT_FILE: &str = "CURRENT";
/// Temporary name while installing a new `CURRENT`.
pub const CURRENT_TMP: &str = "CURRENT.tmp";
/// Prefix of version inventory files (`MANIFEST-000001`).
pub const MANIFEST_PREFIX: &str = "MANIFEST-";

const MAGIC: &[u8; 4] = b"PDBM";
/// Legacy: file numbers only (all treated as L0).
const FORMAT_VERSION_V1: u32 = 1;
/// Levels only (no vlog flag).
const FORMAT_VERSION_V2: u32 = 2;
/// Levels + `vlog_use_new` (RFC-0016 crash-safe value-log GC).
const FORMAT_VERSION_V3: u32 = 3;
/// Current: v3 + `earliest_readable_seq` (open-items §2.1 watermark across reopen).
const FORMAT_VERSION: u32 = 4;

/// Live SST set + allocator cursor recovered from (or written to) disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionSet {
    /// Next SST / MANIFEST number to allocate (`000001.sst`, …).
    pub next_file_num: u64,
    /// Live SST file numbers in oldest → newest order within the inventory.
    pub sst_file_nums: Vec<u64>,
    /// LSM level for each entry in [`Self::sst_file_nums`] (same length; 0 = L0).
    pub sst_levels: Vec<u32>,
    /// File number of this MANIFEST record (for `MANIFEST-{n:06}`).
    pub manifest_file_num: u64,
    /// When true, open must use `VALUES.vlog.new` (SST pointers already remapped).
    pub vlog_use_new: bool,
    /// Version-GC watermark: snapshots with `seq < this` are too old after reopen.
    pub earliest_readable_seq: u64,
}

impl VersionSet {
    /// Empty database: first file number is 1.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            next_file_num: 1,
            sst_file_nums: Vec::new(),
            sst_levels: Vec::new(),
            manifest_file_num: 0,
            vlog_use_new: false,
            earliest_readable_seq: 0,
        }
    }

    /// Path of SST `num` under `dir`.
    #[must_use]
    pub fn sst_path(dir: &Path, num: u64) -> PathBuf {
        dir.join(format!("{num:06}.sst"))
    }

    /// MANIFEST path for this version set.
    #[must_use]
    pub fn manifest_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{MANIFEST_PREFIX}{:06}", self.manifest_file_num))
    }

    /// Ensure `sst_levels` matches `sst_file_nums` (pad with L0 if short).
    pub fn normalize_levels(&mut self) {
        while self.sst_levels.len() < self.sst_file_nums.len() {
            self.sst_levels.push(0);
        }
        self.sst_levels.truncate(self.sst_file_nums.len());
    }
}

/// Encode a version set to bytes (payload + trailing CRC32C of the payload).
///
/// Writes format **v4**: each live file is `(file_num u64, level u32)`, then
/// `vlog_use_new u8`, then `earliest_readable_seq u64`.
#[must_use]
pub fn encode(vs: &VersionSet) -> Vec<u8> {
    let n = vs.sst_file_nums.len();
    let mut buf = Vec::with_capacity(4 + 4 + 8 + 8 + 4 + n * 12 + 1 + 8 + 4);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&vs.next_file_num.to_le_bytes());
    buf.extend_from_slice(&vs.manifest_file_num.to_le_bytes());
    let n_u32 = u32::try_from(n).unwrap_or(u32::MAX);
    buf.extend_from_slice(&n_u32.to_le_bytes());
    for i in 0..n {
        let num = vs.sst_file_nums[i];
        let level = vs.sst_levels.get(i).copied().unwrap_or(0);
        buf.extend_from_slice(&num.to_le_bytes());
        buf.extend_from_slice(&level.to_le_bytes());
    }
    buf.push(u8::from(vs.vlog_use_new));
    buf.extend_from_slice(&vs.earliest_readable_seq.to_le_bytes());
    let crc = crc32c::crc32c(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf
}

/// Decode a version set from bytes (v1, v2, or v3).
///
/// # Errors
/// Corrupt or truncated payload.
#[allow(clippy::too_many_lines)]
pub fn decode(buf: &[u8]) -> Result<VersionSet> {
    fn le_u32(buf: &[u8], off: usize) -> Result<u32> {
        let bytes: [u8; 4] = buf
            .get(off..off + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| CoreError::CorruptManifest("truncated".into()))?;
        Ok(u32::from_le_bytes(bytes))
    }
    fn le_u64(buf: &[u8], off: usize) -> Result<u64> {
        let bytes: [u8; 8] = buf
            .get(off..off + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| CoreError::CorruptManifest("truncated".into()))?;
        Ok(u64::from_le_bytes(bytes))
    }

    if buf.len() < 4 + 4 + 8 + 8 + 4 + 4 {
        return Err(CoreError::CorruptManifest("too short".into()));
    }
    // Trailing CRC32C over the payload (F5: silent empty inventory on bit-flip of `n`).
    let (payload, crc_bytes) = buf.split_at(buf.len() - 4);
    let stored = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
    let computed = crc32c::crc32c(payload);
    if stored != computed {
        return Err(CoreError::CorruptManifest(format!(
            "CRC mismatch: stored {stored:#010x}, computed {computed:#010x}"
        )));
    }
    if &payload[0..4] != MAGIC {
        return Err(CoreError::CorruptManifest("bad magic".into()));
    }
    let version = le_u32(payload, 4)?;
    let next_file_num = le_u64(payload, 8)?;
    let manifest_file_num = le_u64(payload, 16)?;
    let n = le_u32(payload, 24)? as usize;
    if n > 1_000_000 {
        return Err(CoreError::CorruptManifest(format!(
            "implausible SST count {n}"
        )));
    }
    match version {
        FORMAT_VERSION_V1 => {
            let need = 28 + n * 8;
            if payload.len() < need {
                return Err(CoreError::CorruptManifest("truncated file list".into()));
            }
            if payload.len() != need {
                return Err(CoreError::CorruptManifest(
                    "trailing garbage before CRC".into(),
                ));
            }
            let mut sst_file_nums = Vec::with_capacity(n);
            for i in 0..n {
                sst_file_nums.push(le_u64(payload, 28 + i * 8)?);
            }
            Ok(VersionSet {
                next_file_num,
                sst_levels: vec![0; n],
                sst_file_nums,
                manifest_file_num,
                vlog_use_new: false,
                earliest_readable_seq: 0,
            })
        }
        FORMAT_VERSION_V2 => {
            let need = 28 + n * 12;
            if payload.len() < need {
                return Err(CoreError::CorruptManifest("truncated file list".into()));
            }
            if payload.len() != need {
                return Err(CoreError::CorruptManifest(
                    "trailing garbage before CRC".into(),
                ));
            }
            let mut sst_file_nums = Vec::with_capacity(n);
            let mut sst_levels = Vec::with_capacity(n);
            for i in 0..n {
                let base = 28 + i * 12;
                sst_file_nums.push(le_u64(payload, base)?);
                sst_levels.push(le_u32(payload, base + 8)?);
            }
            Ok(VersionSet {
                next_file_num,
                sst_file_nums,
                sst_levels,
                manifest_file_num,
                vlog_use_new: false,
                earliest_readable_seq: 0,
            })
        }
        FORMAT_VERSION_V3 => {
            let need = 28 + n * 12 + 1;
            if payload.len() < need {
                return Err(CoreError::CorruptManifest("truncated file list".into()));
            }
            if payload.len() != need {
                return Err(CoreError::CorruptManifest(
                    "trailing garbage before CRC".into(),
                ));
            }
            let mut sst_file_nums = Vec::with_capacity(n);
            let mut sst_levels = Vec::with_capacity(n);
            for i in 0..n {
                let base = 28 + i * 12;
                sst_file_nums.push(le_u64(payload, base)?);
                sst_levels.push(le_u32(payload, base + 8)?);
            }
            let vlog_use_new = payload[28 + n * 12] != 0;
            Ok(VersionSet {
                next_file_num,
                sst_file_nums,
                sst_levels,
                manifest_file_num,
                vlog_use_new,
                earliest_readable_seq: 0,
            })
        }
        FORMAT_VERSION => {
            let need = 28 + n * 12 + 1 + 8;
            if payload.len() < need {
                return Err(CoreError::CorruptManifest("truncated file list".into()));
            }
            if payload.len() != need {
                return Err(CoreError::CorruptManifest(
                    "trailing garbage before CRC".into(),
                ));
            }
            let mut sst_file_nums = Vec::with_capacity(n);
            let mut sst_levels = Vec::with_capacity(n);
            for i in 0..n {
                let base = 28 + i * 12;
                sst_file_nums.push(le_u64(payload, base)?);
                sst_levels.push(le_u32(payload, base + 8)?);
            }
            let flag_off = 28 + n * 12;
            let vlog_use_new = payload[flag_off] != 0;
            let earliest_readable_seq = le_u64(payload, flag_off + 1)?;
            Ok(VersionSet {
                next_file_num,
                sst_file_nums,
                sst_levels,
                manifest_file_num,
                vlog_use_new,
                earliest_readable_seq,
            })
        }
        other => Err(CoreError::CorruptManifest(format!(
            "unsupported version {other}"
        ))),
    }
}

/// Load the active version set, if `CURRENT` exists.
///
/// # Errors
/// I/O or corrupt MANIFEST / CURRENT.
pub fn load<E: Env>(env: &E, dir: &Path) -> Result<Option<VersionSet>> {
    let current_path = dir.join(CURRENT_FILE);
    if !env.exists(&current_path) {
        return Ok(None);
    }
    let mut f = env.open_read(&current_path)?;
    let mut text = String::new();
    f.read_to_string(&mut text)
        .map_err(|e| CoreError::CorruptManifest(format!("read CURRENT: {e}")))?;
    let name = text.trim();
    // Empty CURRENT (torn write / unsynced crash) → treat as missing inventory.
    if name.is_empty() {
        return Ok(None);
    }
    if !name.starts_with(MANIFEST_PREFIX) {
        return Err(CoreError::CorruptManifest(format!(
            "bad CURRENT contents: {name:?}"
        )));
    }
    let path = dir.join(name);
    if !env.exists(&path) {
        return Err(CoreError::CorruptManifest(format!(
            "CURRENT points to missing {name}"
        )));
    }
    let mut f = env.open_read(&path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(Some(decode(&buf)?))
}

/// Persist `vs` as a new MANIFEST and swing `CURRENT` (tmp + rename).
///
/// Bumps `vs.manifest_file_num` to `next_file_num`-style allocator: caller should
/// set `manifest_file_num` to a fresh value before calling, or pass through
/// [`install_next`].
///
/// # Errors
/// I/O while writing.
pub fn store<E: Env>(env: &E, dir: &Path, vs: &VersionSet, sync: bool) -> Result<()> {
    let man_name = format!("{MANIFEST_PREFIX}{:06}", vs.manifest_file_num);
    let man_tmp = dir.join(format!("{man_name}.tmp"));
    let man_final = dir.join(&man_name);
    let payload = encode(vs);

    {
        let mut f = env.create(&man_tmp)?;
        f.write_all(&payload)?;
        f.sync_data()?;
    }
    env.rename(&man_tmp, &man_final)?;
    if sync {
        // F195: the MANIFEST rename must be durable BEFORE `CURRENT` can
        // point at it. Without this dir fsync a crash between the two
        // renames can leave CURRENT → a missing MANIFEST (LevelDB/Rocks
        // persist order: MANIFEST, dir, CURRENT, dir).
        env.sync_dir(dir)?;
    }

    let cur_tmp = dir.join(CURRENT_TMP);
    {
        let mut f = env.create(&cur_tmp)?;
        f.write_all(man_name.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_data()?;
    }
    env.rename(&cur_tmp, &dir.join(CURRENT_FILE))?;

    if sync {
        // RFC-0015 H2: durability-required paths must not discard dir fsync errors.
        env.sync_dir(dir)?;
    }

    // Best-effort: drop older MANIFEST-* files (not the one we just wrote).
    if let Ok(names) = env.read_dir_names(dir) {
        for name in names {
            if name.starts_with(MANIFEST_PREFIX) && name != man_name && !is_tmp_name(&name) {
                let _ = env.remove_file(&dir.join(name));
            }
        }
    }
    Ok(())
}

fn is_tmp_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
}

/// Allocate the next manifest file number and store.
///
/// Uses `vs.next_file_num` only for SST numbering; manifest numbers are independent
/// and stored in `vs.manifest_file_num` (incremented here).
///
/// # Errors
/// I/O while writing.
pub fn install_next<E: Env>(env: &E, dir: &Path, vs: &mut VersionSet, sync: bool) -> Result<()> {
    vs.manifest_file_num = vs.manifest_file_num.saturating_add(1).max(1);
    store(env, dir, vs, sync)
}

/// Parse `N….sst` → file number.
///
/// Accepts any non-empty all-digit stem so numbers `>= 1_000_000` (7+ digits
/// from `format!("{n:06}")`) still parse for MANIFEST rebuild / GC (F19).
#[must_use]
pub fn parse_sst_name(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".sst")?;
    if stem.is_empty() || !stem.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// Remove `*.sst.tmp`, `CURRENT.tmp`, and `MANIFEST-*.tmp` left by crashes.
///
/// # Errors
/// Directory list I/O (remove is best-effort).
pub fn cleanup_tmp_files<E: Env>(env: &E, dir: &Path) -> Result<()> {
    if !env.exists(dir) {
        return Ok(());
    }
    for name in env.read_dir_names(dir)? {
        let kill = name.ends_with(".sst.tmp")
            || name == CURRENT_TMP
            || (name.starts_with(MANIFEST_PREFIX) && is_tmp_name(&name));
        if kill {
            let _ = env.remove_file(&dir.join(name));
        }
    }
    Ok(())
}

/// Delete SST files in `dir` whose numbers are not in `live`.
///
/// # Errors
/// Directory list I/O (remove is best-effort per file).
pub fn gc_orphan_ssts<E: Env>(env: &E, dir: &Path, live: &[u64]) -> Result<()> {
    use std::collections::HashSet;
    let live: HashSet<u64> = live.iter().copied().collect();
    if !env.exists(dir) {
        return Ok(());
    }
    for name in env.read_dir_names(dir)? {
        if let Some(n) = parse_sst_name(&name) {
            if !live.contains(&n) {
                let _ = env.remove_file(&dir.join(name));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-manifest-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_sst_name_accepts_wide_numbers() {
        // F19: :06 is min width — 1_000_000 formats as 7 digits.
        assert_eq!(parse_sst_name("000001.sst"), Some(1));
        assert_eq!(parse_sst_name("999999.sst"), Some(999_999));
        assert_eq!(parse_sst_name("1000000.sst"), Some(1_000_000));
        assert_eq!(parse_sst_name("10000000.sst"), Some(10_000_000));
        assert_eq!(parse_sst_name(".sst"), None);
        assert_eq!(parse_sst_name("abc.sst"), None);
        assert_eq!(parse_sst_name("1a.sst"), None);
    }

    #[test]
    fn encode_decode_round_trip() {
        let vs = VersionSet {
            next_file_num: 7,
            sst_file_nums: vec![1, 3, 5],
            sst_levels: vec![0, 1, 1],
            manifest_file_num: 2,
            vlog_use_new: true,
            earliest_readable_seq: 42,
        };
        let out = decode(&encode(&vs)).unwrap();
        assert_eq!(out, vs);
    }

    #[test]
    fn decode_v3_legacy_watermark_zero() {
        // Hand-build v3: levels + vlog flag, no earliest_readable.
        let mut body = Vec::new();
        body.extend_from_slice(MAGIC);
        body.extend_from_slice(&FORMAT_VERSION_V3.to_le_bytes());
        body.extend_from_slice(&4u64.to_le_bytes());
        body.extend_from_slice(&1u64.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&1u64.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.push(1u8); // vlog_use_new
        let crc = crc32c::crc32c(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        let vs = decode(&body).unwrap();
        assert_eq!(vs.sst_file_nums, vec![1]);
        assert!(vs.vlog_use_new);
        assert_eq!(vs.earliest_readable_seq, 0);
    }

    #[test]
    fn decode_v1_legacy_as_all_l0() {
        // Hand-build v1 payload: magic, ver=1, next, man, n=2, nums, crc
        let mut body = Vec::new();
        body.extend_from_slice(MAGIC);
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&4u64.to_le_bytes());
        body.extend_from_slice(&1u64.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&1u64.to_le_bytes());
        body.extend_from_slice(&3u64.to_le_bytes());
        let crc = crc32c::crc32c(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        let vs = decode(&body).unwrap();
        assert_eq!(vs.sst_file_nums, vec![1, 3]);
        assert_eq!(vs.sst_levels, vec![0, 0]);
        assert!(!vs.vlog_use_new);
    }

    #[test]
    fn decode_v2_legacy_vlog_false() {
        let vs = VersionSet {
            next_file_num: 4,
            sst_file_nums: vec![1],
            sst_levels: vec![0],
            manifest_file_num: 1,
            vlog_use_new: false,
            earliest_readable_seq: 0,
        };
        // Build v2 payload manually (no flag byte).
        let mut body = Vec::new();
        body.extend_from_slice(MAGIC);
        body.extend_from_slice(&FORMAT_VERSION_V2.to_le_bytes());
        body.extend_from_slice(&vs.next_file_num.to_le_bytes());
        body.extend_from_slice(&vs.manifest_file_num.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&1u64.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        let crc = crc32c::crc32c(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        let got = decode(&body).unwrap();
        assert_eq!(got.sst_file_nums, vec![1]);
        assert!(!got.vlog_use_new);
    }

    #[test]
    fn store_load_and_orphan_gc() {
        let dir = temp_dir();
        let env = StdEnv;
        // Fake SST files: live 1,3 and orphan 2.
        fs::write(dir.join("000001.sst"), b"a").unwrap();
        fs::write(dir.join("000002.sst"), b"b").unwrap();
        fs::write(dir.join("000003.sst"), b"c").unwrap();

        let mut vs = VersionSet {
            next_file_num: 4,
            sst_file_nums: vec![1, 3],
            sst_levels: vec![0, 1],
            manifest_file_num: 0,
            vlog_use_new: false,
            earliest_readable_seq: 9,
        };
        install_next(&env, &dir, &mut vs, true).unwrap();
        assert_eq!(vs.manifest_file_num, 1);

        let loaded = load(&env, &dir).unwrap().unwrap();
        assert_eq!(loaded.sst_file_nums, vec![1, 3]);
        assert_eq!(loaded.sst_levels, vec![0, 1]);
        assert_eq!(loaded.next_file_num, 4);
        assert!(!loaded.vlog_use_new);
        assert_eq!(loaded.earliest_readable_seq, 9);

        gc_orphan_ssts(&env, &dir, &loaded.sst_file_nums).unwrap();
        assert!(dir.join("000001.sst").exists());
        assert!(!dir.join("000002.sst").exists());
        assert!(dir.join("000003.sst").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_tmp_removes_partials() {
        let dir = temp_dir();
        let env = StdEnv;
        fs::write(dir.join("000009.sst.tmp"), b"x").unwrap();
        fs::write(dir.join("CURRENT.tmp"), b"y").unwrap();
        fs::write(dir.join("MANIFEST-000001.tmp"), b"z").unwrap();
        cleanup_tmp_files(&env, &dir).unwrap();
        assert!(!dir.join("000009.sst.tmp").exists());
        assert!(!dir.join("CURRENT.tmp").exists());
        assert!(!dir.join("MANIFEST-000001.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
