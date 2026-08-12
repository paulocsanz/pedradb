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
}

impl EnvFile for File {
    fn sync_data(&mut self) -> io::Result<()> {
        File::sync_data(self)
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
}

/// Helper: join dir + name (avoids pulling [`PathBuf`] logic into every caller).
#[must_use]
pub fn join(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}
