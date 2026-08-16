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
    /// Default is a **no-op**. Linux `posix_fadvise` lives in `pedradb-io-uring`
    /// (`IoUringEnv`) so this crate stays `#![forbid(unsafe_code)]`. Sim / DST
    /// envs inherit the no-op. Errors are best-effort — callers must not fail
    /// the request on advise failure.
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
        // Same class as WAL/SST (RFC-0036): Apple File::sync_all is F_FULLFSYNC.
        fdatasync_file(&dir)
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
        // StdEnv is always a no-op (Linux fadvise is IoUringEnv).
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
}

/// Helper: join dir + name (avoids pulling [`PathBuf`] logic into every caller).
#[must_use]
pub fn join(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}
