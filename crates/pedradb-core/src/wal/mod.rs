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
pub mod reopen_kernel;
pub mod writer;

pub use reader::WalReader;
pub use writer::WalWriter;

/// Space reservation chunk for a WAL segment (Darwin `F_PREALLOCATE`,
/// Linux `fallocate(FALLOC_FL_KEEP_SIZE)`).
///
/// APFS assigns a fresh extent when a plain append crosses an ~8 MiB
/// boundary; that `write(2)` blocks 10–50 ms inside the commit path
/// (`findings/2026-08-22-rearm7/`). Linux G1 `fdatasync` of a growing WAL
/// pays delayed-allocation in the Ok path unless extents exist already
/// (RFC-0062 P1.1). Segments reserve this much storage past physical EOF
/// up front (lazily, on first write) and re-reserve as the segment grows.
/// RocksDB `PosixWritableFile::Allocate` does the same.
const WAL_PREALLOC_CHUNK: u64 = 8 * 1024 * 1024;

/// High-level, file-backed WAL with real durability semantics.
///
/// Wraps a [`WalWriter`] over an [`EnvFile`] and exposes `sync_all`/`sync_data`
/// for fsync. For unit tests that don't need the filesystem, use [`WalWriter`]
/// directly over a `Cursor`.
pub struct Wal<F: EnvFile = <StdEnv as Env>::File> {
    writer: WalWriter<F>,
    /// Reused logical-record encode buffer (RFC-0040).
    logical: Vec<u8>,
    /// Logical offset believed covered by space reservation. 0 = nothing
    /// reserved yet ([`WAL_PREALLOC_CHUNK`] semantics; best-effort — an env
    /// without support no-ops and the segment simply appends plain).
    prealloc_to: u64,
    /// Every WAL barrier on this DB uses the platform's strongest data
    /// class ([`EnvFile::sync_data_strong`]) — on Darwin
    /// `fcntl(F_FULLFSYNC)`, the CMake-RocksDB `WriteOptions.sync` class.
    /// Set from [`crate::OpenOptions::wal_full_fsync`] (default **true**,
    /// RFC-0036 addendum v2); `false` = `fdatasync` weak class, the
    /// `librocksdb-sys` crate-build class (dev opt-out on Apple hardware).
    full_fsync: bool,
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
            prealloc_to: 0,
            full_fsync: false,
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
            prealloc_to: 0,
            full_fsync: false,
        })
    }

    /// Append one logical record. Not durable until [`Self::sync_all`] /
    /// [`Self::sync_data`] is called.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying file.
    pub fn append_record(&mut self, data: &[u8]) -> Result<()> {
        self.reserve_space(data.len() as u64 + 2 * format::HEADER_SIZE as u64);
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
        self.reserve_space(n + 2 * format::HEADER_SIZE as u64);
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
        let n: u64 = datas.iter().map(|d| d.len() as u64).sum();
        self.reserve_space(n + 2 * format::HEADER_SIZE as u64);
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
    /// Hits the file (`write()`) on every call — G1, async, close, and
    /// `Db::sync` all use it. Async callers skip the `fdatasync` (that is
    /// the only difference from G1), so every acked record reaches the OS
    /// page cache before `Ok` — the same process-crash class as RocksDB
    /// default (`sync=false`, `manual_wal_flush=false` flushes per record).
    ///
    /// # Errors
    /// Underlying file write.
    pub fn write_pending_frame(&mut self) -> Result<()> {
        let mut frame = self.writer.take_frame();
        if frame.is_empty() {
            self.writer.restore_frame(frame);
            return Ok(());
        }
        self.reserve_space(frame.len() as u64);
        let r = self.writer.write_frame(&frame);
        frame.clear();
        self.writer.restore_frame(frame);
        r
    }

    /// Keep [`WAL_PREALLOC_CHUNK`] of storage reserved ahead of the append
    /// point. Best-effort: on an env without support (or a failed
    /// reservation) the segment appends plain and the frontier stays put —
    /// the next write just retries.
    fn reserve_space(&mut self, upcoming: u64) {
        let pos = self.writer.position();
        // Bytes below `pos` are written (allocated); anchor the frontier
        // there so a recovered segment never over-reserves.
        self.prealloc_to = self.prealloc_to.max(pos);
        let need = pos
            .saturating_add(upcoming)
            .saturating_add(WAL_PREALLOC_CHUNK);
        while self.prealloc_to < need {
            // `F_PEOFPOSMODE` allocates past physical EOF; the invariant
            // physEOF ≥ prealloc_to makes each call cover exactly one chunk.
            if self
                .writer
                .inner_mut()
                .preallocate(WAL_PREALLOC_CHUNK)
                .is_err()
            {
                break;
            }
            self.prealloc_to = self.prealloc_to.saturating_add(WAL_PREALLOC_CHUNK);
        }
    }

    /// Flush + `fdatasync` (sync data only).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from flush or `sync_data`.
    pub fn sync_data(&mut self) -> Result<()> {
        self.write_pending_frame()?;
        self.writer.flush()?;
        if self.full_fsync {
            self.writer.inner_mut().sync_data_strong()?;
        } else {
            self.writer.inner_mut().sync_data()?;
        }
        Ok(())
    }

    /// Switch the barrier class of every subsequent WAL sync on this handle
    /// ([`Self::sync_data`]) to the platform's strongest data barrier.
    /// WAL rotation (`Db`) carries the flag to the new segment. See
    /// [`EnvFile::sync_data_strong`] for the class table (RFC-0036 addendum).
    pub fn set_full_fsync(&mut self, on: bool) {
        self.full_fsync = on;
    }

    /// Whether this WAL syncs with the strong barrier class.
    #[must_use]
    pub fn full_fsync(&self) -> bool {
        self.full_fsync
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
    ) -> Result<(Vec<Vec<u8>>, u64, Option<u64>)> {
        let file = env.open_read(path.as_ref())?;
        let mut reader = WalReader::new(BufReader::new(file));
        let records = reader.collect_all()?;
        let end = reader.last_good_offset();
        Ok((records, end, reader.resync_origin()))
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
    ) -> Result<(Vec<Vec<u8>>, u64, Option<CoreError>, Option<u64>)> {
        let file = env.open_read(path.as_ref())?;
        let mut reader = WalReader::new(BufReader::new(file));
        let (records, err) = reader.collect_prefix_all();
        let end = reader.last_good_offset();
        Ok((records, end, err, reader.resync_origin()))
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

#[cfg(test)]
mod probe_tests {
    use super::*;

    #[test]
    fn prealloc_keeps_logical_size_and_recovers() {
        let dir = std::env::temp_dir().join(format!("wal-prealloc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wal.log");

        // Cross several frames so `reserve_space` runs and the file
        // grows past one write.
        let val = bytes::Bytes::from(vec![b'p'; 1024]);
        let mut ops: Vec<crate::batch::WriteOp> = Vec::new();
        for i in 1..=1024u64 {
            ops.push(crate::batch::WriteOp::put(
                i,
                format!("k/{i:06}"),
                val.clone(),
            ));
        }
        let mut w = Wal::create(&path).unwrap();
        for _ in 0..8 {
            w.append_write_ops(&ops).unwrap();
            w.write_pending_frame().unwrap();
        }
        let append_end = w.stream_position().unwrap();
        drop(w);
        // Reservation covers physical space only: the logical size must
        // stay at the append point (readers never see reserved zeros).
        assert_eq!(StdEnv.metadata_len(&path).unwrap(), append_end);

        // Crash-shaped reopen: recover exactly the appended records;
        // last-good offset == append point == file size.
        let (recs, end, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
        assert_eq!(recs.len(), 8);
        assert_eq!(end, append_end);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0044 P2.2 micro: deps_raftlog WAL floor —
    /// `encode_write_op_batches` + frame `write()` only (no Db lock,
    /// memtable, or publish). Run:
    /// `cargo test -p pedradb-core --lib --release wal_encode_raftlog_micro -- --ignored --nocapture`
    /// `WAL_MICRO_OPS` sets ops/batch (default 16), `WAL_MICRO_N` batches.
    #[test]
    #[ignore]
    fn wal_encode_raftlog_micro() {
        let dir = std::env::temp_dir().join(format!("wal-micro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = Wal::create(dir.join("wal.log")).unwrap();
        let per: usize = std::env::var("WAL_MICRO_OPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16);
        let n: u64 = std::env::var("WAL_MICRO_N")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200_000);
        let val = bytes::Bytes::from(vec![b'r'; 100]);
        let mut ops: Vec<crate::batch::WriteOp> = Vec::with_capacity(per);
        for i in 1..=per as u64 {
            ops.push(crate::batch::WriteOp::put(
                i,
                format!("raftlog/{i:08}"),
                val.clone(),
            ));
        }
        let sl = ops.as_slice();
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            w.encode_write_op_batches(&[sl]).unwrap();
            w.write_pending_frame().unwrap();
        }
        let el = t0.elapsed();
        println!(
            "wal micro: {n} batches x {per} ops, {el:?} ({:.3} µs/batch, {:.4} µs/op)",
            el.as_secs_f64() * 1e6 / n as f64,
            el.as_secs_f64() * 1e6 / (n as f64 * per as f64),
        );
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
