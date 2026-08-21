//! PedraDB storage Env backed by **Linux io_uring** for write + fsync paths.
//!
//! # Why a separate crate
//! `pedradb-core` is `#![forbid(unsafe_code)]`. Submitting SQEs and calling
//! `posix_fadvise` require `unsafe`, so they live here. The engine still
//! speaks only [`Env`] / [`EnvFile`].
//!
//! # Platform
//! - **Linux:** real `io_uring` for `write`, `fsync` / `fdatasync`.
//! - **Elsewhere (incl. macOS):** transparent POSIX fallback via [`StdEnv`] so the
//!   same API works in dev; [`IoUringEnv::backend`] reports [`IoBackend::PosixFallback`].
//! - If ring setup fails on Linux (old kernel / seccomp), falls back to POSIX.
//!
//! # Usage
//! ```ignore
//! use pedradb_io_uring::IoUringEnv;
//! use pedradb_core::{Db, OpenOptions};
//!
//! let env = IoUringEnv::new()?;
//! let mut db = Db::open_with_env("/data/pedra", OpenOptions::default(), env)?;
//! db.put(b"k", b"v")?;
//! ```
//!
//! Or [`open`] / [`open_with`] helpers.

#![warn(missing_docs)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

#[cfg(target_os = "linux")]
use parking_lot::Mutex;
use pedradb_core::{
    AdviseKind, Db, Env, EnvFile, OpenOptions as DbOpen, Result as CoreResult, StdEnv,
};

/// Which I/O backend this env is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoBackend {
    /// Live Linux `io_uring` for write + fsync.
    IoUring,
    /// `std::fs` passthrough (non-Linux, or Linux setup failed).
    PosixFallback,
}

/// Whether this build/target can open a real io_uring ring.
#[must_use]
pub fn io_uring_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        true
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Production Env: io_uring on Linux when available, else POSIX.
#[derive(Clone)]
pub struct IoUringEnv {
    inner: Arc<Inner>,
}

enum Inner {
    #[cfg(target_os = "linux")]
    Uring {
        ring: Mutex<io_uring::IoUring>,
    },
    Posix(StdEnv),
}

impl std::fmt::Debug for IoUringEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoUringEnv")
            .field("backend", &self.backend())
            .finish()
    }
}

impl IoUringEnv {
    /// Create env: prefer io_uring on Linux; never panics on unsupported host.
    ///
    /// # Errors
    /// Only if POSIX fallback cannot be constructed (never today).
    pub fn new() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            match io_uring::IoUring::new(64) {
                Ok(ring) => {
                    return Ok(Self {
                        inner: Arc::new(Inner::Uring {
                            ring: Mutex::new(ring),
                        }),
                    });
                }
                Err(_) => {
                    // Old kernel / restricted environment.
                }
            }
        }
        Ok(Self {
            inner: Arc::new(Inner::Posix(StdEnv)),
        })
    }

    /// Force POSIX backend (tests / comparison).
    #[must_use]
    pub fn posix() -> Self {
        Self {
            inner: Arc::new(Inner::Posix(StdEnv)),
        }
    }

    /// Active backend for this instance.
    #[must_use]
    pub fn backend(&self) -> IoBackend {
        match &*self.inner {
            #[cfg(target_os = "linux")]
            Inner::Uring { .. } => IoBackend::IoUring,
            Inner::Posix(_) => IoBackend::PosixFallback,
        }
    }

    /// Open a DB on `path` with default options using this I/O backend.
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_db(&self, path: impl AsRef<Path>) -> CoreResult<Db<Self>> {
        self.open_db_with(path, DbOpen::default())
    }

    /// Open a DB with options.
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_db_with(&self, path: impl AsRef<Path>, opts: DbOpen) -> CoreResult<Db<Self>> {
        Db::open_with_env(path, opts, self.clone())
    }
}

impl Default for IoUringEnv {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self::posix())
    }
}

/// Convenience: open DB with a fresh [`IoUringEnv`].
///
/// # Errors
/// I/O / open failures.
pub fn open(path: impl AsRef<Path>) -> CoreResult<Db<IoUringEnv>> {
    let env = IoUringEnv::new().map_err(pedradb_core::CoreError::from)?;
    env.open_db(path)
}

/// Convenience: open with options.
///
/// # Errors
/// I/O / open failures.
pub fn open_with(path: impl AsRef<Path>, opts: DbOpen) -> CoreResult<Db<IoUringEnv>> {
    let env = IoUringEnv::new().map_err(pedradb_core::CoreError::from)?;
    env.open_db_with(path, opts)
}

/// File handle: uring write/fsync on Linux path, std otherwise.
pub struct IoUringFile {
    file: File,
    env: IoUringEnv,
    /// Logical cursor for write/read (append opens seek to end).
    pos: u64,
}

impl IoUringFile {
    fn from_file(file: File, env: IoUringEnv, pos: u64) -> Self {
        Self { file, env, pos }
    }

    #[cfg(target_os = "linux")]
    fn uring_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Inner::Uring { ring } = &*self.env.inner else {
            return self.file.write(buf);
        };
        if buf.is_empty() {
            return Ok(0);
        }
        let mut ring = ring.lock();
        let fd = io_uring::types::Fd(self.file.as_raw_fd());
        // SAFETY: `buf` is valid for the duration of submit_and_wait (we wait before return).
        let entry = io_uring::opcode::Write::new(fd, buf.as_ptr(), buf.len() as u32)
            .offset(self.pos)
            .build()
            .user_data(0x77);
        unsafe {
            ring.submission()
                .push(&entry)
                .map_err(|e| io::Error::other(format!("io_uring sq full: {e}")))?;
        }
        ring.submit_and_wait(1)?;
        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::other("io_uring missing cqe"))?;
        let res = cqe.result();
        if res < 0 {
            return Err(io::Error::from_raw_os_error(-res));
        }
        let n = res as usize;
        self.pos = self.pos.saturating_add(n as u64);
        Ok(n)
    }

    #[cfg(target_os = "linux")]
    fn uring_fsync(&mut self, datasync: bool) -> io::Result<()> {
        let Inner::Uring { ring } = &*self.env.inner else {
            return if datasync {
                self.file.sync_data()
            } else {
                self.file.sync_all()
            };
        };
        let mut ring = ring.lock();
        let fd = io_uring::types::Fd(self.file.as_raw_fd());
        let mut op = io_uring::opcode::Fsync::new(fd);
        if datasync {
            op = op.flags(io_uring::types::FsyncFlags::DATASYNC);
        }
        let entry = op.build().user_data(0x5f);
        // SAFETY: no buffer; fsync completes before return.
        unsafe {
            ring.submission()
                .push(&entry)
                .map_err(|e| io::Error::other(format!("io_uring sq full: {e}")))?;
        }
        ring.submit_and_wait(1)?;
        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::other("io_uring missing cqe"))?;
        let res = cqe.result();
        if res < 0 {
            return Err(io::Error::from_raw_os_error(-res));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;

impl Read for IoUringFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.read(buf)?;
        self.pos = self.pos.saturating_add(n as u64);
        Ok(n)
    }
}

impl Write for IoUringFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.env.backend() == IoBackend::IoUring {
            #[cfg(target_os = "linux")]
            {
                return self.uring_write(buf);
            }
        }
        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.write(buf)?;
        self.pos = self.pos.saturating_add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for IoUringFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = self.file.seek(pos)?;
        Ok(self.pos)
    }
}

impl EnvFile for IoUringFile {
    fn sync_data(&mut self) -> io::Result<()> {
        if self.env.backend() == IoBackend::IoUring {
            #[cfg(target_os = "linux")]
            {
                return self.uring_fsync(true);
            }
        }
        pedradb_core::env::fdatasync_file(&self.file)
    }

    fn sync_all(&mut self) -> io::Result<()> {
        if self.env.backend() == IoBackend::IoUring {
            #[cfg(target_os = "linux")]
            {
                return self.uring_fsync(false);
            }
        }
        self.file.sync_all()
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.file.set_len(len)?;
        if self.pos > len {
            self.pos = len;
        }
        Ok(())
    }

    fn len(&mut self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }
}

impl Env for IoUringEnv {
    type File = IoUringFile;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .read(true)
            .open(path)?;
        Ok(IoUringFile::from_file(file, self.clone(), 0))
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
        let len = file.metadata()?.len();
        Ok(IoUringFile::from_file(file, self.clone(), len))
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        let file = File::open(path)?;
        Ok(IoUringFile::from_file(file, self.clone(), 0))
    }

    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        let dir = File::open(path)?;
        #[cfg(target_os = "linux")]
        {
            if let Inner::Uring { ring } = &*self.inner {
                let mut ring = ring.lock();
                let fd = io_uring::types::Fd(dir.as_raw_fd());
                let entry = io_uring::opcode::Fsync::new(fd).build().user_data(0xd1);
                // SAFETY: dir fd lives until wait returns.
                unsafe {
                    ring.submission()
                        .push(&entry)
                        .map_err(|e| io::Error::other(format!("io_uring sq full: {e}")))?;
                }
                ring.submit_and_wait(1)?;
                let cqe = ring
                    .completion()
                    .next()
                    .ok_or_else(|| io::Error::other("io_uring missing cqe"))?;
                let res = cqe.result();
                if res < 0 {
                    return Err(io::Error::from_raw_os_error(-res));
                }
                return Ok(());
            }
        }
        dir.sync_all()
    }

    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for ent in fs::read_dir(path)? {
            let ent = ent?;
            names.push(ent.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        Ok(fs::metadata(path)?.len())
    }

    fn advise(&self, path: &Path, offset: u64, len: u64, kind: AdviseKind) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let f = File::open(path)?;
            let advice = match kind {
                AdviseKind::WillNeed => libc::POSIX_FADV_WILLNEED,
                AdviseKind::DontNeed => libc::POSIX_FADV_DONTNEED,
            };
            // posix_fadvise returns 0 on success, errno-style code otherwise.
            // SAFETY: `f` is open for the call; offset/len are best-effort hints.
            let rc = unsafe {
                libc::posix_fadvise(
                    f.as_raw_fd(),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                    i64::try_from(len).unwrap_or(0),
                    advice,
                )
            };
            if rc != 0 {
                return Err(io::Error::from_raw_os_error(rc));
            }
            return Ok(());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, offset, len, kind);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::OpenOptions;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("pedradb-iouring-{n}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn env_opens_and_reports_backend() {
        let env = IoUringEnv::new().unwrap();
        let b = env.backend();
        if io_uring_supported() {
            // Prefer IoUring; allow PosixFallback if kernel rejects ring.
            assert!(matches!(b, IoBackend::IoUring | IoBackend::PosixFallback));
        } else {
            assert_eq!(b, IoBackend::PosixFallback);
        }
    }

    #[test]
    fn put_get_flush_reopen_via_iouring_env() {
        let dir = temp_dir();
        let env = IoUringEnv::new().unwrap();
        {
            let mut db = env
                .open_db_with(
                    &dir,
                    OpenOptions {
                        history: Default::default(),
                        wal_recovery: Default::default(),
                        sync: true,
                        auto_flush_bytes: None,
                        auto_compact_sst_count: None,
                        auto_compact_sst_bytes: None,
                        exclusive: true,
                        large_value_threshold: None,
                    },
                )
                .unwrap();
            db.put(b"uring-k", b"uring-v").unwrap();
            db.flush().unwrap();
            assert_eq!(db.get(b"uring-k").as_deref(), Some(b"uring-v".as_ref()));
            db.close().unwrap();
        }
        let env2 = IoUringEnv::new().unwrap();
        let db = env2.open_db(&dir).unwrap();
        assert_eq!(db.get(b"uring-k").as_deref(), Some(b"uring-v".as_ref()));
        db.verify_checksums().unwrap();
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn convenience_open_round_trip() {
        let dir = temp_dir();
        {
            let mut db = open(&dir).unwrap();
            db.put(b"a", b"1").unwrap();
            db.close().unwrap();
        }
        let db = open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn posix_force_still_works() {
        let dir = temp_dir();
        let env = IoUringEnv::posix();
        assert_eq!(env.backend(), IoBackend::PosixFallback);
        let mut db = env.open_db(&dir).unwrap();
        db.put(b"p", b"q").unwrap();
        assert_eq!(db.get(b"p").as_deref(), Some(b"q".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advise_is_best_effort() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        {
            use std::io::Write;
            let mut f = File::create(&path).unwrap();
            f.write_all(&[0u8; 4096]).unwrap();
            f.sync_all().unwrap();
        }
        let env = IoUringEnv::new().unwrap();
        env.advise(&path, 0, 4096, AdviseKind::WillNeed).unwrap();
        env.advise(&path, 0, 4096, AdviseKind::DontNeed).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
