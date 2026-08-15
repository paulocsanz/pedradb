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
        })
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
        let mut left = data.len();
        let mut begin = true;
        let mut off = 0usize;

        loop {
            let leftover = BLOCK_SIZE
                .checked_sub(self.block_offset)
                .expect("block_offset never exceeds BLOCK_SIZE");

            // Not enough room for even a header in this block: pad to the end
            // and continue in a fresh block.
            if leftover < HEADER_SIZE {
                let pad = [0u8; HEADER_SIZE];
                self.out.write_all(&pad[..leftover])?;
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

            self.emit_physical_record(rtype, &data[off..off + fragment_len])?;
            off += fragment_len;
            left -= fragment_len;
            begin = false;

            if left == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Emit a single physical record (header + payload fragment).
    ///
    /// # Panics
    /// Panics if `data.len()` exceeds `u16::MAX`, which is impossible by
    /// construction since fragments are bounded by a block's payload capacity.
    fn emit_physical_record(&mut self, rtype: RecordType, data: &[u8]) -> Result<()> {
        let length_u16 =
            u16::try_from(data.len()).expect("physical record fragment must fit in u16");

        let checksum = crc::record_checksum(rtype as u8, length_u16, data);

        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&checksum.to_le_bytes());
        header[4..6].copy_from_slice(&length_u16.to_le_bytes());
        header[6] = rtype as u8;

        let length = usize::from(length_u16);
        self.out.write_all(&header)?;
        self.out.write_all(data)?;
        self.block_offset += HEADER_SIZE + length;
        Ok(())
    }

    /// Flush buffered writes to the OS. Does **not** fsync — use a file
    /// wrapper for crash durability.
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying sink.
    pub fn flush(&mut self) -> Result<()> {
        self.out.flush()?;
        Ok(())
    }

    /// Byte offset in the sink after the last flush (start of next write).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] if the sink position cannot be queried.
    pub fn stream_position(&mut self) -> Result<u64> {
        self.out.flush()?;
        Ok(self.out.stream_position()?)
    }

    /// Consume the writer and return the underlying sink.
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Borrow the underlying sink mutably.
    pub(crate) fn inner_mut(&mut self) -> &mut W {
        &mut self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
        assert_eq!(collect_records(&buf), vec![Vec::new()]);
    }
}
