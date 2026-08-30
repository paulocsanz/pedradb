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
use super::format::{decode_crc, decode_length, RecordType, BLOCK_SIZE, HEADER_SIZE};
use super::recover_kernel::{
    fragment_act, is_length_resyncable, physical_payload_act, recover_collect_act, FragAct,
    FragKind, PhysicalAct, RecoverAct, RecoverKind,
};

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
    /// Stream offset just past the last record yielded by [`Self::collect_all`]
    /// — the last known-good append point (0 when nothing was recovered).
    last_good_offset: u64,
    /// F171 (pending): offset where the current resync walk began; promoted to
    /// `resync_origin` only if a later CRC-valid record re-anchors the log.
    pending_resync: Option<u64>,
    /// F171 (sticky): a resync walk skipped damaged bytes AND a later record
    /// re-anchored — the skipped region is lost from the recovered set and
    /// fail-closed callers must refuse the log (routine torn tails that walk
    /// to EOF without re-anchoring never set this).
    resync_origin: Option<u64>,
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
            last_good_offset: 0,
            pending_resync: None,
            resync_origin: None,
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
        reader.last_good_offset = offset;
        if !reader.read_next_block()? {
            return Ok(reader);
        }
        let within = usize::try_from(offset - block_start)
            .map_err(|_| CoreError::Internal("WAL offset does not fit usize".into()))?;
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
            if self.block_cursor + HEADER_SIZE > self.block_end && !self.read_next_block()? {
                // Truncated or clean EOF: any pending scratch is abandoned
                // (the crash happened mid-fragmented record).
                return Ok(None);
            }

            let header_offset = self.block_cursor;
            let rtype_byte = self.block[header_offset + 6];
            let length =
                decode_length([self.block[header_offset + 4], self.block[header_offset + 5]]);

            // A zero type + zero length header is block padding (or prealloc)
            // — legitimate only when the rest of the block is zero. A zero
            // header followed by live bytes is corruption and must fail
            // closed instead of silently swallowing the block's records.
            if rtype_byte == RecordType::Zero as u8 && length == 0 {
                if self.block[self.block_cursor + HEADER_SIZE..self.block_end]
                    .iter()
                    .any(|&b| b != 0)
                {
                    return Err(CoreError::WalZeroHeader {
                        offset: self.current_record_stream_offset(),
                    });
                }
                self.block_cursor = self.block_end;
                continue;
            }

            let rtype = RecordType::from_byte(rtype_byte).ok_or_else(|| {
                CoreError::Internal(format!(
                    "unknown record type {rtype_byte:#x} at stream offset {}",
                    self.current_record_stream_offset()
                ))
            })?;

            // Physical records never span blocks. Length vs block bounds is
            // F4: oversize / full-block overrun used to look like clean EOF.
            let max_payload = BLOCK_SIZE - HEADER_SIZE;
            let payload_start = header_offset + HEADER_SIZE;
            let payload_end_u64 = payload_start as u64 + length as u64;
            match physical_payload_act(
                length as u64,
                max_payload as u64,
                payload_end_u64,
                self.block_end as u64,
                BLOCK_SIZE as u64,
            ) {
                PhysicalAct::FailStop => {
                    if length > max_payload {
                        return Err(CoreError::Internal(format!(
                            "WAL record length {length} exceeds max physical payload {max_payload} at offset {}",
                            self.current_record_stream_offset()
                        )));
                    }
                    return Err(CoreError::Internal(format!(
                        "WAL record length {length} exceeds remainder of full block at offset {}",
                        self.current_record_stream_offset()
                    )));
                }
                PhysicalAct::Truncated => {
                    // Short final block: torn tail *or* length bitrot. Report
                    // Truncated so `collect_all` can keep a decoded prefix; a
                    // Truncated at offset 0 with no prior records fail-stops
                    // instead of a silently empty WAL (F4).
                    return Err(CoreError::Truncated(self.current_record_stream_offset()));
                }
                PhysicalAct::CleanEof => return Ok(None),
                PhysicalAct::Continue => {}
            }
            let payload_end = payload_start + length;

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

            if !crc::crc_match_ok(stored_crc, actual_crc) {
                return Err(CoreError::Crc {
                    offset: self.current_record_stream_offset(),
                    expected: stored_crc,
                    found: actual_crc,
                });
            }

            let payload = &self.block[payload_start..payload_end];
            self.block_cursor = payload_end;

            match fragment_act(FragKind::from_record_type(rtype), self.scratch.is_empty()) {
                FragAct::Yield => {
                    if rtype == RecordType::Full {
                        self.scratch.clear();
                        return Ok(Some(payload.to_vec()));
                    }
                    self.scratch.extend_from_slice(payload);
                    return Ok(Some(std::mem::take(&mut self.scratch)));
                }
                FragAct::Start => {
                    self.scratch.clear();
                    self.scratch.extend_from_slice(payload);
                }
                FragAct::Accumulate => {
                    self.scratch.extend_from_slice(payload);
                }
                FragAct::FailStop => {
                    // F14: orphan Middle/Last must not look like clean EOF.
                    let name = if rtype == RecordType::Middle {
                        "Middle"
                    } else {
                        "Last"
                    };
                    return Err(CoreError::Internal(format!(
                        "WAL orphan {name} fragment at offset {}",
                        self.current_record_stream_offset()
                    )));
                }
                FragAct::Skip => {}
                FragAct::CleanEof => return Ok(None),
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
    /// Stream offset just past the last record recovered by
    /// [`Self::collect_all`] — the last known-good append point. When recovery
    /// stopped early (torn tail / resynced framing damage) this is where the
    /// WAL should be truncated before new appends land, so a second crash
    /// never replays the damaged region as if it were records.
    #[must_use]
    pub fn last_good_offset(&self) -> u64 {
        self.last_good_offset
    }

    /// F171: `Some(offset)` when a resync walk skipped damaged bytes and a
    /// later CRC-valid record re-anchored the log — the skipped region is
    /// lost from the recovered set and fail-closed callers must refuse it.
    /// `None` on a clean log or a plain torn tail (no re-anchor).
    #[must_use]
    pub fn resync_origin(&self) -> Option<u64> {
        self.resync_origin
    }
}

impl<R: Read> WalReader<R> {
    /// Collect every remaining record into a `Vec`.
    ///
    /// Policy is [`recover_collect_act`] (F4 / F14 / CRC):
    /// - Clean EOF → stop.
    /// - Truncated / length / unknown type → resync one byte (mid-WAL length bitrot).
    /// - CRC at a fresh alignment and orphan fragment → **fail-stop** (do not resync).
    /// - CRC during a resync walk is the walk's own garbage: keep walking; at
    ///   EOF keep the prefix (torn tail), never journal it as disk corruption.
    /// - Resync at true EOF with a prefix → keep the prefix (torn tail).
    /// - Resync at true EOF with an empty prefix → fail-stop (F4 empty WAL).
    ///
    /// # Errors
    /// See [`WalReader::read_record`]; resync exhaustion.
    pub fn collect_all(&mut self) -> Result<Vec<Vec<u8>>> {
        let (out, err) = self.collect_prefix_all();
        match err {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    /// RFC-0047 P0.2: like [`Self::collect_all`] but instead of discarding
    /// the decoded prefix on a fail-stop (CRC / orphan) observation, returns
    /// it together with the error that stopped collection (`None` = clean
    /// end of log). Point-in-time recovery serves the prefix and reports
    /// the discard; it never turns a fail-stop into a silent skip.
    pub fn collect_prefix_all(&mut self) -> (Vec<Vec<u8>>, Option<CoreError>) {
        let mut out = Vec::new();
        let mut consecutive_skips = 0u64;
        // Inside an ongoing garbage walk? A CRC there is the walk's own
        // garbage (RFC-0038 D), not a corrupted real record.
        let mut in_resync = false;

        loop {
            let outcome = self.read_record();
            let kind = recover_kind_from_read(&outcome);
            let prefix_n = out.len() as u64;
            let can_skip = if is_length_resyncable(kind)
                || (in_resync && matches!(kind, RecoverKind::Crc | RecoverKind::ZeroHeaderTail))
            {
                match self.skip_byte_for_resync() {
                    Ok(v) => v,
                    Err(e) => return (out, Some(e)),
                }
            } else {
                false
            };
            let skips = if can_skip {
                consecutive_skips.saturating_add(1)
            } else {
                consecutive_skips
            };

            match recover_collect_act(kind, prefix_n, can_skip, skips, in_resync) {
                RecoverAct::KeepRecord => match outcome {
                    Ok(Some(rec)) => {
                        if in_resync {
                            // F171: this record re-anchored the log after a
                            // walk that skipped damaged bytes — the skipped
                            // region is lost; keep the earliest origin.
                            if let Some(p) = self.pending_resync.take() {
                                self.resync_origin = self.resync_origin.or(Some(p));
                            }
                        }
                        out.push(rec);
                        consecutive_skips = 0;
                        // A CRC-valid record re-anchors the alignment.
                        in_resync = false;
                        self.last_good_offset = self.current_record_stream_offset();
                    }
                    _ => {
                        return (
                            out,
                            Some(CoreError::Internal(
                                "WAL recover KeepRecord without a record".into(),
                            )),
                        );
                    }
                },
                RecoverAct::Stop | RecoverAct::KeepPrefix => {
                    // Walk ended without re-anchoring (torn tail at EOF):
                    // the skipped bytes are the torn region, not lost data.
                    self.pending_resync = None;
                    break;
                }
                RecoverAct::Resync => {
                    if !in_resync && self.pending_resync.is_none() {
                        self.pending_resync = Some(self.current_record_stream_offset());
                    }
                    consecutive_skips = skips;
                    in_resync = true;
                }
                RecoverAct::FailStop => {
                    return (
                        out,
                        match outcome {
                            Err(e) => Some(e),
                            Ok(_) => Some(CoreError::Internal(
                                "WAL recover fail-stop on clean read".into(),
                            )),
                        },
                    );
                }
            }
        }
        (out, None)
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

fn recover_kind_from_read(r: &Result<Option<Vec<u8>>>) -> RecoverKind {
    match r {
        Ok(Some(_)) => RecoverKind::Record,
        Ok(None) => RecoverKind::CleanEof,
        Err(e) => recover_kind(e),
    }
}

fn recover_kind(err: &CoreError) -> RecoverKind {
    match err {
        CoreError::Truncated(_) => RecoverKind::Truncated,
        CoreError::Crc { .. } => RecoverKind::Crc,
        CoreError::WalZeroHeader { .. } => RecoverKind::ZeroHeaderTail,
        CoreError::Internal(msg) => {
            if msg.contains("orphan") {
                RecoverKind::OrphanFragment
            } else if msg.contains("WAL record length")
                || msg.contains("exceeds max physical")
                || msg.contains("exceeds remainder of full block")
            {
                RecoverKind::LengthCorrupt
            } else if msg.contains("unknown record type") {
                RecoverKind::UnknownType
            } else {
                RecoverKind::Other
            }
        }
        _ => RecoverKind::Other,
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

    /// RFC-0152 P2.2.39: production `WalReader` gates stored vs computed
    /// through `crc_match_ok`. Live writer then XOR of the stored CRC field
    /// (payload intact) is Crc; AS-IS would accept. Direct
    /// `crc_mismatch_on_live_wal_is_not_ok` /
    /// `crc_mismatch_on_live_wal_open_is_not_ok` / `crc_mismatch_is_not_ok`
    /// are not this tooth. Equality of two u32s is not R-crc.
    #[test]
    fn crc_match_ok_on_live_wal_is_not_ok() {
        assert!(!crc::crc_match_ok(1, 2));
        assert!(
            crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any checksum matches"
        );
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"durable-payload").unwrap();
        let mut buf = writer.into_inner().into_inner();
        assert!(buf.len() > HEADER_SIZE);
        buf[0] ^= 0xff;
        let mut reader = WalReader::new(Cursor::new(buf));
        let err = reader.read_record().unwrap_err();
        assert!(matches!(err, CoreError::Crc { .. }), "got {err:?}");
    }

    /// RFC-0076 P0: production writer+reader; a flipped payload is Crc,
    /// never a valid record. AS-IS `crc_match_ok` would accept it.
    #[test]
    fn crc_mismatch_on_live_wal_is_not_ok() {
        assert!(!crc::crc_match_ok(1, 2));
        assert!(crc::crc_match_ok_as_is(1, 2));
        let mut writer = WalWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.add_record(b"durable-payload").unwrap();
        let mut buf = writer.into_inner().into_inner();
        assert!(buf.len() > HEADER_SIZE);
        buf[HEADER_SIZE] ^= 0xff;
        let mut reader = WalReader::new(Cursor::new(buf));
        let err = reader.read_record().unwrap_err();
        assert!(matches!(err, CoreError::Crc { .. }), "got {err:?}");
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
        let new_crc = super::super::crc::record_checksum(RecordType::Middle as u8, len, payload);
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

    #[test]
    fn recover_kind_tags_match_read_record_errors() {
        assert_eq!(
            recover_kind(&CoreError::Truncated(0)),
            RecoverKind::Truncated
        );
        assert_eq!(
            recover_kind(&CoreError::Crc {
                offset: 0,
                expected: 1,
                found: 2
            }),
            RecoverKind::Crc
        );
        assert_eq!(
            recover_kind(&CoreError::Internal(
                "WAL orphan Middle fragment at offset 3".into()
            )),
            RecoverKind::OrphanFragment
        );
        assert_eq!(
            recover_kind(&CoreError::Internal(
                "WAL record length 9 exceeds max physical payload 8 at offset 0".into()
            )),
            RecoverKind::LengthCorrupt
        );
        assert_eq!(
            recover_kind(&CoreError::Internal(
                "unknown record type 0x5 at stream offset 0".into()
            )),
            RecoverKind::UnknownType
        );
        assert!(!is_length_resyncable(RecoverKind::Crc));
        assert!(!is_length_resyncable(RecoverKind::OrphanFragment));
        assert!(is_length_resyncable(RecoverKind::Truncated));
        assert!(is_length_resyncable(RecoverKind::LengthCorrupt));
        assert!(is_length_resyncable(RecoverKind::UnknownType));
    }
}
