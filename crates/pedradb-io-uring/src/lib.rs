//! PedraDB **production** storage Env: Linux `io_uring` for write + fsync.
//!
//! Product `open` paths (CLI, store, compat, HTTP, SQL, …) use this Env.
//! [`pedradb_core::StdEnv`] remains the test/DST filesystem (FailingEnv).
//!
//! # Why a separate crate
//! `pedradb-core` is `#![forbid(unsafe_code)]`. Submitting SQEs lives in
//! `ring.rs` (Linux). `posix_fadvise` lives in `pedradb-posix`. The engine
//! still speaks only [`Env`] / [`EnvFile`]. SQE `user_data` is unique per
//! issue (U1 / F203): leftover CQEs after a failed submit cannot be adopted
//! as a later write/fsync.
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

mod cqe_kernel;
#[cfg(target_os = "linux")]
mod ring;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

#[cfg(target_os = "linux")]
use parking_lot::Mutex;
use pedradb_core::{
    AdviseKind, ConcurrentDb, Db, Env, EnvFile, OpenOptions as DbOpen, Result as CoreResult, StdEnv,
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
        state: Mutex<ring::UringState>,
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
                            state: Mutex::new(ring::UringState::new(ring)),
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

    /// Replace the next harvested CQE result (Linux test soak — RFC-0050 P0.2).
    ///
    /// The real `io_uring_enter` still runs (buffer lifetime intact); only the
    /// `res` the caller sees is overridden. Returns whether the live ring accepted
    /// the inject.
    #[cfg(test)]
    #[must_use]
    pub fn inject_next_cqe_res(&self, res: i32) -> bool {
        #[cfg(target_os = "linux")]
        {
            if let Inner::Uring { state } = &*self.inner {
                state.lock().inject_next_cqe(res);
                return true;
            }
        }
        let _ = res;
        false
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
        production_env()
    }
}

/// RFC-0080 P1.2: full mode uses [`IoBackend::PosixFallback`] when the
/// ring is unavailable. AS-IS would claim a live ring anyway.
#[must_use]
pub fn full_uses_posix_fallback(ring_available: bool) -> bool {
    !ring_available
}

/// AS-IS: full mode is rounded to a live ring even when setup failed.
#[must_use]
pub fn full_uses_posix_fallback_as_is(_ring_available: bool) -> bool {
    false
}

/// Production Env: Linux `io_uring` when the kernel allows it, else POSIX.
#[must_use]
pub fn production_env() -> IoUringEnv {
    let candidate = IoUringEnv::new().unwrap_or_else(|_| IoUringEnv::posix());
    let ring_available = candidate.backend() == IoBackend::IoUring;
    if full_uses_posix_fallback(ring_available) {
        IoUringEnv::posix()
    } else {
        candidate
    }
}

/// Production concurrent DB (same Env as [`open`]).
///
/// # Errors
/// Same as [`ConcurrentDb::open_with_env`].
pub fn open_concurrent(path: impl AsRef<Path>) -> CoreResult<ConcurrentDb<IoUringEnv>> {
    open_concurrent_with(path, DbOpen::default())
}

/// Production concurrent DB with options.
///
/// # Errors
/// Same as [`ConcurrentDb::open_with_env`].
pub fn open_concurrent_with(
    path: impl AsRef<Path>,
    opts: DbOpen,
) -> CoreResult<ConcurrentDb<IoUringEnv>> {
    ConcurrentDb::open_with_env(path, opts, production_env())
}

/// Convenience: open DB with a fresh [`IoUringEnv`].
///
/// # Errors
/// I/O / open failures.
pub fn open(path: impl AsRef<Path>) -> CoreResult<Db<IoUringEnv>> {
    production_env().open_db(path)
}

/// Convenience: open with options.
///
/// # Errors
/// I/O / open failures.
pub fn open_with(path: impl AsRef<Path>, opts: DbOpen) -> CoreResult<Db<IoUringEnv>> {
    production_env().open_db_with(path, opts)
}

/// File handle: uring write/fsync on Linux path, std otherwise.
pub struct IoUringFile {
    file: File,
    /// Ring handle for test CQE inject. Production write/fsync is POSIX
    /// (RFC-0062 P1.1: uring `submit_and_wait` was the coluna B tax).
    #[allow(dead_code)]
    env: IoUringEnv,
    /// Logical cursor for write/read (append opens seek to end).
    pos: u64,
}

impl IoUringFile {
    fn from_file(file: File, env: IoUringEnv, pos: u64) -> Self {
        Self { file, env, pos }
    }

    /// Data path: `pwrite(2)` at the shadow cursor. Not the ring —
    /// `submit_and_wait(1)` serialized every WAL/SST write (diag-6).
    /// Durability is [`EnvFile::sync_data`] (`fdatasync(2)`, not the ring).
    fn posix_pwrite(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            let n = self.file.write_at(buf, self.pos)?;
            self.pos = self.pos.saturating_add(n as u64);
            return Ok(n);
        }
        #[cfg(not(unix))]
        {
            self.file.seek(SeekFrom::Start(self.pos))?;
            let n = self.file.write(buf)?;
            self.pos = self.pos.saturating_add(n as u64);
            Ok(n)
        }
    }

    /// Ring write (tests / RFC-0050 CQE inject). Production uses [`Self::posix_pwrite`].
    #[cfg(target_os = "linux")]
    #[cfg_attr(not(test), allow(dead_code))]
    fn uring_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Inner::Uring { state } = &*self.env.inner else {
            return self.posix_pwrite(buf);
        };
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = state.lock();
        let n = state.pwrite(&self.file, buf, self.pos)?;
        self.pos = self.pos.saturating_add(n as u64);
        Ok(n)
    }

    /// Ring fsync (tests / RFC-0050 CQE inject). Production G1 uses POSIX
    /// `fdatasync` / `fsync` (coluna B: `submit_and_wait` was the tax).
    #[cfg(target_os = "linux")]
    #[cfg_attr(not(test), allow(dead_code))]
    fn uring_fsync(&mut self, datasync: bool) -> io::Result<()> {
        let Inner::Uring { state } = &*self.env.inner else {
            return if datasync {
                self.file.sync_data()
            } else {
                self.file.sync_all()
            };
        };
        let mut state = state.lock();
        state.fsync(&self.file, datasync)
    }
}

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
        // Production WAL/SST write is POSIX `pwrite` (RFC-0062 / 0080 P2.2).
        // Linux *tests* use the ring so CQE inject (RFC-0074) hits
        // [`UringState::pwrite`]. `wal_on_sqe_admitted` is always false.
        #[cfg(all(test, target_os = "linux"))]
        {
            if self.env.backend() == IoBackend::IoUring {
                return self.uring_write(buf);
            }
        }
        if pedradb_core::wal_on_sqe_admitted() {
            #[cfg(target_os = "linux")]
            {
                return self.uring_write(buf);
            }
        }
        self.posix_pwrite(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for IoUringFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // F202: all I/O on this handle goes through the shadow cursor
        // (`self.pos`: uring pwrites at it; the POSIX fallback seeks the fd to
        // it first), so Seek must resolve against `self.pos` too. Delegating
        // to `self.file` reads the kernel fd offset, which pwrite-style I/O
        // never advances — and an `open_append` fd (O_APPEND) starts at 0
        // even on a non-empty file, so `stream_position()` returned 0 and
        // clobbered the correct cursor, corrupting WAL block framing on
        // every reopen (`WalWriter::new` derives its in-block offset from it).
        let new = match pos {
            SeekFrom::Start(n) => n,
            // End is absolute w.r.t. file length; the kernel offset is
            // irrelevant, so delegating is safe here.
            SeekFrom::End(n) => self.file.seek(SeekFrom::End(n))?,
            SeekFrom::Current(n) => self.pos.checked_add_signed(n).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid seek (cursor underflow)",
                )
            })?,
        };
        self.pos = new;
        Ok(self.pos)
    }
}

impl EnvFile for IoUringFile {
    fn sync_data(&mut self) -> io::Result<()> {
        // G1 / coluna B: `submit_and_wait` on every Ok was the Linux tax.
        // Tests still use the ring so CQE inject sees the fsync.
        // RFC-0080 P2.2: production WAL sync is not SQE.
        #[cfg(all(test, target_os = "linux"))]
        {
            if self.env.backend() == IoBackend::IoUring {
                return self.uring_fsync(true);
            }
        }
        if pedradb_core::wal_on_sqe_admitted() {
            #[cfg(target_os = "linux")]
            {
                return self.uring_fsync(true);
            }
        }
        pedradb_core::env::fdatasync_file(&self.file)
    }

    fn sync_data_strong(&mut self) -> io::Result<()> {
        // Darwin `File::sync_data` = `F_FULLFSYNC` (G1 advertised class).
        // Linux `File::sync_data` = `fdatasync` (same as [`Self::sync_data`]).
        self.file.sync_data()
    }

    fn sync_all(&mut self) -> io::Result<()> {
        #[cfg(all(test, target_os = "linux"))]
        {
            if self.env.backend() == IoBackend::IoUring {
                return self.uring_fsync(false);
            }
        }
        self.file.sync_all()
    }

    fn preallocate(&mut self, len: u64) -> io::Result<()> {
        pedradb_posix::preallocate_file(&self.file, len)
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
        #[cfg(all(test, target_os = "linux"))]
        {
            if let Inner::Uring { state } = &*self.inner {
                let mut state = state.lock();
                return state.fsync(&dir, false);
            }
        }
        pedradb_posix::sync_dir_fd(&dir)
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
        let f = File::open(path)?;
        let hint = match kind {
            AdviseKind::WillNeed => pedradb_posix::FileAdvise::WillNeed,
            AdviseKind::DontNeed => pedradb_posix::FileAdvise::DontNeed,
        };
        pedradb_posix::advise_file(&f, offset, len, hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::OpenOptions;
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
        let d = std::env::temp_dir().join(format!("pedradb-iouring-{n}-{i}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn production_env_is_default_open_backend() {
        let env = production_env();
        if cfg!(target_os = "linux") {
            assert_eq!(env.backend(), IoBackend::IoUring);
        } else {
            assert_eq!(env.backend(), IoBackend::PosixFallback);
        }
    }

    /// RFC-0080 P1.2: where the ring cannot open, full mode is PosixFallback.
    /// AS-IS would claim a live ring.
    #[test]
    fn full_mode_posix_fallback_when_ring_unavailable() {
        assert!(full_uses_posix_fallback(false));
        assert!(!full_uses_posix_fallback(true));
        assert!(
            !full_uses_posix_fallback_as_is(false),
            "AS-IS dente: claim live ring when unavailable"
        );
        let env = production_env();
        let ring = env.backend() == IoBackend::IoUring;
        assert_eq!(
            env.backend() == IoBackend::PosixFallback,
            full_uses_posix_fallback(ring)
        );
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
                        wal_full_fsync: true,
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

    /// RFC-0080 P2.2: production write/sync is POSIX, not SQE.
    #[test]
    fn production_wal_is_not_on_sqe() {
        assert!(!pedradb_core::wal_on_sqe_admitted());
        assert!(
            pedradb_core::wal_on_sqe_admitted_as_is(),
            "AS-IS dente: WAL back on SQE"
        );
        assert!(!pedradb_core::ring_twin_admitted());
        let dir = temp_dir();
        let env = IoUringEnv::posix();
        let mut db = env.open_db(&dir).unwrap();
        db.put(b"wal-sqe", b"off").unwrap();
        assert_eq!(db.get(b"wal-sqe").as_deref(), Some(&b"off"[..]));
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
    fn preallocate_is_forwarded_and_keep_size() {
        let dir = temp_dir();
        let env = IoUringEnv::new().unwrap();
        env.create_dir_all(&dir).unwrap();
        let mut f = env.create(&dir.join("wal.log")).unwrap();
        f.preallocate(1024 * 1024).unwrap();
        assert_eq!(
            f.len().unwrap(),
            0,
            "WAL reservation must not become recoverable bytes"
        );
        f.write_all(b"abc").unwrap();
        assert_eq!(f.len().unwrap(), 3);
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

    /// Linux-only tests open with explicit durable options (their macOS
    /// twins use `OpenOptions::default()` inline).
    #[cfg(target_os = "linux")]
    fn db_opts() -> OpenOptions {
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
        }
    }

    /// Linux CI / Docker soak: the ring must actually come up.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ring_is_live() {
        let env = IoUringEnv::new().unwrap();
        assert_eq!(
            env.backend(),
            IoBackend::IoUring,
            "linux soak requires a live io_uring (not PosixFallback)"
        );
    }

    /// Mixed write + `fdatasync` + dir fsync on the live ring, then Db
    /// put/flush/reopen/CRC. Unique CQE tags (U1) would mis-attribute a
    /// leftover same-opcode CQE; this loop would corrupt WAL/SST if they
    /// still reused 0x77/0x5f.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ring_soak_write_fsync_reopen() {
        let env = IoUringEnv::new().unwrap();
        assert_eq!(env.backend(), IoBackend::IoUring);
        let dir = temp_dir();
        env.create_dir_all(&dir).unwrap();

        let mut a = env.create(&dir.join("a.bin")).unwrap();
        let mut b = env.create(&dir.join("b.bin")).unwrap();
        let payload = [0x5a_u8; 1024];
        for i in 0..256u32 {
            a.write_all(&payload).unwrap();
            a.sync_data().unwrap();
            b.write_all(&i.to_le_bytes()).unwrap();
            b.sync_all().unwrap();
        }
        env.sync_dir(&dir).unwrap();
        drop(a);
        drop(b);

        // F202: open_append must see the shadow cursor at EOF, not kernel 0.
        let mut a = env.open_append(&dir.join("a.bin")).unwrap();
        assert_eq!(
            a.stream_position().unwrap(),
            256 * 1024,
            "open_append cursor must be file len (F202)"
        );
        drop(a);

        {
            let mut db = env.open_db_with(&dir.join("db"), db_opts()).unwrap();
            for i in 0..128u32 {
                let k = format!("k{i:03}");
                db.put(k.as_bytes(), &payload).unwrap();
                if i % 16 == 15 {
                    db.flush().unwrap();
                }
            }
            db.close().unwrap();
        }
        let env2 = IoUringEnv::new().unwrap();
        assert_eq!(env2.backend(), IoBackend::IoUring);
        let db = env2.open_db(&dir.join("db")).unwrap();
        assert_eq!(db.get(b"k000").as_deref(), Some(payload.as_ref()));
        assert_eq!(db.get(b"k127").as_deref(), Some(payload.as_ref()));
        db.verify_checksums().unwrap();
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// F208/F203 Linux: SIGALRM without `SA_RESTART` during write+`fdatasync`
    /// on the live ring. `io_uring_enter` can return `EINTR` after the SQE
    /// is in the kernel. Production waits for the matching CQE (does not
    /// drop `buf`). Assert: no crash, no `Ok(0)` on a non-empty write, file
    /// bytes equal the sum of `Ok(n)` returns.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_eintr_storm_write_sync_intact() {
        let env = IoUringEnv::new().unwrap();
        assert_eq!(env.backend(), IoBackend::IoUring);
        let dir = temp_dir();
        env.create_dir_all(&dir).unwrap();
        let path = dir.join("storm.bin");
        let mut f = env.create(&path).unwrap();

        unsafe {
            unsafe extern "C" fn nop(_: libc::c_int) {}
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = nop as *const () as libc::sighandler_t;
            sa.sa_flags = 0; // no SA_RESTART → EINTR
            libc::sigemptyset(&mut sa.sa_mask);
            assert_eq!(libc::sigaction(libc::SIGALRM, &sa, std::ptr::null_mut()), 0);
            let it = libc::itimerval {
                it_interval: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 500,
                },
                it_value: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 500,
                },
            };
            assert_eq!(
                libc::setitimer(libc::ITIMER_REAL, &it, std::ptr::null_mut()),
                0
            );
        }

        crate::cqe_kernel::F208_WAITMORE_AFTER_SUBMIT_ERR
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let payload = [0x3c_u8; 64];
        let mut acked: u64 = 0;
        let mut eintr_hits = 0u64;
        for _ in 0..2_000 {
            match f.write(&payload) {
                Ok(0) => panic!("Ok(0) on non-empty write (F203 class)"),
                Ok(n) => {
                    acked += n as u64;
                    loop {
                        match f.sync_data() {
                            Ok(()) => break,
                            Err(e)
                                if e.kind() == io::ErrorKind::Interrupted
                                    || e.raw_os_error() == Some(libc::EINTR) =>
                            {
                                eintr_hits += 1;
                            }
                            Err(e) => panic!("sync_data: {e}"),
                        }
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::Interrupted
                        || e.raw_os_error() == Some(libc::EINTR) =>
                {
                    eintr_hits += 1;
                }
                Err(e) => panic!("write: {e}"),
            }
        }

        unsafe {
            let it = libc::itimerval {
                it_interval: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                it_value: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
            };
            libc::setitimer(libc::ITIMER_REAL, &it, std::ptr::null_mut());
        }

        f.sync_all().unwrap();
        drop(f);
        let on_disk = fs::metadata(&path).unwrap().len();
        assert_eq!(
            on_disk, acked,
            "disk len must match Ok(n) sum (F208: no silent extra write after Err)"
        );
        let f208 = crate::cqe_kernel::F208_WAITMORE_AFTER_SUBMIT_ERR
            .load(std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "linux_eintr_storm acked={acked} eintr_hits={eintr_hits} disk={on_disk} f208_waitmore_after_submit_err={f208}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0152 P2.2.37: live `IoUringEnv` harvest gates CQE `res` through
    /// `cqe_res_ok`. Negative res is Err; AS-IS would Ok a failed fsync.
    /// Direct `cqe_negative_res_is_not_ok` / `linux_cqe_eio_is_not_ok` are
    /// not this tooth. Production G1 stays POSIX (RFC-0062 / 0073).
    #[test]
    fn cqe_res_ok_on_live_uring_is_not_ok() {
        assert!(!crate::cqe_kernel::cqe_res_ok(-5));
        assert!(
            crate::cqe_kernel::cqe_res_ok_as_is(-5),
            "AS-IS dente: negative CQE looks Ok"
        );
        let env = IoUringEnv::new().unwrap();
        let dir = temp_dir();
        env.create_dir_all(&dir).unwrap();
        let mut f = env.create(&dir.join("cqe.bin")).unwrap();
        f.write_all(b"wal").unwrap();
        f.sync_data().unwrap();
        #[cfg(target_os = "linux")]
        {
            assert_eq!(env.backend(), IoBackend::IoUring);
            assert!(
                env.inject_next_cqe_res(-libc::EIO),
                "live ring must accept CQE inject"
            );
            let err = f.sync_data().expect_err("EIO CQE must not be Ok");
            assert_eq!(err.raw_os_error(), Some(libc::EIO));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(env.backend(), IoBackend::PosixFallback);
            assert!(!io_uring_supported());
            assert!(
                !env.inject_next_cqe_res(-5),
                "PosixFallback has no CQE harvest"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0074 P0: negative CQE `res` is not Ok. On Linux this injects
    /// `-EIO` / `-ENOSPC` into the **live ring** harvest (`sync_data` /
    /// `write_all` under `cfg(test)`), so dropping [`cqe_res_ok`] from
    /// `UringState::{pwrite,fsync}` fails. Production G1 is POSIX
    /// `fdatasync` (RFC-0062 / 0073) — this is the only live SQE path.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cqe_eio_is_not_ok() {
        assert!(
            !crate::cqe_kernel::cqe_res_ok(-libc::EIO),
            "kernel: -EIO is not Ok"
        );
        assert!(crate::cqe_kernel::cqe_res_ok_as_is(-libc::EIO));
        let env = IoUringEnv::new().unwrap();
        assert_eq!(env.backend(), IoBackend::IoUring);
        let dir = temp_dir();
        env.create_dir_all(&dir).unwrap();
        let mut f = env.create(&dir.join("eio.bin")).unwrap();
        f.write_all(b"hello").unwrap();
        assert!(
            env.inject_next_cqe_res(-libc::EIO),
            "live ring must accept CQE inject"
        );
        let err = f.sync_data().expect_err("EIO CQE must not be Ok");
        assert_eq!(err.raw_os_error(), Some(libc::EIO));
        assert!(
            env.inject_next_cqe_res(-libc::ENOSPC),
            "live ring must accept ENOSPC inject"
        );
        let err = f.write_all(b"more").expect_err("ENOSPC CQE must not be Ok");
        assert_eq!(err.raw_os_error(), Some(libc::ENOSPC));
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0050 P0.2: FailingEnv wrapping a live IoUringEnv injects ENOSPC.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_failingenv_wrap_uring_enospc() {
        use pedradb_core::Db;
        use pedradb_sim::{FailingEnv, FaultKind, OpClass};

        let inner = IoUringEnv::new().unwrap();
        assert_eq!(inner.backend(), IoBackend::IoUring);
        let env = FailingEnv::wrap(inner);
        let dir = temp_dir();
        let mut db = Db::open_with_env(&dir, db_opts(), env.clone()).unwrap();
        db.put(b"seed", b"ok").unwrap();
        env.arm_op_class(OpClass::Write, 0, true, FaultKind::StorageFull);
        assert!(db.put(b"x", b"y").is_err(), "wrapped ENOSPC");
        drop(db);
        env.disarm();
        let db = Db::open_with_env(
            &dir,
            db_opts(),
            FailingEnv::wrap(IoUringEnv::new().unwrap()),
        )
        .unwrap();
        assert_eq!(db.get(b"seed").as_deref(), Some(b"ok".as_ref()));
        db.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
