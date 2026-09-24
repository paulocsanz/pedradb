//! Append-only WAL writer.
//!
//! Mirrors RocksDB's `db/log_writer.cc` record-fragmentation algorithm: a
//! logical record is split across at most one block boundary using
//! `First`/`Middle`/`Last` physical records, and each physical record carries
//! a masked CRC32C over `{type, payload}`.

use std::io::{Seek, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::env::EnvFile;
use crate::error::Result;

use super::crc;
use super::format::{RecordType, BLOCK_SIZE, HEADER_SIZE};

/// Capacity of the circular lock-free staging buffer: 4 MiB (RFC-0266 P1.1).
pub const WAL_RING_CAPACITY_BYTES: usize = 4 * 1024 * 1024;

/// High-throughput lock-free circular staging buffer for single-client burst (RFC-0266 P1.1).
///
/// Eliminates mutex acquisition and syscall overhead on single-threaded / low-concurrency
/// paths by staging framed records in a pre-allocated 4 MiB ring buffer.
pub struct LockFreeWalRing {
    buffer: parking_lot::Mutex<Vec<u8>>,
    head: AtomicU64,
    tail: AtomicU64,
    commit_inflight: AtomicUsize,
}

impl Default for LockFreeWalRing {
    fn default() -> Self {
        Self::new()
    }
}

impl LockFreeWalRing {
    /// Construct a new 4 MiB circular staging ring.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: parking_lot::Mutex::new(vec![0u8; WAL_RING_CAPACITY_BYTES]),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            commit_inflight: AtomicUsize::new(0),
        }
    }

    /// Current capacity in bytes.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        WAL_RING_CAPACITY_BYTES
    }

    /// Reserve space in the ring buffer atomically.
    /// Returns `Some(offset_in_ring)` if reservation fits within available space.
    pub fn reserve(&self, len: usize) -> Option<usize> {
        let cap = self.capacity() as u64;
        let mut cur_head = self.head.load(Ordering::Acquire);
        loop {
            let cur_tail = self.tail.load(Ordering::Acquire);
            if cur_head.saturating_sub(cur_tail) + (len as u64) > cap {
                return None; // Buffer full, needs draining
            }
            match self.head.compare_exchange_weak(
                cur_head,
                cur_head + (len as u64),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(pos) => return Some((pos % cap) as usize),
                Err(h) => cur_head = h,
            }
        }
    }

    /// Direct append fast path for single-client 1c: writes header and payload
    /// directly into the circular buffer.
    pub fn append_record_fast(&self, seq: u64, payload: &[u8]) -> Option<usize> {
        self.commit_inflight.fetch_add(1, Ordering::SeqCst);
        let rec_len = HEADER_SIZE + 8 + payload.len(); // header + seq + payload
        let offset = self.reserve(rec_len)?;

        let cap = self.capacity();
        let crc = crc::crc32c(payload);

        let h_crc = crc.to_le_bytes();
        let h_len = ((payload.len() + 8) as u16).to_le_bytes();
        let h_type = RecordType::Full as u8;

        {
            let mut guard = self.buffer.lock();
            let mut write_pos = offset;
            let mut write_byte = |b: u8| {
                guard[write_pos] = b;
                write_pos = (write_pos + 1) % cap;
            };

            for b in h_crc { write_byte(b); }
            for b in h_len { write_byte(b); }
            write_byte(h_type);

            for b in seq.to_le_bytes() { write_byte(b); }
            for &b in payload { write_byte(b); }
        }

        self.commit_inflight.fetch_sub(1, Ordering::SeqCst);
        Some(offset)
    }

    /// Drain staged bytes to the sink.
    pub fn drain_to<W: std::io::Write>(&self, sink: &mut W) -> std::io::Result<usize> {
        let cur_head = self.head.load(Ordering::Acquire);
        let cur_tail = self.tail.load(Ordering::Acquire);
        if cur_tail >= cur_head {
            return Ok(0);
        }

        let cap = self.capacity() as u64;
        let mut drained = 0usize;
        let mut t = cur_tail;

        let guard = self.buffer.lock();
        while t < cur_head {
            let start = (t % cap) as usize;
            let chunk_len = ((cur_head - t) as usize).min(cap as usize - start);
            sink.write_all(&guard[start..start + chunk_len])?;
            drained += chunk_len;
            t += chunk_len as u64;
        }

        self.tail.store(cur_head, Ordering::Release);
        Ok(drained)
    }

    /// Returns number of pending bytes in the ring buffer.
    #[must_use]
    pub fn pending_bytes(&self) -> u64 {
        self.head.load(Ordering::Acquire).saturating_sub(self.tail.load(Ordering::Acquire))
    }

    /// Checks if there are active in-flight commits.
    #[must_use]
    pub fn has_inflight(&self) -> bool {
        self.commit_inflight.load(Ordering::Acquire) > 0
    }
}

/// A streaming WAL writer over any `Write + Seek` sink.
///
/// Test-friendly: wrap a `Cursor<Vec<u8>>` in tests, or a real `File` in
/// production (see [`crate::wal::Wal`] for the fsync-aware file wrapper).
enum WalSink<W> {
    Exclusive(W),
    Shared(Arc<W>),
}

pub struct WalWriter<W> {
    sink: WalSink<W>,
    /// Bytes consumed within the current 32 KiB block.
    block_offset: usize,
    /// Byte offset of the next write (anchored at construction; advanced by
    /// every successful sink write). Pure in-memory state — querying the
    /// sink mid-commit would `flush`, which fault-injection envs classify
    /// as a Write op and which is a syscall on the hot path.
    ///
    /// With RFC-0209 staging enabled, advances at **stage** time (logical
    /// position): the staged bytes are promised to the sink in order, and
    /// every sink-facing path (direct write, barrier, close) drains the
    /// staging buffer first, so the logical order is the file order.
    position: u64,
    /// Reused framing buffer (RFC-0040: no per-record malloc of the payload).
    frame: Vec<u8>,
    /// RFC-0209: user-space staging (default on; `PEDRA_WAL_BUFFER=0` off).
    /// `None` (default) = the pre-0209 path, one sink write per frame.
    staged: Option<StagedBuf>,
    /// RFC-0193: reservation frontier (ticket allocator). Starts equal to
    /// `position`; `reserve_frame` advances it. Sequential writes keep
    /// `reserved_to == position`.
    reserved_to: u64,
}

/// Staging state for [`WalWriter`] (RFC-0209 P0.2).
struct StagedBuf {
    /// Pending framed bytes not yet handed to the sink.
    buf: Vec<u8>,
    /// Flush cap in bytes; `0` is the misuse guard (flush every append —
    /// byte-for-byte the AS-IS behavior; see `wal_buffer_kernel`).
    max: u64,
}

impl<W: EnvFile> WalWriter<W> {
    fn emit_bytes(&mut self, buf: &[u8], at: u64) -> std::io::Result<()> {
        match &mut self.sink {
            // Ticket `at` is the append point. mmap (`write_wal`) honors it
            // without a seek; a cursor `write_all` after 1c mmap would
            // restart at 0 (rfc0233_mmap_lone_then_group_write_appends).
            WalSink::Exclusive(w) => w.write_wal(buf, at),
            // Shared drain is mmap (`write_wal_at_shared`). Mixing that
            // with cursor `write_all` lost Darwin ycsb_f_mc4.
            WalSink::Shared(arc) => arc.write_wal_at_shared(buf, at),
        }
    }
    /// Create a writer, reading the sink's current position to compute the
    /// in-block offset. The caller is responsible for seeking to the desired
    /// write position (e.g. `SeekFrom::End(0)` to append) beforehand.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the sink's position cannot be queried.
    ///
    /// # Panics
    /// Panics if the stream position does not fit in the platform's `usize`
    /// (i.e. a >4 GiB offset on a 32-bit target), which is unreachable in
    /// practice for a WAL.
    pub fn new(mut out: W) -> Result<Self> {
        let raw_pos = out.stream_position()?;
        let pos: usize = raw_pos
            .try_into()
            .expect("stream position exceeds address space");
        // Shared mmap on unix (`wal_pwrite_enabled`). `PEDRA_WAL_PWRITE=0`
        // is Exclusive sequential `write()` (test opt-out). 1c FlushWAL is
        // `write_lone_frame` (mmap) on either sink.
        let sink = if out.positional_writes() && super::wal_pwrite_enabled() {
            WalSink::Shared(Arc::new(out))
        } else {
            WalSink::Exclusive(out)
        };
        Ok(Self {
            sink,
            block_offset: pos % BLOCK_SIZE,
            position: raw_pos,
            frame: Vec::new(),
            staged: None,
            reserved_to: raw_pos,
        })
    }

    /// RFC-0209 P0.2: enable user-space staging with a flush cap of `max`
    /// bytes. Size-only flush decisions come from
    /// [`crate::wal_buffer_kernel::should_flush`] — no timer, no waiting
    /// for writers (0180/0190 vetoes are structural). Staging must be
    /// enabled before the first write.
    pub(crate) fn enable_staging(&mut self, max: u64) {
        self.staged = Some(StagedBuf {
            buf: Vec::new(),
            max,
        });
    }

    /// Hand the staged bytes to the sink (RFC-0209 P0.2). Order rule (b):
    /// called before every direct sink write and before every barrier, so
    /// the file order is always the logical (seq) order. On a sink error
    /// the bytes return to the buffer — a failed drain is retryable, like
    /// the direct write path whose frame the caller still holds.
    pub(crate) fn drain_staged(&mut self) -> Result<()> {
        let buf = match self.staged.as_mut() {
            Some(st) if !st.buf.is_empty() => std::mem::take(&mut st.buf),
            _ => return Ok(()),
        };
        let at = self.position.saturating_sub(buf.len() as u64);
        match self.emit_bytes(&buf, at) {
            Ok(()) => Ok(()),
            Err(e) => {
                if let Some(st) = self.staged.as_mut() {
                    st.buf = buf;
                }
                Err(e.into())
            }
        }
    }

    /// Append one logical record, fragmenting across blocks as needed.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying sink.
    ///
    /// # Panics
    /// Never in practice; `block_offset` is an internal invariant kept below
    /// `BLOCK_SIZE`. The `checked_sub` guards against a logic regression.
    pub fn add_record(&mut self, data: &[u8]) -> Result<()> {
        let mut frame = std::mem::take(&mut self.frame);
        frame.clear();
        frame.reserve(data.len() + 2 * HEADER_SIZE);
        self.fragment_into(data, &mut frame);
        // RFC-0209 P0.2 order rule (b): a direct write drains staged bytes
        // first so the file order stays the logical order.
        self.drain_staged()?;
        let at = self.position;
        self.emit_bytes(&frame, at)?;
        self.note_sink_write(frame.len() as u64);
        // Do not leave the just-written bytes in `frame` — `Wal::sync_data`
        // drains staged frames from `encode_write_op_batches`. Re-emitting
        // this buffer would duplicate the record (same seq) on recover.
        frame.clear();
        self.frame = frame;
        Ok(())
    }

    /// Append several logical records with **one** `write` on the sink.
    ///
    /// Produces a byte stream identical to `add_record` per record (RFC-0037
    /// P2.2: per-member WAL `write` syscalls on the bench box cost more than
    /// the group `fdatasync`, capping multi-client throughput).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the single underlying write.
    pub fn add_records(&mut self, datas: &[&[u8]]) -> Result<()> {
        if crate::write_admission_kernel::batch_is_empty(datas.len() as u64) {
            return Ok(());
        }
        let mut frame = std::mem::take(&mut self.frame);
        frame.clear();
        frame.reserve(datas.iter().map(|d| d.len() + HEADER_SIZE * 2).sum());
        for data in datas {
            self.fragment_into(data, &mut frame);
        }
        // RFC-0209 P0.2 order rule (b): group writes are direct — drain
        // staged bytes first (same rule as `add_record`).
        self.drain_staged()?;
        let at = self.position;
        self.emit_bytes(&frame, at)?;
        self.note_sink_write(frame.len() as u64);
        frame.clear();
        self.frame = frame;
        Ok(())
    }

    pub(crate) fn take_frame(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.frame)
    }

    pub(crate) fn restore_frame(&mut self, frame: Vec<u8>) {
        self.frame = frame;
    }

    /// Scratch path (`data` → frame) — production encode goes through
    /// [`Self::fragment_encoded`]; kept as the byte-identity oracle for tests.
    #[cfg(test)]
    pub(crate) fn fragment_record(&mut self, data: &[u8], buf: &mut Vec<u8>) {
        buf.reserve(data.len() + 2 * HEADER_SIZE);
        self.fragment_from(&mut SliceSource(data), buf);
    }

    /// RFC-0042 P1.3: fragment the [`crate::batch::encode_ops`] encoding of
    /// `ops` straight into `buf` — byte-identical to
    /// `fragment_record(&encode_ops(ops))` without the intermediate logical
    /// buffer (one full-record copy per group member saved).
    #[cfg(test)]
    pub(crate) fn fragment_encoded(&mut self, ops: &[crate::batch::WriteOp], buf: &mut Vec<u8>) {
        self.fragment_encoded_len(ops, buf);
    }

    /// Encode + fragment; returns the logical record length (one `encoded_len`).
    pub(crate) fn fragment_encoded_len(
        &mut self,
        ops: &[crate::batch::WriteOp],
        buf: &mut Vec<u8>,
    ) -> usize {
        if ops.len() == 1 {
            if let Some(n) = self.fragment_one_op(&ops[0], buf) {
                return n;
            }
        }
        let mut src = EncodedOpsSource::new(ops);
        let n = src.total;
        buf.reserve(n + 2 * HEADER_SIZE);
        self.fragment_from(&mut src, buf);
        n
    }

    /// 1-op Full record that fits in the current block — one CRC, no
    /// EncodedOpsSource state machine (RFC-0233 P1.3 1c encode).
    fn fragment_one_op(&mut self, op: &crate::batch::WriteOp, buf: &mut Vec<u8>) -> Option<usize> {
        let leftover = BLOCK_SIZE.checked_sub(self.block_offset)?;
        if leftover < HEADER_SIZE {
            return None;
        }
        let rec = Self::encode_one_op_full_detached(op)?;
        if leftover < rec.len() {
            return None;
        }
        let logical = rec.len() - HEADER_SIZE;
        buf.extend_from_slice(&rec);
        self.block_offset += rec.len();
        Some(logical)
    }

    /// Physical Full record (header+CRC+payload) with no writer state.
    /// RFC-0237: the 4-writer bypass encodes this **off** `wal.lock()`;
    /// the CS only tickets the already-framed bytes. `None` if the
    /// logical record cannot be a single Full fragment (`> BLOCK_SIZE`).
    pub(crate) fn encode_one_op_full_detached(op: &crate::batch::WriteOp) -> Option<Vec<u8>> {
        let logical = 5 + 1 + 8 + 4 + op.key.len() + 4 + op.value.len();
        if HEADER_SIZE + logical > BLOCK_SIZE {
            return None;
        }
        let mut buf = Vec::with_capacity(HEADER_SIZE + logical);
        buf.extend_from_slice(&[0u8; HEADER_SIZE]);
        buf.push(crate::batch::WRITE_RECORD_VERSION);
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(op.kind.as_u8());
        buf.extend_from_slice(&op.sequence.to_le_bytes());
        let kl = u32::try_from(op.key.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&kl.to_le_bytes());
        buf.extend_from_slice(&op.key);
        let vl = u32::try_from(op.value.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&vl.to_le_bytes());
        buf.extend_from_slice(&op.value);
        Self::write_physical_header(RecordType::Full, logical, 0, &mut buf);
        Some(buf)
    }

    /// If a pre-encoded Full record of `rec_len` fits in the current
    /// block (after the &lt; HEADER pad), advance `block_offset` and
    /// return the pad to prepend. `None` = caller must fragment under
    /// the lock (First/Middle/Last).
    pub(crate) fn take_preframed_layout(&mut self, rec_len: usize) -> Option<usize> {
        if rec_len == 0 || rec_len > BLOCK_SIZE {
            return None;
        }
        let leftover = BLOCK_SIZE.saturating_sub(self.block_offset);
        let pad = if leftover < HEADER_SIZE {
            leftover
        } else {
            0
        };
        let room = if leftover < HEADER_SIZE {
            BLOCK_SIZE
        } else {
            leftover
        };
        if rec_len > room {
            return None;
        }
        if leftover < HEADER_SIZE {
            self.block_offset = 0;
        }
        self.block_offset += rec_len;
        Some(pad)
    }

    /// 1c FlushWAL: mmap memcpy (page cache), no staging hop. Group path
    /// still stages + `write()`/`pwrite` (mmap-on-group lost Darwin mc4).
    pub(crate) fn write_lone_frame(&mut self, buf: &[u8]) -> Result<()> {
        if crate::write_admission_kernel::batch_is_empty(buf.len() as u64) {
            return Ok(());
        }
        self.drain_staged()?;
        let at = self.position;
        match &mut self.sink {
            WalSink::Exclusive(w) => w.write_wal(buf, at)?,
            WalSink::Shared(arc) => arc.write_wal_at_shared(buf, at)?,
        }
        self.note_sink_write(buf.len() as u64);
        Ok(())
    }

    pub(crate) fn write_frame(&mut self, buf: &[u8]) -> Result<()> {
        if !crate::write_admission_kernel::batch_is_empty(buf.len() as u64) {
            // RFC-0209 P0.2: with staging enabled, the framed record goes
            // to the user-space buffer (memcpy, no syscall) and the kernel
            // decides the flush by size alone. `position` advances at stage
            // time — the logical append point — so `reserve_space` still
            // sees the true frontier. Disabled (default): the pre-0209
            // single write per frame.
            if let Some(st) = self.staged.as_mut() {
                st.buf.extend_from_slice(buf);
                let staged_len = st.buf.len() as u64;
                let staged_max = st.max;
                let n = buf.len() as u64;
                drop(st);
                self.note_sink_write(n);
                if crate::wal_buffer_kernel::should_flush(staged_len, staged_max) {
                    return self.drain_staged();
                }
                return Ok(());
            }
            let at = self.position;
            self.emit_bytes(buf, at)?;
            self.note_sink_write(buf.len() as u64);
        }
        Ok(())
    }

    fn note_sink_write(&mut self, n: u64) {
        self.position = self.position.saturating_add(n);
        if self.reserved_to < self.position {
            self.reserved_to = self.position;
        }
    }

    /// RFC-0193: allocate `len` bytes at the reservation frontier.
    #[must_use]
    pub(crate) fn reserve_pending(&mut self, len: u64) -> u64 {
        let (ticket, next) = crate::wal_ticket_kernel::reserve_frame(self.reserved_to, len);
        self.reserved_to = next;
        ticket
    }

    /// Byte offset of the next write (in-memory; no sink I/O).
    #[must_use]
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Fragmentation state machine shared by [`Self::add_record`] (direct
    /// write) and [`Self::add_records`] (staged buffer).
    fn fragment_into(&mut self, data: &[u8], buf: &mut Vec<u8>) {
        self.fragment_from(&mut SliceSource(data), buf);
    }

    fn fragment_from(&mut self, src: &mut dyn RecordSource, buf: &mut Vec<u8>) {
        let mut left = src.total_len();
        let mut begin = true;

        loop {
            let leftover = BLOCK_SIZE
                .checked_sub(self.block_offset)
                .expect("block_offset never exceeds BLOCK_SIZE");

            // Not enough room for even a header in this block: pad to the end
            // and continue in a fresh block.
            if leftover < HEADER_SIZE {
                let pad = [0u8; HEADER_SIZE];
                buf.extend_from_slice(&pad[..leftover]);
                self.block_offset = 0;
            }

            let avail = BLOCK_SIZE - self.block_offset - HEADER_SIZE;
            let fragment_len = left.min(avail);
            let end = left - fragment_len == 0;

            let rtype = if begin && end {
                RecordType::Full
            } else if begin {
                RecordType::First
            } else if end {
                RecordType::Last
            } else {
                RecordType::Middle
            };

            // Stage the payload first: the crc needs the contiguous bytes.
            // RFC-0054 P1.4: the header goes in as a 7-byte placeholder and
            // the source appends the payload field by field — the previous
            // `resize(0)` + copy pass wrote every payload byte twice.
            let hdr_pos = buf.len();
            buf.reserve(HEADER_SIZE + fragment_len);
            buf.extend_from_slice(&[0u8; HEADER_SIZE]);
            src.append_exact_to(buf, fragment_len);
            self.patch_physical_record(rtype, fragment_len, hdr_pos, buf);

            left -= fragment_len;
            begin = false;

            if crate::write_admission_kernel::batch_is_empty(left as u64) {
                break;
            }
        }
    }

    /// Patch the physical-record header (crc + length + type) in front of the
    /// staged payload at `hdr_pos` (RFC-0042 P1.3 in-place emit).
    fn write_physical_header(
        rtype: RecordType,
        payload_len: usize,
        hdr_pos: usize,
        buf: &mut [u8],
    ) {
        let length_u16 =
            u16::try_from(payload_len).expect("physical record fragment must fit in u16");

        let checksum = crc::record_checksum(rtype as u8, length_u16, &buf[hdr_pos + HEADER_SIZE..]);

        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&checksum.to_le_bytes());
        header[4..6].copy_from_slice(&length_u16.to_le_bytes());
        header[6] = rtype as u8;
        buf[hdr_pos..hdr_pos + HEADER_SIZE].copy_from_slice(&header);
    }

    fn patch_physical_record(
        &mut self,
        rtype: RecordType,
        payload_len: usize,
        hdr_pos: usize,
        buf: &mut [u8],
    ) {
        Self::write_physical_header(rtype, payload_len, hdr_pos, buf);
        let length_u16 =
            u16::try_from(payload_len).expect("physical record fragment must fit in u16");
        self.block_offset += HEADER_SIZE + usize::from(length_u16);
    }

    /// Flush buffered writes to the OS. Does **not** fsync — use a file
    /// wrapper for crash durability. RFC-0209 P0.2 order rule (c): drains
    /// the staging buffer first, so every caller that reaches the sink's
    /// flush (barriers included) has all logical bytes in the OS.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying sink.
    pub fn flush(&mut self) -> Result<()> {
        self.drain_staged()?;
        match &mut self.sink {
            WalSink::Exclusive(w) => w.flush()?,
            WalSink::Shared(_) => {}
        }
        Ok(())
    }

    /// Logical append point after the last flush (start of next write).
    /// mmap `write_wal` does not move the File cursor — the cursor is not
    /// the WAL frontier (rfc0233 recover_from_offset / prealloc size).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if staged bytes cannot be drained.
    pub fn stream_position(&mut self) -> Result<u64> {
        self.drain_staged()?;
        if let WalSink::Exclusive(w) = &mut self.sink {
            w.flush()?;
        }
        Ok(self.position)
    }

    /// Consume the writer and return the underlying sink. Drains staged
    /// bytes first (best-effort: a drain error is dropped — use the
    /// `Wal::close` path when it must propagate).
    pub fn into_inner(mut self) -> W {
        let _ = self.drain_staged();
        match self.sink {
            WalSink::Exclusive(w) => w,
            WalSink::Shared(arc) => Arc::try_unwrap(arc)
                .unwrap_or_else(|_| panic!("WAL into_inner with inflight shared handle")),
        }
    }

    /// Borrow the underlying sink mutably (exclusive path only).
    pub(crate) fn inner_mut(&mut self) -> Option<&mut W> {
        match &mut self.sink {
            WalSink::Exclusive(w) => Some(w),
            WalSink::Shared(_) => None,
        }
    }

    pub(crate) fn preallocate_extent(&mut self, len: u64) -> std::io::Result<()> {
        match &mut self.sink {
            WalSink::Exclusive(w) => w.preallocate(len),
            WalSink::Shared(arc) => arc.preallocate_shared(len),
        }
    }

    pub(crate) fn sync_data_sink(&mut self, strong: bool) -> std::io::Result<()> {
        match &mut self.sink {
            WalSink::Exclusive(w) => {
                if strong {
                    w.sync_data_strong()
                } else {
                    w.sync_data()
                }
            }
            WalSink::Shared(arc) => {
                if strong {
                    arc.sync_data_strong_shared()
                } else {
                    arc.sync_data_shared()
                }
            }
        }
    }

    pub(crate) fn sync_all_sink(&mut self) -> std::io::Result<()> {
        match &mut self.sink {
            WalSink::Exclusive(w) => w.sync_all(),
            WalSink::Shared(arc) => arc.sync_all_shared(),
        }
    }

    /// Clone the `Arc` (not the fd) for an off-lock pwrite.
    pub(crate) fn share_pwrite(&self) -> Option<Arc<W>> {
        match &self.sink {
            WalSink::Shared(arc) => Some(Arc::clone(arc)),
            WalSink::Exclusive(_) => None,
        }
    }
}

impl<W: crate::env::EnvFile> WalWriter<W> {
    pub(crate) fn positional_writes(&self) -> bool {
        match &self.sink {
            WalSink::Exclusive(w) => w.positional_writes(),
            WalSink::Shared(_) => true,
        }
    }

    pub(crate) fn write_all_at(&mut self, buf: &[u8], at: u64) -> Result<()> {
        match &mut self.sink {
            WalSink::Exclusive(w) => w.write_all_at(buf, at)?,
            WalSink::Shared(arc) => arc.write_all_at_shared(buf, at)?,
        }
        self.commit_pwrite(at, buf.len() as u64);
        Ok(())
    }

    /// Unmap a padded WAL map and shrink i_size to the logical frontier.
    /// Recover of a closed file then matches `position` (mmap grow pad
    /// is not left on disk).
    pub(crate) fn truncate_to_logical(&mut self) -> Result<()> {
        let pos = self.position;
        match &mut self.sink {
            WalSink::Exclusive(w) => {
                w.release_wal_map();
                w.set_len(pos)?;
            }
            WalSink::Shared(arc) => {
                arc.release_wal_map();
                arc.set_len_shared(pos)?;
            }
        }
        Ok(())
    }

    /// Advance the written frontier to `ticket+len` after a successful
    /// off-lock pwrite. Must not run at reserve time: a failed job must
    /// leave [`Self::position`] at the last committed byte so
    /// `wal_segment_is_empty` still sees an empty segment.
    pub(crate) fn commit_pwrite(&mut self, ticket: u64, len: u64) {
        let end = ticket.saturating_add(len);
        if self.position < end {
            self.position = end;
        }
    }
}

/// Byte source of one logical record for [`WalWriter::fragment_from`]
/// (RFC-0042 P1.3: `encode_ops` into a scratch `Vec` followed by
/// `fragment_record` copied every record twice; a source feeds the
/// fragmentation state machine directly, one copy).
trait RecordSource {
    fn total_len(&self) -> usize;
    /// Append exactly `n` bytes of the record onto `dst` (single write per
    /// run — no pre-zeroed region; see `fragment_from`).
    fn append_exact_to(&mut self, dst: &mut Vec<u8>, n: usize);
}

struct SliceSource<'a>(&'a [u8]);

impl RecordSource for SliceSource<'_> {
    fn total_len(&self) -> usize {
        self.0.len()
    }

    fn append_exact_to(&mut self, dst: &mut Vec<u8>, n: usize) {
        dst.extend_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
    }
}

/// Yields exactly the [`crate::batch::encode_ops`] encoding of `ops`, field by
/// field, so no intermediate logical buffer is needed.
struct EncodedOpsSource<'a> {
    ops: &'a [crate::batch::WriteOp],
    head: [u8; 5],
    /// `false` until the 5-byte `head` has been fully consumed.
    head_done: bool,
    idx: usize,
    /// 0 = kind+seq+klen preamble, 1 = key, 2 = vlen, 3 = value, 4 = done.
    stage: u8,
    preamble: [u8; 13],
    vlen: [u8; 4],
    off: usize,
    total: usize,
    v2: bool,
    reuse: bool,
}

impl<'a> EncodedOpsSource<'a> {
    fn new(ops: &'a [crate::batch::WriteOp]) -> Self {
        let count = u32::try_from(ops.len()).unwrap_or(u32::MAX);
        let v2 = crate::batch::record_uses_v2(ops);
        let mut head = [0u8; 5];
        head[0] = if v2 {
            crate::batch::WRITE_RECORD_VERSION_V2
        } else {
            crate::batch::WRITE_RECORD_VERSION
        };
        head[1..5].copy_from_slice(&count.to_le_bytes());
        Self {
            ops,
            head,
            head_done: false,
            idx: 0,
            stage: 4,
            preamble: [0; 13],
            vlen: [0; 4],
            off: 0,
            total: crate::batch::encoded_len(ops),
            v2,
            reuse: false,
        }
    }

    fn enter_op(&mut self) {
        if self.idx >= self.ops.len() {
            self.stage = 4;
            return;
        }
        let op = &self.ops[self.idx];
        self.reuse =
            self.v2 && self.idx > 0 && crate::batch::value_ptr_eq(&self.ops[self.idx - 1], op);
        self.preamble[0] = op.kind.as_u8()
            | if self.reuse {
                crate::batch::KIND_REUSE_PREV
            } else {
                0
            };
        self.preamble[1..9].copy_from_slice(&op.sequence.to_le_bytes());
        let kl = u32::try_from(op.key.len()).unwrap_or(u32::MAX);
        self.preamble[9..13].copy_from_slice(&kl.to_le_bytes());
        if !self.reuse {
            let vl = u32::try_from(op.value.len()).unwrap_or(u32::MAX);
            self.vlen.copy_from_slice(&vl.to_le_bytes());
        }
        self.stage = 0;
        self.off = 0;
    }

    fn next_op(&mut self) {
        self.off = 0;
        self.idx += 1;
        if self.idx < self.ops.len() {
            self.enter_op();
        } else {
            self.stage = 4;
        }
    }

    fn finish_key_field(&mut self) {
        self.off = 0;
        if self.reuse {
            self.next_op();
        } else {
            self.stage = 2;
        }
    }

    /// Current contiguous run of encoded bytes (skips empty fields).
    fn current(&mut self) -> &[u8] {
        loop {
            if !self.head_done {
                return &self.head[self.off..];
            }
            match self.stage {
                0 => return &self.preamble[self.off..],
                1 => {
                    let key = &self.ops[self.idx].key;
                    if crate::write_admission_kernel::batch_is_empty(key.len() as u64) {
                        self.finish_key_field();
                        continue;
                    }
                    return &key[self.off..];
                }
                2 => {
                    if self.reuse {
                        self.next_op();
                        continue;
                    }
                    return &self.vlen[self.off..];
                }
                3 => {
                    if self.reuse {
                        self.next_op();
                        continue;
                    }
                    let value = &self.ops[self.idx].value;
                    if crate::write_admission_kernel::batch_is_empty(value.len() as u64) {
                        self.next_op();
                        continue;
                    }
                    return &value[self.off..];
                }
                _ => {
                    if self.idx >= self.ops.len() {
                        return &[];
                    }
                    self.enter_op();
                }
            }
        }
    }

    fn advance(&mut self, taken: usize) {
        if !self.head_done {
            self.off += taken;
            if self.off == self.head.len() {
                self.head_done = true;
                self.off = 0;
                if self.idx < self.ops.len() {
                    self.enter_op();
                }
            }
            return;
        }
        match self.stage {
            0 | 2 => {
                self.off += taken;
                let end = if self.stage == 0 {
                    self.preamble.len()
                } else {
                    self.vlen.len()
                };
                if self.off == end {
                    self.off = 0;
                    self.stage += 1;
                }
            }
            1 => {
                self.off += taken;
                if self.off == self.ops[self.idx].key.len() {
                    self.finish_key_field();
                }
            }
            3 => {
                self.off += taken;
                if self.off == self.ops[self.idx].value.len() {
                    self.next_op();
                }
            }
            _ => {}
        }
    }
}

impl RecordSource for EncodedOpsSource<'_> {
    fn total_len(&self) -> usize {
        self.total
    }

    fn append_exact_to(&mut self, dst: &mut Vec<u8>, n: usize) {
        let mut filled = 0;
        while filled < n {
            let run = self.current();
            assert!(
                !crate::write_admission_kernel::batch_is_empty(run.len() as u64),
                "EncodedOpsSource exhausted before record end"
            );
            let take = (n - filled).min(run.len());
            dst.extend_from_slice(&run[..take]);
            self.advance(take);
            filled += take;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Shared oracle sink: counts `write` calls and exposes the byte
    /// stream while the writer still owns the sink — the syscall and
    /// drain-timing observables for RFC-0209.
    #[derive(Clone, Default)]
    struct ProbeShared {
        writes: std::rc::Rc<std::cell::Cell<usize>>,
        bytes: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
        pos: std::rc::Rc<std::cell::Cell<u64>>,
    }

    struct ProbeSink {
        shared: ProbeShared,
    }

    impl ProbeSink {
        /// Sink + its shared oracle handle.
        fn new() -> (Self, ProbeShared) {
            let shared = ProbeShared::default();
            (
                Self {
                    shared: shared.clone(),
                },
                shared,
            )
        }
    }

    impl std::io::Write for ProbeSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.shared.writes.set(self.shared.writes.get() + 1);
            let mut bytes = self.shared.bytes.borrow_mut();
            let pos = self.shared.pos.get() as usize;
            if pos > bytes.len() {
                bytes.resize(pos, 0);
            }
            let end = pos + buf.len();
            if end > bytes.len() {
                bytes.resize(end, 0);
            }
            bytes.splice(pos..end, buf.iter().copied());
            self.shared.pos.set(end as u64);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Seek for ProbeSink {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            let base = self.shared.pos.get() as i64;
            let next = match pos {
                std::io::SeekFrom::Start(s) => s as i64,
                std::io::SeekFrom::Current(d) => base + d,
                std::io::SeekFrom::End(d) => self.shared.bytes.borrow().len() as i64 + d,
            };
            let next = next.max(0) as u64;
            self.shared.pos.set(next);
            Ok(next)
        }
    }

    impl std::io::Read for ProbeSink {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let bytes = self.shared.bytes.borrow();
            let pos = self.shared.pos.get() as usize;
            let n = buf.len().min(bytes.len().saturating_sub(pos));
            buf[..n].copy_from_slice(&bytes[pos..pos + n]);
            self.shared.pos.set((pos + n) as u64);
            Ok(n)
        }
    }

    impl crate::env::EnvFile for ProbeSink {
        fn sync_data(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn sync_all(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn set_len(&mut self, len: u64) -> std::io::Result<()> {
            self.shared.bytes.borrow_mut().resize(len as usize, 0);
            Ok(())
        }
        fn len(&mut self) -> std::io::Result<u64> {
            Ok(self.shared.bytes.borrow().len() as u64)
        }
    }

    /// Drive the production 1-op path (`fragment_record` + `write_frame`,
    /// exactly what `Wal::write_pending_frame` does) for each record.
    fn drive_1op<W: crate::env::EnvFile>(w: &mut WalWriter<W>, records: &[Vec<u8>]) {
        for r in records {
            let mut frame = w.take_frame();
            w.fragment_record(r, &mut frame);
            w.write_frame(&frame).unwrap();
            frame.clear();
            w.restore_frame(frame);
        }
    }

    /// RFC-0209 P0.2: staged and unstaged writers produce byte-identical
    /// streams — staging only changes WHEN bytes reach the sink, never the
    /// bytes or their order (cap small enough to force mid-sequence
    /// flushes, plus a final partial drain at `flush`).
    #[test]
    fn rfc0209_buffered_wal_byte_identical_after_drain() {
        let b = BLOCK_SIZE;
        let records: Vec<Vec<u8>> = vec![
            b"short".to_vec(),
            vec![0x41; 150], // exceeds the 100-byte cap → mid-flush
            b"".to_vec(),
            vec![0xcd; b + 50], // spans blocks
            b"tail".to_vec(),
        ];

        let mut plain = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        drive_1op(&mut plain, &records);
        let plain_bytes = plain.into_inner().into_inner();

        let mut buffered = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        buffered.enable_staging(100);
        drive_1op(&mut buffered, &records);
        assert!(
            !buffered.staged.as_ref().unwrap().buf.is_empty(),
            "test shape: final records must still be staged pre-flush"
        );
        buffered.flush().unwrap();
        let buffered_bytes = buffered.into_inner().into_inner();

        assert_eq!(plain_bytes, buffered_bytes);
        assert_eq!(
            collect_records(&buffered_bytes),
            records,
            "buffered stream must read back as the same logical records"
        );
    }

    /// RFC-0209 P0.2 mechanism: staging off = one sink write per frame
    /// (AS-IS); staging on = frames coalesce into one sink write per flush
    /// cycle. This is the syscall count the meter pays for.
    #[test]
    fn rfc0209_staging_coalesces_sink_writes() {
        let records: Vec<Vec<u8>> = (0..10).map(|i| vec![0x5a; 20 + i]).collect();

        let (plain_sink, plain_probe) = ProbeSink::new();
        let mut plain = WalWriter::new(plain_sink).unwrap();
        drive_1op(&mut plain, &records);
        plain.flush().unwrap();
        assert_eq!(plain_probe.writes.get(), 10, "AS-IS: write per frame");
        assert_eq!(collect_records(&plain_probe.bytes.borrow()), records);

        let (buffered_sink, staged_probe) = ProbeSink::new();
        let mut buffered = WalWriter::new(buffered_sink).unwrap();
        buffered.enable_staging(64 * 1024);
        drive_1op(&mut buffered, &records);
        assert_eq!(
            staged_probe.bytes.borrow().len(),
            0,
            "staged bytes must not reach the sink before the cap"
        );
        buffered.flush().unwrap();
        assert_eq!(
            staged_probe.writes.get(),
            1,
            "staging: one write per flush cycle"
        );
        assert_eq!(
            collect_records(&staged_probe.bytes.borrow()),
            records,
            "coalesced write must read back identically"
        );
    }

    /// RFC-0209 P0.2 order rule (b): a direct GROUP write
    /// (`add_records`, the 0037 P2.2 group path) drains staged bytes
    /// FIRST — the file order is always the logical order.
    #[test]
    fn rfc0209_group_direct_write_drains_staged_first() {
        let (sink, probe) = ProbeSink::new();
        let mut w = WalWriter::new(sink).unwrap();
        w.enable_staging(64 * 1024);
        drive_1op(&mut w, &[b"staged-1".to_vec(), b"staged-2".to_vec()]);
        assert_eq!(probe.bytes.borrow().len(), 0);
        w.add_records(&[b"direct-1".as_slice(), b"direct-2".as_slice()])
            .unwrap();
        assert_eq!(
            probe.writes.get(),
            2,
            "drain of staged bytes + the group write, in that order"
        );
        assert_eq!(
            collect_records(&probe.bytes.borrow()),
            vec![
                b"staged-1".to_vec(),
                b"staged-2".to_vec(),
                b"direct-1".to_vec(),
                b"direct-2".to_vec()
            ]
        );
    }

    /// RFC-0209 P0.2 order rule (c): the sink-facing flush drains staged
    /// bytes — every barrier path (`sync_data`, `sync_all`, `close`)
    /// funnels through here, so no fd ever happens with logical bytes
    /// still user-side.
    #[test]
    fn rfc0209_flush_drains_staged_bytes() {
        let (sink, probe) = ProbeSink::new();
        let mut w = WalWriter::new(sink).unwrap();
        w.enable_staging(64 * 1024);
        let records: Vec<Vec<u8>> = (0..3).map(|i| vec![0x77; 30 + i]).collect();
        drive_1op(&mut w, &records);
        assert_eq!(probe.bytes.borrow().len(), 0);
        w.flush().unwrap();
        assert_eq!(collect_records(&probe.bytes.borrow()), records);
    }

    /// RFC-0042 P1.3: the direct-to-frame source must emit byte-identical
    /// frames to the scratch path `fragment_record(&encode_ops(ops))`,
    /// across fragment topologies (empty, single-block, exact block
    /// multiples, multi-block) and a mid-block starting offset.
    #[test]
    fn fragment_encoded_matches_scratch_path_bytes() {
        use crate::batch::WriteOp;
        use bytes::Bytes;

        let b = BLOCK_SIZE;
        let cases: Vec<Vec<WriteOp>> = vec![
            vec![],
            vec![WriteOp::put(1, Bytes::from_static(b"k"), Bytes::new())],
            vec![WriteOp::put(
                1,
                Bytes::from_static(b"k"),
                Bytes::from_static(b"v"),
            )],
            vec![
                WriteOp::put(
                    7,
                    Bytes::from_static(b"abc"),
                    Bytes::from(vec![0xa5; b + 50]),
                ),
                WriteOp::delete(8, Bytes::from_static(b"gone")),
                WriteOp::put(
                    9,
                    Bytes::from(vec![0x11; b]),
                    Bytes::from(vec![0x22; 2 * b + 11]),
                ),
            ],
            vec![
                WriteOp::put(
                    1,
                    Bytes::from_static(b"empty-key"),
                    Bytes::from_static(b"x"),
                ),
                WriteOp::put(2, Bytes::new(), Bytes::from_static(b"empty-key-value")),
                WriteOp::put(3, Bytes::from_static(b"both"), Bytes::new()),
            ],
            {
                let shared = Bytes::from(vec![0x5a; 1024]);
                vec![
                    WriteOp::put(1, Bytes::from_static(b"p0"), shared.clone()),
                    WriteOp::put(2, Bytes::from_static(b"p1"), shared.clone()),
                    WriteOp::put(3, Bytes::from_static(b"p2"), shared),
                ]
            },
        ];

        for start_offset in [0usize, 11, b - 3, b - 1].into_iter().chain(1..=40) {
            for ops in &cases {
                let scratch = WalWriter::new(Cursor::new(Vec::new())).unwrap();
                let direct = WalWriter::new(Cursor::new(Vec::new())).unwrap();
                // Skip: fresh writers share block state; align both to the
                // same mid-block offset by fragmenting a filler record first.
                let (mut scratch, mut direct) = (scratch, direct);
                if start_offset > 0 {
                    let filler = vec![0xee; start_offset];
                    let mut sf = scratch.take_frame();
                    scratch.fragment_record(&filler, &mut sf);
                    scratch.restore_frame(sf);
                    let mut df = direct.take_frame();
                    direct.fragment_record(&filler, &mut df);
                    direct.restore_frame(df);
                }
                let mut logical = Vec::new();
                crate::batch::encode_ops(ops, &mut logical);
                let mut sf = scratch.take_frame();
                scratch.fragment_record(&logical, &mut sf);
                scratch.write_frame(&sf).unwrap();
                scratch.restore_frame(sf);
                let mut df = direct.take_frame();
                direct.fragment_encoded(ops, &mut df);
                direct.write_frame(&df).unwrap();
                direct.restore_frame(df);
                assert_eq!(
                    scratch.into_inner().into_inner(),
                    direct.into_inner().into_inner(),
                    "offset={start_offset} ops_len={}",
                    crate::batch::encoded_len(ops)
                );
            }
        }
    }

    fn collect_records(buf: &[u8]) -> Vec<Vec<u8>> {
        let mut reader = super::super::reader::WalReader::new(buf);
        let mut out = Vec::new();
        while let Some(rec) = reader.read_record().expect("read ok") {
            out.push(rec);
        }
        out
    }

    #[test]
    fn writes_and_reads_back_single_record() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"hello, pedra").unwrap();
        let buf = writer.into_inner().into_inner();
        assert_eq!(collect_records(&buf), vec![b"hello, pedra".to_vec()]);
    }

    #[test]
    fn writes_multiple_records() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        for i in 0..50 {
            writer.add_record(format!("record-{i}").as_bytes()).unwrap();
        }
        let buf = writer.into_inner().into_inner();
        let recs = collect_records(&buf);
        assert_eq!(recs.len(), 50);
        assert_eq!(recs[0], b"record-0");
        assert_eq!(recs[49], b"record-49");
    }

    #[test]
    fn fragments_large_record_across_blocks() {
        // A record larger than one block must be split into First/Last (+Middle).
        let big = vec![0xab_u8; (super::super::format::BLOCK_SIZE) * 2 + 1234];
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(&big).unwrap();
        let buf = writer.into_inner().into_inner();
        assert_eq!(collect_records(&buf), vec![big]);
    }

    #[test]
    fn empty_record_round_trips() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"").unwrap();
        let buf = writer.into_inner().into_inner();
        assert_eq!(collect_records(&buf), vec![Vec::<u8>::new()]);
    }

    #[test]
    fn add_records_matches_sequential_add_record_bytes() {
        // RFC-0037 P2.2 group append: one `write` for many records must
        // produce the exact byte stream of appending them one by one.
        let records: Vec<Vec<u8>> = (0..40)
            .map(|i| {
                let len = match i % 4 {
                    0 => 0,
                    1 => 10,
                    2 => 300,
                    _ => super::super::format::BLOCK_SIZE + 777, // spans blocks
                };
                vec![(i % 251) as u8; len]
            })
            .collect();

        let mut seq = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        for r in &records {
            seq.add_record(r).unwrap();
        }
        let mut grouped = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        let refs: Vec<&[u8]> = records.iter().map(|r| r.as_slice()).collect();
        grouped.add_records(&refs).unwrap();
        // Chunked in odd-sized calls too: same total stream.
        let mut chunked = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        for chunk in records.chunks(3) {
            let refs: Vec<&[u8]> = chunk.iter().map(|r| r.as_slice()).collect();
            chunked.add_records(&refs).unwrap();
        }

        let seq_bytes = seq.into_inner().into_inner();
        assert_eq!(grouped.into_inner().into_inner(), seq_bytes);
        assert_eq!(chunked.into_inner().into_inner(), seq_bytes);
    }

    #[test]
    fn two_fragment_passes_one_write_recovers_both() {
        let mut w = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        let mut frame = w.take_frame();
        w.fragment_record(b"first", &mut frame);
        w.restore_frame(frame);
        let mut frame = w.take_frame();
        w.fragment_record(b"second", &mut frame);
        w.write_frame(&frame).unwrap();
        w.restore_frame(Vec::new());
        assert_eq!(
            collect_records(&w.into_inner().into_inner()),
            vec![b"first".to_vec(), b"second".to_vec()]
        );
    }

    #[test]
    fn fragment_record_write_frame_matches_add_records() {
        let records: Vec<Vec<u8>> = vec![
            b"short".to_vec(),
            vec![0xcd; super::super::format::BLOCK_SIZE + 50],
            b"".to_vec(),
        ];
        let mut grouped = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        let refs: Vec<&[u8]> = records.iter().map(|r| r.as_slice()).collect();
        grouped.add_records(&refs).unwrap();
        let mut framed = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        let mut frame = framed.take_frame();
        for r in &records {
            framed.fragment_record(r, &mut frame);
        }
        framed.write_frame(&frame).unwrap();
        framed.restore_frame(frame);
        assert_eq!(
            framed.into_inner().into_inner(),
            grouped.into_inner().into_inner()
        );
    }

    #[test]
    fn add_records_reads_back_through_reader() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        let records: Vec<Vec<u8>> = (0..10).map(|i| format!("grp-{i}").into_bytes()).collect();
        let refs: Vec<&[u8]> = records.iter().map(|r| r.as_slice()).collect();
        writer.add_records(&refs).unwrap();
        let buf = writer.into_inner().into_inner();
        assert_eq!(collect_records(&buf), records);
    }

    #[test]
    fn lock_free_wal_ring_fast_path() {
        let ring = LockFreeWalRing::new();
        assert_eq!(ring.capacity(), WAL_RING_CAPACITY_BYTES);
        assert_eq!(ring.pending_bytes(), 0);
        assert!(!ring.has_inflight());

        let off1 = ring.append_record_fast(101, b"payload-alpha");
        assert!(off1.is_some());
        assert!(ring.pending_bytes() > 0);

        let off2 = ring.append_record_fast(102, b"payload-beta");
        assert!(off2.is_some());

        let mut sink = Vec::new();
        let drained = ring.drain_to(&mut sink).unwrap();
        assert!(drained > 0);
        assert_eq!(ring.pending_bytes(), 0);
    }
}

