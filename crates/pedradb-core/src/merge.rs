//! Merge MemTable ∪ SST layers for point/range reads with MVCC visibility.
//!
//! Given versioned entries ordered by [`InternalKey`] (user key ascending,
//! sequence descending), emit one live value per user key at a snapshot.
//! Supports range tombstones ([`ValueType::RangeDeletion`]) and a streaming
//! merge path that does not require materialising the full keyspace first.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};
use std::ops::Bound;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::OnceLock;
use std::time::Instant;

use bytes::Bytes;

use crate::error::Result;
use crate::key::{InternalKey, SequenceNumber, ValueType};

/// `PEDRA_SCAN_DIAG=1` arms [`Db::scan_at_raw`]'s periodic SCANDIAG print;
/// read once per process.
pub(crate) fn scan_diag_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PEDRA_SCAN_DIAG").is_some())
}

/// Candidate rows examined by `next_window_kv` (Some returns only).
pub(crate) static SCAN_DIAG_ROWS: AtomicU64 = AtomicU64::new(0);

/// Nanoseconds spent inside `next_window_kv` (includes block loads on
/// cache miss, which happen under the stream's `next`).
pub(crate) static SCAN_DIAG_ROW_NS: AtomicU64 = AtomicU64::new(0);

/// Rows emitted from the single-live-stream fast path (diag only).
pub(crate) static SCAN_DIAG_SINGLE_ROWS: AtomicU64 = AtomicU64::new(0);

/// Streams retired early because their head passed `end` (diag only).
pub(crate) static SCAN_DIAG_STREAM_EVICTS: AtomicU64 = AtomicU64::new(0);

/// One user-visible key/value after MVCC filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleKv {
    /// User key.
    pub key: Bytes,
    /// Value at snapshot.
    pub value: Bytes,
}

/// Newest version of a user key in a scan window, before keep/drop.
///
/// `snapshot_live` is [`visible_at`] of that version (`kind` + covering
/// range tombstone). The iterator window (RFC-0151) calls
/// [`iter_window_keep`] on this bit — not a constant live-put.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowKv {
    /// User key.
    pub key: Bytes,
    /// Payload of the winning version (empty on a deletion).
    pub value: Bytes,
    /// [`visible_at(kind, range_hidden)`] for this version.
    pub snapshot_live: bool,
}

/// A range tombstone covering `[start, end)` at `sequence`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeTombstone {
    /// Inclusive start user key.
    pub start: Bytes,
    /// Exclusive end user key.
    pub end: Bytes,
    /// Sequence of the range delete.
    pub sequence: SequenceNumber,
}

/// Half-open cover `[start, end)` (F30 OCC / range delete).
#[must_use]
pub fn range_tombstone_covers(start: &[u8], end: &[u8], key: &[u8]) -> bool {
    key >= start && key < end
}

/// AS-IS F30: only the range **start** conflicts (misses covering deletes).
#[must_use]
pub fn range_tombstone_covers_as_is(start: &[u8], _end: &[u8], key: &[u8]) -> bool {
    key == start
}

/// Whether the winning version at a snapshot is live (RFC-0150 P1).
///
/// Candidate versions already satisfy `sequence <= snapshot` (newest first).
/// A `Value` is live unless a covering range tombstone with `t.seq > point_seq`
/// hides it. `Deletion` / `RangeDeletion` are not live.
#[must_use]
pub fn visible_at(kind: ValueType, range_hidden: bool) -> bool {
    match kind {
        ValueType::Value => !range_hidden,
        ValueType::Deletion | ValueType::RangeDeletion => false,
    }
}

/// AS-IS: never hide (deleted / range-covered keys scan as live).
#[must_use]
pub fn visible_at_as_is(_kind: ValueType, _range_hidden: bool) -> bool {
    true
}

/// Emit a merge/iterator window row only when snapshot merge marked it live.
///
/// Production [`StreamingVisibleIter`] / [`visible_range`] call this with
/// [`visible_at`]. AS-IS keeps a hidden version (scan leak).
#[must_use]
pub fn iter_window_keep(snapshot_live: bool) -> bool {
    snapshot_live
}

/// AS-IS scan leak: emit a deleted / range-covered version.
#[must_use]
pub fn iter_window_keep_as_is(_snapshot_live: bool) -> bool {
    true
}

impl RangeTombstone {
    /// Whether `user_key` is covered by this tombstone.
    #[must_use]
    pub fn covers(&self, user_key: &[u8]) -> bool {
        range_tombstone_covers(self.start.as_ref(), self.end.as_ref(), user_key)
    }
}

/// Whether `user_key` falls within `[start, end)` style bounds.
#[must_use]
pub fn user_key_in_range(user_key: &[u8], start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    let after_start = match start {
        Bound::Unbounded => true,
        Bound::Included(s) => user_key >= s,
        Bound::Excluded(s) => user_key > s,
    };
    let before_end = match end {
        Bound::Unbounded => true,
        Bound::Included(e) => user_key <= e,
        Bound::Excluded(e) => user_key < e,
    };
    after_start && before_end
}

/// True when `user_key` sits past `end`: every later key of a sorted stream
/// is then out of range too, so the stream can be retired early.
#[must_use]
pub fn past_end(user_key: &[u8], end: Bound<&[u8]>) -> bool {
    match end {
        Bound::Unbounded => false,
        Bound::Included(e) => user_key > e,
        Bound::Excluded(e) => user_key >= e,
    }
}

/// Extract range tombstones visible at `snapshot` from a stream of versions.
#[must_use]
pub fn collect_range_tombstones(
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    snapshot: SequenceNumber,
) -> Vec<RangeTombstone> {
    let mut out = Vec::new();
    for (ikey, value) in entries {
        if ikey.kind != ValueType::RangeDeletion || ikey.sequence > snapshot {
            continue;
        }
        out.push(RangeTombstone {
            start: ikey.user_key,
            end: value,
            sequence: ikey.sequence,
        });
    }
    out
}

/// Whether a point version at `point_seq` for `user_key` is hidden by a range del.
#[must_use]
pub fn range_deleted(
    user_key: &[u8],
    point_seq: SequenceNumber,
    tombstones: &[RangeTombstone],
) -> bool {
    tombstones
        .iter()
        .any(|t| t.covers(user_key) && t.sequence > point_seq)
}

/// Merge version streams and return visible puts in user-key order.
///
/// `entries` must be iterable in any order; they are sorted via [`BTreeMap`].
/// For each user key, the newest version with `sequence <= snapshot` wins;
/// deletions and covering range tombstones hide the key.
pub fn visible_range(
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    snapshot: SequenceNumber,
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
) -> Vec<VisibleKv> {
    visible_range_limited(entries, snapshot, start, end, None)
}

/// Like [`visible_range`], but stops after `limit` live keys when `Some`.
pub fn visible_range_limited(
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    snapshot: SequenceNumber,
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
    limit: Option<usize>,
) -> Vec<VisibleKv> {
    let mut map: BTreeMap<InternalKey, Bytes> = BTreeMap::new();
    let mut range_dels = Vec::new();
    for (ikey, value) in entries {
        if ikey.sequence > snapshot {
            continue;
        }
        if ikey.kind == ValueType::RangeDeletion {
            // Keep all range dels that might cover keys in range (start key may
            // be before `start` bound).
            range_dels.push(RangeTombstone {
                start: ikey.user_key,
                end: value,
                sequence: ikey.sequence,
            });
            continue;
        }
        if !user_key_in_range(ikey.user_key.as_ref(), start, end) {
            continue;
        }
        map.insert(ikey, value);
    }

    let mut out = Vec::new();
    let mut iter = map.into_iter().peekable();
    while let Some((ikey, value)) = iter.next() {
        if let Some(max) = limit {
            if out.len() >= max {
                break;
            }
        }
        let user_key = ikey.user_key.clone();
        let range_hidden = range_deleted(user_key.as_ref(), ikey.sequence, &range_dels);
        let snapshot_live = visible_at(ikey.kind, range_hidden);
        let live = if iter_window_keep(snapshot_live) {
            Some(VisibleKv {
                key: user_key.clone(),
                value,
            })
        } else {
            None
        };
        // Skip older versions of the same user key (map order = newest first).
        while let Some((next, _)) = iter.peek() {
            if next.user_key == user_key {
                iter.next();
            } else {
                break;
            }
        }
        if let Some(kv) = live {
            out.push(kv);
        }
    }
    out
}

/// One sorted point-key stream (RFC-0033: pulled lazily so `limit` cuts I/O).
pub type LayerStream<'a> = Box<dyn Iterator<Item = (InternalKey, Bytes)> + 'a>;

/// True when head row `a` orders before `b` in [`InternalKey`] order
/// (user_key asc, newest sequence first). Exhausted heads order last.
fn head_before(a: &Option<(InternalKey, Bytes)>, b: &Option<(InternalKey, Bytes)>) -> bool {
    match (a, b) {
        (Some((ka, _)), Some((kb, _))) => ka.cmp(kb) == Ordering::Less,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Streaming merge of **pre-sorted** internal entry streams into visible KVs.
///
/// Each stream must already be ordered by [`InternalKey`]. The iterator pulls
/// one entry at a time (O(streams) memory beyond the streams themselves) and
/// never materialises the full keyspace as a single `Vec` of all pairs.
/// When `limit` is set, later blocks of a lazy SST stream are never decoded
/// (RFC-0033 P0.3). Range tombstones must be supplied up front so a deleted
/// prefix cannot hide later live keys (G2).
pub struct StreamingVisibleIter<'a> {
    /// Min-heap of **stream indices** ordered by each stream's head row in
    /// `heads`. Sifting moves plain `usize`s; the owned head rows never
    /// move until they are emitted (the old `BinaryHeap<HeapItem>` copied
    /// key+value handles through every sift level).
    heap: Vec<usize>,
    /// Head row per stream (`None` = exhausted; never re-inserted).
    heads: Vec<Option<(InternalKey, Bytes)>>,
    streams: Vec<LayerStream<'a>>,
    snapshot: SequenceNumber,
    range_dels: Vec<RangeTombstone>,
    start: Bound<Bytes>,
    end: Bound<Bytes>,
    limit: Option<usize>,
    emitted: usize,
    /// Last user key for which we already decided visibility (skip older versions).
    /// A reused byte buffer, not a `Bytes`: cloning the emitted key was one
    /// block-Arc inc plus one dec per row inside the scan hot loop.
    skip_user: Option<Vec<u8>>,
}

impl StreamingVisibleIter<'static> {
    /// Build from sorted streams (each `Vec` sorted by [`InternalKey`]).
    ///
    /// Range tombstones are collected from all streams first (typically few).
    #[must_use]
    pub fn new(
        streams: Vec<Vec<(InternalKey, Bytes)>>,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Self {
        let mut range_dels = Vec::new();
        let mut point_streams = Vec::with_capacity(streams.len());
        for stream in streams {
            let mut points = Vec::with_capacity(stream.len());
            for (ikey, value) in stream {
                if ikey.sequence > snapshot {
                    continue;
                }
                if ikey.kind == ValueType::RangeDeletion {
                    range_dels.push(RangeTombstone {
                        start: ikey.user_key,
                        end: value,
                        sequence: ikey.sequence,
                    });
                } else {
                    points.push((ikey, value));
                }
            }
            point_streams.push(points);
        }
        let boxed: Vec<LayerStream<'static>> = point_streams
            .into_iter()
            .map(|s| Box::new(s.into_iter()) as LayerStream<'static>)
            .collect();
        StreamingVisibleIter::from_point_streams(boxed, range_dels, snapshot, start, end, limit)
    }
}

impl<'a> StreamingVisibleIter<'a> {
    /// Merge already-filtered point streams. Range tombstones are **not**
    /// taken from the streams — pass every covering tombstone in `range_dels`
    /// (including those whose start key sits before `start`).
    #[must_use]
    pub fn from_point_streams(
        mut streams: Vec<LayerStream<'a>>,
        range_dels: Vec<RangeTombstone>,
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: Option<usize>,
    ) -> Self {
        let mut heap = Vec::new();
        let mut heads = Vec::with_capacity(streams.len());
        let mut evicts = 0u64;
        for (i, it) in streams.iter_mut().enumerate() {
            let head = it.next();
            // Sorted stream: a head already past `end` can only be followed
            // by keys further past it — retire the stream before it ever
            // competes in the heap.
            let retire = match &head {
                Some((k, _)) => past_end(k.user_key.as_ref(), end),
                None => false,
            };
            if retire {
                evicts += 1;
                heads.push(None);
                continue;
            }
            heads.push(head);
            if heads[i].is_some() {
                heap.push(i);
            }
        }
        if evicts > 0 && scan_diag_enabled() {
            SCAN_DIAG_STREAM_EVICTS.fetch_add(evicts, AtomicOrdering::Relaxed);
        }
        // heap built in registration order with all Some heads: heapify once.
        let mut iter = Self {
            heap,
            heads,
            streams,
            snapshot,
            range_dels,
            start: bound_to_owned(start),
            end: bound_to_owned(end),
            limit,
            emitted: 0,
            skip_user: None,
        };
        iter.heapify();
        iter
    }

    /// Restore the min-heap invariant over `heap` (bottom-up heapify).
    fn heapify(&mut self) {
        for start in (0..self.heap.len() / 2).rev() {
            self.sift_down(start);
        }
    }

    fn head_lt(&self, a: usize, b: usize) -> bool {
        head_before(&self.heads[a], &self.heads[b])
    }

    fn sift_down(&mut self, mut hole: usize) {
        let n = self.heap.len();
        loop {
            let l = 2 * hole + 1;
            if l >= n {
                break;
            }
            let r = l + 1;
            let mut best = l;
            if r < n && self.head_lt(self.heap[r], self.heap[l]) {
                best = r;
            }
            if self.head_lt(self.heap[best], self.heap[hole]) {
                self.heap.swap(best, hole);
                hole = best;
            } else {
                break;
            }
        }
    }

    /// Push stream `i` (its head must be `Some`) and sift it up.
    fn heap_push(&mut self, i: usize) {
        let mut hole = self.heap.len();
        self.heap.push(i);
        while hole > 0 {
            let parent = (hole - 1) / 2;
            if self.head_lt(self.heap[hole], self.heap[parent]) {
                self.heap.swap(hole, parent);
                hole = parent;
            } else {
                break;
            }
        }
    }

    /// Pop the smallest stream index (`None` = all exhausted).
    fn heap_pop(&mut self) -> Option<usize> {
        let top = *self.heap.first()?;
        let last = self.heap.pop();
        if let Some(last) = last {
            if !self.heap.is_empty() {
                self.heap[0] = last;
                self.sift_down(0);
            }
        }
        Some(top)
    }

    fn in_range(&self, user_key: &[u8]) -> bool {
        let start = bound_as_ref(&self.start);
        let end = bound_as_ref(&self.end);
        user_key_in_range(user_key, start, end)
    }

    /// `past_end` against this iterator's owned `end` bound.
    fn beyond_end(&self, user_key: &[u8]) -> bool {
        past_end(user_key, bound_as_ref(&self.end))
    }

    #[inline]
    fn skips_user(&self, user_key: &[u8]) -> bool {
        match &self.skip_user {
            Some(skip) => user_key == skip.as_slice(),
            None => false,
        }
    }

    /// Record the winning key as the new skip target (reused buffer, no
    /// `Bytes` clone) and build the window row.
    fn emit(&mut self, ikey: InternalKey, value: Bytes) -> WindowKv {
        let skip = self.skip_user.get_or_insert_with(Vec::new);
        skip.clear();
        skip.extend_from_slice(ikey.user_key.as_ref());
        let range_hidden = range_deleted(ikey.user_key.as_ref(), ikey.sequence, &self.range_dels);
        let snapshot_live = visible_at(ikey.kind, range_hidden);
        WindowKv {
            key: ikey.user_key,
            value,
            snapshot_live,
        }
    }

    /// Newest version per user key, with [`WindowKv::snapshot_live`].
    ///
    /// Does **not** apply [`iter_window_keep`] — the caller (compat window
    /// or [`Iterator::next`]) decides keep/drop from that bit.
    pub fn next_window_kv(&mut self) -> Option<WindowKv> {
        // PEDRA_SCAN_DIAG aggregates per-row cost here (the compat scan
        // path consumes this via `into_window_kvs`, not `Iterator::next`).
        // Disabled = one relaxed load per row.
        if !scan_diag_enabled() {
            return self.next_window_kv_inner();
        }
        let t0 = Instant::now();
        let out = self.next_window_kv_inner();
        SCAN_DIAG_ROW_NS.fetch_add(t0.elapsed().as_nanos() as u64, AtomicOrdering::Relaxed);
        SCAN_DIAG_ROWS.fetch_add(u64::from(out.is_some()), AtomicOrdering::Relaxed);
        out
    }

    fn next_window_kv_inner(&mut self) -> Option<WindowKv> {
        // Single live stream: nothing to compete with, so skip all heap
        // work. Reached at setup when one run overlaps the range, or after
        // the other streams exhaust / retire past `end`.
        if self.heap.len() == 1 {
            let si = self.heap[0];
            let diag = scan_diag_enabled();
            loop {
                let head = self.streams[si].next();
                let cur = self.heads[si].take();
                if head.is_none() {
                    self.heap.clear();
                } else {
                    self.heads[si] = head;
                }
                let Some((ikey, value)) = cur else {
                    return None;
                };
                if ikey.sequence > self.snapshot {
                    continue;
                }
                if self.skips_user(ikey.user_key.as_ref()) {
                    continue;
                }
                if !self.in_range(ikey.user_key.as_ref()) {
                    // Sorted stream: past `end` nothing re-enters the range.
                    if self.beyond_end(ikey.user_key.as_ref()) {
                        self.heap.clear();
                        self.heads[si] = None;
                        return None;
                    }
                    continue;
                }
                if diag {
                    SCAN_DIAG_SINGLE_ROWS.fetch_add(1, AtomicOrdering::Relaxed);
                }
                return Some(self.emit(ikey, value));
            }
        }
        while let Some(si) = self.heap_pop() {
            // Refill this stream's head before deciding on the popped row so
            // the successor competes with the other streams immediately.
            let head = self.streams[si].next();
            let cur = self.heads[si].take();
            let retire = match &cur {
                // Sorted stream: this head passed `end`, so every successor
                // is out of range too — retire instead of re-pushing.
                Some((k, _)) => self.beyond_end(k.user_key.as_ref()),
                None => false,
            };
            if retire {
                if scan_diag_enabled() {
                    SCAN_DIAG_STREAM_EVICTS.fetch_add(1, AtomicOrdering::Relaxed);
                }
                // `head` is the successor of a key past `end`: drop it.
                self.heads[si] = None;
            } else {
                let retire_head = match &head {
                    Some((k, _)) => self.beyond_end(k.user_key.as_ref()),
                    None => false,
                };
                if retire_head {
                    if scan_diag_enabled() {
                        SCAN_DIAG_STREAM_EVICTS.fetch_add(1, AtomicOrdering::Relaxed);
                    }
                    self.heads[si] = None;
                } else {
                    self.heads[si] = head;
                    if self.heads[si].is_some() {
                        self.heap_push(si);
                    }
                }
            }
            let Some((ikey, value)) = cur else {
                continue;
            };
            if ikey.sequence > self.snapshot {
                continue;
            }
            if self.skips_user(ikey.user_key.as_ref()) {
                continue;
            }
            if !self.in_range(ikey.user_key.as_ref()) {
                continue;
            }
            return Some(self.emit(ikey, value));
        }
        None
    }

    /// Consume as window candidates (hidden rows included, `snapshot_live` set).
    #[must_use]
    pub fn into_window_kvs(self) -> WindowKvIter<'a> {
        WindowKvIter { inner: self }
    }
}

/// Iterator adapter over [`StreamingVisibleIter::next_window_kv`].
///
/// Yields hidden rows (`snapshot_live == false`) so a window keep can drop them.
pub struct WindowKvIter<'a> {
    inner: StreamingVisibleIter<'a>,
}

impl Iterator for WindowKvIter<'_> {
    type Item = WindowKv;

    fn next(&mut self) -> Option<WindowKv> {
        self.inner.next_window_kv()
    }
}

impl Iterator for StreamingVisibleIter<'_> {
    type Item = VisibleKv;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(max) = self.limit {
            if self.emitted >= max {
                return None;
            }
        }
        while let Some(row) = self.next_window_kv() {
            if iter_window_keep(row.snapshot_live) {
                self.emitted = self.emitted.saturating_add(1);
                return Some(VisibleKv {
                    key: row.key,
                    value: row.value,
                });
            }
        }
        None
    }
}

pub(crate) fn bound_to_owned(b: Bound<&[u8]>) -> Bound<Bytes> {
    match b {
        Bound::Unbounded => Bound::Unbounded,
        Bound::Included(s) => Bound::Included(Bytes::copy_from_slice(s)),
        Bound::Excluded(s) => Bound::Excluded(Bytes::copy_from_slice(s)),
    }
}

pub(crate) fn bound_as_ref(b: &Bound<Bytes>) -> Bound<&[u8]> {
    match b {
        Bound::Unbounded => Bound::Unbounded,
        Bound::Included(s) => Bound::Included(s.as_ref()),
        Bound::Excluded(s) => Bound::Excluded(s.as_ref()),
    }
}

/// Options for version GC during compaction (RFC-0009 P1.3 / open-items §2.1).
#[derive(Debug, Clone, Copy, Default)]
pub struct CompactGcOptions {
    /// Drop any version with `sequence < min_sequence` (coarse floor).
    pub min_sequence: SequenceNumber,
    /// If true, keep only the newest remaining version per user key
    /// (and drop a lone tombstone so the key disappears).
    pub keep_only_latest: bool,
    /// Rocks-style snapshot-safe GC: drop a superseded point version when the
    /// **next newer** version of the same key has `sequence <= oldest_snapshot`.
    ///
    /// All open read pins at/after that sequence already see the newer version
    /// (or something newer still). When `Some`, takes precedence over
    /// [`Self::keep_only_latest`] for point-key retention.
    ///
    /// `Some(MAX)` / a high watermark with no open pins ≈ latest-only for values.
    pub oldest_snapshot: Option<SequenceNumber>,
    /// Whether the compaction input covers **every** live SST down to the
    /// bottom level (F177). Dropping a point tombstone or a range tombstone
    /// is only visibility-safe when no older version of the key can survive
    /// in a file outside the input — i.e. only on a bottommost rewrite. A
    /// partial compaction (e.g. L0 → L1 leaving existing L1+ untouched) must
    /// keep tombstones, or the older version below resurrects after the
    /// merge (and durably, after reopen). Callers set this; default `false`
    /// keeps tombstones.
    pub bottommost: bool,
}

impl CompactGcOptions {
    /// Aggressive GC for single-writer DBs with no long-lived snapshots:
    /// keep only the newest version of each user key.
    #[must_use]
    pub fn latest_only() -> Self {
        Self {
            min_sequence: 0,
            keep_only_latest: true,
            oldest_snapshot: None,
            bottommost: false,
        }
    }

    /// Snapshot-safe piggyback GC (open-items §2.1 option b).
    ///
    /// `oldest` is the minimum sequence of open [`crate::db::SnapshotPin`]s,
    /// or the DB's last sequence when none are open.
    #[must_use]
    pub fn for_oldest_snapshot(oldest: SequenceNumber) -> Self {
        Self {
            min_sequence: 0,
            keep_only_latest: false,
            oldest_snapshot: Some(oldest),
            bottommost: false,
        }
    }

    /// True when any GC rewrite is requested (not a pure merge).
    #[must_use]
    pub fn requests_gc(self) -> bool {
        self.keep_only_latest || self.min_sequence > 0 || self.oldest_snapshot.is_some()
    }
}

/// Filter/merge versions for a compacted SST.
///
/// Input may be unsorted; output is sorted by [`InternalKey`].
/// Range tombstones are kept (when not GC'd) and applied to drop covered values
/// when [`CompactGcOptions::keep_only_latest`] is set.
#[must_use]
pub fn gc_compact_entries(
    entries: impl IntoIterator<Item = (InternalKey, Bytes)>,
    gc: CompactGcOptions,
) -> Vec<(InternalKey, Bytes)> {
    let mut map: BTreeMap<InternalKey, Bytes> = BTreeMap::new();
    let mut range_dels = Vec::new();
    for (ikey, value) in entries {
        if ikey.sequence < gc.min_sequence {
            continue;
        }
        if ikey.kind == ValueType::RangeDeletion {
            range_dels.push((ikey, value));
            continue;
        }
        map.insert(ikey, value);
    }

    // Snapshot-safe: walk each user key newest→oldest; drop an older
    // version when its next-newer sibling has sequence ≤ oldest_snapshot.
    if let Some(oldest) = gc.oldest_snapshot {
        return gc_snapshot_safe(map, range_dels, oldest, gc.bottommost);
    }

    if !gc.keep_only_latest {
        let mut out: Vec<_> = map.into_iter().collect();
        out.extend(range_dels);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        return out;
    }

    // Apply range dels as coverage, keep only latest point version, drop covered.
    let tombs: Vec<RangeTombstone> = range_dels
        .iter()
        .map(|(k, v)| RangeTombstone {
            start: k.user_key.clone(),
            end: v.clone(),
            sequence: k.sequence,
        })
        .collect();

    let mut out = Vec::new();
    let mut iter = map.into_iter().peekable();
    while let Some((ikey, value)) = iter.next() {
        let user_key = ikey.user_key.clone();
        let keep = match ikey.kind {
            ValueType::Value => {
                if range_deleted(user_key.as_ref(), ikey.sequence, &tombs) {
                    None
                } else {
                    Some((ikey, value))
                }
            }
            // F177: a newest point Deletion must survive a partial
            // compaction — an older version can live below the input.
            // Only a bottommost rewrite may collapse it away.
            ValueType::Deletion if !gc.bottommost => Some((ikey, value)),
            ValueType::Deletion | ValueType::RangeDeletion => None,
        };
        while let Some((next, _)) = iter.peek() {
            if next.user_key == user_key {
                iter.next();
            } else {
                break;
            }
        }
        if let Some(pair) = keep {
            out.push(pair);
        }
    }
    // Drop range tombstones under bottommost latest_only (keys already gone
    // everywhere). A partial rewrite keeps them: they still hide versions in
    // files outside the input (F177).
    if gc.bottommost {
        out
    } else {
        out.extend(range_dels);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

/// Snapshot-safe point retention + pass-through range tombstones.
fn gc_snapshot_safe(
    map: BTreeMap<InternalKey, Bytes>,
    range_dels: Vec<(InternalKey, Bytes)>,
    oldest_snapshot: SequenceNumber,
    bottommost: bool,
) -> Vec<(InternalKey, Bytes)> {
    let mut out: Vec<(InternalKey, Bytes)> = Vec::new();
    let mut iter = map.into_iter().peekable();
    while let Some((ikey, value)) = iter.next() {
        let user_key = ikey.user_key.clone();
        // Collect all versions of this user key (already newest-first via Ord).
        let mut versions = vec![(ikey, value)];
        while let Some((next, _)) = iter.peek() {
            if next.user_key == user_key {
                versions.push(iter.next().expect("peeked"));
            } else {
                break;
            }
        }
        // Newest always kept; each older drops when immediate newer.seq ≤ oldest
        // (decided by `compact_kernel::point_version_fate`, RFC-0056 P0.3).
        let mut keep: Vec<(InternalKey, Bytes)> = Vec::with_capacity(versions.len());
        for (ikey, value) in versions {
            if keep.is_empty() {
                keep.push((ikey, value));
                continue;
            }
            let newer_seq = keep.last().expect("non-empty").0.sequence;
            if crate::compact_kernel::point_version_fate(
                ikey.sequence,
                Some(newer_seq),
                oldest_snapshot,
            ) == crate::compact_kernel::VersionFate::Drop
            {
                // All open snapshots ≥ oldest see `newer` (or something newer).
                continue;
            }
            keep.push((ikey, value));
        }
        // Drop lone deletion when it is the only kept version and no snap needs
        // an older value (newest is a tombstone and nothing older survived).
        // F177: only on a bottommost rewrite — in a partial compaction an
        // older version of the key can live in a file outside the input
        // (e.g. existing L1+ untouched by compact_l0_into_l1); dropping the
        // tombstone there resurrects that version (durably, after reopen).
        // Decided by `compact_kernel::lone_tombstone_fate`.
        let lone_tombstone = keep.len() == 1 && keep[0].0.kind == ValueType::Deletion;
        if crate::compact_kernel::lone_tombstone_fate(bottommost, lone_tombstone)
            == crate::compact_kernel::VersionFate::Drop
        {
            // Tombstone only needed if some open snap is ≥ tombstone seq and
            // would otherwise see an older value we already dropped — if we
            // dropped everything under it, snaps see NotFound either way.
            // Safe to drop lone tombstones when nothing older remains.
            keep.clear();
        }
        out.extend(keep);
    }
    out.extend(range_dels);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Head of one sorted internal stream (min-heap via reversed [`Ord`]).
struct MergeHead {
    key: InternalKey,
    value: Bytes,
    src: usize,
}

impl PartialEq for MergeHead {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.src == other.src
    }
}
impl Eq for MergeHead {}
impl PartialOrd for MergeHead {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for MergeHead {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .key
            .cmp(&self.key)
            .then_with(|| other.src.cmp(&self.src))
    }
}

/// K-way merge of InternalKey-sorted streams (RFC-0037). Dedups identical keys.
pub struct KwayInternalMerge<S> {
    streams: Vec<S>,
    heap: BinaryHeap<MergeHead>,
    last: Option<InternalKey>,
}

/// Pull the next entry from a compact source.
pub trait CompactSource {
    /// Next internal pair.
    ///
    /// # Errors
    /// Source decode / I/O.
    fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>>;
}

impl CompactSource for std::vec::IntoIter<(InternalKey, Bytes)> {
    fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>> {
        Ok(Iterator::next(self))
    }
}

impl<S: CompactSource> CompactSource for KwayInternalMerge<S> {
    fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>> {
        KwayInternalMerge::next_entry(self)
    }
}

impl<S: CompactSource> KwayInternalMerge<S> {
    /// Seed the heap from each stream's first entry.
    ///
    /// # Errors
    /// A source failed to decode its first block.
    pub fn from_streams(mut streams: Vec<S>) -> Result<Self> {
        let mut heap = BinaryHeap::new();
        for src in 0..streams.len() {
            if let Some((key, value)) = streams[src].next_entry()? {
                heap.push(MergeHead { key, value, src });
            }
        }
        Ok(Self {
            streams,
            heap,
            last: None,
        })
    }

    /// Next merged pair, skipping duplicate [`InternalKey`]s.
    ///
    /// # Errors
    /// A source failed to decode.
    pub fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>> {
        loop {
            let Some(head) = self.heap.pop() else {
                return Ok(None);
            };
            match self.streams[head.src].next_entry()? {
                Some((key, value)) => self.heap.push(MergeHead {
                    key,
                    value,
                    src: head.src,
                }),
                None => {}
            }
            if self.last.as_ref() == Some(&head.key) {
                continue;
            }
            self.last = Some(head.key.clone());
            return Ok(Some((head.key, head.value)));
        }
    }
}

/// Streaming GC compaction over a k-way merge.
///
/// Applies the retention decisions of [`gc_compact_entries`] without
/// materializing the whole input — the batch path held every decoded input
/// table plus a `BTreeMap` copy, which tripled the merge footprint and
/// OOMed a 4 GiB host under sustained L0→L1 compaction (25M-entry slipstream
/// hydrate, 2026-08-31).
///
/// Equivalence with the batch path: the merge yields [`InternalKey`] order,
/// so one user key's version run arrives contiguously (sequence descending),
/// and every range tombstone that can cover key K sorts at its start key
/// ≤ K — i.e. it is already in `tombs` when K's run closes. Run decisions
/// are therefore identical to the batch map walk, and emitting survivors at
/// their stream position equals the batch's final global sort.
pub struct GcMergeSource<S: CompactSource> {
    merge: KwayInternalMerge<S>,
    gc: CompactGcOptions,
    tombs: Vec<RangeTombstone>,
    run: Vec<(InternalKey, Bytes)>,
    out: VecDeque<(InternalKey, Bytes)>,
}

impl<S: CompactSource> GcMergeSource<S> {
    /// Wrap a k-way merge with GC options.
    #[must_use]
    pub fn new(merge: KwayInternalMerge<S>, gc: CompactGcOptions) -> Self {
        Self {
            merge,
            gc,
            tombs: Vec::new(),
            run: Vec::new(),
            out: VecDeque::new(),
        }
    }

    /// Decide the buffered run (one user key) and queue its survivors.
    fn close_run(&mut self) {
        // This run's tombstones have start == this user key, so they can
        // cover this run's keys: collect them before the coverage decision
        // (the batch path builds `tombs` from every deletion, including
        // ones a bottommost rewrite later drops from the output).
        // Bottommost latest-only drops the tombstones themselves (F177
        // mirrors `gc_compact_entries`); every other mode passes them
        // through. `oldest_snapshot` takes precedence over keep-only-latest.
        let drop_tombs =
            self.gc.oldest_snapshot.is_none() && self.gc.keep_only_latest && self.gc.bottommost;
        for (ikey, value) in &self.run {
            if ikey.kind == ValueType::RangeDeletion {
                self.tombs.push(RangeTombstone {
                    start: ikey.user_key.clone(),
                    end: value.clone(),
                    sequence: ikey.sequence,
                });
            }
        }
        let user = self.run[0].0.user_key.clone();
        let mut keep = vec![false; self.run.len()];
        let points: Vec<usize> = (0..self.run.len())
            .filter(|&i| self.run[i].0.kind != ValueType::RangeDeletion)
            .collect();
        if let Some(oldest) = self.gc.oldest_snapshot {
            // `gc_snapshot_safe`: newest always kept; each older version
            // drops when the newest kept sibling has sequence <= oldest;
            // a lone bottommost tombstone collapses away.
            let mut newer_kept: Option<SequenceNumber> = None;
            for &i in &points {
                if crate::compact_kernel::point_version_fate(
                    self.run[i].0.sequence,
                    newer_kept,
                    oldest,
                ) == crate::compact_kernel::VersionFate::Drop
                {
                    continue;
                }
                keep[i] = true;
                newer_kept = Some(self.run[i].0.sequence);
            }
            let kept: Vec<usize> = points.iter().copied().filter(|&i| keep[i]).collect();
            let lone = kept.len() == 1 && self.run[kept[0]].0.kind == ValueType::Deletion;
            if crate::compact_kernel::lone_tombstone_fate(self.gc.bottommost, lone)
                == crate::compact_kernel::VersionFate::Drop
            {
                for &i in &kept {
                    keep[i] = false;
                }
            }
        } else if self.gc.keep_only_latest {
            // Keep only the newest version; drop it when a newer range
            // tombstone covers it. A point tombstone survives a partial
            // rewrite (F177) and collapses on a bottommost one.
            if let Some(&i) = points.first() {
                let ikey = &self.run[i].0;
                keep[i] = match ikey.kind {
                    ValueType::Value => !range_deleted(user.as_ref(), ikey.sequence, &self.tombs),
                    ValueType::Deletion => !self.gc.bottommost,
                    ValueType::RangeDeletion => false,
                };
            }
        } else {
            // Pure min-sequence floor: every surviving point is kept.
            for &i in &points {
                keep[i] = true;
            }
        }
        let run = std::mem::take(&mut self.run);
        for (i, (ikey, value)) in run.into_iter().enumerate() {
            if ikey.kind == ValueType::RangeDeletion {
                if !drop_tombs {
                    self.out.push_back((ikey, value));
                }
            } else if keep[i] {
                self.out.push_back((ikey, value));
            }
        }
    }
}

impl<S: CompactSource> CompactSource for GcMergeSource<S> {
    fn next_entry(&mut self) -> Result<Option<(InternalKey, Bytes)>> {
        loop {
            if let Some(pair) = self.out.pop_front() {
                return Ok(Some(pair));
            }
            match self.merge.next_entry()? {
                Some((ikey, value)) => {
                    // Same pre-filter as the batch path: the floor applies
                    // to point versions and range tombstones alike.
                    if ikey.sequence < self.gc.min_sequence {
                        continue;
                    }
                    if self
                        .run
                        .last()
                        .is_some_and(|(k, _)| k.user_key != ikey.user_key)
                    {
                        self.close_run();
                    }
                    self.run.push((ikey, value));
                }
                None => {
                    if self.run.is_empty() {
                        return Ok(None);
                    }
                    self.close_run();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::ValueType;

    fn ik(user: &[u8], seq: u64, kind: ValueType) -> InternalKey {
        InternalKey::new(Bytes::copy_from_slice(user), seq, kind)
    }

    #[test]
    fn f30_covers_interior_not_only_start() {
        assert!(range_tombstone_covers(b"a", b"z", b"m"));
        assert!(range_tombstone_covers(b"a", b"z", b"a"));
        assert!(!range_tombstone_covers(b"a", b"z", b"z"));
        assert!(!range_tombstone_covers_as_is(b"a", b"z", b"m"));
        assert!(range_tombstone_covers_as_is(b"a", b"z", b"a"));
    }

    #[test]
    fn theorem_covers_on_short_keys() {
        const A: [&[u8]; 4] = [b"a", b"m", b"y", b"z"];
        for start in A {
            for end in A {
                for key in A {
                    let d = range_tombstone_covers(start, end, key);
                    assert_eq!(d, key >= start && key < end);
                    if key != start && d {
                        assert!(!range_tombstone_covers_as_is(start, end, key));
                    }
                }
            }
        }
    }

    #[test]
    fn range_mvcc_newest_and_tombstone() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"b2")),
            (ik(b"b", 3, ValueType::Value), Bytes::from_static(b"b3")),
            (ik(b"c", 4, ValueType::Value), Bytes::from_static(b"c4")),
            (ik(b"c", 5, ValueType::Deletion), Bytes::new()),
            (ik(b"d", 6, ValueType::Value), Bytes::from_static(b"d6")),
        ];
        let got = visible_range(
            entries,
            10,
            Bound::Included(b"b".as_ref()),
            Bound::Excluded(b"d".as_ref()),
        );
        assert_eq!(
            got,
            vec![VisibleKv {
                key: Bytes::from_static(b"b"),
                value: Bytes::from_static(b"b3"),
            }]
        );
    }

    #[test]
    fn snapshot_hides_newer_versions() {
        let entries = vec![
            (ik(b"k", 1, ValueType::Value), Bytes::from_static(b"old")),
            (ik(b"k", 5, ValueType::Value), Bytes::from_static(b"new")),
        ];
        let at_3 = visible_range(entries.clone(), 3, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(at_3[0].value.as_ref(), b"old");
        let at_10 = visible_range(entries, 10, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(at_10[0].value.as_ref(), b"new");
    }

    /// Brute-force window oracle: sort every point entry into internal
    /// order, keep the first version at/below `snapshot` per user key
    /// (hidden rows included), range-filter, apply range tombstones.
    fn window_oracle(
        streams: &[Vec<(InternalKey, Bytes)>],
        range_dels: &[RangeTombstone],
        snapshot: SequenceNumber,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Vec<WindowKv> {
        let mut all: Vec<(InternalKey, Bytes)> = streams.iter().flatten().cloned().collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = Vec::new();
        let mut last: Option<Vec<u8>> = None;
        for (ikey, value) in all {
            if ikey.sequence > snapshot {
                continue;
            }
            if let Some(l) = &last {
                if ikey.user_key.as_ref() == l.as_slice() {
                    continue;
                }
            }
            if !user_key_in_range(ikey.user_key.as_ref(), start, end) {
                continue;
            }
            last = Some(ikey.user_key.to_vec());
            let hidden = range_deleted(ikey.user_key.as_ref(), ikey.sequence, range_dels);
            out.push(WindowKv {
                key: ikey.user_key,
                value,
                snapshot_live: visible_at(ikey.kind, hidden),
            });
        }
        out
    }

    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state
    }

    /// Randomized oracle for `StreamingVisibleIter::from_point_streams`:
    /// 1-4 streams, duplicate keys across streams, versions above snapshot,
    /// deletions, range tombstones, and bounds that leave whole streams
    /// past `end` (exercising setup eviction + the single-live fast path).
    #[test]
    fn streaming_merge_fast_path_random_oracle() {
        let mut seed = 0x0000_5eed_0002_u64;
        let snapshot: SequenceNumber = 8;
        for case in 0..300 {
            let n_streams = 1 + (lcg(&mut seed) % 4) as usize;
            let mut used: std::collections::HashSet<(Vec<u8>, u64)> =
                std::collections::HashSet::new();
            let mut streams: Vec<Vec<(InternalKey, Bytes)>> = Vec::new();
            for _s in 0..n_streams {
                let mut rows = Vec::new();
                let mut k = lcg(&mut seed) % 50;
                for _r in 0..(lcg(&mut seed) % 30) {
                    let key = format!(
                        "{}{}",
                        (b'a' + (k / 10) as u8) as char,
                        (b'0' + (k % 10) as u8) as char
                    );
                    let mut seq = 1 + lcg(&mut seed) % 12;
                    while !used.insert((key.clone().into_bytes(), seq)) {
                        seq = 1 + lcg(&mut seed) % 12;
                    }
                    let kind = if lcg(&mut seed) % 5 == 0 {
                        ValueType::Deletion
                    } else {
                        ValueType::Value
                    };
                    let val = Bytes::from(format!("v{case}/{seq}"));
                    rows.push((ik(key.as_bytes(), seq, kind), val));
                    k += 1 + lcg(&mut seed) % 3;
                }
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                streams.push(rows);
            }
            let (start, end) = match lcg(&mut seed) % 3 {
                0 => (Bound::Unbounded, Bound::<&[u8]>::Unbounded),
                1 => (
                    Bound::Included(b"b3".as_ref()),
                    Bound::Excluded(b"d7".as_ref()),
                ),
                _ => (
                    Bound::Excluded(b"a1".as_ref()),
                    Bound::Included(b"c0".as_ref()),
                ),
            };
            let range_dels = if lcg(&mut seed) % 3 == 0 {
                vec![RangeTombstone {
                    start: Bytes::from_static(b"b1"),
                    end: Bytes::from_static(b"b8"),
                    sequence: 5,
                }]
            } else {
                Vec::new()
            };
            let expected = window_oracle(&streams, &range_dels, snapshot, start, end);
            let boxed: Vec<LayerStream<'static>> = streams
                .iter()
                .map(|s| Box::new(s.clone().into_iter()) as LayerStream<'static>)
                .collect();
            let got: Vec<WindowKv> = StreamingVisibleIter::from_point_streams(
                boxed, range_dels, snapshot, start, end, None,
            )
            .into_window_kvs()
            .collect();
            assert_eq!(got, expected, "case {case}");
        }
    }

    struct CountingIter {
        rows: std::vec::IntoIter<(InternalKey, Bytes)>,
        nexts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Iterator for CountingIter {
        type Item = (InternalKey, Bytes);
        fn next(&mut self) -> Option<Self::Item> {
            self.nexts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.rows.next()
        }
    }

    /// A stream whose head is already past `end` must be retired at setup:
    /// one poll for the head, zero heap competition, no row drain.
    #[test]
    fn stream_past_end_retired_without_competing() {
        let a: Vec<(InternalKey, Bytes)> = (0..10)
            .map(|i| {
                (
                    ik(format!("k{i:02}").as_bytes(), 1, ValueType::Value),
                    Bytes::from(format!("a{i}")),
                )
            })
            .collect();
        let b: Vec<(InternalKey, Bytes)> = (0..10)
            .map(|i| {
                (
                    ik(format!("z{i:02}").as_bytes(), 1, ValueType::Value),
                    Bytes::from(format!("b{i}")),
                )
            })
            .collect();
        let nexts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let streams: Vec<LayerStream<'static>> = vec![
            Box::new(a.into_iter()) as LayerStream<'static>,
            Box::new(CountingIter {
                rows: b.into_iter(),
                nexts: nexts.clone(),
            }) as LayerStream<'static>,
        ];
        let iter = StreamingVisibleIter::from_point_streams(
            streams,
            Vec::new(),
            10,
            Bound::Unbounded,
            Bound::Excluded(b"m".as_ref()),
            None,
        );
        let got: Vec<WindowKv> = iter.into_window_kvs().collect();
        assert_eq!(got.len(), 10);
        assert_eq!(got[0].key.as_ref(), b"k00");
        assert_eq!(got[9].key.as_ref(), b"k09");
        assert_eq!(nexts.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    /// Interleaved streams with a cross-stream duplicate key: after the
    /// second stream exhausts, the first continues on the single-live fast
    /// path — output still matches the oracle.
    #[test]
    fn interleaved_streams_match_oracle_after_exhaustion() {
        let a = vec![
            (ik(b"k01", 5, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"k03", 3, ValueType::Value), Bytes::from_static(b"a3")),
            (ik(b"k07", 2, ValueType::Value), Bytes::from_static(b"a7")),
        ];
        let b = vec![
            (ik(b"k02", 4, ValueType::Value), Bytes::from_static(b"b2")),
            (ik(b"k03", 6, ValueType::Value), Bytes::from_static(b"b3")),
            (ik(b"k04", 1, ValueType::Value), Bytes::from_static(b"b4")),
        ];
        let streams = vec![a, b];
        let expected = window_oracle(&streams, &[], 8, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(expected.len(), 5);
        assert_eq!(expected[2].key.as_ref(), b"k03");
        assert_eq!(expected[2].value.as_ref(), b"b3");
        let boxed: Vec<LayerStream<'static>> = streams
            .iter()
            .map(|s| Box::new(s.clone().into_iter()) as LayerStream<'static>)
            .collect();
        let got: Vec<WindowKv> = StreamingVisibleIter::from_point_streams(
            boxed,
            Vec::new(),
            8,
            Bound::Unbounded,
            Bound::Unbounded,
            None,
        )
        .into_window_kvs()
        .collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn gc_latest_only_drops_history_and_tombstones() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"a", 3, ValueType::Value), Bytes::from_static(b"a3")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"b2")),
            (ik(b"b", 4, ValueType::Deletion), Bytes::new()),
        ];
        // Full-DB rewrite profile: bottommost → collapse tombstones.
        let gc = CompactGcOptions {
            bottommost: true,
            ..CompactGcOptions::latest_only()
        };
        let out = gc_compact_entries(entries, gc);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.user_key.as_ref(), b"a");
        assert_eq!(out[0].1.as_ref(), b"a3");
    }

    /// F177: a partial compaction (input does not cover every level) must
    /// KEEP point tombstones and range tombstones — dropping them lets an
    /// older version in a file outside the input resurrect.
    #[test]
    fn gc_keeps_tombstones_when_not_bottommost() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"a", 3, ValueType::Value), Bytes::from_static(b"a3")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"b2")),
            (ik(b"b", 4, ValueType::Deletion), Bytes::new()),
            (ik(b"c", 5, ValueType::Value), Bytes::from_static(b"c5")),
            (
                ik(b"c", 6, ValueType::RangeDeletion),
                Bytes::from_static(b"z"),
            ),
        ];
        // latest_only, partial input: b's newest is a Deletion → keep the
        // tombstone; the range del passes through instead of being dropped.
        let out = gc_compact_entries(entries.clone(), CompactGcOptions::latest_only());
        let kinds: Vec<(&[u8], u64, ValueType)> = out
            .iter()
            .map(|(k, _)| (k.user_key.as_ref(), k.sequence, k.kind))
            .collect();
        assert!(
            kinds.contains(&(&b"b"[..], 4, ValueType::Deletion)),
            "partial latest_only must keep the point tombstone: {kinds:?}"
        );
        assert!(
            kinds.contains(&(&b"c"[..], 6, ValueType::RangeDeletion)),
            "partial latest_only must keep the range tombstone: {kinds:?}"
        );
        // Same for the snapshot-safe profile: lone point tombstone survives.
        let out = gc_compact_entries(entries, CompactGcOptions::for_oldest_snapshot(u64::MAX));
        assert!(
            out.iter()
                .any(|(k, _)| k.user_key.as_ref() == b"b" && k.kind == ValueType::Deletion),
            "partial for_oldest_snapshot must keep the lone tombstone"
        );
    }

    #[test]
    fn gc_min_sequence_drops_old() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"a", 5, ValueType::Value), Bytes::from_static(b"a5")),
        ];
        let out = gc_compact_entries(
            entries,
            CompactGcOptions {
                min_sequence: 5,
                keep_only_latest: false,
                oldest_snapshot: None,
                bottommost: false,
            },
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.as_ref(), b"a5");
    }

    #[test]
    fn gc_snapshot_safe_keeps_history_for_open_pin() {
        // k@1 old, k@5 mid, k@10 new. Oldest pin at 5 → keep @10 and @5, drop @1.
        let entries = vec![
            (ik(b"k", 1, ValueType::Value), Bytes::from_static(b"v1")),
            (ik(b"k", 5, ValueType::Value), Bytes::from_static(b"v5")),
            (ik(b"k", 10, ValueType::Value), Bytes::from_static(b"v10")),
        ];
        let out = gc_compact_entries(entries, CompactGcOptions::for_oldest_snapshot(5));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.sequence, 10);
        assert_eq!(out[0].1.as_ref(), b"v10");
        assert_eq!(out[1].0.sequence, 5);
        assert_eq!(out[1].1.as_ref(), b"v5");
        // Pin at last write → only newest.
        let entries = vec![
            (ik(b"k", 1, ValueType::Value), Bytes::from_static(b"v1")),
            (ik(b"k", 10, ValueType::Value), Bytes::from_static(b"v10")),
        ];
        let out = gc_compact_entries(entries, CompactGcOptions::for_oldest_snapshot(10));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.as_ref(), b"v10");
    }

    #[test]
    fn range_limit_stops_early() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"1")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"2")),
            (ik(b"c", 3, ValueType::Value), Bytes::from_static(b"3")),
            (ik(b"d", 4, ValueType::Value), Bytes::from_static(b"4")),
        ];
        let got = visible_range_limited(entries, 10, Bound::Unbounded, Bound::Unbounded, Some(2));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].key.as_ref(), b"a");
        assert_eq!(got[1].key.as_ref(), b"b");
    }

    #[test]
    fn range_deletion_hides_keys_in_interval() {
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"1")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"2")),
            (ik(b"c", 3, ValueType::Value), Bytes::from_static(b"3")),
            (
                ik(b"a", 4, ValueType::RangeDeletion),
                Bytes::from_static(b"c"),
            ),
        ];
        let got = visible_range(entries, 10, Bound::Unbounded, Bound::Unbounded);
        // [a,c) deleted → only c remains
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].key.as_ref(), b"c");
    }

    #[test]
    fn streaming_matches_batch_visible() {
        let s1 = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"1")),
            (ik(b"c", 3, ValueType::Value), Bytes::from_static(b"3")),
        ];
        let s2 = vec![
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"2")),
            (ik(b"c", 4, ValueType::Value), Bytes::from_static(b"3b")),
        ];
        let batch = visible_range(
            s1.iter()
                .chain(s2.iter())
                .map(|(k, v)| (k.clone(), v.clone())),
            10,
            Bound::Unbounded,
            Bound::Unbounded,
        );
        let stream: Vec<_> =
            StreamingVisibleIter::new(vec![s1, s2], 10, Bound::Unbounded, Bound::Unbounded, None)
                .collect();
        assert_eq!(stream, batch);
    }

    #[test]
    fn kway_matches_gc_concat_default() {
        let s1 = vec![
            (ik(b"a", 2, ValueType::Value), Bytes::from_static(b"a2")),
            (ik(b"b", 1, ValueType::Value), Bytes::from_static(b"b1")),
        ];
        let s2 = vec![
            (ik(b"a", 2, ValueType::Value), Bytes::from_static(b"a2")),
            (ik(b"c", 1, ValueType::Value), Bytes::from_static(b"c1")),
        ];
        let s3 = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"a1")),
            (ik(b"b", 3, ValueType::Deletion), Bytes::new()),
            (
                ik(b"x", 4, ValueType::RangeDeletion),
                Bytes::from_static(b"z"),
            ),
        ];
        let concat: Vec<_> = s1
            .iter()
            .chain(s2.iter())
            .chain(s3.iter())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let expected = gc_compact_entries(concat, CompactGcOptions::default());
        let mut merge =
            KwayInternalMerge::from_streams(vec![s1.into_iter(), s2.into_iter(), s3.into_iter()])
                .unwrap();
        let mut got = Vec::new();
        while let Some(pair) = merge.next_entry().unwrap() {
            got.push(pair);
        }
        assert_eq!(got, expected);
    }

    /// Randomized cross-validation: the streaming GC source must reproduce
    /// `gc_compact_entries` exactly (same survivors, same order) across the
    /// whole option matrix. Small key space forces multi-version runs and
    /// overlapping range tombstones.
    #[test]
    fn gc_merge_source_matches_batch_across_option_matrix() {
        let keys: [&[u8]; 5] = [b"a", b"b", b"c", b"d", b"e"];
        let mut state = 0x243F_6A88_85A3_08D3u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state
        };
        let mut all: Vec<(InternalKey, Bytes)> = Vec::new();
        let mut seq = 0u64;
        for _ in 0..400 {
            seq += 1 + next() % 3;
            let idx = (next() % keys.len() as u64) as usize;
            let kind = match next() % 10 {
                0..=1 => ValueType::Deletion,
                2..=3 => ValueType::RangeDeletion,
                _ => ValueType::Value,
            };
            let end = if idx + 1 < keys.len() {
                keys[idx + 1 + (next() % (keys.len() - idx - 1) as u64) as usize]
            } else {
                b"z"
            };
            let value = if kind == ValueType::RangeDeletion {
                Bytes::copy_from_slice(end)
            } else {
                Bytes::from(format!("v{seq}"))
            };
            all.push((ik(keys[idx], seq, kind), value));
        }
        all.sort_by(|a, b| a.0.cmp(&b.0));
        // Deal sorted entries round-robin into three sorted streams so the
        // k-way merge interleaves all of them.
        let mut streams: [Vec<(InternalKey, Bytes)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (i, pair) in all.iter().enumerate() {
            streams[i % 3].push(pair.clone());
        }
        let variants = [
            CompactGcOptions::default(),
            CompactGcOptions {
                min_sequence: 300,
                ..CompactGcOptions::default()
            },
            CompactGcOptions::latest_only(),
            CompactGcOptions {
                bottommost: true,
                ..CompactGcOptions::latest_only()
            },
            CompactGcOptions::for_oldest_snapshot(5),
            CompactGcOptions {
                bottommost: true,
                ..CompactGcOptions::for_oldest_snapshot(5)
            },
            CompactGcOptions::for_oldest_snapshot(u64::MAX),
            // Both flags set: oldest_snapshot takes precedence, and the
            // tombstone-drop condition must not fire in that branch.
            CompactGcOptions {
                bottommost: true,
                keep_only_latest: true,
                ..CompactGcOptions::for_oldest_snapshot(7)
            },
        ];
        for gc in variants {
            let expected = gc_compact_entries(all.clone(), gc);
            let merge = KwayInternalMerge::from_streams(vec![
                streams[0].clone().into_iter(),
                streams[1].clone().into_iter(),
                streams[2].clone().into_iter(),
            ])
            .unwrap();
            let mut src = GcMergeSource::new(merge, gc);
            let mut got = Vec::new();
            while let Some(pair) = src.next_entry().unwrap() {
                got.push(pair);
            }
            assert_eq!(got, expected, "streaming GC diverged for {gc:?}");
        }
    }

    /// A bottommost latest-only rewrite drops the range tombstone from the
    /// output but must still use it as coverage for the point it hides
    /// (the batch path builds `tombs` before deciding tombstone drops).
    #[test]
    fn gc_stream_bottommost_dropped_tombstone_still_covers() {
        let entries = |bottommost: bool| {
            let stream = vec![
                (
                    ik(b"k", 9, ValueType::RangeDeletion),
                    Bytes::from_static(b"z"),
                ),
                (ik(b"k", 5, ValueType::Value), Bytes::from_static(b"v5")),
            ];
            let gc = CompactGcOptions {
                bottommost,
                ..CompactGcOptions::latest_only()
            };
            let expected = gc_compact_entries(stream.clone(), gc);
            let mut src = GcMergeSource::new(
                KwayInternalMerge::from_streams(vec![stream.into_iter()]).unwrap(),
                gc,
            );
            let mut got = Vec::new();
            while let Some(pair) = src.next_entry().unwrap() {
                got.push(pair);
            }
            assert_eq!(got, expected, "mismatch for bottommost={bottommost}");
            // The covered value is gone in both modes; only the partial
            // rewrite keeps (and passes through) the tombstone itself.
            if bottommost {
                assert!(got.is_empty());
            } else {
                assert_eq!(got.len(), 1);
                assert_eq!(got[0].0.kind, ValueType::RangeDeletion);
            }
        };
        entries(true);
        entries(false);
    }

    #[test]
    fn visible_at_put_delete_range_del() {
        assert!(visible_at(ValueType::Value, false));
        assert!(!visible_at(ValueType::Value, true));
        assert!(!visible_at(ValueType::Deletion, false));
        assert!(!visible_at(ValueType::RangeDeletion, false));
        assert!(
            visible_at_as_is(ValueType::Deletion, true),
            "AS-IS dente: never hides"
        );
        let entries = vec![
            (ik(b"a", 1, ValueType::Value), Bytes::from_static(b"1")),
            (ik(b"b", 2, ValueType::Value), Bytes::from_static(b"2")),
            (ik(b"c", 3, ValueType::Value), Bytes::from_static(b"3")),
            (ik(b"b", 4, ValueType::Deletion), Bytes::new()),
            (
                ik(b"a", 5, ValueType::RangeDeletion),
                Bytes::from_static(b"c"),
            ),
        ];
        let got = visible_range(entries, 10, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].key.as_ref(), b"c");
    }

    #[test]
    fn visible_at_on_live_range_del_is_not_ok() {
        assert!(!visible_at(ValueType::Value, true));
        assert!(
            visible_at_as_is(ValueType::Value, true),
            "AS-IS dente: hidden value scans live"
        );
        let entries = vec![
            (ik(b"b", 1, ValueType::Value), Bytes::from_static(b"1")),
            (
                ik(b"a", 5, ValueType::RangeDeletion),
                Bytes::from_static(b"c"),
            ),
        ];
        let got = visible_range(entries, 10, Bound::Unbounded, Bound::Unbounded);
        assert!(got.is_empty(), "covering range-del hides b");
        let hidden = range_deleted(
            b"b",
            1,
            &[RangeTombstone {
                start: Bytes::from_static(b"a"),
                end: Bytes::from_static(b"c"),
                sequence: 5,
            }],
        );
        assert!(hidden);
        assert!(!visible_at(ValueType::Value, hidden));
    }

    #[test]
    fn f30_as_is_misses_mid_range_key() {
        assert!(range_tombstone_covers(b"a", b"c", b"b"));
        assert!(
            !range_tombstone_covers_as_is(b"a", b"c", b"b"),
            "AS-IS F30 only matches the range start"
        );
        assert!(range_tombstone_covers_as_is(b"a", b"c", b"a"));
        let hidden = range_deleted(
            b"b",
            1,
            &[RangeTombstone {
                start: Bytes::from_static(b"a"),
                end: Bytes::from_static(b"c"),
                sequence: 5,
            }],
        );
        assert!(hidden);
        assert!(!visible_at(ValueType::Value, hidden));
    }

    #[test]
    fn dictionary_replay_get_is_visible_at() {
        // Crash-dictionary last arrow: an acked Value at seq <= snapshot
        // with no covering range del is what get returns.
        assert!(visible_at(ValueType::Value, false));
        let entries = vec![(ik(b"k", 7, ValueType::Value), Bytes::from_static(b"acked"))];
        let got = visible_range(entries, 7, Bound::Unbounded, Bound::Unbounded);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value.as_ref(), b"acked");
    }
}
