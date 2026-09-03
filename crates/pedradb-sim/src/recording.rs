//! [`RecordingEnv`]: buffered writes until sync; crash drops unsynced data (RFC-0011 P2.1).
//!
//! Optional [`SyncPolicy::Lying`] returns `Ok` from sync without promoting pending
//! bytes to durable (RFC-0011 P2.2). Optional short-write injects a partial
//! `write` then error (RFC-0011 P2.3).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pedradb_core::env::{Env, EnvFile};

/// How [`RecordingEnv`] treats `sync_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncPolicy {
    /// `sync_*` promotes pending writes to durable (crash-safe after Ok).
    #[default]
    Honest,
    /// `sync_*` returns `Ok` but does **not** promote pending (lying fsync).
    Lying,
}

/// Shared virtual filesystem image.
#[derive(Debug, Default)]
struct Image {
    files: HashMap<PathBuf, FileRec>,
    dirs: HashSet<PathBuf>,
    /// Remaining bytes allowed for the next short-write injection (`None` = off).
    short_write_left: Option<usize>,
    policy: SyncPolicy,
}

#[derive(Debug, Clone, Default)]
struct FileRec {
    /// Bytes that survive [`RecordingEnv::crash`].
    durable: Vec<u8>,
    /// Bytes written since last honest sync (lost on crash).
    pending: Vec<u8>,
}

impl FileRec {
    fn logical(&self) -> Vec<u8> {
        let mut v = self.durable.clone();
        v.extend_from_slice(&self.pending);
        v
    }

    fn promote(&mut self) {
        if !self.pending.is_empty() {
            self.durable.extend_from_slice(&self.pending);
            self.pending.clear();
        }
    }

    fn drop_pending(&mut self) {
        self.pending.clear();
    }
}

/// In-memory [`Env`] with crash image + optional lying sync / short-write.
#[derive(Debug, Clone)]
pub struct RecordingEnv {
    image: Rc<RefCell<Image>>,
}

impl RecordingEnv {
    /// Empty virtual FS, honest sync.
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(SyncPolicy::Honest)
    }

    /// Empty virtual FS with sync policy.
    #[must_use]
    pub fn with_policy(policy: SyncPolicy) -> Self {
        Self {
            image: Rc::new(RefCell::new(Image {
                files: HashMap::new(),
                dirs: HashSet::new(),
                short_write_left: None,
                policy,
            })),
        }
    }

    /// Lying fsync mode (writes appear synced but crash still drops them).
    #[must_use]
    pub fn lying() -> Self {
        Self::with_policy(SyncPolicy::Lying)
    }

    /// Next `write` returns at most `n` bytes then `WriteZero` / short error path.
    pub fn arm_short_write(&self, n: usize) {
        self.image.borrow_mut().short_write_left = Some(n);
    }

    /// Drop all pending (unsynced) data — process crash with buffered I/O.
    pub fn crash(&self) {
        let mut img = self.image.borrow_mut();
        for f in img.files.values_mut() {
            f.drop_pending();
        }
    }

    /// Bytes durable for `path` after crash semantics (empty if missing).
    #[must_use]
    pub fn durable_len(&self, path: &Path) -> usize {
        self.image
            .borrow()
            .files
            .get(path)
            .map_or(0, |f| f.durable.len())
    }

    /// Logical length including pending (pre-crash view).
    #[must_use]
    pub fn logical_len(&self, path: &Path) -> usize {
        self.image
            .borrow()
            .files
            .get(path)
            .map_or(0, |f| f.logical().len())
    }
}

impl Default for RecordingEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Open file on a [`RecordingEnv`].
pub struct RecordingFile {
    path: PathBuf,
    image: Rc<RefCell<Image>>,
    pos: u64,
}

impl RecordingFile {
    fn with_rec<R>(&mut self, f: impl FnOnce(&mut FileRec) -> R) -> io::Result<R> {
        let mut img = self.image.borrow_mut();
        let rec = img
            .files
            .get_mut(&self.path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "recording file missing"))?;
        Ok(f(rec))
    }
}

impl Read for RecordingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let data = self.with_rec(|r| r.logical())?;
        let start = self.pos as usize;
        if start >= data.len() {
            return Ok(0);
        }
        let n = (data.len() - start).min(buf.len());
        buf[..n].copy_from_slice(&data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Write for RecordingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut img = self.image.borrow_mut();
        let mut to_write = buf;
        if let Some(left) = img.short_write_left {
            if left == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "injected short-write exhausted",
                ));
            }
            let n = left.min(buf.len());
            to_write = &buf[..n];
            img.short_write_left = Some(left.saturating_sub(n));
            // After partial, next write fails hard if budget hit zero mid-call.
            if n < buf.len() {
                // Apply partial then signal short write.
                let rec = img.files.get_mut(&self.path).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "recording file missing")
                })?;
                let pos = self.pos as usize;
                let mut logical = rec.logical();
                if pos > logical.len() {
                    logical.resize(pos, 0);
                }
                if pos + n <= logical.len() {
                    logical[pos..pos + n].copy_from_slice(to_write);
                } else {
                    logical.truncate(pos);
                    logical.extend_from_slice(to_write);
                }
                // Split durable/pending: treat entire logical as durable+pending rewrite.
                // Simpler: append-only WAL style — write extends at pos.
                let dur_len = rec.durable.len();
                if pos < dur_len {
                    // Overwrite durable region + maybe pending — recompute.
                    rec.durable.truncate(pos);
                    rec.pending.clear();
                    rec.pending.extend_from_slice(to_write);
                } else {
                    let pend_off = pos - dur_len;
                    if pend_off > rec.pending.len() {
                        rec.pending.resize(pend_off, 0);
                    }
                    if pend_off + n <= rec.pending.len() {
                        rec.pending[pend_off..pend_off + n].copy_from_slice(to_write);
                    } else {
                        rec.pending.truncate(pend_off);
                        rec.pending.extend_from_slice(to_write);
                    }
                }
                self.pos += n as u64;
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "injected short write",
                ));
            }
        }
        let rec = img
            .files
            .get_mut(&self.path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "recording file missing"))?;
        let n = to_write.len();
        let pos = self.pos as usize;
        let dur_len = rec.durable.len();
        if pos < dur_len {
            rec.durable.truncate(pos);
            rec.pending.clear();
            rec.pending.extend_from_slice(to_write);
        } else {
            let pend_off = pos - dur_len;
            if pend_off > rec.pending.len() {
                rec.pending.resize(pend_off, 0);
            }
            if pend_off + n <= rec.pending.len() {
                rec.pending[pend_off..pend_off + n].copy_from_slice(to_write);
            } else {
                rec.pending.truncate(pend_off);
                rec.pending.extend_from_slice(to_write);
            }
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for RecordingFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let len = self.with_rec(|r| r.logical().len())? as u64;
        let next = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::End(o) => {
                if o >= 0 {
                    len + o as u64
                } else {
                    len.saturating_sub((-o) as u64)
                }
            }
            SeekFrom::Current(o) => {
                if o >= 0 {
                    self.pos + o as u64
                } else {
                    self.pos.saturating_sub((-o) as u64)
                }
            }
        };
        self.pos = next;
        Ok(self.pos)
    }
}

impl EnvFile for RecordingFile {
    fn sync_data(&mut self) -> io::Result<()> {
        let mut img = self.image.borrow_mut();
        let honest = img.policy == SyncPolicy::Honest;
        if pedradb_core::group_commit_kernel::fsync_promotes_pending(honest) {
            if let Some(rec) = img.files.get_mut(&self.path) {
                rec.promote();
            }
        }
        Ok(())
    }

    fn sync_all(&mut self) -> io::Result<()> {
        self.sync_data()
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.with_rec(|r| {
            let mut logical = r.logical();
            logical.resize(len as usize, 0);
            // Put all into pending if growing past durable; simplify: durable = logical, pending clear
            // For truncate of unsynced: keep durable min(len), pending adjusted.
            let dlen = r.durable.len();
            if (len as usize) <= dlen {
                r.durable.truncate(len as usize);
                r.pending.clear();
            } else {
                // durable stays; pending is the rest
                let need = len as usize - dlen;
                r.pending.resize(need, 0);
            }
        })
    }

    fn len(&mut self) -> io::Result<u64> {
        self.with_rec(|r| r.logical().len() as u64)
    }
}

impl Env for RecordingEnv {
    type File = RecordingFile;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut img = self.image.borrow_mut();
        let mut cur = PathBuf::new();
        for c in path.components() {
            cur.push(c);
            img.dirs.insert(cur.clone());
        }
        Ok(())
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        let mut img = self.image.borrow_mut();
        if let Some(parent) = path.parent() {
            img.dirs.insert(parent.to_path_buf());
        }
        img.files.insert(path.to_path_buf(), FileRec::default());
        Ok(RecordingFile {
            path: path.to_path_buf(),
            image: Rc::clone(&self.image),
            pos: 0,
        })
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        let mut img = self.image.borrow_mut();
        let rec = img.files.entry(path.to_path_buf()).or_default();
        let pos = rec.logical().len() as u64;
        Ok(RecordingFile {
            path: path.to_path_buf(),
            image: Rc::clone(&self.image),
            pos,
        })
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        let img = self.image.borrow();
        if !img.files.contains_key(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
        }
        Ok(RecordingFile {
            path: path.to_path_buf(),
            image: Rc::clone(&self.image),
            pos: 0,
        })
    }

    fn sync_dir(&self, _path: &Path) -> io::Result<()> {
        let mut img = self.image.borrow_mut();
        if img.policy == SyncPolicy::Honest {
            // Directory sync: promote all files (conservative).
            for f in img.files.values_mut() {
                f.promote();
            }
        }
        Ok(())
    }

    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
        let img = self.image.borrow();
        let mut names = Vec::new();
        for p in img.files.keys() {
            if p.parent() == Some(path) {
                if let Some(name) = p.file_name() {
                    names.push(name.to_string_lossy().into_owned());
                }
            }
        }
        // HashMap key order is per-instance; listing must be a function of
        // the image, not the hasher seed (DST / World replay).
        names.sort_unstable();
        Ok(names)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.image.borrow_mut().files.remove(path);
        Ok(())
    }

    /// F5: decide from the in-memory image, not the host filesystem — the
    /// trait default would silently bypass this Env.
    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        Ok(self.image.borrow().dirs.contains(path))
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut img = self.image.borrow_mut();
        let rec = img
            .files
            .remove(from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "rename src missing"))?;
        img.files.insert(to.to_path_buf(), rec);
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        let img = self.image.borrow();
        img.files.contains_key(path) || img.dirs.contains(path)
    }

    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        let img = self.image.borrow();
        img.files
            .get(path)
            .map(|f| f.logical().len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "metadata"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::{Db, OpenOptions, WAL_FILE_NAME};
    use std::path::PathBuf;

    fn opts() -> OpenOptions {
        OpenOptions {
            wal_full_fsync: true,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: false, // virtual FS has no real LOCK process semantics
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        }
    }

    #[test]
    fn honest_sync_survives_crash() {
        let env = RecordingEnv::new();
        let dir = PathBuf::from("/virt/db-honest");
        {
            let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
            db.put(b"k", b"v").unwrap();
            // put with sync=true already fsynced WAL
            drop(db);
        }
        env.crash();
        let db = Db::open_with_env(&dir, opts(), env).unwrap();
        assert_eq!(db.get(b"k").as_deref(), Some(b"v".as_ref()));
    }

    #[test]
    fn lying_sync_loses_write_after_crash() {
        let env = RecordingEnv::lying();
        let dir = PathBuf::from("/virt/db-lie");
        {
            let mut db = Db::open_with_env(&dir, opts(), env.clone()).unwrap();
            db.put(b"k", b"v").unwrap(); // sync Ok but lying
            drop(db);
        }
        // Before crash, data may be visible in-process... after crash pending gone.
        env.crash();
        // WAL file may be empty or partial — open must not invent the key.
        let db = Db::open_with_env(&dir, opts(), env).unwrap();
        assert_eq!(
            db.get(b"k"),
            None,
            "lying fsync must not retain after crash"
        );
    }

    /// Catalog three-teeth plant. Direct `fsync_ok_is_not_media_proof` /
    /// `lying_fsync_does_not_promote_pending` / `lying_sync_loses_write_after_crash`
    /// are **not** this tooth.
    #[test]
    fn fsync_promotes_pending_on_live_sim_is_not_ok() {
        assert!(!pedradb_core::group_commit_kernel::fsync_promotes_pending(
            false
        ));
        assert!(
            pedradb_core::group_commit_kernel::fsync_promotes_pending_as_is(false),
            "AS-IS dente: fsync Ok always promotes"
        );
        let env = RecordingEnv::lying();
        let dir = PathBuf::from("/virt/db-lie-0152");
        {
            let mut db = Db::open_with_env(
                &dir,
                OpenOptions {
                    exclusive: false,
                    ..OpenOptions::default()
                },
                env.clone(),
            )
            .unwrap();
            db.put(b"k", b"pending").unwrap();
            drop(db);
        }
        env.crash();
        let db = Db::open_with_env(
            &dir,
            OpenOptions {
                exclusive: false,
                ..OpenOptions::default()
            },
            env,
        )
        .unwrap();
        assert_eq!(
            db.get(b"k"),
            None,
            "live RecordingFile::sync_data must not promote pending on a lying fsync"
        );
    }

    #[test]
    fn short_write_errors() {
        let env = RecordingEnv::new();
        let path = PathBuf::from("/virt/short.bin");
        env.create_dir_all(Path::new("/virt")).unwrap();
        {
            let mut f = env.create(&path).unwrap();
            env.arm_short_write(2);
            let r = f.write(b"abcdef");
            assert!(r.is_err() || r.ok() == Some(2));
        }
    }

    #[test]
    fn crash_drops_unsynced_append() {
        let env = RecordingEnv::new();
        let path = PathBuf::from("/virt/wal");
        env.create_dir_all(Path::new("/virt")).unwrap();
        {
            let mut f = env.create(&path).unwrap();
            f.write_all(b"durable").unwrap();
            f.sync_all().unwrap();
            f.write_all(b"-pending").unwrap();
            // no sync
        }
        assert_eq!(env.logical_len(&path), b"durable-pending".len());
        env.crash();
        assert_eq!(env.durable_len(&path), b"durable".len());
        assert_eq!(env.logical_len(&path), b"durable".len());
        let _ = WAL_FILE_NAME; // keep import used if needed
    }
}
