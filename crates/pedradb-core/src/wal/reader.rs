//! WAL reader: parses physical records out of blocks and reassembles them
//! into logical application records, verifying CRCs along the way.
//!
//! Semantics mirror RocksDB's `db/log_reader.cc`:
//!   * `RecordType::Zero` padding (a header whose type/length are both zero)
//!     signals the tail of a block — we skip to the next block.
//!   * A short read that truncates a header is treated as a clean end of log
//!     (the writer crashed before finishing the record) and yields `Ok(None)`.
//!   * A valid-looking header whose payload is truncated by EOF also stops the
//!     stream cleanly.
//!   * A CRC mismatch yields a [`CoreError::Crc`].

use std::io::Read;

use crate::error::{CoreError, Result};

use super::crc;
use super::format::{decode_crc, decode_length, BLOCK_SIZE, HEADER_SIZE, RecordType};

/// A streaming reader that yields logical records from a byte source.
pub struct WalReader<R> {
    src: R,
    /// The current block under examination.
    block: Vec<u8>,
    /// How many valid bytes are in [`Self::block`] for the current block.
    block_end: usize,
    /// Cursor into [`Self::block`] for the next physical record.
    block_cursor: usize,
    /// Accumulator for multi-fragment records.
    scratch: Vec<u8>,
    /// Byte offset in the stream where the current block started.
    block_start_offset: u64,
}

impl<R: Read> WalReader<R> {
    /// Build a reader that starts reading physical records from the current
    /// position of `src`.
    pub fn new(src: R) -> Self {
        Self {
            src,
            block: vec![0u8; BLOCK_SIZE],
            block_end: 0,
            block_cursor: 0,
            scratch: Vec::new(),
            block_start_offset: 0,
        }
    }

    /// Read the next complete logical record, or `Ok(None)` at a clean
    /// end-of-log.
    ///
    /// # Errors
    /// Returns [`CoreError::Crc`] when a physical record's stored checksum
    /// does not match the recomputed value, or [`CoreError::Internal`] for an
    /// unknown record type byte.
    pub fn read_record(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            // Need at least HEADER_SIZE bytes from the current block.
            if self.block_cursor + HEADER_SIZE > self.block_end
                && !self.read_next_block()?
            {
                // Truncated or clean EOF: any pending scratch is abandoned
                // (the crash happened mid-fragmented record).
                return Ok(None);
            }

            let header_offset = self.block_cursor;
            let rtype_byte = self.block[header_offset + 6];
            let length = decode_length([
                self.block[header_offset + 4],
                self.block[header_offset + 5],
            ]);

            // A zero type + zero length header is block padding (or prealloc).
            if rtype_byte == RecordType::Zero as u8 && length == 0 {
                // Skip the remainder of this block.
                self.block_cursor = self.block_end;
                continue;
            }

            let rtype = RecordType::from_byte(rtype_byte).ok_or_else(|| {
                CoreError::Internal(format!(
                    "unknown record type {rtype_byte:#x} at stream offset {}",
                    self.current_record_stream_offset()
                ))
            })?;

            let payload_start = header_offset + HEADER_SIZE;
            let payload_end = payload_start + length;

            if payload_end > self.block_end {
                // Payload extends past what we have: truncated record (crash).
                return Ok(None);
            }

            let stored_crc = decode_crc([
                self.block[header_offset],
                self.block[header_offset + 1],
                self.block[header_offset + 2],
                self.block[header_offset + 3],
            ]);
            let actual_crc =
                crc::record_checksum(rtype_byte, &self.block[payload_start..payload_end]);

            if stored_crc != actual_crc {
                return Err(CoreError::Crc {
                    offset: self.current_record_stream_offset(),
                    expected: stored_crc,
                    found: actual_crc,
                });
            }

            let payload = &self.block[payload_start..payload_end];
            self.block_cursor = payload_end;

            match rtype {
                RecordType::Full => {
                    self.scratch.clear();
                    return Ok(Some(payload.to_vec()));
                }
                RecordType::First => {
                    self.scratch.clear();
                    self.scratch.extend_from_slice(payload);
                }
                RecordType::Middle => {
                    if self.scratch.is_empty() {
                        // Middle without a preceding First: corrupt log.
                        return Ok(None);
                    }
                    self.scratch.extend_from_slice(payload);
                }
                RecordType::Last => {
                    if self.scratch.is_empty() {
                        // Last without a preceding First: corrupt log.
                        return Ok(None);
                    }
                    self.scratch.extend_from_slice(payload);
                    return Ok(Some(std::mem::take(&mut self.scratch)));
                }
                RecordType::Zero => {} // non-padding zero (length != 0): unusual, skipped
            }
        }
    }

    /// Fill `self.block` with the next up-to-`BLOCK_SIZE` bytes. Returns
    /// `false` when there is nothing left to read (clean EOF).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] propagated from the underlying source.
    fn read_next_block(&mut self) -> Result<bool> {
        self.block_start_offset += self.block_end as u64;
        self.block_cursor = 0;
        let mut filled = 0usize;
        while filled < BLOCK_SIZE {
            let n = self.src.read(&mut self.block[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        self.block_end = filled;
        Ok(filled > 0)
    }

    /// Stream offset (start of file) where the physical record at
    /// `block_cursor` begins.
    fn current_record_stream_offset(&self) -> u64 {
        self.block_start_offset + self.block_cursor as u64
    }
}

impl<R: Read> WalReader<R> {
    /// Collect every remaining record into a `Vec`. Errors on the first bad
    /// record.
    ///
    /// # Errors
    /// See [`WalReader::read_record`].
    pub fn collect_all(&mut self) -> Result<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        while let Some(rec) = self.read_record()? {
            out.push(rec);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Seek};

    use super::super::writer::WalWriter;

    fn round_trip(records: &[Vec<u8>]) -> Vec<Vec<u8>> {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        for r in records {
            writer.add_record(r).unwrap();
        }
        let buf = writer.into_inner().into_inner();
        WalReader::new(Cursor::new(buf)).collect_all().unwrap()
    }

    #[test]
    fn round_trips_mixed_sizes() {
        let records = vec![
            b"".to_vec(),
            b"x".to_vec(),
            b"a fairly normal length record".to_vec(),
            vec![0x7e_u8; 1],
            vec![0xa5_u8; BLOCK_SIZE * 3 + 17],
            b"trailing".to_vec(),
        ];
        assert_eq!(round_trip(&records), records);
    }

    #[test]
    fn detects_crc_corruption() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"clean record").unwrap();
        let mut buf = writer.into_inner().into_inner();

        // Flip a payload byte inside the first record (after the 7-byte header).
        buf[HEADER_SIZE] ^= 0xff;

        let mut reader = WalReader::new(Cursor::new(buf));
        let err = reader.read_record().unwrap_err();
        assert!(matches!(err, CoreError::Crc { .. }), "got {err:?}");
    }

    #[test]
    fn truncated_tail_is_clean_eof() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"complete").unwrap();
        writer.add_record(b"incomplete-at-crash").unwrap();
        let mut buf = writer.into_inner().into_inner();

        // Cut off the tail so the second record's payload is truncated.
        buf.truncate(buf.len() - 5);

        let mut reader = WalReader::new(Cursor::new(buf));
        assert_eq!(reader.read_record().unwrap(), Some(b"complete".to_vec()));
        // The truncated second record should look like clean EOF, not an error.
        assert_eq!(reader.read_record().unwrap(), None);
    }

    #[test]
    fn reader_can_seek_and_replay() {
        // Recovery scenario: open the file, seek to 0, replay all records.
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        for i in 0..10 {
            writer.add_record(format!("rec-{i}").as_bytes()).unwrap();
        }
        let mut file = writer.into_inner();
        file.rewind().unwrap();
        let recs = WalReader::new(file).collect_all().unwrap();
        assert_eq!(recs.len(), 10);
        assert_eq!(recs[0], b"rec-0");
        assert_eq!(recs[9], b"rec-9");
    }
}
