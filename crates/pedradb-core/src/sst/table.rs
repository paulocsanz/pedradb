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
//! Trailing file CRC32C covers the full body (v2–v5). v6: header+tail only.
//!
//! # On-disk v4
//! v3 + lz4-compressed data blocks (no per-block CRC).
//!
//! # On-disk v5 (compressed writer default, RFC-0077 P1.1)
//! v4 + 4-byte CRC32C after each data block (`sst_block_crc_ok`).
//!
//! # On-disk v6 (RFC-0159 bulk)
//! v3 uncompressed blocks + per-block CRC32C (no lz4). Evicted point
//! gets are one 4 KiB `read_range`. File CRC covers **header + index/bloom
//! tail only** — data is fail-closed per block (a whole-file CRC of the
//! 5.75 GiB body was 1–2 s of the 25M hydrate and duplicated the block
//! CRCs). v1–v5 trailers still cover the full body.
//!
//! v1–v6 files are still readable.
//!
//! # Lazy blocks (RFC-0014 P1.2)
//!
//! v2+ tables keep the CRC-stripped payload and sparse index in memory and
//! **decode data blocks on demand** for point gets and bounded ranges. Full
//! entry materialization happens only for compaction / whole-table clone.
//! Range tombstones are extracted once at open so point gets stay correct
//! without scanning every block for deletes.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::bloom::{BloomFilter, DEFAULT_BITS_PER_KEY};
use crate::cache::{BlockCache, PayloadKit, PayloadSlot};
use crate::env::{Env, EnvFile, StdEnv};
use crate::error::{CoreError, Result};
use crate::key::{pack_sequence_and_type, InternalKey, SequenceNumber, ValueType};
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
/// Uncompressed + bloom + per-block CRC32C (RFC-0159 bulk). Evicted
/// point gets read one 4 KiB block instead of the whole v3 file.
pub const SST_VERSION_V6: u32 = 6;
/// Default target encoded size per data block (pre-compression). Reads are
/// self-describing per block (the index carries real offsets), so tables with
/// different targets coexist; `PEDRA_BLOCK_TARGET` overrides new writes.
pub const BLOCK_TARGET: usize = 4_096;
/// Bulk-run SST block target: same 4 KiB as Rocks so evicted get_hit
/// is one block, not a 256 KiB (or whole-file v3) read.
pub const BULK_BLOCK_TARGET: usize = BLOCK_TARGET;

/// Absolute cap on a decompressed SST block. Matches [`block_target`]'s
/// upper clamp (256 KiB). PEDRA-001: a forged lz4 size prefix of tens of
/// MiB used to allocate on GET (`plain_scratch.resize`) / open
/// (`decompress_size_prepended`) after the block CRC was rewritten; the
/// engine never writes a block larger than this.
const LZ4_MAX_PLAIN_BLOCK: usize = 256 * 1024;
/// Expansion ratio cap (PEDRA-001). A 200-byte frame claiming the full
/// 256 KiB cap is still a bomb; the absolute cap alone would admit it.
/// 4 KiB of zeros compresses well under 256× (~20 B frame).
const LZ4_MAX_EXPANSION: usize = 256;

/// `true` iff `plain` is a size we could have written for `compressed_len`.
fn lz4_plain_size_ok(plain: usize, compressed_len: usize) -> bool {
    if plain == 0 || plain > LZ4_MAX_PLAIN_BLOCK {
        return false;
    }
    // Default-target blocks (≤ 4 KiB) may be all zeros — lz4 of 4 KiB
    // zeros is ~20 B, ratio ~200×, still under 256×. Larger blocks are
    // bounded by the absolute cap.
    if plain <= BLOCK_TARGET {
        return plain <= compressed_len.saturating_mul(LZ4_MAX_EXPANSION);
    }
    true
}

fn lz4_size_rejected(path: &Path, plain: usize, compressed_len: usize) -> CoreError {
    CoreError::Internal(format!(
        "SST lz4 uncompressed size {plain} exceeds 256x compressed length ({compressed_len}) in {}",
        path.display()
    ))
}

/// Effective block target for new writes: `PEDRA_BLOCK_TARGET` (bytes,
/// clamped 1 KiB–256 KiB) when set, else [`BLOCK_TARGET`].
#[must_use]
pub fn block_target() -> usize {
    static OVERRIDE: OnceLock<usize> = OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        std::env::var("PEDRA_BLOCK_TARGET")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map_or(BLOCK_TARGET, |b| b.clamp(1_024, 262_144))
    })
}

/// Absolute ceiling on SST entry count (defense-in-depth vs corrupt headers).
///
/// A real file cannot hold more entries than its byte length allows; we also
/// reject absurd counts before `Vec::with_capacity` (F2: bit-flip of
/// `num_entries` caused multi-EiB allocation attempts).
pub const MAX_SST_ENTRIES: usize = 64 * 1024 * 1024;

thread_local! {
    static SST_BLOCKS_DECODED: Cell<usize> = const { Cell::new(0) };
}

thread_local! {
    /// Probes served without a CRC re-run (verified-residency marks).
    static SST_BLOCK_CRC_SKIPPED: Cell<usize> = const { Cell::new(0) };
}

/// Verified raw 4 KiB blocks (CRC trailer included) for evicted files.
/// get_hit of one key does not need this; lookup_100's 100 keys do —
/// 25M v63 get_loop was 0.88× because each of 100 probes re-pread+CRC
/// while Rocks reused its block cache. 512 × 4 KiB = 2 MiB TLS.
const RAW_BLOCK_CACHE_CAP: usize = 512;

#[derive(Default)]
struct RawBlockCache {
    map: HashMap<(u64, u64), Arc<[u8]>>,
    order: VecDeque<(u64, u64)>,
}

impl RawBlockCache {
    fn get(&mut self, key: &(u64, u64)) -> Option<Arc<[u8]>> {
        self.map.get(key).cloned()
    }

    fn insert(&mut self, key: (u64, u64), img: Arc<[u8]>) {
        if self.map.contains_key(&key) {
            return;
        }
        while self.map.len() >= RAW_BLOCK_CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
        self.order.push_back(key);
        self.map.insert(key, img);
    }
}

thread_local! {
    static RAW_BLOCKS: RefCell<RawBlockCache> = RefCell::new(RawBlockCache::default());
}

/// RFC-0160 P0.2: split point-get wall into pread / block CRC / block walk.
/// `PEDRA_GET_STAGES=1` arms it for a guest capture; tests call
/// [`force_get_stages`].
static GET_STAGES: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static GET_PREAD_NS: Cell<u64> = const { Cell::new(0) };
    static GET_CRC_NS: Cell<u64> = const { Cell::new(0) };
    static GET_WALK_NS: Cell<u64> = const { Cell::new(0) };
}

fn get_stages_enabled() -> bool {
    match GET_STAGES.load(AtomicOrdering::Relaxed) {
        2 => true,
        1 => false,
        _ => {
            let on = std::env::var_os("PEDRA_GET_STAGES").is_some();
            GET_STAGES.store(if on { 2 } else { 1 }, AtomicOrdering::Relaxed);
            on
        }
    }
}

/// Test/guest knob: pin get-stage clocks on or off (`PEDRA_GET_STAGES`).
pub fn force_get_stages(on: bool) {
    GET_STAGES.store(if on { 2 } else { 1 }, AtomicOrdering::Relaxed);
}

/// Zero the thread-local get-stage clocks.
pub fn reset_get_stages() {
    GET_PREAD_NS.with(|c| c.set(0));
    GET_CRC_NS.with(|c| c.set(0));
    GET_WALK_NS.with(|c| c.set(0));
}

/// Take pread / CRC / walk nanoseconds since the last reset (RFC-0160 P0.2).
#[must_use]
pub fn take_get_stages() -> (u64, u64, u64) {
    (
        GET_PREAD_NS.with(Cell::take),
        GET_CRC_NS.with(Cell::take),
        GET_WALK_NS.with(Cell::take),
    )
}

fn add_stage(cell: &Cell<u64>, ns: u128) {
    cell.set(cell.get().saturating_add(ns as u64));
}

/// Reset the thread-local CRC-skip counter (verified-residency tests).
pub fn reset_sst_block_crc_skipped() {
    SST_BLOCK_CRC_SKIPPED.with(|c| c.set(0));
}

/// CRC re-runs skipped via verified-residency marks since the last reset.
#[must_use]
pub fn sst_block_crc_skipped() -> usize {
    SST_BLOCK_CRC_SKIPPED.with(Cell::get)
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

fn check_sst_entry_count(
    n: usize,
    file_len: usize,
    compressed_blocks: bool,
    path: &Path,
) -> Result<()> {
    if n > MAX_SST_ENTRIES {
        return Err(CoreError::Internal(format!(
            "SST entry count {n} exceeds MAX_SST_ENTRIES ({MAX_SST_ENTRIES}) in {}",
            path.display()
        )));
    }
    // Even a 1-byte entry cannot exceed the file; tighter: min encoded size.
    // Block compression breaks the per-byte floor — a run of identical
    // values packs thousands of entries into a few KiB — so only the hard
    // MAX_SST_ENTRIES cap applies to compressed files (their count is
    // verified by decoding the blocks either way).
    if !compressed_blocks {
        let max_by_size = file_len / MIN_ENCODED_ENTRY + 1;
        if n > max_by_size {
            return Err(CoreError::Internal(format!(
                "SST entry count {n} impossible for file size {file_len} in {}",
                path.display()
            )));
        }
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
    /// Big-endian u64 window of `first_user_key[key_cp..key_cp+8]`
    /// (zero-padded), derived with the table's `key_cp`. Comparing `p8`
    /// before the full memcmp preserves byte order exactly and starts at
    /// the first byte where index keys actually differ — route-fold keys
    /// share a 10-byte prefix, so a fixed offset-0 window would carry no
    /// entropy. Filled by `derive_index_accel`; 0 until then.
    ///
    /// v65 widened this to u128 (16 B). Guest 25M get_hit dropped
    /// 1.15× → 0.78–0.91×; prefix held. Reverted: 25M point gets are
    /// pread-bound, and the extra window did not pay.
    p8: u64,
}

/// Cached full entry materialization for an SST (shared across clones).
type EntriesCache = Arc<Mutex<Option<Vec<(InternalKey, Bytes)>>>>;

/// Reusable buffers for [`SstTable::point_at_seeking`]: one per lookup loop
/// amortizes the evicted-block read and the lz4 decompress to zero
/// steady-state allocations.
#[derive(Debug, Default)]
pub struct PointSeekScratch {
    /// Raw block image (CRC included) for blocks served from file.
    raw: Vec<u8>,
    /// Decompressed block image.
    plain: Vec<u8>,
}

thread_local! {
    static POINT_SEEK_SCRATCH: RefCell<Option<PointSeekScratch>> = const { RefCell::new(None) };
}

/// Take the thread-local seek scratch (or empty). Pair with
/// [`put_tls_point_seek_scratch`] so lookup_100's 100 gets reuse one
/// 4 KiB buffer instead of alloc/free per key.
pub(crate) fn take_tls_point_seek_scratch() -> PointSeekScratch {
    POINT_SEEK_SCRATCH.with(|c| c.borrow_mut().take().unwrap_or_default())
}

/// Return a seek scratch to the thread-local slot.
pub(crate) fn put_tls_point_seek_scratch(s: PointSeekScratch) {
    POINT_SEEK_SCRATCH.with(|c| *c.borrow_mut() = Some(s));
}

/// In-memory view of one SST file.
///
/// For v2+ (`payload` non-empty): data blocks are decoded lazily. For v1:
/// all entries are eager in the materialization cache.
#[derive(Debug, Clone)]
pub struct SstTable {
    path: PathBuf,
    /// CRC-stripped file body for lazy block decode, behind an evictable
    /// shared slot (RFC-0042 v18). Empty slot = evicted, blocks served from
    /// file via `kit`. Empty at construction for legacy v1.
    payload: Arc<PayloadSlot>,
    /// Byte length of the (possibly evicted) file body; 0 for v1/eager.
    payload_len: usize,
    /// RFC-0159 streaming bulk writer contract: this table was created with
    /// its body deliberately left on disk (empty payload slot, no kit yet
    /// — hydrate attaches one at install). Only such a table may reload its
    /// payload from `path` without a kit; any other kit-less table with an
    /// empty payload slot fails loud instead of silently re-reading a path
    /// the owner never vouched for (RFC-0042 v18 fail-closed contract).
    body_on_disk: bool,
    /// Whether data blocks are lz4 (SST v4+).
    compressed_blocks: bool,
    /// Whether each data block carries a trailing CRC32C (SST v5).
    block_crc: bool,
    /// Cached full decode (`None` until first materialize for lazy tables).
    entries: EntriesCache,
    /// Owning `Db`'s file source + payload pool (RFC-0042 v18), shared with
    /// every clone of this handle. `None` on a free-standing table — its
    /// payload then never evicts and never reloads.
    kit: Arc<RwLock<Option<PayloadKit>>>,
    /// Range tombstones extracted at open (lazy tables) or from entries (v1).
    range_tombstones: Vec<(InternalKey, Bytes)>,
    /// Header entry count (or materialized length for v1).
    num_entries: usize,
    max_sequence: SequenceNumber,
    /// Sparse index (v2+); empty for v1.
    /// Sparse block index, shared across clones: every installed table is
    /// held twice (`ssts` + `table_cache`), and a `Vec` clone doubled the
    /// 100M index (~350 MB) plus one `Bytes` promote per handle.
    index: Arc<[BlockHandle]>,
    /// Longest prefix shared by every `index` key — the offset the `p8`
    /// windows start at. Derived with the index (`derive_index_accel`).
    key_cp: usize,
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

    /// Whether this table decodes blocks on demand (v2+). Residency of the
    /// payload does not change laziness: an evicted body (RFC-0042 v18) is
    /// served from file block-by-block instead.
    #[must_use]
    pub fn is_lazy(&self) -> bool {
        !self.index.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn payload_bytes(&self) -> usize {
        self.payload_len
    }

    #[cfg(test)]
    pub(crate) fn payload_resident(&self) -> bool {
        !self.payload.read().img.is_empty()
    }

    /// Weak handle to the shared payload slot (pool registration).
    pub(crate) fn payload_slot_weak(&self) -> std::sync::Weak<PayloadSlot> {
        Arc::downgrade(&self.payload)
    }

    /// Drop the resident file body. Reload goes through `kit` (hydrate
    /// does not read the chunk it just wrote; keeping every L3 image
    /// resident is the 3.9 GiB guest OOM at 100M).
    pub(crate) fn release_resident(&self) {
        *self.payload.write() = crate::cache::ResidentBody::empty();
        if let Some(kit) = self.kit.read().clone() {
            kit.pool.register(&self.path, self.payload_slot_weak(), 0);
        }
    }

    /// Attach the owning `Db`'s file source + payload pool (RFC-0042 v18)
    /// and register the payload. Idempotent; a v1/eager table is a no-op.
    /// Empty (streaming bulk) slots are **not** registered — counting them
    /// as `payload_len` ghost-fills the 256 MiB budget so 1M get_hit never
    /// promotes and pays `pread`+CRC on every probe.
    pub(crate) fn attach_payload_kit(
        &self,
        source: &Arc<dyn crate::env::SstFileSource>,
        pool: &Arc<crate::cache::SstPayloadPool>,
    ) {
        if self.payload_len == 0 {
            return;
        }
        *self.kit.write() = Some(PayloadKit {
            source: Arc::clone(source),
            pool: Arc::clone(pool),
        });
        if !self.payload.read().img.is_empty() {
            pool.register(
                &self.path,
                self.payload_slot_weak(),
                self.payload_len as u64,
            );
        }
    }

    /// If this file is empty and the pool still has room, load the body so
    /// subsequent point seeks hit the verified-residency path (no per-get
    /// `pread`+CRC). No-op when the budget is full — evicted v6 stays one
    /// 4 KiB `read_range`. Does not run during hydrate (no gets).
    fn try_promote_payload(&self) -> Result<bool> {
        if !self.payload.read().img.is_empty() {
            return Ok(true);
        }
        let Some(kit) = self.kit.read().clone() else {
            return Ok(false);
        };
        if !kit.pool.can_admit(&self.path, self.payload_len as u64) {
            return Ok(false);
        }
        self.ensure_payload(&kit)?;
        Ok(!self.payload.read().img.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn materialize_cache_filled(&self) -> bool {
        self.entries.lock().is_some()
    }

    /// Decoded-entries cache occupancy (observability): 0 when cold. The
    /// cache is unbounded per table, so diag callers sum this across tables.
    #[must_use]
    pub fn cached_entries_count(&self) -> usize {
        self.entries.lock().as_ref().map_or(0, Vec::len)
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

    /// Point version at `user_key` ≤ `snapshot`, seeking the **encoded**
    /// block instead of decoding it into entries.
    ///
    /// Visibility is identical to [`point_at_with`]: same bounds/bloom gate,
    /// same candidate window ([`Self::blocks_for_point`]), same newest-visible
    /// merge. The difference is per-block work — CRC-verify the raw image,
    /// lz4-decompress into `scratch`, walk `ikey_len|ikey|val_len|val`
    /// comparing user-key prefixes without materialising `InternalKey`s, and
    /// copy out only the winning value. The decoded-block cache thrashed at
    /// random-key scale (insert + evict on nearly every get); the seek pays a
    /// bounded decompress instead of the cache machinery.
    ///
    /// Fail-closed: a CRC or framing fault returns `Err` — never a silent
    /// miss. The caller must fail-stop or propagate.
    ///
    /// # Errors
    /// Corrupt block payload/framing or I/O on an evicted table.
    pub fn point_at_seeking(
        &self,
        user_key: &[u8],
        snapshot: SequenceNumber,
        scratch: &mut PointSeekScratch,
    ) -> Result<Option<(SequenceNumber, Lookup)>> {
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            if user_key < lo || user_key > hi {
                return Ok(None);
            }
        }
        if !self.bloom.may_contain(user_key) {
            return Ok(None);
        }
        if !self.is_lazy() {
            let block = self.materialize_entries()?;
            return Ok(Self::best_point_in_entry_slice(&block, user_key, snapshot));
        }
        if self.index.is_empty() {
            return Ok(None);
        }
        if !self.block_crc {
            // ≤v4 carries no per-block CRC: reuse the whole-body path so the
            // file-level CRC gate re-runs before any block decodes.
            return Ok(self.point_in_blocks(user_key, snapshot, &mut |bi| {
                self.decode_block(bi).ok().map(Arc::new)
            }));
        }
        // Bulk hydrate leaves the slot empty (100M RAM). If the 256 MiB
        // pool still has room, promote now so get_hit is a slice+CRC-skip
        // (v55 1M was 0.79× vs Rocks because every probe was pread+CRC).
        if self.payload.read().img.is_empty() {
            self.try_promote_payload()?;
        }
        // v5: each block carries its own CRC, so one image suffices — a
        // resident-payload slice or a single positioned read via the kit.
        let mut best: Option<(SequenceNumber, Lookup)> = None;
        for bi in self.blocks_for_point(user_key) {
            let Some(h) = self.index.get(bi) else {
                continue;
            };
            let len = usize::try_from(h.length)
                .map_err(|_| CoreError::Internal("SST block length overflow".into()))?;
            let mut served_from_file = false;
            {
                let g = self.payload.read();
                let p: &Arc<[u8]> = &g.img;
                if p.is_empty() {
                    served_from_file = true;
                } else {
                    let start = usize::try_from(h.offset)
                        .map_err(|_| CoreError::Internal("SST block offset overflow".into()))?;
                    let Some(end) = start.checked_add(len) else {
                        return Err(CoreError::Internal("SST block length overflow".into()));
                    };
                    if end > p.len() {
                        return Err(CoreError::Internal(format!(
                            "SST block past EOF in {}",
                            self.path.display()
                        )));
                    }
                    // Verified-residency fast path (RFC-0077 fail-closed):
                    // a mark set for exactly these resident bytes skips the
                    // per-probe CRC re-run — the first probe of a residency
                    // verifies and marks under the write guard, and every
                    // payload write installs fresh, empty marks. RocksDB's
                    // cached blocks carry the same contract.
                    if g.is_verified(bi) {
                        SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
                        SST_BLOCK_CRC_SKIPPED.with(|c| c.set(c.get().saturating_add(1)));
                        let img = &p[start..end];
                        if img.len() < 4 {
                            return Err(CoreError::Internal(format!(
                                "SST block CRC truncated in {}",
                                self.path.display()
                            )));
                        }
                        if let Some(found) = seek_point_in_block_body(
                            &img[..img.len() - 4],
                            self.compressed_blocks,
                            user_key,
                            snapshot,
                            &mut scratch.plain,
                            &self.path,
                        )? {
                            if best.as_ref().is_none_or(|(s, _)| found.0 > *s) {
                                best = Some(found);
                            }
                        }
                    } else {
                        drop(g);
                        let mut w = self.payload.write();
                        let p: &Arc<[u8]> = &w.img;
                        if p.is_empty() || end > p.len() {
                            // Evicted (or re-installed shorter) between the
                            // guards: serve this block from file.
                            drop(w);
                            served_from_file = true;
                        } else {
                            SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
                            let found = seek_point_in_block_image(
                                &p[start..end],
                                self.compressed_blocks,
                                user_key,
                                snapshot,
                                &mut scratch.plain,
                                &self.path,
                            )?;
                            w.mark_verified(bi, self.index.len());
                            if let Some(found) = found {
                                if best.as_ref().is_none_or(|(s, _)| found.0 > *s) {
                                    best = Some(found);
                                }
                            }
                        }
                    }
                }
            }
            if served_from_file {
                let kit = self.kit.read().clone();
                let Some(kit) = kit else {
                    return Err(CoreError::Internal(format!(
                        "SST {} payload evicted without a file source (free-standing table)",
                        self.path.display()
                    )));
                };
                let cache_key = (crate::cache::path_id(&self.path), h.offset);
                let cached = RAW_BLOCKS.with(|c| c.borrow_mut().get(&cache_key));
                if let Some(raw) = cached {
                    SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
                    SST_BLOCK_CRC_SKIPPED.with(|c| c.set(c.get().saturating_add(1)));
                    if raw.len() >= 4 {
                        if let Some(found) = seek_point_in_block_body(
                            &raw[..raw.len() - 4],
                            self.compressed_blocks,
                            user_key,
                            snapshot,
                            &mut scratch.plain,
                            &self.path,
                        )? {
                            if best.as_ref().is_none_or(|(s, _)| found.0 > *s) {
                                best = Some(found);
                            }
                        }
                    }
                } else {
                    scratch.raw.clear();
                    scratch.raw.resize(len, 0);
                    if get_stages_enabled() {
                        let t = Instant::now();
                        kit.source
                            .read_range(&self.path, h.offset, &mut scratch.raw)
                            .map_err(CoreError::Io)?;
                        GET_PREAD_NS.with(|c| add_stage(c, t.elapsed().as_nanos()));
                    } else {
                        kit.source
                            .read_range(&self.path, h.offset, &mut scratch.raw)
                            .map_err(CoreError::Io)?;
                    }
                    SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
                    let found = seek_point_in_block_image(
                        &scratch.raw,
                        self.compressed_blocks,
                        user_key,
                        snapshot,
                        &mut scratch.plain,
                        &self.path,
                    )?;
                    RAW_BLOCKS.with(|c| {
                        c.borrow_mut()
                            .insert(cache_key, Arc::from(scratch.raw.as_slice()));
                    });
                    if let Some(found) = found {
                        if best.as_ref().is_none_or(|(s, _)| found.0 > *s) {
                            best = Some(found);
                        }
                    }
                }
            }
        }
        Ok(best)
    }

    /// Point seek through a byte-budgeted [`BlockCache`] (RFC-0160 P2.3).
    ///
    /// First load CRC-verifies via [`Self::decode_block`] and only then
    /// inserts; a fault is `Err`, never an empty cached block (silent miss).
    /// Hits skip CRC — Rocks checksum-on-read-into-cache.
    ///
    /// # Errors
    /// Corrupt block payload/framing or I/O on an evicted table.
    pub fn point_at_cached(
        &self,
        user_key: &[u8],
        snapshot: SequenceNumber,
        cache: &BlockCache,
    ) -> Result<Option<(SequenceNumber, Lookup)>> {
        if let (Some(lo), Some(hi)) = (
            self.smallest_user_key.as_deref(),
            self.largest_user_key.as_deref(),
        ) {
            if user_key < lo || user_key > hi {
                return Ok(None);
            }
        }
        if !self.bloom.may_contain(user_key) {
            return Ok(None);
        }
        if !self.is_lazy() {
            let block = self.materialize_entries()?;
            return Ok(Self::best_point_in_entry_slice(&block, user_key, snapshot));
        }
        if self.index.is_empty() {
            return Ok(None);
        }
        let path = self.path();
        let id = crate::cache::path_id(path);
        let mut best: Option<(SequenceNumber, Lookup)> = None;
        for bi in self.blocks_for_point(user_key) {
            let block = if let Some(hit) = cache.get_id(id, bi) {
                hit
            } else {
                let decoded = self.decode_block(bi)?;
                cache.get_or_insert_with_id(id, bi, || decoded)
            };
            if let Some((seq, look)) = Self::best_point_in_entry_slice(&block, user_key, snapshot) {
                if best.as_ref().is_none_or(|(s, _)| seq > *s) {
                    best = Some((seq, look));
                }
            }
        }
        Ok(best)
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

    /// RAM of the sparse index (handles + first-key bytes). Bulk 100M is
    /// O(blocks) here, not O(values) — payload must stay empty.
    #[must_use]
    pub fn index_memory_bytes(&self) -> usize {
        let mut n = self
            .index
            .len()
            .saturating_mul(std::mem::size_of::<BlockHandle>());
        for h in self.index.iter() {
            n = n.saturating_add(h.first_user_key.len());
        }
        n
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

    /// Retarget the path after the file is renamed into place. Freshly
    /// written tables are constructed in place from the writer state (no
    /// post-write re-read); the writer saw the `.tmp` path, so the rename
    /// must update it — the payload is the on-disk image by construction
    /// and eviction re-reads resolve through this path.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = path.into();
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
    /// Big-endian u64 of `key[cp..cp+8]`, zero-padded past the key's end.
    /// Because the padding byte (0) is ≤ any real byte, comparing windows
    /// then falling back to a full memcmp on equality is order-exact.
    fn p8_window(key: &[u8], cp: usize) -> u64 {
        let mut buf = [0u8; 8];
        let end = (cp + 8).min(key.len());
        if end > cp {
            buf[..end - cp].copy_from_slice(&key[cp..end]);
        }
        u64::from_be_bytes(buf)
    }

    /// Fill every handle's `p8` and return the common-prefix offset they
    /// are relative to. Must run once per index after all handles exist
    /// (both at open and at flush/finish, before the table is used).
    fn derive_index_accel(index: &mut [BlockHandle]) -> usize {
        let mut cp = usize::MAX;
        for w in index.windows(2) {
            let n = w[0]
                .first_user_key
                .iter()
                .zip(w[1].first_user_key.iter())
                .take_while(|(a, b)| a == b)
                .count();
            cp = cp.min(n);
        }
        if cp == usize::MAX {
            cp = 0;
        }
        for h in index.iter_mut() {
            h.p8 = Self::p8_window(&h.first_user_key, cp);
        }
        cp
    }

    /// `index[i]` covers `[first_user_key[i], first_user_key[i+1])`. If an
    /// older writer split mid-key, versions also sit in the previous block
    /// and in any following run with `first_user_key == user_key`.
    fn blocks_for_point(&self, user_key: &[u8]) -> std::ops::Range<usize> {
        if self.index.is_empty() {
            return 0..0;
        }
        let t8 = Self::p8_window(user_key, self.key_cp);
        let ge = self
            .index
            .partition_point(|h| h.p8 < t8 || (h.p8 == t8 && h.first_user_key.as_ref() < user_key));
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
    /// An evicted payload (RFC-0042 v18) is served from file: v5+ reads the
    /// block and verifies its CRC32C; ≤v4 re-reads the whole body and
    /// re-verifies the file-level CRC. Both fail closed.
    ///
    /// # Errors
    /// Corrupt block payload, invalid index, or I/O on an evicted table.
    pub fn decode_block(&self, block_idx: usize) -> Result<Vec<(InternalKey, Bytes)>> {
        let h = self.index.get(block_idx).ok_or_else(|| {
            CoreError::Internal(format!(
                "SST block index {block_idx} out of range in {}",
                self.path.display()
            ))
        })?;
        {
            let g = self.payload.read();
            let p: &Arc<[u8]> = &g.img;
            if !p.is_empty() {
                SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
                return decode_block_from_payload(
                    p,
                    h,
                    self.compressed_blocks,
                    self.block_crc,
                    &self.path,
                );
            }
        }
        self.decode_block_on_file(h)
    }

    /// Serve one block of an evicted payload through the attached source.
    fn decode_block_on_file(&self, h: &BlockHandle) -> Result<Vec<(InternalKey, Bytes)>> {
        let kit = self.kit.read().clone();
        let Some(kit) = kit else {
            if !self.body_on_disk {
                return Err(CoreError::Internal(format!(
                    "SST {} payload evicted without a file source (free-standing table)",
                    self.path.display()
                )));
            }
            // Streaming bulk writer leaves the body on disk and the
            // in-memory slot empty. Tests use `open_with` (no kit);
            // hydrate attaches a kit at install.
            let payload = self.ensure_payload_from_path()?;
            SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
            return decode_block_from_payload(
                &payload,
                h,
                self.compressed_blocks,
                self.block_crc,
                &self.path,
            );
        };
        if self.block_crc {
            // v5: read exactly this block (CRC included) — rocks-shaped 4 KiB I/O.
            let len = usize::try_from(h.length)
                .map_err(|_| CoreError::Internal("SST block length overflow".into()))?;
            let mut raw = vec![0u8; len];
            kit.source
                .read_range(&self.path, h.offset, &mut raw)
                .map_err(CoreError::Io)?;
            SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
            decode_block_bytes(&raw, self.compressed_blocks, true, &self.path)
        } else {
            // ≤v4 has no per-block CRC: reload the whole body so the
            // file-level CRC gate re-runs before any block decodes.
            let payload = self.ensure_payload(&kit)?;
            SST_BLOCKS_DECODED.with(|c| c.set(c.get().saturating_add(1)));
            decode_block_from_payload(
                &payload,
                h,
                self.compressed_blocks,
                self.block_crc,
                &self.path,
            )
        }
    }

    /// Make the CRC-stripped file body resident again (whole-file read, file
    /// CRC verified — fail-closed) and re-register it with the pool.
    fn ensure_payload(&self, kit: &PayloadKit) -> Result<Arc<[u8]>> {
        {
            let g = self.payload.read();
            let p: &Arc<[u8]> = &g.img;
            if !p.is_empty() {
                return Ok(Arc::clone(p));
            }
        }
        let buf = kit.source.read_all(&self.path).map_err(CoreError::Io)?;
        let body = crc_stripped_body(&buf, &self.path)?;
        let body: Arc<[u8]> = Arc::from(body.to_vec().into_boxed_slice());
        if body.len() != self.payload_len {
            return Err(CoreError::Internal(format!(
                "SST {} payload length drift on reload: {} != {}",
                self.path.display(),
                body.len(),
                self.payload_len
            )));
        }
        // Fresh residency installs fresh (empty) CRC marks — a mark from the
        // previous residency must never trust new bytes.
        *self.payload.write() = crate::cache::ResidentBody::from_image(Arc::clone(&body));
        kit.pool.register(
            &self.path,
            self.payload_slot_weak(),
            self.payload_len as u64,
        );
        Ok(body)
    }

    fn ensure_payload_from_path(&self) -> Result<Arc<[u8]>> {
        {
            let g = self.payload.read();
            if !g.img.is_empty() {
                return Ok(Arc::clone(&g.img));
            }
        }
        let mut file = StdEnv.open_read(&self.path).map_err(CoreError::Io)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(CoreError::Io)?;
        let body = crc_stripped_body(&buf, &self.path)?;
        let body: Arc<[u8]> = Arc::from(body.to_vec().into_boxed_slice());
        if self.payload_len != 0 && body.len() != self.payload_len {
            return Err(CoreError::Internal(format!(
                "SST {} payload length drift on path reload: {} != {}",
                self.path.display(),
                body.len(),
                self.payload_len
            )));
        }
        *self.payload.write() = crate::cache::ResidentBody::from_image(Arc::clone(&body));
        Ok(body)
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
        let payload = crc_stripped_body(buf, path)?;

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
            SST_VERSION_V6 => Self::decode_v2_or_v3(path, payload, &mut c, true, false, true),
            other => Err(CoreError::Internal(format!(
                "unsupported SST version {other} in {}",
                path.display()
            ))),
        }
    }

    fn decode_v1(path: &Path, file_len: usize, c: &mut Cursor<'_>) -> Result<Self> {
        let n = usize::try_from(c.read_u64()?)
            .map_err(|_| CoreError::Internal("SST entry count does not fit usize".into()))?;
        check_sst_entry_count(n, file_len, false, path)?;
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
        check_sst_entry_count(n, buf.len(), compressed_blocks, path)?;
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
                // Real value assigned by `derive_index_accel` below.
                p8: 0,
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
        let payload_len = payload.len();
        let key_cp = Self::derive_index_accel(&mut index);
        let cf = crate::cf_kernel::infer_sst_cf(
            smallest_user_key.as_deref(),
            largest_user_key.as_deref(),
        );
        Ok(Self {
            path: path.to_path_buf(),
            payload: Arc::new(parking_lot::RwLock::new(
                crate::cache::ResidentBody::from_image(payload),
            )),
            payload_len,
            body_on_disk: false,
            compressed_blocks,
            block_crc,
            // Lazy: do not retain full entry vec after open verification.
            entries: Arc::new(Mutex::new(None)),
            kit: Arc::new(RwLock::new(None)),
            range_tombstones,
            num_entries: n,
            max_sequence,
            index: index.into(),
            key_cp,
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
        let payload_len = payload.len();
        let mut index = index;
        let key_cp = Self::derive_index_accel(&mut index);
        Self {
            path,
            payload: Arc::new(parking_lot::RwLock::new(
                crate::cache::ResidentBody::from_image(payload),
            )),
            payload_len,
            body_on_disk: false,
            compressed_blocks,
            block_crc,
            entries: Arc::new(Mutex::new(Some(entries))),
            kit: Arc::new(RwLock::new(None)),
            range_tombstones,
            num_entries,
            max_sequence,
            index: index.into(),
            key_cp,
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

    /// First user key of every data block, from the in-memory index (no I/O).
    /// Used to sample key-space split points for the parallel merge.
    #[must_use]
    pub fn block_first_user_keys(&self) -> Vec<&[u8]> {
        self.index
            .iter()
            .map(|h| h.first_user_key.as_ref())
            .collect()
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
            hi_excl: None,
        }
    }

    /// Like [`SstTable::iter_internal_streaming`] but bounded to user keys in
    /// `[lo, hi)` (`None` = unbounded on that side). Used by the parallel
    /// merge: each span owns a half-open user-key window, so a user key's
    /// whole version run always lands in exactly one span. Blocks whose
    /// first user key is below `lo` are skipped by index seek — never
    /// decoded.
    #[must_use]
    pub fn iter_internal_between(
        &self,
        lo: Option<&[u8]>,
        hi_excl: Option<&[u8]>,
    ) -> SstInternalStream<'_> {
        // First block whose first user key is >= lo: everything before it
        // ends below lo (entries are sorted by user key).
        let block_i = lo
            .map(|lo| {
                self.index
                    .partition_point(|h| h.first_user_key.as_ref() < lo)
            })
            .unwrap_or(0);
        SstInternalStream {
            table: self,
            block_i,
            block: None,
            entry_i: 0,
            failed: false,
            hi_excl: hi_excl.map(Bytes::copy_from_slice),
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
    /// Half-open span end (`[lo, hi)` user keys): the first entry at or past
    /// this user key ends the stream permanently.
    hi_excl: Option<Bytes>,
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
                while self.entry_i < block.len() {
                    let (k, v) = &block[self.entry_i];
                    if let Some(hi) = self.hi_excl.as_deref() {
                        if k.user_key.as_ref() >= hi {
                            // Past the span end: stop for good (entries are
                            // user-key sorted, so nothing later qualifies).
                            // `failed` is the terminal flag for both lazy
                            // and eager tables; no error was raised.
                            self.failed = true;
                            return Ok(None);
                        }
                    }
                    let e = (k.clone(), v.clone());
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

/// Seek one raw block image (v5: CRC trailer included).
///
/// Verifies the CRC fail-closed, decompresses into `plain_scratch` when the
/// block is lz4, then walks. The scratch is cleared and re-sized per block, so
/// steady-state gets allocate nothing.
fn seek_point_in_block_image(
    raw: &[u8],
    compressed: bool,
    user_key: &[u8],
    snapshot: SequenceNumber,
    plain_scratch: &mut Vec<u8>,
    path: &Path,
) -> Result<Option<(SequenceNumber, Lookup)>> {
    let body = split_block_crc(raw, path)?;
    if get_stages_enabled() {
        let t = Instant::now();
        let found =
            seek_point_in_block_body(body, compressed, user_key, snapshot, plain_scratch, path)?;
        GET_WALK_NS.with(|c| add_stage(c, t.elapsed().as_nanos()));
        Ok(found)
    } else {
        seek_point_in_block_body(body, compressed, user_key, snapshot, plain_scratch, path)
    }
}

/// Seek a CRC-stripped block body the caller already verified: decompress
/// when lz4, then walk. No CRC work — the verified-residency fast path.
fn seek_point_in_block_body(
    body: &[u8],
    compressed: bool,
    user_key: &[u8],
    snapshot: SequenceNumber,
    plain_scratch: &mut Vec<u8>,
    path: &Path,
) -> Result<Option<(SequenceNumber, Lookup)>> {
    let plain: &[u8] = if compressed {
        let (size, input) = lz4_flex::block::uncompressed_size(body).map_err(|e| {
            CoreError::Internal(format!(
                "SST lz4 decompress failed in {}: {e}",
                path.display()
            ))
        })?;
        if !lz4_plain_size_ok(size, body.len()) {
            return Err(lz4_size_rejected(path, size, body.len()));
        }
        plain_scratch.clear();
        plain_scratch.resize(size, 0);
        let written = lz4_flex::block::decompress_into(input, plain_scratch).map_err(|e| {
            CoreError::Internal(format!(
                "SST lz4 decompress failed in {}: {e}",
                path.display()
            ))
        })?;
        if written != size {
            return Err(CoreError::Internal(format!(
                "SST lz4 size prefix mismatch in {}: wrote {written} of {size} bytes",
                path.display()
            )));
        }
        plain_scratch.as_slice()
    } else {
        body
    };
    seek_point_in_plain_block(plain, user_key, snapshot, path)
}

/// Newest visible point version of `user_key` in one decoded block image.
///
/// Raw walk over `ikey_len|ikey|val_len|val` records: compare the user-key
/// prefix without allocating an `InternalKey` per entry, skip lesser keys by
/// jumping `val_len`, stop at the first greater key, and copy out only a
/// winning value. Entries are user-key ascending, sequence descending, so the
/// first visible equal-key entry is the newest — the max-seq keep matches
/// [`SstTable::best_point_in_entry_slice`] in case ordering ever changes.
/// Framing faults return `Err` (fail-closed): a truncated or corrupt block
/// must never read as a miss.
fn seek_point_in_plain_block(
    plain: &[u8],
    user_key: &[u8],
    snapshot: SequenceNumber,
    path: &Path,
) -> Result<Option<(SequenceNumber, Lookup)>> {
    let mut pos = 0usize;
    let mut best: Option<(SequenceNumber, Lookup)> = None;
    while pos < plain.len() {
        if pos + 4 > plain.len() {
            return Err(CoreError::Internal(format!(
                "SST block entry truncated in {}",
                path.display()
            )));
        }
        let ikey_len = u32::from_le_bytes(plain[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if ikey_len < 8 {
            return Err(CoreError::Internal(format!(
                "SST block internal key too short in {}",
                path.display()
            )));
        }
        let Some(ikey_end) = pos.checked_add(ikey_len) else {
            return Err(CoreError::Internal(format!(
                "SST block entry truncated in {}",
                path.display()
            )));
        };
        if ikey_end + 4 > plain.len() {
            return Err(CoreError::Internal(format!(
                "SST block entry truncated in {}",
                path.display()
            )));
        }
        let uk_end = ikey_end - 8;
        let mut trailer = [0u8; 8];
        trailer.copy_from_slice(&plain[uk_end..ikey_end]);
        let val_len =
            u32::from_le_bytes(plain[ikey_end..ikey_end + 4].try_into().unwrap()) as usize;
        let Some(val_end) = ikey_end.checked_add(4).and_then(|v| v.checked_add(val_len)) else {
            return Err(CoreError::Internal(format!(
                "SST block entry length overflow in {}",
                path.display()
            )));
        };
        if val_end > plain.len() {
            return Err(CoreError::Internal(format!(
                "SST block entry truncated in {}",
                path.display()
            )));
        }
        match plain[pos..uk_end].cmp(user_key) {
            Ordering::Less => {}
            Ordering::Greater => break,
            Ordering::Equal => {
                let (sequence, kind) =
                    crate::key::unpack_sequence_and_type(u64::from_be_bytes(trailer))?;
                if kind != ValueType::RangeDeletion && sequence <= snapshot {
                    if best.as_ref().is_none_or(|(s, _)| sequence > *s) {
                        let look = match kind {
                            ValueType::Deletion => Lookup::Deleted,
                            ValueType::Value => {
                                Lookup::Found(Bytes::copy_from_slice(&plain[ikey_end + 4..val_end]))
                            }
                            // Unreachable: RangeDeletion is filtered above.
                            ValueType::RangeDeletion => Lookup::NotFound,
                        };
                        best = Some((sequence, look));
                    }
                }
            }
        }
        pos = val_end;
    }
    Ok(best)
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
    decode_block_bytes(&buf[start..end], compressed_blocks, block_crc, path)
}

/// Strip and verify a v5 block's trailing CRC32C — fail-closed on mismatch
/// or truncation. Shared by the decode path and the point seek so a block
/// never parses before its integrity gate passes.
fn split_block_crc<'a>(raw: &'a [u8], path: &Path) -> Result<&'a [u8]> {
    if raw.len() < 4 {
        return Err(CoreError::Internal(format!(
            "SST block CRC truncated in {}",
            path.display()
        )));
    }
    let (body, crc_bytes) = raw.split_at(raw.len() - 4);
    let stored = u32::from_le_bytes(crc_bytes.try_into().unwrap());
    let computed = if get_stages_enabled() {
        let t = Instant::now();
        let c = crc32c::crc32c(body);
        GET_CRC_NS.with(|cell| add_stage(cell, t.elapsed().as_nanos()));
        c
    } else {
        crc32c::crc32c(body)
    };
    if !crate::sst::sst_block_crc_ok(stored, computed) {
        return Err(CoreError::Internal(format!(
            "SST block CRC mismatch in {}",
            path.display()
        )));
    }
    Ok(body)
}

/// Decode one on-disk block image. `raw` includes the trailing CRC32C when
/// `block_crc` — verified here, fail-closed (bitrot never decodes garbage).
fn decode_block_bytes(
    raw: &[u8],
    compressed_blocks: bool,
    block_crc: bool,
    path: &Path,
) -> Result<Vec<(InternalKey, Bytes)>> {
    let raw = if block_crc {
        split_block_crc(raw, path)?
    } else {
        raw
    };
    let plain: Vec<u8> = if compressed_blocks {
        let (size, _) = lz4_flex::block::uncompressed_size(raw).map_err(|e| {
            CoreError::Internal(format!(
                "SST lz4 decompress failed in {}: {e}",
                path.display()
            ))
        })?;
        if !lz4_plain_size_ok(size, raw.len()) {
            return Err(lz4_size_rejected(path, size, raw.len()));
        }
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

/// CRC-stripped file body: new files append a 4-byte LE CRC32C trailer (F3);
/// a stored/computed mismatch refuses the file (fail-stop on bitrot), legacy
/// trailer-less files parse the whole buffer. Shared by open and the evicted
/// whole-body reload (RFC-0042 v18) so both run the same integrity gate.
fn crc_stripped_body<'a>(buf: &'a [u8], path: &Path) -> Result<&'a [u8]> {
    if buf.len() >= 12 && buf.starts_with(SST_MAGIC) {
        let (head, tail4) = buf.split_at(buf.len() - 4);
        let stored = u32::from_le_bytes([tail4[0], tail4[1], tail4[2], tail4[3]]);
        let computed = if head.len() >= 12
            && u32::from_le_bytes(head[8..12].try_into().unwrap_or([0; 4])) == SST_VERSION_V6
        {
            v6_file_crc(head).ok_or_else(|| {
                CoreError::Internal(format!("SST v6 CRC range invalid in {}", path.display()))
            })?
        } else {
            crc32c::crc32c(head)
        };
        match super::scan_kernel::sst_crc_fate(stored, computed, buf.len()) {
            super::scan_kernel::SstCrcFate::StripTrailer => Ok(head),
            super::scan_kernel::SstCrcFate::WholeBuffer => Ok(buf),
            super::scan_kernel::SstCrcFate::Reject => Err(CoreError::Internal(format!(
                "SST file CRC mismatch in {} (stored {stored:#010x}, computed {computed:#010x})",
                path.display()
            ))),
        }
    } else {
        Ok(buf)
    }
}

/// v6 trailer is CRC32C(header ‖ index/bloom tail), not the data blocks
/// (those carry per-block CRC32C). `None` = not a v6 body; caller uses
/// whole-body CRC (v2–v5).
fn v6_file_crc(head: &[u8]) -> Option<u32> {
    if head.len() < BULK_SST_HEADER_LEN {
        return None;
    }
    if u32::from_le_bytes(head[8..12].try_into().ok()?) != SST_VERSION_V6 {
        return None;
    }
    let data_len = u64::from_le_bytes(head[32..40].try_into().ok()?);
    let tail_off = (BULK_SST_HEADER_LEN as u64).checked_add(data_len)?;
    let tail_off = usize::try_from(tail_off).ok()?;
    if tail_off > head.len() {
        return None;
    }
    let hdr_crc = crc32c::crc32c(&head[..BULK_SST_HEADER_LEN]);
    let tail = &head[tail_off..];
    Some(crc32c::crc32c_combine(
        hdr_crc,
        crc32c::crc32c(tail),
        tail.len(),
    ))
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

/// L0 flush: [`write_sst_on_with`] with compressed blocks (SST v5, lz4 +
/// per-block CRC32C).
///
/// Runs on the flush worker, not the apply tail, so the lz4 cost does not
/// gate puts; a compressed body keeps the writer's in-RAM file body and the
/// pooled payload several times smaller than a v3 one.
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
        true,
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
        true,
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
/// flight; bloom built from the distinct keys written (`bloom_hint` > 0
/// enables the filter). Does not clone the input set.
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

/// Bulk-run SST: trusted-sorted `(key, val, seq)` arrays, SST **v6**
/// (uncompressed 4 KiB blocks + per-block CRC + bloom). No `InternalKey`,
/// no per-entry `Result`, no lz4. v3 evicted gets re-read the whole file;
/// v6 is a Rocks-shaped 4 KiB `read_range`.
///
/// The filter is sized by the distinct keys written (same 10 bits/key
/// as the sorted writer). An always-true bloom made absent-key
/// `probe_miss` a hidden loss; bulk files skip the filter only if the
/// key set is empty, which this writer rejects.
///
/// Always `fdatasync`s before returning (RFC-0162 P1.1): the chunk
/// install is a durability + page-cache hygiene point, not the DB
/// `sync` default — an unsynced 64 MiB chunk leaves writeback debt that
/// throttles later chunks.
///
/// # Errors
/// Length mismatch, oversized field, or I/O.
pub fn write_sst_bulk_arrays(
    env: &impl Env,
    path: impl AsRef<Path>,
    keys: &[Bytes],
    vals: &[Bytes],
    seqs: &[SequenceNumber],
) -> Result<SstTable> {
    write_sst_bulk_arrays_body(env, path.as_ref(), keys, vals, seqs)
}

const BULK_SST_HEADER_LEN: usize = 40;
/// Stage this many encoded bytes before a `write` (hot cache, few syscalls).
const BULK_STREAM_BATCH: usize = 4 * 1024 * 1024;
/// Bitset cap shared with the sorted writer: inserts past this raise
/// FPR; the bitset does not grow. One 64 MiB bulk chunk is ~2.5e5 keys
/// at 200 B values, well under 2M.
const BLOOM_CAP_MAX: usize = 2_097_152;

/// Bloom for a bulk file: sized by distinct written keys, 10 bits/key.
fn bulk_bloom(keys: &[Bytes]) -> BloomFilter {
    let mut bloom = BloomFilter::with_capacity(keys.len().min(BLOOM_CAP_MAX), DEFAULT_BITS_PER_KEY);
    let mut last: Option<&[u8]> = None;
    for k in keys {
        let uk = k.as_ref();
        if last != Some(uk) {
            bloom.insert(uk);
            last = Some(uk);
        }
    }
    bloom
}

fn write_sst_bulk_arrays_body(
    env: &impl Env,
    path: &Path,
    keys: &[Bytes],
    vals: &[Bytes],
    seqs: &[SequenceNumber],
) -> Result<SstTable> {
    if keys.len() != vals.len() || keys.len() != seqs.len() {
        return Err(CoreError::Internal(
            "bulk SST keys/vals/seqs length mismatch".into(),
        ));
    }
    let n_entries = keys.len();
    if n_entries == 0 {
        return Err(CoreError::Internal("bulk SST empty".into()));
    }
    let mut stages = StageTotals {
        enabled: std::env::var_os("PEDRA_FLUSH_STAGES").is_some(),
        ..StageTotals::default()
    };
    let t_enc = std::time::Instant::now();
    let target = block_target().min(BULK_BLOCK_TARGET).max(BLOCK_TARGET);
    let n_blocks_est = n_entries.saturating_mul(256) / target + 2;
    let mut file = env.create(path)?;
    let mut header = [0u8; BULK_SST_HEADER_LEN];
    file.write_all(&header)?;
    let mut pos = BULK_SST_HEADER_LEN as u64;
    let mut staged = Vec::with_capacity(BULK_STREAM_BATCH.saturating_add(target));
    let mut index: Vec<BlockHandle> = Vec::with_capacity(n_blocks_est);
    let mut block_first_user: Option<Bytes> = None;
    let mut block_start = 0usize;
    let mut max_sequence = 0u64;
    // Encode 4 KiB blocks straight into the 4 MiB write batch. A side
    // `block_buf` plus copy was 5.75 GiB extra memcpy (v56 25M hydrate
    // 33.3 s / 0.86× vs Rocks; v54 256 KiB did the same copy at 1/64 the
    // call rate). CRC still runs on the in-place slice.
    for i in 0..n_entries {
        let k = keys[i].as_ref();
        let v = vals[i].as_ref();
        let seq = seqs[i];
        if seq > max_sequence {
            max_sequence = seq;
        }
        let need = k.len() + v.len() + 16;
        if staged.len() - block_start > 0 && staged.len() - block_start + need > target {
            finish_staged_block(
                &mut file,
                &mut staged,
                block_start,
                &mut pos,
                block_first_user.take(),
                &mut index,
            )?;
            block_start = staged.len();
        }
        if staged.len() == block_start {
            block_first_user = Some(keys[i].clone());
        }
        append_bulk_entry(&mut staged, k, seq, v);
    }
    if staged.len() > block_start {
        finish_staged_block(
            &mut file,
            &mut staged,
            block_start,
            &mut pos,
            block_first_user.take(),
            &mut index,
        )?;
    }
    if !staged.is_empty() {
        file.write_all(&staged)?;
        staged.clear();
    }
    // Owned copies: `keys[i]` are slices of the caller's pooled key buffer
    // (compat `KEY_POOL`, 8 KiB chunks). Retaining a slice pins the whole
    // chunk; the table must not keep the ingest pool alive (100M RSS).
    let smallest_user_key = Some(Bytes::copy_from_slice(&keys[0]));
    let largest_user_key = Some(Bytes::copy_from_slice(&keys[n_entries - 1]));
    let data_len = pos - BULK_SST_HEADER_LEN as u64;
    let key_cp = SstTable::derive_index_accel(&mut index);
    let bloom_bytes = n_entries
        .saturating_mul(DEFAULT_BITS_PER_KEY)
        .saturating_div(8)
        .saturating_add(16);
    let mut tail = Vec::with_capacity(
        index
            .len()
            .saturating_mul(48)
            .saturating_add(64)
            .saturating_add(bloom_bytes),
    );
    for h in &index {
        tail.extend_from_slice(&h.offset.to_le_bytes());
        tail.extend_from_slice(&h.length.to_le_bytes());
        let kl = h.first_user_key.len() as u32;
        tail.extend_from_slice(&kl.to_le_bytes());
        tail.extend_from_slice(&h.first_user_key);
    }
    stages.add(|s| &mut s.enc_ns, t_enc);
    let t_bloom = std::time::Instant::now();
    let bloom = bulk_bloom(keys);
    tail.extend_from_slice(&bloom.encode());
    stages.add(|s| &mut s.bloom_ns, t_bloom);
    let n = n_entries as u64;
    let num_blocks = index.len() as u32;
    write_bulk_header(&mut header, n, max_sequence, num_blocks, data_len);

    let t_crc = std::time::Instant::now();
    let hdr_crc = crc32c::crc32c(&header);
    let file_crc = crc32c::crc32c_combine(hdr_crc, crc32c::crc32c(&tail), tail.len());
    stages.add(|s| &mut s.crc_ns, t_crc);
    let payload_len = (pos as usize).saturating_add(tail.len());
    let t_write = std::time::Instant::now();
    file.write_all(&tail)?;
    file.write_all(&file_crc.to_le_bytes())?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    // RFC-0162 P1.1: unconditional — see `write_sst_bulk_arrays` docs.
    file.sync_data()?;
    drop(file);
    stages.add(|s| &mut s.write_ns, t_write);
    if stages.enabled {
        println!(
            "FLUSHSTAGES entries={n_entries} bytes={payload_len_hint} enc_ms={:.1} \
             lz4_ms=0.0 bloom_ms={:.1} crc_ms={:.1} write_ms={:.1} lz4=false",
            stages.enc_ns as f64 / 1e6,
            stages.bloom_ns as f64 / 1e6,
            stages.crc_ns as f64 / 1e6,
            stages.write_ns as f64 / 1e6,
            payload_len_hint = payload_len + 4,
        );
    }
    let cf =
        crate::cf_kernel::infer_sst_cf(smallest_user_key.as_deref(), largest_user_key.as_deref());
    Ok(SstTable {
        path: path.to_path_buf(),
        payload: Arc::new(parking_lot::RwLock::new(crate::cache::ResidentBody::empty())),
        payload_len,
        // Body stays on disk: this table may path-reload without a kit
        // until hydrate attaches one at install (RFC-0159).
        body_on_disk: true,
        compressed_blocks: false,
        block_crc: true,
        entries: Arc::new(Mutex::new(None)),
        kit: Arc::new(RwLock::new(None)),
        range_tombstones: Vec::new(),
        num_entries: n_entries,
        max_sequence,
        index: index.into(),
        key_cp,
        bloom,
        smallest_user_key,
        largest_user_key,
        cf,
    })
}

fn write_bulk_header(image: &mut [u8], n: u64, max_sequence: u64, num_blocks: u32, data_len: u64) {
    debug_assert!(image.len() >= BULK_SST_HEADER_LEN);
    image[0..8].copy_from_slice(SST_MAGIC);
    image[8..12].copy_from_slice(&SST_VERSION_V6.to_le_bytes());
    image[12..20].copy_from_slice(&n.to_le_bytes());
    image[20..28].copy_from_slice(&max_sequence.to_le_bytes());
    image[28..32].copy_from_slice(&num_blocks.to_le_bytes());
    image[32..40].copy_from_slice(&data_len.to_le_bytes());
}

fn finish_staged_block(
    file: &mut impl Write,
    staged: &mut Vec<u8>,
    block_start: usize,
    pos: &mut u64,
    first: Option<Bytes>,
    index: &mut Vec<BlockHandle>,
) -> Result<()> {
    debug_assert!(block_start < staged.len());
    let crc = crc32c::crc32c(&staged[block_start..]);
    let crc_bytes = crc.to_le_bytes();
    let stored = (staged.len() - block_start + 4) as u32;
    // Copy, do not slice: `first` shares the ingest key pool chunk. One
    // sparse-index entry per block would otherwise pin every pooled key of
    // the run for the table's lifetime (measured ~37 B/entry resident at
    // 4M–100M; the 3.9 GiB guest SIGKILLs at 100M).
    let first = first.expect("bulk block missing first key");
    index.push(BlockHandle {
        offset: *pos,
        length: stored,
        first_user_key: Bytes::copy_from_slice(&first),
        p8: 0,
    });
    *pos += u64::from(stored);
    staged.extend_from_slice(&crc_bytes);
    if staged.len() >= BULK_STREAM_BATCH {
        file.write_all(staged)?;
        staged.clear();
    }
    Ok(())
}

#[inline]
fn append_bulk_entry(buf: &mut Vec<u8>, k: &[u8], seq: SequenceNumber, v: &[u8]) {
    // Hydrate keys/vals are tens/hundreds of bytes; skip try_from / Result.
    let ikey_len = (k.len() + 8) as u32;
    let val_len = v.len() as u32;
    buf.extend_from_slice(&ikey_len.to_le_bytes());
    buf.extend_from_slice(k);
    buf.extend_from_slice(&pack_sequence_and_type(seq, ValueType::Value).to_be_bytes());
    buf.extend_from_slice(&val_len.to_le_bytes());
    buf.extend_from_slice(v);
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

/// Per-stage timing for `PEDRA_FLUSH_STAGES` (RFC-0159 P1.1): where the
/// materialize wall goes. Zero-cost when the env is unset.
#[derive(Default)]
struct StageTotals {
    enabled: bool,
    enc_ns: u64,
    lz4_ns: u64,
    bloom_ns: u64,
    crc_ns: u64,
    write_ns: u64,
}

impl StageTotals {
    fn add(&mut self, which: fn(&mut Self) -> &mut u64, t0: std::time::Instant) {
        if self.enabled {
            let ns = t0.elapsed().as_nanos() as u64;
            *which(self) = which(self).saturating_add(ns);
        }
    }
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
    let mut stages = StageTotals {
        enabled: std::env::var_os("PEDRA_FLUSH_STAGES").is_some(),
        ..StageTotals::default()
    };
    // The bloom is built AFTER the entry loop, sized by the distinct user
    // keys actually written. `with_capacity(bloom_hint)` sized every output
    // file by the caller's TOTAL: whole-levels rewrites pass the sum over
    // all inputs (25M keys), so every 64 MiB chunk carried a ~31 MB
    // mostly-zero bloom — retained per opened table (a plain field, never
    // payload-evictable) and shipped on disk. Distinct user keys are
    // retained through the encode loop and inserted once at the end:
    // `Bytes` clones share the source allocation, and the retention is
    // bounded by the keys of one output file, whose full image this writer
    // already assembles in memory. `bloom_hint == 0` still means no
    // filter; the cap only bounds the bitset when a single file holds
    // tens of millions of keys (inserts past capacity raise FPR; the
    // bitset does not grow).
    let bloom_active = bloom_hint != 0;
    let mut bloom_keys: Vec<Bytes> = Vec::new();
    let mut data = Vec::new();
    let mut index: Vec<BlockHandle> = Vec::new();
    let mut block_buf = Vec::new();
    let mut lz4_scratch = Vec::new();
    let mut block_first_user: Option<Bytes> = None;
    let mut max_sequence = 0u64;
    let mut n_entries = 0usize;
    // Compression policy: undecided until the first block probes the ratio.
    // `PEDRA_LZ4_PROBE=0` restores the unconditional-lz4 policy (A/B arm).
    let mut policy_compress = compress;
    let mut policy_decided =
        !compress || std::env::var("PEDRA_LZ4_PROBE").map_or(false, |v| v == "0");

    // RFC-0159 P1.1: entries encode straight into `block_buf` (the old path
    // staged into `enc_scratch` then copied — one full extra pass over every
    // byte). A block split truncates the just-encoded tail and re-encodes it
    // into the fresh block (once per block, not per entry).
    fn lz4_into(src: &[u8], scratch: &mut Vec<u8>) -> Result<()> {
        let max = 4usize.saturating_add(lz4_flex::block::get_maximum_output_size(src.len()));
        scratch.clear();
        scratch.resize(max, 0);
        let n = u32::try_from(src.len())
            .map_err(|_| CoreError::Internal("SST block too large".into()))?;
        scratch[..4].copy_from_slice(&n.to_le_bytes());
        let wrote = lz4_flex::block::compress_into(src, &mut scratch[4..])
            .map_err(|_| CoreError::Internal("lz4 compress failed".into()))?;
        scratch.truncate(4usize.saturating_add(wrote));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn flush_block(
        data: &mut Vec<u8>,
        block_buf: &mut Vec<u8>,
        block_first_user: &mut Option<Bytes>,
        index: &mut Vec<BlockHandle>,
        stages: &mut StageTotals,
        policy_compress: &mut bool,
        policy_decided: &mut bool,
        lz4_scratch: &mut Vec<u8>,
    ) -> Result<()> {
        if block_buf.is_empty() {
            return Ok(());
        }
        let t0 = std::time::Instant::now();
        // First block probes the ratio for the whole file: keep lz4 only if
        // it saves ≥10 %; random payloads (bulk chunks) write v3 raw instead.
        if !*policy_decided {
            *policy_decided = true;
            if *policy_compress {
                lz4_into(block_buf, lz4_scratch)?;
                stages.add(|s| &mut s.lz4_ns, t0);
                *policy_compress = lz4_scratch.len() * 10 < block_buf.len() * 9;
                if !*policy_compress {
                    lz4_scratch.clear();
                }
            }
        }
        let offset = data.len() as u64;
        if *policy_compress {
            if lz4_scratch.is_empty() {
                lz4_into(block_buf, lz4_scratch)?;
                stages.add(|s| &mut s.lz4_ns, t0);
            }
            let crc = crc32c::crc32c(lz4_scratch);
            lz4_scratch.extend_from_slice(&crc.to_le_bytes());
            let length = u32::try_from(lz4_scratch.len())
                .map_err(|_| CoreError::Internal("SST block too large".into()))?;
            data.extend_from_slice(lz4_scratch);
            lz4_scratch.clear();
            let first = block_first_user
                .take()
                .ok_or_else(|| CoreError::Internal("block missing first key".into()))?;
            block_buf.clear();
            index.push(BlockHandle {
                offset,
                length,
                // Owned copy — never pin the memtable / pool buffer.
                first_user_key: Bytes::copy_from_slice(&first),
                p8: 0,
            });
            return Ok(());
        }
        stages.add(|s| &mut s.lz4_ns, t0);
        let raw = std::mem::take(block_buf);
        let length = u32::try_from(raw.len())
            .map_err(|_| CoreError::Internal("SST block too large".into()))?;
        data.extend_from_slice(&raw);
        let first = block_first_user
            .take()
            .ok_or_else(|| CoreError::Internal("block missing first key".into()))?;
        index.push(BlockHandle {
            offset,
            length,
            // Owned copy — never pin the memtable / pool buffer.
            first_user_key: Bytes::copy_from_slice(&first),
            p8: 0,
        });
        Ok(())
    }

    // NEVER split a user key across blocks (same contract as write_sst_entries_on).
    let mut prev_ikey: Option<InternalKey> = None;
    let mut smallest_user_key: Option<Bytes> = None;
    let mut range_tombstones: Vec<(InternalKey, Bytes)> = Vec::new();
    let t_enc = std::time::Instant::now();
    for item in entries {
        let (ikey, value) = item?;
        // Same invariant the open-time decode verify enforced: entries must
        // arrive InternalKey-sorted. Checked here (cheap memcmp) instead of
        // via a full decompress+decode pass after writing.
        if let Some(prev) = &prev_ikey {
            if prev > &ikey {
                return Err(CoreError::Internal(format!(
                    "SST entries not sorted in {path_check}",
                    path_check = path.display()
                )));
            }
        }
        max_sequence = max_sequence.max(ikey.sequence);
        n_entries = n_entries.saturating_add(1);
        if smallest_user_key.is_none() {
            smallest_user_key = Some(Bytes::copy_from_slice(&ikey.user_key));
        }
        if ikey.kind == ValueType::RangeDeletion {
            range_tombstones.push((ikey.clone(), value.clone()));
        }
        let uk = ikey.user_key.as_ref();
        // Same-user as the previous entry (block split + bloom distinct).
        // `prev_ikey` already owns that key — do not clone it per entry.
        let same_user = prev_ikey
            .as_ref()
            .is_some_and(|p| p.user_key.as_ref() == uk);
        if !same_user && bloom_active {
            // Refcount clone; the key bytes are inserted into the filter
            // after the loop, when their count is known.
            bloom_keys.push(ikey.user_key.clone());
        }
        if block_buf.is_empty() {
            block_first_user = Some(ikey.user_key.clone());
        }
        let pre_len = block_buf.len();
        encode_entry_into(&ikey, &value, &mut block_buf)?;
        if !same_user && block_buf.len() > block_target() && pre_len > 0 {
            // Overflow: the just-encoded entry moves to the fresh block.
            block_buf.truncate(pre_len);
            flush_block(
                &mut data,
                &mut block_buf,
                &mut block_first_user,
                &mut index,
                &mut stages,
                &mut policy_compress,
                &mut policy_decided,
                &mut lz4_scratch,
            )?;
            block_first_user = Some(ikey.user_key.clone());
            encode_entry_into(&ikey, &value, &mut block_buf)?;
        }
        // `prev_ikey` (and the file's largest key) is the last entry — keep
        // it by move, not by clone.
        prev_ikey = Some(ikey);
    }
    // The file's largest user key is the last entry's — derived from
    // `prev_ikey` by move, not tracked with a per-entry clone.
    let largest_user_key = prev_ikey
        .as_ref()
        .map(|k| Bytes::copy_from_slice(&k.user_key));
    flush_block(
        &mut data,
        &mut block_buf,
        &mut block_first_user,
        &mut index,
        &mut stages,
        &mut policy_compress,
        &mut policy_decided,
        &mut lz4_scratch,
    )?;
    let key_cp = SstTable::derive_index_accel(&mut index);
    stages.add(|s| &mut s.enc_ns, t_enc);
    // Size by the distinct keys written, then insert them all. `bloom_hint`
    // only decided whether the file gets a filter at all.
    let t_bloom = std::time::Instant::now();
    let bloom = if bloom_active && !bloom_keys.is_empty() {
        let mut b =
            BloomFilter::with_capacity(bloom_keys.len().min(BLOOM_CAP_MAX), DEFAULT_BITS_PER_KEY);
        for key in &bloom_keys {
            b.insert(key);
        }
        b
    } else {
        BloomFilter::always_true()
    };
    drop(bloom_keys);
    stages.add(|s| &mut s.bloom_ns, t_bloom);

    // Header: magic version num_entries max_seq num_blocks data_len (fixed 40 B)
    let mut header = Vec::with_capacity(40);
    header.extend_from_slice(SST_MAGIC);
    let version = if policy_compress {
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
    let mut bloom_bytes = bloom.encode();

    let t_crc = std::time::Instant::now();
    let mut file_crc = crc32c::crc32c(&header);
    file_crc = crc32c::crc32c_append(file_crc, &data);
    file_crc = crc32c::crc32c_append(file_crc, &index_bytes);
    file_crc = crc32c::crc32c_append(file_crc, &bloom_bytes);
    stages.add(|s| &mut s.crc_ns, t_crc);

    // Exact file image assembled once: a single write syscall (was five),
    // and the SstTable is constructed from this in-memory state — the
    // RocksDB `TableBuilder::Finish` class. The old `SstTable::open_on`
    // re-read the whole file, re-CRC'd it, and decompress+decode verified
    // every block and entry: ~2 extra full passes over every flushed or
    // compacted byte (the drain-pipeline tax behind slipstream settle).
    // Read-side opens keep the full verify; blocks still CRC lazily on
    // first read, so torn files fail closed exactly as before.
    let mut image = header;
    image.append(&mut data);
    image.append(&mut index_bytes);
    image.append(&mut bloom_bytes);
    image.extend_from_slice(&file_crc.to_le_bytes());
    // PEDRA_PARK_DIAG2: split the write stage (create / write_all+sync /
    // close). The #31 re-parse put the whole stage at 8.7 s of the 44.8 s
    // files wall — which third is syscall-bound decides the next lever.
    let diag2 = std::env::var_os("PEDRA_PARK_DIAG2").is_some();
    let d2_create_ms;
    let d2_wr_ms;
    let t_write = std::time::Instant::now();
    {
        let t_c = std::time::Instant::now();
        let mut file = env.create(path)?;
        d2_create_ms = t_c.elapsed().as_secs_f64() * 1e3;
        let t_w = std::time::Instant::now();
        file.write_all(&image)?;
        if sync {
            file.sync_data()?;
        }
        d2_wr_ms = t_w.elapsed().as_secs_f64() * 1e3;
    }
    stages.add(|s| &mut s.write_ns, t_write);
    if diag2 {
        let total_ms = t_write.elapsed().as_secs_f64() * 1e3;
        eprintln!(
            "PARKDIAG2 sst create_ms={d2_create_ms:.1} write_ms={d2_wr_ms:.1} close_ms={:.1}",
            total_ms - d2_create_ms - d2_wr_ms
        );
    }
    if stages.enabled {
        println!(
            "FLUSHSTAGES entries={n_entries} bytes={payload_len_hint} enc_ms={:.1} \
             lz4_ms={:.1} bloom_ms={:.1} crc_ms={:.1} write_ms={:.1} lz4={policy_compress}",
            stages.enc_ns as f64 / 1e6,
            stages.lz4_ns as f64 / 1e6,
            stages.bloom_ns as f64 / 1e6,
            stages.crc_ns as f64 / 1e6,
            stages.write_ns as f64 / 1e6,
            payload_len_hint = image.len(),
        );
    }
    // The retained body is the CRC-stripped image (header included), the
    // same slice `crc_stripped_body` would hand back on an open-on-read.
    image.truncate(image.len() - core::mem::size_of::<u32>());
    let payload: Arc<[u8]> = image.into();
    let payload_len = payload.len();
    let cf =
        crate::cf_kernel::infer_sst_cf(smallest_user_key.as_deref(), largest_user_key.as_deref());
    Ok(SstTable {
        path: path.to_path_buf(),
        payload: Arc::new(parking_lot::RwLock::new(
            crate::cache::ResidentBody::from_image(payload),
        )),
        payload_len,
        body_on_disk: false,
        compressed_blocks: policy_compress,
        block_crc: policy_compress,
        entries: Arc::new(Mutex::new(None)),
        kit: Arc::new(RwLock::new(None)),
        range_tombstones,
        num_entries: n_entries,
        max_sequence,
        index: index.into(),
        key_cp,
        bloom,
        smallest_user_key,
        largest_user_key,
        cf,
    })
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

    /// The bloom is sized by the keys actually written, never by the
    /// caller's `bloom_hint` (whole-levels rewrites pass the sum over
    /// all inputs — every 64 MiB chunk then carried a ~31 MB near-zero
    /// bloom at the 25M scale, retained per opened table and shipped on
    /// disk). 100 entries with a 100M-key hint must stay a small file.
    #[test]
    fn write_sst_bloom_is_sized_by_written_keys_not_hint() {
        let path = temp_path();
        let entries: Vec<(InternalKey, Bytes)> = (0..100u32)
            .map(|i| {
                (
                    InternalKey::new(
                        format!("key{i:06}").into_bytes(),
                        u64::from(i) + 1,
                        ValueType::Value,
                    ),
                    Bytes::from_static(b"payload"),
                )
            })
            .collect();
        let table =
            write_sst_try_sorted_on(&StdEnv, &path, entries.into_iter().map(Ok), 100_000_000)
                .unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        let _ = std::fs::remove_file(&path);
        assert_eq!(table.len(), 100);
        assert!(
            size < 1_000_000,
            "bloom must be sized by written keys, not the hint: {size} bytes"
        );
        // The filter itself must stay active for the written keys.
        assert!(table.has_bloom());
        assert!(table.point_at(b"key000042", u64::MAX).is_some());
    }

    #[test]
    fn write_sst_bulk_arrays_is_v6_and_roundtrips() {
        let path = temp_path();
        let n = 64usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:04}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("v{i:04}").into_bytes()))
            .collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        assert!(!table.compressed_blocks);
        assert!(table.block_crc);
        assert!(table.has_bloom());
        assert_eq!(table.len(), n);
        for i in 0..n {
            let k = format!("k{i:04}");
            assert!(
                table.key_may_match(k.as_bytes()),
                "bloom must not false-negative written key {k}"
            );
        }
        // In-range absents (between existing keys). Bloom FP ~1%; eight
        // probes is enough to prove the filter is live, not always-true.
        let misses: [&[u8]; 8] = [
            b"k0003x",
            b"k0010x",
            b"k0020x",
            b"k0030x",
            b"k0040x",
            b"k0050x",
            b"k0060x",
            b"k0025-gone",
        ];
        let rejected = misses.iter().filter(|k| !table.key_may_match(k)).count();
        assert!(
            rejected >= 6,
            "bulk bloom must reject in-range misses, rejected={rejected}/8"
        );
        assert!(
            !table.payload_resident(),
            "streaming writer must not keep the file body resident"
        );
        drop(table);
        let re = SstTable::open_on(&StdEnv, &path).unwrap();
        assert!(re.has_bloom());
        assert!(matches!(
            re.get(b"k0003", u64::MAX),
            Lookup::Found(v) if v.as_ref() == b"v0003"
        ));
        assert!(matches!(re.get(b"k0003x", u64::MAX), Lookup::NotFound));
        assert!(matches!(re.get(b"missing", u64::MAX), Lookup::NotFound));
        let _ = std::fs::remove_file(&path);
    }

    /// v6 file CRC is header+tail; data bitrot is the per-block CRC on get.
    #[test]
    fn v6_file_crc_covers_tail_not_data() {
        let path = temp_path();
        let n = 64usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:04}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("v{i:04}").into_bytes()))
            .collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        let orig = std::fs::read(&path).unwrap();
        assert!(orig.len() > BULK_SST_HEADER_LEN + 8);

        let mut data_rot = orig.clone();
        data_rot[BULK_SST_HEADER_LEN] ^= 0xff;
        std::fs::write(&path, &data_rot).unwrap();
        let err = SstTable::open_on(&StdEnv, &path).unwrap_err().to_string();
        assert!(
            err.contains("block CRC"),
            "data bitrot is the per-block gate, not the file CRC; got {err}"
        );
        assert!(
            !err.contains("file CRC"),
            "v6 file CRC must not cover data blocks; got {err}"
        );

        let mut tail_rot = orig.clone();
        let i = tail_rot.len() - 8;
        tail_rot[i] ^= 0xff;
        std::fs::write(&path, &tail_rot).unwrap();
        let err = SstTable::open_on(&StdEnv, &path).unwrap_err().to_string();
        assert!(
            err.contains("file CRC") || err.contains("CRC mismatch"),
            "tail bitrot must fail v6 file CRC; got {err}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// PEDRA-001: a forged lz4 size prefix must not allocate.
    #[test]
    fn lz4_plain_size_ok_rejects_prefix_bomb_admits_zero_block() {
        assert!(!lz4_plain_size_ok(64 * 1024 * 1024, 106));
        assert!(!lz4_plain_size_ok(0, 16));
        assert!(lz4_plain_size_ok(16, 20));
        let src = vec![0u8; BLOCK_TARGET];
        let max = 4usize.saturating_add(lz4_flex::block::get_maximum_output_size(src.len()));
        let mut frame = vec![0u8; max];
        frame[..4].copy_from_slice(&(src.len() as u32).to_le_bytes());
        let n = lz4_flex::block::compress_into(&src, &mut frame[4..]).expect("lz4 zeros");
        frame.truncate(4 + n);
        let (size, _) = lz4_flex::block::uncompressed_size(&frame).expect("size prefix");
        assert!(
            lz4_plain_size_ok(size, frame.len()),
            "4 KiB zeros compressed to {} bytes must pass the 256x gate",
            frame.len()
        );
        let src_max = vec![0u8; LZ4_MAX_PLAIN_BLOCK];
        let max = 4usize.saturating_add(lz4_flex::block::get_maximum_output_size(src_max.len()));
        let mut frame = vec![0u8; max];
        frame[..4].copy_from_slice(&(src_max.len() as u32).to_le_bytes());
        let n = lz4_flex::block::compress_into(&src_max, &mut frame[4..]).expect("lz4 256k zeros");
        frame.truncate(4 + n);
        let (size, _) = lz4_flex::block::uncompressed_size(&frame).expect("size prefix");
        assert!(
            lz4_plain_size_ok(size, frame.len()),
            "256 KiB zeros compressed to {} bytes must pass via the absolute cap",
            frame.len()
        );
    }

    #[test]
    fn lz4_size_prefix_bomb_rejected_on_decode_and_seek() {
        let src = [b'x'; 16];
        let max = 4usize.saturating_add(lz4_flex::block::get_maximum_output_size(src.len()));
        let mut frame = vec![0u8; max];
        frame[..4].copy_from_slice(&16u32.to_le_bytes());
        let n = lz4_flex::block::compress_into(&src, &mut frame[4..]).expect("lz4");
        frame.truncate(4 + n);
        frame[..4].copy_from_slice(&(64 * 1024 * 1024u32).to_le_bytes());
        let path = Path::new("bomb.sst");
        let err = decode_block_bytes(&frame, true, false, path)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("exceeds 256x compressed length"),
            "decode must name the ceiling, got {err}"
        );
        let mut scratch = Vec::new();
        let err = seek_point_in_block_body(&frame, true, b"k", u64::MAX, &mut scratch, path)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("exceeds 256x compressed length"),
            "seek must name the ceiling, got {err}"
        );
        assert!(
            scratch.len() < 64 * 1024,
            "seek must not allocate the forged 64 MiB, scratch={}",
            scratch.len()
        );
    }

    #[test]
    fn write_sst_bulk_arrays_large_blocks_roundtrip() {
        let path = temp_path();
        let n = 2000usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:06}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n).map(|_| Bytes::from(vec![b'v'; 80])).collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        let blocks = table.data_block_count();
        // ~160 KiB of values at 4 KiB → tens of blocks (not one 256 KiB).
        assert!(
            (20..=80).contains(&blocks),
            "expected ~4 KiB blocks, got {blocks}"
        );
        assert!(table.block_crc);
        assert!(!table.payload_resident());
        drop(table);
        let re = SstTable::open_on(&StdEnv, &path).unwrap();
        assert!(matches!(
            re.get(b"k000003", u64::MAX),
            Lookup::Found(v) if v.as_ref() == [b'v'; 80]
        ));
        assert!(matches!(
            re.get(b"k001999", u64::MAX),
            Lookup::Found(v) if v.as_ref() == [b'v'; 80]
        ));
        assert!(matches!(
            re.get(b"k000500", u64::MAX),
            Lookup::Found(v) if v.as_ref() == [b'v'; 80]
        ));
        let _ = std::fs::remove_file(&path);
    }

    /// Streaming bulk SST is empty at install; first point seek promotes
    /// into the payload pool when the file fits, then CRC-skips.
    #[test]
    fn bulk_v6_point_seek_promotes_when_budget_allows() {
        let path = temp_path();
        let n = 64usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:04}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("v{i:04}").into_bytes()))
            .collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        assert!(!table.payload_resident());
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(1 << 20)));
        pool.arm();
        table.attach_payload_kit(&source, &pool);
        assert!(
            !table.payload_resident(),
            "attach must not ghost-register an empty bulk slot"
        );
        assert_eq!(pool.resident_bytes(), 0);

        let mut scratch = PointSeekScratch::default();
        reset_sst_block_crc_skipped();
        let first = table
            .point_at_seeking(b"k0003", u64::MAX, &mut scratch)
            .unwrap();
        assert!(matches!(&first, Some((_, Lookup::Found(v))) if v.as_ref() == b"v0003"));
        assert!(
            table.payload_resident(),
            "first seek must promote a file that fits the budget"
        );
        assert!(pool.resident_bytes() > 0);
        assert_eq!(
            sst_block_crc_skipped(),
            0,
            "promote verifies on first probe"
        );

        let second = table
            .point_at_seeking(b"k0003", u64::MAX, &mut scratch)
            .unwrap();
        assert_eq!(first, second);
        assert!(
            sst_block_crc_skipped() >= 1,
            "repeat probe skips CRC on the resident image"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Over-budget bulk files stay empty and still answer via 4 KiB pread.
    #[test]
    fn bulk_v6_point_seek_pread_when_budget_full() {
        let path = temp_path();
        let n = 200usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:04}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n).map(|_| Bytes::from(vec![b'v'; 80])).collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(1)));
        pool.arm();
        table.attach_payload_kit(&source, &pool);
        let mut scratch = PointSeekScratch::default();
        let found = table
            .point_at_seeking(b"k0003", u64::MAX, &mut scratch)
            .unwrap();
        assert!(matches!(&found, Some((_, Lookup::Found(v))) if v.as_ref() == [b'v'; 80]));
        assert!(
            !table.payload_resident(),
            "1-byte budget must not whole-file promote"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Repeat evicted point seek reuses the verified 4 KiB image (lookup_100).
    #[test]
    fn evicted_v6_raw_block_cache_skips_crc_on_repeat() {
        let path = temp_path();
        let n = 64usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:04}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("v{i:04}").into_bytes()))
            .collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(1)));
        pool.arm();
        table.attach_payload_kit(&source, &pool);
        let mut scratch = PointSeekScratch::default();
        reset_sst_block_crc_skipped();
        let first = table
            .point_at_seeking(b"k0003", u64::MAX, &mut scratch)
            .unwrap();
        assert_eq!(sst_block_crc_skipped(), 0, "first pread verifies");
        let second = table
            .point_at_seeking(b"k0003", u64::MAX, &mut scratch)
            .unwrap();
        assert_eq!(first, second);
        assert!(
            sst_block_crc_skipped() >= 1,
            "repeat evicted seek must skip CRC"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// RFC-0160 P0.2: evicted v6 point seek splits into pread / CRC / walk.
    #[test]
    fn get_stages_split_pread_crc_walk_on_evicted_v6() {
        let path = temp_path();
        let n = 32usize;
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:04}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("v{i:04}").into_bytes()))
            .collect();
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(1)));
        pool.arm();
        table.attach_payload_kit(&source, &pool);
        force_get_stages(true);
        reset_get_stages();
        let mut scratch = PointSeekScratch::default();
        let found = table
            .point_at_seeking(b"k0003", u64::MAX, &mut scratch)
            .unwrap();
        assert!(matches!(&found, Some((_, Lookup::Found(v))) if v.as_ref() == b"v0003"));
        let (pread, crc, walk) = take_get_stages();
        force_get_stages(false);
        assert!(pread > 0, "evicted v6 must pread, pread_ns={pread}");
        assert!(crc > 0, "first probe must CRC, crc_ns={crc}");
        assert!(walk > 0, "first probe must walk the block, walk_ns={walk}");
        let _ = std::fs::remove_file(&path);
    }

    /// RFC-0161: number the v73b miss-path tax. `point_at_cached` fully
    /// `decode_block`s every entry (`InternalKey` + `Bytes` per record) and
    /// mutex-inserts into `BlockCache`. `point_at_seeking` preads 4 KiB,
    /// CRC-verifies, and walks encoded bytes with no per-entry alloc.
    /// lookup_100's keys never repeat, so the insert never hits. Guest v73b
    /// was 5.25–5.30 ms vs v71 seeking 3.868 ms.
    #[test]
    fn cold_point_cached_miss_is_slower_than_seeking_on_evicted_v6() {
        let path = temp_path();
        let n = 1024usize;
        let val = Bytes::from(vec![b'v'; 200]);
        let keys: Vec<Bytes> = (0..n)
            .map(|i| Bytes::from(format!("k{i:06}").into_bytes()))
            .collect();
        let vals: Vec<Bytes> = vec![val; n];
        let seqs: Vec<u64> = (1..=n as u64).collect();
        let table = write_sst_bulk_arrays(&StdEnv, &path, &keys, &vals, &seqs).unwrap();
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(1)));
        pool.arm();
        table.attach_payload_kit(&source, &pool);
        assert!(
            table.payload.read().img.is_empty(),
            "payload must be evicted so both paths pread"
        );

        let mut pread_ns = Vec::with_capacity(200);
        let mut buf = vec![0u8; 4096];
        let off = table.index.first().map(|h| h.offset).unwrap_or(40);
        let len = table
            .index
            .first()
            .map(|h| h.length as usize)
            .unwrap_or(4096)
            .min(4096);
        buf.truncate(len.max(1));
        for _ in 0..200 {
            let t = Instant::now();
            source.read_range(&path, off, &mut buf).unwrap();
            pread_ns.push(t.elapsed().as_nanos() as u64);
        }
        pread_ns.sort_unstable();
        let pread_p50 = pread_ns[pread_ns.len() / 2];

        // Stride past one 4 KiB block (~15 × 200 B values) so each probe is a
        // distinct block — lookup_100's shape.
        let seek_keys: Vec<&[u8]> = (0..32).map(|i| keys[i * 20].as_ref()).collect();
        let cached_keys: Vec<&[u8]> = (0..32).map(|i| keys[i * 20 + 10].as_ref()).collect();

        let mut scratch = PointSeekScratch::default();
        force_get_stages(true);
        reset_get_stages();
        for k in &seek_keys {
            assert!(table
                .point_at_seeking(k, u64::MAX, &mut scratch)
                .unwrap()
                .is_some());
        }
        let stages = take_get_stages();
        force_get_stages(false);

        let mut scratch = PointSeekScratch::default();
        let t_seek = Instant::now();
        for k in &seek_keys {
            assert!(table
                .point_at_seeking(k, u64::MAX, &mut scratch)
                .unwrap()
                .is_some());
        }
        let seek_ns = t_seek.elapsed().as_nanos() as u64;

        // Tiny budget: every decoded insert evicts (guest 1 GiB cache is full
        // after the first Criterion samples of unique keys).
        let cache = BlockCache::with_budget_bytes(256);
        let t_cached = Instant::now();
        for k in &cached_keys {
            assert!(table
                .point_at_cached(k, u64::MAX, &cache)
                .unwrap()
                .is_some());
        }
        let cached_ns = t_cached.elapsed().as_nanos() as u64;

        eprintln!(
            "RFC-0161 miss-path: isolated_4kib_pread_p50_ns={pread_p50} \
             seeking_32_ns={seek_ns} cached_32_ns={cached_ns} \
             stages_pread_crc_walk={stages:?} cache_used={} cache_misses={} \
             n_blocks={}",
            cache.used_bytes(),
            cache.misses(),
            table.index.len()
        );
        assert!(
            cache.used_bytes() > 0 && cache.misses() > 0,
            "cached miss must decode+insert, used={} misses={}",
            cache.used_bytes(),
            cache.misses()
        );
        assert!(stages.0 > 0 && stages.1 > 0 && stages.2 > 0);
        assert!(pread_p50 > 0, "isolated pread must run");
        let _ = std::fs::remove_file(&path);
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
        // Repetitive payload: the writer's first-block probe must keep lz4
        // (v5) — a small/incompressible fixture now legitimately writes v3.
        mem.put(b"k".as_slice(), 1, Bytes::from(vec![0x5Au8; 8192]));
        let path = temp_path();
        write_sst(&path, &mem).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() >= 40 + 8, "header + at least one data block");
        let ver = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(ver, SST_VERSION, "compressible writer must emit v5");
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
            p8: SstTable::p8_window(first, 0),
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
            payload: Arc::new(parking_lot::RwLock::new(crate::cache::ResidentBody::empty())),
            payload_len: 0,
            body_on_disk: false,
            compressed_blocks: false,
            block_crc: false,
            entries: Arc::new(Mutex::new(Some(entries))),
            kit: Arc::new(parking_lot::RwLock::new(None)),
            range_tombstones: Vec::new(),
            num_entries: 5,
            max_sequence: 5,
            index: index.into(),
            key_cp: 0,
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

    /// Oracle: the accelerated `blocks_for_point` (u64 window past the
    /// index common prefix, full-memcmp fallback on window equality) must
    /// return exactly what the pre-acceleration partition_point returned,
    /// on adversarial indexes — long shared prefixes, equal-key runs
    /// (mid-key splits), keys shorter than the common prefix, embedded
    /// 0x00 bytes, and window boundaries at the key's end.
    #[test]
    fn blocks_for_point_accel_matches_plain_oracle() {
        // Verbatim copy of the pre-acceleration implementation.
        fn plain(index: &[BlockHandle], user_key: &[u8]) -> std::ops::Range<usize> {
            if index.is_empty() {
                return 0..0;
            }
            let ge = index.partition_point(|h| h.first_user_key.as_ref() < user_key);
            let start = ge.saturating_sub(1);
            let mut end = ge;
            while end < index.len() && index[end].first_user_key.as_ref() <= user_key {
                end += 1;
            }
            start..end
        }

        fn table_with(firsts: &[&[u8]]) -> SstTable {
            let mut index: Vec<BlockHandle> = firsts
                .iter()
                .map(|k| BlockHandle {
                    offset: 0,
                    length: 16,
                    first_user_key: Bytes::copy_from_slice(k),
                    p8: 0,
                })
                .collect();
            let key_cp = SstTable::derive_index_accel(&mut index);
            SstTable {
                path: PathBuf::from("/tmp/accel-oracle.sst"),
                payload: Arc::new(parking_lot::RwLock::new(crate::cache::ResidentBody::empty())),
                payload_len: 0,
                body_on_disk: false,
                compressed_blocks: false,
                block_crc: false,
                entries: Arc::new(Mutex::new(None)),
                kit: Arc::new(parking_lot::RwLock::new(None)),
                range_tombstones: Vec::new(),
                num_entries: 0,
                max_sequence: 0,
                index: index.into(),
                key_cp,
                bloom: BloomFilter::always_true(),
                smallest_user_key: None,
                largest_user_key: None,
                cf: String::new(),
            }
        }

        // Route-fold shape: every key shares "route.svc-" (10 B); entropy
        // starts inside the u64 window only when cp skips those bytes.
        let route: Vec<Vec<u8>> = (0..40)
            .map(|i| format!("route.svc-{:06}.{:08}", i / 4, i % 4).into_bytes())
            .collect();
        let route_refs: Vec<&[u8]> = route.iter().map(|v| v.as_slice()).collect();
        let cases: Vec<(Vec<&[u8]>, Vec<Vec<u8>>)> = vec![
            (
                route_refs.clone(),
                vec![
                    b"".to_vec(),
                    b"r".to_vec(),
                    b"route.svc-".to_vec(),
                    b"route.svc-000000.00000000".to_vec(),
                    b"route.svc-000009.00000003".to_vec(),
                    b"route.svc-000005.99999999".to_vec(),
                    b"route.svc-999999.99999999".to_vec(),
                    b"route.svc-000000.\x00".to_vec(),
                ],
            ),
            (
                // Equal-key run (mid-key split) + short keys + embedded NULs.
                vec![
                    &b"a"[..],
                    &b"k\x00\x00\x00\x00\x00\x00\x00\x00"[..],
                    b"k\x00\x00\x00\x00\x00\x00\x00\x00",
                    b"kz",
                    b"z",
                ],
                vec![
                    b"".to_vec(),
                    b"a".to_vec(),
                    b"ab".to_vec(),
                    b"k".to_vec(),
                    b"k\x00".to_vec(),
                    b"k\x00\x00\x00\x00\x00\x00\x00".to_vec(),
                    b"k\x00\x00\x00\x00\x00\x00\x00\x00".to_vec(),
                    b"k\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec(),
                    b"ky".to_vec(),
                    b"kz".to_vec(),
                    b"zz".to_vec(),
                ],
            ),
            (
                // Single-block and window-past-end regimes.
                vec![&b"prefix-only"[..]],
                vec![
                    b"".to_vec(),
                    b"prefix".to_vec(),
                    b"prefix-only".to_vec(),
                    b"prefix-only-longer".to_vec(),
                ],
            ),
        ];

        for (firsts, probes) in cases {
            let table = table_with(&firsts);
            if firsts == route_refs {
                // "route.svc-00000" is shared by every key (svc < 10 keeps
                // 5 leading zeros): the window must start past it.
                assert!(table.key_cp >= 10, "route cp sanity: {}", table.key_cp);
            }
            for p in probes {
                assert_eq!(
                    table.blocks_for_point(&p),
                    plain(&table.index, &p),
                    "accel vs plain: index={firsts:?} probe={p:?} cp={}",
                    table.key_cp
                );
            }
        }
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
    fn l0_flush_roundtrip() {
        let mut mem = MemTable::new();
        // Repetitive payloads so the first-block probe keeps lz4 (v5).
        mem.put(Bytes::from_static(b"a"), 1, Bytes::from(vec![0x61u8; 4096]));
        mem.put(Bytes::from_static(b"b"), 2, Bytes::from(vec![0x62u8; 4096]));
        let path = temp_path();
        let table = write_l0_sst(&StdEnv, &path, &mem, false).unwrap();
        assert!(table.block_crc, "compressible L0 flush is v5");
        assert_eq!(
            table.get(b"a", 10),
            Lookup::Found(Bytes::from(vec![0x61u8; 4096]))
        );
        let re = SstTable::open(&path).unwrap();
        assert_eq!(
            re.get(b"b", 10),
            Lookup::Found(Bytes::from(vec![0x62u8; 4096]))
        );
        let _ = std::fs::remove_file(&path);
    }

    /// RFC-0159 P1.1: incompressible payloads (bulk-chunk shapes) make the
    /// first block decide v3 raw for the whole file — no lz4 CPU, no block
    /// CRCs, byte-identical round trip.
    #[test]
    fn incompressible_flush_writes_v3_raw() {
        let mut rng = 0x5EED_5EED_5EED_5EEDu64;
        let mut mem = MemTable::new();
        for i in 0..2048u32 {
            let mut value = vec![0u8; 200];
            for chunk in value.chunks_mut(8) {
                rng ^= rng >> 12;
                rng ^= rng << 25;
                rng ^= rng >> 27;
                chunk.copy_from_slice(&rng.to_le_bytes()[..chunk.len()]);
            }
            mem.put(
                Bytes::from(format!("key-{i:06}")),
                i as u64 + 1,
                Bytes::from(value),
            );
        }
        let path = temp_path();
        let table = write_l0_sst(&StdEnv, &path, &mem, false).unwrap();
        assert!(
            !table.compressed_blocks && !table.block_crc,
            "incompressible flush must skip lz4 (v3 raw)"
        );
        assert_eq!(table.len(), mem.len());
        for i in (0..2048u32).step_by(97) {
            let key = format!("key-{i:06}");
            let want = match mem.get(key.as_bytes(), u64::MAX) {
                Lookup::Found(v) => v.clone(),
                other => panic!("memtable missing {key}: {other:?}"),
            };
            assert_eq!(
                table.get(key.as_bytes(), u64::MAX),
                Lookup::Found(want),
                "key-{i:06}"
            );
        }
        let re = SstTable::open(&path).unwrap();
        assert_eq!(re.len(), mem.len());
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

    /// RFC-0042 v18: attach a budget-0 pool (armed) — every registration
    /// evicts — then confirm reads are byte-identical from file.
    fn zero_pool_kit() -> (
        Arc<dyn crate::env::SstFileSource>,
        Arc<crate::cache::SstPayloadPool>,
    ) {
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(0)));
        pool.arm();
        (source, pool)
    }

    #[test]
    fn evicted_payload_serves_identical_blocks() {
        let path = temp_path();
        let mut mem = MemTable::new();
        for i in 0..300u32 {
            let key = format!("key-{i:05}").into_bytes();
            let val = vec![(i % 251) as u8; 40];
            mem.put(key, u64::from(i), val);
        }
        let table = write_sst(&path, &mem).unwrap();
        assert!(table.block_count() >= 3, "need multiple blocks");
        assert!(table.block_crc, "v5 writer default");
        let expected_all = table.entries_cloned();
        let Lookup::Found(expected_get) = table.get(b"key-00150", 1_000) else {
            panic!("baseline get must hit");
        };

        let (source, pool) = zero_pool_kit();
        table.attach_payload_kit(&source, &pool);
        assert!(!table.payload_resident(), "budget 0 must evict");
        assert_eq!(pool.resident_bytes(), 0);
        assert!(
            table.payload_bytes() > 0,
            "evicted table still knows its file-body length"
        );

        // Identical answers from file (per-block read + CRC).
        assert_eq!(table.get(b"key-00150", 1_000), Lookup::Found(expected_get));
        for bi in 0..table.block_count() {
            let from_file = table.decode_block(bi).unwrap();
            let from_payload = expected_all
                .iter()
                .filter(|(k, _)| {
                    table
                        .index
                        .get(bi)
                        .is_some_and(|h| h.first_user_key.as_ref() <= k.user_key.as_ref())
                        && table
                            .index
                            .get(bi + 1)
                            .is_none_or(|n| k.user_key.as_ref() < n.first_user_key.as_ref())
                })
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(from_file, from_payload, "block {bi}");
        }
        let mut stream = table.iter_internal_streaming();
        let mut streamed = Vec::new();
        while let Some(e) = stream.next_entry().unwrap() {
            streamed.push(e);
        }
        assert_eq!(streamed, expected_all);
        let _ = std::fs::remove_file(&path);
    }

    /// RFC-0077 P2 parity: the encoded-block seek returns exactly what the
    /// decoded-block path returns — newest version ≤ snapshot, tombstones,
    /// misses, empty keys, and a version run wide enough to span blocks —
    /// both with the payload resident and evicted (per-block file reads).
    #[test]
    fn point_seek_matches_decoded_path() {
        let path = temp_path();
        let mut mem = MemTable::new();
        // 40 keys × 3 versions × 180 B values → several 4 KiB blocks.
        for i in 0..40u32 {
            let key = format!("key-{i:05}").into_bytes();
            for ver in 1..=3u64 {
                let val = vec![b'a' + ((i % 7) + (ver % 5) as u32) as u8; 180];
                mem.put(key.clone(), u64::from(i) * 10 + ver, val);
            }
        }
        // Tombstone as the newest version of key-00007, older put at seq 5.
        mem.put(b"key-00007".as_slice(), 5, b"old".as_slice());
        mem.delete(b"key-00007".as_slice(), 99_999);
        // One user key whose version run must span a block boundary.
        for ver in 10..30u64 {
            mem.put(b"wide-key".as_slice(), 100_000 + ver, vec![b'w'; 512]);
        }
        let table = write_sst(&path, &mem).unwrap();
        assert!(table.block_count() >= 4, "need several blocks");

        let mut scratch = PointSeekScratch::default();
        let mut keys: Vec<Vec<u8>> = (0..40u32)
            .map(|i| format!("key-{i:05}").into_bytes())
            .collect();
        keys.push(b"wide-key".to_vec());
        keys.push(b"absent".to_vec());
        keys.push(b"key-00006\x00".to_vec());
        keys.push(Vec::new());
        let mut check = |table: &SstTable, keys: &[Vec<u8>]| {
            for key in keys {
                for snap in [0u64, 1, 12, 37, 100_019, u64::MAX] {
                    let decoded = table.point_at(key, snap);
                    let sought = table
                        .point_at_seeking(key, snap, &mut scratch)
                        .unwrap_or_else(|e| panic!("seek {key:?}@{snap}: {e}"));
                    assert_eq!(sought, decoded, "seek {key:?}@{snap}");
                }
            }
        };
        check(&table, &keys);
        // Same answers once the payload evicts (per-block file reads + CRC).
        let (source, pool) = zero_pool_kit();
        table.attach_payload_kit(&source, &pool);
        assert!(!table.payload_resident());
        check(&table, &keys);
        let _ = std::fs::remove_file(&path);
    }

    /// RFC-0077 P2 fail-closed: a block whose body no longer matches its
    /// CRC32C refuses the seek — a corrupt block must never read as a miss.
    /// The same fault also trips the legacy `decode_block` gate.
    #[test]
    fn point_seek_crc_mismatch_fails_closed() {
        let path = temp_path();
        let mut mem = MemTable::new();
        for i in 0..200u32 {
            let key = format!("key-{i:05}").into_bytes();
            mem.put(key, u64::from(i), vec![(i % 251) as u8; 60]);
        }
        let table = write_sst(&path, &mem).unwrap();
        assert!(table.block_count() >= 2, "need multiple blocks");
        let mut scratch = PointSeekScratch::default();
        assert!(
            table
                .point_at_seeking(b"key-00000", u64::MAX, &mut scratch)
                .unwrap()
                .is_some(),
            "clean seek must hit"
        );

        // Flip one byte inside the first block's body (not its CRC trailer).
        let h0 = table.index[0].clone();
        {
            let mut g = table.payload.write();
            let mut body = g.img.as_ref().to_vec();
            body[h0.offset as usize + 1] ^= 0xff;
            *g = crate::cache::ResidentBody::from_image(Arc::from(body.into_boxed_slice()));
        }
        let err = table
            .point_at_seeking(b"key-00000", u64::MAX, &mut scratch)
            .unwrap_err();
        assert!(
            err.to_string().contains("CRC"),
            "block CRC must fail the seek; got {err:?}"
        );
        assert!(table.decode_block(0).is_err(), "legacy gate trips too");
        let _ = std::fs::remove_file(&path);
    }

    /// Verified-residency marks (RFC-0077): the first resident probe verifies
    /// and marks, later probes skip the CRC re-run, and any payload swap
    /// installs fresh marks — so a swapped image re-verifies (and a rotten one
    /// fails closed instead of reading stale-verified bytes).
    #[test]
    fn point_seek_verified_marks_skip_crc_and_invalidate_on_swap() {
        let path = temp_path();
        let mut mem = MemTable::new();
        for i in 0..200u32 {
            let key = format!("key-{i:05}").into_bytes();
            mem.put(key, u64::from(i), vec![(i % 251) as u8; 60]);
        }
        let table = write_sst(&path, &mem).unwrap();
        assert!(table.block_count() >= 2, "need multiple blocks");
        let mut scratch = PointSeekScratch::default();

        reset_sst_block_crc_skipped();
        let first = table
            .point_at_seeking(b"key-00000", u64::MAX, &mut scratch)
            .unwrap();
        assert_eq!(sst_block_crc_skipped(), 0, "first probe verifies, no skip");

        let second = table
            .point_at_seeking(b"key-00000", u64::MAX, &mut scratch)
            .unwrap();
        assert_eq!(
            sst_block_crc_skipped(),
            table.blocks_for_point(b"key-00000").count(),
            "repeat probe skips CRC per probed block"
        );
        assert_eq!(first, second, "skipped probe answers identically");

        // Swapping in a fresh Arc of the SAME bytes must drop the marks.
        reset_sst_block_crc_skipped();
        let img_copy: Arc<[u8]> = Arc::from(table.payload.read().img.as_ref().to_vec());
        *table.payload.write() = crate::cache::ResidentBody::from_image(img_copy);
        let third = table
            .point_at_seeking(b"key-00000", u64::MAX, &mut scratch)
            .unwrap();
        assert_eq!(sst_block_crc_skipped(), 0, "swap invalidates marks");
        assert_eq!(first, third);

        // A rotten swapped image must fail closed, not read as verified.
        let h0 = table.index[0].clone();
        {
            let mut g = table.payload.write();
            let mut body = g.img.as_ref().to_vec();
            body[h0.offset as usize + 1] ^= 0xff;
            *g = crate::cache::ResidentBody::from_image(Arc::from(body.into_boxed_slice()));
        }
        let err = table
            .point_at_seeking(b"key-00000", u64::MAX, &mut scratch)
            .unwrap_err();
        assert!(
            err.to_string().contains("CRC"),
            "rotten swap must fail closed; got {err:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn evicted_payload_without_kit_fails_closed() {
        let path = temp_path();
        let mut mem = MemTable::new();
        mem.put(&b"k"[..], 1, &b"v"[..]);
        let table = write_sst(&path, &mem).unwrap();
        // Free-standing table: force-clear the slot, no kit attached.
        *table.payload.write() = crate::cache::ResidentBody::empty();
        let err = table.decode_block(0).unwrap_err();
        assert!(
            err.to_string().contains("file source"),
            "want loud no-source error, got {err:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn evicted_v3_reloads_whole_body_and_rejects_bitrot() {
        let path = temp_path();
        let mut mem = MemTable::new();
        for i in 0..40u32 {
            let key = format!("v3-{i:03}").into_bytes();
            mem.put(key, u64::from(i), &b"payload-value"[..]);
        }
        // Private writer with compress=false = SST v3, no per-block CRC.
        // (write_l0_sst moved to v5 in v19, so v3 needs the private path.)
        let table = write_sst_try_sorted_opts(
            &StdEnv,
            &path,
            mem.iter_internal().map(|(k, v)| Ok((k.clone(), v.clone()))),
            mem.len(),
            true,
            false,
        )
        .unwrap();
        assert!(table.is_lazy());
        assert!(!table.block_crc, "v3 has no per-block CRC");
        let expected = table.entries_cloned();
        // Drop the decoded-entries cache warmed above so the reload test
        // actually walks the payload path instead of answering from cache.
        *table.entries.lock() = None;

        // Pool big enough to hold this file: a budget-0 pool would re-evict
        // the reload the instant `ensure_payload` re-registers it.
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(crate::env::EnvSource(StdEnv));
        let pool = Arc::new(crate::cache::SstPayloadPool::with_budget(Some(1 << 20)));
        pool.arm();
        table.attach_payload_kit(&source, &pool);
        assert!(table.payload_resident(), "fits the budget, stays resident");
        // Force eviction by hand (pool is at no pressure).
        *table.payload.write() = crate::cache::ResidentBody::empty();
        assert!(!table.payload_resident());
        let reloaded = table.materialize_entries().unwrap();
        assert_eq!(reloaded, expected, "whole-body reload must decode equally");
        // ≤v4 reload makes the payload resident again (file CRC re-verified).
        assert!(table.payload_resident());

        // Bitrot after eviction must refuse, never decode garbage.
        *table.payload.write() = crate::cache::ResidentBody::empty();
        let raw = std::fs::read(&path).unwrap();
        let mut corrupt = raw.clone();
        corrupt[60] ^= 0x40;
        std::fs::write(&path, &corrupt).unwrap();
        let err = table.decode_block(0).unwrap_err();
        assert!(
            err.to_string().contains("CRC mismatch"),
            "corrupt reload must fail closed, got {err:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// v21b regression: a compressed L0 flush of highly repetitive values
    /// packs more entries than the uncompressed min-size arithmetic allows.
    /// The entry-count check rejected the freshly written file, so
    /// `materialize_parked_once` errored (and retried the same parked
    /// memtable forever — compat auto-reclaim livelock, every put waiting
    /// out the 30 s flush-debt ceiling).
    #[test]
    fn compressed_repetitive_flush_passes_entry_count_check() {
        let path = temp_path();
        let mut mem = MemTable::new();
        // Same shape as the compat auto-reclaim livelock: a few hot keys,
        // version piles, near-identical values. 600 versions per key.
        let val = vec![b'v'; 100];
        for i in 0..600u32 {
            mem.put(b"hot".as_slice(), u64::from(i) * 2 + 1, val.clone());
            let seq8 = format!("{i:08}").into_bytes();
            mem.put(b"hot2".as_slice(), u64::from(i) * 2 + 2, seq8);
        }
        let table = write_l0_sst(&StdEnv, &path, &mem, true).unwrap();
        assert!(table.compressed_blocks, "L0 flush must write v5 blocks");
        assert_eq!(table.len(), 1200);
        let file_len = std::fs::metadata(&path).unwrap().len() as usize;
        assert!(
            file_len / MIN_ENCODED_ENTRY + 1 < 1200,
            "fixture must cross the old uncompressed floor: {file_len} bytes / 1200 entries"
        );
        // Reopen from disk: the header count vs file-size check must pass
        // for a compressed body, and the table must read back whole.
        let reopened = SstTable::open(&path).unwrap();
        assert_eq!(reopened.len(), 1200);
        assert_eq!(reopened.cached_entries_count(), 0);
        let mut hot_vals = 0usize;
        let mut hot2_vals = 0usize;
        let mut it = reopened.iter_internal_streaming();
        while let Some((k, v)) = it.next_entry().unwrap() {
            match k.user_key.as_ref() {
                b"hot" => {
                    assert_eq!(v.as_ref(), val.as_slice());
                    hot_vals += 1;
                }
                b"hot2" => {
                    assert_eq!(v.len(), 8);
                    hot2_vals += 1;
                }
                other => panic!("unexpected key {other:?}"),
            }
        }
        assert_eq!(hot_vals, 600);
        assert_eq!(hot2_vals, 600);
        let _ = std::fs::remove_file(&path);
    }
}
