//! Directory layout classifier (RFC-0186).
//!
//! Pedra does **not** open a C++ Rocks SST. This module only peeks magics
//! and well-known filenames — it never decodes BlockBasedTable.

use std::io::Read;
use std::path::Path;

use pedradb_core::manifest::{self, CURRENT_FILE, MANIFEST_PREFIX};
// RFC-0186 P2.2: one admission predicate shared with `SstTable::decode`.
pub use pedradb_core::sst::sst_magic_is_pedra;
pub use pedradb_core::sst::sst_magic_is_pedra_as_is;
use pedradb_core::{Env, WAL_FILE_NAME};

use crate::{OpsError, Result};

/// On-disk layout of a directory we might be asked to open or copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirKind {
    /// No Pedra or Rocks markers (missing dir counts as empty).
    Empty,
    /// Pedra MANIFEST (`PDBM`) / SST (`PEDRSST\0`) / `CURRENT.log`.
    Pedra,
    /// C++ RocksDB markers (`IDENTITY`, `OPTIONS-*`, non-Pedra SST/MANIFEST).
    Rocks,
    /// Mixed or unrecognised markers — refuse both open-as-Pedra rewrite
    /// and migrate-from-rocks rather than guess.
    Unknown,
}

impl DirKind {
    /// Stable label for CLI / inspect.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Pedra => "pedra",
            Self::Rocks => "rocks",
            Self::Unknown => "unknown",
        }
    }
}

/// Operator-facing refusal: Pedra will not parse a C++ SST directory.
pub const NOT_DROPIN: &str = "C++ Rocks directory is not drop-in; Pedra does not open a C++ SST. Use `pedra migrate-from-rocks <src> <dst>`";

/// `migrate_from_rocks` source is admitted only when the dir is Rocks.
#[must_use]
pub fn rocks_source_admitted(kind: DirKind) -> bool {
    kind == DirKind::Rocks
}

/// AS-IS: any non-empty dir is a Rocks source (would feed a Pedra dir to
/// the C++ reader, or overwrite-copy Unknown).
#[must_use]
pub fn rocks_source_admitted_as_is(kind: DirKind) -> bool {
    kind != DirKind::Empty
}

/// Destination of a from-rocks copy must be empty (new Pedra dir).
#[must_use]
pub fn rocks_dest_admitted(kind: DirKind) -> bool {
    kind == DirKind::Empty
}

/// AS-IS: overwrite a live Pedra dest.
#[must_use]
pub fn rocks_dest_admitted_as_is(_kind: DirKind) -> bool {
    true
}

/// Classify `path` using the production Env default.
///
/// # Errors
/// Directory list / peek I/O.
pub fn classify_dir(path: impl AsRef<Path>) -> Result<DirKind> {
    classify_dir_env(&pedradb_io_uring::IoUringEnv::default(), path.as_ref())
}

/// Classify via `env`.
///
/// # Errors
/// Directory list / peek I/O.
pub fn classify_dir_env(env: &impl Env, path: &Path) -> Result<DirKind> {
    if !env.exists(path) {
        return Ok(DirKind::Empty);
    }
    let names = env.read_dir_names(path)?;
    if names.is_empty() {
        return Ok(DirKind::Empty);
    }

    let mut pedra = false;
    let mut rocks = false;

    for name in &names {
        if name == "IDENTITY" || name.starts_with("OPTIONS-") {
            rocks = true;
            continue;
        }
        if name == WAL_FILE_NAME {
            pedra = true;
            continue;
        }
        if name.ends_with(".ldb") {
            rocks = true;
            continue;
        }
        if manifest::parse_sst_name(name).is_some() {
            let hdr = peek_bytes(env, &path.join(name), 8)?;
            if sst_magic_is_pedra(&hdr) {
                pedra = true;
            } else if !hdr.is_empty() {
                rocks = true;
            }
            continue;
        }
        if let Some(rest) = name.strip_prefix(MANIFEST_PREFIX) {
            if rest.chars().all(|c| c.is_ascii_digit()) {
                let hdr = peek_bytes(env, &path.join(name), 4)?;
                if hdr == b"PDBM" {
                    pedra = true;
                } else if !hdr.is_empty() {
                    rocks = true;
                }
            }
        }
    }

    match (pedra, rocks) {
        (false, false) => {
            if names.iter().any(|n| n == CURRENT_FILE || n == "LOCK") {
                Ok(DirKind::Unknown)
            } else {
                Ok(DirKind::Empty)
            }
        }
        (true, false) => Ok(DirKind::Pedra),
        (false, true) => Ok(DirKind::Rocks),
        (true, true) => Ok(DirKind::Unknown),
    }
}

fn peek_bytes(env: &impl Env, path: &Path, n: usize) -> Result<Vec<u8>> {
    if !env.exists(path) {
        return Ok(Vec::new());
    }
    let mut f = env.open_read(path)?;
    let mut buf = vec![0u8; n];
    let got = f.read(&mut buf)?;
    buf.truncate(got);
    Ok(buf)
}

/// Error for a Rocks (or mixed) directory Pedra must not open as itself.
#[must_use]
pub fn not_dropin_err(path: &Path) -> OpsError {
    OpsError::Msg(format!("{}: {NOT_DROPIN}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::sst::SST_MAGIC;
    use pedradb_core::StdEnv;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-ops-kind-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// RFC-0186 P0.2: a C++ SST header is not Pedra. AS-IS would bless it.
    #[test]
    fn sst_magic_is_pedra_on_cpp_header_is_not_ok() {
        assert!(sst_magic_is_pedra(SST_MAGIC));
        let rocks_like = [0u8; 8];
        assert!(
            !sst_magic_is_pedra(&rocks_like),
            "Rocks BlockBasedTable does not start with PEDRSST"
        );
        assert!(
            sst_magic_is_pedra_as_is(&rocks_like),
            "AS-IS tooth: any header is Pedra (drop-in lie)"
        );
    }

    #[test]
    fn rocks_source_dest_gates() {
        assert!(rocks_source_admitted(DirKind::Rocks));
        assert!(!rocks_source_admitted(DirKind::Pedra));
        assert!(rocks_source_admitted_as_is(DirKind::Pedra));
        assert!(rocks_dest_admitted(DirKind::Empty));
        assert!(!rocks_dest_admitted(DirKind::Pedra));
        assert!(rocks_dest_admitted_as_is(DirKind::Pedra));
    }

    #[test]
    fn classify_synthetic_rocks_dir() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("IDENTITY"), b"not-a-uuid-but-rocks-marker").unwrap();
        std::fs::write(dir.join("CURRENT"), b"MANIFEST-000001\n").unwrap();
        std::fs::write(dir.join("MANIFEST-000001"), b"RLOG not PDBM").unwrap();
        std::fs::write(dir.join("000001.sst"), vec![0xABu8; 64]).unwrap();
        let kind = classify_dir_env(&StdEnv, &dir).unwrap();
        assert_eq!(kind, DirKind::Rocks);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classify_missing_is_empty() {
        let dir = temp();
        assert_eq!(classify_dir_env(&StdEnv, &dir).unwrap(), DirKind::Empty);
    }
}
