//! SST file builder and reader (v2/v3 block layout + sparse index + bloom).
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
//! # On-disk v3
//! Same as v2, then a bloom filter section:
//! ```text
//! bloom: nbits u32 | k u32 | nbytes u32 | bits[nbytes]
//! ```
//! Trailing file CRC32C covers the full body (all versions that write CRC).
//!
//! # On-disk v4
//! v3 + lz4-compressed data blocks (no per-block CRC).
//!
//! # On-disk v5 (compressed writer default, RFC-0077 P1.1)
//! v4 + 4-byte CRC32C after each data block (`sst_block_crc_ok`).
//!
//! v1–v4 files are still readable.
//!
//! # Lazy blocks (RFC-0014 P1.2)
//!
//! v2+ tables keep the CRC-stripped payload and sparse index in memory and
//! **decode data blocks on demand** for point gets and bounded ranges. Full
//! entry materialization happens only for compaction / whole-table clone.
//! Range tombstones are extracted once at open so point gets stay correct
//! without scanning every block for deletes.

use std::cell::Cell;
use std::io::{Read, Write};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::bloom::{BloomFilter, DEFAULT_BITS_PER_KEY};
use crate::env::{Env, EnvFile, StdEnv};
use crate::error::{CoreError, Result};
use crate::key::{InternalKey, SequenceNumber, ValueType};
use crate::memtable::{Lookup, MemTable};
use crate::merge::user_key_in_range;

/// File magic: PEDRSST + NUL.
pub const SST_MAGIC: &[u8; 8] = b"PEDRSST\0";
/// Legacy flat format.
pub const SST_VERSION_V1: u32 = 1;
/// Block + sparse index format (no on-disk bloom).
pub const SST_VERSION_V2: u32 = 2;
/// Block + sparse index + bloom filter (uncompressed blocks).
pub const SST_VERSION_V3: u32 = 3;
/// Block + sparse index + bloom + lz4-compressed data blocks (no per-block CRC).
pub const SST_VERSION_V4: u32 = 4;
/// v4 + per-block CRC32C (compressed writer default, RFC-0077 P1.1).
pub const SST_VERSION: u32 = 5;
/// Target encoded size per data block (pre-compression).
pub const BLOCK_TARGET: usize = 4_096;

/// Absolute ceiling on SST entry count (defense-in-depth vs corrupt headers).
///
/// A real file cannot hold more entries than its byte length allows; we also
/// reject absurd counts before `Vec::with_capacity` (F2: bit-flip of
/// `num_entries` caused multi-EiB allocation attempts).
pub const MAX_SST_ENTRIES: usize = 64 * 1024 * 1024;

thread_local! {
    static SST_BLOCKS_DECODED: Cell<usize> = const { Cell::new(0) };
}

/// Reset the thread-local SST block-decode counter (RFC-0033 tests).
pub fn reset_sst_blocks_decoded() {
    SST_BLOCKS_DECODED.with(|c| c.set(0));
}

/// Blocks actually decompressed on this thread since the last reset.
#[must_use]
pub fn sst_blocks_decoded() -> usize {
    SST_BLOCKS_DECODED.with(Cell::get)
}

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

/// Cached full entry materialization for an SST (shared across clones).
type EntriesCache = Arc<Mutex<Option<Vec<(InternalKey, Bytes)>>>>;

/// In-memory view of one SST file.
///
/// For v2+ (`payload` non-empty): data blocks are decoded lazily. For v1:
/// all entries are eager in the materialization cache.
#[derive(Debug, Clone)]
pub struct SstTable {
    path: PathBuf,
    /// CRC-stripped file body for lazy block decode (empty for legacy v1).
    payload: Arc<[u8]>,
    /// Whether data blocks are lz4 (SST v4+).
    compressed_blocks: bool,
    /// Whether each data block carries a trailing CRC32C (SST v5).
    block_crc: bool,
    /// Cached full decode (`None` until first materialize for lazy tables).
    entries: EntriesCache,
    /// Range tombstones extracted at open (lazy tables) or from entries (v1).
    range_tombstones: Vec<(InternalKey, Bytes)>,
    /// Header entry count (or materialized length for v1).
    num_entries: usize,
    max_sequence: SequenceNumber,
    /// Sparse index (v2+); empty for v1.
    index: Vec<BlockHandle>,
    /// On-disk or rebuilt bloom (always-true when inactive).
    bloom: BloomFilter,
    /// Smallest user key in file (None if empty).
    smallest_user_key: Option<Bytes>,
    /// Largest user key in file (None if empty).
    largest_user_key: Option<Bytes>,
    /// Column-family name (RFC-0065). Empty = mixed / prefix-era.
    cf: String,
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
        self.num_entries
    }

    /// Data-block count for the at-rest scrub (RFC-0060). v1 files have no
    /// sparse index — they count as one file-level block after the CRC check.
    #[must_use]
    pub fn data_block_count(&self) -> usize {
        self.index.len().max(1)
    }

    /// Whether the table has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.num_entries == 0
    }

    /// Whether this table uses on-demand block decode (v2+ with retained payload).
    #[must_use]
    pub fn is_lazy(&self) -> bool {
        !self.payload.is_empty() && !self.index.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn materialize_cache_filled(&self) -> bool {
        self.entries.lock().is_some()
    }

    /// Append range tombstones visible at `snapshot` into `out` (no full materialize).
    pub fn collect_range_tombstones(
        &self,
        snapshot: SequenceNumber,
        out: &mut Vec<crate::merge::RangeTombstone>,
    ) {
        if self.range_tombstones.is_empty() {
            return;
        }
        for (ikey, end) in &self.range_tombstones {
            if ikey.sequence > snapshot {
                continue;
            }
            out.push(crate::merge::RangeTombstone {
                start: ikey.user_key.clone(),
                end: end.clone(),
                sequence: ikey.sequence,
            });
        }
    }

    /// Point version at `user_key` ≤ `snapshot`, ignoring range tombstones.
    ///
    /// Loads one data block for lazy tables. Returns `(sequence, lookup)`.
    #[must_use]
    pub fn point_at(
        &self,
        user_key: &[u8],
        snapshot: SequenceNumber,
    ) -> Option<(SequenceNumber, Lookup)> {
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            if user_key < lo || user_key > hi {
                // Still may need range del from start < lo covering key — handled
                // by caller collecting tombstones. Point cannot be here if key > hi.
                if user_key < lo {
                    return None;
                }
                if user_key > hi {
                    return None;
                }
            }
        }
        if !self.bloom.may_contain(user_key) && !self.has_range_tombstones() {
            return None;
        }
        // Bloom miss: still probe if only checking points with active bloom.
        if !self.bloom.may_contain(user_key) {
            return None;
        }
        self.point_in_blocks(user_key, snapshot, &mut |bi| {
            self.decode_block(bi).ok().map(Arc::new)
        })
    }

    /// Like [`point_at`] but loads blocks via `load` (caller may hit a block cache).
    ///
    /// Visibility is identical to [`point_at`]. Used by `Db::lookup` so zipfian
    /// point-gets do not lz4-decode the same block on every seek.
    pub fn point_at_with<F>(
        &self,
        user_key: &[u8],
        snapshot: SequenceNumber,
        load: F,
    ) -> Option<(SequenceNumber, Lookup)>
    where
        F: FnMut(usize) -> Option<Arc<Vec<(InternalKey, Bytes)>>>,
    {
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            if user_key < lo || user_key > hi {
                return None;
            }
        }
        if !self.bloom.may_contain(user_key) {
            return None;
        }
        let mut load = load;
        self.point_in_blocks(user_key, snapshot, &mut load)
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

    /// Whether the on-disk / rebuilt bloom is active.
    #[must_use]
    pub fn has_bloom(&self) -> bool {
        self.bloom.is_active()
    }

    /// Smallest user key, if any.
    #[must_use]
    pub fn smallest_user_key(&self) -> Option<&[u8]> {
        self.smallest_user_key.as_deref()
    }

    /// Largest user key, if any.
    #[must_use]
    pub fn largest_user_key(&self) -> Option<&[u8]> {
        self.largest_user_key.as_deref()
    }

    /// Column-family tag (empty = mixed / unknown).
    #[must_use]
    pub fn cf(&self) -> &str {
        &self.cf
    }

    /// Overlay the MANIFEST CF tag (empty leaves the inferred value).
    #[must_use]
    pub fn with_cf(mut self, cf: String) -> Self {
        if !cf.is_empty() {
            self.cf = cf;
        }
        self
    }

    /// Fast negative: key cannot be in this file (bounds and/or bloom).
    #[must_use]
    pub fn key_may_match(&self, user_key: &[u8]) -> bool {
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            if user_key < lo || user_key > hi {
                return false;
            }
        }
        self.bloom.may_contain(user_key)
    }

    /// Point lookup at `snapshot` (same semantics as [`MemTable::get`]).
    ///
    /// Lazy tables load **one** data block (plus open-time range tombstones).
    #[must_use]
    pub fn get(&self, user_key: &[u8], snapshot: SequenceNumber) -> Lookup {
        // Bounds prune only (bloom can miss keys covered solely by a range tombstone
        // whose start key differs from `user_key`).
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            // Range tombstone end may extend past largest point key; still probe if
            // any range del could cover (start <= key). Conservative: skip only
            // when key < lo (no start can cover) — ends may be > hi.
            if user_key < lo {
                return Lookup::NotFound;
            }
            let _ = hi;
        }

        let mut point = Lookup::NotFound;
        let mut point_seq = 0u64;
        if self.bloom.may_contain(user_key) || self.has_range_tombstones() {
            if let Some((seq, look)) = self.point_in_blocks(user_key, snapshot, &mut |bi| {
                self.decode_block(bi).ok().map(Arc::new)
            }) {
                point_seq = seq;
                point = look;
            }
        }

        match point {
            Lookup::Found(v) => {
                if self.range_deleted(user_key, point_seq, snapshot) {
                    Lookup::Deleted
                } else {
                    Lookup::Found(v)
                }
            }
            Lookup::Deleted => Lookup::Deleted,
            Lookup::NotFound => {
                if self.range_deleted(user_key, 0, snapshot) {
                    Lookup::Deleted
                } else {
                    Lookup::NotFound
                }
            }
        }
    }

    pub(crate) fn has_range_tombstones(&self) -> bool {
        !self.range_tombstones.is_empty()
    }

    /// Whether a point version is covered by a range tombstone in this file.
    fn range_deleted(
        &self,
        user_key: &[u8],
        point_seq: SequenceNumber,
        snapshot: SequenceNumber,
    ) -> bool {
        for (ikey, end) in &self.range_tombstones {
            if ikey.sequence > snapshot {
                continue;
            }
            if ikey.sequence > point_seq
                && user_key >= ikey.user_key.as_ref()
                && user_key < end.as_ref()
            {
                return true;
            }
        }
        false
    }

    /// Point version at `user_key` ≤ `snapshot` from data blocks (lazy or materialised).
    ///
    /// The current writer never splits a user key across blocks (see
    /// `writer_never_splits_user_key_across_blocks`). Older files might; we
    /// still load the O(1) candidate window via [`Self::blocks_for_point`]
    /// (previous block + any run whose `first_user_key` equals `user_key`)
    /// and take the newest visible version. A linear walk of the sparse
    /// index was O(blocks) per get and dominated `deps_mvcc_latest` after
    /// YCSB+deps compacted into one large L1.
    fn point_in_blocks<F>(
        &self,
        user_key: &[u8],
        snapshot: SequenceNumber,
        load: &mut F,
    ) -> Option<(SequenceNumber, Lookup)>
    where
        F: FnMut(usize) -> Option<Arc<Vec<(InternalKey, Bytes)>>>,
    {
        if !self.is_lazy() {
            let block = self.materialize_entries().ok()?;
            return Self::best_point_in_entry_slice(&block, user_key, snapshot);
        }
        if self.index.is_empty() {
            return None;
        }
        let mut best: Option<(SequenceNumber, Lookup)> = None;
        for bi in self.blocks_for_point(user_key) {
            let Some(block) = load(bi) else {
                continue;
            };
            if let Some((seq, look)) = Self::best_point_in_entry_slice(&block, user_key, snapshot) {
                if best.as_ref().is_none_or(|(s, _)| seq > *s) {
                    best = Some((seq, look));
                }
            }
        }
        best
    }

    /// Blocks that can hold versions of `user_key` (O(log N + spans)).
    ///
    /// `index[i]` covers `[first_user_key[i], first_user_key[i+1])`. If an
    /// older writer split mid-key, versions also sit in the previous block
    /// and in any following run with `first_user_key == user_key`.
    fn blocks_for_point(&self, user_key: &[u8]) -> std::ops::Range<usize> {
        if self.index.is_empty() {
            return 0..0;
        }
        let ge = self
            .index
            .partition_point(|h| h.first_user_key.as_ref() < user_key);
        let start = ge.saturating_sub(1);
        let mut end = ge;
        while end < self.index.len() && self.index[end].first_user_key.as_ref() <= user_key {
            end += 1;
        }
        start..end
    }

    /// Newest point version of `user_key` with `sequence <= snapshot` in a sorted entry slice.
    fn best_point_in_entry_slice(
        block: &[(InternalKey, Bytes)],
        user_key: &[u8],
        snapshot: SequenceNumber,
    ) -> Option<(SequenceNumber, Lookup)> {
        let mut i = block.partition_point(|(ikey, _)| ikey.user_key.as_ref() < user_key);
        let mut best: Option<(SequenceNumber, Lookup)> = None;
        while i < block.len() {
            let (ikey, value) = &block[i];
            if ikey.user_key.as_ref() != user_key {
                break;
            }
            if ikey.kind == ValueType::RangeDeletion {
                i += 1;
                continue;
            }
            if ikey.sequence <= snapshot {
                // Entries are newest-first for a user key; first hit is best in one block.
                // Still scan if we ever change order — keep max seq for safety.
                if best.as_ref().is_none_or(|(s, _)| ikey.sequence > *s) {
                    let look = match ikey.kind {
                        ValueType::Deletion => Lookup::Deleted,
                        ValueType::Value => Lookup::Found(value.clone()),
                        ValueType::RangeDeletion => Lookup::NotFound,
                    };
                    best = Some((ikey.sequence, look));
                }
            }
            i += 1;
        }
        best
    }

    /// Decode one data block by index (v2+). Verified at open; re-decode is infallible
    /// unless payload was corrupted in RAM — then returns `Err`.
    ///
    /// # Errors
    /// Corrupt block payload or invalid index.
    pub fn decode_block(&self, block_idx: usize) -> Result<Vec<(InternalKey, Bytes)>> {
        let h = self.index.get(block_idx).ok_or_else(|| {
            CoreError::Internal(format!(
                "SST block index {block_idx} out of range in {}",
                self.path.display()
            ))
        })?;
        if self.payload.is_empty() {
            return Err(CoreError::Internal(
                "decode_block on v1/eager SST without payload".into(),
            ));
        }
        SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
        decode_block_from_payload(
            &self.payload,
            h,
            self.compressed_blocks,
            self.block_crc,
            &self.path,
        )
    }

    /// Whether this file's user-key bounds can meet `[start, end)`.
    #[must_use]
    pub fn overlaps_user_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
        let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) else {
            return false;
        };
        let file_before_end = match end {
            Bound::Unbounded => true,
            Bound::Included(e) => lo <= e,
            Bound::Excluded(e) => lo < e,
        };
        let file_after_start = match start {
            Bound::Unbounded => true,
            Bound::Included(s) => hi >= s,
            Bound::Excluded(s) => hi > s,
        };
        file_before_end && file_after_start
    }

    /// Point keys in `[start, end)`, one SST block at a time (RFC-0033 P0.3).
    ///
    /// `load` decodes block `i` (caller may hit [`crate::cache::BlockCache`]).
    /// Range tombstones are **not** yielded — collect them separately so a
    /// covering delete whose start sits outside the bound still applies.
    /// `want_values` is false for key-only / count (no value clone).
    pub fn iter_user_range<'a>(
        &'a self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        snapshot: SequenceNumber,
        want_values: bool,
        load: Box<dyn FnMut(usize) -> Option<Arc<Vec<(InternalKey, Bytes)>>> + 'a>,
    ) -> SstRangeIter<'a> {
        let start_b = match start {
            Bound::Unbounded => Bound::Unbounded,
            Bound::Included(s) => Bound::Included(Bytes::copy_from_slice(s)),
            Bound::Excluded(s) => Bound::Excluded(Bytes::copy_from_slice(s)),
        };
        let end_b = match end {
            Bound::Unbounded => Bound::Unbounded,
            Bound::Included(s) => Bound::Included(Bytes::copy_from_slice(s)),
            Bound::Excluded(s) => Bound::Excluded(Bytes::copy_from_slice(s)),
        };
        if !self.overlaps_user_range(start, end) {
            return SstRangeIter {
                current: None,
                idx: 0,
                blocks: Vec::new().into_iter(),
                load,
                start: start_b,
                end: end_b,
                snapshot,
                skip_user: None,
                want_values,
            };
        }
        if self.is_lazy() {
            SstRangeIter {
                current: None,
                idx: 0,
                blocks: self.blocks_overlapping_range(start, end).into_iter(),
                load,
                start: start_b,
                end: end_b,
                snapshot,
                skip_user: None,
                want_values,
            }
        } else {
            let leftover: Vec<_> = self
                .entries_cloned()
                .into_iter()
                .filter(|(k, _)| {
                    k.kind != ValueType::RangeDeletion
                        && user_key_in_range(k.user_key.as_ref(), start, end)
                })
                .collect();
            SstRangeIter {
                current: Some(Arc::new(leftover)),
                idx: 0,
                blocks: Vec::new().into_iter(),
                load,
                start: start_b,
                end: end_b,
                snapshot,
                skip_user: None,
                want_values,
            }
        }
    }

    /// Materialize all entries (cached). Used by compaction and full scans.
    ///
    /// # Errors
    /// Block decode failure.
    pub fn materialize_entries(&self) -> Result<Vec<(InternalKey, Bytes)>> {
        {
            let g = self.entries.lock();
            if let Some(ref e) = *g {
                return Ok(e.clone());
            }
        }
        let all = if self.index.is_empty() {
            // v1 should already have cache filled at open.
            return Err(CoreError::Internal(format!(
                "SST {} has no entries cache and no index",
                self.path.display()
            )));
        } else {
            let mut out = Vec::with_capacity(self.num_entries);
            for i in 0..self.index.len() {
                out.extend(self.decode_block(i)?);
            }
            if out.len() != self.num_entries {
                return Err(CoreError::Internal(format!(
                    "SST entry count mismatch on materialize: header {}, got {} in {}",
                    self.num_entries,
                    out.len(),
                    self.path.display()
                )));
            }
            out
        };
        let mut g = self.entries.lock();
        if g.is_none() {
            *g = Some(all.clone());
        }
        Ok(g.as_ref().map_or(all, Clone::clone))
    }

    /// Which index block would contain `user_key` (for tests / future lazy load).
    #[must_use]
    pub fn block_for_user_key(&self, user_key: &[u8]) -> Option<usize> {
        if self.index.is_empty() {
            return None;
        }
        // Last block whose first_user_key <= user_key (same as the linear scan).
        let gt = self
            .index
            .partition_point(|h| h.first_user_key.as_ref() <= user_key);
        Some(gt.saturating_sub(1))
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
            match super::scan_kernel::sst_crc_fate(stored, computed, buf.len()) {
                super::scan_kernel::SstCrcFate::StripTrailer => head,
                super::scan_kernel::SstCrcFate::WholeBuffer => buf,
                super::scan_kernel::SstCrcFate::Reject => {
                    return Err(CoreError::Internal(format!(
                        "SST file CRC mismatch in {} (stored {stored:#010x}, computed {computed:#010x})",
                        path.display()
                    )));
                }
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
            SST_VERSION_V2 => Self::decode_v2_or_v3(path, payload, &mut c, false, false, false),
            SST_VERSION_V3 => Self::decode_v2_or_v3(path, payload, &mut c, true, false, false),
            SST_VERSION_V4 => Self::decode_v2_or_v3(path, payload, &mut c, true, true, false),
            SST_VERSION => Self::decode_v2_or_v3(path, payload, &mut c, true, true, true),
            other => Err(CoreError::Internal(format!(
                "unsupported SST version {other} in {}",
                path.display()
            ))),
        }
    }

    fn decode_v1(path: &Path, file_len: usize, c: &mut Cursor<'_>) -> Result<Self> {
        let n = usize::try_from(c.read_u64()?)
            .map_err(|_| CoreError::Internal("SST entry count does not fit usize".into()))?;
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
        Ok(Self::from_eager_entries(
            path.to_path_buf(),
            entries,
            max_sequence,
            Vec::new(),
            BloomFilter::always_true(),
            Arc::from([]),
            false,
            false,
        ))
    }

    #[allow(clippy::too_many_lines)] // single open/decode path; split would obscure format steps
    fn decode_v2_or_v3(
        path: &Path,
        buf: &[u8],
        c: &mut Cursor<'_>,
        expect_bloom: bool,
        compressed_blocks: bool,
        block_crc: bool,
    ) -> Result<Self> {
        let n = usize::try_from(c.read_u64()?)
            .map_err(|_| CoreError::Internal("SST entry count does not fit usize".into()))?;
        check_sst_entry_count(n, buf.len(), path)?;
        let max_sequence = c.read_u64()?;
        let num_blocks = c.read_u32()? as usize;
        check_sst_block_count(num_blocks, buf.len(), path)?;
        let data_len = usize::try_from(c.read_u64()?)
            .map_err(|_| CoreError::Internal("SST data_len overflow".into()))?;
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

        let bloom = if expect_bloom {
            let rest = &buf[data_end + (ic.pos)..];
            BloomFilter::decode(rest)
                .map_err(|e| CoreError::Internal(format!("SST bloom in {}: {e}", path.display())))?
        } else {
            if !ic.is_empty() {
                return Err(CoreError::Internal(format!(
                    "trailing index bytes in SST {}",
                    path.display()
                )));
            }
            BloomFilter::always_true()
        };

        // Verify blocks + collect range tombstones and key bounds without retaining
        // every point entry (lazy steady-state memory).
        let mut range_tombstones = Vec::new();
        let mut smallest_user_key: Option<Bytes> = None;
        let mut largest_user_key: Option<Bytes> = None;
        let mut decoded_n = 0usize;
        let mut last_ikey: Option<InternalKey> = None;
        for h in &index {
            let block = decode_block_from_payload(buf, h, compressed_blocks, block_crc, path)?;
            for (ikey, value) in block {
                if let Some(ref prev) = last_ikey {
                    if prev > &ikey {
                        return Err(CoreError::Internal(format!(
                            "SST entries not sorted in {}",
                            path.display()
                        )));
                    }
                }
                last_ikey = Some(ikey.clone());
                decoded_n += 1;
                let uk = ikey.user_key.clone();
                if smallest_user_key.is_none() {
                    smallest_user_key = Some(uk.clone());
                }
                largest_user_key = Some(uk);
                if ikey.kind == ValueType::RangeDeletion {
                    range_tombstones.push((ikey, value));
                }
            }
        }

        if decoded_n != n {
            return Err(CoreError::Internal(format!(
                "SST entry count mismatch: header {n}, decoded {decoded_n}"
            )));
        }

        // Rebuild bloom for v2 files that lack an on-disk filter (needs all keys).
        let bloom = if bloom.is_active() {
            bloom
        } else {
            // One full pass for bloom rebuild only (v2 legacy).
            let mut all = Vec::with_capacity(n);
            for h in &index {
                all.extend(decode_block_from_payload(
                    buf,
                    h,
                    compressed_blocks,
                    block_crc,
                    path,
                )?);
            }
            rebuild_bloom(&all)
        };

        let payload: Arc<[u8]> = Arc::from(buf.to_vec().into_boxed_slice());
        let cf = crate::cf_kernel::infer_sst_cf(
            smallest_user_key.as_deref(),
            largest_user_key.as_deref(),
        );
        Ok(Self {
            path: path.to_path_buf(),
            payload,
            compressed_blocks,
            block_crc,
            // Lazy: do not retain full entry vec after open verification.
            entries: Arc::new(Mutex::new(None)),
            range_tombstones,
            num_entries: n,
            max_sequence,
            index,
            bloom,
            smallest_user_key,
            largest_user_key,
            cf,
        })
    }

    fn from_eager_entries(
        path: PathBuf,
        entries: Vec<(InternalKey, Bytes)>,
        max_sequence: SequenceNumber,
        index: Vec<BlockHandle>,
        bloom: BloomFilter,
        payload: Arc<[u8]>,
        compressed_blocks: bool,
        block_crc: bool,
    ) -> Self {
        let (smallest_user_key, largest_user_key) = user_key_bounds(&entries);
        let range_tombstones: Vec<_> = entries
            .iter()
            .filter(|(k, _)| k.kind == ValueType::RangeDeletion)
            .cloned()
            .collect();
        let num_entries = entries.len();
        let cf = crate::cf_kernel::infer_sst_cf(
            smallest_user_key.as_deref(),
            largest_user_key.as_deref(),
        );
        Self {
            path,
            payload,
            compressed_blocks,
            block_crc,
            entries: Arc::new(Mutex::new(Some(entries))),
            range_tombstones,
            num_entries,
            max_sequence,
            index,
            bloom,
            smallest_user_key,
            largest_user_key,
            cf,
        }
    }

    /// All internal versions in sorted order (materializes lazy tables).
    ///
    /// # Panics
    /// Does not panic; corrupt re-decode yields empty iterator after logging via empty vec.
    pub fn iter_internal(&self) -> impl Iterator<Item = (InternalKey, Bytes)> + '_ {
        self.entries_cloned().into_iter()
    }

    /// Internal versions one block at a time (RFC-0037 compact). Does **not**
    /// fill the materialize cache on a lazy table.
    #[must_use]
    pub fn iter_internal_streaming(&self) -> SstInternalStream<'_> {
        SstInternalStream {
            table: self,
            block_i: 0,
            block: None,
            entry_i: 0,
            failed: false,
        }
    }

    /// Clone all entries (for compaction merge). Materializes lazy tables.
    #[must_use]
    pub fn entries_cloned(&self) -> Vec<(InternalKey, Bytes)> {
        self.materialize_entries().unwrap_or_default()
    }

    /// Clone only internal entries whose user key falls in `[start, end)`.
    ///
    /// Lazy tables decode **only overlapping index blocks** (plus all range
    /// tombstones, which may start outside the bound but cover keys inside).
    #[must_use]
    pub fn entries_in_user_range(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<(InternalKey, Bytes)> {
        self.entries_in_user_range_capped(start, end, None)
    }

    /// Like [`entries_in_user_range`] but stop after `max_user_keys` distinct
    /// user keys (RFC-0033: `limit` cuts collection, not only emit).
    #[must_use]
    pub fn entries_in_user_range_capped(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        max_user_keys: Option<usize>,
    ) -> Vec<(InternalKey, Bytes)> {
        // F167: point bounds alone would skip a file whose range tombstone spans
        // into the window (the tombstone end key lives in the value and never
        // extends `largest_user_key`). Ask the kernel; `range_tombstones` are
        // already resident (open-time collection).
        let tomb_pairs: Vec<(&[u8], &[u8])> = self
            .range_tombstones
            .iter()
            .map(|(k, end)| (k.user_key.as_ref(), end.as_ref()))
            .collect();
        if !super::scan_kernel::scan_reads_file(
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
            &tomb_pairs,
            start,
            end,
        ) {
            return Vec::new();
        }

        if self.is_lazy() {
            let mut out = Vec::new();
            // Always include range tombstones (coverage may extend into the range).
            for (k, v) in &self.range_tombstones {
                out.push((k.clone(), v.clone()));
            }
            let mut users = 0usize;
            let mut last_user: Option<Bytes> = None;
            let mut stop = false;
            for bi in self.blocks_overlapping_range(start, end) {
                if stop {
                    break;
                }
                if let Ok(block) = self.decode_block(bi) {
                    for (k, v) in block {
                        if k.kind == ValueType::RangeDeletion {
                            continue; // already added
                        }
                        if user_key_in_range(k.user_key.as_ref(), start, end) {
                            if let Some(max) = max_user_keys {
                                if last_user.as_ref().is_none_or(|u| u != &k.user_key) {
                                    if users >= max {
                                        stop = true;
                                        break;
                                    }
                                    users += 1;
                                    last_user = Some(k.user_key.clone());
                                }
                            }
                            out.push((k, v));
                        }
                    }
                }
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            return out;
        }

        let entries = self.entries_cloned();
        entries
            .into_iter()
            .filter(|(ikey, _)| {
                ikey.kind == ValueType::RangeDeletion
                    || user_key_in_range(ikey.user_key.as_ref(), start, end)
            })
            .collect()
    }

    /// Largest user key in `[prefix, before)` visible at `snapshot` (RFC-0033).
    ///
    /// Walks overlapping blocks from the back. Newest version is taken from the
    /// already-loaded block when possible so a newer deletion in this file
    /// cannot leak. `before` is exclusive (`None` = `prefix_succ`).
    #[must_use]
    pub fn last_visible_under_prefix(
        &self,
        prefix: &[u8],
        snapshot: SequenceNumber,
        before: Option<&[u8]>,
    ) -> Option<(Bytes, Bytes)> {
        self.last_visible_under_prefix_with(prefix, snapshot, before, |bi| {
            self.decode_block(bi).ok().map(Arc::new)
        })
    }

    /// Like [`last_visible_under_prefix`] with a block loader (block cache).
    pub fn last_visible_under_prefix_with<F>(
        &self,
        prefix: &[u8],
        snapshot: SequenceNumber,
        before: Option<&[u8]>,
        mut load: F,
    ) -> Option<(Bytes, Bytes)>
    where
        F: FnMut(usize) -> Option<Arc<Vec<(InternalKey, Bytes)>>>,
    {
        let prefix_end = crate::prefix::prefix_exclusive_end(prefix);
        let end_owned: Option<Vec<u8>> = match (before, prefix_end.as_deref()) {
            (Some(b), Some(p)) if b < p => Some(b.to_vec()),
            (Some(b), None) => Some(b.to_vec()),
            (_, Some(p)) => Some(p.to_vec()),
            (_, None) => None,
        };
        let end_b = match end_owned.as_deref() {
            None => Bound::Unbounded,
            Some(e) => Bound::Excluded(e),
        };
        let start_b = Bound::Included(prefix);
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            if !prefix.is_empty() && hi < prefix {
                return None;
            }
            if let Some(e) = end_owned.as_deref() {
                if lo >= e {
                    return None;
                }
            }
        }
        let in_window = |uk: &[u8]| -> bool {
            if !prefix.is_empty() && !uk.starts_with(prefix) {
                return false;
            }
            match end_owned.as_deref() {
                Some(e) => uk < e,
                None => true,
            }
        };
        let decide =
            |uk: &Bytes, block: &[(InternalKey, Bytes)]| -> Option<Option<(Bytes, Bytes)>> {
                if !in_window(uk) {
                    return Some(None);
                }
                match Self::best_point_in_entry_slice(block, uk, snapshot) {
                    Some((seq, Lookup::Found(v))) if !self.range_deleted(uk, seq, snapshot) => {
                        Some(Some((uk.clone(), v)))
                    }
                    Some((_, Lookup::Deleted)) => Some(None),
                    _ => None, // versions may sit in another block
                }
            };
        if self.is_lazy() {
            let mut blocks = self.blocks_overlapping_range(start_b, end_b);
            blocks.sort_unstable();
            for bi in blocks.into_iter().rev() {
                let Some(block) = load(bi) else {
                    continue;
                };
                // Seek to `prefix` then walk only in-window users (typically
                // 2–3 MVCC versions). Do not reverse-scan the rest of the block.
                let start_i = block.partition_point(|(k, _)| k.user_key.as_ref() < prefix);
                let mut users: Vec<Bytes> = Vec::new();
                for (k, _) in &block[start_i..] {
                    if !in_window(&k.user_key) {
                        break;
                    }
                    if k.kind == ValueType::RangeDeletion {
                        continue;
                    }
                    if users.last().is_none_or(|u| u != &k.user_key) {
                        users.push(k.user_key.clone());
                    }
                }
                for uk in users.into_iter().rev() {
                    match decide(&uk, &block) {
                        Some(Some(hit)) => return Some(hit),
                        Some(None) => continue,
                        None => match self.point_in_blocks(&uk, snapshot, &mut load) {
                            Some((seq, Lookup::Found(v)))
                                if !self.range_deleted(&uk, seq, snapshot) =>
                            {
                                return Some((uk, v));
                            }
                            _ => continue,
                        },
                    }
                }
            }
            None
        } else {
            let entries = self.entries_cloned();
            let mut last: Option<Bytes> = None;
            let mut users = Vec::new();
            for (k, _) in entries.iter().rev() {
                if k.kind == ValueType::RangeDeletion {
                    continue;
                }
                if last.as_ref().is_some_and(|u| u == &k.user_key) {
                    continue;
                }
                last = Some(k.user_key.clone());
                users.push(k.user_key.clone());
            }
            for uk in users {
                if let Some(Some(hit)) = decide(&uk, &entries) {
                    return Some(hit);
                }
            }
            None
        }
    }

    /// Index blocks that may contain point keys in `[start, end)`.
    ///
    /// Conservative: block `i` covers keys from `first_user_key[i]` up to
    /// `first_user_key[i+1]` (exclusive), or +∞ for the last block.
    ///
    /// O(log N + hits) via `partition_point` — a linear walk of the sparse
    /// index was ~µs×blocks and dominated `deps_scan` after decode went away
    /// (RFC-0035: thousands of 4 KiB blocks in one L0).
    pub(crate) fn blocks_overlapping_range(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<usize> {
        if self.index.is_empty() {
            return Vec::new();
        }
        let start_i = match start {
            Bound::Unbounded => 0,
            Bound::Included(s) | Bound::Excluded(s) => {
                // Symmetric with `blocks_for_point`: under a mid-key user-key
                // split (an older or future writer format — the current
                // writer never splits, see `writer_never_splits_user_key_`
                // `across_blocks`), the block BEFORE the first block whose
                // `first_user_key == s` can still hold trailing versions of
                // `s`. Partition on `< s` (not `<= s`) so that previous
                // block stays in the window.
                let ge = self
                    .index
                    .partition_point(|h| h.first_user_key.as_ref() < s);
                ge.saturating_sub(1)
            }
        };
        let mut out = Vec::new();
        for i in start_i..self.index.len() {
            let block_lo = self.index[i].first_user_key.as_ref();
            let starts_before_end = match end {
                Bound::Unbounded => true,
                Bound::Included(e) => block_lo <= e,
                Bound::Excluded(e) => block_lo < e,
            };
            if !starts_before_end {
                break;
            }
            let block_hi_excl = self.index.get(i + 1).map(|n| n.first_user_key.as_ref());
            let ends_after_start = match start {
                Bound::Unbounded => true,
                // `hi == s` means the next block starts at `s`; this block
                // may hold trailing versions of `s` from a mid-key split —
                // keep it (same window rule as `blocks_for_point`).
                Bound::Included(s) | Bound::Excluded(s) => block_hi_excl.is_none_or(|hi| hi >= s),
            };
            if ends_after_start {
                out.push(i);
            }
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn overlapping_blocks_for_test(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<usize> {
        self.blocks_overlapping_range(start, end)
    }
}

/// One-block-at-a-time internal scan (RFC-0037). Lazy tables stay unmaterialised.
pub struct SstInternalStream<'a> {
    table: &'a SstTable,
    block_i: usize,
    block: Option<Vec<(InternalKey, Bytes)>>,
    entry_i: usize,
    failed: bool,
}

impl crate::merge::CompactSource for SstInternalStream<'_> {
    fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>> {
        SstInternalStream::next_entry(self)
    }
}

impl SstInternalStream<'_> {
    /// Next internal entry, or `Err` on a corrupt block.
    ///
    /// # Errors
    /// Block decode / CRC.
    pub fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>> {
        if self.failed {
            return Ok(None);
        }
        loop {
            if let Some(block) = &self.block {
                if self.entry_i < block.len() {
                    let e = block[self.entry_i].clone();
                    self.entry_i += 1;
                    return Ok(Some(e));
                }
            }
            if !self.table.is_lazy() {
                if self.block.is_some() {
                    return Ok(None);
                }
                self.block = Some(self.table.entries_cloned());
                self.entry_i = 0;
                continue;
            }
            if self.block_i >= self.table.block_count() {
                return Ok(None);
            }
            match self.table.decode_block(self.block_i) {
                Ok(decoded) => {
                    self.block_i += 1;
                    self.entry_i = 0;
                    self.block = Some(decoded);
                }
                Err(e) => {
                    self.failed = true;
                    return Err(e);
                }
            }
        }
    }
}

/// Lazy per-block SST range (RFC-0033 P0.3). Stops when the merge stops pulling.
///
/// Holds an `Arc` of the cached block — does not clone the whole block per scan.
pub struct SstRangeIter<'a> {
    current: Option<Arc<Vec<(InternalKey, Bytes)>>>,
    idx: usize,
    blocks: std::vec::IntoIter<usize>,
    load: Box<dyn FnMut(usize) -> Option<Arc<Vec<(InternalKey, Bytes)>>> + 'a>,
    start: Bound<Bytes>,
    end: Bound<Bytes>,
    snapshot: SequenceNumber,
    /// Newest visible version per user already yielded (skip older seqs).
    skip_user: Option<Bytes>,
    /// When false, yield empty values (`KeyOnly` / count).
    want_values: bool,
}

impl Iterator for SstRangeIter<'_> {
    type Item = (InternalKey, Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        // Bounds hoisted out of the entry loop. Entries are sorted by user
        // key, so the first key past `end` ends the iterator — no tail walk
        // of a block whose remaining keys all exceed the window.
        let start_b: Option<(&Bytes, bool)> = match &self.start {
            Bound::Unbounded => None,
            Bound::Included(s) => Some((s, true)),
            Bound::Excluded(s) => Some((s, false)),
        };
        let end_b: Option<(&Bytes, bool)> = match &self.end {
            Bound::Unbounded => None,
            Bound::Included(e) => Some((e, true)),
            Bound::Excluded(e) => Some((e, false)),
        };
        loop {
            if let Some(ref block) = self.current {
                while self.idx < block.len() {
                    let (k, v) = &block[self.idx];
                    self.idx += 1;
                    let uk = k.user_key.as_ref();
                    let past_end = match end_b {
                        Some((e, true)) => uk > e.as_ref(),
                        Some((e, false)) => uk >= e.as_ref(),
                        None => false,
                    };
                    if past_end {
                        // Sorted: nothing later can be in range.
                        self.current = None;
                        self.blocks = Vec::new().into_iter();
                        return None;
                    }
                    if k.kind == ValueType::RangeDeletion {
                        continue;
                    }
                    if k.sequence > self.snapshot {
                        continue;
                    }
                    if self.skip_user.as_ref().is_some_and(|u| u == &k.user_key) {
                        continue;
                    }
                    let before_start = match start_b {
                        Some((s, true)) => uk < s.as_ref(),
                        Some((s, false)) => uk <= s.as_ref(),
                        None => false,
                    };
                    if before_start {
                        continue;
                    }
                    self.skip_user = Some(k.user_key.clone());
                    let value = if self.want_values {
                        v.clone()
                    } else {
                        Bytes::new()
                    };
                    return Some((k.clone(), value));
                }
            }
            let bi = self.blocks.next()?;
            self.current = (self.load)(bi);
            self.idx = match (&self.current, &self.start) {
                (Some(block), Bound::Included(s)) => {
                    block.partition_point(|(k, _)| k.user_key.as_ref() < s.as_ref())
                }
                (Some(block), Bound::Excluded(s)) => {
                    block.partition_point(|(k, _)| k.user_key.as_ref() <= s.as_ref())
                }
                _ => 0,
            };
        }
    }
}

fn decode_block_from_payload(
    buf: &[u8],
    h: &BlockHandle,
    compressed_blocks: bool,
    block_crc: bool,
    path: &Path,
) -> Result<Vec<(InternalKey, Bytes)>> {
    let start = usize::try_from(h.offset)
        .map_err(|_| CoreError::Internal("block offset overflow".into()))?;
    let len = h.length as usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| CoreError::Internal("block length overflow".into()))?;
    if end > buf.len() {
        return Err(CoreError::Internal(format!(
            "block past EOF in {}",
            path.display()
        )));
    }
    let mut raw = &buf[start..end];
    if block_crc {
        if raw.len() < 4 {
            return Err(CoreError::Internal(format!(
                "SST block CRC truncated in {}",
                path.display()
            )));
        }
        let (body, crc_bytes) = raw.split_at(raw.len() - 4);
        let stored = u32::from_le_bytes(crc_bytes.try_into().unwrap());
        let computed = crc32c::crc32c(body);
        if !crate::sst::sst_block_crc_ok(stored, computed) {
            return Err(CoreError::Internal(format!(
                "SST block CRC mismatch in {}",
                path.display()
            )));
        }
        raw = body;
    }
    let plain: Vec<u8> = if compressed_blocks {
        lz4_flex::decompress_size_prepended(raw).map_err(|e| {
            CoreError::Internal(format!(
                "SST lz4 decompress failed in {}: {e}",
                path.display()
            ))
        })?
    } else {
        raw.to_vec()
    };
    let mut bc = Cursor::new(&plain);
    let mut entries = Vec::new();
    while !bc.is_empty() {
        entries.push(read_entry(&mut bc)?);
    }
    Ok(entries)
}

fn user_key_bounds(entries: &[(InternalKey, Bytes)]) -> (Option<Bytes>, Option<Bytes>) {
    if entries.is_empty() {
        return (None, None);
    }
    let first = entries[0].0.user_key.clone();
    let last = entries[entries.len() - 1].0.user_key.clone();
    (Some(first), Some(last))
}

fn rebuild_bloom(entries: &[(InternalKey, Bytes)]) -> BloomFilter {
    if entries.is_empty() {
        return BloomFilter::always_true();
    }
    // Distinct user keys ≈ entries (upper bound is fine for sizing).
    let mut bloom = BloomFilter::with_capacity(entries.len(), DEFAULT_BITS_PER_KEY);
    let mut last: Option<&[u8]> = None;
    for (ikey, _) in entries {
        let uk = ikey.user_key.as_ref();
        if last != Some(uk) {
            bloom.insert(uk);
            last = Some(uk);
        }
    }
    bloom
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

fn encode_entry_into(ikey: &InternalKey, value: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let ikey_len = u32::try_from(ikey.user_key.len().saturating_add(8))
        .map_err(|_| CoreError::Internal("internal key too large for SST".into()))?;
    let val_len = u32::try_from(value.len())
        .map_err(|_| CoreError::Internal("value too large for SST".into()))?;
    out.extend_from_slice(&ikey_len.to_le_bytes());
    ikey.encode_into(out);
    out.extend_from_slice(&val_len.to_le_bytes());
    out.extend_from_slice(value);
    Ok(())
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
    write_sst_on_with(env, path, mem, true)
}

/// [`write_sst_on`] with explicit file-`fdatasync`. `sync = false` is the L0
/// flush path: the WAL still covers the keys until rotate.
///
/// # Errors
/// I/O failures.
pub fn write_sst_on_with(
    env: &impl Env,
    path: impl AsRef<Path>,
    mem: &MemTable,
    sync: bool,
) -> Result<SstTable> {
    // MemTable is already InternalKey-ordered (user key asc, seq desc).
    // Do not collect+sort a 64 MiB snapshot — that was the apply tail.
    write_sst_try_sorted_opts(
        env,
        path,
        mem.iter_internal().map(|(k, v)| Ok((k.clone(), v.clone()))),
        mem.len(),
        sync,
        true,
    )
}

/// L0 flush: same as [`write_sst_on_with`] but **uncompressed** (SST v3).
///
/// Skip 64 MiB of lz4 on the apply tail; L0→L1 compact still writes v4.
///
/// # Errors
/// I/O failures.
pub fn write_l0_sst(
    env: &impl Env,
    path: impl AsRef<Path>,
    mem: &MemTable,
    sync: bool,
) -> Result<SstTable> {
    write_sst_try_sorted_opts(
        env,
        path,
        mem.iter_internal().map(|(k, v)| Ok((k.clone(), v.clone()))),
        mem.len(),
        sync,
        false,
    )
}

/// L0 flush of keys belonging to one CF family (RFC-0065 P0).
///
/// # Errors
/// I/O failures.
pub fn write_l0_sst_for_family(
    env: &impl Env,
    path: impl AsRef<Path>,
    mem: &MemTable,
    family: &str,
    sync: bool,
) -> Result<SstTable> {
    let fam = family.to_string();
    write_sst_try_sorted_opts(
        env,
        path,
        mem.iter_internal().filter_map(|(k, v)| {
            if crate::cf_kernel::key_in_cf_family(k.user_key.as_ref(), &fam) {
                Some(Ok((k.clone(), v.clone())))
            } else {
                None
            }
        }),
        mem.len(),
        sync,
        false,
    )
    .map(|t| t.with_cf(fam))
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

/// Write SST entries via `env` (fsyncs before return). Writes **v4** (lz4 blocks + bloom).
///
/// # Errors
/// I/O failures.
pub fn write_sst_entries_on(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: &[(InternalKey, Bytes)],
) -> Result<SstTable> {
    let mut sorted: Vec<(InternalKey, Bytes)> = entries.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let n = sorted.len();
    write_sst_sorted_on(env, path, sorted, n)
}

/// Write an **already InternalKey-sorted** stream (RFC-0037). One entry in
/// flight; bloom sized from `bloom_hint`. Does not clone the input set.
///
/// # Errors
/// I/O, encode, or a corrupt/oversized field.
pub fn write_sst_sorted_on(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    bloom_hint: usize,
) -> Result<SstTable> {
    write_sst_try_sorted_on(env, path, entries.into_iter().map(Ok), bloom_hint)
}

/// Like [`write_sst_try_sorted_on`] with an explicit file-`fdatasync` switch.
///
/// L0 flush passes `sync = false` and fsyncs the file only when the WAL that
/// still covers those keys is about to rotate (RFC-0041). Compact/checkpoint
/// keep `sync = true`.
///
/// # Errors
/// Source error, I/O, encode, or a corrupt/oversized field.
pub fn write_sst_try_sorted_with(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: impl IntoIterator<Item = Result<(InternalKey, Bytes)>>,
    bloom_hint: usize,
    sync: bool,
) -> Result<SstTable> {
    write_sst_try_sorted_opts(env, path, entries, bloom_hint, sync, true)
}

fn write_sst_try_sorted_opts(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: impl IntoIterator<Item = Result<(InternalKey, Bytes)>>,
    bloom_hint: usize,
    sync: bool,
    compress: bool,
) -> Result<SstTable> {
    write_sst_try_sorted_body(env, path, entries, bloom_hint, sync, compress)
}

/// Like [`write_sst_sorted_on`] but the stream may fail mid-file (k-way decode).
///
/// # Errors
/// Source error, I/O, encode, or a corrupt/oversized field.
pub fn write_sst_try_sorted_on(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: impl IntoIterator<Item = Result<(InternalKey, Bytes)>>,
    bloom_hint: usize,
) -> Result<SstTable> {
    write_sst_try_sorted_with(env, path, entries, bloom_hint, true)
}

fn write_sst_try_sorted_body(
    env: &impl Env,
    path: impl AsRef<Path>,
    entries: impl IntoIterator<Item = Result<(InternalKey, Bytes)>>,
    bloom_hint: usize,
    sync: bool,
    compress: bool,
) -> Result<SstTable> {
    let path = path.as_ref();
    let mut bloom = if bloom_hint == 0 {
        BloomFilter::always_true()
    } else {
        BloomFilter::with_capacity(bloom_hint, DEFAULT_BITS_PER_KEY)
    };

    let mut data = Vec::new();
    let mut index: Vec<BlockHandle> = Vec::new();
    let mut block_buf = Vec::new();
    let mut block_first_user: Option<Bytes> = None;
    let mut block_last_user: Option<Bytes> = None;
    let mut max_sequence = 0u64;
    let mut n_entries = 0usize;
    let mut last_bloom: Option<Bytes> = None;
    let mut enc_scratch = Vec::new();

    let flush_block = |data: &mut Vec<u8>,
                       block_buf: &mut Vec<u8>,
                       block_first_user: &mut Option<Bytes>,
                       index: &mut Vec<BlockHandle>|
     -> Result<()> {
        if block_buf.is_empty() {
            return Ok(());
        }
        let payload = if compress {
            lz4_flex::compress_prepend_size(block_buf)
        } else {
            std::mem::take(block_buf)
        };
        let offset = data.len() as u64;
        let mut on_disk = payload;
        if compress {
            // v5: CRC32C of the on-disk block (RFC-0077 P1.1).
            let crc = crc32c::crc32c(&on_disk);
            on_disk.extend_from_slice(&crc.to_le_bytes());
        }
        let length = u32::try_from(on_disk.len())
            .map_err(|_| CoreError::Internal("SST block too large".into()))?;
        let first = block_first_user
            .take()
            .ok_or_else(|| CoreError::Internal("block missing first key".into()))?;
        data.extend_from_slice(&on_disk);
        block_buf.clear();
        index.push(BlockHandle {
            offset,
            length,
            first_user_key: first,
        });
        Ok(())
    };

    // NEVER split a user key across blocks (same contract as write_sst_entries_on).
    for item in entries {
        let (ikey, value) = item?;
        max_sequence = max_sequence.max(ikey.sequence);
        n_entries = n_entries.saturating_add(1);
        let uk = ikey.user_key.as_ref();
        if last_bloom.as_ref().is_none_or(|p| p.as_ref() != uk) {
            bloom.insert(uk);
            last_bloom = Some(ikey.user_key.clone());
        }
        enc_scratch.clear();
        encode_entry_into(&ikey, &value, &mut enc_scratch)?;
        let same_user = block_last_user.as_ref().is_some_and(|u| u.as_ref() == uk);
        if !block_buf.is_empty() && block_buf.len() + enc_scratch.len() > BLOCK_TARGET && !same_user
        {
            flush_block(&mut data, &mut block_buf, &mut block_first_user, &mut index)?;
        }
        if block_buf.is_empty() {
            block_first_user = Some(ikey.user_key.clone());
        }
        block_buf.extend_from_slice(&enc_scratch);
        block_last_user = Some(ikey.user_key.clone());
    }
    flush_block(&mut data, &mut block_buf, &mut block_first_user, &mut index)?;

    // Header: magic version num_entries max_seq num_blocks data_len (fixed 40 B)
    let mut header = Vec::with_capacity(40);
    header.extend_from_slice(SST_MAGIC);
    let version = if compress {
        SST_VERSION
    } else {
        SST_VERSION_V3
    };
    header.extend_from_slice(&version.to_le_bytes());
    let n =
        u64::try_from(n_entries).map_err(|_| CoreError::Internal("too many SST entries".into()))?;
    header.extend_from_slice(&n.to_le_bytes());
    header.extend_from_slice(&max_sequence.to_le_bytes());
    let num_blocks = u32::try_from(index.len())
        .map_err(|_| CoreError::Internal("too many SST blocks".into()))?;
    header.extend_from_slice(&num_blocks.to_le_bytes());
    let data_len =
        u64::try_from(data.len()).map_err(|_| CoreError::Internal("SST data too large".into()))?;
    header.extend_from_slice(&data_len.to_le_bytes());

    let header_len = header.len() as u64;
    for h in &mut index {
        h.offset += header_len;
    }

    let mut index_bytes = Vec::new();
    for h in &index {
        index_bytes.extend_from_slice(&h.offset.to_le_bytes());
        index_bytes.extend_from_slice(&h.length.to_le_bytes());
        let kl = u32::try_from(h.first_user_key.len())
            .map_err(|_| CoreError::Internal("user key too large".into()))?;
        index_bytes.extend_from_slice(&kl.to_le_bytes());
        index_bytes.extend_from_slice(&h.first_user_key);
    }
    let bloom_bytes = bloom.encode();

    let mut file_crc = crc32c::crc32c(&header);
    file_crc = crc32c::crc32c_append(file_crc, &data);
    file_crc = crc32c::crc32c_append(file_crc, &index_bytes);
    file_crc = crc32c::crc32c_append(file_crc, &bloom_bytes);

    {
        let mut file = env.create(path)?;
        file.write_all(&header)?;
        file.write_all(&data)?;
        file.write_all(&index_bytes)?;
        file.write_all(&bloom_bytes)?;
        file.write_all(&file_crc.to_le_bytes())?;
        if sync {
            file.sync_data()?;
        }
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
        // Nanos alone can collide across parallel test threads on coarse
        // clocks; mix in a process-wide counter.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("pedradb-sst-{n}-{seq}.sst"))
    }

    /// RFC-0152 P2.2.40: production `SstTable::decode` gates the file
    /// trailer through `sst_crc_fate`. Live writer then XOR of the stored
    /// CRC trailer (payload intact) is Reject; AS-IS would StripTrailer.
    /// Direct `crc_mismatch_on_live_sst_is_not_ok` /
    /// `crc_mismatch_on_live_sst_block_is_not_ok` /
    /// `crc_mismatch_on_live_sst_db_open_is_not_ok` are not this tooth.
    #[test]
    fn sst_crc_fate_on_live_sst_is_not_ok() {
        assert_eq!(
            crate::sst::sst_crc_fate(1, 2, 100),
            crate::sst::SstCrcFate::Reject
        );
        assert_eq!(
            crate::sst::sst_crc_fate_as_is(1, 2, 100),
            crate::sst::SstCrcFate::StripTrailer,
            "AS-IS dente: mismatch still strips"
        );
        let mut mem = MemTable::new();
        mem.put(b"k".as_slice(), 1, b"sst-crc-trailer-0152".as_slice());
        let path = temp_path();
        write_sst(&path, &mem).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.len() >= crate::sst::SST_LEGACY_NO_CRC_MAX,
            "modern SST must not take the tiny-legacy path"
        );
        let n = bytes.len();
        bytes[n - 1] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let err = SstTable::open(&path).unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_file(&path);
        assert!(
            msg.contains("CRC mismatch"),
            "trailer CRC lie must not open as a table; got {err:?}"
        );
    }

    /// RFC-0077 P0: production writer+open; a flipped payload is CRC mismatch,
    /// never a table. AS-IS `sst_crc_fate` would strip the trailer.
    #[test]
    fn crc_mismatch_on_live_sst_is_not_ok() {
        assert_eq!(
            crate::sst::sst_crc_fate(1, 2, 100),
            crate::sst::SstCrcFate::Reject
        );
        assert_eq!(
            crate::sst::sst_crc_fate_as_is(1, 2, 100),
            crate::sst::SstCrcFate::StripTrailer
        );
        let mut mem = MemTable::new();
        mem.put(b"k".as_slice(), 1, b"sst-crc-payload-0077".as_slice());
        let path = temp_path();
        write_sst(&path, &mem).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.len() >= crate::sst::SST_LEGACY_NO_CRC_MAX,
            "modern SST must not take the tiny-legacy path"
        );
        let pos = bytes.len() / 2;
        assert!(pos + 4 < bytes.len(), "flip must not be the CRC trailer");
        bytes[pos] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let err = SstTable::open(&path).unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_file(&path);
        assert!(
            msg.contains("CRC mismatch"),
            "flipped SST must not open as a table; got {err:?}"
        );
    }

    /// RFC-0077 P1.1: production writer (v5) puts a CRC on each data block.
    /// After a payload flip, rewrite the *file* trailer so `sst_crc_fate`
    /// would StripTrailer; open still fails on `sst_block_crc_ok`.
    #[test]
    fn crc_mismatch_on_live_sst_block_is_not_ok() {
        assert!(!crate::sst::sst_block_crc_ok(1, 2));
        assert!(
            crate::sst::sst_block_crc_ok_as_is(1, 2),
            "AS-IS dente: ignore block mismatch"
        );
        let mut mem = MemTable::new();
        mem.put(b"k".as_slice(), 1, b"sst-block-crc-0077".as_slice());
        let path = temp_path();
        write_sst(&path, &mem).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() >= 40 + 8, "header + at least one data block");
        let ver = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(ver, SST_VERSION, "compressed writer must emit v5");
        // First data byte (header is 40 B). Not the file trailer.
        let pos = 40;
        assert!(pos + 4 < bytes.len() - 4);
        bytes[pos] ^= 0xff;
        let body_len = bytes.len() - 4;
        let file_crc = crc32c::crc32c(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(&file_crc.to_le_bytes());
        let stored = u32::from_le_bytes(bytes[body_len..].try_into().unwrap());
        assert_eq!(
            crate::sst::sst_crc_fate(stored, file_crc, bytes.len()),
            crate::sst::SstCrcFate::StripTrailer,
            "repaired file trailer must pass sst_crc_fate"
        );
        std::fs::write(&path, &bytes).unwrap();
        let err = SstTable::open(&path).unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_file(&path);
        assert!(
            msg.contains("CRC mismatch"),
            "block CRC must reject after a matching file trailer; got {err:?}"
        );
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
        assert!(table.has_bloom(), "v3 writer embeds bloom");
        assert!(
            table.is_lazy(),
            "v4 SST must retain payload for lazy block load"
        );
        // Point get must work without full materialize cache.
        assert!(table.entries.lock().is_none());
        assert_eq!(
            table.get(b"b", 10),
            Lookup::Found(Bytes::from_static(b"vb"))
        );
        assert!(
            table.entries.lock().is_none(),
            "point get must not force full materialize"
        );
        assert_eq!(table.get(b"a", 10), Lookup::Deleted);
        assert_eq!(table.get(b"a", 1), Lookup::Found(Bytes::from_static(b"va")));
        assert!(!table.key_may_match(b"zzz-absent-key-xxxxxxxx"));

        let streamed: Vec<_> = {
            let mut s = table.iter_internal_streaming();
            let mut out = Vec::new();
            while let Some(e) = s.next_entry().unwrap() {
                out.push(e);
            }
            out
        };
        assert_eq!(streamed, table.entries_cloned());
        // Streaming must not fill the materialize cache.
        drop(streamed);
        // Re-open so the cache from entries_cloned() above is gone.
        let reopened = SstTable::open(&path).unwrap();
        assert!(reopened.entries.lock().is_none());
        let mut s = reopened.iter_internal_streaming();
        let mut n = 0usize;
        while s.next_entry().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        assert!(
            reopened.entries.lock().is_none(),
            "block-at-a-time compact must not materialize the SST"
        );
        assert_eq!(
            reopened.get(b"b", 10),
            Lookup::Found(Bytes::from_static(b"vb"))
        );
        assert!(reopened.block_for_user_key(b"b").is_some());
        assert!(reopened.has_bloom());
        let _ = std::fs::remove_file(&path);
    }

    /// RFC-0037 P1.3: the range iterator must stop at the first key past
    /// `end` instead of tail-walking the block — same results as full
    /// filtering, no loads past the window's last block, fused after None.
    #[test]
    fn iter_user_range_stops_at_end_and_skips_tail_blocks() {
        let payload = vec![b'x'; 256];
        let mut entries = Vec::new();
        for i in 0..200u32 {
            let k = format!("k{i:03}");
            // Two versions per user: newest (higher seq) sorts first.
            entries.push((
                InternalKey::new(
                    Bytes::copy_from_slice(k.as_bytes()),
                    u64::from(i) * 10 + 2,
                    ValueType::Value,
                ),
                Bytes::copy_from_slice(b"new"),
            ));
            entries.push((
                InternalKey::new(
                    Bytes::copy_from_slice(k.as_bytes()),
                    u64::from(i) * 10 + 1,
                    ValueType::Value,
                ),
                Bytes::copy_from_slice(&payload),
            ));
        }
        let path = temp_path();
        let table = write_sst_entries(&path, &entries).unwrap();
        assert!(table.block_count() >= 8);

        let loads = std::cell::Cell::new(0usize);
        let collect = |start, end| {
            let mut out = Vec::new();
            let load = |bi: usize| {
                loads.set(loads.get() + 1);
                table.decode_block(bi).ok().map(std::sync::Arc::new)
            };
            let mut it = table.iter_user_range(start, end, u64::MAX, false, Box::new(load));
            while let Some((k, _)) = it.next() {
                out.push(k.user_key.to_vec());
            }
            // Fused: stays None after the window ends.
            assert!(it.next().is_none());
            out
        };

        // Excluded end: newest visible per user, no older duplicates, and
        // keys at/after the end never leak.
        let got = collect(
            Bound::Included(b"k050".as_ref()),
            Bound::Excluded(b"k053".as_ref()),
        );
        assert_eq!(got, vec![b"k050", b"k051", b"k052"]);
        let window_loads = loads.replace(0);
        assert!(
            window_loads <= 2,
            "25-user window must not tail-walk blocks: {window_loads} loads"
        );

        // Included end with multiple versions of the last key: the newest
        // version of k053 must still be yielded before termination.
        let got = collect(
            Bound::Included(b"k050".as_ref()),
            Bound::Included(b"k053".as_ref()),
        );
        assert_eq!(got, vec![b"k050", b"k051", b"k052", b"k053"]);

        // Window before every key in a later block still yields exactly the
        // prefix (start-block seek, no loads past it).
        let got = collect(
            Bound::Included(b"k000".as_ref()),
            Bound::Excluded(b"k002".as_ref()),
        );
        assert_eq!(got, vec![b"k000", b"k001"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn entries_in_user_range_prunes() {
        let mut entries = Vec::new();
        for i in 0..20u32 {
            let k = format!("k{i:02}");
            entries.push((
                InternalKey::new(
                    Bytes::copy_from_slice(k.as_bytes()),
                    u64::from(i) + 1,
                    ValueType::Value,
                ),
                Bytes::from_static(b"v"),
            ));
        }
        let path = temp_path();
        let table = write_sst_entries(&path, &entries).unwrap();
        let mid = table.entries_in_user_range(
            Bound::Included(b"k05".as_ref()),
            Bound::Excluded(b"k10".as_ref()),
        );
        assert_eq!(mid.len(), 5);
        assert_eq!(mid[0].0.user_key.as_ref(), b"k05");
        assert_eq!(mid[4].0.user_key.as_ref(), b"k09");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn overlapping_blocks_is_log_not_full_index() {
        let payload = vec![b'x'; 256];
        let mut entries = Vec::new();
        for i in 0..200u32 {
            let k = format!("k{i:03}");
            entries.push((
                InternalKey::new(
                    Bytes::copy_from_slice(k.as_bytes()),
                    u64::from(i) + 1,
                    ValueType::Value,
                ),
                Bytes::copy_from_slice(&payload),
            ));
        }
        let path = temp_path();
        let table = write_sst_entries(&path, &entries).unwrap();
        assert!(
            table.block_count() >= 8,
            "need a multi-block file, got {}",
            table.block_count()
        );
        let hit = table.overlapping_blocks_for_test(
            Bound::Included(b"k050".as_ref()),
            Bound::Excluded(b"k055".as_ref()),
        );
        assert!(
            !hit.is_empty() && hit.len() <= 3,
            "tight range must not scan the whole index: {hit:?} / {}",
            table.block_count()
        );
        let got = table.entries_in_user_range(
            Bound::Included(b"k050".as_ref()),
            Bound::Excluded(b"k055".as_ref()),
        );
        assert_eq!(got.len(), 5);
        assert_eq!(got[0].0.user_key.as_ref(), b"k050");
        let none = table.overlapping_blocks_for_test(
            Bound::Included(b"zzz".as_ref()),
            Bound::Excluded(b"zzzz".as_ref()),
        );
        assert!(
            none.is_empty() || {
                // last block is [kN, +∞); a seek past all keys may name it, but
                // it must not return the whole file.
                none.len() == 1 && none[0] + 1 == table.block_count()
            }
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Writer invariant the scan fast-path depends on: all versions of a user
    /// key land in a single block even when they exceed `BLOCK_TARGET`.
    /// If this ever breaks, `blocks_overlapping_range` (used by scan/range
    /// via `entries_in_user_range`) starts dropping the newest versions at the
    /// split key — see the comment on the spill condition.
    #[test]
    fn writer_never_splits_user_key_across_blocks() {
        // One key with versions far past BLOCK_TARGET, then a later key.
        let mut entries = Vec::new();
        for seq in (1..=12u64).rev() {
            entries.push((
                InternalKey::new(Bytes::copy_from_slice(b"k"), seq, ValueType::Value),
                Bytes::from(vec![seq as u8; 1024]),
            ));
        }
        entries.push((
            InternalKey::new(Bytes::copy_from_slice(b"z"), 13, ValueType::Value),
            Bytes::from_static(b"zv"),
        ));
        let path = temp_path();
        let table = write_sst_entries(&path, &entries).unwrap();
        assert!(
            table.block_count() >= 2,
            "later key must force a second block"
        );
        // Every k-version must be visible to a scan starting exactly at k
        // (the split-key boundary the block-overlap fast-path reasons about).
        let binding = table.entries_in_user_range(Bound::Included(&b"k"[..]), Bound::Unbounded);
        let k_seqs: Vec<u64> = binding
            .iter()
            .filter(|(ik, _)| ik.user_key.as_ref() == b"k")
            .map(|(ik, _)| ik.sequence)
            .collect();
        assert_eq!(k_seqs, (1..=12).rev().collect::<Vec<_>>());
        let _ = std::fs::remove_file(&path);
    }

    /// P2.2 (RFC-0048): `blocks_overlapping_range` must window a scan start
    /// the same way `blocks_for_point` windows a get — the previous block
    /// plus the equal-key run — so a mid-key user-key split (an older or
    /// future writer format) cannot make a scan starting exactly at the
    /// split key miss the newest versions parked in the previous block.
    #[test]
    fn blocks_overlapping_range_defends_mid_key_split_like_point() {
        let bh = |offset: u64, length: u32, first: &[u8]| BlockHandle {
            offset,
            length,
            first_user_key: Bytes::copy_from_slice(first),
        };
        // Hand-built sparse index emulating a writer that split `k` across
        // blocks: block 0 = [a@1, k@5, k@3], block 1 = [k@2, z@1].
        let index = vec![bh(0, 16, b"a"), bh(16, 16, b"k")];
        let entries = vec![
            (
                InternalKey::new(Bytes::copy_from_slice(b"a"), 1, ValueType::Value),
                Bytes::from_static(b"av"),
            ),
            (
                InternalKey::new(Bytes::copy_from_slice(b"k"), 5, ValueType::Value),
                Bytes::from_static(b"kv5"),
            ),
            (
                InternalKey::new(Bytes::copy_from_slice(b"k"), 3, ValueType::Value),
                Bytes::from_static(b"kv3"),
            ),
            (
                InternalKey::new(Bytes::copy_from_slice(b"k"), 2, ValueType::Value),
                Bytes::from_static(b"kv2"),
            ),
            (
                InternalKey::new(Bytes::copy_from_slice(b"z"), 1, ValueType::Value),
                Bytes::from_static(b"zv"),
            ),
        ];
        let table = SstTable {
            path: PathBuf::from("/tmp/hand-made-mid-key-split.sst"),
            payload: Arc::from(vec![]),
            compressed_blocks: false,
            block_crc: false,
            entries: Arc::new(Mutex::new(Some(entries))),
            range_tombstones: Vec::new(),
            num_entries: 5,
            max_sequence: 5,
            index,
            bloom: BloomFilter::always_true(),
            smallest_user_key: Some(Bytes::copy_from_slice(b"a")),
            largest_user_key: Some(Bytes::copy_from_slice(b"z")),
            cf: String::new(),
        };

        // Reference: the point path already defends the split.
        assert_eq!(table.blocks_for_point(b"k"), 0..2);

        // Prova: a scan starting exactly at the split key must include
        // block 0 (k@5, k@3) — AS-IS windowed on `first <= k` and returned
        // only [1], silently dropping the newest versions of `k`.
        let got = table.blocks_overlapping_range(Bound::Included(&b"k"[..]), Bound::Unbounded);
        assert_eq!(
            got,
            vec![0, 1],
            "mid-key split: scan at `k` must see block 0 like the point path"
        );

        // Controls — identical window when no split is involved.
        assert_eq!(
            table.blocks_overlapping_range(Bound::Included(&b"m"[..]), Bound::Unbounded),
            vec![1],
            "m sits inside block 1 only"
        );
        assert_eq!(
            table.blocks_overlapping_range(Bound::Included(&b"b"[..]), Bound::Unbounded),
            vec![0, 1],
            "b..∞ spans both blocks (k and z are past b)"
        );
        assert_eq!(
            table.blocks_overlapping_range(Bound::Unbounded, Bound::Included(&b"k"[..])),
            vec![0, 1],
            "range ending at k covers both k blocks"
        );
    }

    /// L0 flush streams the BTree in InternalKey order — no collect+sort.
    /// Reverse insert order must still round-trip every key (RFC-0041).
    #[test]
    fn memtable_stream_flush_roundtrip_unsorted_inserts() {
        let mut mem = MemTable::new();
        for i in (0..80u32).rev() {
            let k = format!("k{i:03}");
            mem.put(
                Bytes::copy_from_slice(k.as_bytes()),
                u64::from(i) + 1,
                Bytes::from(vec![i as u8; 64]),
            );
        }
        mem.delete(Bytes::from_static(b"k010"), 200);
        let path = temp_path();
        let table = write_sst_on_with(&StdEnv, &path, &mem, false).unwrap();
        assert_eq!(table.len(), mem.len());
        for i in 0..80u32 {
            let k = format!("k{i:03}");
            let got = table.get(k.as_bytes(), 10_000);
            if i == 10 {
                assert_eq!(got, Lookup::Deleted, "k010 tombstone");
            } else {
                assert_eq!(got, Lookup::Found(Bytes::from(vec![i as u8; 64])), "{k}");
            }
        }
        // Re-open must accept the incremental CRC trailer.
        let re = SstTable::open(&path).unwrap();
        assert_eq!(
            re.get(b"k000", 10_000),
            Lookup::Found(Bytes::from(vec![0; 64]))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn uncompressed_l0_roundtrip() {
        let mut mem = MemTable::new();
        mem.put(Bytes::from_static(b"a"), 1, Bytes::from_static(b"va"));
        mem.put(Bytes::from_static(b"b"), 2, Bytes::from_static(b"vb"));
        let path = temp_path();
        let table = write_l0_sst(&StdEnv, &path, &mem, false).unwrap();
        assert_eq!(
            table.get(b"a", 10),
            Lookup::Found(Bytes::from_static(b"va"))
        );
        let re = SstTable::open(&path).unwrap();
        assert_eq!(re.get(b"b", 10), Lookup::Found(Bytes::from_static(b"vb")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn multi_block_index_points_into_key_space() {
        let mut entries = Vec::new();
        for i in 0..200u32 {
            let k = format!("k{i:04}");
            entries.push((
                InternalKey::new(
                    Bytes::copy_from_slice(k.as_bytes()),
                    u64::from(i) + 1,
                    ValueType::Value,
                ),
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

    /// A wide L1 (YCSB+deps compacted) used to decode O(blocks) per point get.
    /// The sparse-index seek must stay O(log N + spans), not a linear walk.
    #[test]
    fn point_get_decodes_o1_blocks_on_wide_sst() {
        let mut entries = Vec::new();
        for i in 0..400u32 {
            let k = format!("k{i:04}");
            entries.push((
                InternalKey::new(Bytes::copy_from_slice(k.as_bytes()), 1, ValueType::Value),
                Bytes::from(vec![0u8; 1024]),
            ));
        }
        let path = temp_path();
        let table = write_sst_entries(&path, &entries).unwrap();
        assert!(
            table.block_count() > 50,
            "need a wide index, got {}",
            table.block_count()
        );
        for key in [b"k0000".as_slice(), b"k0200", b"k0399"] {
            reset_sst_blocks_decoded();
            assert_eq!(
                table.get(key, 10),
                Lookup::Found(Bytes::from(vec![0u8; 1024])),
                "{}",
                String::from_utf8_lossy(key)
            );
            let n = sst_blocks_decoded();
            assert!(
                n <= 2,
                "point get of {} decoded {n} blocks (index {}), want ≤2",
                String::from_utf8_lossy(key),
                table.block_count()
            );
        }
        let _ = std::fs::remove_file(&path);
    }
}
