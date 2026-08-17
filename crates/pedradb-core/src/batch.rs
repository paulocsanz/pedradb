//! Logical write records stored inside WAL payloads.
//!
//! Physical framing (blocks, CRC, fragmentation) lives in [`crate::wal`].
//! This module encodes **application** puts/deletes so recovery can rebuild
//! the MemTable without a format break when multi-op batches land in P0.4.
//!
//! # On-disk layout (version 1)
//!
//! ```text
//! version:   u8      = 1
//! count:     u32 LE  number of ops in this record
//! for each op:
//!   kind:    u8      ValueType (0=Deletion, 1=Value)
//!   seq:     u64 LE  sequence number for this op
//!   key_len: u32 LE
//!   key:     [u8; key_len]
//!   val_len: u32 LE  (0 for deletions)
//!   value:   [u8; val_len]
//! ```
//!
//! Self-describing records support future multi-key commits (one WAL record
//! per TX) and keep P1.6 export from needing a rewrite of historical logs.

use bytes::Bytes;

use crate::error::{CoreError, Result};
use crate::key::{SequenceNumber, ValueType};

/// Current logical record format version.
pub const WRITE_RECORD_VERSION: u8 = 1;

/// One put or delete inside a write record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOp {
    /// Put vs deletion.
    pub kind: ValueType,
    /// Sequence assigned to this op.
    pub sequence: SequenceNumber,
    /// User key.
    pub key: Bytes,
    /// Value (empty for deletions).
    pub value: Bytes,
}

impl WriteOp {
    /// Put `key → value` at `sequence`.
    #[must_use]
    pub fn put(sequence: SequenceNumber, key: impl Into<Bytes>, value: impl Into<Bytes>) -> Self {
        Self {
            kind: ValueType::Value,
            sequence,
            key: key.into(),
            value: value.into(),
        }
    }

    /// Delete `key` at `sequence`.
    #[must_use]
    pub fn delete(sequence: SequenceNumber, key: impl Into<Bytes>) -> Self {
        Self {
            kind: ValueType::Deletion,
            sequence,
            key: key.into(),
            value: Bytes::new(),
        }
    }

    /// Range-delete `[start, end)` at `sequence` (end stored in value).
    #[must_use]
    pub fn delete_range(
        sequence: SequenceNumber,
        start: impl Into<Bytes>,
        end: impl Into<Bytes>,
    ) -> Self {
        Self {
            kind: ValueType::RangeDeletion,
            sequence,
            key: start.into(),
            value: end.into(),
        }
    }
}

/// A batch of ops written as one logical WAL record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteRecord {
    /// Ordered ops (usually one for auto-commit; many after P0.4 TX commit).
    pub ops: Vec<WriteOp>,
}

impl WriteRecord {
    /// Empty record.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Single-op convenience.
    #[must_use]
    pub fn single(op: WriteOp) -> Self {
        Self { ops: vec![op] }
    }

    /// Encode to bytes for [`crate::wal::Wal::append_record`].
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(estimate_size(self));
        self.encode_into(&mut out);
        out
    }

    /// Encode into `out` (does not clear it). Same bytes as [`Self::encode`].
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        encode_ops(&self.ops, out);
    }

    /// Decode a payload produced by [`Self::encode`].
    ///
    /// # Errors
    /// Returns [`CoreError::Internal`] on truncated or unknown version/kind.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut cur = Cursor::new(bytes);
        let version = cur.read_u8()?;
        if version != WRITE_RECORD_VERSION {
            return Err(CoreError::Internal(format!(
                "unsupported write record version: {version}"
            )));
        }
        let count_raw = cur.read_u32()?;
        // F13: refuse multi-GiB `with_capacity` from a bit-flipped / hostile count
        // (same class as SST F2). Min op = kind+seq+key_len+val_len headers (17).
        let min_op: usize = 1 + 8 + 4 + 4;
        let rem = bytes.len().saturating_sub(cur.pos);
        let max_ops = rem / min_op;
        if count_raw as usize > max_ops {
            return Err(CoreError::Internal(format!(
                "write record op count {count_raw} exceeds remaining {rem} bytes"
            )));
        }
        let count = count_raw as usize;
        let mut ops = Vec::with_capacity(count);
        for _ in 0..count {
            let kind_byte = cur.read_u8()?;
            let kind = ValueType::from_u8(kind_byte).ok_or_else(|| {
                CoreError::Internal(format!("unknown write op kind: {kind_byte:#04x}"))
            })?;
            let sequence = cur.read_u64()?;
            let key_len = cur.read_u32()? as usize;
            let key = Bytes::copy_from_slice(cur.read_slice(key_len)?);
            let val_len = cur.read_u32()? as usize;
            let value = Bytes::copy_from_slice(cur.read_slice(val_len)?);
            ops.push(WriteOp {
                kind,
                sequence,
                key,
                value,
            });
        }
        if !cur.is_empty() {
            return Err(CoreError::Internal(
                "trailing bytes after write record".into(),
            ));
        }
        Ok(Self { ops })
    }

    /// Highest sequence number in the record, if any.
    #[must_use]
    pub fn max_sequence(&self) -> Option<SequenceNumber> {
        self.ops.iter().map(|o| o.sequence).max()
    }
}

/// Encode `ops` as one logical WAL payload (RFC-0040: no extra `WriteRecord` clone).
pub fn encode_ops(ops: &[WriteOp], out: &mut Vec<u8>) {
    // One resize, then indexed copies — apply_mc4 is 64 ops / ~32 KiB of
    // values; per-field `extend_from_slice` was a write-lock cost (RFC-0041).
    let n = 1 + 4 + ops.iter().map(op_encoded_len).sum::<usize>();
    let start = out.len();
    out.resize(start + n, 0);
    let buf = &mut out[start..];
    let mut i = 0;
    buf[i] = WRITE_RECORD_VERSION;
    i += 1;
    buf[i..i + 4].copy_from_slice(&(u32::try_from(ops.len()).unwrap_or(u32::MAX)).to_le_bytes());
    i += 4;
    for op in ops {
        buf[i] = op.kind.as_u8();
        i += 1;
        buf[i..i + 8].copy_from_slice(&op.sequence.to_le_bytes());
        i += 8;
        let kl = u32::try_from(op.key.len()).unwrap_or(u32::MAX);
        buf[i..i + 4].copy_from_slice(&kl.to_le_bytes());
        i += 4;
        let k = op.key.len();
        buf[i..i + k].copy_from_slice(&op.key);
        i += k;
        let vl = u32::try_from(op.value.len()).unwrap_or(u32::MAX);
        buf[i..i + 4].copy_from_slice(&vl.to_le_bytes());
        i += 4;
        let v = op.value.len();
        buf[i..i + v].copy_from_slice(&op.value);
        i += v;
    }
    debug_assert_eq!(i, n);
}

fn op_encoded_len(o: &WriteOp) -> usize {
    1 + 8 + 4 + o.key.len() + 4 + o.value.len()
}

fn estimate_size(rec: &WriteRecord) -> usize {
    1 + 4 + rec.ops.iter().map(op_encoded_len).sum::<usize>()
}

/// Minimal little-endian cursor for decoding (no external dep).
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_u8(&mut self) -> Result<u8> {
        let s = self.read_slice(1)?;
        Ok(s[0])
    }

    fn read_u32(&mut self) -> Result<u32> {
        let s = self.read_slice(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let s = self.read_slice(8)?;
        Ok(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| CoreError::Internal("write record length overflow".into()))?;
        if end > self.data.len() {
            return Err(CoreError::Internal(format!(
                "write record truncated: need {len} bytes at pos {}",
                self.pos
            )));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_put_and_delete() {
        let rec = WriteRecord {
            ops: vec![
                WriteOp::put(1, b"a".as_slice(), b"va".as_slice()),
                WriteOp::delete(2, b"b".as_slice()),
            ],
        };
        let encoded = rec.encode();
        let decoded = WriteRecord::decode(&encoded).unwrap();
        assert_eq!(decoded, rec);
        assert_eq!(decoded.max_sequence(), Some(2));
        let mut into = Vec::new();
        rec.encode_into(&mut into);
        assert_eq!(into, encoded);
    }

    #[test]
    fn rejects_bad_version() {
        let err = WriteRecord::decode(&[99, 0, 0, 0, 0]).unwrap_err();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn rejects_truncated() {
        assert!(WriteRecord::decode(&[WRITE_RECORD_VERSION, 1, 0, 0, 0]).is_err());
    }

    #[test]
    fn empty_record() {
        let rec = WriteRecord::new();
        let decoded = WriteRecord::decode(&rec.encode()).unwrap();
        assert!(decoded.ops.is_empty());
        assert_eq!(decoded.max_sequence(), None);
    }

    #[test]
    fn encode_ops_fat_apply_round_trip() {
        let ops: Vec<_> = (0..64u32)
            .map(|i| {
                WriteOp::put(
                    u64::from(i) + 1,
                    Bytes::copy_from_slice(&i.to_le_bytes()),
                    vec![b'v'; 64],
                )
            })
            .collect();
        let mut a = Vec::new();
        encode_ops(&ops, &mut a);
        let decoded = WriteRecord::decode(&a).unwrap();
        assert_eq!(decoded.ops.len(), 64);
        assert_eq!(decoded.ops[0], ops[0]);
        assert_eq!(decoded.ops[63], ops[63]);
        let mut b = vec![0xDE, 0xAD];
        encode_ops(&ops, &mut b);
        assert_eq!(&b[0..2], &[0xDE, 0xAD]);
        assert_eq!(&b[2..], a.as_slice());
    }

    /// F13: huge op count must fail-stop before multi-GiB allocation.
    #[test]
    fn decode_rejects_huge_op_count() {
        let mut raw = vec![WRITE_RECORD_VERSION];
        raw.extend_from_slice(&0x1000_0000u32.to_le_bytes()); // 268M ops, 5-byte payload
        let err = WriteRecord::decode(&raw).unwrap_err();
        assert!(err.to_string().contains("exceeds remaining"), "got {err}");
    }
}
