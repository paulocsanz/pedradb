//! rust-rocksdb `backup::{BackupEngine, BackupEngineOptions, RestoreOptions}`
//! wrapping [`pedradb_ops::BackupEngine`].

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pedradb_core::Env as PedraEnv;
use pedradb_io_uring::IoUringEnv;
use pedradb_ops::{BackupEngine as OpsEngine, BackupMeta};

use super::{Env, Error, ErrorKind, Result, DB};

/// rust-rocksdb `BackupEngineInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEngineInfo {
    /// Unix seconds when the backup was listed (not Rocks' create time).
    pub timestamp: i64,
    /// Backup id (Pedra `base-NNNNNN`).
    pub backup_id: u32,
    /// SST count at checkpoint (not byte size).
    pub size: u64,
    /// Same as [`Self::size`] — Pedra does not share files across backups.
    pub num_files: u32,
}

/// rust-rocksdb `BackupEngineOptions`.
#[derive(Debug, Clone)]
pub struct BackupEngineOptions {
    path: PathBuf,
}

impl BackupEngineOptions {
    /// rust-rocksdb `BackupEngineOptions::new`.
    ///
    /// # Errors
    /// Never (directory is created on [`BackupEngine::open`]).
    pub fn new<P: AsRef<Path>>(backup_dir: P) -> Result<Self> {
        Ok(Self {
            path: backup_dir.as_ref().to_path_buf(),
        })
    }
}

/// rust-rocksdb `RestoreOptions`. `keep_log_files` is accepted; Pedra restore
/// always writes a recovered `CURRENT.log` from the checkpoint + archive.
#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    keep_log_files: bool,
}

impl RestoreOptions {
    /// rust-rocksdb `set_keep_log_files`.
    pub fn set_keep_log_files(&mut self, v: bool) {
        self.keep_log_files = v;
    }
}

/// rust-rocksdb `BackupEngine` on [`pedradb_ops::BackupEngine`].
pub struct BackupEngine {
    root: PathBuf,
    inner: OpsEngine<IoUringEnv>,
}

/// CFREG (and any future compat-only file) is not in the core checkpoint
/// inventory. Omitting it flips `default_raw` on restore and hides prefixed
/// default-CF keys (silent-wrong).
pub(crate) fn copy_compat_sidecars(src_db: &Path, dest: &Path) -> Result<()> {
    for name in ["CFREG"] {
        let src = src_db.join(name);
        if src.exists() {
            std::fs::copy(&src, dest.join(name)).map_err(|e| Error {
                msg: format!("copy {name}: {e}"),
                kind: ErrorKind::Io,
            })?;
        }
    }
    Ok(())
}

fn map_ops(e: pedradb_ops::OpsError) -> Error {
    match e {
        pedradb_ops::OpsError::Core(c) => Error::from(c),
        pedradb_ops::OpsError::Io(io) => Error {
            msg: format!("io: {io}"),
            kind: ErrorKind::Io,
        },
        pedradb_ops::OpsError::Msg(m) => Error {
            msg: m,
            kind: ErrorKind::Other,
        },
    }
}

impl BackupEngine {
    /// rust-rocksdb `BackupEngine::open`. `env` is accepted for API shape;
    /// the catalog lives on `IoUringEnv` (POSIX fallback off Linux).
    ///
    /// # Errors
    /// Catalog I/O.
    pub fn open(opts: &BackupEngineOptions, _env: &Env) -> Result<Self> {
        let inner = OpsEngine::open(&opts.path).map_err(map_ops)?;
        Ok(Self {
            root: opts.path.clone(),
            inner,
        })
    }

    /// rust-rocksdb `create_new_backup` (no extra flush).
    ///
    /// # Errors
    /// Checkpoint / catalog I/O.
    pub fn create_new_backup<E: PedraEnv + Clone>(&mut self, db: &DB<E>) -> Result<()> {
        self.create_new_backup_flush(db, false)
    }

    /// rust-rocksdb `create_new_backup_flush`.
    ///
    /// # Errors
    /// Flush / checkpoint / catalog I/O.
    pub fn create_new_backup_flush<E: PedraEnv + Clone>(
        &mut self,
        db: &DB<E>,
        flush_before_backup: bool,
    ) -> Result<()> {
        if flush_before_backup {
            db.flush()?;
        }
        let meta = self
            .inner
            .create_base_backup_concurrent(&db.inner)
            .map_err(map_ops)?;
        copy_compat_sidecars(&db.inner.path(), &meta.path)?;
        Ok(())
    }

    /// rust-rocksdb `purge_old_backups`. Deletes oldest base-* dirs until
    /// `num_backups_to_keep` remain.
    ///
    /// # Errors
    /// I/O.
    pub fn purge_old_backups(&mut self, num_backups_to_keep: usize) -> Result<()> {
        let mut list = self.inner.list_backups().map_err(map_ops)?;
        if list.len() <= num_backups_to_keep {
            return Ok(());
        }
        list.sort_by_key(|b| b.id);
        let drop_n = list.len() - num_backups_to_keep;
        for b in list.into_iter().take(drop_n) {
            let _ = std::fs::remove_dir_all(&b.path);
        }
        Ok(())
    }

    /// rust-rocksdb `restore_from_latest_backup`. `wal_dir` is ignored —
    /// Pedra WAL lives in the DB directory.
    ///
    /// # Errors
    /// Missing backup / I/O.
    pub fn restore_from_latest_backup<D: AsRef<Path>, W: AsRef<Path>>(
        &mut self,
        db_dir: D,
        _wal_dir: W,
        _opts: &RestoreOptions,
    ) -> Result<()> {
        let list = self.inner.list_backups().map_err(map_ops)?;
        let Some(latest) = list.iter().max_by_key(|b| b.id) else {
            return Err(Error {
                msg: "no backups to restore".into(),
                kind: ErrorKind::InvalidArgument,
            });
        };
        self.inner.restore(latest.id, db_dir).map_err(map_ops)
    }

    /// rust-rocksdb `restore_from_backup`.
    ///
    /// # Errors
    /// Missing backup / I/O.
    pub fn restore_from_backup<D: AsRef<Path>, W: AsRef<Path>>(
        &mut self,
        db_dir: D,
        _wal_dir: W,
        _opts: &RestoreOptions,
        backup_id: u32,
    ) -> Result<()> {
        self.inner
            .restore(u64::from(backup_id), db_dir)
            .map_err(map_ops)
    }

    /// rust-rocksdb `verify_backup` — at-rest scrub + checksums (stronger
    /// than Rocks' size-only verify).
    ///
    /// # Errors
    /// Corrupt backup.
    pub fn verify_backup(&self, backup_id: u32) -> Result<()> {
        self.inner
            .verify_backup(u64::from(backup_id))
            .map(|_| ())
            .map_err(map_ops)
    }

    /// rust-rocksdb `get_backup_info`.
    #[must_use]
    pub fn get_backup_info(&self) -> Vec<BackupEngineInfo> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match self.inner.list_backups() {
            Ok(list) => list
                .into_iter()
                .map(|b: BackupMeta| BackupEngineInfo {
                    timestamp: ts,
                    backup_id: u32::try_from(b.id).unwrap_or(u32::MAX),
                    size: b.sst_count as u64,
                    num_files: b.sst_count as u32,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Backup root (Pedra extension; not in rust-rocksdb).
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}
