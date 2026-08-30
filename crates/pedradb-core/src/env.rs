//! Filesystem seam for PedraDB (RBS/`depot-store` `Media` pattern, sync).
//!
//! Production default is [`StdEnv`] (passthrough to `std::fs`). On Linux, use
//! the `pedradb-io-uring` crate (`IoUringEnv`) for **`io_uring`** write + fsync
//! without changing engine code. Tests and `pedradb-sim` inject faults via a
//! wrapping [`Env`] (e.g. `FailingEnv`) without rewriting the engine.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

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
}

/// Hint for [`Env::advise`] (RFC-0029 P1.2 — `posix_fadvise`-shaped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdviseKind {
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
