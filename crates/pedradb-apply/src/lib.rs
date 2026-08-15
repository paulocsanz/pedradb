//! Ordered apply of log entries into PedraDB (RFC-0010).
//!
//! A Raft (or test) log calls [`LogApplier::apply_entries`] with already-ordered
//! batches. Each entry becomes one atomic [`pedradb_core::Db::apply_batch`] —
//! no OCC inside PedraDB.
//!
//! Also provides:
//! - [`InProcessCluster`] — majority-commit fake Raft (P1.1 educational)
//! - [`KvService`] — thin get/put/TX façade (P1.3)
//!
//! This crate does **not** implement real Raft networking or elections.
//! WAL-shipped replicas live in `pedradb-replicate` (P1.2).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use pedradb_core::{BatchOp, Db, Result, SequenceNumber, Snapshot, Transaction, WriteOptions};

/// One committed log entry: an ordered multi-op batch for PedraDB.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Opaque index from the log (Raft index, fake counter, …).
    pub index: u64,
    /// Ops to apply atomically.
    pub ops: Vec<BatchOp>,
}

impl LogEntry {
    /// Build an entry from put pairs.
    #[must_use]
    pub fn puts(index: u64, kvs: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) -> Self {
        Self {
            index,
            ops: kvs.into_iter().map(|(k, v)| BatchOp::put(k, v)).collect(),
        }
    }
}

/// Applies ordered log entries to a [`Db`].
pub struct LogApplier<'a> {
    db: &'a mut Db,
    /// Last applied log index (for callers; PedraDB stores data only).
    last_index: u64,
}

impl<'a> LogApplier<'a> {
    /// Wrap a database; `last_index` is caller's cursor (usually 0 at start).
    pub fn new(db: &'a mut Db, last_index: u64) -> Self {
        Self { db, last_index }
    }

    /// Last successfully applied log index.
    #[must_use]
    pub fn last_index(&self) -> u64 {
        self.last_index
    }

    /// Current PedraDB snapshot sequence (export for followers / backups).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.db.snapshot()
    }

    /// Apply consecutive entries with indices `last_index+1`, `+2`, …
    ///
    /// Skips empty op lists but still advances index. Uses durable sync by
    /// default (`WriteOptions::default()` → DB open policy).
    ///
    /// # Errors
    /// PedraDB write errors, or non-monotonic index.
    pub fn apply_entries(&mut self, entries: &[LogEntry]) -> Result<SequenceNumber> {
        self.apply_entries_with(entries, WriteOptions::default())
    }

    /// Apply with explicit write options (e.g. no_sync + outer fsync barrier).
    ///
    /// # Errors
    /// PedraDB write errors, or non-monotonic index.
    pub fn apply_entries_with(
        &mut self,
        entries: &[LogEntry],
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        let mut last_seq = self.db.last_sequence();
        for e in entries {
            let expected = self.last_index.saturating_add(1);
            if e.index != expected {
                return Err(pedradb_core::CoreError::Internal(format!(
                    "log index gap: got {}, expected {expected}",
                    e.index
                )));
            }
            if !e.ops.is_empty() {
                last_seq = self.db.apply_batch_with(e.ops.clone(), opts)?;
            }
            self.last_index = e.index;
        }
        Ok(last_seq)
    }
}

/// In-memory ordered log for demos and tests (not Raft).
#[derive(Debug, Default)]
pub struct FakeLog {
    entries: Vec<LogEntry>,
    next_index: u64,
}

impl FakeLog {
    /// Empty log; next append gets index 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_index: 1,
        }
    }

    /// Append a batch of puts; returns the log index.
    pub fn append_puts(&mut self, kvs: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) -> u64 {
        let index = self.next_index;
        self.next_index += 1;
        self.entries.push(LogEntry::puts(index, kvs));
        index
    }

    /// All entries with index `> after` (exclusive), in order.
    #[must_use]
    pub fn entries_after(&self, after: u64) -> Vec<LogEntry> {
        self.entries
            .iter()
            .filter(|e| e.index > after)
            .cloned()
            .collect()
    }
}

/// Apply the same committed log to multiple PedraDB directories (follower catch-up demo).
///
/// Not Raft — just shows multi-node **apply** of an identical ordered log.
pub fn replicate_log_to_nodes(
    log: &FakeLog,
    nodes: &mut [Db],
    last_index_per_node: &mut [u64],
) -> Result<SequenceNumber> {
    assert_eq!(nodes.len(), last_index_per_node.len());
    let mut last_seq = 0;
    for (db, last) in nodes.iter_mut().zip(last_index_per_node.iter_mut()) {
        let mut applier = LogApplier::new(db, *last);
        let pending = log.entries_after(*last);
        last_seq = applier.apply_entries(&pending)?;
        *last = applier.last_index();
    }
    Ok(last_seq)
}

// ─── P1.1: in-process majority commit (Raft-shaped, not real Raft) ───────────

/// Single node state in an [`InProcessCluster`].
pub struct ClusterNode {
    /// Local PedraDB.
    pub db: Db,
    /// Last applied log index on this node.
    pub last_index: u64,
}

/// In-process multi-node cluster with a shared ordered log and majority apply.
///
/// Models the **shape** of single-region Raft without elections, network, or
/// term numbers:
///
/// 1. Client proposes a batch to the "leader" (node 0).
/// 2. Entry is appended to the shared [`FakeLog`] (committed).
/// 3. Entry is applied to a **majority** of nodes (including leader).
/// 4. Lagging nodes catch up via [`InProcessCluster::catch_up`].
///
/// Replace `FakeLog` + this struct with a real Raft library for production.
pub struct InProcessCluster {
    log: FakeLog,
    nodes: Vec<ClusterNode>,
    /// Leader index (fixed at 0 for this educational shim).
    leader: usize,
}

impl InProcessCluster {
    /// Build a cluster from already-opened databases (one per node).
    ///
    /// # Panics
    /// If `dbs` is empty.
    #[must_use]
    pub fn new(dbs: Vec<Db>) -> Self {
        assert!(!dbs.is_empty(), "cluster needs at least one node");
        let nodes = dbs
            .into_iter()
            .map(|db| ClusterNode { db, last_index: 0 })
            .collect();
        Self {
            log: FakeLog::new(),
            nodes,
            leader: 0,
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the cluster has zero nodes (always false after [`Self::new`]).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Majority size (`n/2 + 1`).
    #[must_use]
    pub fn majority(&self) -> usize {
        self.nodes.len() / 2 + 1
    }

    /// Shared log (for inspection / WAL-alternative ship of logical entries).
    #[must_use]
    pub fn log(&self) -> &FakeLog {
        &self.log
    }

    /// Leader node (node 0).
    #[must_use]
    pub fn leader(&self) -> &ClusterNode {
        &self.nodes[self.leader]
    }

    /// Mutable leader.
    pub fn leader_mut(&mut self) -> &mut ClusterNode {
        let i = self.leader;
        &mut self.nodes[i]
    }

    /// Node by index.
    #[must_use]
    pub fn node(&self, i: usize) -> &ClusterNode {
        &self.nodes[i]
    }

    /// Propose puts through the leader: append to log, apply to majority.
    ///
    /// # Errors
    /// Apply I/O on any majority node.
    pub fn propose_puts(
        &mut self,
        kvs: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    ) -> Result<u64> {
        let index = self.log.append_puts(kvs);
        self.apply_committed_to_majority()?;
        Ok(index)
    }

    /// Apply all committed log entries to the first `majority` nodes.
    ///
    /// # Errors
    /// PedraDB apply errors.
    pub fn apply_committed_to_majority(&mut self) -> Result<()> {
        let m = self.majority();
        for node in self.nodes.iter_mut().take(m) {
            let pending = self.log.entries_after(node.last_index);
            if pending.is_empty() {
                continue;
            }
            let mut applier = LogApplier::new(&mut node.db, node.last_index);
            applier.apply_entries(&pending)?;
            node.last_index = applier.last_index();
        }
        Ok(())
    }

    /// Bring every node up to the log end (learner / slow follower catch-up).
    ///
    /// # Errors
    /// PedraDB apply errors.
    pub fn catch_up_all(&mut self) -> Result<()> {
        for node in &mut self.nodes {
            let pending = self.log.entries_after(node.last_index);
            if pending.is_empty() {
                continue;
            }
            let mut applier = LogApplier::new(&mut node.db, node.last_index);
            applier.apply_entries(&pending)?;
            node.last_index = applier.last_index();
        }
        Ok(())
    }

    /// Read from a follower (after catch-up). Node 0 is leader.
    #[must_use]
    pub fn get_on(&self, node: usize, key: &[u8]) -> Option<bytes::Bytes> {
        self.nodes.get(node).and_then(|n| n.db.get(key))
    }
}

// ─── P1.3: thin KV façade ────────────────────────────────────────────────────

/// Thin in-process KV / TX API over PedraDB (no network).
///
/// Outer products can wrap this with gRPC/HTTP later; the contract stays
/// `get` / `put` / `delete` / multi-key [`Transaction`].
pub struct KvService {
    db: Db,
}

impl KvService {
    /// Wrap an open database.
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Open a path with durable defaults (sync, no auto-flush for predictable demos).
    ///
    /// # Errors
    /// PedraDB open.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        use pedradb_core::OpenOptions;
        let db = Db::open_with(
            path,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )?;
        Ok(Self::new(db))
    }

    /// Point get.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<bytes::Bytes> {
        self.db.get(key)
    }

    /// Auto-commit put.
    ///
    /// # Errors
    /// WAL I/O.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.db.put(key, value)
    }

    /// Auto-commit delete.
    ///
    /// # Errors
    /// WAL I/O.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.db.delete(key)
    }

    /// Begin a multi-key transaction.
    pub fn begin(&mut self) -> Transaction<'_> {
        self.db.begin()
    }

    /// Atomic multi-op apply (Raft / batch path).
    ///
    /// # Errors
    /// WAL I/O.
    pub fn apply(&mut self, ops: impl IntoIterator<Item = BatchOp>) -> Result<SequenceNumber> {
        self.db.apply_batch(ops)
    }

    /// Underlying DB for flush/compact/snapshot.
    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Mutable DB.
    pub fn db_mut(&mut self) -> &mut Db {
        &mut self.db
    }

    /// Close and release the directory lock.
    ///
    /// # Errors
    /// WAL close.
    pub fn close(self) -> Result<()> {
        self.db.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-apply-{tag}-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn open_db(dir: &std::path::Path) -> Db {
        Db::open_with(
            dir,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn fake_log_apply_and_reopen() {
        let dir = temp_dir("one");
        let mut log = FakeLog::new();
        log.append_puts([(b"a".to_vec(), b"1".to_vec())]);
        log.append_puts([
            (b"b".to_vec(), b"2".to_vec()),
            (b"c".to_vec(), b"3".to_vec()),
        ]);

        {
            let mut db = open_db(&dir);
            let mut applier = LogApplier::new(&mut db, 0);
            let pending = log.entries_after(0);
            applier.apply_entries(&pending).unwrap();
            assert_eq!(applier.last_index(), 2);
            assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
            assert_eq!(db.get(b"c").as_deref(), Some(b"3".as_ref()));
            db.close().unwrap();
        }
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert_eq!(db.get(b"b").as_deref(), Some(b"2".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn three_nodes_same_log_same_state() {
        let d0 = temp_dir("n0");
        let d1 = temp_dir("n1");
        let d2 = temp_dir("n2");
        let mut log = FakeLog::new();
        log.append_puts([(b"k".to_vec(), b"v1".to_vec())]);
        log.append_puts([(b"k".to_vec(), b"v2".to_vec())]);

        let mut nodes = [open_db(&d0), open_db(&d1), open_db(&d2)];
        let mut cursors = [0u64, 0, 0];
        replicate_log_to_nodes(&log, &mut nodes, &mut cursors).unwrap();
        assert_eq!(cursors, [2, 2, 2]);
        for db in &nodes {
            assert_eq!(db.get(b"k").as_deref(), Some(b"v2".as_ref()));
        }
        // New committed entry; all nodes catch up from their cursors.
        log.append_puts([(b"k".to_vec(), b"v3".to_vec())]);
        replicate_log_to_nodes(&log, &mut nodes, &mut cursors).unwrap();
        assert_eq!(cursors, [3, 3, 3]);
        for db in &nodes {
            assert_eq!(db.get(b"k").as_deref(), Some(b"v3".as_ref()));
        }
        for d in [d0, d1, d2] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn rejects_index_gap() {
        let dir = temp_dir("gap");
        let mut db = open_db(&dir);
        let mut applier = LogApplier::new(&mut db, 0);
        let bad = [LogEntry::puts(2, [(b"x".to_vec(), b"y".to_vec())])];
        assert!(applier.apply_entries(&bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_process_cluster_majority_then_catch_up() {
        let d0 = temp_dir("c0");
        let d1 = temp_dir("c1");
        let d2 = temp_dir("c2");
        let mut cluster = InProcessCluster::new(vec![open_db(&d0), open_db(&d1), open_db(&d2)]);
        assert_eq!(cluster.majority(), 2);

        cluster
            .propose_puts([(b"k".to_vec(), b"v1".to_vec())])
            .unwrap();
        // Majority (nodes 0,1) applied; node 2 still behind.
        assert_eq!(cluster.node(0).last_index, 1);
        assert_eq!(cluster.node(1).last_index, 1);
        assert_eq!(cluster.node(2).last_index, 0);
        assert_eq!(cluster.get_on(0, b"k").as_deref(), Some(b"v1".as_ref()));
        assert_eq!(cluster.get_on(2, b"k"), None);

        cluster.catch_up_all().unwrap();
        assert_eq!(cluster.node(2).last_index, 1);
        assert_eq!(cluster.get_on(2, b"k").as_deref(), Some(b"v1".as_ref()));

        cluster
            .propose_puts([(b"k".to_vec(), b"v2".to_vec())])
            .unwrap();
        cluster.catch_up_all().unwrap();
        for i in 0..3 {
            assert_eq!(cluster.get_on(i, b"k").as_deref(), Some(b"v2".as_ref()));
        }
        for d in [d0, d1, d2] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn kv_service_put_get_tx() {
        let dir = temp_dir("kv");
        let mut kv = KvService::open(&dir).unwrap();
        kv.put(b"a", b"1").unwrap();
        assert_eq!(kv.get(b"a").as_deref(), Some(b"1".as_ref()));
        {
            let mut tx = kv.begin();
            tx.put(b"b", b"2").unwrap();
            tx.put(b"c", b"3").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(kv.get(b"b").as_deref(), Some(b"2".as_ref()));
        assert_eq!(kv.get(b"c").as_deref(), Some(b"3".as_ref()));
        kv.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
