//! At-rest integrity scrub (RFC-0060 P0).
//!
//! Walks every env-visible SST (all data blocks), value-log record, WAL
//! segment and MANIFEST under a DB directory, checking CRC/decode. The CLI
//! (`pedra verify`) and `maintain --verify` call this same function — tests
//! that flip a byte on a closed DB invoke it too.
//!
//! Named holes (not CRC-covered): one-line legacy `CURRENT` has no CRC
//! (two-line pointer does — RFC-0060 P2.15). `LOCK` has none. `CORRUPTLOG`
//! is TSV parse-walked (P2.22). Leftover
//! `*.tmp` install temps are **named FAIL** (not inventory; open GCs them —
//! P2.16). `CHANGELOG` is a WAL-rebuild cache (F33: open quarantines
//! poison); the scrub CRC-walks it when present so `pedra verify` names a
//! rotten cache (P2.6). `CHECKPOINT` is CRC-walked when present (P2.7).
//! History tier is CRC-walked (P2.4). Backup `CATALOG` (`PDBCAT01`) and
//! `wal/*.warch` (`PDBWAR01`) are CRC-walked when present (P2.17/P2.18).
//! See `docs/formal/coverage-map.md`.

use std::io::{Read, Write};
use std::path::Path;

use crate::change_feed::{decode_changelog, CHANGELOG_CORRUPT_FILE_NAME, CHANGELOG_FILE_NAME};
use crate::corrupt::CORRUPTLOG_NAME;
use crate::db::{read_checkpoint_meta, CHECKPOINT_META_FILE};
use crate::env::{Env, EnvFile};
use crate::error::CoreError;
use crate::lock::LOCK_FILE;
use crate::manifest::CURRENT_FILE;
use crate::manifest::{self, MANIFEST_PREFIX};
use crate::sst::SstTable;
use crate::vlog::{parse_blob_name, VLOG_ADOPT_NAME, VLOG_FILE_NAME, VLOG_NEW_NAME};
use crate::wal::Wal;
use crate::WAL_FILE_NAME;

/// Backup catalog magic (`pedradb-ops` `PDBCAT01`). Core only checks
/// magic + CRC32C trailer — catalog fields stay in ops.
const CATALOG_MAGIC: &[u8; 8] = b"PDBCAT01";
/// WAL-archive segment magic (`pedradb-ops` `PDBWAR01`).
const WARCH_MAGIC: &[u8; 8] = b"PDBWAR01";
/// Compat CF registry (`rocksdb-compat` `CFREG`).
const CFREG_FILE: &str = "CFREG";
const CFREG_MAGIC: &[u8] = b"COMPATCF1\n";

/// One file the scrub could not decode or whose CRC failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyFailure {
    /// File name relative to the DB directory.
    pub file: String,
    /// Byte offset of the bad record/trailer when known (else 0).
    pub offset: u64,
    /// Engine error text (CRC mismatch, bad magic, truncated record, …).
    pub message: String,
}

/// Aggregate of one at-rest walk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerifyReport {
    /// Inventory files actually inspected (SST / vlog / WAL / MANIFEST).
    pub files: u64,
    /// SST data blocks + vlog records + WAL records + MANIFEST payloads.
    pub blocks: u64,
    /// Sum of inspected file sizes.
    pub bytes: u64,
    /// Number of [`Self::failures`].
    pub errors: u64,
    /// Per-file decode/CRC failures.
    pub failures: Vec<VerifyFailure>,
}

impl VerifyReport {
    /// Product stdout line: `files=… blocks=… bytes=… errors=…`.
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "files={} blocks={} bytes={} errors={}",
            self.files, self.blocks, self.bytes, self.errors
        )
    }

    /// True when the walk found no CRC/decode errors.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.errors == 0
    }

    fn fail(&mut self, file: &str, offset: u64, message: impl Into<String>) {
        self.errors = self.errors.saturating_add(1);
        self.failures.push(VerifyFailure {
            file: file.to_string(),
            offset,
            message: message.into(),
        });
    }
}

fn read_all<E: Env>(env: &E, path: &Path) -> Result<Vec<u8>, String> {
    let mut f = env.open_read(path).map_err(|e| format!("open: {e}"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| format!("read: {e}"))?;
    Ok(buf)
}

fn walk_vlog(buf: &[u8]) -> Result<u64, (u64, String)> {
    const MAGIC: &[u8; 8] = b"PDBVLOG1";
    if buf.len() < MAGIC.len() || &buf[..MAGIC.len()] != MAGIC {
        return Err((0, "bad vlog magic".into()));
    }
    let mut off = MAGIC.len();
    let mut n = 0u64;
    while off + 8 <= buf.len() {
        let rec_off = off as u64;
        let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let stored = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap());
        off += 8;
        if off.checked_add(len).is_none_or(|end| end > buf.len()) {
            return Err((rec_off, format!("truncated vlog record len={len}")));
        }
        let data = &buf[off..off + len];
        let computed = crc32c::crc32c(data);
        if !crate::wal::crc::crc_match_ok(stored, computed) {
            return Err((
                rec_off,
                format!("vlog crc mismatch stored={stored:#010x} computed={computed:#010x}"),
            ));
        }
        off += len;
        n = n.saturating_add(1);
    }
    if off < buf.len() {
        return Err((off as u64, "trailing truncated vlog record".into()));
    }
    Ok(n)
}

fn crc_offset(err: &CoreError) -> u64 {
    match err {
        CoreError::Crc { offset, .. } | CoreError::Truncated(offset) => *offset,
        _ => 0,
    }
}

fn is_nested_db<E: Env>(env: &E, path: &Path) -> bool {
    env.exists(&path.join(CURRENT_FILE)) || env.exists(&path.join(CHECKPOINT_META_FILE))
}

fn merge_nested(report: &mut VerifyReport, prefix: &str, nested: VerifyReport) {
    report.files = report.files.saturating_add(nested.files);
    report.blocks = report.blocks.saturating_add(nested.blocks);
    report.bytes = report.bytes.saturating_add(nested.bytes);
    report.errors = report.errors.saturating_add(nested.errors);
    for f in nested.failures {
        report.failures.push(VerifyFailure {
            file: format!("{prefix}/{}", f.file),
            offset: f.offset,
            message: f.message,
        });
    }
}

fn has_ext(name: &str, ext: &str) -> bool {
    name.rsplit('.')
        .next()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// RFC-0060 P2.3/P2.15: parse `CURRENT` the same way [`manifest::load`] does.
/// Empty (torn) is not a scrub error — open treats it as missing inventory.
/// Optional CRC line is checked against the named MANIFEST bytes.
fn check_current_pointer<E: Env>(env: &E, dir: &Path, bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "CURRENT is not UTF-8".to_string())?;
    if text.trim().is_empty() {
        return Ok(());
    }
    let (name, crc) = manifest::parse_current_pointer(text).map_err(|e| e.to_string())?;
    if !env.exists(&dir.join(&name)) {
        return Err(format!("CURRENT points to missing {name}"));
    }
    if let Some(expect) = crc {
        let man = read_all(env, &dir.join(&name))?;
        if !crate::wal::crc::crc_match_ok(expect, crc32c::crc32c(&man)) {
            return Err(format!("CURRENT crc mismatch for {name}"));
        }
    }
    Ok(())
}

/// Magic + CRC32C trailer of the payload (same layout as CATALOG / warch).
fn check_magic_crc_trailer(bytes: &[u8], magic: &[u8; 8]) -> Result<(), String> {
    if bytes.len() < magic.len() + 4 {
        return Err("too short for magic+crc".into());
    }
    if !bytes.starts_with(magic) {
        return Err("bad magic".into());
    }
    let crc_off = bytes.len() - 4;
    let stored = u32::from_le_bytes(
        bytes[crc_off..]
            .try_into()
            .map_err(|_| "crc trailer".to_string())?,
    );
    let computed = crc32c::crc32c(&bytes[..crc_off]);
    if !crate::wal::crc::crc_match_ok(stored, computed) {
        return Err(format!(
            "crc mismatch stored={stored:#010x} computed={computed:#010x}"
        ));
    }
    Ok(())
}

/// RFC-0060 P2.22: `CORRUPTLOG` is append-only TSV (`ts\tkind\toffset`),
/// not CRC-covered. Parse-walk so garbage is a named scrub error.
fn check_corruptlog(bytes: &[u8]) -> Result<u64, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "CORRUPTLOG is not UTF-8".to_string())?;
    let mut n = 0u64;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let ts = parts.next().unwrap_or("");
        let kind = parts.next().unwrap_or("");
        let off = parts.next().unwrap_or("");
        if parts.next().is_some() {
            return Err(format!("line {i}: extra fields"));
        }
        if ts.parse::<u128>().is_err() {
            return Err(format!("line {i}: bad ts"));
        }
        if kind.is_empty() || kind.chars().any(char::is_whitespace) {
            return Err(format!("line {i}: bad kind"));
        }
        if off.parse::<u64>().is_err() {
            return Err(format!("line {i}: bad offset"));
        }
        n = n.saturating_add(1);
    }
    Ok(n)
}

/// RFC-0060 P2.26: `CFREG` = `COMPATCF1\n` + `R|P` + names, optional last
/// line `c:` + 8 hex CRC32C of the prefix (legacy without that line is ok).
fn check_cfreg(bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(CFREG_MAGIC) {
        return Err("CFREG bad magic".into());
    }
    let s = std::str::from_utf8(bytes).map_err(|_| "CFREG is not UTF-8".to_string())?;
    let trimmed = s.trim_end_matches('\n');
    let payload = if let Some((head, last)) = trimmed.rsplit_once('\n') {
        if let Some(hex) = last.strip_prefix("c:") {
            if hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                let expect =
                    u32::from_str_radix(hex, 16).map_err(|_| "CFREG crc not hex".to_string())?;
                let payload = bytes
                    .get(..head.len() + 1)
                    .ok_or_else(|| "CFREG crc payload".to_string())?;
                let got = crc32c::crc32c(payload);
                if !crate::wal::crc::crc_match_ok(expect, got) {
                    return Err(format!(
                        "CFREG crc mismatch stored={expect:#010x} computed={got:#010x}"
                    ));
                }
                payload
            } else {
                bytes
            }
        } else {
            bytes
        }
    } else {
        bytes
    };
    let rest = payload
        .get(CFREG_MAGIC.len()..)
        .ok_or_else(|| "CFREG truncated".to_string())?;
    let mut lines = rest.split(|&b| b == b'\n');
    match lines.next() {
        Some(b"R" | b"P") => {}
        _ => return Err("CFREG bad codec flag".into()),
    }
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if std::str::from_utf8(line).is_err() {
            return Err("CFREG name not UTF-8".into());
        }
    }
    Ok(())
}

/// Walk every CRC-covered inventory file under `dir`.
///
/// Does not take the DB lock and does not open a writer — safe on a closed
/// DB, and the function the CLI binary calls.
#[must_use]
pub fn verify_at_rest<E: Env>(env: &E, dir: impl AsRef<Path>) -> VerifyReport {
    let dir = dir.as_ref();
    let mut report = VerifyReport::default();
    let mut names = match env.read_dir_names(dir) {
        Ok(n) => n,
        Err(e) => {
            report.fail(".", 0, format!("cannot list db directory: {e}"));
            return report;
        }
    };
    names.sort();
    for name in names {
        if name == LOCK_FILE || name.starts_with('.') {
            continue;
        }
        let path = dir.join(&name);
        if env.is_dir(&path).unwrap_or(false) {
            if name == "history" {
                walk_history_dir(env, &path, &mut report);
            } else if is_nested_db(env, &path) {
                merge_nested(&mut report, &name, verify_at_rest(env, &path));
            } else if name == "wal" {
                walk_warch_dir(env, &path, &mut report);
            }
            continue;
        }
        if has_ext(&name, "tmp") {
            // RFC-0060 P2.16: leftover install temps are crash debris.
            // Open GCs them; the scrub names them so `pedra verify` is honest.
            report.files = report.files.saturating_add(1);
            report.fail(
                &name,
                0,
                "leftover install temp (not inventory; open GCs these)",
            );
            continue;
        }
        if name == VLOG_ADOPT_NAME {
            // RFC-0060 P2.25: leftover adopt marker from mid-GC crash.
            report.files = report.files.saturating_add(1);
            report.fail(
                &name,
                0,
                "leftover vlog adopt marker (not inventory; open GCs these)",
            );
            continue;
        }
        let bytes = match read_all(env, &path) {
            Ok(b) => b,
            Err(e) => {
                report.files = report.files.saturating_add(1);
                report.fail(&name, 0, e);
                continue;
            }
        };
        report.bytes = report.bytes.saturating_add(bytes.len() as u64);

        if name == CURRENT_FILE {
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(msg) = check_current_pointer(env, dir, &bytes) {
                report.fail(CURRENT_FILE, 0, msg);
            }
        } else if has_ext(&name, "sst") {
            report.files = report.files.saturating_add(1);
            match SstTable::open_on(env, &path) {
                Ok(t) => {
                    report.blocks = report.blocks.saturating_add(t.data_block_count() as u64);
                }
                Err(e) => {
                    let off = bytes.len().saturating_sub(4) as u64;
                    report.fail(&name, off, e.to_string());
                }
            }
        } else if name == VLOG_FILE_NAME
            || name == VLOG_NEW_NAME
            || parse_blob_name(&name).is_some()
        {
            report.files = report.files.saturating_add(1);
            match walk_vlog(&bytes) {
                Ok(n) => report.blocks = report.blocks.saturating_add(n),
                Err((off, msg)) => report.fail(&name, off, msg),
            }
        } else if name == WAL_FILE_NAME {
            report.files = report.files.saturating_add(1);
            match Wal::recover_on(env, &path) {
                Ok(recs) => report.blocks = report.blocks.saturating_add(recs.len() as u64),
                Err(e) => report.fail(&name, crc_offset(&e), e.to_string()),
            }
        } else if name.starts_with(MANIFEST_PREFIX) {
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(e) = manifest::decode(&bytes) {
                let off = bytes.len().saturating_sub(4) as u64;
                report.fail(&name, off, e.to_string());
            }
        } else if name == CHANGELOG_FILE_NAME || name == CHANGELOG_CORRUPT_FILE_NAME {
            // RFC-0085 P1.1: same `decode_changelog` / `crc_match_ok` as the
            // live cache path. Cache (F33): open quarantines poison to
            // CHANGELOG.corrupt and rebuilds from WAL. Scrub CRC-walks both
            // so `pedra verify` names a rotten live cache *and* a leftover
            // quarantine (P2.6 / P2.24).
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(e) = decode_changelog(&bytes) {
                let off = bytes.len().saturating_sub(4) as u64;
                report.fail(&name, off, e.to_string());
            }
        } else if name == CHECKPOINT_META_FILE {
            // RFC-0084 P1.2: same `read_checkpoint_meta` / `crc_match_ok` as
            // the live restore path. Present only in checkpoint destinations
            // (and copies). Live DBs have none — missing is not a scrub error.
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(e) = read_checkpoint_meta(env, dir) {
                let off = bytes.len().saturating_sub(4) as u64;
                report.fail(&name, off, e.to_string());
            }
        } else if name == "CATALOG" {
            // RFC-0060 P2.17: ops-layer backup catalog; magic+CRC only.
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(msg) = check_magic_crc_trailer(&bytes, CATALOG_MAGIC) {
                let off = bytes.len().saturating_sub(4) as u64;
                report.fail(&name, off, format!("CATALOG {msg}"));
            }
        } else if name == CORRUPTLOG_NAME {
            // RFC-0060 P2.22: journal is TSV (no CRC); parse-walk so garbage
            // is a named FAIL, not a silent skip.
            report.files = report.files.saturating_add(1);
            match check_corruptlog(&bytes) {
                Ok(n) => report.blocks = report.blocks.saturating_add(n),
                Err(msg) => report.fail(CORRUPTLOG_NAME, 0, msg),
            }
        } else if name == CFREG_FILE {
            // RFC-0060 P2.26: compat CF registry; magic + optional CRC line.
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(msg) = check_cfreg(&bytes) {
                report.fail(CFREG_FILE, 0, msg);
            }
        } else {
            // RFC-0060 P2.27: leftover unknown files are named, not skipped.
            report.files = report.files.saturating_add(1);
            report.fail(&name, 0, "unrecognized file (not inventory)");
        }
    }
    report
}

/// RFC-0060 P2.18: CRC-walk `wal/*.warch` (`PDBWAR01` magic + CRC trailer).
fn walk_warch_dir<E: Env>(env: &E, wal: &Path, report: &mut VerifyReport) {
    let mut names = match env.read_dir_names(wal) {
        Ok(n) => n,
        Err(e) => {
            report.fail("wal", 0, format!("cannot list wal/: {e}"));
            return;
        }
    };
    names.sort();
    for name in names {
        if has_ext(&name, "tmp") {
            let rel = format!("wal/{name}");
            report.files = report.files.saturating_add(1);
            report.fail(
                &rel,
                0,
                "leftover install temp (not inventory; open GCs these)",
            );
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        if !name.ends_with(".warch") {
            let rel = format!("wal/{name}");
            report.files = report.files.saturating_add(1);
            report.fail(&rel, 0, "unrecognized file (not inventory)");
            continue;
        }
        let rel = format!("wal/{name}");
        let path = wal.join(&name);
        let bytes = match read_all(env, &path) {
            Ok(b) => b,
            Err(e) => {
                report.files = report.files.saturating_add(1);
                report.fail(&rel, 0, e);
                continue;
            }
        };
        report.bytes = report.bytes.saturating_add(bytes.len() as u64);
        report.files = report.files.saturating_add(1);
        report.blocks = report.blocks.saturating_add(1);
        if let Err(msg) = check_magic_crc_trailer(&bytes, WARCH_MAGIC) {
            let off = bytes.len().saturating_sub(4) as u64;
            report.fail(&rel, off, format!("warch {msg}"));
        }
    }
}

/// RFC-0060 P2.4: CRC-walk `history/MANIFEST`, `seg-*.hist`, `seg-*.bloom`.
fn walk_history_dir<E: Env>(env: &E, hist: &Path, report: &mut VerifyReport) {
    let mut names = match env.read_dir_names(hist) {
        Ok(n) => n,
        Err(e) => {
            report.fail("history", 0, format!("cannot list history/: {e}"));
            return;
        }
    };
    names.sort();
    for name in names {
        if name.starts_with('.') {
            continue;
        }
        if has_ext(&name, "tmp") {
            let rel = format!("history/{name}");
            report.files = report.files.saturating_add(1);
            report.fail(
                &rel,
                0,
                "leftover install temp (not inventory; open GCs these)",
            );
            continue;
        }
        let rel = format!("history/{name}");
        let path = hist.join(&name);
        if env.is_dir(&path).unwrap_or(false) {
            continue;
        }
        let bytes = match read_all(env, &path) {
            Ok(b) => b,
            Err(e) => {
                report.files = report.files.saturating_add(1);
                report.fail(&rel, 0, e);
                continue;
            }
        };
        report.bytes = report.bytes.saturating_add(bytes.len() as u64);
        if name == "MANIFEST" {
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(e) = crate::history::verify_history_manifest(&bytes) {
                let off = bytes.len().saturating_sub(4) as u64;
                report.fail(&rel, off, e.to_string());
            }
        } else if name.starts_with("seg-") && name.ends_with(".hist") {
            report.files = report.files.saturating_add(1);
            match crate::history::walk_segment_records(&bytes) {
                Ok(recs) => report.blocks = report.blocks.saturating_add(recs.len() as u64),
                Err(e) => report.fail(&rel, 0, e.to_string()),
            }
        } else if name.ends_with(".bloom") {
            report.files = report.files.saturating_add(1);
            report.blocks = report.blocks.saturating_add(1);
            if let Err(e) = crate::history::verify_bloom_sidecar(&bytes) {
                let off = bytes.len().saturating_sub(4) as u64;
                report.fail(&rel, off, e.to_string());
            }
        } else {
            report.files = report.files.saturating_add(1);
            report.fail(&rel, 0, "unrecognized file (not inventory)");
        }
    }
}

fn collect_durable_relpaths<E: Env>(env: &E, dir: &Path) -> Vec<String> {
    let Ok(top) = env.read_dir_names(dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for n in top {
        if has_ext(&n, "sst")
            || n == WAL_FILE_NAME
            || n == VLOG_FILE_NAME
            || n == VLOG_NEW_NAME
            || parse_blob_name(&n).is_some()
            || n == CURRENT_FILE
            || n == CHANGELOG_FILE_NAME
            || n == CHECKPOINT_META_FILE
            || n == CFREG_FILE
            || (n.starts_with(MANIFEST_PREFIX) && !has_ext(&n, "tmp"))
        {
            names.push(n);
        } else if n == "history" && env.is_dir(&dir.join("history")).unwrap_or(false) {
            if let Ok(hs) = env.read_dir_names(&dir.join("history")) {
                for h in hs {
                    if h == "MANIFEST"
                        || (h.starts_with("seg-") && h.ends_with(".hist"))
                        || h.ends_with(".bloom")
                    {
                        names.push(format!("history/{h}"));
                    }
                }
            }
        }
    }
    names.sort();
    names
}

/// Deterministically pick a durable inventory file and XOR `n_bits` of it.
///
/// Inventory is top-level SST/WAL/vlog, `CURRENT` / `MANIFEST-*` /
/// `CHANGELOG` / `CHECKPOINT` (RFC-0060 P2.20), **and** `history/` (P2.5).
/// `apply = false` selects the same file/offset but does not write — the
/// non-vacuity mutant (RFC-0060 P1.2): a skip must leave [`verify_at_rest`]
/// clean, proving the `errors>0` check is not vacuously true.
#[must_use]
pub fn xor_durable_bits<E: Env>(
    env: &E,
    dir: impl AsRef<Path>,
    seed: u64,
    n_bits: u32,
    apply: bool,
) -> Option<VerifyFailure> {
    if n_bits == 0 {
        return None;
    }
    let dir = dir.as_ref();
    let names = collect_durable_relpaths(env, dir);
    if names.is_empty() {
        return None;
    }
    let idx = usize::try_from(seed % names.len() as u64).unwrap_or(0);
    let name = names[idx].clone();
    let path = dir.join(&name);
    let mut buf = read_all(env, &path).ok()?;
    if buf.len() < 16 {
        return None;
    }
    let span = buf.len() - 8;
    let mut first_off = 0u64;
    for i in 0..n_bits {
        let mixed = seed.wrapping_add(u64::from(i)) % span as u64;
        let off = 8 + usize::try_from(mixed).unwrap_or(0);
        if i == 0 {
            first_off = off as u64;
        }
        if apply {
            buf[off] ^= 1;
        }
    }
    if apply {
        let mut f = env.create(&path).ok()?;
        f.write_all(&buf).ok()?;
        let _ = f.sync_all();
    }
    Some(VerifyFailure {
        file: name,
        offset: first_off,
        message: if apply {
            format!("xor {n_bits} bit(s)")
        } else {
            "xor skipped (mutant)".into()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, OpenOptions};
    use crate::StdEnv;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-verify-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn seed_opts() -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: Some(32),
        }
    }

    fn seed_closed_db(dir: &std::path::Path) {
        let mut db = Db::open_with(dir, seed_opts()).unwrap();
        db.put(b"k", b"small").unwrap();
        db.put(b"big", vec![0xAB; 128]).unwrap();
        db.flush().unwrap();
        db.close().unwrap();
    }

    #[test]
    fn verify_clean_db_reports_zero_errors() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(
            r.is_clean(),
            "clean db must report errors=0: {} {:?}",
            r.summary_line(),
            r.failures
        );
        assert!(r.files >= 3, "expected SST+WAL+MANIFEST, got {}", r.files);
        assert!(r.bytes > 0, "bytes=0");
        assert!(r.blocks >= 1, "blocks=0");
        let line = r.summary_line();
        assert!(line.contains("errors=0"), "{line}");
        assert!(line.contains("files="), "{line}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_flags_corrupted_sst_block() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let sst = fs::read_dir(&dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "sst"))
            .expect("sst");
        let mut bytes = fs::read(&sst).unwrap();
        assert!(bytes.len() > 20);
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        fs::write(&sst, &bytes).unwrap();

        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "flipped SST must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file.ends_with(".sst")),
            "must name the SST file, got {:?}",
            r.failures
        );

        match Db::open(&dir) {
            Err(_) => {}
            Ok(db) => {
                assert!(
                    db.verify_checksums().is_err(),
                    "read path must fail-closed on the flipped SST"
                );
                db.close().unwrap();
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_flags_corrupted_vlog_record() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let vlog = dir.join(VLOG_FILE_NAME);
        assert!(vlog.exists(), "seed must spill a vlog record");
        let mut bytes = fs::read(&vlog).unwrap();
        assert!(bytes.len() > 16);
        bytes[12] ^= 0xff;
        fs::write(&vlog, &bytes).unwrap();

        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "flipped vlog must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file == VLOG_FILE_NAME),
            "must name VALUES.vlog, got {:?}",
            r.failures
        );

        match Db::open(&dir) {
            Err(_) => {}
            Ok(db) => {
                let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| db.get(b"big")));
                match got {
                    Ok(None) | Err(_) => {}
                    Ok(Some(v)) => {
                        assert_ne!(
                            v.as_ref(),
                            vec![0xAB; 128].as_slice(),
                            "must not serve the preimage from a CRC-failed vlog record"
                        );
                    }
                }
                db.close().unwrap();
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.3: garbage in `CURRENT` is a named scrub error (no CRC
    /// on the pointer — parse + target-exists only).
    #[test]
    fn verify_flags_garbage_current() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        fs::write(dir.join(CURRENT_FILE), b"not-a-manifest\n").unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "garbage CURRENT must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file == CURRENT_FILE),
            "must name CURRENT, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.15: CURRENT CRC line that does not match the MANIFEST.
    #[test]
    fn verify_flags_current_crc_mismatch() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let body = fs::read(dir.join(CURRENT_FILE)).unwrap();
        let (name, _) =
            crate::manifest::parse_current_pointer(&String::from_utf8(body).unwrap()).unwrap();
        fs::write(
            dir.join(CURRENT_FILE),
            format!("{name}\nffffffff\n").as_bytes(),
        )
        .unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "CURRENT crc mismatch must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == CURRENT_FILE && f.message.contains("crc")),
            "must name CURRENT crc mismatch, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.3: `CURRENT` naming a MANIFEST that is not on disk.
    #[test]
    fn verify_flags_dangling_current() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        fs::write(dir.join(CURRENT_FILE), b"MANIFEST-999999\n").unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "dangling CURRENT must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == CURRENT_FILE && f.message.contains("missing")),
            "must name CURRENT + missing target, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.6: CHANGELOG is a WAL-rebuild cache (F33: open still
    /// works) but the scrub names a CRC miss so `pedra verify` is honest.
    #[test]
    fn verify_flags_corrupted_changelog() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let path = dir.join(CHANGELOG_FILE_NAME);
        let mut bytes = if path.exists() {
            fs::read(&path).unwrap()
        } else {
            let mut b = b"PDBCHLG1".to_vec();
            b.extend_from_slice(&0u32.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            b
        };
        assert!(bytes.len() >= 4);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "flipped CHANGELOG must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file == CHANGELOG_FILE_NAME),
            "must name CHANGELOG, got {:?}",
            r.failures
        );
        let db = Db::open(&dir).expect("F33: poison CHANGELOG must not brick open");
        assert_eq!(db.get(b"k").as_deref(), Some(&b"small"[..]));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0085 P1.1: same CHANGELOG trailer lie as P0; `verify_at_rest`
    /// names the file and `crc mismatch`. F33 open still Ok. AS-IS
    /// `crc_match_ok` would scrub clean.
    #[test]
    fn crc_mismatch_on_live_changelog_verify_at_rest_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any changelog crc would match"
        );
        let dir = temp_dir();
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
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "trailer lie must fail the scrub");
        assert!(
            r.failures.iter().any(|f| {
                f.file == CHANGELOG_FILE_NAME
                    && f.message.to_ascii_lowercase().contains("crc mismatch")
            }),
            "must name CHANGELOG crc mismatch, got {:?}",
            r.failures
        );
        let db = Db::open(&dir).expect("F33: trailer lie must not brick open");
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.7: `CHECKPOINT` CRC is walked when the file is present
    /// (checkpoint dest). A live DB without it stays clean.
    #[test]
    fn verify_flags_corrupted_checkpoint_meta() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let ckpt = dir.parent().unwrap().join(format!(
            "{}-ckpt",
            dir.file_name().unwrap().to_string_lossy()
        ));
        let _ = fs::remove_dir_all(&ckpt);
        {
            let mut db = Db::open(&dir).unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let clean = verify_at_rest(&StdEnv, &ckpt);
        assert!(
            clean.is_clean(),
            "fresh checkpoint must be clean: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        let meta = ckpt.join(CHECKPOINT_META_FILE);
        let mut bytes = fs::read(&meta).unwrap();
        assert!(bytes.len() >= 4);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&meta, &bytes).unwrap();
        let r = verify_at_rest(&StdEnv, &ckpt);
        assert!(!r.is_clean(), "flipped CHECKPOINT must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file == CHECKPOINT_META_FILE),
            "must name CHECKPOINT, got {:?}",
            r.failures
        );
        let live = verify_at_rest(&StdEnv, &dir);
        assert!(
            live.is_clean(),
            "live DB without CHECKPOINT must stay clean: {:?}",
            live.failures
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    /// RFC-0084 P1.2: same CHECKPOINT trailer lie as P0; `verify_at_rest`
    /// names the file. AS-IS `crc_match_ok` would scrub clean.
    #[test]
    fn crc_mismatch_on_live_checkpoint_verify_at_rest_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any checkpoint crc would match"
        );
        let dir = temp_dir();
        seed_closed_db(&dir);
        let ckpt = dir.parent().unwrap().join(format!(
            "{}-ckpt-0084",
            dir.file_name().unwrap().to_string_lossy()
        ));
        let _ = fs::remove_dir_all(&ckpt);
        {
            let mut db = Db::open(&dir).unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let meta = ckpt.join(CHECKPOINT_META_FILE);
        let mut bytes = fs::read(&meta).unwrap();
        assert!(bytes.len() >= 12, "CHECKPOINT must have payload + trailer");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&meta, &bytes).unwrap();
        let r = verify_at_rest(&StdEnv, &ckpt);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
        assert!(!r.is_clean(), "trailer lie must fail the scrub");
        assert!(
            r.failures.iter().any(|f| {
                f.file == CHECKPOINT_META_FILE
                    && f.message.to_ascii_lowercase().contains("crc mismatch")
            }),
            "must name CHECKPOINT crc mismatch, got {:?}",
            r.failures
        );
    }

    /// RFC-0084 P2.2: backup `CATALOG` CRC stays RFC-0060 (`check_magic_crc_trailer`
    /// / ops `crc_mismatch_on_live_ops_catalog_is_not_ok`). Not this RFC.
    #[test]
    fn backup_catalog_crc_stays_rfc0060() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let mut cat = CATALOG_MAGIC.to_vec();
        cat.extend_from_slice(&[0u8; 24]);
        let crc = crc32c::crc32c(&cat);
        cat.extend_from_slice(&crc.to_le_bytes());
        let last = cat.len() - 1;
        cat[last] ^= 0xff;
        fs::write(dir.join("CATALOG"), &cat).unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        let _ = fs::remove_dir_all(&dir);
        assert!(!r.is_clean(), "CATALOG trailer lie must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == "CATALOG"
                    && f.message.to_ascii_lowercase().contains("crc mismatch")),
            "RFC-0060 walk must name CATALOG crc, got {:?}",
            r.failures
        );
    }

    /// RFC-0060 P2.13: a backup-root-shaped directory (`base-*` with
    /// CURRENT/CHECKPOINT) is walked as a nested DB.
    #[test]
    fn verify_walks_nested_base_checkpoint() {
        let parent = temp_dir();
        let dbdir = parent.join("db");
        fs::create_dir_all(&dbdir).unwrap();
        seed_closed_db(&dbdir);
        let ckpt = parent.join("base-000001");
        {
            let mut db = Db::open(&dbdir).unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let clean = verify_at_rest(&StdEnv, &parent);
        assert!(
            clean.is_clean(),
            "parent with nested checkpoint must be clean: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        let meta = ckpt.join(CHECKPOINT_META_FILE);
        let mut bytes = fs::read(&meta).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&meta, &bytes).unwrap();
        let r = verify_at_rest(&StdEnv, &parent);
        assert!(!r.is_clean(), "nested CHECKPOINT poison must surface");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == "base-000001/CHECKPOINT"),
            "must prefix nested path, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&parent);
    }

    fn encode_magic_crc(magic: &[u8; 8], rest: &[u8]) -> Vec<u8> {
        let mut b = Vec::with_capacity(magic.len() + rest.len() + 4);
        b.extend_from_slice(magic);
        b.extend_from_slice(rest);
        let crc = crc32c::crc32c(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }

    /// RFC-0060 P2.22: garbage in `CORRUPTLOG` is a named scrub error.
    #[test]
    fn verify_flags_garbage_corruptlog() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        fs::write(dir.join(CORRUPTLOG_NAME), b"1\tcrc\t0\n").unwrap();
        let clean = verify_at_rest(&StdEnv, &dir);
        assert!(
            clean.is_clean(),
            "valid CORRUPTLOG must be clean: {:?}",
            clean.failures
        );
        fs::write(dir.join(CORRUPTLOG_NAME), b"not-a-journal\n").unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "garbage CORRUPTLOG must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file == CORRUPTLOG_NAME),
            "must name CORRUPTLOG, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn encode_cfreg(body: &[u8]) -> Vec<u8> {
        let mut b = CFREG_MAGIC.to_vec();
        b.extend_from_slice(body);
        let crc = crc32c::crc32c(&b);
        b.extend_from_slice(format!("c:{crc:08x}\n").as_bytes());
        b
    }

    /// RFC-0060 P2.26: poison `CFREG` CRC is a named scrub error; legacy
    /// one-line registry without CRC stays clean.
    #[test]
    fn verify_flags_corrupted_cfreg() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let cat = encode_cfreg(b"P\nwrite\n");
        fs::write(dir.join(CFREG_FILE), &cat).unwrap();
        let clean = verify_at_rest(&StdEnv, &dir);
        assert!(
            clean.is_clean(),
            "intact CFREG must be clean: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        let mut poison = cat;
        let last = poison.len() - 2;
        poison[last] = if poison[last] == b'0' { b'1' } else { b'0' };
        fs::write(dir.join(CFREG_FILE), &poison).unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "flipped CFREG crc must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == CFREG_FILE && f.message.contains("crc")),
            "must name CFREG crc, got {:?}",
            r.failures
        );
        fs::write(dir.join(CFREG_FILE), b"COMPATCF1\nR\n").unwrap();
        let legacy = verify_at_rest(&StdEnv, &dir);
        assert!(
            legacy.is_clean(),
            "legacy CFREG without crc must stay clean: {:?}",
            legacy.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.24: F33 quarantine `CHANGELOG.corrupt` is CRC-walked.
    #[test]
    fn verify_flags_corrupted_changelog_quarantine() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let mut b = b"PDBCHLG1".to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        let last = b.len() - 1;
        b[last] ^= 0xff;
        fs::write(dir.join(CHANGELOG_CORRUPT_FILE_NAME), &b).unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(
            !r.is_clean(),
            "poison CHANGELOG.corrupt must fail the scrub"
        );
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == CHANGELOG_CORRUPT_FILE_NAME),
            "must name CHANGELOG.corrupt, got {:?}",
            r.failures
        );
        let db = Db::open(&dir).expect("F33: quarantine file must not brick open");
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.25: leftover `VALUES.vlog.adopt` is a named FAIL.
    #[test]
    fn verify_flags_leftover_vlog_adopt() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        fs::write(dir.join(VLOG_ADOPT_NAME), b"legacy").unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "leftover adopt marker must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == VLOG_ADOPT_NAME && f.message.contains("adopt")),
            "must name VALUES.vlog.adopt, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.27: leftover unknown files are named FAIL; dotfiles and
    /// LOCK stay silent.
    #[test]
    fn verify_flags_unrecognized_file() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        fs::write(dir.join(".DS_Store"), b"mac").unwrap();
        let with_dot = verify_at_rest(&StdEnv, &dir);
        assert!(
            with_dot.is_clean(),
            "dotfiles must not fail the scrub: {:?}",
            with_dot.failures
        );
        fs::write(dir.join("junk.dat"), b"??").unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "unrecognized junk.dat must fail");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == "junk.dat" && f.message.contains("unrecognized")),
            "must name junk.dat, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.16: leftover `CURRENT.tmp` is a named FAIL (open GCs it).
    #[test]
    fn verify_flags_leftover_install_temp() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let clean = verify_at_rest(&StdEnv, &dir);
        assert!(
            clean.is_clean(),
            "closed db must be clean before temp: {:?}",
            clean.failures
        );
        fs::write(dir.join("CURRENT.tmp"), b"torn pointer").unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "leftover CURRENT.tmp must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == "CURRENT.tmp" && f.message.contains("leftover")),
            "must name CURRENT.tmp, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.17: `CATALOG` magic+CRC is walked (backup root).
    #[test]
    fn verify_flags_corrupted_catalog() {
        let parent = temp_dir();
        let dbdir = parent.join("db");
        fs::create_dir_all(&dbdir).unwrap();
        seed_closed_db(&dbdir);
        let ckpt = parent.join("base-000001");
        {
            let mut db = Db::open(&dbdir).unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let cat = encode_magic_crc(CATALOG_MAGIC, &[0u8; 24]);
        fs::write(parent.join("CATALOG"), &cat).unwrap();
        let clean = verify_at_rest(&StdEnv, &parent);
        assert!(
            clean.is_clean(),
            "backup root with intact CATALOG must be clean: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        let mut poison = cat;
        let last = poison.len() - 1;
        poison[last] ^= 0xff;
        fs::write(parent.join("CATALOG"), &poison).unwrap();
        let r = verify_at_rest(&StdEnv, &parent);
        assert!(!r.is_clean(), "flipped CATALOG must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == "CATALOG" && f.message.contains("crc")),
            "must name CATALOG crc, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&parent);
    }

    /// RFC-0060 P2.18: `wal/*.warch` magic+CRC is walked (backup increment).
    #[test]
    fn verify_flags_corrupted_warch() {
        let parent = temp_dir();
        fs::create_dir_all(parent.join("wal")).unwrap();
        let body = encode_magic_crc(WARCH_MAGIC, &0u32.to_le_bytes());
        let path = parent.join("wal").join("000001.warch");
        fs::write(&path, &body).unwrap();
        let clean = verify_at_rest(&StdEnv, &parent);
        assert!(
            clean.is_clean(),
            "empty-record warch must be clean: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        let mut poison = body;
        let last = poison.len() - 1;
        poison[last] ^= 0xff;
        fs::write(&path, &poison).unwrap();
        let r = verify_at_rest(&StdEnv, &parent);
        assert!(!r.is_clean(), "flipped warch must fail the scrub");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == "wal/000001.warch" && f.message.contains("crc")),
            "must name wal/*.warch crc, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&parent);
    }

    /// RFC-0060 P2.19: `create_checkpoint` copies a two-line CURRENT
    /// pointer whose CRC matches the dest MANIFEST; dest verifies clean;
    /// flipping dest CRC names CURRENT.
    #[test]
    fn verify_checkpoint_copies_current_crc() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let ckpt = dir.parent().unwrap().join(format!(
            "{}-ckpt-crc",
            dir.file_name().unwrap().to_string_lossy()
        ));
        let _ = fs::remove_dir_all(&ckpt);
        {
            let mut db = Db::open(&dir).unwrap();
            db.create_checkpoint(&ckpt).unwrap();
            db.close().unwrap();
        }
        let dest = fs::read_to_string(ckpt.join(CURRENT_FILE)).unwrap();
        let (name, crc) = crate::manifest::parse_current_pointer(&dest).unwrap();
        let expect = crc.expect("checkpoint CURRENT must carry a CRC trailer");
        let man = fs::read(ckpt.join(&name)).unwrap();
        assert_eq!(
            crc32c::crc32c(&man),
            expect,
            "copied CURRENT crc must match dest {name}"
        );
        let clean = verify_at_rest(&StdEnv, &ckpt);
        assert!(
            clean.is_clean(),
            "checkpoint dest must verify: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        fs::write(
            ckpt.join(CURRENT_FILE),
            format!("{name}\nffffffff\n").as_bytes(),
        )
        .unwrap();
        let r = verify_at_rest(&StdEnv, &ckpt);
        assert!(!r.is_clean(), "flipped dest CURRENT crc must fail");
        assert!(
            r.failures
                .iter()
                .any(|f| f.file == CURRENT_FILE && f.message.contains("crc")),
            "must name CURRENT crc on dest, got {:?}",
            r.failures
        );
        let live = verify_at_rest(&StdEnv, &dir);
        assert!(
            live.is_clean(),
            "source DB must stay clean: {:?}",
            live.failures
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ckpt);
    }

    fn seed_history(dir: &std::path::Path) {
        let mut tier = crate::history::HistoryTier::open(&StdEnv, dir).unwrap();
        tier.archive_stream(
            &StdEnv,
            vec![(b"hk".to_vec(), vec![0xCD; 32], 1u64, 0u8)].into_iter(),
        )
        .unwrap();
    }

    /// RFC-0060 P2.4: a flipped history segment is a named scrub error.
    #[test]
    fn verify_flags_corrupted_history_segment() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        seed_history(&dir);
        let seg = fs::read_dir(dir.join("history"))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "hist"))
            .expect("hist");
        let mut bytes = fs::read(&seg).unwrap();
        assert!(bytes.len() > 8);
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        fs::write(&seg, &bytes).unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "flipped hist must fail the scrub");
        assert!(
            r.failures.iter().any(|f| f.file.contains(".hist")),
            "must name the hist file, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.4: history MANIFEST CRC trailer is walked.
    #[test]
    fn verify_flags_corrupted_history_manifest() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        seed_history(&dir);
        let man = dir.join("history").join("MANIFEST");
        let mut bytes = fs::read(&man).unwrap();
        assert!(bytes.len() > 8);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&man, &bytes).unwrap();
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(!r.is_clean(), "flipped history MANIFEST must fail");
        assert!(
            r.failures.iter().any(|f| f.file == "history/MANIFEST"),
            "must name history/MANIFEST, got {:?}",
            r.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_clean_db_with_history_reports_zero_errors() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        seed_history(&dir);
        let r = verify_at_rest(&StdEnv, &dir);
        assert!(
            r.is_clean(),
            "history-bearing db must be clean: {} {:?}",
            r.summary_line(),
            r.failures
        );
        assert!(
            r.files >= 4,
            "expected SST+WAL+MANIFEST+history, got {}",
            r.files
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.5: BitFlip inventory includes `history/seg-*.hist`.
    #[test]
    fn xor_durable_bits_can_flip_history_segment() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        seed_history(&dir);
        let names = collect_durable_relpaths(&StdEnv, &dir);
        let hist_idx = names
            .iter()
            .position(|n| n.contains(".hist"))
            .expect("history segment in inventory");
        let applied = xor_durable_bits(&StdEnv, &dir, hist_idx as u64, 1, true).expect("xor hist");
        assert!(
            applied.file.contains(".hist"),
            "seed must select the hist file, got {}",
            applied.file
        );
        let dirty = verify_at_rest(&StdEnv, &dir);
        assert!(
            !dirty.is_clean(),
            "flipped hist via xor_durable_bits must fail scrub: {:?}",
            dirty.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0060 P2.20: BitFlip inventory includes `CURRENT` (CRC pointer).
    #[test]
    fn xor_durable_bits_can_flip_current() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let names = collect_durable_relpaths(&StdEnv, &dir);
        let idx = names
            .iter()
            .position(|n| n == CURRENT_FILE)
            .expect("CURRENT in BitFlip inventory");
        let cur = fs::read(dir.join(CURRENT_FILE)).unwrap();
        assert!(
            cur.len() >= 16,
            "two-line CURRENT must be long enough to xor: {cur:?}"
        );
        let applied = xor_durable_bits(&StdEnv, &dir, idx as u64, 1, true).expect("xor CURRENT");
        assert_eq!(applied.file, CURRENT_FILE);
        let dirty = verify_at_rest(&StdEnv, &dir);
        assert!(
            !dirty.is_clean(),
            "flipped CURRENT via xor_durable_bits must fail scrub: {:?}",
            dirty.failures
        );
        assert!(
            dirty.failures.iter().any(|f| f.file == CURRENT_FILE),
            "must name CURRENT, got {:?}",
            dirty.failures
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn xor_durable_bits_mutant_is_not_vacuous() {
        let dir = temp_dir();
        seed_closed_db(&dir);
        let skipped = xor_durable_bits(&StdEnv, &dir, 0x60, 1, false);
        assert!(skipped.is_some());
        let clean = verify_at_rest(&StdEnv, &dir);
        assert!(
            clean.is_clean(),
            "unapplied flip must leave the DB clean: {:?}",
            clean.failures
        );
        let applied = xor_durable_bits(&StdEnv, &dir, 0x60, 1, true);
        assert!(applied.is_some());
        let dirty = verify_at_rest(&StdEnv, &dir);
        assert!(
            !dirty.is_clean(),
            "applied flip must be visible to the same scrub"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
