//! Write-Ahead Log (WAL) — Fatia 1.
//!
//! Append-only, crash-safe durability log. A logical record is written before
//! the corresponding `MemTable` mutation is acknowledged, so that after a crash
//! the engine can replay the log to reconstruct in-memory state.
//!
//! The on-disk format is block-based (32 KiB blocks) with per-physical-record
//! masked CRC32C and `First`/`Middle`/`Last` fragmentation, mirroring
//! RocksDB's `db/log_format.h`. See [`format`] and [`crc`] for details.
//!
//! # Crash safety contract
//! A record is durable only after [`Wal::sync_all`] (or `sync_data`) returns.
//! A partial trailing record left by a crash is silently skipped on recovery.

use std::io::BufReader;
use std::path::Path;

use crate::env::{Env, EnvFile, StdEnv};
use crate::error::Result;

pub mod crc;
pub mod format;
pub mod reader;
pub mod recover_choose;
pub mod recover_kernel;
pub mod writer;

pub use reader::WalReader;
pub use writer::WalWriter;

/// High-level, file-backed WAL with real durability semantics.
///
/// Wraps a [`WalWriter`] over an [`EnvFile`] and exposes `sync_all`/`sync_data`
/// for fsync. For unit tests that don't need the filesystem, use [`WalWriter`]
/// directly over a `Cursor`.
pub struct Wal<F: EnvFile = <StdEnv as Env>::File> {
    writer: WalWriter<F>,
}

impl Wal<<StdEnv as Env>::File> {
    /// Create (or truncate) a fresh WAL at `path` on the real filesystem.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the file cannot be created or opened.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::create_on(&StdEnv, path)
    }

    /// Open an existing WAL for appending on the real filesystem.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the file cannot be opened or seeked.
    pub fn append<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::append_on(&StdEnv, path)
    }

    /// Replay every complete logical record (real filesystem).
    ///
    /// # Errors
    /// Read failure or CRC mismatch.
    pub fn recover<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<u8>>> {
        Self::recover_on(&StdEnv, path)
    }

    /// Replay from byte offset (real filesystem).
    ///
    /// # Errors
    /// I/O, CRC, or invalid offset.
    pub fn recover_from_offset<P: AsRef<Path>>(path: P, offset: u64) -> Result<Vec<Vec<u8>>> {
        Self::recover_from_offset_on(&StdEnv, path, offset)
    }
}

impl<F: EnvFile> Wal<F> {
    /// Create (or truncate) a fresh WAL via `env`.
    ///
    /// # Errors
    /// Env I/O.
    pub fn create_on<E: Env<File = F>, P: AsRef<Path>>(env: &E, path: P) -> Result<Self> {
        let file = env.create(path.as_ref())?;
        Ok(Self {
            writer: WalWriter::new(file)?,
        })
    }

    /// Open existing WAL for appending via `env` (creates if missing).
    ///
    /// # Errors
    /// Env I/O.
    pub fn append_on<E: Env<File = F>, P: AsRef<Path>>(env: &E, path: P) -> Result<Self> {
        let file = env.open_append(path.as_ref())?;
        Ok(Self {
            writer: WalWriter::new(file)?,
        })
    }

    /// Append one logical record. Not durable until [`Self::sync_all`] /
    /// [`Self::sync_data`] is called.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying file.
    pub fn append_record(&mut self, data: &[u8]) -> Result<()> {
        self.writer.add_record(data)
    }

    /// Flush + `fdatasync` (sync data only).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `sync_data`.
    pub fn sync_data(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.inner_mut().sync_data()?;
        Ok(())
    }

    /// Flush + `fsync` (data + metadata).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `fsync`.
    pub fn sync_all(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.inner_mut().sync_all()?;
        Ok(())
    }

    /// Replay every complete logical record via `env`.
    ///
    /// On a CRC error, replay stops and the error is returned; callers can
    /// decide whether to truncate or halt. A truncated trailing record is
    /// skipped.
    ///
    /// # Errors
    /// Read failure or CRC mismatch.
    pub fn recover_on<E: Env<File = F>, P: AsRef<Path>>(env: &E, path: P) -> Result<Vec<Vec<u8>>> {
        let file = env.open_read(path.as_ref())?;
        WalReader::new(BufReader::new(file)).collect_all()
    }

    /// Replay complete logical records starting at byte `offset` via `env`.
    ///
    /// # Errors
    /// I/O, CRC, or invalid offset.
    pub fn recover_from_offset_on<E: Env<File = F>, P: AsRef<Path>>(
        env: &E,
        path: P,
        offset: u64,
    ) -> Result<Vec<Vec<u8>>> {
        let file = env.open_read(path.as_ref())?;
        WalReader::from_offset(BufReader::new(file), offset)?.collect_all()
    }

    /// Current end offset of the WAL (after flush); use for export cursors.
    ///
    /// # Errors
    /// I/O from flush or seeking.
    pub fn stream_position(&mut self) -> Result<u64> {
        self.writer.stream_position()
    }

    /// Flush buffered WAL data without taking ownership (for `Db` paths with `Drop`).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if flushing fails.
    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }

    /// Flush and close the underlying file.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if flushing fails.
    pub fn close(mut self) -> Result<()> {
        self.flush()
    }
}
