//! Append-only WAL writer.
//!
//! Mirrors RocksDB's `db/log_writer.cc` record-fragmentation algorithm: a
//! logical record is split across at most one block boundary using
//! `First`/`Middle`/`Last` physical records, and each physical record carries
//! a masked CRC32C over `{type, payload}`.

use std::io::{Seek, Write};

use crate::error::Result;

use super::crc;
use super::format::{RecordType, BLOCK_SIZE, HEADER_SIZE};

/// A streaming WAL writer over any `Write + Seek` sink.
///
/// Test-friendly: wrap a `Cursor<Vec<u8>>` in tests, or a real `File` in
/// production (see [`crate::wal::Wal`] for the fsync-aware file wrapper).
pub struct WalWriter<W> {
    out: W,
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
    /// RFC-0209 P0.2: user-space staging (opt-in via `PEDRA_WAL_BUFFER`).
    /// `None` (default) = the pre-0209 path, one sink write per frame.
    staged: Option<StagedBuf>,
}

/// Staging state for [`WalWriter`] (RFC-0209 P0.2).
struct StagedBuf {
    /// Pending framed bytes not yet handed to the sink.
    buf: Vec<u8>,
    /// Flush cap in bytes; `0` is the misuse guard (flush every append —
    /// byte-for-byte the AS-IS behavior; see `wal_buffer_kernel`).
    max: u64,
}

impl<W: Write + Seek> WalWriter<W> {
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
        Ok(Self {
            out,
            block_offset: pos % BLOCK_SIZE,
            position: raw_pos,
            frame: Vec::new(),
            staged: None,
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
    fn drain_staged(&mut self) -> Result<()> {
        let buf = match self.staged.as_mut() {
            Some(st) if !st.buf.is_empty() => std::mem::take(&mut st.buf),
            _ => return Ok(()),
        };
        match self.out.write_all(&buf) {
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
        self.out.write_all(&frame)?;
        self.position = self.position.saturating_add(frame.len() as u64);
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
        self.out.write_all(&frame)?;
        self.position = self.position.saturating_add(frame.len() as u64);
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
        let mut src = EncodedOpsSource::new(ops);
        let n = src.total;
        buf.reserve(n + 2 * HEADER_SIZE);
        self.fragment_from(&mut src, buf);
        n
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
                self.position = self.position.saturating_add(buf.len() as u64);
                if crate::wal_buffer_kernel::should_flush(st.buf.len() as u64, st.max) {
                    return self.drain_staged();
                }
                return Ok(());
            }
            self.out.write_all(buf)?;
            self.position = self.position.saturating_add(buf.len() as u64);
        }
        Ok(())
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
    fn patch_physical_record(
        &mut self,
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
        self.out.flush()?;
        Ok(())
    }

    /// Byte offset in the sink after the last flush (start of next write).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the sink position cannot be queried.
    pub fn stream_position(&mut self) -> Result<u64> {
        self.drain_staged()?;
        self.out.flush()?;
        Ok(self.out.stream_position()?)
    }

    /// Consume the writer and return the underlying sink. Drains staged
    /// bytes first (best-effort: a drain error is dropped — use the
    /// `Wal::close` path when it must propagate).
    pub fn into_inner(mut self) -> W {
        let _ = self.drain_staged();
        self.out
    }

    /// Borrow the underlying sink mutably.
    pub(crate) fn inner_mut(&mut self) -> &mut W {
        &mut self.out
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
            (Self { shared: shared.clone() }, shared)
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

    /// Drive the production 1-op path (`fragment_record` + `write_frame`,
    /// exactly what `Wal::write_pending_frame` does) for each record.
    fn drive_1op<W: Write + Seek>(w: &mut WalWriter<W>, records: &[Vec<u8>]) {
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
            vec![0x41; 150],           // exceeds the 100-byte cap → mid-flush
            b"".to_vec(),
            vec![0xcd; b + 50],        // spans blocks
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
        assert_eq!(
            collect_records(&plain_probe.bytes.borrow()),
            records
        );

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
        assert_eq!(staged_probe.writes.get(), 1, "staging: one write per flush cycle");
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
}
