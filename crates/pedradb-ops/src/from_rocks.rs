//! Logical copy RocksDB → Pedra MANIFEST v5 (RFC-0186).
//!
//! The C++ reader is feature-gated. Pedra still never decodes a Rocks SST.

use std::io::Read;
use std::path::Path;
#[cfg(any(test, feature = "from-rocks"))]
use std::path::PathBuf;

#[cfg(feature = "from-rocks")]
use pedradb_core::sst::SST_VERSION;
#[cfg(feature = "from-rocks")]
use pedradb_core::{encode_cf_key, BatchOp, Db, OpenOptions, WriteOptions};
use pedradb_core::{Env, SequenceNumber};

use crate::dir_kind::{
    classify_dir_env, not_dropin_err, rocks_dest_admitted, rocks_source_admitted, DirKind,
};
#[cfg(feature = "from-rocks")]
use crate::inspect_format_env;
use crate::{OpsError, Result};

/// Result of copying a Rocks live snapshot into a new Pedra directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RocksMigrateReport {
    /// Visible keys written (iterator snapshot; deletes omitted).
    pub keys: u64,
    /// Sum of key+value bytes copied.
    pub bytes: u64,
    /// SST files in the dest after flush.
    pub ssts_written: usize,
    /// Pedra last sequence after flush.
    pub last_sequence: SequenceNumber,
    /// Max Pedra SST version on dest (v5 lz4; v3 raw when the lz4 probe
    /// finds the data incompressible — same production writer).
    pub sst_version: u32,
    /// Rocks CF names declared as physical Pedra CFs on dest (RFC-0065).
    pub cfs: Vec<String>,
    /// `verify_checksums` passed and dest inspect is Pedra.
    pub verified: bool,
}

#[cfg(not(feature = "from-rocks"))]
const FEATURE_HINT: &str = "migrate-from-rocks requires rebuilding with --features from-rocks (links RocksDB C++ as a reader only; Pedra still does not open a C++ SST directory)";

/// Ops grouped through [`pedradb_core::Db::apply_batch`] (RFC-0186 P1.2).
#[cfg(feature = "from-rocks")]
const APPLY_CHUNK: usize = 256;

/// OPTIONS `merge_operator=<name>` is set (not null / empty).
///
/// Rocks 8.10 serializes the operator `Name()`; a null pointer dumps
/// `nullptr` (`options/cf_options.cc` `kByNameAllowFromNull`).
#[must_use]
pub fn options_has_merge_operator(text: &str) -> bool {
    option_values(text, "merge_operator").any(value_is_named)
}

/// AS-IS: merge operands would be copied as plain values.
#[must_use]
pub fn options_has_merge_operator_as_is(_text: &str) -> bool {
    false
}

/// Titan / Rocks blob: `titan.` OPTIONS keys, `enable_blob_files=true`,
/// or a `*.blob` file in the directory.
#[must_use]
pub fn options_has_blob(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("titan.") {
        return true;
    }
    option_values(text, "enable_blob_files").any(|v| v.eq_ignore_ascii_case("true"))
}

/// Directory listing carries Titan/Rocks blob files (`*.blob`).
#[must_use]
pub fn dir_names_have_blob(names: &[String]) -> bool {
    names.iter().any(|n| {
        let n = n.as_str();
        n.ends_with(".blob") || n.ends_with(".blob.tmp")
    })
}

/// AS-IS: blob files would be copied as plain SST values.
#[must_use]
pub fn options_has_blob_as_is(_text: &str) -> bool {
    false
}

/// User-defined timestamps: Rocks comparator class name ends in `.u64ts`
/// (`util/udt_util.cc` `kUDTSuffix`). `persist_user_defined_timestamps=true`
/// is the default and is **not** a UDT marker.
#[must_use]
pub fn options_has_user_timestamps(text: &str) -> bool {
    text.contains(".u64ts")
}

/// AS-IS: UDT key+ts bytes would be copied as the user key.
#[must_use]
pub fn options_has_user_timestamps_as_is(_text: &str) -> bool {
    false
}

/// Wide-column source. Rocks 8.10 `PutEntity` has **no** OPTIONS token;
/// we refuse a `wide_column` / `wide-column` marker when present (synthetic
/// tests + any future dump). Live `PutEntity` without that marker is a
/// published gap (RFC-0186 P1.3).
#[must_use]
pub fn options_has_wide_column(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("wide_column") || lower.contains("wide-column")
}

/// AS-IS: entity values would be copied as opaque bytes.
#[must_use]
pub fn options_has_wide_column_as_is(_text: &str) -> bool {
    false
}

/// First unsupported Rocks feature in this OPTIONS dump + file list.
///
/// `None` = admitted (P1.3 does not refuse).
#[must_use]
pub fn unsupported_rocks_feature(options_text: &str, names: &[String]) -> Option<&'static str> {
    if options_has_merge_operator(options_text) {
        return Some("merge operator");
    }
    if options_has_blob(options_text) || dir_names_have_blob(names) {
        return Some("blob");
    }
    if options_has_wide_column(options_text) {
        return Some("wide-column");
    }
    if options_has_user_timestamps(options_text) {
        return Some("user timestamps");
    }
    None
}

/// Scan a Rocks directory's OPTIONS files + names; refuse merge / blob /
/// wide-column / user-timestamps (RFC-0186 P1.3).
///
/// # Errors
/// Directory list / OPTIONS peek I/O, or an unsupported feature.
pub fn refuse_unsupported_rocks_source(env: &impl Env, path: &Path) -> Result<()> {
    let names = if env.exists(path) {
        env.read_dir_names(path)?
    } else {
        Vec::new()
    };
    let mut options = String::new();
    for name in &names {
        if name.starts_with("OPTIONS") {
            if let Ok(mut f) = env.open_read(&path.join(name)) {
                let mut s = String::new();
                if f.read_to_string(&mut s).is_ok() {
                    options.push_str(&s);
                    options.push('\n');
                }
            }
        }
    }
    if let Some(feat) = unsupported_rocks_feature(&options, &names) {
        return Err(OpsError::Msg(format!(
            "{}: refuses {feat} (RFC-0186 P1.3; merge operands / blobs / wide-columns / user timestamps are not copied as plain values)",
            path.display()
        )));
    }
    Ok(())
}

fn option_values<'a>(text: &'a str, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    text.lines().filter_map(move |line| {
        let line = line.trim().trim_end_matches(';').trim();
        let rest = line.strip_prefix(key)?.strip_prefix('=')?;
        Some(rest.trim().trim_matches('"'))
    })
}

fn value_is_named(v: &str) -> bool {
    let v = v.trim();
    !v.is_empty()
        && !v.eq_ignore_ascii_case("nullptr")
        && !v.eq_ignore_ascii_case("null")
        && !v.eq_ignore_ascii_case("none")
}

/// Temp dir removed on drop (success and failure). RFC-0186 P2.1.
#[cfg(any(test, feature = "from-rocks"))]
struct CheckpointGuard {
    path: PathBuf,
}

#[cfg(any(test, feature = "from-rocks"))]
impl Drop for CheckpointGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Copy the visible snapshot of every CF in a Rocks directory into `dst`.
///
/// `dst` must be empty. Named CFs become physical Pedra CFs (RFC-0065):
/// `default` stays raw, others `cf\0key`, one MANIFEST/WAL lineage.
/// Merge / blob / wide-column / user-timestamp sources refuse (P1.3).
/// Crash mid-copy leaves `dst` incomplete; delete and retry. `Ok` means
/// dest MANIFEST is Pedra v5 and checksums verified.
///
/// # Errors
/// Source is not Rocks, dest is not empty, unsupported Rocks feature, I/O,
/// or (without the `from-rocks` feature) a rebuild hint.
pub fn migrate_from_rocks(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
) -> Result<RocksMigrateReport> {
    migrate_from_rocks_env(src, dst, pedradb_io_uring::IoUringEnv::default())
}

/// [`migrate_from_rocks`] with an injectible Env for the Pedra dest.
///
/// # Errors
/// Same as [`migrate_from_rocks`].
pub fn migrate_from_rocks_env(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    env: impl Env,
) -> Result<RocksMigrateReport> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    if src == dst {
        return Err(OpsError::Msg(
            "migrate-from-rocks: src and dst must be distinct directories".into(),
        ));
    }
    let src_kind = classify_dir_env(&env, src)?;
    if !rocks_source_admitted(src_kind) {
        return Err(if src_kind == DirKind::Pedra {
            OpsError::Msg(format!(
                "{} is a Pedra directory; `migrate-from-rocks` copies from Rocks C++. Use `pedra migrate` to rewrite Pedra SST in place",
                src.display()
            ))
        } else if src_kind == DirKind::Empty {
            OpsError::Msg(format!(
                "{} is empty (not a Rocks directory)",
                src.display()
            ))
        } else {
            not_dropin_err(src)
        });
    }
    refuse_unsupported_rocks_source(&env, src)?;
    let dst_kind = classify_dir_env(&env, dst)?;
    if !rocks_dest_admitted(dst_kind) {
        return Err(OpsError::Msg(format!(
            "{} is not empty (kind={}); migrate-from-rocks writes a new Pedra directory",
            dst.display(),
            dst_kind.as_str()
        )));
    }

    #[cfg(not(feature = "from-rocks"))]
    {
        let _ = env;
        Err(OpsError::Msg(FEATURE_HINT.into()))
    }

    #[cfg(feature = "from-rocks")]
    {
        copy_visible_all_cfs(src, dst, env)
    }
}

#[cfg(feature = "from-rocks")]
fn copy_visible_all_cfs(src: &Path, dst: &Path, env: impl Env) -> Result<RocksMigrateReport> {
    let ckpt = checkpoint_src(src)?;
    copy_from_rocks_dir(&ckpt.path, dst, env)
}

/// Create a Rocks checkpoint of `src`. Src is opened read-only so its
/// bytes stay put. The guard deletes the checkpoint on every exit path.
#[cfg(feature = "from-rocks")]
fn checkpoint_src(src: &Path) -> Result<CheckpointGuard> {
    use rocksdb::{checkpoint::Checkpoint, Options, DB};

    let path = unique_ckpt_path(src);
    if path.exists() {
        let _ = std::fs::remove_dir_all(&path);
    }
    let mut opts = Options::default();
    opts.create_if_missing(false);
    let cfs = DB::list_cf(&opts, src).map_err(|e| OpsError::Msg(format!("rocks list_cf: {e}")))?;
    let db = DB::open_cf_for_read_only(&opts, src, &cfs, false)
        .map_err(|e| OpsError::Msg(format!("rocks open_cf_for_read_only (checkpoint): {e}")))?;
    let made = Checkpoint::new(&db).and_then(|c| c.create_checkpoint(&path));
    drop(db);
    match made {
        Ok(()) => Ok(CheckpointGuard { path }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&path);
            Err(OpsError::Msg(format!("rocks create_checkpoint: {e}")))
        }
    }
}

#[cfg(feature = "from-rocks")]
fn unique_ckpt_path(src: &Path) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = src.file_name().and_then(|s| s.to_str()).unwrap_or("src");
    let parent = src.parent().filter(|p| !p.as_os_str().is_empty());
    let base = parent
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!(
        ".pedra-rocks-ckpt-{name}-{n}-{}",
        std::process::id()
    ))
}

/// Injected-failure helper: checkpoint then error. The guard still drops.
///
/// Used by the RFC-0186 P2.1 feature test; not an operator API.
#[cfg(feature = "from-rocks")]
pub fn checkpoint_then_err(src: &Path) -> Result<()> {
    let _guard = checkpoint_src(src)?;
    Err(OpsError::Msg(
        "injected failure after checkpoint (RFC-0186 P2.1)".into(),
    ))
}

#[cfg(feature = "from-rocks")]
fn copy_from_rocks_dir(src: &Path, dst: &Path, env: impl Env) -> Result<RocksMigrateReport> {
    use rocksdb::{IteratorMode, Options, DB};

    let mut opts = Options::default();
    opts.create_if_missing(false);
    let cfs = DB::list_cf(&opts, src).map_err(|e| OpsError::Msg(format!("rocks list_cf: {e}")))?;
    if cfs.is_empty() {
        return Err(OpsError::Msg(format!(
            "{} has no column families",
            src.display()
        )));
    }

    let rocks = DB::open_cf_for_read_only(&opts, src, &cfs, false)
        .map_err(|e| OpsError::Msg(format!("rocks open_cf_for_read_only: {e}")))?;

    let mut dest = Db::open_with_env(
        dst,
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: false,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
        env.clone(),
    )?;
    dest.set_physical_cfs(cfs.clone());
    dest.set_defer_auto_compact(true);
    let declared_cfs = cfs.clone();

    let mut keys = 0u64;
    let mut bytes = 0u64;
    let nosync = WriteOptions::no_sync();
    for name in &cfs {
        let handle = rocks
            .cf_handle(name)
            .ok_or_else(|| OpsError::Msg(format!("rocks cf_handle missing {name}")))?;
        let mut chunk: Vec<BatchOp> = Vec::with_capacity(APPLY_CHUNK);
        for item in rocks.iterator_cf(&handle, IteratorMode::Start) {
            let (k, v) =
                item.map_err(|e| OpsError::Msg(format!("rocks iterator_cf {name}: {e}")))?;
            bytes = bytes
                .saturating_add(k.len() as u64)
                .saturating_add(v.len() as u64);
            let encoded = encode_cf_key(name, k.as_ref(), true);
            chunk.push(BatchOp::put(encoded, v.as_ref()));
            keys = keys.saturating_add(1);
            if chunk.len() >= APPLY_CHUNK {
                dest.apply_batch_with(std::mem::take(&mut chunk), nosync)?;
            }
        }
        if !chunk.is_empty() {
            dest.apply_batch_with(chunk, nosync)?;
        }
    }
    dest.sync()?;
    dest.flush()?;
    dest.verify_checksums()?;
    let last_sequence = dest.last_sequence();
    let ssts_written = dest.sst_count();
    dest.close()?;

    let after = inspect_format_env(&env, dst)?;
    if after.kind != DirKind::Pedra && after.sst_count > 0 {
        return Err(OpsError::Msg(format!(
            "dest {} is not a Pedra directory after copy (kind={})",
            dst.display(),
            after.kind.as_str()
        )));
    }
    let sst_version = after
        .sst_versions
        .iter()
        .map(|(_, v)| *v)
        .max()
        .unwrap_or(SST_VERSION);
    if after.has_manifest && after.manifest_format != 5 {
        return Err(OpsError::Msg(format!(
            "dest MANIFEST format {} is not Pedra v5",
            after.manifest_format
        )));
    }
    let verified = after.kind == DirKind::Pedra || after.sst_count == 0;
    Ok(RocksMigrateReport {
        keys,
        bytes,
        ssts_written,
        last_sequence,
        sst_version,
        cfs: declared_cfs,
        verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::StdEnv;

    #[test]
    fn merge_operator_named_refuses_null_does_not() {
        assert!(options_has_merge_operator(
            "[CFOptions \"default\"]\n  merge_operator=uint64add;\n"
        ));
        assert!(options_has_merge_operator("merge_operator=rfc0186-merge"));
        assert!(!options_has_merge_operator(
            "[CFOptions \"default\"]\n  merge_operator=nullptr;\n"
        ));
        assert!(!options_has_merge_operator("merge_operator=null"));
        assert!(!options_has_merge_operator(
            "compaction_style=kCompactionStyleLevel"
        ));
        assert!(
            !options_has_merge_operator_as_is("merge_operator=uint64add"),
            "AS-IS tooth: merge operands copied as values"
        );
    }

    #[test]
    fn blob_titan_and_enable_blob_files_refuse() {
        assert!(options_has_blob("titan.min_blob_size=4096"));
        assert!(options_has_blob("enable_blob_files=true"));
        assert!(!options_has_blob("enable_blob_files=false"));
        assert!(dir_names_have_blob(&[
            "IDENTITY".into(),
            "000007.blob".into()
        ]));
        assert!(!dir_names_have_blob(&[
            "000001.sst".into(),
            "IDENTITY".into()
        ]));
        assert!(
            !options_has_blob_as_is("titan.min_blob_size=4096"),
            "AS-IS tooth: blob copied as values"
        );
    }

    #[test]
    fn wide_column_and_udt_markers_refuse() {
        assert!(options_has_wide_column("wide_column=true"));
        assert!(options_has_wide_column("# wide-column entity dump"));
        assert!(!options_has_wide_column(
            "compaction_pri=kMinOverlappingRatio"
        ));
        assert!(options_has_user_timestamps(
            "comparator=leveldb.BytewiseComparator.u64ts;"
        ));
        assert!(
            !options_has_user_timestamps("persist_user_defined_timestamps=true;"),
            "default persist_user_defined_timestamps is not a UDT marker"
        );
        assert!(
            !options_has_wide_column_as_is("wide_column=true"),
            "AS-IS tooth: entity copied as bytes"
        );
        assert!(
            !options_has_user_timestamps_as_is("comparator=x.u64ts"),
            "AS-IS tooth: ts suffix copied as key"
        );
    }

    #[test]
    fn unsupported_feature_names_the_refused_thing() {
        assert_eq!(
            unsupported_rocks_feature("merge_operator=x", &[]),
            Some("merge operator")
        );
        assert_eq!(
            unsupported_rocks_feature("", &["0001.blob".into()]),
            Some("blob")
        );
        assert_eq!(
            unsupported_rocks_feature("wide_column=1", &[]),
            Some("wide-column")
        );
        assert_eq!(
            unsupported_rocks_feature("comparator=c.u64ts", &[]),
            Some("user timestamps")
        );
        assert_eq!(
            unsupported_rocks_feature("merge_operator=nullptr", &[]),
            None
        );
    }

    #[test]
    fn refuse_unsupported_scans_options_file() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-ops-p13-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("IDENTITY"), b"rocks").unwrap();
        std::fs::write(
            dir.join("OPTIONS-000001"),
            b"[CFOptions \"default\"]\n  merge_operator=PutOperator;\n",
        )
        .unwrap();
        let err = refuse_unsupported_rocks_source(&StdEnv, &dir)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("merge operator"),
            "error must name the feature: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuse_unsupported_scans_blob_filename() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-ops-p13-blob-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("IDENTITY"), b"rocks").unwrap();
        std::fs::write(dir.join("000009.blob"), b"titan-blob").unwrap();
        let err = refuse_unsupported_rocks_source(&StdEnv, &dir)
            .unwrap_err()
            .to_string();
        assert!(err.contains("blob"), "error must name blob: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuse_unsupported_scans_wide_column_and_udt() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-ops-p13-wc-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("IDENTITY"), b"rocks").unwrap();
        std::fs::write(dir.join("OPTIONS-1"), b"wide_column=true\n").unwrap();
        let err = refuse_unsupported_rocks_source(&StdEnv, &dir)
            .unwrap_err()
            .to_string();
        assert!(err.contains("wide-column"), "{err}");
        std::fs::write(
            dir.join("OPTIONS-1"),
            b"comparator=leveldb.BytewiseComparator.u64ts;\n",
        )
        .unwrap();
        let err = refuse_unsupported_rocks_source(&StdEnv, &dir)
            .unwrap_err()
            .to_string();
        assert!(err.contains("user timestamps"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_guard_removes_dir_on_drop() {
        let dir = std::env::temp_dir().join(format!(
            "pedradb-ops-ckpt-guard-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x"), b"y").unwrap();
        {
            let _g = CheckpointGuard { path: dir.clone() };
            assert!(dir.exists());
        }
        assert!(!dir.exists(), "Drop must remove the checkpoint dir");
    }
}
