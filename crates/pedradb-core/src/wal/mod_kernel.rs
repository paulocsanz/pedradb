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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::env::{Env, EnvFile, StdEnv};

fn walfd_diag_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PEDRA_FDSYNC_DIAG").is_some())
}

/// RFC-0209: staging cap. Default **on** (64 KiB, Rocks WritableFile).
/// `PEDRA_WAL_BUFFER=0`/`false` restores one-write-per-frame. Lone
/// writers still drain every frame (`write_pending_frame_lone`) so 1c
/// does not wait for the cap (p209b regression). Read per segment
/// construction, never cached.
fn wal_staged_max() -> u64 {
    match std::env::var("PEDRA_WAL_BUFFER") {
        Ok(s) if s == "0" || s.eq_ignore_ascii_case("false") => 0,
        _ => std::env::var_os("PEDRA_WAL_BUF_MAX")
            .and_then(|v| v.to_str().and_then(|s| s.parse::<u64>().ok()))
            .filter(|&v| v > 0)
            .unwrap_or(crate::wal_buffer_kernel::WAL_BUF_MAX_DEFAULT_BYTES),
    }
}

/// RFC-0233 / RFC-0237: production WAL sink is mmap-on-Shared
/// (`write_wal_at_shared`) when the Env has positional writes. Darwin
/// FileExt pwrite lost to `write()` (P0.4); mmap is the FlushWAL class
/// on both unix hosts. `PEDRA_WAL_PWRITE=0` test opt-out (sequential
/// `write()`); `=1` forces Shared on (tests). Default on for unix.
fn wal_pwrite_enabled() -> bool {
    match std::env::var("PEDRA_WAL_PWRITE").as_deref() {
        Ok("0") | Ok("false") => false,
        Ok("1") | Ok("true") => true,
        _ => cfg!(unix),
    }
}
use crate::error::{CoreError, Result};

#[path = "crc_kernel.rs"]
pub mod crc;
#[path = "format_kernel.rs"]
pub mod format;
#[path = "reader_kernel.rs"]
pub mod reader;
#[path = "recover_choose_kernel.rs"]
pub mod recover_choose;
pub mod recover_kernel;
pub mod reopen_kernel;
pub mod wal_state_kernel;
#[path = "writer_kernel.rs"]
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
///
/// 64 MiB (was 8): the 15M-hydrate profile caught the writer spending
/// 4.1 s per 25 s window inside `preallocate_file` on this path — each
/// reservation is a blocking `fcntl(F_PREALLOCATE)`/`fallocate` on the
/// commit thread. 8× fewer of them for the same extent property (still
/// well past the 8 MiB APFS boundary); the tail waste is bounded by one
/// chunk past the frontier per live segment.
const WAL_PREALLOC_CHUNK: u64 = 64 * 1024 * 1024;

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
    /// RFC-0233: positional off-lock write when the Env supports it.
    /// Test opt-out: `PEDRA_WAL_PWRITE=0`.
    pwrite: bool,
    inflight: AtomicUsize,
}

/// Off-lock positional write (RFC-0193). The meta lock is not held.
pub(crate) struct PwriteJob<F: EnvFile> {
    buf: Vec<u8>,
    ticket: u64,
    file: Arc<F>,
}

impl<F: EnvFile> PwriteJob<F> {
    /// Write the reserved span on the **same** File (`Arc` clone, not
    /// `dup(2)`). Host File is mmap memcpy (`write_wal_at_shared`) — the
    /// WAL CS is encode+ticket; this I/O does not hold `wal.lock()`.
    /// Returns `(ticket, len)` so the caller can [`Wal::finish_pwrite`].
    ///
    /// # Errors
    /// Underlying positional / mmap write.
    pub(crate) fn run(self) -> Result<(u64, u64)> {
        let len = self.buf.len() as u64;
        self.file.write_wal_at_shared(&self.buf, self.ticket)?;
        Ok((self.ticket, len))
    }
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
        let mut writer = WalWriter::new(file)?;
        let staged_max = wal_staged_max();
        if staged_max > 0 {
            writer.enable_staging(staged_max);
        }
        let mut wal = Self {
            writer,
            logical: Vec::new(),
            prealloc_to: 0,
            full_fsync: false,
            pwrite: wal_pwrite_enabled(),
            inflight: AtomicUsize::new(0),
        };
        // Fjall `set_len(64 MiB)` at journal create, not on the first put.
        // `upcoming=1` so the first write's `pos+len+CHUNK` is already
        // covered — `reserve_space(0)` left `prealloc_to == CHUNK` and the
        // first put paid a second 64 MiB `F_PREALLOCATE` (WRITEPHASE wal
        // 40 ms / 2k commits).
        wal.reserve_space(1);
        Ok(wal)
    }

    /// Open existing WAL for appending via `env` (creates if missing).
    ///
    /// # Errors
    /// Env I/O.
    pub fn append_on<E: Env<File = F>, P: AsRef<Path>>(env: &E, path: P) -> Result<Self> {
        let file = env.open_rw(path.as_ref())?;
        let mut writer = WalWriter::new(file)?;
        let staged_max = wal_staged_max();
        if staged_max > 0 {
            writer.enable_staging(staged_max);
        }
        let mut wal = Self {
            writer,
            logical: Vec::new(),
            prealloc_to: 0,
            full_fsync: false,
            pwrite: wal_pwrite_enabled(),
            inflight: AtomicUsize::new(0),
        };
        wal.reserve_space(1);
        Ok(wal)
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
        if crate::write_admission_kernel::batch_is_empty(batches.len() as u64) {
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
        self.write_pending_inner(false)
    }

    /// Lone/1c path: stage then **drain** so each Ok is in the page cache
    /// (Rocks `FlushWAL` per `Write()`). Production drain is mmap memcpy
    /// (`write_wal_at_shared`); still before Ok — not a userspace lie.
    /// Group path uses [`Self::write_pending_frame`] and only flushes at
    /// the 64 KiB cap.
    pub fn write_pending_frame_lone(&mut self) -> Result<()> {
        self.write_pending_inner(true)
    }

    fn write_pending_inner(&mut self, lone_writer: bool) -> Result<()> {
        let mut frame = self.writer.take_frame();
        if crate::write_admission_kernel::batch_is_empty(frame.len() as u64) {
            self.writer.restore_frame(frame);
            return Ok(());
        }
        self.reserve_space(frame.len() as u64);
        // Staging (0209) stays on write_pending_frame (group). Lone 1c
        // skips the staging hop — one memcpy into the mmap sink before Ok
        // (FlushWAL / page cache). Off-lock pwrite is `take_pwrite_job`.
        let r = if lone_writer {
            self.writer.write_lone_frame(&frame)
        } else {
            self.writer.write_frame(&frame)
        };
        frame.clear();
        self.writer.restore_frame(frame);
        r
    }

    /// Pull the pending frame as an off-lock pwrite job (`Arc` clone of the
    /// same File — not `dup(2)`). `None` = caller must
    /// [`Self::write_pending_frame`] (AS-IS / no positional capability).
    pub(crate) fn take_pwrite_job(&mut self) -> Result<Option<PwriteJob<F>>> {
        if !crate::wal_ticket_kernel::pwrite_off_lock(self.pwrite, self.writer.positional_writes())
        {
            return Ok(None);
        }
        let mut frame = self.writer.take_frame();
        if crate::write_admission_kernel::batch_is_empty(frame.len() as u64) {
            self.writer.restore_frame(frame);
            return Ok(None);
        }
        let Some(file) = self.writer.share_pwrite() else {
            // Positional but not shared (should not happen on host File):
            // write under the lock at the ticket.
            self.reserve_space(frame.len() as u64);
            let ticket = self.writer.reserve_pending(frame.len() as u64);
            if let Err(e) = self.writer.write_all_at(&frame, ticket) {
                self.writer.restore_frame(frame);
                return Err(e);
            }
            frame.clear();
            self.writer.restore_frame(frame);
            return Ok(None);
        };
        self.reserve_space(frame.len() as u64);
        let ticket = self.writer.reserve_pending(frame.len() as u64);
        self.inflight.fetch_add(1, Ordering::Release);
        Ok(Some(PwriteJob {
            buf: frame,
            ticket,
            file,
        }))
    }

    /// Ticket a pre-encoded Full record (RFC-0237). Encode/CRC happened
    /// off `wal.lock()`; this CS is pad + `reserve_frame` + Arc clone.
    /// `None` = Exclusive sink or the record does not fit this block —
    /// caller fragments via [`Self::encode_write_op_batches`].
    pub(crate) fn take_preframed_pwrite_job(
        &mut self,
        mut frame: Vec<u8>,
    ) -> Result<Option<PwriteJob<F>>> {
        if crate::write_admission_kernel::batch_is_empty(frame.len() as u64) {
            return Ok(None);
        }
        if !crate::wal_ticket_kernel::pwrite_off_lock(self.pwrite, self.writer.positional_writes())
        {
            return Ok(None);
        }
        let Some(file) = self.writer.share_pwrite() else {
            return Ok(None);
        };
        let rec_len = frame.len();
        let Some(pad) = self.writer.take_preframed_layout(rec_len) else {
            return Ok(None);
        };
        if pad > 0 {
            let mut padded = Vec::with_capacity(pad + rec_len);
            padded.resize(pad, 0);
            padded.append(&mut frame);
            frame = padded;
        }
        self.reserve_space(frame.len() as u64);
        let ticket = self.writer.reserve_pending(frame.len() as u64);
        self.inflight.fetch_add(1, Ordering::Release);
        Ok(Some(PwriteJob {
            buf: frame,
            ticket,
            file,
        }))
    }

    /// Record a completed off-lock pwrite: move the written frontier to
    /// `ticket+len` (not at reserve) and drop the inflight count.
    /// `len == 0` is the I/O-failure path — inflight still drops, position
    /// stays at the last committed byte.
    pub(crate) fn finish_pwrite(&mut self, ticket: u64, len: u64) {
        if !crate::write_admission_kernel::batch_is_empty(len) {
            self.writer.commit_pwrite(ticket, len);
        }
        self.inflight.fetch_sub(1, Ordering::AcqRel);
    }

    fn wait_inflight(&self) {
        while self.inflight.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
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
            if self.writer.preallocate_extent(WAL_PREALLOC_CHUNK).is_err() {
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
        // PEDRA_FDSYNC_DIAG: the per-batch G1 barrier lives here — the
        // underlying `E::File` may resolve to the inherent std method
        // (Linux `fdatasync` inside std), bypassing the posix choke-point
        // counter, so count at this seam instead.
        if !walfd_diag_enabled() {
            return self.sync_data_inner();
        }
        let t0 = std::time::Instant::now();
        let out = self.sync_data_inner();
        let us = t0.elapsed().as_micros() as u64;
        static NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static MAX_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        use std::sync::atomic::Ordering::Relaxed;
        NS.fetch_add(us * 1000, Relaxed);
        MAX_US.fetch_max(us, Relaxed);
        let n = N.fetch_add(1, Relaxed) + 1;
        if n % 2048 == 0 {
            println!(
                "WALFDIAG n={n} cum_ms={} avg_us={:.0} max_ms={:.1}",
                NS.load(Relaxed) / 1_000_000,
                (NS.load(Relaxed) / 1000) / n,
                MAX_US.load(Relaxed) as f64 / 1000.0,
            );
        }
        out
    }

    fn sync_data_inner(&mut self) -> Result<()> {
        self.wait_inflight();
        self.write_pending_frame()?;
        self.writer.flush()?;
        self.writer.sync_data_sink(self.full_fsync)?;
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
        self.writer.sync_all_sink()?;
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

    /// Logical bytes written to the current segment (framed payload size;
    /// preallocated space beyond EOF does not count).
    #[must_use]
    pub fn position(&self) -> u64 {
        self.writer.position()
    }

    /// Flush and close the underlying file.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if flushing fails.
    pub fn close(mut self) -> Result<()> {
        self.flush()?;
        self.writer.truncate_to_logical()
    }
}

impl<F: EnvFile> Drop for Wal<F> {
    fn drop(&mut self) {
        let _ = self.writer.drain_staged();
        let _ = self.writer.truncate_to_logical();
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

    /// RFC-0209 env axis: `PEDRA_WAL_BUFFER` is read at segment
    /// construction, and `cargo test` is one process — serialize the
    /// env-flipping tests. Restore on drop so a panic cannot leak the
    /// axis into unrelated tests.
    static RFC0209_ENV_AXIS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Rfc0209EnvGuard;
    impl Drop for Rfc0209EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("PEDRA_WAL_BUFFER");
            std::env::remove_var("PEDRA_WAL_BUF_MAX");
            std::env::remove_var("PEDRA_WAL_PWRITE");
        }
    }

    fn rfc0209_put(seq: u64) -> Vec<crate::batch::WriteOp> {
        vec![crate::batch::WriteOp::put(
            seq,
            format!("k/{seq:04}"),
            bytes::Bytes::from_static(b"payload-0209"),
        )]
    }

    fn rfc0209_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// RFC-0209: `PEDRA_WAL_BUFFER=0` restores one-write-per-frame.
    #[test]
    fn rfc0209_staging_disabled_without_env() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "0");
        std::env::remove_var("PEDRA_WAL_BUF_MAX");

        let dir = rfc0209_dir("nostage");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        for seq in 1..=4u64 {
            let ops = rfc0209_put(seq);
            w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
            w.write_pending_frame().unwrap();
            let (recs, end, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
            assert_eq!(
                recs.len(),
                seq as usize,
                "AS-IS: each frame lands immediately"
            );
            assert!(end > 0);
            // mmap grow pads i_size to 1 MiB; recover stops at a zero
            // header. Logical frontier is stream_position; Drop truncates.
            assert_eq!(end, w.stream_position().unwrap());
        }
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Default ON: group frames stay in user-space below the 64 KiB cap.
    #[test]
    fn rfc0209_default_stages_group_frames() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::remove_var("PEDRA_WAL_BUFFER");
        std::env::remove_var("PEDRA_WAL_BUF_MAX");
        let dir = rfc0209_dir("default-stage");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        for seq in 1..=4u64 {
            let ops = rfc0209_put(seq);
            w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
            w.write_pending_frame().unwrap();
        }
        let (recs, _, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
        assert_eq!(recs.len(), 0, "default group path stages below 64KiB");
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Lone path drains even with default staging (1c = FlushWAL per Write).
    #[test]
    fn rfc0209_lone_drains_under_default_staging() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::remove_var("PEDRA_WAL_BUFFER");
        let dir = rfc0209_dir("lone-drain");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        for seq in 1..=3u64 {
            let ops = rfc0209_put(seq);
            w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
            w.write_pending_frame_lone().unwrap();
            let (recs, _, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
            assert_eq!(recs.len(), seq as usize, "lone Ok is a kernel write");
        }
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0209 P0.2 order rule (c): with a cap above the test's payload
    /// nothing flushes by size — the logical bytes live only in the
    /// user-space buffer (a crash-shaped reader sees none of them), and
    /// `sync_data` drains BEFORE the fd barrier, so every acked record is
    /// durable-visible right after the barrier.
    #[test]
    fn rfc0209_sync_after_staged_flushes_before_fd() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "1");
        std::env::set_var("PEDRA_WAL_BUF_MAX", "1048576");

        let dir = rfc0209_dir("syncdrain");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        for seq in 1..=4u64 {
            let ops = rfc0209_put(seq);
            w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
            w.write_pending_frame().unwrap();
        }
        let (recs, end, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
        assert_eq!(
            recs.len(),
            0,
            "staged bytes are user-side only before the barrier"
        );
        assert_eq!(end, 0);
        w.sync_data().unwrap();
        let (recs, end, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
        assert_eq!(recs.len(), 4, "the barrier drains before the fd");
        assert!(end > 0);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0209 P0.2 order rule (d): `close` drains the staging buffer —
    /// no acked record dies in user space when the segment is closed
    /// without any explicit sync.
    #[test]
    fn rfc0209_close_drains_staged_bytes() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "1");
        std::env::set_var("PEDRA_WAL_BUF_MAX", "1048576");

        let dir = rfc0209_dir("closedrain");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        for seq in 1..=3u64 {
            let ops = rfc0209_put(seq);
            w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
            w.write_pending_frame().unwrap();
        }
        w.close().unwrap();
        let (recs, end, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
        assert_eq!(recs.len(), 3, "close must drain staged bytes");
        assert!(end > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0209 P0.2 acceptance: the same op sequence through the real
    /// file path produces a byte-identical segment in both modes — with
    /// `PEDRA_WAL_BUFFER=1` and a 300-byte cap several size-flushes fire
    /// mid-sequence plus a partial drain at close. Staging changes WHEN
    /// bytes reach the file, never the bytes (`cmp`-identical after close).
    #[test]
    fn rfc0209_buffered_wal_byte_identical_after_close() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;

        let ops: Vec<Vec<crate::batch::WriteOp>> = (0..6u64)
            .map(|seq| {
                let payload = vec![0x5a_u8; (90 + seq * 17) as usize]; // ~100-175 B records
                vec![crate::batch::WriteOp::put(
                    seq + 1,
                    format!("k/{seq:04}"),
                    bytes::Bytes::from(payload),
                )]
            })
            .collect();

        let dir_plain = rfc0209_dir("ident-plain");
        let path_plain = dir_plain.join("wal.log");
        std::env::set_var("PEDRA_WAL_BUFFER", "0");
        std::env::remove_var("PEDRA_WAL_BUF_MAX");
        {
            let mut w = Wal::create(&path_plain).unwrap();
            for ops in &ops {
                w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
                w.write_pending_frame().unwrap();
            }
            w.close().unwrap();
        }

        let dir_staged = rfc0209_dir("ident-staged");
        let path_staged = dir_staged.join("wal.log");
        std::env::set_var("PEDRA_WAL_BUFFER", "1");
        std::env::set_var("PEDRA_WAL_BUF_MAX", "300");
        {
            let mut w = Wal::create(&path_staged).unwrap();
            for ops in &ops {
                w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
                w.write_pending_frame().unwrap();
            }
            w.close().unwrap();
        }

        let plain = std::fs::read(&path_plain).unwrap();
        let staged = std::fs::read(&path_staged).unwrap();
        assert_eq!(plain, staged, "staged segment must be byte-identical");
        assert!(plain.len() > 300, "test shape: multiple flush cycles");
        let _ = std::fs::remove_dir_all(&dir_plain);
        let _ = std::fs::remove_dir_all(&dir_staged);
    }

    /// RFC-0193: file order follows ticket order even if pwrites complete
    /// reverse (holes filled by later writes at lower offsets).
    #[test]
    fn rfc0193_write_all_at_order_equals_tickets() {
        let dir = rfc0209_dir("pwrite-order");
        let path = dir.join("blob");
        let mut f = std::fs::File::create(&path).unwrap();
        use crate::env::EnvFile;
        f.write_all_at(b"CD", 2).unwrap();
        f.write_all_at(b"AB", 0).unwrap();
        drop(f);
        assert_eq!(std::fs::read(&path).unwrap(), b"ABCD");
        assert!(
            std::fs::File::open(&path).unwrap().positional_writes(),
            "unix File is the positional capability"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0193 / RFC-0230 P0.4: pwrite path emits the same WAL bytes as
    /// the sequential AS-IS write on a real file.
    #[test]
    fn rfc0193_pwrite_wal_bytes_match_as_is() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "0");

        let ops: Vec<Vec<crate::batch::WriteOp>> =
            (0..4u64).map(|seq| rfc0209_put(seq + 1)).collect();

        let dir_as = rfc0209_dir("pwrite-asis");
        let path_as = dir_as.join("wal.log");
        std::env::set_var("PEDRA_WAL_PWRITE", "0");
        {
            let mut w = Wal::create(&path_as).unwrap();
            for o in &ops {
                w.encode_write_op_batches(&[o.as_slice()]).unwrap();
                w.write_pending_frame().unwrap();
            }
            w.close().unwrap();
        }

        let dir_pw = rfc0209_dir("pwrite-on");
        let path_pw = dir_pw.join("wal.log");
        std::env::set_var("PEDRA_WAL_PWRITE", "1");
        {
            let mut w = Wal::create(&path_pw).unwrap();
            for o in &ops {
                w.encode_write_op_batches(&[o.as_slice()]).unwrap();
                if let Some(job) = w.take_pwrite_job().unwrap() {
                    let (ticket, len) = job.run().unwrap();
                    w.finish_pwrite(ticket, len);
                } else {
                    w.write_pending_frame().unwrap();
                }
            }
            w.close().unwrap();
        }

        let as_is = std::fs::read(&path_as).unwrap();
        let pwrite = std::fs::read(&path_pw).unwrap();
        assert_eq!(as_is, pwrite, "pwrite WAL must be byte-identical to AS-IS");
        assert!(!as_is.is_empty());
        let _ = std::fs::remove_dir_all(&dir_as);
        let _ = std::fs::remove_dir_all(&dir_pw);
    }

    /// RFC-0193: off-lock pwrite must advance [`Wal::position`] to the
    /// file length after `take_pwrite_job` + `run` + `finish_pwrite`.
    /// Reserve must not move the written frontier — `wal_segment_is_empty`
    /// keys rotation off `position()`, and a stuck 0 skips rotate (WAL leak).
    #[test]
    fn rfc0193_pwrite_job_advances_position_to_file_len() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "0");
        std::env::set_var("PEDRA_WAL_PWRITE", "1");

        let dir = rfc0209_dir("pwrite-pos");
        let path = dir.join("wal.log");
        let ops = rfc0209_put(1);
        let mut w = Wal::create(&path).unwrap();
        w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
        let job = w
            .take_pwrite_job()
            .unwrap()
            .expect("PEDRA_WAL_PWRITE=1 on create_on File must yield a job");
        // Reserve advanced `reserved_to` only — written frontier stays 0
        // until the job commits.
        assert_eq!(
            w.position(),
            0,
            "position must not move at reserve (failed job must not fake a non-empty segment)"
        );
        let (ticket, len) = job.run().unwrap();
        assert!(
            len > 0,
            "framed record is non-empty; got ticket={ticket} len={len}"
        );
        w.finish_pwrite(ticket, len);
        let pos = w.position();
        let file_len = StdEnv.metadata_len(&path).unwrap();
        assert!(
            file_len >= pos,
            "Wal::position ({pos}) must not exceed file length ({file_len}) after off-lock write"
        );
        assert_eq!(
            pos,
            len.saturating_add(ticket),
            "position is the ticket span, not the mmap grow pad"
        );
        assert!(
            !crate::flush_kernel::wal_segment_is_empty(pos),
            "rotation must see a non-empty segment after a committed pwrite job"
        );
        drop(w);
        assert_eq!(
            StdEnv.metadata_len(&path).unwrap(),
            pos,
            "Drop/close must shrink the mmap pad to the logical frontier"
        );
        let recs = Wal::recover(&path).unwrap();
        assert_eq!(
            recs.len(),
            1,
            "padded mmap WAL recovers the committed record"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0233 P0.2: `open_rw` (production append path) is not `O_APPEND` —
    /// pwrite at a ticket offset lands at that offset, not EOF.
    #[test]
    fn rfc0233_pwrite_honors_offset_on_production_append() {
        let dir = rfc0209_dir("rfc0233-rw");
        let path = dir.join("wal.log");
        let mut f = crate::env::StdEnv.open_rw(&path).unwrap();
        use crate::env::{Env, EnvFile};
        f.write_all_at_shared(b"CD", 2).unwrap();
        f.write_all_at_shared(b"AB", 0).unwrap();
        drop(f);
        let got = std::fs::read(&path).unwrap();
        assert!(got.len() >= 4, "open_rw file must contain the pwrite span");
        assert_eq!(
            &got[..4],
            b"ABCD",
            "pwrite on open_rw must honor offset (O_APPEND would append both at EOF)"
        );
        // Production WAL append_on uses open_rw.
        let wal_src = include_str!("mod_kernel.rs");
        assert!(
            wal_src.contains("env.open_rw("),
            "Wal::append_on must open without O_APPEND"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0233 P0.3: group off-lock path clones `Arc`, not the fd.
    #[test]
    fn rfc0233_no_dup_on_group_path() {
        let wal = include_str!("mod_kernel.rs");
        let take = wal
            .split("fn take_pwrite_job")
            .nth(1)
            .expect("take_pwrite_job");
        let take = take.split("fn finish_pwrite").next().expect("finish");
        assert!(
            take.contains("share_pwrite"),
            "take_pwrite_job must Arc-clone the same File"
        );
        assert!(
            !take.contains("try_clone_handle") && !take.contains("try_clone_out"),
            "take_pwrite_job must not dup(2)"
        );
        let lead = include_str!("../concurrent_kernel.rs");
        assert!(
            lead.contains("take_pwrite_job("),
            "group leader still takes the off-lock job"
        );
        let writer = include_str!("writer_kernel.rs");
        assert!(
            writer.contains("write_wal_at_shared"),
            "Shared WAL sink is mmap (write_wal_at_shared), not generic pwrite"
        );
        let run = wal
            .split("pub(crate) fn run(self)")
            .nth(1)
            .expect("PwriteJob::run")
            .split("impl Wal")
            .next()
            .expect("run body");
        assert!(
            run.contains("write_wal_at_shared"),
            "off-lock job I/O is mmap (write_wal_at_shared), not pwrite"
        );
        assert!(
            !run.contains("write_all_at_shared"),
            "off-lock job must not pwrite; mmap is the FlushWAL class"
        );
    }

    /// RFC-0237: detached Full encode + `take_preframed_pwrite_job` is
    /// byte-identical to locked `encode_write_op_batches` + `take_pwrite_job`
    /// at block offset 0 (CRC included).
    #[test]
    fn rfc0237_detached_full_matches_locked_fragment() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "0");
        std::env::set_var("PEDRA_WAL_PWRITE", "1");

        let op = crate::batch::WriteOp::put(
            7,
            bytes::Bytes::from_static(b"k"),
            bytes::Bytes::from_static(b"v"),
        );
        let dir_a = rfc0209_dir("rfc0237-detached-a");
        let path_a = dir_a.join("wal.log");
        {
            let mut w = Wal::create(&path_a).unwrap();
            w.encode_write_op_batches(&[std::slice::from_ref(&op)])
                .unwrap();
            let job = w.take_pwrite_job().unwrap().expect("locked job");
            let (ticket, len) = job.run().unwrap();
            w.finish_pwrite(ticket, len);
            w.close().unwrap();
        }
        let dir_b = rfc0209_dir("rfc0237-detached-b");
        let path_b = dir_b.join("wal.log");
        {
            let mut w = Wal::create(&path_b).unwrap();
            let frame = writer::WalWriter::<std::fs::File>::encode_one_op_full_detached(&op)
                .expect("1-op Full fits in a block");
            let job = w
                .take_preframed_pwrite_job(frame)
                .unwrap()
                .expect("preframed job");
            let (ticket, len) = job.run().unwrap();
            w.finish_pwrite(ticket, len);
            w.close().unwrap();
        }
        let a = std::fs::read(&path_a).unwrap();
        let b = std::fs::read(&path_b).unwrap();
        assert_eq!(
            a, b,
            "off-lock Full record must match locked fragment at offset 0"
        );
        assert!(!a.is_empty());
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// RFC-0237: preframed job recovers after a &lt; HEADER pad at the
    /// block boundary (the CS prepends zeros, then the Full record).
    #[test]
    fn rfc0237_preframed_job_recovers_across_header_pad() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_BUFFER", "0");
        std::env::set_var("PEDRA_WAL_PWRITE", "1");

        let dir = rfc0209_dir("rfc0237-preframed-pad");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        // One Full record sized so leftover after it is 3 (< HEADER).
        // physical = 29 + key_len + val_len; want leftover-3.
        let leftover = format::BLOCK_SIZE;
        let key = bytes::Bytes::from_static(b"x");
        let want_physical = leftover - 3;
        let vallen = want_physical
            .saturating_sub(29)
            .saturating_sub(key.len());
        let filler = crate::batch::WriteOp::put(1, key, bytes::Bytes::from(vec![b'y'; vallen]));
        w.encode_write_op_batches(&[std::slice::from_ref(&filler)])
            .unwrap();
        let job = w.take_pwrite_job().unwrap().expect("Shared pwrite job");
        let (ticket, len) = job.run().unwrap();
        w.finish_pwrite(ticket, len);
        let off = (w.position() as usize) % format::BLOCK_SIZE;
        let left = if off == 0 {
            format::BLOCK_SIZE
        } else {
            format::BLOCK_SIZE - off
        };
        assert!(
            left > 0 && left < format::HEADER_SIZE,
            "filler must land leftover in 1..6, got {left} (pos={})",
            w.position()
        );
        let last = crate::batch::WriteOp::put(
            2,
            bytes::Bytes::from_static(b"pad-key"),
            bytes::Bytes::from_static(b"pad-val"),
        );
        let frame = writer::WalWriter::<std::fs::File>::encode_one_op_full_detached(&last)
            .expect("Full record");
        let job = w
            .take_preframed_pwrite_job(frame)
            .unwrap()
            .expect("preframed job after header pad");
        let (ticket, len) = job.run().unwrap();
        w.finish_pwrite(ticket, len);
        w.close().unwrap();
        let recs = Wal::recover(&path).unwrap();
        assert_eq!(
            recs.len(),
            2,
            "pad zeros are not records; filler + last recover"
        );
        let decoded = crate::batch::WriteRecord::decode(recs.last().expect("last"))
            .expect("last logical record");
        assert_eq!(decoded.ops.len(), 1);
        assert_eq!(&decoded.ops[0].key[..], b"pad-key");
        assert_eq!(&decoded.ops[0].value[..], b"pad-val");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0233 P1.3: 1c FlushWAL drain on the production mmap sink is
    /// visible to a crash-shaped reader (page cache, not a userspace lie).
    #[test]
    fn rfc0233_mmap_lone_drain_is_recoverable() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::remove_var("PEDRA_WAL_PWRITE");
        std::env::remove_var("PEDRA_WAL_BUFFER");
        let dir = rfc0209_dir("rfc0233-mmap-lone");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        for seq in 1..=3u64 {
            let ops = rfc0209_put(seq);
            w.encode_write_op_batches(&[ops.as_slice()]).unwrap();
            w.write_pending_frame_lone().unwrap();
            let (recs, _, _) = Wal::recover_span_on(&StdEnv, &path).unwrap();
            assert_eq!(
                recs.len(),
                seq as usize,
                "lone Ok must be page-cache visible (FlushWAL); mmap pad zeros are not records"
            );
        }
        let pos = w.position();
        w.close().unwrap();
        assert_eq!(
            StdEnv.metadata_len(&path).unwrap(),
            pos,
            "close shrinks the mmap grow pad"
        );
        let recs = Wal::recover(&path).unwrap();
        assert_eq!(recs.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0233 P1.3: mmap 1c must leave the Exclusive cursor at the
    /// frontier so a later sequential `write()` (group drain) appends.
    #[test]
    fn rfc0233_mmap_lone_then_group_write_appends() {
        let _env_axis = RFC0209_ENV_AXIS.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = Rfc0209EnvGuard;
        std::env::set_var("PEDRA_WAL_PWRITE", "0");
        std::env::set_var("PEDRA_WAL_BUFFER", "0");
        let dir = rfc0209_dir("rfc0233-mmap-then-write");
        let path = dir.join("wal.log");
        let mut w = Wal::create(&path).unwrap();
        w.encode_write_op_batches(&[rfc0209_put(1).as_slice()])
            .unwrap();
        w.write_pending_frame_lone().unwrap();
        w.encode_write_op_batches(&[rfc0209_put(2).as_slice()])
            .unwrap();
        w.write_pending_frame().unwrap();
        w.close().unwrap();
        let recs = Wal::recover(&path).unwrap();
        assert_eq!(
            recs.len(),
            2,
            "group write_all after mmap must append, not overwrite from 0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0233 P1.4: Exclusive group drain is `write_wal` at the ticket
    /// (mmap), not a cursor `write_all` that restarts at 0 after 1c mmap.
    #[test]
    fn rfc0233_group_emit_uses_write_wal() {
        let writer = include_str!("writer_kernel.rs");
        let emit = writer
            .split("fn emit_bytes")
            .nth(1)
            .expect("emit_bytes")
            .split("pub fn new")
            .next()
            .expect("new");
        assert!(
            emit.contains("w.write_wal(buf, at)"),
            "Exclusive group drain must mmap at the ticket"
        );
        assert!(
            emit.contains("arc.write_wal_at_shared(buf, at)"),
            "Shared group drain must mmap at the ticket (not pwrite)"
        );
        assert!(
            !emit.contains("write_all(buf)"),
            "Exclusive emit must not cursor-write (mmap does not move the cursor)"
        );
        let host = include_str!("../env_kernel.rs");
        let wal = host
            .split("fn write_wal(&mut self, buf: &[u8], at: u64)")
            .nth(1)
            .expect("File::write_wal")
            .split("fn write_wal_at_shared")
            .next()
            .expect("write_wal_at_shared");
        assert!(
            wal.contains("SeekFrom::Start(at)"),
            "write_all fallback after mmap miss must seek to the ticket"
        );
        let lead = include_str!("../concurrent_kernel.rs");
        let after = lead
            .split("unlock_fair(g)")
            .nth(1)
            .expect("leader unlock_fair")
            .split("(None, None)")
            .next()
            .expect("leader return");
        assert!(
            after.contains("if !self.seal_async") && after.contains("yield_now()"),
            "seal_async leaders must not donate a yield per group-of-1"
        );
    }

    /// RFC-0230 P0.4: production group leader asks for a pwrite job.
    #[test]
    fn rfc0193_concurrent_calls_take_pwrite_job() {
        let body = include_str!("../concurrent_kernel.rs");
        assert!(
            body.contains("take_pwrite_job("),
            "lead's off-lock WAL path must try the 0193 pwrite job"
        );
        assert!(
            !body.contains("take_pwrite_job().ok().flatten()"),
            "WAL I/O Err from take_pwrite_job must not become None"
        );
        assert!(
            !body.contains("match wal.lock().take_pwrite_job()"),
            "MutexGuard from take_pwrite_job must drop before a nested wal.lock()"
        );
        assert!(
            body.contains("let job = wal.lock().take_pwrite_job()"),
            "bind take_pwrite_job so the WAL mutex is not held across job.run/finish"
        );
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
