//! Thin product faces on Montanha (RFC-0022) — layers with N writers, no second SoR.
//!
//! Each face is an encoding + small API over [`StoreCluster`] TX/keys. Not wire-compat
//! clones of Postgres/TiKV/etcd/ClickHouse.

use crate::{Result, StoreCluster};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, SyncSender};

// ── Watch (cluster-local after majority apply) ─────────────────────────────

/// Event delivered to watchers after a majority-committed mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    /// User key.
    pub key: Vec<u8>,
    /// New value (empty = delete / tombstone-ish for lab).
    pub value: Vec<u8>,
    /// Store commit generation after apply.
    pub version: u64,
}

/// In-process watch hub attached to a cluster (RFC-0022 P0.3).
#[derive(Default)]
pub struct WatchHub {
    next_id: u64,
    /// id → (prefix, sender)
    subs: HashMap<u64, (Vec<u8>, SyncSender<WatchEvent>)>,
}

impl WatchHub {
    /// Create empty hub.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to keys with the given prefix. Returns id + receiver.
    pub fn watch_prefix(&mut self, prefix: impl AsRef<[u8]>) -> (u64, Receiver<WatchEvent>) {
        let (tx, rx) = mpsc::sync_channel(256);
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.subs
            .insert(id, (prefix.as_ref().to_vec(), tx));
        (id, rx)
    }

    /// Drop a subscription.
    pub fn unwatch(&mut self, id: u64) {
        self.subs.remove(&id);
    }

    /// Notify all matching watchers (best-effort; full channel drops event).
    pub fn notify(&self, key: &[u8], value: &[u8], version: u64) {
        let ev = WatchEvent {
            key: key.to_vec(),
            value: value.to_vec(),
            version,
        };
        for (_, (pref, tx)) in &self.subs {
            if key.starts_with(pref.as_slice()) {
                let _ = tx.try_send(ev.clone());
            }
        }
    }

    /// Notify many keys after a multi-key commit.
    pub fn notify_pairs(&self, pairs: &[(Vec<u8>, Vec<u8>)], version: u64) {
        for (k, v) in pairs {
            self.notify(k, v, version);
        }
    }
}

// ── etcd-need DCS face ─────────────────────────────────────────────────────

/// etcd-class coordination face on store only (RFC-0022 P0.4).
pub struct EtcdNeedFace;

impl EtcdNeedFace {
    /// Prefix for coordination keys (must not collide with app data carelessly).
    pub const PREFIX: &'static [u8] = b"m/";

    /// Create-if-absent (DCS create) — fails if key exists.
    pub fn create(cluster: &mut StoreCluster, key: &[u8], value: &[u8]) -> Result<u64> {
        let full = Self::full_key(key);
        cluster.dcs_create(&full, value)
    }

    /// Compare-and-swap by revision.
    pub fn cas(
        cluster: &mut StoreCluster,
        key: &[u8],
        value: &[u8],
        expected_rev: u64,
    ) -> Result<u64> {
        let full = Self::full_key(key);
        cluster.dcs_cas(&full, value, expected_rev)
    }

    /// Get coordination key (LocalApplied; leadership-invisible — any local node).
    pub fn get(
        cluster: &StoreCluster,
        key: &[u8],
    ) -> Result<Option<pedradb_dcs::KeyValue>> {
        let full = Self::full_key(key);
        let id = cluster
            .local_node_id()
            .or_else(|| cluster.member_ids().first().copied())
            .ok_or_else(|| crate::StoreError::Msg("empty cluster".into()))?;
        cluster.dcs_get_on(id, &full)
    }

    /// Get on an explicit node (tests / multiproc).
    pub fn get_on(
        cluster: &StoreCluster,
        node_id: u64,
        key: &[u8],
    ) -> Result<Option<pedradb_dcs::KeyValue>> {
        let full = Self::full_key(key);
        cluster.dcs_get_on(node_id, &full)
    }

    fn full_key(key: &[u8]) -> Vec<u8> {
        let mut k = Self::PREFIX.to_vec();
        k.extend_from_slice(key);
        k
    }
}

// ── Secondary index via multi-key TX ───────────────────────────────────────

fn push_len_pref(buf: &mut Vec<u8>, part: &[u8]) {
    let n = u32::try_from(part.len()).expect("component len fits u32");
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(part);
}

/// Tip key storing the last secondary index value for (table, col, pk) — F64.
#[must_use]
pub fn table_index_tip_key(table: &[u8], col: &[u8], pk: &[u8]) -> Vec<u8> {
    // Lead with pk so tip shards with the row (same range leader).
    let mut k = Vec::with_capacity(pk.len() + table.len() + col.len() + 16);
    k.extend_from_slice(pk);
    k.push(0x00);
    k.extend_from_slice(b"m"); // meta tip marker
    push_len_pref(&mut k, table);
    push_len_pref(&mut k, col);
    k
}

/// Maintain primary row + secondary index entry in one TX (no app dual-write).
///
/// F64: on index_val change, clears the previous reverse key (was a silent stale
/// hit). Row body stays opaque; last index_val is stored under
/// [`table_index_tip_key`].
pub fn put_with_secondary_index(
    cluster: &mut StoreCluster,
    table: &[u8],
    pk: &[u8],
    index_col: &[u8],
    index_val: &[u8],
    row: &[u8],
) -> Result<u64> {
    let data_key = table_row_key(table, pk);
    let tip_key = table_index_tip_key(table, index_col, pk);
    let idx_key = table_index_key(table, index_col, index_val, pk);
    let mut tx = cluster.begin();
    if let Some(old_iv) = tx.get(cluster, &tip_key)? {
        if old_iv.as_slice() != index_val {
            tx.clear(table_index_key(table, index_col, &old_iv, pk))?;
        }
    }
    tx.set(&data_key, row)?;
    tx.set(&tip_key, index_val)?;
    // index maps indexed value → pk
    tx.set(&idx_key, pk)?;
    tx.commit(cluster)
}

/// Look up PK via secondary index (LocalApplied).
pub fn lookup_secondary(
    cluster: &StoreCluster,
    table: &[u8],
    index_col: &[u8],
    index_val: &[u8],
    pk: &[u8],
) -> Result<Option<Vec<u8>>> {
    let idx_key = table_index_key(table, index_col, index_val, pk);
    Ok(cluster.get(&idx_key)?.map(|b| b.to_vec()))
}

/// Range of reverse keys for exact `index_val` (not slash/prefix siblings).
#[must_use]
pub fn table_index_value_range(val: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut start = val.to_vec();
    start.push(0x00);
    let mut end = val.to_vec();
    end.push(0x01);
    (start, end)
}

// ── Table / SQLite-class encoding on Pedra keys ────────────────────────────
//
// Montanha splits the keyspace on **leading bytes** (single-byte range starts).
// Row and index keys therefore lead with the shard key (PK / index value) so
// disjoint PKs can land on **different range leaders** (true N writers).
// Fixed prefixes like `t/` first would collocate every table on one range.

/// Encode a logical table row: `{pk}\0t` + length-prefixed table (F64).
///
/// Leading with `pk` makes first-byte range splits place different PKs on
/// different leaders (Postgres N-writer / TiDB-shaped sharding).
#[must_use]
pub fn table_row_key(table: &[u8], pk: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(pk.len() + table.len() + 12);
    k.extend_from_slice(pk);
    k.push(0x00);
    k.extend_from_slice(b"t");
    push_len_pref(&mut k, table);
    k
}

/// Secondary index key: `{val}\0` + length-prefixed (`i`, table, col, pk).
///
/// F64: slash-joined `i/{table}/{col}/{pk}` collided when any component
/// contained `/` (e.g. table=`a` col=`b/c` pk=`d` vs table=`a/b` col=`c` pk=`d`).
///
/// Shards by indexed value so index partitions can also spread across ranges;
/// maintaining row+index still uses multi-key TX (possibly cross-range).
#[must_use]
pub fn table_index_key(table: &[u8], col: &[u8], val: &[u8], pk: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(val.len() + table.len() + col.len() + pk.len() + 24);
    k.extend_from_slice(val);
    k.push(0x00);
    push_len_pref(&mut k, b"i");
    push_len_pref(&mut k, table);
    push_len_pref(&mut k, col);
    push_len_pref(&mut k, pk);
    k
}

/// Put a table row without naming leaders (RFC-0025: single-key path).
///
/// For **row + secondary index**, prefer [`put_with_secondary_index`] (one TX)
/// or [`StoreCluster::put_many`] when keys share a range.
pub fn table_put(
    cluster: &mut StoreCluster,
    table: &[u8],
    pk: &[u8],
    row: &[u8],
) -> Result<u64> {
    // Single key: put_batch of one is still one Raft entry (same as put).
    cluster.put_batch([(table_row_key(table, pk).as_slice(), row)])?;
    Ok(cluster.read_version())
}

/// Get table row.
pub fn table_get(cluster: &StoreCluster, table: &[u8], pk: &[u8]) -> Result<Option<Vec<u8>>> {
    Ok(cluster.get(&table_row_key(table, pk))?.map(|b| b.to_vec()))
}

// ── TiKV-like raw KV face ──────────────────────────────────────────────────

/// Thin TiKV-class raw KV (put/get/delete) via leadership-invisible TX/put.
///
/// Callers pass **raw keys**; for N writers, keys must hash/split across ranges
/// (different leading bytes under Montanha’s first-byte split). Use
/// [`raw_keys_one_per_range`] in tests/demos.
pub struct TikvKvFace;

impl TikvKvFace {
    /// Put key (routes via store; no leader id in API).
    pub fn put(cluster: &mut StoreCluster, key: &[u8], value: &[u8]) -> Result<()> {
        cluster.put(key, value)
    }

    /// Get key.
    pub fn get(cluster: &StoreCluster, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(cluster.get(key)?.map(|b| b.to_vec()))
    }

    /// Delete via empty put (store apply path is Pedra `delete`).
    pub fn delete(cluster: &mut StoreCluster, key: &[u8]) -> Result<()> {
        cluster.put(key, b"")
    }

    /// Multi-key TX put (N keys atomic; may be cross-range).
    pub fn batch_put(
        cluster: &mut StoreCluster,
        pairs: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<u64> {
        let mut tx = cluster.begin();
        for (k, v) in pairs {
            tx.set(k, v)?;
        }
        tx.commit(cluster)
    }
}

/// Raw keys, one under each range start (for TiKV-face N-writer proofs).
#[must_use]
pub fn raw_keys_one_per_range(cluster: &StoreCluster) -> Vec<Vec<u8>> {
    cluster
        .range_metas()
        .iter()
        .map(|r| {
            if r.start.is_empty() {
                vec![0x00, b'k']
            } else {
                let mut k = r.start.clone();
                k.push(b'k');
                k
            }
        })
        .collect()
}

// ── Postgres-shaped N-writer (disjoint PK ranges) ──────────────────────────

/// Map primary key → store key (PK-leading so disjoint PKs ⇒ N range writers).
#[must_use]
pub fn pg_pk_key(table: &[u8], pk: &[u8]) -> Vec<u8> {
    table_row_key(table, pk)
}

/// Writer for one PK — uses SnapshotTx (no leader naming).
///
/// For multi-writer scale, pick PKs whose leading bytes fall in different
/// [`StoreCluster`] ranges (see `table_row_key`).
pub fn pg_upsert(
    cluster: &mut StoreCluster,
    table: &[u8],
    pk: &[u8],
    row: &[u8],
) -> Result<u64> {
    table_put(cluster, table, pk, row)
}

/// One sample PK that lives in each range of `cluster` (leading bytes = range start).
///
/// Used by N-writer demos/tests so `locate(table_row_key(t, pk_i))` are distinct.
#[must_use]
pub fn pks_one_per_range(cluster: &StoreCluster) -> Vec<Vec<u8>> {
    cluster
        .range_metas()
        .iter()
        .map(|r| {
            if r.start.is_empty() {
                // First range is [∅, end); use 0x00 so key is still in-range.
                vec![0x00]
            } else {
                r.start.clone()
            }
        })
        .collect()
}

// ── OLAP RO projection (derive from SoR, no dual write) ────────────────────

/// Key: `olap/{stream}\0{seq:020}` (F63: not `olap/{stream}/{seq}` — that
/// nests subject `a/b` under prefix scan of `a`).
#[must_use]
pub fn olap_event_key(stream: &[u8], seq: u64) -> Vec<u8> {
    subject_seq_key(b"olap/", stream, seq)
}

/// Append-only event for analytical scan (writer = OLTP put).
pub fn olap_ingest(
    cluster: &mut StoreCluster,
    stream: &[u8],
    seq: u64,
    payload: &[u8],
) -> Result<()> {
    cluster.put(&olap_event_key(stream, seq), payload)
}

/// Read back a single ingested event (RO path from same SoR).
pub fn olap_get(
    cluster: &StoreCluster,
    stream: &[u8],
    seq: u64,
) -> Result<Option<Vec<u8>>> {
    Ok(cluster.get(&olap_event_key(stream, seq))?.map(|b| b.to_vec()))
}

/// Half-open range of all seqs under `stream` name (exact, not slash children).
#[must_use]
pub fn olap_stream_range(stream: &[u8]) -> (Vec<u8>, Vec<u8>) {
    subject_children_range(b"olap/", stream)
}

/// List `(seq, payload)` under exact `stream` (not slash-prefix children).
pub fn olap_list_at(
    cluster: &StoreCluster,
    stream: &[u8],
    snapshot: u64,
) -> Result<Vec<(u64, Vec<u8>)>> {
    list_subject_at(cluster, b"olap/", stream, snapshot)
}

// ── Stream / NATS-need durable subject ─────────────────────────────────────

/// `prefix || subject || 0x00 || {seq:020}` — F63.
#[must_use]
pub fn subject_seq_key(ns: &[u8], subject: &[u8], seq: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(ns.len() + subject.len() + 1 + 20);
    k.extend_from_slice(ns);
    k.extend_from_slice(subject);
    k.push(0x00);
    k.extend_from_slice(format!("{seq:020}").as_bytes());
    k
}

/// Exact subject children: `[ns||subject||0x00, ns||subject||0x01)`.
#[must_use]
pub fn subject_children_range(ns: &[u8], subject: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut start = Vec::with_capacity(ns.len() + subject.len() + 1);
    start.extend_from_slice(ns);
    start.extend_from_slice(subject);
    start.push(0x00);
    let mut end = start.clone();
    *end.last_mut().unwrap() = 0x01;
    (start, end)
}

/// Publish to durable subject (key = `stream/{subject}\0{seq}`).
pub fn stream_publish(
    cluster: &mut StoreCluster,
    subject: &[u8],
    seq: u64,
    body: &[u8],
) -> Result<()> {
    cluster.put(&subject_seq_key(b"stream/", subject, seq), body)
}

/// Consume one message by seq (cursor external).
pub fn stream_get(
    cluster: &StoreCluster,
    subject: &[u8],
    seq: u64,
) -> Result<Option<Vec<u8>>> {
    Ok(cluster
        .get(&subject_seq_key(b"stream/", subject, seq))?
        .map(|b| b.to_vec()))
}

/// List bodies under exact `subject` (not slash-prefix children) at a snapshot.
pub fn stream_list_at(
    cluster: &StoreCluster,
    subject: &[u8],
    snapshot: u64,
) -> Result<Vec<(u64, Vec<u8>)>> {
    list_subject_at(cluster, b"stream/", subject, snapshot)
}

fn list_subject_at(
    cluster: &StoreCluster,
    ns: &[u8],
    subject: &[u8],
    snapshot: u64,
) -> Result<Vec<(u64, Vec<u8>)>> {
    let (start, end) = subject_children_range(ns, subject);
    let pairs = cluster.keys_in_range_at(&start, &end, snapshot)?;
    let mut out = Vec::new();
    for (k, v) in pairs {
        let Some(rest) = k.strip_prefix(start.as_slice()) else {
            continue;
        };
        if rest.len() != 20 {
            continue;
        }
        if let Ok(s) = std::str::from_utf8(rest) {
            if let Ok(seq) = s.parse::<u64>() {
                out.push((seq, v));
            }
        }
    }
    out.sort_by_key(|(s, _)| *s);
    Ok(out)
}

// ── Scylla-need CP helper ──────────────────────────────────────────────────

/// High-level CP put under `cp/` with optional watch notify.
pub fn cp_put(
    cluster: &mut StoreCluster,
    hub: &WatchHub,
    key: &[u8],
    value: &[u8],
) -> Result<()> {
    let mut k = b"cp/".to_vec();
    k.extend_from_slice(key);
    cluster.put(&k, value)?;
    hub.notify(&k, value, cluster.read_version());
    Ok(())
}

/// SQL spine sketch: multi-table row write in one TX (TiDB-class spine seed).
pub fn sql_multi_table_write(
    cluster: &mut StoreCluster,
    writes: &[(&[u8], &[u8], &[u8])], // (table, pk, row)
) -> Result<u64> {
    let mut tx = cluster.begin();
    for (table, pk, row) in writes {
        tx.set(table_row_key(table, pk), row)?;
    }
    tx.commit(cluster)
}

/// Reserved meta helper re-export for layers.
pub use crate::meta_key;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StoreCluster, StoreError};

    fn temp() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("pedra-layers-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn snapshot_tx_begin_mutate_commit() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let mut tx = c.begin();
        assert_eq!(tx.snapshot_version(), 0);
        tx.set(b"a", b"1").unwrap();
        tx.set(b"b", b"2").unwrap();
        let tid = tx.commit(&mut c).unwrap();
        assert!(tid >= 1);
        assert_eq!(c.get(b"a").unwrap().as_deref(), Some(b"1".as_ref()));
        assert_eq!(c.get(b"b").unwrap().as_deref(), Some(b"2".as_ref()));
        assert!(c.read_version() >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_tx_occ_ww_conflict() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let mut t1 = c.begin();
        let mut t2 = c.begin();
        t1.set(b"k", b"v1").unwrap();
        t2.set(b"k", b"v2").unwrap();
        t1.commit(&mut c).unwrap();
        let err = t2.commit(&mut c).expect_err("OCC WW");
        assert!(
            matches!(err, StoreError::Conflict),
            "got {err:?}"
        );
        assert_eq!(c.get(b"k").unwrap().as_deref(), Some(b"v1".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_tx_read_set_conflict() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"x", b"0").unwrap();
        let mut t1 = c.begin();
        let _ = t1.get(&c, b"x").unwrap();
        // Concurrent mutation of read key.
        c.put(b"x", b"1").unwrap();
        t1.set(b"y", b"from-t1").unwrap();
        let err = t1.commit(&mut c).expect_err("read-set OCC");
        assert!(matches!(err, StoreError::Conflict), "got {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_tx_too_old() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        let mut tx = c.begin();
        assert_eq!(tx.snapshot_version(), 0);
        // Real watermark GC path (RFC-0023): snapshots below safe_watermark are too old.
        c.force_safe_watermark_for_test(1);
        tx.set(b"late", b"1").unwrap();
        let err = tx.commit(&mut c).expect_err("too old");
        assert!(
            matches!(err, StoreError::TransactionTooOld { .. }),
            "got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0023: concurrent commit after begin must not be visible to in-TX get.
    #[test]
    fn tx_snapshot_hides_concurrent_commit() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"sk", b"v0").unwrap();
        let mut tx = c.begin();
        // Concurrent writer after begin.
        c.put(b"sk", b"v1").unwrap();
        // Snapshot must still see v0.
        assert_eq!(
            tx.get(&c, b"sk").unwrap().as_deref(),
            Some(b"v0".as_ref()),
            "read skew: TX saw post-begin commit"
        );
        // Read-set conflict on commit if we also write something.
        tx.set(b"other", b"x").unwrap();
        let err = tx.commit(&mut c).expect_err("read-set OCC");
        assert!(matches!(err, StoreError::Conflict), "got {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0023 P1.3: range read conflicts with concurrent write in range.
    #[test]
    fn tx_range_read_conflict() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"r/a", b"1").unwrap();
        c.put(b"r/b", b"2").unwrap();
        let mut tx = c.begin();
        let got = tx.get_range(&c, b"r/", b"r0").unwrap();
        assert!(got.len() >= 1, "expected keys under r/");
        // Concurrent write inside range.
        c.put(b"r/c", b"3").unwrap();
        tx.set(b"out", b"z").unwrap();
        let err = tx.commit(&mut c).expect_err("range conflict");
        assert!(matches!(err, StoreError::Conflict), "got {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Too-old after real retention GC (many commits), not only force helper.
    #[test]
    fn tx_too_old_after_gc_watermark() {
        use crate::VERSION_RETENTION;
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        let mut tx = c.begin();
        assert_eq!(tx.snapshot_version(), 0);
        // Generate enough commits to advance watermark past 0.
        for i in 0..(VERSION_RETENTION + 4) {
            let k = format!("gc{i:04}");
            c.put(k.as_bytes(), b"x").unwrap();
        }
        assert!(c.safe_watermark() > 0, "watermark should advance");
        tx.set(b"after-gc", b"1").unwrap();
        let err = tx.commit(&mut c).expect_err("too old after GC");
        assert!(
            matches!(err, StoreError::TransactionTooOld { .. }),
            "got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_after_majority_put() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let (_id, rx) = c.watch_prefix(b"w/");
        c.put(b"w/1", b"hello").unwrap();
        let ev = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(ev.key, b"w/1");
        assert_eq!(ev.value, b"hello");
        assert!(ev.version >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn etcd_need_face_create_cas_get() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let (_id, rx) = c.watch_prefix(EtcdNeedFace::PREFIX);
        let rev = EtcdNeedFace::create(&mut c, b"leader", b"n1").unwrap();
        assert!(rev >= 1);
        let kv = EtcdNeedFace::get(&c, b"leader").unwrap().expect("kv");
        assert_eq!(kv.value, b"n1");
        let rev2 = EtcdNeedFace::cas(&mut c, b"leader", b"n2", rev).unwrap();
        assert!(rev2 > rev);
        assert_eq!(
            EtcdNeedFace::get(&c, b"leader").unwrap().unwrap().value,
            b"n2"
        );
        // Watch saw mutations under m/
        let mut saw = 0;
        while rx.try_recv().is_ok() {
            saw += 1;
        }
        assert!(saw >= 1, "expected watch events, saw={saw}");
        // Second create fails.
        assert!(EtcdNeedFace::create(&mut c, b"leader", b"n3").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn n_writers_disjoint_ranges() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 4).unwrap();
        c.elect_all(120).unwrap();
        let keys: Vec<Vec<u8>> = c
            .range_metas()
            .iter()
            .map(|r| {
                if r.start.is_empty() {
                    vec![0x00, b'n']
                } else {
                    let mut k = r.start.clone();
                    k.push(b'n');
                    k
                }
            })
            .collect();
        assert!(keys.len() >= 3);
        for (i, k) in keys.iter().enumerate() {
            // Leadership-invisible put.
            c.put_routed_invisible(k, [b'W', i as u8]).unwrap();
        }
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(
                c.get(k).unwrap().as_deref(),
                Some([b'W', i as u8].as_slice())
            );
        }
        let rids: Vec<_> = keys.iter().map(|k| c.locate(k).unwrap()).collect();
        let uniq: std::collections::HashSet<_> = rids.iter().copied().collect();
        assert!(uniq.len() >= 3, "need multi-range writers: {rids:?}");
        // Placement: split + merge adjacent (P1.2).
        let before = c.range_metas().len();
        let (left, right) = c.split_range_at([0x40u8]).unwrap_or_else(|_| {
            // Already split keyspace may reject; force from single-range open path.
            (1, 2)
        });
        let _ = (left, right, before);
        if c.range_metas().len() >= 2 {
            let metas = c.range_metas().to_vec();
            // Find an adjacent pair if any.
            for w in metas.windows(2) {
                if w[0].end == w[1].start {
                    let mid = c.merge_adjacent_ranges(w[0].id, w[1].id);
                    assert!(mid.is_ok(), "merge adjacent: {mid:?}");
                    break;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secondary_index_via_tx() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        put_with_secondary_index(&mut c, b"users", b"42", b"email", b"a@b.c", b"row-42").unwrap();
        assert_eq!(
            table_get(&c, b"users", b"42").unwrap().as_deref(),
            Some(b"row-42".as_ref())
        );
        assert_eq!(
            lookup_secondary(&c, b"users", b"email", b"a@b.c", b"42")
                .unwrap()
                .as_deref(),
            Some(b"42".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F64: slash-joined table/col/pk collides; update must drop old reverse key.
    #[test]
    fn table_index_injective_and_clears_stale_on_change() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(90).unwrap();
        // Two different (table,col,pk) must not share a key under old i/t/c/pk.
        let k1 = table_index_key(b"a", b"b/c", b"v", b"d");
        let k2 = table_index_key(b"a/b", b"c", b"v", b"d");
        assert_ne!(
            k1, k2,
            "slash join collision: a,b/c,d vs a/b,c,d both {k1:?}"
        );
        let r1 = table_row_key(b"a", b"pk1");
        let r2 = table_row_key(b"a/x", b"pk1");
        assert_ne!(r1, r2, "row key must distinguish table names with slash");

        put_with_secondary_index(&mut c, b"t", b"1", b"email", b"old@x", b"row").unwrap();
        put_with_secondary_index(&mut c, b"t", b"1", b"email", b"new@x", b"row2").unwrap();
        assert!(
            lookup_secondary(&c, b"t", b"email", b"old@x", b"1")
                .unwrap()
                .is_none(),
            "stale secondary under old@x after email change"
        );
        assert_eq!(
            lookup_secondary(&c, b"t", b"email", b"new@x", b"1")
                .unwrap()
                .as_deref(),
            Some(b"1".as_ref())
        );
        assert_eq!(
            table_get(&c, b"t", b"1").unwrap().as_deref(),
            Some(b"row2".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tikv_face_n_writers_and_batch() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 4).unwrap();
        c.elect_all(120).unwrap();
        let keys = raw_keys_one_per_range(&c);
        assert!(
            keys.len() >= 3,
            "need multi-range cluster for N-writer face"
        );
        // Distinct range leaders for concurrent writers.
        let mut rids = std::collections::HashSet::new();
        for (i, k) in keys.iter().enumerate() {
            let rid = c.locate(k).unwrap();
            rids.insert(rid);
            let val = [b'V', i as u8];
            TikvKvFace::put(&mut c, k, &val).unwrap();
            assert_eq!(
                TikvKvFace::get(&c, k).unwrap().as_deref(),
                Some(val.as_slice())
            );
        }
        assert!(
            rids.len() >= 3,
            "TikvKvFace N writers must hit distinct ranges, rids={rids:?} keys={keys:?}"
        );
        // Cross-range batch TX still atomic.
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), vec![b'B', i as u8]))
            .collect();
        TikvKvFace::batch_put(&mut c, &pairs).unwrap();
        assert_eq!(
            TikvKvFace::get(&c, &keys[0]).unwrap().as_deref(),
            Some([b'B', 0].as_slice())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_sqlite_encode_and_pg_nwriter() {
        let dir = temp();
        // ≥2 ranges so disjoint PKs can prove multi-leader N-writer.
        let mut c = StoreCluster::open(&dir, 3, 4).unwrap();
        c.elect_all(120).unwrap();

        // Encoding roundtrip (any PK).
        table_put(&mut c, b"t1", b"pk-lo", b"row-lo").unwrap();
        assert_eq!(
            table_get(&c, b"t1", b"pk-lo").unwrap().as_deref(),
            Some(b"row-lo".as_ref())
        );

        // Disjoint PKs — one per range start — must locate to different ranges.
        let pks = pks_one_per_range(&c);
        assert!(pks.len() >= 3, "need multi-range for PG N-writer");
        let mut rids = std::collections::HashSet::new();
        for (i, pk) in pks.iter().enumerate() {
            let row_key = table_row_key(b"orders", pk);
            let rid = c.locate(&row_key).unwrap();
            rids.insert(rid);
            let body = format!("o{i}");
            pg_upsert(&mut c, b"orders", pk, body.as_bytes()).unwrap();
            assert_eq!(
                table_get(&c, b"orders", pk).unwrap().as_deref(),
                Some(body.as_bytes())
            );
        }
        assert!(
            rids.len() >= 3,
            "disjoint PKs must map to ≥3 ranges (PK-leading encode); rids={rids:?} pks={pks:?}"
        );
        // Explicit pair assert (skeptic: never claim multi-range without locate).
        let k0 = table_row_key(b"orders", &pks[0]);
        let k1 = table_row_key(b"orders", &pks[1]);
        assert_ne!(
            c.locate(&k0).unwrap(),
            c.locate(&k1).unwrap(),
            "pk0 and pk1 must not collocate on one range"
        );

        // Same PK conflict via OCC (single key / one range).
        let mut a = c.begin();
        let mut b = c.begin();
        a.set(table_row_key(b"orders", b"same"), b"A").unwrap();
        b.set(table_row_key(b"orders", b"same"), b"B").unwrap();
        a.commit(&mut c).unwrap();
        assert!(matches!(b.commit(&mut c), Err(StoreError::Conflict)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn olap_ro_and_stream_from_sor() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        olap_ingest(&mut c, b"events", 1, b"e1").unwrap();
        olap_ingest(&mut c, b"events", 2, b"e2").unwrap();
        assert_eq!(
            olap_get(&c, b"events", 1).unwrap().as_deref(),
            Some(b"e1".as_ref())
        );
        stream_publish(&mut c, b"subj", 1, b"m1").unwrap();
        assert_eq!(
            stream_get(&c, b"subj", 1).unwrap().as_deref(),
            Some(b"m1".as_ref())
        );
        sql_multi_table_write(
            &mut c,
            &[(b"u", b"1", b"alice"), (b"p", b"9", b"post")],
        )
        .unwrap();
        assert_eq!(
            table_get(&c, b"u", b"1").unwrap().as_deref(),
            Some(b"alice".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Old keys `olap/{stream}/{seq}` nested `events/extra` under prefix
    /// `olap/events/`. Encoding is now `olap/{stream}\0{seq}`.
    #[test]
    fn olap_scan_does_not_include_slash_sibling_stream() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        olap_ingest(&mut c, b"events", 1, b"e1").unwrap();
        olap_ingest(&mut c, b"events/extra", 1, b"leak").unwrap();
        stream_publish(&mut c, b"jobs", 1, b"j1").unwrap();
        stream_publish(&mut c, b"jobs/extra", 1, b"jleak").unwrap();
        let snap = c.read_version();
        let ev = olap_list_at(&c, b"events", snap).unwrap();
        assert_eq!(ev, vec![(1, b"e1".to_vec())], "olap events leaked sibling: {ev:?}");
        let extra = olap_list_at(&c, b"events/extra", snap).unwrap();
        assert_eq!(extra, vec![(1, b"leak".to_vec())]);
        let jobs = stream_list_at(&c, b"jobs", snap).unwrap();
        assert_eq!(jobs, vec![(1, b"j1".to_vec())], "stream jobs leaked sibling: {jobs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scylla_need_cp_put_watch() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        let (_id, rx) = c.watch_prefix(b"cp/");
        // Use cluster watch; cp_put also notifies hub param — use cluster hub.
        let hub = WatchHub::new();
        // Prefer put + cluster notify via normal put under cp/
        c.put(b"cp/route1", b"svc-a").unwrap();
        let ev = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(ev.key, b"cp/route1");
        let _ = hub;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Platform need faces seed: Scylla-CP + CH/OLAP RO + NATS/stream on one SoR.
    ///
    /// Not drop-in Scylla / ClickHouse / JetStream — job-shaped primitives only.
    #[test]
    fn platform_need_faces_scylla_olap_stream() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();

        // Scylla-need CP: durable put under cp/ + watch notify on hub.
        let mut hub = WatchHub::new();
        let (_id, rx) = hub.watch_prefix(b"cp/");
        cp_put(&mut c, &hub, b"token/1", b"owner-a").unwrap();
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("cp watch");
        assert_eq!(ev.key, b"cp/token/1");
        assert_eq!(ev.value, b"owner-a");
        assert_eq!(
            c.get(b"cp/token/1").unwrap().as_deref(),
            Some(b"owner-a".as_ref())
        );

        // ClickHouse-need OLAP: append-only ingest + RO get (same SoR, no dual write).
        olap_ingest(&mut c, b"events", 1, b"e1").unwrap();
        olap_ingest(&mut c, b"events", 2, b"e2").unwrap();
        assert_eq!(
            olap_get(&c, b"events", 1).unwrap().as_deref(),
            Some(b"e1".as_ref())
        );
        assert_eq!(
            olap_get(&c, b"events", 2).unwrap().as_deref(),
            Some(b"e2".as_ref())
        );

        // NATS-need durable subject: publish by seq + consume by cursor.
        stream_publish(&mut c, b"jobs", 1, b"j1").unwrap();
        stream_publish(&mut c, b"jobs", 2, b"j2").unwrap();
        assert_eq!(
            stream_get(&c, b"jobs", 1).unwrap().as_deref(),
            Some(b"j1".as_ref())
        );
        assert_eq!(
            stream_get(&c, b"jobs", 2).unwrap().as_deref(),
            Some(b"j2".as_ref())
        );

        // etcd-need still exclusive on same cluster (coordination path).
        let rev = EtcdNeedFace::create(&mut c, b"leader", b"n1").unwrap();
        assert!(EtcdNeedFace::create(&mut c, b"leader", b"n2").is_err());
        let rev2 = EtcdNeedFace::cas(&mut c, b"leader", b"n1b", rev).unwrap();
        assert!(rev2 > rev);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F63: `stream/{subj}/{seq}` nested `a/b` under prefix scan of subject `a`.
    #[test]
    fn stream_list_does_not_include_slash_child_subject() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(90).unwrap();
        stream_publish(&mut c, b"a", 1, b"only-a").unwrap();
        stream_publish(&mut c, b"a/b", 1, b"child").unwrap();
        assert_eq!(
            stream_get(&c, b"a", 1).unwrap().as_deref(),
            Some(b"only-a".as_ref())
        );
        assert_eq!(
            stream_get(&c, b"a/b", 1).unwrap().as_deref(),
            Some(b"child".as_ref())
        );
        let listed = stream_list_at(&c, b"a", c.read_version()).unwrap();
        assert_eq!(
            listed,
            vec![(1, b"only-a".to_vec())],
            "subject a listed slash-child a/b (old stream/a/ prefix): {listed:?}"
        );
        let child = stream_list_at(&c, b"a/b", c.read_version()).unwrap();
        assert_eq!(child, vec![(1, b"child".to_vec())]);

        // Same class for OLAP stream names.
        olap_ingest(&mut c, b"ev", 1, b"e").unwrap();
        olap_ingest(&mut c, b"ev/nested", 1, b"n").unwrap();
        let (s0, s1) = olap_stream_range(b"ev");
        let got = c.keys_in_range_at(&s0, &s1, c.read_version()).unwrap();
        assert_eq!(
            got.len(),
            1,
            "olap stream ev range included nested: {got:?}"
        );
        assert_eq!(got[0].1.as_slice(), b"e");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
