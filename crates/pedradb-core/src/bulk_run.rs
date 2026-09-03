//! Sorted-run builder for latched bulk ingest (RFC-0159 P0.3).
//!
//! A family that has latched as append-only accumulates puts in a vec
//! (already sorted) instead of the memtable BTree + WAL. Chunks flush
//! straight to `MAX_LSM_LEVEL`. The open tail is RAM-only: a process
//! crash loses it. Installed chunks are in MANIFEST. Same class as
//! Rocks `WriteOptions.disableWAL` during bulk load.

use bytes::Bytes;

use crate::key::SequenceNumber;
use crate::memtable::Lookup;

/// One latched family's uninstalled tail.
#[derive(Debug, Default, Clone)]
pub(crate) struct BulkRun {
    keys: Vec<Bytes>,
    vals: Vec<Bytes>,
    seqs: Vec<SequenceNumber>,
    bytes: usize,
}

impl BulkRun {
    pub(crate) fn reserve(&mut self, n: usize) {
        self.keys.reserve(n);
        self.vals.reserve(n);
        self.seqs.reserve(n);
    }

    pub(crate) fn push(&mut self, key: Bytes, val: Bytes, seq: SequenceNumber) {
        self.bytes = self
            .bytes
            .saturating_add(key.len())
            .saturating_add(val.len())
            .saturating_add(8);
        self.keys.push(key);
        self.vals.push(val);
        self.seqs.push(seq);
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
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
        Lookup::Found(self.vals[i].clone())
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
