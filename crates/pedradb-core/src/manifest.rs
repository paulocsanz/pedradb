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
const FORMAT_VERSION: u32 = 1;

/// Live SST set + allocator cursor recovered from (or written to) disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionSet {
    /// Next SST / MANIFEST number to allocate (`000001.sst`, …).
    pub next_file_num: u64,
    /// Live SST file numbers in oldest → newest order.
    pub sst_file_nums: Vec<u64>,
    /// File number of this MANIFEST record (for `MANIFEST-{n:06}`).
    pub manifest_file_num: u64,
}

impl VersionSet {
    /// Empty database: first file number is 1.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            next_file_num: 1,
            sst_file_nums: Vec::new(),
            manifest_file_num: 0,
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
}

/// Encode a version set to bytes (payload + trailing CRC32C of the payload).
#[must_use]
pub fn encode(vs: &VersionSet) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + 4 + 8 + 4 + vs.sst_file_nums.len() * 8 + 4);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&vs.next_file_num.to_le_bytes());
    buf.extend_from_slice(&vs.manifest_file_num.to_le_bytes());
    let n = u32::try_from(vs.sst_file_nums.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&n.to_le_bytes());
    for num in &vs.sst_file_nums {
        buf.extend_from_slice(&num.to_le_bytes());
    }
    let crc = crc32c::crc32c(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf
}

/// Decode a version set from bytes.
///
/// # Errors
/// Corrupt or truncated payload.
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
    let stored = u32::from_le_bytes([
        crc_bytes[0],
        crc_bytes[1],
        crc_bytes[2],
        crc_bytes[3],
    ]);
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
    if version != FORMAT_VERSION {
        return Err(CoreError::CorruptManifest(format!(
            "unsupported version {version}"
        )));
    }
    let next_file_num = le_u64(payload, 8)?;
    let manifest_file_num = le_u64(payload, 16)?;
    let n = le_u32(payload, 24)? as usize;
    if n > 1_000_000 {
        return Err(CoreError::CorruptManifest(format!(
            "implausible SST count {n}"
        )));
    }
    let need = 28 + n * 8;
    if payload.len() < need {
        return Err(CoreError::CorruptManifest("truncated file list".into()));
    }
    if payload.len() != need {
        return Err(CoreError::CorruptManifest("trailing garbage before CRC".into()));
    }
    let mut sst_file_nums = Vec::with_capacity(n);
    for i in 0..n {
        sst_file_nums.push(le_u64(payload, 28 + i * 8)?);
    }
    Ok(VersionSet {
        next_file_num,
        sst_file_nums,
        manifest_file_num,
    })
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
        f.sync_all()?;
    }
    env.rename(&man_tmp, &man_final)?;

    let cur_tmp = dir.join(CURRENT_TMP);
    {
        let mut f = env.create(&cur_tmp)?;
        f.write_all(man_name.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    env.rename(&cur_tmp, &dir.join(CURRENT_FILE))?;

    if sync {
        let _ = env.sync_dir(dir);
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

/// Delete `NNNNNN.sst` files not present in `live`.
///
/// # Errors
/// Directory list I/O.
pub fn gc_orphan_ssts<E: Env>(env: &E, dir: &Path, live: &[u64]) -> Result<()> {
    let live: std::collections::HashSet<u64> = live.iter().copied().collect();
    if !env.exists(dir) {
        return Ok(());
    }
    for name in env.read_dir_names(dir)? {
        if let Some(num) = parse_sst_name(&name) {
            if !live.contains(&num) {
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

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
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
            manifest_file_num: 2,
        };
        let out = decode(&encode(&vs)).unwrap();
        assert_eq!(out, vs);
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
            manifest_file_num: 0,
        };
        install_next(&env, &dir, &mut vs, true).unwrap();
        assert_eq!(vs.manifest_file_num, 1);

        let loaded = load(&env, &dir).unwrap().unwrap();
        assert_eq!(loaded.sst_file_nums, vec![1, 3]);
        assert_eq!(loaded.next_file_num, 4);

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
