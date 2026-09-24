//! Sorted-run builder for latched bulk ingest (RFC-0159 P0.3).
//!
//! A family that has latched as append-only accumulates puts in a vec
//! (already sorted) instead of the memtable BTree + WAL. Chunks flush
//! straight to `MAX_LSM_LEVEL`. The open tail is RAM-only: a process
//! crash loses it. Installed chunks are in MANIFEST. Same class as
//! Rocks `WriteOptions.disableWAL` during bulk load.

use bytes::Bytes;

use std::ops::Bound;

use crate::key::{InternalKey, SequenceNumber, ValueType};
use crate::memtable::Lookup;

/// One latched family's uninstalled tail.
#[derive(Debug, Default, Clone)]
pub(crate) struct BulkRun {
    keys: Vec<Bytes>,
    vals: Vec<Bytes>,
    seqs: Vec<SequenceNumber>,
    kinds: Vec<ValueType>,
    bytes: usize,
}

impl BulkRun {
    pub(crate) fn reserve(&mut self, n: usize) {
        self.keys.reserve(n);
        self.vals.reserve(n);
        self.seqs.reserve(n);
        self.kinds.reserve(n);
    }

    pub(crate) fn push(&mut self, key: Bytes, val: Bytes, seq: SequenceNumber) {
        self.push_with_kind(key, val, seq, ValueType::Value);
    }

    pub(crate) fn push_with_kind(
        &mut self,
        key: Bytes,
        val: Bytes,
        seq: SequenceNumber,
        kind: ValueType,
    ) {
        self.bytes = self
            .bytes
            .saturating_add(key.len())
            .saturating_add(val.len())
            .saturating_add(8);
        self.keys.push(key);
        self.vals.push(val);
        self.seqs.push(seq);
        self.kinds.push(kind);
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        crate::write_admission_kernel::batch_is_empty(self.keys.len() as u64)
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    #[must_use]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    #[must_use]
    pub(crate) fn lookup(&self, key: &[u8], snapshot: SequenceNumber) -> Lookup {
        let Ok(i) = self.keys.binary_search_by(|k| k.as_ref().cmp(key)) else {
            return Lookup::NotFound;
        };
        if self.seqs[i] > snapshot {
            return Lookup::NotFound;
        }
        match self.kinds.get(i).copied().unwrap_or(ValueType::Value) {
            ValueType::Value => Lookup::Found(self.vals[i].clone()),
            ValueType::Deletion => Lookup::Deleted,
            _ => Lookup::NotFound,
        }
    }

    #[must_use]
    pub(crate) fn keys(&self) -> &[Bytes] {
        &self.keys
    }

    #[must_use]
    pub(crate) fn vals(&self) -> &[Bytes] {
        &self.vals
    }

    #[must_use]
    pub(crate) fn seqs(&self) -> &[SequenceNumber] {
        &self.seqs
    }

    #[must_use]
    pub(crate) fn kinds(&self) -> &[ValueType] {
        &self.kinds
    }

    pub(crate) fn last_visible_under_prefix(
        &self,
        prefix: &[u8],
        snapshot: SequenceNumber,
        hi: Option<&[u8]>,
    ) -> Option<Bytes> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        let end_idx = match hi {
            Some(h) => match self.keys.binary_search_by(|k| k.as_ref().cmp(h)) {
                Ok(i) | Err(i) => i,
            },
            None => n,
        };
        for i in (0..end_idx).rev() {
            let k = &self.keys[i];
            if !k.starts_with(prefix) {
                if k.as_ref() < prefix {
                    break;
                }
                continue;
            }
            if self.seqs[i] <= snapshot {
                return Some(k.clone());
            }
        }
        None
    }

    pub(crate) fn iter_range<'a>(
        &'a self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        snapshot: SequenceNumber,
    ) -> impl Iterator<Item = (InternalKey, Bytes)> + 'a {
        let start_idx = match start {
            Bound::Unbounded => 0,
            Bound::Included(s) => match self.keys.binary_search_by(|k| k.as_ref().cmp(s)) {
                Ok(i) | Err(i) => i,
            },
            Bound::Excluded(s) => match self.keys.binary_search_by(|k| k.as_ref().cmp(s)) {
                Ok(i) => i.saturating_add(1),
                Err(i) => i,
            },
        };
        let end_idx = match end {
            Bound::Unbounded => self.keys.len(),
            Bound::Included(e) => match self.keys.binary_search_by(|k| k.as_ref().cmp(e)) {
                Ok(i) => i.saturating_add(1),
                Err(i) => i,
            },
            Bound::Excluded(e) => match self.keys.binary_search_by(|k| k.as_ref().cmp(e)) {
                Ok(i) | Err(i) => i,
            },
        };
        let effective_end = end_idx.min(self.keys.len());
        let effective_start = start_idx.min(effective_end);
        (effective_start..effective_end).filter_map(move |i| {
            if self.seqs[i] > snapshot {
                return None;
            }
            let k = &self.keys[i];
            let kind = self.kinds.get(i).copied().unwrap_or(ValueType::Value);
            let ik = InternalKey::new(k.clone(), self.seqs[i], kind);
            Some((ik, self.vals[i].clone()))
        })
    }
}

/// Sort parallel key/value vecs by user key (RFC-0159 P2.1 nearly-sorted
/// batches). No-op when already strictly ascending. Bytes clones are
/// refcount bumps.
pub(crate) fn sort_bulk_key_vals(keys: &mut Vec<Bytes>, vals: &mut Vec<Bytes>) {
    debug_assert_eq!(keys.len(), vals.len());
    let n = keys.len();
    if n < 2 {
        return;
    }
    if keys.windows(2).all(|w| w[0].as_ref() < w[1].as_ref()) {
        return;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_unstable_by(|&a, &b| keys[a].as_ref().cmp(keys[b].as_ref()));
    let mut nk = Vec::with_capacity(n);
    let mut nv = Vec::with_capacity(n);
    for i in idx {
        nk.push(std::mem::take(&mut keys[i]));
        nv.push(std::mem::take(&mut vals[i]));
    }
    *keys = nk;
    *vals = nv;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_sorted_puts() {
        let mut r = BulkRun::default();
        r.push(Bytes::from_static(b"a"), Bytes::from_static(b"1"), 1);
        r.push(Bytes::from_static(b"c"), Bytes::from_static(b"3"), 2);
        assert!(matches!(r.lookup(b"a", 10), Lookup::Found(v) if v.as_ref() == b"1"));
        assert!(matches!(r.lookup(b"b", 10), Lookup::NotFound));
        assert!(matches!(r.lookup(b"c", 1), Lookup::NotFound));
        assert!(matches!(r.lookup(b"c", 2), Lookup::Found(_)));
    }

    #[test]
    fn reserve_then_push_looks_up() {
        let mut r = BulkRun::default();
        r.reserve(2);
        r.push(Bytes::from_static(b"a"), Bytes::from_static(b"1"), 1);
        r.push(Bytes::from_static(b"b"), Bytes::from_static(b"2"), 2);
        assert_eq!(r.len(), 2);
        assert!(matches!(r.lookup(b"b", 2), Lookup::Found(v) if v.as_ref() == b"2"));
    }

    #[test]
    fn sort_bulk_key_vals_orders_pairs() {
        let mut keys = vec![Bytes::from_static(b"c"), Bytes::from_static(b"a")];
        let mut vals = vec![Bytes::from_static(b"3"), Bytes::from_static(b"1")];
        sort_bulk_key_vals(&mut keys, &mut vals);
        assert_eq!(keys[0].as_ref(), b"a");
        assert_eq!(vals[0].as_ref(), b"1");
        assert_eq!(keys[1].as_ref(), b"c");
        assert_eq!(vals[1].as_ref(), b"3");
    }
}
