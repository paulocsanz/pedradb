//! SST file builder and reader (v2 block layout + sparse index).
//!
//! # On-disk v2
//! ```text
//! magic:        8 bytes  "PEDRSST\0"
//! version:      u32 LE   = 2
//! num_entries:  u64 LE
//! max_sequence: u64 LE
//! num_blocks:   u32 LE
//! // data blocks (concatenated):
//! //   each block: raw entry stream (ikey_len|ikey|val_len|val)*
//! // index:
//! //   for each block: offset u64, length u32, first_user_key_len u32, first_user_key
//! ```
//!
//! v1 files (flat entry list) are still readable.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bytes::Bytes;

use crate::env::{Env, EnvFile, StdEnv};
use crate::error::{CoreError, Result};
use crate::key::{InternalKey, SequenceNumber, ValueType};
use crate::memtable::{Lookup, MemTable};

/// File magic: PEDRSST + NUL.
pub const SST_MAGIC: &[u8; 8] = b"PEDRSST\0";
/// Legacy flat format.
pub const SST_VERSION_V1: u32 = 1;
/// Block + sparse index format.
pub const SST_VERSION: u32 = 2;
/// Target encoded size per data block.
pub const BLOCK_TARGET: usize = 4_096;

/// Absolute ceiling on SST entry count (defense-in-depth vs corrupt headers).
///
/// A real file cannot hold more entries than its byte length allows; we also
/// reject absurd counts before `Vec::with_capacity` (F2: bit-flip of
/// `num_entries` caused multi-EiB allocation attempts).
pub const MAX_SST_ENTRIES: usize = 64 * 1024 * 1024;

/// Minimum encoded bytes we assume per SST entry (`ikey_len` + `val_len` headers alone).
const MIN_ENCODED_ENTRY: usize = 8;

fn check_sst_entry_count(n: usize, file_len: usize, path: &Path) -> Result<()> {
    if n > MAX_SST_ENTRIES {
        return Err(CoreError::Internal(format!(
            "SST entry count {n} exceeds MAX_SST_ENTRIES ({MAX_SST_ENTRIES}) in {}",
            path.display()
        )));
    }
    // Even a 1-byte entry cannot exceed the file; tighter: min encoded size.
    let max_by_size = file_len / MIN_ENCODED_ENTRY + 1;
    if n > max_by_size {
        return Err(CoreError::Internal(format!(
            "SST entry count {n} impossible for file size {file_len} in {}",
            path.display()
        )));
    }
    Ok(())
}

/// Heuristic: legacy SSTs (no CRC trailer) parse cleanly as full buffer; if
/// stripping 4 bytes would break a v2 layout we already reject via CRC path.
fn looks_like_legacy_sst_without_crc(buf: &[u8]) -> bool {
    // Prefer CRC-required for all new magic files; only treat as legacy when
    // the CRC field is not present as a distinct trailer (tiny files).
    buf.len() < 32
}

fn check_sst_block_count(num_blocks: usize, file_len: usize, path: &Path) -> Result<()> {
    // Each block handle is at least 8+4+4 = 16 bytes in the index; data ≥ 0.
    let max_blocks = file_len / 16 + 1;
    if num_blocks > max_blocks || num_blocks > MAX_SST_ENTRIES {
        return Err(CoreError::Internal(format!(
            "SST block count {num_blocks} impossible for file size {file_len} in {}",
            path.display()
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct BlockHandle {
    offset: u64,
    length: u32,
    first_user_key: Bytes,
}

/// In-memory view of one SST file.
#[derive(Debug, Clone)]
pub struct SstTable {
    path: PathBuf,
    /// Sorted by [`InternalKey`] order (same as MemTable).
    entries: Vec<(InternalKey, Bytes)>,
    max_sequence: SequenceNumber,
    /// Sparse index (v2); empty for v1.
    index: Vec<BlockHandle>,
}

impl SstTable {
    /// Path of the SST file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of internal key versions stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Highest sequence number present in this file.
    #[must_use]
    pub fn max_sequence(&self) -> SequenceNumber {
        self.max_sequence
    }

    /// Number of data blocks in the sparse index (0 for legacy v1).
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.index.len()
    }

    /// Point lookup at `snapshot` (same semantics as [`MemTable::get`]).
    #[must_use]
    pub fn get(&self, user_key: &[u8], snapshot: SequenceNumber) -> Lookup {
        let mut i = self.lower_bound_user_key(user_key);
        while i < self.entries.len() {
            let (ikey, value) = &self.entries[i];
            if ikey.user_key.as_ref() != user_key {
                break;
            }
            if ikey.sequence <= snapshot {
                return match ikey.kind {
                    ValueType::Deletion => Lookup::Deleted,
                    ValueType::Value => Lookup::Found(value.clone()),
                };
            }
            i += 1;
        }
        Lookup::NotFound
    }

    /// Which index block would contain `user_key` (for tests / future lazy load).
    #[must_use]
    pub fn block_for_user_key(&self, user_key: &[u8]) -> Option<usize> {
        if self.index.is_empty() {
            return None;
        }
        // Last block whose first_user_key <= user_key.
        let mut best = 0usize;
        for (i, h) in self.index.iter().enumerate() {
            if h.first_user_key.as_ref() <= user_key {
                best = i;
            } else {
                break;
            }
        }
        Some(best)
    }

    /// Load an SST from disk (real filesystem).
    ///
    /// # Errors
    /// I/O or corrupt format.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_on(&StdEnv, path)
    }

    /// Load an SST via `env`.
    ///
    /// # Errors
    /// I/O or corrupt format.
    pub fn open_on(env: &impl Env, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = env.open_read(&path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Self::decode(&path, &buf)
    }

    fn decode(path: &Path, buf: &[u8]) -> Result<Self> {
        // New files append a 4-byte LE CRC32C of the preceding bytes (F3).
        // If the file starts with our magic and has a trailer, require a match
        // (fail-stop on bitrot). Legacy files without a valid trailer still parse
        // the full buffer for upgrade.
        let payload = if buf.len() >= 12 && buf.starts_with(SST_MAGIC) {
            let (head, tail) = buf.split_at(buf.len() - 4);
            let stored = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]);
            let computed = crc32c::crc32c(head);
            if stored == computed {
                head
            } else if looks_like_legacy_sst_without_crc(buf) {
                buf
            } else {
                return Err(CoreError::Internal(format!(
                    "SST file CRC mismatch in {} (stored {stored:#010x}, computed {computed:#010x})",
                    path.display()
                )));
            }
        } else {
            buf
        };

        let mut c = Cursor::new(payload);
        let magic = c.read_slice(8)?;
        if magic != SST_MAGIC {
            return Err(CoreError::Internal(format!(
                "bad SST magic in {}",
                path.display()
            )));
        }
        let version = c.read_u32()?;
        match version {
            SST_VERSION_V1 => Self::decode_v1(path, payload.len(), &mut c),
            SST_VERSION => Self::decode_v2(path, payload, &mut c),
            other => Err(CoreError::Internal(format!(
                "unsupported SST version {other} in {}",
                path.display()
            ))),
        }
    }

    fn decode_v1(path: &Path, file_len: usize, c: &mut Cursor<'_>) -> Result<Self> {
        let n = usize::try_from(c.read_u64()?).map_err(|_| {
            CoreError::Internal("SST entry count does not fit usize".into())
        })?;
        check_sst_entry_count(n, file_len, path)?;
        let mut entries = Vec::with_capacity(n);
        let mut max_sequence = 0;
        for _ in 0..n {
            let (ikey, value) = read_entry(c)?;
            max_sequence = max_sequence.max(ikey.sequence);
            entries.push((ikey, value));
        }
        let file_max = c.read_u64()?;
        if n > 0 {
            max_sequence = max_sequence.max(file_max);
        } else {
            max_sequence = file_max;
        }
        if !c.is_empty() {
            return Err(CoreError::Internal(format!(
                "trailing bytes in SST {}",
                path.display()
            )));
        }
        ensure_sorted(&entries, path)?;
        Ok(Self {
            path: path.to_path_buf(),
            entries,
            max_sequence,
            index: Vec::new(),
        })
    }

    fn decode_v2(path: &Path, buf: &[u8], c: &mut Cursor<'_>) -> Result<Self> {
        let n = usize::try_from(c.read_u64()?).map_err(|_| {
            CoreError::Internal("SST entry count does not fit usize".into())
        })?;
        check_sst_entry_count(n, buf.len(), path)?;
        let max_sequence = c.read_u64()?;
        let num_blocks = c.read_u32()? as usize;
        check_sst_block_count(num_blocks, buf.len(), path)?;
        let data_len = usize::try_from(c.read_u64()?).map_err(|_| {
            CoreError::Internal("SST data_len overflow".into())
        })?;
        let data_start = c.pos;
        let data_end = data_start
            .checked_add(data_len)
            .ok_or_else(|| CoreError::Internal("SST data overflow".into()))?;
        if data_end > buf.len() {
            return Err(CoreError::Internal(format!(
                "SST data extends past file in {}",
                path.display()
            )));
        }

        let mut index = Vec::with_capacity(num_blocks);
        let mut ic = Cursor::new(&buf[data_end..]);
        for _ in 0..num_blocks {
            let block_off = ic.read_u64()?;
            let block_len = ic.read_u32()?;
            let key_len = ic.read_u32()? as usize;
            let first_user_key = Bytes::copy_from_slice(ic.read_slice(key_len)?);
            index.push(BlockHandle {
                offset: block_off,
                length: block_len,
                first_user_key,
            });
        }
        if !ic.is_empty() {
            return Err(CoreError::Internal(format!(
                "trailing index bytes in SST {}",
                path.display()
            )));
        }

        let mut entries = Vec::with_capacity(n);
        for h in &index {
            let start = usize::try_from(h.offset).map_err(|_| {
                CoreError::Internal("block offset overflow".into())
            })?;
            let len = h.length as usize;
            let end = start
                .checked_add(len)
                .ok_or_else(|| CoreError::Internal("block length overflow".into()))?;
            if end > buf.len() {
                return Err(CoreError::Internal("block past EOF".into()));
            }
            let mut bc = Cursor::new(&buf[start..end]);
            while !bc.is_empty() {
                let (ikey, value) = read_entry(&mut bc)?;
                entries.push((ikey, value));
            }
        }

        if entries.len() != n {
            return Err(CoreError::Internal(format!(
                "SST entry count mismatch: header {n}, decoded {}",
                entries.len()
            )));
        }
        ensure_sorted(&entries, path)?;
        Ok(Self {
            path: path.to_path_buf(),
            entries,
            max_sequence,
            index,
        })
    }

    fn lower_bound_user_key(&self, user_key: &[u8]) -> usize {
        self.entries
            .partition_point(|(ikey, _)| ikey.user_key.as_ref() < user_key)
    }

    /// All internal versions in sorted order.
    pub fn iter_internal(&self) -> impl Iterator<Item = (&InternalKey, &Bytes)> + '_ {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// Clone all entries (for compaction merge).
    #[must_use]
    pub fn entries_cloned(&self) -> Vec<(InternalKey, Bytes)> {
        self.entries.clone()
    }
}

fn ensure_sorted(entries: &[(InternalKey, Bytes)], path: &Path) -> Result<()> {
    for w in entries.windows(2) {
        if w[0].0 > w[1].0 {
            return Err(CoreError::Internal(format!(
                "SST entries not sorted in {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn read_entry(c: &mut Cursor<'_>) -> Result<(InternalKey, Bytes)> {
    let ikey_len = c.read_u32()? as usize;
    let ikey_bytes = c.read_slice(ikey_len)?;
    let ikey = InternalKey::decode(ikey_bytes)?;
    let val_len = c.read_u32()? as usize;
    let value = Bytes::copy_from_slice(c.read_slice(val_len)?);
    Ok((ikey, value))
}

fn encode_entry(ikey: &InternalKey, value: &Bytes) -> Result<Vec<u8>> {
    let enc = ikey.encode();
    let ikey_len = u32::try_from(enc.len()).map_err(|_| {
        CoreError::Internal("internal key too large for SST".into())
    })?;
    let val_len = u32::try_from(value.len()).map_err(|_| {
        CoreError::Internal("value too large for SST".into())
    })?;
    let mut out = Vec::with_capacity(8 + enc.len() + value.len());
    out.extend_from_slice(&ikey_len.to_le_bytes());
    out.extend_from_slice(&enc);
    out.extend_from_slice(&val_len.to_le_bytes());
    out.extend_from_slice(value);
    Ok(out)
}

/// Write `mem` contents to a new SST at `path` (syncs file).
///
/// # Errors
/// I/O failures.
pub fn write_sst(path: impl AsRef<Path>, mem: &MemTable) -> Result<SstTable> {
    write_sst_on(&StdEnv, path, mem)
}

/// Write `mem` to SST via `env`.
///
/// # Errors
/// I/O failures.
pub fn write_sst_on(env: &impl Env, path: impl AsRef<Path>, mem: &MemTable) -> Result<SstTable> {
    let entries: Vec<(InternalKey, Bytes)> = mem
        .iter_internal()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    write_sst_entries_on(env, path, &entries)
}

/// Write pre-sorted (or sortable) internal entries to SST v2 (block + index).
///
/// # Errors
/// I/O failures.
pub fn write_sst_entries(
    path: impl AsRef<Path>,
    entries: &[(InternalKey, Bytes)],
) -> Result<SstTable> {
    write_sst_entries_on(&StdEnv, path, entries)
}

/// Write SST entries via `env` (fsyncs before return).
///
/// # Errors
/// I/O failures.
pub fn write_sst_entries_on(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: &[(InternalKey, Bytes)],
) -> Result<SstTable> {
    let path = path.as_ref();
    let mut sorted: Vec<(InternalKey, Bytes)> = entries.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut max_sequence = 0u64;
    for (ikey, _) in &sorted {
        max_sequence = max_sequence.max(ikey.sequence);
    }

    let mut data = Vec::new();
    let mut index: Vec<BlockHandle> = Vec::new();
    let mut block_buf = Vec::new();
    let mut block_first_user: Option<Bytes> = None;
    let data_base = 0u64; // relative; real offset = header_len + relative

    let flush_block = |data: &mut Vec<u8>,
                       block_buf: &mut Vec<u8>,
                       block_first_user: &mut Option<Bytes>,
                       index: &mut Vec<BlockHandle>|
     -> Result<()> {
        if block_buf.is_empty() {
            return Ok(());
        }
        let offset = data.len() as u64;
        let length = u32::try_from(block_buf.len()).map_err(|_| {
            CoreError::Internal("SST block too large".into())
        })?;
        let first = block_first_user
            .take()
            .ok_or_else(|| CoreError::Internal("block missing first key".into()))?;
        data.extend_from_slice(block_buf);
        block_buf.clear();
        index.push(BlockHandle {
            offset,
            length,
            first_user_key: first,
        });
        Ok(())
    };

    for (ikey, value) in &sorted {
        let enc = encode_entry(ikey, value)?;
        if !block_buf.is_empty() && block_buf.len() + enc.len() > BLOCK_TARGET {
            flush_block(&mut data, &mut block_buf, &mut block_first_user, &mut index)?;
        }
        if block_buf.is_empty() {
            block_first_user = Some(ikey.user_key.clone());
        }
        block_buf.extend_from_slice(&enc);
    }
    flush_block(&mut data, &mut block_buf, &mut block_first_user, &mut index)?;

    // Header: magic version num_entries max_seq num_blocks data_len
    let mut body = Vec::new();
    body.extend_from_slice(SST_MAGIC);
    body.extend_from_slice(&SST_VERSION.to_le_bytes());
    let n = u64::try_from(sorted.len()).map_err(|_| {
        CoreError::Internal("too many SST entries".into())
    })?;
    body.extend_from_slice(&n.to_le_bytes());
    body.extend_from_slice(&max_sequence.to_le_bytes());
    let num_blocks = u32::try_from(index.len()).map_err(|_| {
        CoreError::Internal("too many SST blocks".into())
    })?;
    body.extend_from_slice(&num_blocks.to_le_bytes());
    let data_len = u64::try_from(data.len()).map_err(|_| {
        CoreError::Internal("SST data too large".into())
    })?;
    body.extend_from_slice(&data_len.to_le_bytes());

    let header_len = body.len() as u64;
    // Fix block offsets to absolute file offsets.
    for h in &mut index {
        h.offset += header_len;
    }

    body.extend_from_slice(&data);
    for h in &index {
        body.extend_from_slice(&h.offset.to_le_bytes());
        body.extend_from_slice(&h.length.to_le_bytes());
        let kl = u32::try_from(h.first_user_key.len()).map_err(|_| {
            CoreError::Internal("user key too large".into())
        })?;
        body.extend_from_slice(&kl.to_le_bytes());
        body.extend_from_slice(&h.first_user_key);
    }

    let _ = data_base;
    // Trailing CRC32C over the whole SST body (F3: no prior integrity check).
    let file_crc = crc32c::crc32c(&body);
    body.extend_from_slice(&file_crc.to_le_bytes());
    {
        let mut file = env.create(path)?;
        file.write_all(&body)?;
        file.sync_all()?;
    }

    SstTable::open_on(env, path)
}

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
            .ok_or_else(|| CoreError::Internal("SST length overflow".into()))?;
        if end > self.data.len() {
            return Err(CoreError::Internal(format!(
                "SST truncated: need {len} at {}",
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pedradb-sst-{n}.sst"))
    }

    #[test]
    fn write_read_round_trip_v2() {
        let mut mem = MemTable::new();
        mem.put(b"a".as_slice(), 1, b"va".as_slice());
        mem.put(b"b".as_slice(), 2, b"vb".as_slice());
        mem.delete(b"a".as_slice(), 3);

        let path = temp_path();
        let table = write_sst(&path, &mem).unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(table.max_sequence(), 3);
        assert!(table.block_count() >= 1);
        assert_eq!(table.get(b"b", 10), Lookup::Found(Bytes::from_static(b"vb")));
        assert_eq!(table.get(b"a", 10), Lookup::Deleted);
        assert_eq!(table.get(b"a", 1), Lookup::Found(Bytes::from_static(b"va")));

        let reopened = SstTable::open(&path).unwrap();
        assert_eq!(reopened.get(b"b", 10), Lookup::Found(Bytes::from_static(b"vb")));
        assert!(reopened.block_for_user_key(b"b").is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn multi_block_index_points_into_key_space() {
        let mut entries = Vec::new();
        for i in 0..200u32 {
            let k = format!("k{i:04}");
            entries.push((
                InternalKey::new(Bytes::copy_from_slice(k.as_bytes()), u64::from(i) + 1, ValueType::Value),
                Bytes::from(vec![0u8; 32]),
            ));
        }
        let path = temp_path();
        let table = write_sst_entries(&path, &entries).unwrap();
        assert!(table.block_count() > 1, "expected multiple blocks");
        assert_eq!(
            table.get(b"k0100", 10_000),
            Lookup::Found(Bytes::from(vec![0u8; 32]))
        );
        let bi = table.block_for_user_key(b"k0100").unwrap();
        assert!(bi < table.block_count());
        let _ = std::fs::remove_file(&path);
    }
}
