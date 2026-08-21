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
use crate::error::{CoreError, Result};

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
    /// Reused logical-record encode buffer (RFC-0040).
    logical: Vec<u8>,
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
            logical: Vec::new(),
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
            logical: Vec::new(),
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

    /// Encode `ops` into the reused logical scratch and append (one memcpy).
    ///
    /// # Errors
    /// Same as [`Self::append_record`].
    pub fn append_write_ops(&mut self, ops: &[crate::batch::WriteOp]) -> Result<u64> {
        self.logical.clear();
        crate::batch::encode_ops(ops, &mut self.logical);
        let n = self.logical.len() as u64;
        self.writer.add_record(&self.logical)?;
        Ok(n)
    }

    /// Append several logical records with **one** `write` (group commit).
    ///
    /// Byte stream identical to [`Self::append_record`] per record; all
    /// records land or none do (single write).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying file.
    pub fn append_records(&mut self, datas: &[&[u8]]) -> Result<()> {
        self.writer.add_records(datas)
    }

    /// Encode each batch into the pending WAL frame (no `write` syscall).
    ///
    /// Caller must [`Self::write_pending_frame`] before `fdatasync` so the
    /// Db write lock is not held across the write (RFC-0041). Same bytes as
    /// `encode_ops` + [`Self::append_records`]. RFC-0042 P1.3: fields go
    /// straight into the frame (no logical scratch pass).
    ///
    /// # Errors
    /// None today (encode is infallible); `Result` matches the append path.
    pub fn encode_write_op_batches(&mut self, batches: &[&[crate::batch::WriteOp]]) -> Result<u64> {
        if batches.is_empty() {
            return Ok(0);
        }
        let mut frame = self.writer.take_frame();
        let mut n = 0u64;
        for ops in batches {
            n = n.saturating_add(self.writer.fragment_encoded_len(ops, &mut frame) as u64);
        }
        self.writer.restore_frame(frame);
        Ok(n)
    }

    /// Write the frame built by [`Self::encode_write_op_batches`].
    ///
    /// Always hits the file (G1 / close / `Db::sync`). Prefer
    /// [`Self::write_pending_frame_if`] on the async put path.
    ///
    /// # Errors
    /// Underlying file write.
    pub fn write_pending_frame(&mut self) -> Result<()> {
        self.write_pending_frame_if(true)
    }

    /// Append encoded WAL bytes.
    ///
    /// - `force` (G1 / close / `sync`): `write()` the whole frame, then the
    ///   caller `fdatasync`s.
    /// - async (`force=false`): `write()` when the frame reaches
    ///   [`format::ASYNC_WAL_BUFFER`] (64 KiB, Rocks
    ///   `writable_file_max_buffer_size`). **Not** 1 MiB, not unencoded
    ///   pending. Process crash can lose the tail (&lt; 64 KiB), like Rocks
    ///   `sync=false`. Power loss can lose more.
    ///
    /// Bytes are always encoded into the frame before Ok (WAL v2 intern is
    /// still a real record).
    ///
    /// # Errors
    /// Underlying file write.
    pub fn write_pending_frame_if(&mut self, force: bool) -> Result<()> {
        let mut frame = self.writer.take_frame();
        if frame.is_empty() {
            self.writer.restore_frame(frame);
            return Ok(());
        }
        if !force && frame.len() < format::ASYNC_WAL_BUFFER {
            self.writer.restore_frame(frame);
            return Ok(());
        }
        let r = self.writer.write_frame(&frame);
        frame.clear();
        self.writer.restore_frame(frame);
        r
    }

    /// Flush + `fdatasync` (sync data only).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `sync_data`.
    pub fn sync_data(&mut self) -> Result<()> {
        self.write_pending_frame()?;
        self.writer.flush()?;
        self.writer.inner_mut().sync_data()?;
        Ok(())
    }

    /// Flush + `fsync` (data + metadata).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `fsync`.
    pub fn sync_all(&mut self) -> Result<()> {
        self.write_pending_frame()?;
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

    /// Like [`Self::recover_on`] but also returns the stream offset just past
    /// the last recovered record — the last known-good append point. Callers
    /// that keep appending to an existing WAL should truncate it to this
    /// offset (via `EnvFile::set_len`) when it is below EOF, so the damaged /
    /// torn region is never re-read as records.
    ///
    /// # Errors
    /// Read failure or CRC mismatch.
    pub fn recover_span_on<E: Env<File = F>, P: AsRef<Path>>(
        env: &E,
        path: P,
    ) -> Result<(Vec<Vec<u8>>, u64)> {
        let file = env.open_read(path.as_ref())?;
        let mut reader = WalReader::new(BufReader::new(file));
        let records = reader.collect_all()?;
        let end = reader.last_good_offset();
        Ok((records, end))
    }

    /// RFC-0047 P0.2: point-in-time recovery probe — returns the decoded
    /// prefix, the last known-good append offset, and the error that stopped
    /// collection (`None` on a clean end of log).
    ///
    /// # Errors
    /// I/O opening/reading the log (not the corruption itself — that is the
    /// returned `Option<CoreError>`).
    pub fn recover_prefix_span_on<E: Env<File = F>, P: AsRef<Path>>(
        env: &E,
        path: P,
    ) -> Result<(Vec<Vec<u8>>, u64, Option<CoreError>)> {
        let file = env.open_read(path.as_ref())?;
        let mut reader = WalReader::new(BufReader::new(file));
        let (records, err) = reader.collect_prefix_all();
        let end = reader.last_good_offset();
        Ok((records, end, err))
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
        self.write_pending_frame()?;
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
