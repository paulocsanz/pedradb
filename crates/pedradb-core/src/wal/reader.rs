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

use std::io::{Read, Seek, SeekFrom};

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
}

impl<R: Read + Seek> WalReader<R> {
    /// Start reading at a **byte offset** that points at the start of a physical
    /// record (or block padding). Used for WAL export / follow-the-log.
    ///
    /// # Errors
    /// Seek/read failures.
    pub fn from_offset(mut src: R, offset: u64) -> Result<Self> {
        let block_size = BLOCK_SIZE as u64;
        let block_start = (offset / block_size) * block_size;
        src.seek(SeekFrom::Start(block_start))?;
        let mut reader = Self::new(src);
        reader.block_start_offset = block_start;
        if !reader.read_next_block()? {
            return Ok(reader);
        }
        let within = usize::try_from(offset - block_start).map_err(|_| {
            CoreError::Internal("WAL offset does not fit usize".into())
        })?;
        if within > reader.block_end {
            return Err(CoreError::Internal(format!(
                "WAL offset {offset} past end of block starting at {block_start}"
            )));
        }
        reader.block_cursor = within;
        // After positioning mid-block, the next read_next_block must advance
        // block_start from the true file position after this block.
        Ok(reader)
    }
}

impl<R: Read> WalReader<R> {

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

            // Physical records never span blocks. A length larger than the max
            // payload in a block is always corrupt (bitrot of length used to
            // look like clean EOF and drop the rest of the WAL — F4).
            let max_payload = BLOCK_SIZE - HEADER_SIZE;
            if length > max_payload {
                return Err(CoreError::Internal(format!(
                    "WAL record length {length} exceeds max physical payload {max_payload} at offset {}",
                    self.current_record_stream_offset()
                )));
            }

            let payload_start = header_offset + HEADER_SIZE;
            let payload_end = payload_start + length;

            if payload_end > self.block_end {
                // Payload extends past what we have.
                // Full block → corrupt length (writer always fragments).
                if self.block_end == BLOCK_SIZE {
                    return Err(CoreError::Internal(format!(
                        "WAL record length {length} exceeds remainder of full block at offset {}",
                        self.current_record_stream_offset()
                    )));
                }
                // Short final block: could be a crash torn tail *or* bitrot of
                // length. Report Truncated so `collect_all` can keep any prefix
                // already decoded; a Truncated at offset 0 with no prior records
                // fails open (fail-stop) instead of silently empty WAL (F4).
                return Err(CoreError::Truncated(self.current_record_stream_offset()));
            }

            let stored_crc = decode_crc([
                self.block[header_offset],
                self.block[header_offset + 1],
                self.block[header_offset + 2],
                self.block[header_offset + 3],
            ]);
            let length_u16 = u16::try_from(length).unwrap_or(u16::MAX);
            let actual_crc = crc::record_checksum(
                rtype_byte,
                length_u16,
                &self.block[payload_start..payload_end],
            );

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
                        // F14: orphan Middle must not look like clean EOF — that
                        // silently dropped every durable record after a bit-flipped
                        // type byte (SilentWrong class). Fail-stop instead.
                        return Err(CoreError::Internal(format!(
                            "WAL orphan Middle fragment at offset {}",
                            self.current_record_stream_offset()
                        )));
                    }
                    self.scratch.extend_from_slice(payload);
                }
                RecordType::Last => {
                    if self.scratch.is_empty() {
                        // F14: same as Middle — not clean EOF.
                        return Err(CoreError::Internal(format!(
                            "WAL orphan Last fragment at offset {}",
                            self.current_record_stream_offset()
                        )));
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
    /// Collect every remaining record into a `Vec`.
    ///
    /// - Clean EOF (`Ok(None)`) → stop.
    /// - [`CoreError::Truncated`] / length-style Internal: **resync** one byte at a
    ///   time for the next CRC-valid record (F4 residual mid-WAL length bitrot).
    /// - [`CoreError::Crc`] after a non-empty prefix → keep prefix only (do **not**
    ///   resync: false alignments skipped durable later records and exploded
    ///   `SilentWrong` counts in the dense sweep).
    /// - Resync at true EOF with prefix → return prefix (torn tail).
    ///
    /// # Errors
    /// See [`WalReader::read_record`]; resync exhaustion.
    pub fn collect_all(&mut self) -> Result<Vec<Vec<u8>>> {
        const MAX_CONSECUTIVE_SKIPS: u64 = 4 * 1024 * 1024;
        let mut out = Vec::new();
        let mut consecutive_skips = 0u64;

        loop {
            match self.read_record() {
                Ok(Some(rec)) => {
                    out.push(rec);
                    consecutive_skips = 0;
                }
                Ok(None) => break,
                // CRC mismatch is always fail-stop (do not soft-drop later records,
                // and do not resync — both produced SilentWrong in dense sweeps).
                Err(e) if is_length_resyncable(&e) => {
                    if !self.skip_byte_for_resync()? {
                        if out.is_empty() {
                            return Err(e);
                        }
                        break;
                    }
                    consecutive_skips += 1;
                    if consecutive_skips > MAX_CONSECUTIVE_SKIPS {
                        return Err(CoreError::Internal(
                            "WAL resync exceeded max consecutive skips".into(),
                        ));
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Advance one byte past a bad physical header so the next
    /// [`read_record`] can try a new alignment.
    ///
    /// Returns `false` when the stream is exhausted.
    fn skip_byte_for_resync(&mut self) -> Result<bool> {
        self.scratch.clear();
        if self.block_cursor < self.block_end {
            self.block_cursor += 1;
            return Ok(true);
        }
        if !self.read_next_block()? {
            return Ok(false);
        }
        Ok(true)
    }
}

fn is_length_resyncable(err: &CoreError) -> bool {
    match err {
        CoreError::Truncated(_) => true,
        CoreError::Internal(msg) => {
            // Length / framing errors, and mid-scan junk headers while resyncing.
            msg.contains("WAL record length")
                || msg.contains("exceeds max physical")
                || msg.contains("exceeds remainder of full block")
                || msg.contains("unknown record type")
        }
        _ => false,
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
    fn recover_from_offset_skips_prefix() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"first").unwrap();
        let mid = writer.stream_position().unwrap();
        writer.add_record(b"second").unwrap();
        writer.add_record(b"third").unwrap();
        let buf = writer.into_inner().into_inner();
        let rest = WalReader::from_offset(Cursor::new(buf), mid)
            .unwrap()
            .collect_all()
            .unwrap();
        assert_eq!(rest, vec![b"second".to_vec(), b"third".to_vec()]);
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
    fn truncated_tail_is_reported_not_silent_eof() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"complete").unwrap();
        writer.add_record(b"incomplete-at-crash").unwrap();
        let mut buf = writer.into_inner().into_inner();

        // Cut off the tail so the second record's payload is truncated.
        buf.truncate(buf.len() - 5);

        let mut reader = WalReader::new(Cursor::new(buf));
        assert_eq!(reader.read_record().unwrap(), Some(b"complete".to_vec()));
        // Full header + short payload → Truncated (not silent Ok(None)).
        let err = reader.read_record().unwrap_err();
        assert!(matches!(err, CoreError::Truncated(_)), "got {err:?}");
    }

    #[test]
    fn length_bitrot_mid_wal_resync_recovers_suffix() {
        // F4 residual: flip length high-byte mid-log; resync must still see later records.
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"first").unwrap();
        writer.add_record(b"second").unwrap();
        writer.add_record(b"third-overwrite").unwrap();
        let mut buf = writer.into_inner().into_inner();

        // Second record starts after first Full: header 7 + payload "first"(5) = 12.
        // Header layout: crc[0..4] len[4..6] type[6]. Flip high byte of length.
        let second_hdr = HEADER_SIZE + 5;
        assert!(second_hdr + 6 < buf.len());
        buf[second_hdr + 5] ^= 0x01;

        let recs = WalReader::new(Cursor::new(buf)).collect_all().unwrap();
        assert_eq!(recs[0], b"first");
        // "second" may be lost if we skip past it during resync from corrupt length;
        // "third-overwrite" must be recovered (the residual SilentWrong case).
        assert!(
            recs.iter().any(|r| r == b"third-overwrite"),
            "expected suffix recovered after resync, got {recs:?}"
        );
    }

    #[test]
    fn torn_tail_after_complete_prefix_keeps_prefix() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"durable").unwrap();
        writer.add_record(b"torn-at-crash").unwrap();
        let mut buf = writer.into_inner().into_inner();
        buf.truncate(buf.len() - 5);
        let recs = WalReader::new(Cursor::new(buf)).collect_all().unwrap();
        assert_eq!(recs, vec![b"durable".to_vec()]);
    }

    /// Multi-block logical records: many large payloads + prefix/suffix recover.
    #[test]
    fn multi_block_stress_round_trip() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        let mut expected = Vec::new();
        for i in 0..50u32 {
            // ~1.5 blocks each → First/Middle/Last fragmentation.
            let payload = vec![u8::try_from(i % 251).unwrap(); BLOCK_SIZE + BLOCK_SIZE / 2 + 17];
            writer.add_record(&payload).unwrap();
            expected.push(payload);
        }
        let buf = writer.into_inner().into_inner();
        assert!(buf.len() > BLOCK_SIZE * 50, "should span many blocks");
        let got = WalReader::new(Cursor::new(buf)).collect_all().unwrap();
        assert_eq!(got.len(), expected.len());
        assert_eq!(got, expected);
    }

    #[test]
    fn orphan_middle_fail_stops_not_silent_eof() {
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"first").unwrap();
        writer.add_record(b"second").unwrap();
        writer.add_record(b"third").unwrap();
        let mut buf = writer.into_inner().into_inner();
        // Second Full: header at offset 7+5=12; type byte at +6.
        let second_type = HEADER_SIZE + 5 + 6;
        assert_eq!(buf[second_type], RecordType::Full as u8);
        buf[second_type] = RecordType::Middle as u8;
        // CRC no longer matches → Crc fail-stop (also good). Flip CRC to match
        // forged type so we hit the orphan-Middle path specifically.
        let len = u16::from_le_bytes([buf[HEADER_SIZE + 5 + 4], buf[HEADER_SIZE + 5 + 5]]);
        let payload_start = HEADER_SIZE + 5 + HEADER_SIZE;
        let payload = &buf[payload_start..payload_start + len as usize];
        let new_crc = super::super::crc::record_checksum(
            RecordType::Middle as u8,
            len,
            payload,
        );
        buf[HEADER_SIZE + 5..HEADER_SIZE + 5 + 4].copy_from_slice(&new_crc.to_le_bytes());

        let err = WalReader::new(Cursor::new(buf)).collect_all().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("orphan Middle") || msg.contains("crc"),
            "expected orphan/crc fail-stop, got {msg}"
        );
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
