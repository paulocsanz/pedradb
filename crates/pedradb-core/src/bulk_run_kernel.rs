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
#[derive(Debug, Clone)]
pub(crate) struct BulkRun {
    keys: Vec<Bytes>,
    vals: Vec<Bytes>,
    seqs: Vec<SequenceNumber>,
    kinds: Vec<ValueType>,
    bytes: usize,
    is_sorted: bool,
}

impl Default for BulkRun {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            vals: Vec::new(),
            seqs: Vec::new(),
            kinds: Vec::new(),
            bytes: 0,
            is_sorted: true,
        }
    }
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

    #[must_use]
    pub(crate) fn is_sorted(&self) -> bool {
        self.is_sorted
    }

    pub(crate) fn sort(&mut self) {
        if self.is_sorted || self.keys.len() <= 1 {
            self.is_sorted = true;
            return;
        }
        let mut indices: Vec<usize> = (0..self.keys.len()).collect();
        indices.sort_by(|&a, &b| self.keys[a].cmp(&self.keys[b]));
        let mut new_keys = Vec::with_capacity(self.keys.len());
        let mut new_vals = Vec::with_capacity(self.vals.len());
        let mut new_seqs = Vec::with_capacity(self.seqs.len());
        let mut new_kinds = Vec::with_capacity(self.kinds.len());
        for &idx in &indices {
            new_keys.push(self.keys[idx].clone());
            new_vals.push(self.vals[idx].clone());
            new_seqs.push(self.seqs[idx]);
            new_kinds.push(self.kinds[idx]);
        }
        self.keys = new_keys;
        self.vals = new_vals;
        self.seqs = new_seqs;
        self.kinds = new_kinds;
        self.is_sorted = true;
    }

    pub(crate) fn push_with_kind(
        &mut self,
        key: Bytes,
        val: Bytes,
        seq: SequenceNumber,
        kind: ValueType,
    ) {
        if self.is_sorted && !self.keys.is_empty() {
            if self.keys[self.keys.len() - 1] > key {
                self.is_sorted = false;
            }
        }
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
    #[allow(dead_code)]
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
    /// RFC-0307 P0: point lookup that also reports the winning entry's
    /// sequence — the caller must compare range tombstones against the
    /// ENTRY's seq, not a placeholder (a tomb older than the value must
    /// not hide it).
    pub(crate) fn lookup_with_seq(
        &self,
        key: &[u8],
        snapshot: SequenceNumber,
    ) -> Option<(SequenceNumber, Lookup)> {
        let Ok(i) = self.keys.binary_search_by(|k| k.as_ref().cmp(key)) else {
            return None;
        };
        if self.seqs[i] > snapshot {
            return None;
        }
        match self.kinds.get(i).copied().unwrap_or(ValueType::Value) {
            ValueType::Value => Some((self.seqs[i], Lookup::Found(self.vals[i].clone()))),
            ValueType::Deletion => Some((self.seqs[i], Lookup::Deleted)),
            _ => None,
        }
    }

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
    thread_local! {
        static SCRATCH: std::cell::RefCell<(Vec<usize>, Vec<Bytes>, Vec<Bytes>)> =
            std::cell::RefCell::new((Vec::with_capacity(1024), Vec::with_capacity(1024), Vec::with_capacity(1024)));
    }
    SCRATCH.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let (idx, nk, nv) = &mut *borrow;
        idx.clear();
        idx.extend(0..n);
        idx.sort_unstable_by(|&a, &b| keys[a].as_ref().cmp(keys[b].as_ref()));
        nk.clear();
        nv.clear();
        nk.reserve(n);
        nv.reserve(n);
        for &i in idx.iter() {
            nk.push(std::mem::take(&mut keys[i]));
            nv.push(std::mem::take(&mut vals[i]));
        }
        keys.clear();
        keys.extend(nk.drain(..));
        vals.clear();
        vals.extend(nv.drain(..));
    });
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

    #[test]
    fn bulk_run_is_sorted_tracking() {
        let mut r = BulkRun::default();
        assert!(r.is_sorted());
        r.push(Bytes::from_static(b"a"), Bytes::from_static(b"1"), 1);
        assert!(r.is_sorted());
        r.push(Bytes::from_static(b"b"), Bytes::from_static(b"2"), 2);
        assert!(r.is_sorted());
        r.push(Bytes::from_static(b"a"), Bytes::from_static(b"0"), 3);
        assert!(!r.is_sorted());
        r.sort();
        assert!(r.is_sorted());
        assert_eq!(r.keys()[0].as_ref(), b"a");
        assert_eq!(r.keys()[1].as_ref(), b"a");
        assert_eq!(r.keys()[2].as_ref(), b"b");
    }
}

