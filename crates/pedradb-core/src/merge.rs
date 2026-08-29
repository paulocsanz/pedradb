//! Merge MemTable ∪ SST layers for point/range reads with MVCC visibility.
//!
//! Given versioned entries ordered by [`InternalKey`] (user key ascending,
//! sequence descending), emit one live value per user key at a snapshot.
//! Supports range tombstones ([`ValueType::RangeDeletion`]) and a streaming
//! merge path that does not require materialising the full keyspace first.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
use std::ops::Bound;

use bytes::Bytes;

use crate::error::Result;
use crate::key::{InternalKey, SequenceNumber, ValueType};

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

/// Heap entry for multi-way merge of sorted internal-key streams (min-heap by [`InternalKey`]).
struct HeapItem {
    key: InternalKey,
    value: Bytes,
    stream: usize,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is max-heap; reverse so smallest InternalKey is popped first.
        other.key.cmp(&self.key)
    }
}

/// One sorted point-key stream (RFC-0033: pulled lazily so `limit` cuts I/O).
pub type LayerStream<'a> = Box<dyn Iterator<Item = (InternalKey, Bytes)> + 'a>;

/// Streaming merge of **pre-sorted** internal entry streams into visible KVs.
///
/// Each stream must already be ordered by [`InternalKey`]. The iterator pulls
/// one entry at a time (O(streams) memory beyond the streams themselves) and
/// never materialises the full keyspace as a single `Vec` of all pairs.
/// When `limit` is set, later blocks of a lazy SST stream are never decoded
/// (RFC-0033 P0.3). Range tombstones must be supplied up front so a deleted
/// prefix cannot hide later live keys (G2).
pub struct StreamingVisibleIter<'a> {
    heap: BinaryHeap<HeapItem>,
    streams: Vec<LayerStream<'a>>,
    snapshot: SequenceNumber,
    range_dels: Vec<RangeTombstone>,
    start: Bound<Bytes>,
    end: Bound<Bytes>,
    limit: Option<usize>,
    emitted: usize,
    /// Last user key for which we already decided visibility (skip older versions).
    skip_user: Option<Bytes>,
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
        let mut heap = BinaryHeap::new();
        for (i, it) in streams.iter_mut().enumerate() {
            if let Some((k, v)) = it.next() {
                heap.push(HeapItem {
                    key: k,
                    value: v,
                    stream: i,
                });
            }
        }
        Self {
            heap,
            streams,
            snapshot,
            range_dels,
            start: bound_to_owned(start),
            end: bound_to_owned(end),
            limit,
            emitted: 0,
            skip_user: None,
        }
    }

    fn in_range(&self, user_key: &[u8]) -> bool {
        let start = bound_as_ref(&self.start);
        let end = bound_as_ref(&self.end);
        user_key_in_range(user_key, start, end)
    }

    /// Newest version per user key, with [`WindowKv::snapshot_live`].
    ///
    /// Does **not** apply [`iter_window_keep`] — the caller (compat window
    /// or [`Iterator::next`]) decides keep/drop from that bit.
    pub fn next_window_kv(&mut self) -> Option<WindowKv> {
        while let Some(item) = self.heap.pop() {
            if let Some((k, v)) = self.streams[item.stream].next() {
                self.heap.push(HeapItem {
                    key: k,
                    value: v,
                    stream: item.stream,
                });
            }

            let ikey = item.key;
            let value = item.value;
            if ikey.sequence > self.snapshot {
                continue;
            }
            if let Some(ref skip) = self.skip_user {
                if ikey.user_key == *skip {
                    continue;
                }
            }
            if !self.in_range(ikey.user_key.as_ref()) {
                continue;
            }

            self.skip_user = Some(ikey.user_key.clone());
            let range_hidden =
                range_deleted(ikey.user_key.as_ref(), ikey.sequence, &self.range_dels);
            let snapshot_live = visible_at(ikey.kind, range_hidden);
            return Some(WindowKv {
                key: ikey.user_key,
                value,
                snapshot_live,
            });
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
