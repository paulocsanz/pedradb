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

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use crate::error::Result;

pub mod crc;
pub mod format;
pub mod reader;
pub mod writer;

pub use reader::WalReader;
pub use writer::WalWriter;

/// High-level, file-backed WAL with real durability semantics.
///
/// Wraps a [`WalWriter<File>`] and exposes `sync_all`/`sync_data` for fsync.
/// For unit tests that don't need the filesystem, use [`WalWriter`] directly
/// over a `Cursor`.
pub struct Wal {
    writer: WalWriter<File>,
}

impl Wal {
    /// Create (or truncate) a fresh WAL at `path`.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the file cannot be created or opened.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            writer: WalWriter::new(file)?,
        })
    }

    /// Open an existing WAL for appending (positioned at end).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the file cannot be opened or seeked.
    pub fn append<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        // Position at end so the writer computes the correct in-block offset.
        file.seek(SeekFrom::End(0))?;
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

    /// Flush + `fdatasync` (sync data only). Cheaper than [`Self::sync_all`];
    /// sufficient when file metadata (size) is already stable, i.e. an
    /// existing appended-to file.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `sync_data`.
    pub fn sync_data(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.inner_mut().sync_data()?;
        Ok(())
    }

    /// Flush + `fsync` (data + metadata). Required after creating/truncating
    /// the file so its new size is durable.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `fsync`.
    pub fn sync_all(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.inner_mut().sync_all()?;
        Ok(())
    }

    /// Replay every complete logical record in the WAL file at `path`, in
    /// write order. A truncated trailing record (from a crash) is skipped.
    ///
    /// On a CRC error, replay stops and the error is returned; callers can
    /// decide whether to truncate or halt.
    ///
    /// # Errors
    /// Returns [`crate::error::CoreError`] on read failure or CRC mismatch.
    pub fn recover<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<u8>>> {
        let file = File::open(path)?;
        WalReader::new(BufReader::new(file)).collect_all()
    }

    /// Flush and close the underlying file.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if flushing fails.
    pub fn close(mut self) -> Result<()> {
        self.writer.flush()
    }
}
