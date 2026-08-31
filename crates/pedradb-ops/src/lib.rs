//! PedraDB **ops suite**: local backup, point-in-time restore (PITR), format migration.
//!
//! # Layout under a backup root
//! ```text
//! backup_root/
//!   CATALOG          # next backup id, last shipped seq
//!   base-000001/     # full checkpoint (openable as a DB)
//!   wal/
//!     000001.warch   # logical WriteRecord stream after base (or after last ship)
//! ```
//!
//! # PITR contract
//! 1. [`BackupEngine::create_base_backup`] — flush+checkpoint into `base-NNNNNN`.
//! 2. After more durable writes (while still in WAL, before flush if you need
//!    those keys in the archive), [`BackupEngine::ship_wal`] archives complete
//!    WAL records with `max_sequence > last_shipped`.
//! 3. [`BackupEngine::restore_pitr`] copies the base, filters archived records
//!    to `sequence <= target`, writes them as `CURRENT.log`, then open recovers.
//!
//! This is **local** ops tooling (single-dir embed), not multi-region cluster
//! backup. Call `ship_wal` before flushes that would drop unarchived WAL data,
//! or take a new base backup after heavy flush.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pedradb_core::manifest::{self, VersionSet};
use pedradb_core::wal::Wal;
#[cfg(test)]
use pedradb_core::StdEnv;
use pedradb_core::{
    copy_db_directory, read_checkpoint_meta, verify_at_rest, CheckpointMeta, ConcurrentDb,
    CoreError, Db, Env, EnvFile, OpenOptions, SequenceNumber, WriteOp, WriteRecord, WAL_FILE_NAME,
};
use pedradb_io_uring::IoUringEnv;

/// Ops-layer error (wraps core + structured messages).
#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    /// Core engine error.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// I/O outside CoreError.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Catalog / backup integrity.
    #[error("ops: {0}")]
    Msg(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, OpsError>;

const CATALOG_FILE: &str = "CATALOG";
const WAL_DIR: &str = "wal";
const CATALOG_MAGIC: &[u8; 8] = b"PDBCAT01";
const WARCH_MAGIC: &[u8; 8] = b"PDBWAR01";

/// One base backup entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupMeta {
    /// Monotonic backup id.
    pub id: u64,
    /// Sequence frozen at base checkpoint.
    pub base_sequence: SequenceNumber,
    /// SST count at checkpoint.
    pub sst_count: usize,
    /// Version-GC watermark at checkpoint (MANIFEST v4 / PDBCKP02).
    pub earliest_readable_seq: SequenceNumber,
    /// Directory of the base (`…/base-000001`).
    pub path: PathBuf,
}

/// Result of shipping WAL records into the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalShipMeta {
    /// Archive segment path written (if any records shipped).
    pub segment: Option<PathBuf>,
    /// Number of logical WriteRecords archived.
    pub records: usize,
    /// Highest sequence among shipped ops (watermark).
    pub last_shipped_sequence: SequenceNumber,
}

/// Catalog state for a backup root.
#[derive(Debug, Clone)]
struct Catalog {
    next_id: u64,
    last_shipped_seq: SequenceNumber,
    next_wal_seg: u64,
}

impl Catalog {
    fn empty() -> Self {
        Self {
            next_id: 1,
            last_shipped_seq: 0,
            next_wal_seg: 1,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(8 + 8 + 8 + 8 + 4);
        b.extend_from_slice(CATALOG_MAGIC);
        b.extend_from_slice(&self.next_id.to_le_bytes());
        b.extend_from_slice(&self.last_shipped_seq.to_le_bytes());
        b.extend_from_slice(&self.next_wal_seg.to_le_bytes());
        let crc = crc32c::crc32c(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }

    fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 + 8 + 8 + 8 + 4 {
            return Err(OpsError::Msg("catalog too short".into()));
        }
        let (payload, crc_b) = buf.split_at(buf.len() - 4);
        let stored = u32::from_le_bytes(crc_b.try_into().unwrap());
        if !pedradb_core::wal::crc::crc_match_ok(stored, crc32c::crc32c(payload)) {
            return Err(OpsError::Msg("catalog CRC mismatch".into()));
        }
        if &payload[0..8] != CATALOG_MAGIC {
            return Err(OpsError::Msg("catalog magic".into()));
        }
        Ok(Self {
            next_id: u64::from_le_bytes(payload[8..16].try_into().unwrap()),
            last_shipped_seq: u64::from_le_bytes(payload[16..24].try_into().unwrap()),
            next_wal_seg: u64::from_le_bytes(payload[24..32].try_into().unwrap()),
        })
    }
}

/// Local backup + PITR engine over a backup root directory.
#[derive(Debug)]
pub struct BackupEngine<E: Env = IoUringEnv> {
    root: PathBuf,
    env: E,
    catalog: Catalog,
}

impl BackupEngine<IoUringEnv> {
    /// Open or create a backup root on the production Env.
    ///
    /// # Errors
    /// I/O.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_env(root, IoUringEnv::default())
    }
}

impl<E: Env> BackupEngine<E> {
    /// Open or create backup root via `env`.
    ///
    /// # Errors
    /// I/O or corrupt catalog.
    pub fn open_with_env(root: impl AsRef<Path>, env: E) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        env.create_dir_all(&root)?;
        env.create_dir_all(&root.join(WAL_DIR))?;
        let cat_path = root.join(CATALOG_FILE);
        let catalog = if env.exists(&cat_path) {
            let mut f = env.open_read(&cat_path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Catalog::decode(&buf)?
        } else {
            let c = Catalog::empty();
            Self::write_catalog(&env, &root, &c)?;
            c
        };
        Ok(Self { root, env, catalog })
    }

    fn write_catalog(env: &E, root: &Path, c: &Catalog) -> Result<()> {
        let path = root.join(CATALOG_FILE);
        let mut f = env.create(&path)?;
        f.write_all(&c.encode())?;
        f.sync_all()?;
        Ok(())
    }

    fn persist_catalog(&mut self) -> Result<()> {
        Self::write_catalog(&self.env, &self.root, &self.catalog)
    }

    /// Backup root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Highest sequence shipped into the WAL archive (0 if none).
    #[must_use]
    pub fn last_shipped_sequence(&self) -> SequenceNumber {
        self.catalog.last_shipped_seq
    }

    fn base_dir(&self, id: u64) -> PathBuf {
        self.root.join(format!("base-{id:06}"))
    }

    /// Create a full base backup (checkpoint) of `db` into this root.
    ///
    /// # Errors
    /// Checkpoint / I/O.
    pub fn create_base_backup(&mut self, db: &mut Db<E>) -> Result<BackupMeta>
    where
        E: Clone,
    {
        let id = self.catalog.next_id;
        let dest = self.base_dir(id);
        // create_checkpoint requires empty or missing dest.
        if self.env.exists(&dest) {
            return Err(OpsError::Msg(format!(
                "base dir already exists: {}",
                dest.display()
            )));
        }
        let ck = db.create_checkpoint(&dest)?;
        self.catalog.next_id = id + 1;
        // Base covers up to base_sequence; ship watermark at least that high so
        // we only archive *later* WAL for PITR.
        self.catalog.last_shipped_seq = self.catalog.last_shipped_seq.max(ck.last_sequence);
        self.persist_catalog()?;
        Ok(BackupMeta {
            id,
            base_sequence: ck.last_sequence,
            sst_count: ck.sst_count,
            earliest_readable_seq: ck.earliest_readable_seq,
            path: dest,
        })
    }

    /// Checkpoint a [`ConcurrentDb`] into a new base backup (RFC-0062 P1.5).
    /// Same catalog rules as [`Self::create_base_backup`].
    ///
    /// # Errors
    /// Checkpoint / I/O.
    pub fn create_base_backup_concurrent<E2: Env>(
        &mut self,
        db: &ConcurrentDb<E2>,
    ) -> Result<BackupMeta> {
        let id = self.catalog.next_id;
        let dest = self.base_dir(id);
        if self.env.exists(&dest) {
            return Err(OpsError::Msg(format!(
                "base dir already exists: {}",
                dest.display()
            )));
        }
        let ck = db.create_checkpoint(&dest)?;
        self.catalog.next_id = id + 1;
        self.catalog.last_shipped_seq = self.catalog.last_shipped_seq.max(ck.last_sequence);
        self.persist_catalog()?;
        Ok(BackupMeta {
            id,
            base_sequence: ck.last_sequence,
            sst_count: ck.sst_count,
            earliest_readable_seq: ck.earliest_readable_seq,
            path: dest,
        })
    }

    /// **Incremental backup** step (RFC-0014 P2.3): archive complete logical WAL
    /// records from the live DB whose sequences are greater than the current ship
    /// watermark. Alias of the historical ship path with explicit incremental naming.
    ///
    /// Call while unflushed durable writes still live in `CURRENT.log`, or take
    /// a new base backup after flush.
    ///
    /// # Errors
    /// WAL recover / I/O.
    pub fn create_incremental(&mut self, db: &Db<E>) -> Result<WalShipMeta> {
        self.ship_wal(db)
    }

    /// Archive complete logical WAL records from the live DB whose sequences are
    /// greater than the current ship watermark.
    ///
    /// Call while unflushed durable writes still live in `CURRENT.log`, or take
    /// a new base backup after flush.
    ///
    /// # Errors
    /// WAL recover / I/O.
    pub fn ship_wal(&mut self, db: &Db<E>) -> Result<WalShipMeta> {
        let wal_path = db.path().join(WAL_FILE_NAME);
        if !self.env.exists(&wal_path) {
            return Ok(WalShipMeta {
                segment: None,
                records: 0,
                last_shipped_sequence: self.catalog.last_shipped_seq,
            });
        }
        let raws = Wal::recover_on(&self.env, &wal_path)?;
        let mut to_ship: Vec<Vec<u8>> = Vec::new();
        let mut max_seq = self.catalog.last_shipped_seq;
        for raw in raws {
            let rec = WriteRecord::decode(&raw)?;
            let Some(ms) = rec.max_sequence() else {
                continue;
            };
            if ms > self.catalog.last_shipped_seq {
                to_ship.push(raw);
                max_seq = max_seq.max(ms);
            }
        }
        if to_ship.is_empty() {
            return Ok(WalShipMeta {
                segment: None,
                records: 0,
                last_shipped_sequence: self.catalog.last_shipped_seq,
            });
        }
        let seg_id = self.catalog.next_wal_seg;
        let seg_path = self.root.join(WAL_DIR).join(format!("{seg_id:06}.warch"));
        write_warch(&self.env, &seg_path, &to_ship)?;
        self.catalog.next_wal_seg = seg_id + 1;
        self.catalog.last_shipped_seq = max_seq;
        self.persist_catalog()?;
        Ok(WalShipMeta {
            segment: Some(seg_path),
            records: to_ship.len(),
            last_shipped_sequence: max_seq,
        })
    }

    /// List base backups (id ascending).
    ///
    /// # Errors
    /// I/O / corrupt checkpoint meta.
    pub fn list_backups(&self) -> Result<Vec<BackupMeta>> {
        let mut out = Vec::new();
        for name in self.env.read_dir_names(&self.root)? {
            let Some(id_str) = name.strip_prefix("base-") else {
                continue;
            };
            let Ok(id) = id_str.parse::<u64>() else {
                continue;
            };
            let path = self.root.join(&name);
            let ck = read_checkpoint_meta(&self.env, &path)?;
            out.push(BackupMeta {
                id,
                base_sequence: ck.last_sequence,
                sst_count: ck.sst_count,
                earliest_readable_seq: ck.earliest_readable_seq,
                path,
            });
        }
        out.sort_by_key(|b| b.id);
        Ok(out)
    }

    /// CRC-walk every `wal/*.warch` increment (RFC-0060 P2.9).
    ///
    /// # Errors
    /// Missing/corrupt archive segment.
    pub fn verify_wal_archive(&self) -> Result<usize> {
        let mut n = 0usize;
        for path in self.list_increments()? {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            read_warch(&self.env, &path)
                .map_err(|e| OpsError::Msg(format!("wal archive {name}: {e}")))?;
            n = n.saturating_add(1);
        }
        Ok(n)
    }

    /// Verify a base backup (checkpoint meta + WAL archive CRC + at-rest scrub + open + checksums).
    ///
    /// RFC-0060 P2.8: [`verify_at_rest`] walks CHANGELOG/history/CURRENT/WAL
    /// the live `verify_checksums` path does not. Open still F33-quarantines
    /// a poison CHANGELOG; this method **fails** so `pedra verify-backup`
    /// is honest.
    ///
    /// # Errors
    /// Corrupt backup or I/O.
    pub fn verify_backup(&self, backup_id: u64) -> Result<CheckpointMeta> {
        let path = self.base_dir(backup_id);
        if !self.env.exists(&path) {
            return Err(OpsError::Msg(format!("backup {backup_id} missing")));
        }
        let meta = read_checkpoint_meta(&self.env, &path)?;
        self.verify_wal_archive()?;
        // RFC-0060 P2.17: the backup *root* holds CATALOG + wal/*.warch.
        // Open already CRC'd CATALOG; this walk catches poison written after
        // open, and names the file the same way `pedra verify` does.
        let root_scrub = verify_at_rest(&self.env, &self.root);
        if !root_scrub.is_clean() {
            let first = root_scrub
                .failures
                .first()
                .map(|f| f.file.as_str())
                .unwrap_or("?");
            return Err(OpsError::Msg(format!(
                "backup root at-rest scrub {}: first={first}",
                root_scrub.summary_line()
            )));
        }
        let scrub = verify_at_rest(&self.env, &path);
        if !scrub.is_clean() {
            let first = scrub
                .failures
                .first()
                .map(|f| f.file.as_str())
                .unwrap_or("?");
            return Err(OpsError::Msg(format!(
                "backup {backup_id} at-rest scrub {}: first={first}",
                scrub.summary_line()
            )));
        }
        let db = Db::open_with_env(
            &path,
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
            self.env.clone(),
        )?;
        db.verify_checksums()?;
        if db.last_sequence() != meta.last_sequence {
            return Err(OpsError::Msg(format!(
                "backup {backup_id} seq drift: meta {} open {}",
                meta.last_sequence,
                db.last_sequence()
            )));
        }
        db.close()?;
        Ok(meta)
    }

    /// Restore a base backup to `dest` (full restore at base sequence).
    ///
    /// # Errors
    /// I/O / missing backup.
    pub fn restore(&self, backup_id: u64, dest: impl AsRef<Path>) -> Result<()> {
        self.restore_pitr(backup_id, dest, None)
    }

    /// List incremental WAL archive segment paths (sorted).
    ///
    /// # Errors
    /// I/O.
    pub fn list_increments(&self) -> Result<Vec<PathBuf>> {
        let wal_root = self.root.join(WAL_DIR);
        if !self.env.exists(&wal_root) {
            return Ok(Vec::new());
        }
        let mut segs: Vec<String> = self
            .env
            .read_dir_names(&wal_root)?
            .into_iter()
            .filter(|n| n.ends_with(".warch"))
            .collect();
        segs.sort();
        Ok(segs.into_iter().map(|n| wal_root.join(n)).collect())
    }

    /// Restore base + **all** archived increments up to the current ship watermark
    /// (RFC-0014 P2.3 incremental restore path).
    ///
    /// Equivalent to [`Self::restore_pitr`] with `target_sequence =
    /// Some(last_shipped_sequence)` when any WAL was shipped; base-only if watermark
    /// equals the base sequence.
    ///
    /// # Errors
    /// I/O / missing backup.
    pub fn restore_with_increments(&self, backup_id: u64, dest: impl AsRef<Path>) -> Result<()> {
        let base = self.base_dir(backup_id);
        if !self.env.exists(&base) {
            return Err(OpsError::Msg(format!("backup {backup_id} missing")));
        }
        let base_meta = read_checkpoint_meta(&self.env, &base)?;
        let target = self.catalog.last_shipped_seq;
        if target <= base_meta.last_sequence {
            return self.restore(backup_id, dest);
        }
        self.restore_pitr(backup_id, dest, Some(target))
    }

    /// Restore base `backup_id` to `dest`, optionally replaying archived WAL up
    /// to `target_sequence` (inclusive). `None` = base only.
    ///
    /// # Errors
    /// I/O, missing backup, or target_seq < base_sequence.
    pub fn restore_pitr(
        &self,
        backup_id: u64,
        dest: impl AsRef<Path>,
        target_sequence: Option<SequenceNumber>,
    ) -> Result<()> {
        let base = self.base_dir(backup_id);
        if !self.env.exists(&base) {
            return Err(OpsError::Msg(format!("backup {backup_id} missing")));
        }
        let base_meta = read_checkpoint_meta(&self.env, &base)?;
        if let Some(t) = target_sequence {
            if t < base_meta.last_sequence {
                return Err(OpsError::Msg(format!(
                    "target seq {t} < base {}",
                    base_meta.last_sequence
                )));
            }
        }
        let dest = dest.as_ref();
        copy_db_directory(&self.env, &base, dest)?;

        // Collect archived WAL records with base < seq <= target (or all after base).
        let mut replay: Vec<Vec<u8>> = Vec::new();
        if let Some(target) = target_sequence {
            if target > base_meta.last_sequence {
                let wal_root = self.root.join(WAL_DIR);
                if self.env.exists(&wal_root) {
                    let mut segs: Vec<String> = self
                        .env
                        .read_dir_names(&wal_root)?
                        .into_iter()
                        .filter(|n| n.ends_with(".warch"))
                        .collect();
                    segs.sort();
                    for name in segs {
                        let path = wal_root.join(&name);
                        let recs = read_warch(&self.env, &path)?;
                        for raw in recs {
                            let wr = WriteRecord::decode(&raw)?;
                            let Some(ms) = wr.max_sequence() else {
                                continue;
                            };
                            if ms > base_meta.last_sequence && ms <= target {
                                // Filter individual ops? Whole record is one TX;
                                // include record if max in range (records are atomic).
                                replay.push(raw);
                            }
                        }
                    }
                }
            }
        }

        if !replay.is_empty() {
            // Replace empty/rotated WAL with recovered filtered records.
            let wal_path = dest.join(WAL_FILE_NAME);
            let _ = self.env.remove_file(&wal_path);
            let mut wal = Wal::create_on(&self.env, &wal_path)?;
            for raw in &replay {
                wal.append_record(raw)?;
            }
            wal.sync_all()?;
            wal.close()?;
        }

        // Prove open + integrity.
        let db = Db::open_with_env(
            dest,
            OpenOptions {
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
            },
            self.env.clone(),
        )?;
        db.verify_checksums()?;
        if let Some(t) = target_sequence {
            if db.last_sequence() > t {
                return Err(OpsError::Msg(format!(
                    "restored last_sequence {} > target {t}",
                    db.last_sequence()
                )));
            }
        }
        db.close()?;
        Ok(())
    }
}

/// Report of a history-tier restore (RFC-0046 P1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryRestoreReport {
    /// Remote segments replayed.
    pub segments: usize,
    /// History records applied (post target filter).
    pub records: usize,
    /// `last_sequence` of the restored database.
    pub last_sequence: SequenceNumber,
}

/// Restore a database from the remote history tier (RFC-0046 P1.3):
/// destroys nothing here — reads the newest intact remote manifest,
/// streams every listed segment (per-record CRC verified), and replays the
/// versions up to `target_sequence` (`None` = all) into a fresh WAL at
/// `dest` in sequence order, so recovery rebuilds the database with the
/// original sequences. Open the result with `Db::open`.
///
/// Every record is CRC-verified at replay: corrupt bytes anywhere in the
/// chain are a typed error, never a silent wrong restore.
///
/// # Errors
/// Remote I/O, corrupt manifest/segment, or WAL write failure.
pub fn restore_history_from_remote<E: Env>(
    env: &E,
    remote_root: impl AsRef<Path>,
    dest: impl AsRef<Path>,
    target_sequence: Option<SequenceNumber>,
) -> Result<HistoryRestoreReport> {
    let tier = pedradb_core::history::RemoteTier::new(remote_root.as_ref());
    let segs = tier.latest_segments(env)?;
    if segs.is_empty() {
        return Err(OpsError::Msg(
            "remote history tier has no manifest — nothing to restore".into(),
        ));
    }
    let mut ops: Vec<WriteOp> = Vec::new();
    for seg in &segs {
        let bytes = tier.read_segment(env, &seg.name)?;
        for r in pedradb_core::history::walk_segment_records(&bytes)? {
            if target_sequence.is_some_and(|t| r.seq > t) {
                continue;
            }
            ops.push(match r.kind {
                1 => WriteOp::delete(r.seq, r.key),
                2 => WriteOp::delete_range(r.seq, r.key, r.val),
                _ => WriteOp::put(r.seq, r.key, r.val),
            });
        }
    }
    ops.sort_by_key(|o| o.sequence);
    let dest = dest.as_ref();
    env.create_dir_all(dest)?;
    let wal_path = dest.join(WAL_FILE_NAME);
    {
        let mut wal = Wal::create_on(env, &wal_path)?;
        for chunk in ops.chunks(256) {
            wal.append_write_ops(chunk)?;
        }
        wal.sync_all()?;
        wal.close()?;
    }
    env.sync_dir(dest)?;
    let db = Db::open_with_env(
        dest,
        OpenOptions {
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
        },
        env.clone(),
    )?;
    let report = HistoryRestoreReport {
        segments: segs.len(),
        records: ops.len(),
        last_sequence: db.last_sequence(),
    };
    db.close()?;
    Ok(report)
}

fn write_warch(env: &impl Env, path: &Path, records: &[Vec<u8>]) -> Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(WARCH_MAGIC);
    body.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for r in records {
        body.extend_from_slice(&(r.len() as u32).to_le_bytes());
        body.extend_from_slice(r);
    }
    let crc = crc32c::crc32c(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    let mut f = env.create(path)?;
    f.write_all(&body)?;
    f.sync_all()?;
    Ok(())
}

fn read_warch(env: &impl Env, path: &Path) -> Result<Vec<Vec<u8>>> {
    let mut f = env.open_read(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    if buf.len() < 8 + 4 + 4 {
        return Err(OpsError::Msg("warch too short".into()));
    }
    let (payload, crc_b) = buf.split_at(buf.len() - 4);
    let stored = u32::from_le_bytes(crc_b.try_into().unwrap());
    if !pedradb_core::wal::crc::crc_match_ok(stored, crc32c::crc32c(payload)) {
        return Err(OpsError::Msg("warch CRC mismatch".into()));
    }
    if &payload[0..8] != WARCH_MAGIC {
        return Err(OpsError::Msg("warch magic".into()));
    }
    let n = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let mut off = 12;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if off + 4 > payload.len() {
            return Err(OpsError::Msg("warch truncated".into()));
        }
        let len = u32::from_le_bytes(payload[off..off + 4].try_into().unwrap()) as usize;
        off += 4;
        if off + len > payload.len() {
            return Err(OpsError::Msg("warch record truncated".into()));
        }
        out.push(payload[off..off + len].to_vec());
        off += len;
    }
    Ok(out)
}

// ── Format migration ─────────────────────────────────────────────────────

/// On-disk format inspection report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatReport {
    /// Whether a MANIFEST / CURRENT pair exists.
    pub has_manifest: bool,
    /// Live SST count from inventory or scan.
    pub sst_count: usize,
    /// Detected SST format versions (file_num, version).
    pub sst_versions: Vec<(u64, u32)>,
    /// True if any SST is below current writer version or MANIFEST is legacy-only.
    pub needs_migration: bool,
    /// Version-GC watermark from MANIFEST v4 (`0` if absent / legacy).
    pub earliest_readable_seq: u64,
    /// MANIFEST mid-vlog-GC flag (`VALUES.vlog.new` preferred).
    pub vlog_use_new: bool,
    /// RFC-0060 P2.23: `CURRENT` CRC trailer — `absent` / `legacy` / `ok` /
    /// `mismatch` / `dangling` / `invalid`.
    pub current_crc: &'static str,
}

/// Result of rewriting a DB to current on-disk formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateReport {
    /// SST files rewritten.
    pub ssts_rewritten: usize,
    /// Last sequence after migrate.
    pub last_sequence: SequenceNumber,
    /// Whether verify_checksums passed after migrate.
    pub verified: bool,
}

/// Inspect format of a closed (or unlocked) DB directory.
///
/// # Errors
/// I/O or unreadable SST headers.
pub fn inspect_format(path: impl AsRef<Path>) -> Result<FormatReport> {
    inspect_format_env(&IoUringEnv::default(), path)
}

/// Inspect via `env`.
///
/// # Errors
/// I/O.
pub fn inspect_format_env(env: &impl Env, path: impl AsRef<Path>) -> Result<FormatReport> {
    let path = path.as_ref();
    let has_manifest = env.exists(&path.join(manifest::CURRENT_FILE));
    let mut sst_versions = Vec::new();
    let mut nums: Vec<u64> = Vec::new();
    let mut earliest_readable_seq = 0u64;
    let mut vlog_use_new = false;
    if let Some(vs) = manifest::load(env, path)? {
        nums = vs.sst_file_nums.clone();
        earliest_readable_seq = vs.earliest_readable_seq;
        vlog_use_new = vs.vlog_use_new;
    } else {
        for name in env.read_dir_names(path)? {
            if let Some(n) = manifest::parse_sst_name(&name) {
                nums.push(n);
            }
        }
        nums.sort_unstable();
    }
    for num in &nums {
        let p = VersionSet::sst_path(path, *num);
        if !env.exists(&p) {
            continue;
        }
        let ver = peek_sst_version(env, &p)?;
        sst_versions.push((*num, ver));
    }
    let current_writer = pedradb_core::sst::SST_VERSION;
    let needs_migration = sst_versions.iter().any(|&(_, v)| v < current_writer);
    Ok(FormatReport {
        has_manifest,
        sst_count: sst_versions.len(),
        sst_versions,
        needs_migration,
        earliest_readable_seq,
        vlog_use_new,
        current_crc: classify_current_crc(env, path),
    })
}

/// RFC-0060 P2.23: classify the optional CURRENT CRC trailer without opening a writer.
fn classify_current_crc(env: &impl Env, dir: &Path) -> &'static str {
    let cur = dir.join(manifest::CURRENT_FILE);
    if !env.exists(&cur) {
        return "absent";
    }
    let mut f = match env.open_read(&cur) {
        Ok(f) => f,
        Err(_) => return "invalid",
    };
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return "invalid";
    }
    let (name, crc) = match manifest::parse_current_pointer(&buf) {
        Ok(v) => v,
        Err(_) => return "invalid",
    };
    match crc {
        None => "legacy",
        Some(expect) => {
            let man = dir.join(&name);
            if !env.exists(&man) {
                return "dangling";
            }
            let mut bytes = Vec::new();
            match env.open_read(&man) {
                Ok(mut f) => {
                    if f.read_to_end(&mut bytes).is_err() {
                        return "dangling";
                    }
                }
                Err(_) => return "dangling",
            }
            if pedradb_core::wal::crc::crc_match_ok(expect, crc32c::crc32c(&bytes)) {
                "ok"
            } else {
                "mismatch"
            }
        }
    }
}

fn peek_sst_version(env: &impl Env, path: &Path) -> Result<u32> {
    let mut f = env.open_read(path)?;
    let mut hdr = [0u8; 12];
    f.read_exact(&mut hdr)?;
    // PEDRSST\0 magic (same as core sst writer).
    if &hdr[0..8] != b"PEDRSST\0" {
        return Err(OpsError::Msg(format!(
            "bad SST magic in {}",
            path.display()
        )));
    }
    Ok(u32::from_le_bytes(hdr[8..12].try_into().unwrap()))
}

/// Rewrite a DB directory to the current on-disk formats (SST writer version +
/// MANIFEST v2) by opening, compacting, and verifying.
///
/// # Errors
/// Open / compact / verify failures.
pub fn migrate_to_latest(path: impl AsRef<Path>) -> Result<MigrateReport> {
    migrate_to_latest_env(path, IoUringEnv::default())
}

/// Migrate via `env`.
///
/// # Errors
/// Open / compact / verify.
pub fn migrate_to_latest_env(path: impl AsRef<Path>, env: impl Env) -> Result<MigrateReport> {
    let path = path.as_ref();
    let before = inspect_format_env(&env, path)?;
    let mut db = Db::open_with_env(
        path,
        OpenOptions {
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
        },
        env.clone(),
    )?;
    db.flush()?;
    // Promote / rewrite through leveled compact a few times so live SSTs are
    // rewritten by the current writer (v4 + MANIFEST v2).
    let rounds = 4.max(before.sst_count.saturating_add(1));
    for _ in 0..rounds {
        db.compact()?;
    }
    db.verify_checksums()?;
    let last_sequence = db.last_sequence();
    let ssts_rewritten = db.sst_count();
    db.close()?;
    let after = inspect_format_env(&env, path)?;
    let verified = !after.needs_migration || after.sst_count == 0;
    Ok(MigrateReport {
        ssts_rewritten,
        last_sequence,
        verified,
    })
}

// Fix peek_sst_version - SST_MAGIC might not be re-exported publicly from sst module
// Use open via SstTable instead if needed.

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::{Db, OpenOptions};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-ops-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn open_db(path: &Path) -> Db<IoUringEnv> {
        Db::open_with_env(
            path,
            OpenOptions {
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
            },
            IoUringEnv::default(),
        )
        .unwrap()
    }

    /// RFC-0090 P2.1: production `BackupEngine::open_with_env` writes
    /// `CATALOG`; XOR only the trailer CRC (magic/ids intact). Reopen is
    /// crc mismatch. AS-IS would load the catalog.
    #[test]
    fn crc_mismatch_on_live_ops_catalog_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any catalog crc would match"
        );
        let bak = temp();
        BackupEngine::open_with_env(&bak, StdEnv).unwrap();
        let path = bak.join(CATALOG_FILE);
        let mut raw = std::fs::read(&path).unwrap();
        assert!(
            raw.len() >= 8 + 8 + 8 + 8 + 4,
            "CATALOG must have payload + trailer"
        );
        assert_eq!(&raw[0..8], CATALOG_MAGIC, "live catalog magic");
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        match BackupEngine::open_with_env(&bak, StdEnv) {
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&bak);
                panic!("AS-IS hole: opened catalog after CRC trailer lie");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&bak);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a magic/id parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0091 P0: production `create_incremental` writes `wal/*.warch`;
    /// XOR only the trailer CRC (magic/count/records intact). Verify is
    /// crc mismatch. AS-IS would return the shipped records.
    /// `verify_backup_flags_corrupt_warch` does not pin `crc_match_ok`.
    #[test]
    fn crc_mismatch_on_live_ops_warch_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any warch crc would match"
        );
        let data = temp();
        let bak = temp();
        let mut db = Db::open_with_env(
            &data,
            OpenOptions {
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
            },
            StdEnv,
        )
        .unwrap();
        db.put(b"base", b"0").unwrap();
        let mut eng = BackupEngine::open_with_env(&bak, StdEnv).unwrap();
        eng.create_base_backup(&mut db).unwrap();
        db.put(b"i1", b"a").unwrap();
        let ship = eng.create_incremental(&db).unwrap();
        assert!(ship.records >= 1, "need a WAL archive to lie to");
        let segs = eng.list_increments().unwrap();
        let warch = &segs[0];
        let mut bytes = std::fs::read(warch).unwrap();
        assert!(
            bytes.len() >= 8 + 4 + 4,
            "warch must have magic + count + trailer"
        );
        assert_eq!(&bytes[0..8], WARCH_MAGIC, "live warch magic");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(warch, &bytes).unwrap();
        match eng.verify_wal_archive() {
            Ok(_) => {
                db.close().unwrap();
                let _ = std::fs::remove_dir_all(&data);
                let _ = std::fs::remove_dir_all(&bak);
                panic!("AS-IS hole: verified warch after CRC trailer lie");
            }
            Err(e) => {
                db.close().unwrap();
                let _ = std::fs::remove_dir_all(&data);
                let _ = std::fs::remove_dir_all(&bak);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a magic/count parse; got {msg}"
                );
            }
        }
    }

    fn std_db(path: &Path) -> Db<StdEnv> {
        Db::open_with_env(
            path,
            OpenOptions {
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
            },
            StdEnv,
        )
        .unwrap()
    }

    /// RFC-0091 P1.1: production inspect of CURRENT; XOR only the stored
    /// CRC u32 and rewrite as 8 hex digits (name intact, MANIFEST intact).
    /// Classify is `mismatch`. AS-IS would report `ok`.
    /// `inspect_classifies_current_crc` (`ffffffff` rewrite) is not this tooth.
    #[test]
    fn crc_mismatch_on_live_ops_current_classify_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any CURRENT crc would match"
        );
        let data = temp();
        {
            let mut db = std_db(&data);
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        let ok = inspect_format_env(&StdEnv, &data).unwrap();
        assert_eq!(ok.current_crc, "ok", "live CURRENT must classify clean");
        let cur = data.join("CURRENT");
        let body = std::fs::read_to_string(&cur).unwrap();
        let mut lines = body.lines();
        let name = lines.next().expect("CURRENT name").trim();
        let crc_hex = lines.next().expect("CURRENT crc").trim();
        let crc = u32::from_str_radix(crc_hex, 16).expect("CURRENT crc hex");
        std::fs::write(&cur, format!("{name}\n{:08x}\n", crc ^ 0xffff_ffff)).unwrap();
        // inspect_format_env fail-closes in manifest::load; the inspect
        // classifier is classify_current_crc (`mismatch` vs `ok`).
        let lied = classify_current_crc(&StdEnv, &data);
        let _ = std::fs::remove_dir_all(&data);
        assert_eq!(
            lied, "mismatch",
            "must fail on crc_match_ok, not a pointer parse; got {lied}"
        );
    }

    /// RFC-0091 P1.2: production `restore_with_increments` calls
    /// `read_warch`. XOR only the trailer CRC (magic/count/records intact).
    /// Restore is crc mismatch. `crc_mismatch_on_live_ops_warch_is_not_ok`
    /// (verify path) is not this tooth.
    #[test]
    fn crc_mismatch_on_live_ops_warch_restore_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any warch crc would match"
        );
        let data = temp();
        let bak = temp();
        let rest = temp();
        let mut db = std_db(&data);
        db.put(b"base", b"0").unwrap();
        let mut eng = BackupEngine::open_with_env(&bak, StdEnv).unwrap();
        let meta = eng.create_base_backup(&mut db).unwrap();
        db.put(b"i1", b"a").unwrap();
        let ship = eng.create_incremental(&db).unwrap();
        assert!(ship.records >= 1, "need a WAL archive on the restore path");
        db.close().unwrap();
        let segs = eng.list_increments().unwrap();
        let warch = &segs[0];
        let mut bytes = std::fs::read(warch).unwrap();
        assert_eq!(&bytes[0..8], WARCH_MAGIC, "live warch magic");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(warch, &bytes).unwrap();
        match eng.restore_with_increments(meta.id, &rest) {
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&data);
                let _ = std::fs::remove_dir_all(&bak);
                let _ = std::fs::remove_dir_all(&rest);
                panic!("AS-IS hole: restored after warch CRC trailer lie");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&data);
                let _ = std::fs::remove_dir_all(&bak);
                let _ = std::fs::remove_dir_all(&rest);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on read_warch crc_match_ok, not a magic/count parse; got {msg}"
                );
            }
        }
    }

    #[test]
    fn base_backup_restore_round_trip() {
        let data = temp();
        let bak = temp();
        let rest = temp();
        {
            let mut db = open_db(&data);
            db.put(b"a", b"1").unwrap();
            db.put(b"b", b"2").unwrap();
            let mut eng = BackupEngine::open(&bak).unwrap();
            let meta = eng.create_base_backup(&mut db).unwrap();
            assert_eq!(meta.base_sequence, 2);
            eng.verify_backup(meta.id).unwrap();
            db.put(b"after", b"x").unwrap();
            db.close().unwrap();
        }
        let eng = BackupEngine::open(&bak).unwrap();
        eng.restore(1, &rest).unwrap();
        let db = open_db(&rest);
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(db.get(b"after"), None, "post-backup writes not in base");
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
        let _ = std::fs::remove_dir_all(&rest);
    }

    /// RFC-0060 P2.8: `verify_backup` runs the at-rest scrub (CHANGELOG
    /// poison fails here; `Db::open` would F33-quarantine it).
    #[test]
    fn verify_backup_runs_at_rest_scrub() {
        let data = temp();
        let bak = temp();
        let mut db = open_db(&data);
        db.put(b"k", b"v").unwrap();
        let mut eng = BackupEngine::open(&bak).unwrap();
        let meta = eng.create_base_backup(&mut db).unwrap();
        db.close().unwrap();
        eng.verify_backup(meta.id).unwrap();
        let ch = meta.path.join("CHANGELOG");
        let mut bytes = if ch.exists() {
            std::fs::read(&ch).unwrap()
        } else {
            let mut b = b"PDBCHLG1".to_vec();
            b.extend_from_slice(&0u32.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            b
        };
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&ch, &bytes).unwrap();
        let err = eng.verify_backup(meta.id).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("CHANGELOG") || msg.contains("at-rest scrub"),
            "verify_backup must fail the scrub, got {msg}"
        );
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
    }

    /// RFC-0060 P2.9: poison a `.warch` increment and `verify_backup` fails.
    #[test]
    fn verify_backup_flags_corrupt_warch() {
        let data = temp();
        let bak = temp();
        let mut db = open_db(&data);
        db.put(b"base", b"0").unwrap();
        let mut eng = BackupEngine::open(&bak).unwrap();
        let meta = eng.create_base_backup(&mut db).unwrap();
        db.put(b"i1", b"a").unwrap();
        let ship = eng.create_incremental(&db).unwrap();
        assert!(ship.records >= 1, "need a WAL archive to poison");
        db.close().unwrap();
        eng.verify_backup(meta.id).unwrap();
        let segs = eng.list_increments().unwrap();
        assert!(!segs.is_empty());
        let warch = &segs[0];
        let mut bytes = std::fs::read(warch).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(warch, &bytes).unwrap();
        let err = eng.verify_backup(meta.id).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("warch") || msg.contains("wal archive"),
            "verify_backup must fail the archive CRC, got {msg}"
        );
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
    }

    /// RFC-0060 P2.17: poison `CATALOG` after open; `verify_backup` fails.
    #[test]
    fn verify_backup_flags_corrupt_catalog() {
        let data = temp();
        let bak = temp();
        let mut db = open_db(&data);
        db.put(b"k", b"v").unwrap();
        let mut eng = BackupEngine::open(&bak).unwrap();
        let meta = eng.create_base_backup(&mut db).unwrap();
        db.close().unwrap();
        eng.verify_backup(meta.id).unwrap();
        let cat = bak.join("CATALOG");
        let mut bytes = std::fs::read(&cat).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&cat, &bytes).unwrap();
        let err = eng.verify_backup(meta.id).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("CATALOG") || msg.contains("at-rest scrub"),
            "verify_backup must fail CATALOG crc, got {msg}"
        );
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
    }

    /// RFC-0014 P2.3: base → two incremental ships → restore_with_increments.
    #[test]
    fn incremental_backup_two_ships_then_full_restore() {
        let data = temp();
        let bak = temp();
        let rest = temp();
        {
            let mut db = open_db(&data);
            db.put(b"base", b"0").unwrap();
            let mut eng = BackupEngine::open(&bak).unwrap();
            let base = eng.create_base_backup(&mut db).unwrap();

            db.put(b"i1", b"a").unwrap();
            let s1 = eng.create_incremental(&db).unwrap();
            assert!(s1.records >= 1);
            assert_eq!(eng.list_increments().unwrap().len(), 1);

            db.put(b"i2", b"b").unwrap();
            let s2 = eng.create_incremental(&db).unwrap();
            assert!(s2.records >= 1);
            assert_eq!(eng.list_increments().unwrap().len(), 2);

            eng.restore_with_increments(base.id, &rest).unwrap();
            db.close().unwrap();
        }
        let db = open_db(&rest);
        assert_eq!(db.get(b"base").as_deref(), Some(b"0".as_ref()));
        assert_eq!(db.get(b"i1").as_deref(), Some(b"a".as_ref()));
        assert_eq!(db.get(b"i2").as_deref(), Some(b"b".as_ref()));
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
        let _ = std::fs::remove_dir_all(&rest);
    }

    #[test]
    fn pitr_restore_replays_shipped_wal_to_target_seq() {
        let data = temp();
        let bak = temp();
        let rest = temp();
        {
            let mut db = open_db(&data);
            db.put(b"base", b"0").unwrap();
            let mut eng = BackupEngine::open(&bak).unwrap();
            let base = eng.create_base_backup(&mut db).unwrap();
            assert_eq!(base.base_sequence, 1);

            // Writes after base (live in WAL).
            db.put(b"k1", b"v1").unwrap(); // seq 2
            db.put(b"k2", b"v2").unwrap(); // seq 3
            db.put(b"k3", b"v3").unwrap(); // seq 4
            let ship = eng.ship_wal(&db).unwrap();
            assert!(ship.records >= 3);
            assert_eq!(ship.last_shipped_sequence, 4);

            // PITR to seq 3: k1,k2 present; k3 not.
            eng.restore_pitr(base.id, &rest, Some(3)).unwrap();
            db.close().unwrap();
        }
        let db = open_db(&rest);
        assert_eq!(db.get(b"base").as_deref(), Some(b"0".as_ref()));
        assert_eq!(db.get(b"k1").as_deref(), Some(b"v1".as_ref()));
        assert_eq!(db.get(b"k2").as_deref(), Some(b"v2".as_ref()));
        assert_eq!(db.get(b"k3"), None, "seq 4 must not appear at target 3");
        assert!(db.last_sequence() <= 3);
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
        let _ = std::fs::remove_dir_all(&rest);
    }

    #[test]
    fn migrate_to_latest_rewrites_and_verifies() {
        let data = temp();
        {
            let mut db = open_db(&data);
            for i in 0..20u8 {
                db.put([b'k', i], [b'v', i]).unwrap();
                if i % 5 == 4 {
                    db.flush().unwrap();
                }
            }
            db.close().unwrap();
        }
        let rep = inspect_format(&data).unwrap();
        assert!(rep.sst_count >= 1);
        assert!(rep.has_manifest);
        let m = migrate_to_latest(&data).unwrap();
        assert!(m.verified);
        assert!(m.last_sequence >= 20);
        let db = open_db(&data);
        assert_eq!(db.get(b"k\x00").as_deref(), Some(b"v\x00".as_ref()));
        db.verify_checksums().unwrap();
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
    }

    /// MANIFEST v4 watermark visible via inspect without opening a writer.
    #[test]
    fn inspect_reports_earliest_readable_after_reclaim() {
        use pedradb_core::CompactOptions;
        let data = temp();
        {
            let mut db = open_db(&data);
            db.put(b"k", b"old").unwrap();
            db.flush().unwrap();
            db.put(b"k", b"new").unwrap();
            db.flush().unwrap();
            db.compact_with(CompactOptions::latest_only()).unwrap();
            assert!(db.earliest_readable_sequence() > 0);
            let floor = db.earliest_readable_sequence();
            db.close().unwrap();
            let rep = inspect_format(&data).unwrap();
            assert_eq!(
                rep.earliest_readable_seq, floor,
                "inspect must read durable watermark without open"
            );
            assert!(rep.has_manifest);
            assert!(!rep.vlog_use_new);
            assert_eq!(
                rep.current_crc, "ok",
                "flushed CURRENT must carry a matching CRC"
            );
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    /// RFC-0060 P2.23: one-line CURRENT is `legacy`; two-line CRC mismatch is
    /// reported even though `manifest::load` fail-closes.
    #[test]
    fn inspect_classifies_current_crc() {
        let data = temp();
        {
            let mut db = open_db(&data);
            db.put(b"k", b"v").unwrap();
            db.flush().unwrap();
            db.close().unwrap();
        }
        let ok = inspect_format(&data).unwrap();
        assert_eq!(ok.current_crc, "ok");
        let cur = data.join("CURRENT");
        let body = std::fs::read_to_string(&cur).unwrap();
        let name = body.lines().next().unwrap().trim();
        std::fs::write(&cur, format!("{name}\n")).unwrap();
        let legacy = classify_current_crc(&StdEnv, &data);
        assert_eq!(legacy, "legacy");
        std::fs::write(&cur, format!("{name}\nffffffff\n")).unwrap();
        assert_eq!(classify_current_crc(&StdEnv, &data), "mismatch");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn list_backups_sorted() {
        let data = temp();
        let bak = temp();
        let mut db = open_db(&data);
        let mut eng = BackupEngine::open(&bak).unwrap();
        db.put(b"x", b"1").unwrap();
        eng.create_base_backup(&mut db).unwrap();
        db.put(b"y", b"2").unwrap();
        eng.create_base_backup(&mut db).unwrap();
        let list = eng.list_backups().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 1);
        assert_eq!(list[1].id, 2);
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
    }

    /// BackupMeta carries GC watermark from checkpoint meta (PDBCKP02).
    #[test]
    fn backup_meta_includes_earliest_readable() {
        use pedradb_core::CompactOptions;
        let data = temp();
        let bak = temp();
        let mut db = open_db(&data);
        db.put(b"k", b"old").unwrap();
        db.flush().unwrap();
        db.put(b"k", b"new").unwrap();
        db.flush().unwrap();
        db.compact_with(CompactOptions::latest_only()).unwrap();
        let floor = db.earliest_readable_sequence();
        assert!(floor > 0);
        let mut eng = BackupEngine::open(&bak).unwrap();
        let meta = eng.create_base_backup(&mut db).unwrap();
        assert_eq!(meta.earliest_readable_seq, floor);
        let list = eng.list_backups().unwrap();
        assert_eq!(list[0].earliest_readable_seq, floor);
        let ck = eng.verify_backup(meta.id).unwrap();
        assert_eq!(ck.earliest_readable_seq, floor);
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
    }

    /// RFC-0019 P2.1 / RFC-0016 P2.1: continuous puts + ship_wal; restore ⊆ acked prefix.
    #[test]
    fn rfc19_backup_under_continuous_put_restore_acked_prefix() {
        use std::collections::HashMap;

        let data = temp();
        let bak = temp();
        let rest = temp();
        let mut acked: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        {
            let mut db = open_db(&data);
            let mut eng = BackupEngine::open(&bak).unwrap();
            db.put(b"base", b"0").unwrap();
            acked.insert(b"base".to_vec(), b"0".to_vec());
            let base = eng.create_base_backup(&mut db).unwrap();

            // Continuous puts interleaved with ship_wal (load-like pattern).
            for i in 0..120u32 {
                let k = format!("k{i:05}").into_bytes();
                let v = format!("v{i}").into_bytes();
                db.put(&k, &v).unwrap();
                acked.insert(k, v);
                if i % 25 == 24 {
                    let ship = eng.ship_wal(&db).unwrap();
                    assert!(ship.last_shipped_sequence >= 1);
                }
            }
            let _ = eng.ship_wal(&db).unwrap();
            eng.restore_with_increments(base.id, &rest).unwrap();
            db.close().unwrap();
        }

        let restored = open_db(&rest);
        // Restore must not invent unacked keys; every restored KV ⊆ acked model.
        for (k, v) in
            restored.range_limited(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded, None)
        {
            let expect = acked.get(k.as_ref());
            assert!(
                expect.is_some(),
                "restore must not invent unacked key {:?}",
                String::from_utf8_lossy(&k)
            );
            assert_eq!(expect.map(Vec::as_slice), Some(v.as_ref()));
        }
        // Full incremental restore should include every acked key.
        for (k, v) in &acked {
            assert_eq!(
                restored.get(k).as_deref(),
                Some(v.as_slice()),
                "acked key missing after restore"
            );
        }
        restored.close().unwrap();
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&bak);
        let _ = std::fs::remove_dir_all(&rest);
    }

    /// RFC-0046 P1.3 e2e: destroy the local database, restore from the
    /// remote history tier at an arbitrary seq inside the horizon, verify
    /// values and MVCC, and prove corrupt remote bytes fail the restore.
    #[test]
    fn pitr_restore_from_object_storage() {
        use pedradb_core::{HistoryHorizon, HistoryOptions, Snapshot};
        let data = temp();
        let remote = temp();
        {
            let mut db = Db::open_with(
                &data,
                OpenOptions {
                    wal_full_fsync: true,
                    history: HistoryOptions {
                        horizon: HistoryHorizon::Window(std::time::Duration::from_millis(1)),
                        cap_bytes: 1 << 30,
                    },
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: Some(1),
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            db.set_remote_history(StdEnv, &remote);
            for i in 0..40u32 {
                db.put(b"k", format!("v{i:02}").as_bytes()).unwrap();
            }
            // Let every sample age past the 1 ms horizon, then one archive
            // round: segment → upload → GC.
            std::thread::sleep(std::time::Duration::from_millis(3));
            db.flush().unwrap();
            assert!(
                db.earliest_readable_sequence() > 0,
                "aging + archive round must advance the GC floor"
            );
            db.close().unwrap();
        }
        // Destroy the local database — the remote tier is all that remains.
        std::fs::remove_dir_all(&data).unwrap();

        // Full restore: the remote history tier holds the ARCHIVED PREFIX —
        // every version aged past the horizon. The newest in-window tail
        // lived in local SSTs/WAL (destroyed with the machine); covering it
        // is the WAL-ship path (`restore_with_increments`), as in Postgres
        // base+WAL PITR. So "all" = the state at the archive cutoff.
        let dest = temp();
        let report = restore_history_from_remote(&StdEnv, &remote, &dest, None).unwrap();
        assert_eq!(report.segments, 1);
        let cutoff = report.last_sequence;
        assert!(
            cutoff >= 31,
            "aging must archive nearly everything: {cutoff}"
        );
        let db = open_db(&dest);
        let expect = format!("v{:02}", cutoff - 1).into_bytes();
        assert_eq!(db.get(b"k").as_deref(), Some(expect.as_ref()));
        assert_eq!(
            db.get_at(Snapshot::at(15), b"k").unwrap().as_deref(),
            Some(b"v14".as_ref()),
            "MVCC at an arbitrary in-window seq survives the round trip"
        );
        db.close().unwrap();

        // Point-in-time restore at seq 20 (v19 is the state at that seq).
        let dest2 = temp();
        let pitr = restore_history_from_remote(&StdEnv, &remote, &dest2, Some(20)).unwrap();
        assert_eq!(pitr.last_sequence, 20);
        assert_eq!(pitr.records, 20);
        let db = open_db(&dest2);
        assert_eq!(db.get(b"k").as_deref(), Some(b"v19".as_ref()));
        db.close().unwrap();

        // Corrupt one remote segment byte → restore fails closed (typed).
        // P2.6 added `seg-*.hist.bloom` sidecars next to the segments —
        // match the segment itself, not the (restore-unread) sidecar.
        let seg: PathBuf = std::fs::read_dir(&remote)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("seg-") && n.ends_with(".hist")
                })
            })
            .expect("remote segment object");
        let mut bytes = std::fs::read(&seg).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        std::fs::write(&seg, &bytes).unwrap();
        let dest3 = temp();
        let err = restore_history_from_remote(&StdEnv, &remote, &dest3, None);
        assert!(
            matches!(err, Err(OpsError::Core(CoreError::CorruptHistory(_)))),
            "corrupt remote bytes must fail the restore closed: {err:?}"
        );

        let _ = std::fs::remove_dir_all(&remote);
        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&dest2);
        let _ = std::fs::remove_dir_all(&dest3);
    }
}
