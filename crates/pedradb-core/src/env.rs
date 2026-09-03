//! Filesystem seam for PedraDB (RBS/`depot-store` `Media` pattern, sync).
//!
//! Production default is [`StdEnv`] (passthrough to `std::fs`). On Linux, use
//! the `pedradb-io-uring` crate (`IoUringEnv`) for **`io_uring`** write + fsync
//! without changing engine code. Tests and `pedradb-sim` inject faults via a
//! wrapping [`Env`] (e.g. `FailingEnv`) without rewriting the engine.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

/// Per-file I/O the engine performs on WAL / SST handles.
pub trait EnvFile: Read + Write + Seek {
    /// `fdatasync` — data only.
    ///
    /// # Errors
    /// Underlying I/O.
    fn sync_data(&mut self) -> io::Result<()>;

    /// Strongest **data** barrier the platform has for this file.
    ///
    /// Default: same barrier class as [`Self::sync_data`] (sim / DST envs
    /// inherit it — fault seams see the same op). `File` on Darwin uses std
    /// `File::sync_data` = `fcntl(F_FULLFSYNC)` (~4 ms here), which is the
    /// class a CMake build of RocksDB uses for `WriteOptions.sync`
    /// (`HAVE_FULLFSYNC`); `librocksdb-sys` builds without that macro stay
    /// on `fsync`, the weak class. On Linux the two are the same barrier
    /// (`fdatasync` is a full barrier there). The WAL uses this class by
    /// default ([`OpenOptions::wal_full_fsync`], RFC-0036 addendum v2).
    ///
    /// # Errors
    /// Underlying I/O.
    fn sync_data_strong(&mut self) -> io::Result<()> {
        self.sync_data()
    }

    /// `fsync` — data + metadata.
    ///
    /// # Errors
    /// Underlying I/O.
    fn sync_all(&mut self) -> io::Result<()>;

    /// Truncate or extend to `len`.
    ///
    /// # Errors
    /// Underlying I/O.
    fn set_len(&mut self, len: u64) -> io::Result<()>;

    /// Reserve `len` bytes of storage past physical EOF so appends never
    /// block on filesystem extent allocation (Darwin `F_PREALLOCATE`, Linux
    /// `fallocate(FALLOC_FL_KEEP_SIZE)`; see `pedradb_posix::preallocate_file`).
    /// Does **not** change the logical size — recovery reads stop at `len`
    /// and never observe the reserved region. Best-effort: default is a
    /// no-op (sim / DST / platforms without support); callers treat failure
    /// as a missing optimization. Production `EnvFile` impls must forward
    /// this — a silent default no-op is a G1 tax vs Rocks.
    ///
    /// # Errors
    /// Underlying I/O when the platform implements the reservation.
    fn preallocate(&mut self, len: u64) -> io::Result<()> {
        let _ = len;
        Ok(())
    }

    /// Current file length.
    ///
    /// # Errors
    /// Underlying I/O.
    fn len(&mut self) -> io::Result<u64>;

    /// Whether the file has zero length.
    ///
    /// # Errors
    /// Underlying I/O.
    fn is_empty(&mut self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Positioned read that fills `buf` exactly from `offset` without
    /// leaving the handle's cursor moved (pread-shaped). Used by
    /// [`FileHandleCache`] to serve blocks from cached read-only handles.
    ///
    /// Default: portable seek → read → seek-back. Host-file handle types
    /// override with a stateless syscall so a cache-shared handle needs no
    /// cursor locking ([`FileExt::read_exact_at`] on Unix).
    ///
    /// # Errors
    /// Underlying I/O (missing range, short read).
    fn positioned_read_exact(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let cur = self.stream_position()?;
        self.seek(SeekFrom::Start(offset))?;
        let read = self.read_exact(buf);
        let back = self.seek(SeekFrom::Start(cur));
        read?;
        back?;
        Ok(())
    }

    /// Kernel readahead hint on **this** fd (`posix_fadvise`). Default no-op
    /// (sim / DST). Must be the cached handle — a second `open` of the same
    /// path does not affect this fd's readahead.
    ///
    /// # Errors
    /// Underlying I/O when the platform implements the hint. Callers treat
    /// failure as best-effort.
    fn advise(&mut self, offset: u64, len: u64, kind: AdviseKind) -> io::Result<()> {
        let _ = (offset, len, kind);
        Ok(())
    }
}

/// Hint for [`Env::advise`] (RFC-0029 P1.2 — `posix_fadvise`-shaped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdviseKind {
    /// Disable kernel readahead (Linux `POSIX_FADV_RANDOM`). Rocks SST
    /// opens default this on (`set_advise_random_on_open`). Without it,
    /// each 4 KiB random pread may pull 128 KiB — lookup_100's 100
    /// fresh keys (v69 in-band 0.87×).
    Random,
    /// Prefetch / readahead (Linux `POSIX_FADV_WILLNEED`).
    WillNeed,
    /// Drop pages from cache (Linux `POSIX_FADV_DONTNEED`).
    DontNeed,
}

/// Directory + file namespace the engine uses.
///
/// `Clone` so flush/open paths can hold a copy alongside open file handles
/// (test impls share interior fault state via `Rc`/`Arc`).
pub trait Env: Clone {
    /// Wall clock, milliseconds since Unix epoch (RFC-0046 history horizon).
    /// Default = real system time; tests override for deterministic expiry.
    fn unix_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Open file handle type.
    type File: EnvFile;

    /// Create directory tree (like `create_dir_all`).
    ///
    /// # Errors
    /// Underlying I/O.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Create or truncate a file for writing (WAL create, SST write).
    ///
    /// # Errors
    /// Underlying I/O.
    fn create(&self, path: &Path) -> io::Result<Self::File>;

    /// Open existing file for append (WAL after recover). Creates if missing.
    ///
    /// # Errors
    /// Underlying I/O.
    fn open_append(&self, path: &Path) -> io::Result<Self::File>;

    /// Open existing file read-only (WAL recover, SST open).
    ///
    /// # Errors
    /// Underlying I/O.
    fn open_read(&self, path: &Path) -> io::Result<Self::File>;

    /// fsync a directory so entries are durable.
    ///
    /// # Errors
    /// Underlying I/O.
    fn sync_dir(&self, path: &Path) -> io::Result<()>;

    /// List directory entry names (not full paths).
    ///
    /// # Errors
    /// Underlying I/O.
    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>>;

    /// Remove a file (best-effort durability via caller's `sync_dir`).
    ///
    /// # Errors
    /// Underlying I/O.
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Atomically rename `from` → `to` (same filesystem).
    ///
    /// # Errors
    /// Underlying I/O.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Whether `path` exists.
    fn exists(&self, path: &Path) -> bool;

    /// File length, or error if missing.
    ///
    /// # Errors
    /// Underlying I/O.
    fn metadata_len(&self, path: &Path) -> io::Result<u64>;

    /// Whether `path` is a directory (missing = `Ok(false)`).
    ///
    /// Default consults the **host filesystem** — override if this `Env` is
    /// not backed by it (an in-memory or remote env deciding via the real fs
    /// is a silent seam bypass, audit F5). Only `NotFound` maps to
    /// `Ok(false)`; other errors propagate.
    ///
    /// # Errors
    /// Underlying I/O other than `NotFound`.
    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        match fs::metadata(path) {
            Ok(m) => Ok(m.is_dir()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Copy `from` → `to` (create/truncate dest), then fsync dest.
    ///
    /// Used by [`crate::db::Db::create_checkpoint`]. Default walks `open_read` + `create`.
    ///
    /// # Errors
    /// Underlying I/O.
    fn copy_file(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut src = self.open_read(from)?;
        let mut dst = self.create(to)?;
        io::copy(&mut src, &mut dst)?;
        dst.sync_all()?;
        Ok(())
    }

    /// Optional kernel readahead / cache-drop for `[offset, offset+len)` of `path`.
    ///
    /// Default is a **no-op** (sim / DST). [`StdEnv`] implements Linux
    /// `posix_fadvise` via `pedradb-posix` so this crate stays
    /// `#![forbid(unsafe_code)]`. Errors are best-effort — callers must not
    /// fail the request on advise failure.
    ///
    /// # Errors
    /// Underlying I/O when the platform implements the hint.
    fn advise(&self, path: &Path, offset: u64, len: u64, kind: AdviseKind) -> io::Result<()> {
        let _ = (path, offset, len, kind);
        Ok(())
    }
}

/// POSIX `fdatasync(2)` on the data of `file`.
///
/// On Apple, [`File::sync_data`] is `fcntl(F_FULLFSYNC)` (~5 ms here). RocksDB
/// / TiKV `WriteOptions.sync` call libc `fdatasync` (~30–50 µs). WAL commit
/// uses this so the barrier class matches the peer (RFC-0001 / RFC-0036).
///
/// # Errors
/// Underlying I/O.
pub fn fdatasync_file(file: &File) -> io::Result<()> {
    pedradb_posix::fdatasync_file(file)
}

/// Object-safe byte source for SST files (RFC-0042 v18 payload pool).
///
/// Every v2+ [`SstTable`](crate::sst::SstTable) retains its CRC-stripped file
/// body for lazy block decode; under a byte budget evicted bodies are served
/// back from the file through this seam. It is deliberately not [`Env`]:
/// tables must hold a source without a generic parameter, so the owning
/// `Db` builds one `Arc<dyn SstFileSource>` (via [`EnvSource`]) at open.
pub trait SstFileSource: Send + Sync {
    /// Fill `buf` exactly with the bytes of `path` at `offset`.
    ///
    /// # Errors
    /// Underlying I/O (missing file, short read).
    fn read_range(&self, path: &Path, offset: u64, buf: &mut [u8]) -> io::Result<()>;

    /// Read the whole file into memory.
    ///
    /// # Errors
    /// Underlying I/O.
    fn read_all(&self, path: &Path) -> io::Result<Vec<u8>>;
}

impl std::fmt::Debug for dyn SstFileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SstFileSource").finish_non_exhaustive()
    }
}

/// [`SstFileSource`] over any [`Env`] (open → seek → read).
#[derive(Debug, Clone, Copy)]
pub struct EnvSource<E>(pub E);

impl<E: Env + Send + Sync> SstFileSource for EnvSource<E> {
    fn read_range(&self, path: &Path, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut file = self.0.open_read(path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf)
    }

    fn read_all(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut file = self.0.open_read(path)?;
        let mut out = Vec::new();
        file.read_to_end(&mut out)?;
        Ok(out)
    }
}

/// Bounded LRU of read-only handles for [`SstFileSource`] serving (the
/// file cache RocksDB keeps per DB).
///
/// Without it, every block read from an evicted-payload table pays
/// `open(path)` + seek + read — the 6M slipstream `get_hit` profile put
/// 41% of main-thread samples inside `File::open` alone. A hit here is a
/// single [`EnvFile::positioned_read_exact`].
///
/// Correctness contract:
///
/// * Handles come from [`Env::open_read`] (never a direct `std::fs`
///   open), so fault-injecting `Env`s still see every read.
/// * [`FileHandleCache::invalidate`] is part of file deletion, not an
///   optimization: an open handle keeps an unlinked inode (and its disk
///   space) alive, and a failed SST write rolls the file-number counter
///   back so the same path can be re-allocated with different bytes.
/// * A read racing an invalidate still reads the old inode — SST bodies
///   are immutable, so those are the bytes the reader asked for.
#[derive(Debug)]
pub struct FileHandleCache {
    capacity: usize,
    inner: RwLock<FileHandleCacheInner>,
}

#[derive(Debug, Default)]
struct FileHandleCacheInner {
    map: HashMap<PathBuf, CachedHandle>,
    tick: u64,
}

struct CachedHandle {
    file: Arc<Mutex<Box<dyn EnvFile + Send>>>,
    tick: u64,
}

impl std::fmt::Debug for CachedHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedHandle")
            .field("tick", &self.tick)
            .finish_non_exhaustive()
    }
}

impl FileHandleCache {
    /// Cache holding at most `capacity` read handles. `0` disables
    /// caching (every read opens — the bench A/B knob).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: RwLock::new(FileHandleCacheInner::default()),
        }
    }

    /// Cached handle for `path`. No LRU bump: 25M settle is ~90 SSTs and
    /// the default cap is 256, so get_loop's 100 preads never evict. A
    /// write lock + tick on every miss-path get was exclusive-mutex tax
    /// (lookup_100 calm-1 0.87× vs in-band Rocks).
    fn get(&self, path: &Path) -> Option<Arc<Mutex<Box<dyn EnvFile + Send>>>> {
        self.inner.read().map.get(path).map(|h| Arc::clone(&h.file))
    }

    /// Store a freshly opened handle, evicting the least-recently-used
    /// entry at capacity (the bound is fds, so the linear scan wins).
    fn insert(&self, path: &Path, file: Box<dyn EnvFile + Send>) {
        if self.capacity == 0 {
            return;
        }
        let mut inner = self.inner.write();
        if inner.map.len() >= self.capacity && !inner.map.contains_key(path) {
            if let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, h)| h.tick)
                .map(|(p, _)| p.clone())
            {
                inner.map.remove(&oldest);
            }
        }
        inner.tick = inner.tick.wrapping_add(1);
        let tick = inner.tick;
        inner.map.insert(
            path.to_path_buf(),
            CachedHandle {
                file: Arc::new(Mutex::new(file)),
                tick,
            },
        );
    }

    /// Drop the cached handle for `path` (the fd closes once in-flight
    /// reads finish). No-op when absent.
    pub fn invalidate(&self, path: &Path) {
        self.inner.write().map.remove(path);
    }
}

/// [`SstFileSource`] serving reads through a shared [`FileHandleCache`]:
/// first read opens via the [`Env`] (fault seam intact) and caches the
/// handle; later reads reuse it.
pub struct CachedEnvSource<E> {
    env: E,
    cache: Arc<FileHandleCache>,
}

impl<E: Env> CachedEnvSource<E> {
    /// Source reading through `cache`; misses open via `env`.
    #[must_use]
    pub fn new(env: E, cache: Arc<FileHandleCache>) -> Self {
        Self { env, cache }
    }
}

impl<E: Env + Send + Sync> Clone for CachedEnvSource<E> {
    fn clone(&self) -> Self {
        Self {
            env: <E as Clone>::clone(&self.env),
            cache: Arc::clone(&self.cache),
        }
    }
}

impl<E> std::fmt::Debug for CachedEnvSource<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedEnvSource").finish_non_exhaustive()
    }
}

impl<E: Env + Send + Sync> SstFileSource for CachedEnvSource<E>
where
    E::File: Send + 'static,
{
    fn read_range(&self, path: &Path, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if let Some(handle) = self.cache.get(path) {
            let mut file = handle.lock();
            return file.positioned_read_exact(buf, offset);
        }
        let mut file = self.env.open_read(path)?;
        // Same fd we will pread: Rocks `set_advise_random_on_open`.
        let _ = file.advise(0, 0, AdviseKind::Random);
        file.positioned_read_exact(buf, offset)?;
        self.cache.insert(path, Box::new(file));
        Ok(())
    }

    fn read_all(&self, path: &Path) -> io::Result<Vec<u8>> {
        // ≤v4 whole-body reload (file-CRC gate) — rare; plain open so
        // full-file readers do not pin cache entries.
        let mut file = self.env.open_read(path)?;
        let mut out = Vec::new();
        file.read_to_end(&mut out)?;
        Ok(out)
    }
}

impl EnvFile for File {
    fn sync_data(&mut self) -> io::Result<()> {
        fdatasync_file(self)
    }

    /// Strongest data barrier the platform has for this file. On Darwin that
    /// is std `File::sync_data` = `fcntl(F_FULLFSYNC)` — the class a CMake
    /// build of RocksDB uses for `WriteOptions.sync` (`HAVE_FULLFSYNC`). On
    /// Linux std `sync_data` is `fdatasync`, already a full barrier — the
    /// strong and default classes are identical there. The WAL barrier uses
    /// this class by default (`OpenOptions::wal_full_fsync`, RFC-0036
    /// addendum v2).
    fn sync_data_strong(&mut self) -> io::Result<()> {
        File::sync_data(self)
    }

    fn preallocate(&mut self, len: u64) -> io::Result<()> {
        pedradb_posix::preallocate_file(self, len)
    }

    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self)
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        File::set_len(self, len)
    }

    fn len(&mut self) -> io::Result<u64> {
        let pos = self.stream_position()?;
        let end = self.seek(SeekFrom::End(0))?;
        self.seek(SeekFrom::Start(pos))?;
        Ok(end)
    }

    /// `pread`-class: no cursor state, no seek syscalls — safe on a handle
    /// shared through the file cache (kernel serializes the read itself).
    #[cfg(unix)]
    fn positioned_read_exact(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }

    fn advise(&mut self, offset: u64, len: u64, kind: AdviseKind) -> io::Result<()> {
        let hint = match kind {
            AdviseKind::Random => pedradb_posix::FileAdvise::Random,
            AdviseKind::WillNeed => pedradb_posix::FileAdvise::WillNeed,
            AdviseKind::DontNeed => pedradb_posix::FileAdvise::DontNeed,
        };
        pedradb_posix::advise_file(self, offset, len, hint)
    }
}

/// Production [`Env`]: zero-cost passthrough to `std::fs`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdEnv;

impl Env for StdEnv {
    type File = File;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.seek(SeekFrom::End(0))?;
        Ok(file)
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        File::open(path)
    }

    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        let dir = File::open(path)?;
        // Same class as WAL G1 (RFC-0036): not Apple F_FULLFSYNC.
        pedradb_posix::sync_dir_fd(&dir)
    }

    fn advise(&self, path: &Path, offset: u64, len: u64, kind: AdviseKind) -> io::Result<()> {
        let f = File::open(path)?;
        let hint = match kind {
            AdviseKind::Random => pedradb_posix::FileAdvise::Random,
            AdviseKind::WillNeed => pedradb_posix::FileAdvise::WillNeed,
            AdviseKind::DontNeed => pedradb_posix::FileAdvise::DontNeed,
        };
        pedradb_posix::advise_file(&f, offset, len, hint)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DEFAULT_TEST_CACHE: usize = 8;

    /// `Env` over [`StdEnv`] that counts `open_read` calls — proves the
    /// file cache reuses handles instead of re-opening, and that misses
    /// still flow through the `Env` seam (fault injection stays real).
    #[derive(Clone, Default)]
    struct CountingEnv {
        opens: Arc<AtomicUsize>,
    }

    impl Env for CountingEnv {
        type File = File;

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            StdEnv.create_dir_all(path)
        }
        fn create(&self, path: &Path) -> io::Result<Self::File> {
            StdEnv.create(path)
        }
        fn open_append(&self, path: &Path) -> io::Result<Self::File> {
            StdEnv.open_append(path)
        }
        fn open_read(&self, path: &Path) -> io::Result<Self::File> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            StdEnv.open_read(path)
        }
        fn sync_dir(&self, path: &Path) -> io::Result<()> {
            StdEnv.sync_dir(path)
        }
        fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
            StdEnv.read_dir_names(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            StdEnv.remove_file(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            StdEnv.rename(from, to)
        }
        fn exists(&self, path: &Path) -> bool {
            StdEnv.exists(path)
        }
        fn metadata_len(&self, path: &Path) -> io::Result<u64> {
            StdEnv.metadata_len(path)
        }
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pedra-fdcache-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        let mut f = File::create(path).unwrap();
        f.write_all(bytes).unwrap();
        f.sync_all().unwrap();
    }

    #[test]
    fn cached_source_serves_repeated_reads_from_one_handle() {
        let dir = scratch_dir("reuse");
        let path = dir.join("sst.bin");
        write_file(&path, b"0123456789abcdefGHIJKLMNOP");

        let env = CountingEnv::default();
        let src = CachedEnvSource::new(
            env.clone(),
            Arc::new(FileHandleCache::new(DEFAULT_TEST_CACHE)),
        );
        let mut buf = [0u8; 4];
        src.read_range(&path, 12, &mut buf).unwrap();
        assert_eq!(&buf, b"cdef");
        src.read_range(&path, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"0123");
        src.read_range(&path, 20, &mut buf).unwrap();
        assert_eq!(&buf, b"KLMN");
        // Three reads, one open: the handle was reused.
        assert_eq!(env.opens.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_source_zero_capacity_opens_every_read() {
        let dir = scratch_dir("nocache");
        let path = dir.join("sst.bin");
        write_file(&path, b"0123456789");

        let env = CountingEnv::default();
        let src = CachedEnvSource::new(env.clone(), Arc::new(FileHandleCache::new(0)));
        let mut buf = [0u8; 2];
        src.read_range(&path, 2, &mut buf).unwrap();
        src.read_range(&path, 4, &mut buf).unwrap();
        assert_eq!(&buf, b"45");
        assert_eq!(env.opens.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_cache_invalidate_blocks_stale_path_reuse() {
        // Path re-use after delete+re-create (file-number rollback) must
        // never serve the old inode's bytes through a cached handle.
        let dir = scratch_dir("invalidate");
        let path = dir.join("sst.bin");
        write_file(&path, b"OLD-old-OLD!");

        let env = CountingEnv::default();
        let cache = Arc::new(FileHandleCache::new(DEFAULT_TEST_CACHE));
        let src = CachedEnvSource::new(env.clone(), Arc::clone(&cache));
        let mut buf = [0u8; 4];
        src.read_range(&path, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"OLD-");

        // Delete + re-create at the same path, with the delete-side
        // invalidation the Db performs (`remove_db_file`).
        fs::remove_file(&path).unwrap();
        cache.invalidate(&path);
        write_file(&path, b"NEW-new-NEW!");

        src.read_range(&path, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"NEW-");
        assert_eq!(env.opens.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_cache_evicts_least_recently_used_at_capacity() {
        let dir = scratch_dir("evict");
        let env = CountingEnv::default();
        let src = CachedEnvSource::new(env.clone(), Arc::new(FileHandleCache::new(2)));
        let mut buf = [0u8; 1];
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let c = dir.join("c.bin");
        for p in [&a, &b, &c] {
            write_file(p, b"x");
        }

        src.read_range(&a, 0, &mut buf).unwrap();
        src.read_range(&b, 0, &mut buf).unwrap();
        assert_eq!(env.opens.load(Ordering::SeqCst), 2);
        // Inserting c evicts a (LRU): re-reading a re-opens; b stays cached.
        src.read_range(&c, 0, &mut buf).unwrap();
        src.read_range(&b, 0, &mut buf).unwrap();
        assert_eq!(env.opens.load(Ordering::SeqCst), 3);
        src.read_range(&a, 0, &mut buf).unwrap();
        assert_eq!(env.opens.load(Ordering::SeqCst), 4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advise_default_and_std_are_best_effort() {
        let dir = std::env::temp_dir().join(format!(
            "pedra-advise-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&[0u8; 4096]).unwrap();
            f.sync_all().unwrap();
        }
        // StdEnv: Linux posix_fadvise via pedradb-posix; no-op elsewhere.
        StdEnv.advise(&path, 0, 0, AdviseKind::Random).unwrap();
        StdEnv.advise(&path, 0, 4096, AdviseKind::WillNeed).unwrap();
        StdEnv.advise(&path, 0, 4096, AdviseKind::DontNeed).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fdatasync_file_flushes_without_error() {
        let dir = std::env::temp_dir().join(format!(
            "pedra-fdatasync-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wal.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"pedra").unwrap();
        fdatasync_file(&f).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    /// RFC-0041 P1.2: a lone 1-op write that `fdatasync`s before Ok cannot
    /// reach 2× Rocks **async** YCSB-A on this class of disk. head3 Rocks A
    /// is 201_917 qps; 2.0× needs ≤ 2.48 µs/op. One real `fdatasync` is
    /// tens of µs. This is the shipped G1 primitive, not a mock.
    #[test]
    fn rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!(
            "pedra-fd-ceiling-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wal.bin");
        let mut f = File::create(&path).unwrap();
        let payload = [0xab_u8; 1024];
        let mut samples = Vec::with_capacity(300);
        for _ in 0..300 {
            f.write_all(&payload).unwrap();
            let t = Instant::now();
            fdatasync_file(&f).unwrap();
            samples.push(t.elapsed());
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        // 1 / 201_917 / 2  (head3 rocks ycsb_a × 2.0)
        let budget_2x_a = Duration::from_nanos(2_476);
        assert!(
            p50 > budget_2x_a,
            "fdatasync p50 {p50:?} ≤ {budget_2x_a:?} — P1.2 1c A would be open on this box"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Helper: join dir + name (avoids pulling [`PathBuf`] logic into every caller).
#[must_use]
pub fn join(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}
