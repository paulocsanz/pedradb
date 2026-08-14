//! FoundationDB [design recipes](https://apple.github.io/foundationdb/design-recipes.html)
//! ported onto Montanha [`Transaction`](pedradb_store::Transaction).
//!
//! Purpose: **find bugs in Montanha** (SI, OCC, range, multi-key TX) by exercising
//! the same data models FDB layers use — not to reimplement Record Layer.
//!
//! Recipes: subspaces, tables, simple indexes, queues, multimaps, priority queues.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use pedradb_store::{Result, StoreCluster, Transaction};

/// Live value? Empty bytes are treated as **cleared/tombstone** (FDB clear removes
/// the key from ranges; Montanha stages empty puts — recipes filter them).
#[must_use]
fn is_live(v: &[u8]) -> bool {
    !v.is_empty()
}

// ── Subspace / tuple packing (minimal FDB-style ordered keys) ──────────────

/// Ordered key prefix for a named subspace (like FDB `Subspace`).
#[derive(Debug, Clone)]
pub struct Subspace {
    prefix: Vec<u8>,
}

impl Subspace {
    /// Subspace from raw bytes (must not collide with other prefixes).
    #[must_use]
    pub fn new(prefix: impl AsRef<[u8]>) -> Self {
        Self {
            prefix: prefix.as_ref().to_vec(),
        }
    }

    /// Nested subspace: `self/part`.
    #[must_use]
    pub fn sub(&self, part: impl AsRef<[u8]>) -> Self {
        let mut p = self.prefix.clone();
        p.push(0x00);
        p.extend_from_slice(part.as_ref());
        Self { prefix: p }
    }

    /// Pack a key under this subspace: `prefix\0part1\0part2...`.
    #[must_use]
    pub fn pack(&self, parts: &[&[u8]]) -> Vec<u8> {
        let mut k = self.prefix.clone();
        for p in parts {
            k.push(0x00);
            k.extend_from_slice(p);
        }
        k
    }

    /// Inclusive start of range for this subspace.
    #[must_use]
    pub fn range_start(&self) -> Vec<u8> {
        let mut s = self.prefix.clone();
        s.push(0x00);
        s
    }

    /// Exclusive end: first byte after prefix (FDB range end).
    #[must_use]
    pub fn range_end(&self) -> Vec<u8> {
        let mut e = self.prefix.clone();
        // successor of prefix as half-open end for keys under this subspace
        e.push(0xff);
        e
    }

    /// Prefix bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.prefix
    }
}

// ── Tables (sparse row: subspace + row_id + column → value) ────────────────

/// Sparse table: keys `(table, row_id, column) = value`.
pub struct Table {
    /// Row data subspace.
    pub rows: Subspace,
}

impl Table {
    /// Named table.
    #[must_use]
    pub fn new(name: &[u8]) -> Self {
        Self {
            rows: Subspace::new(b"tbl").sub(name),
        }
    }

    fn cell_key(&self, row: &[u8], col: &[u8]) -> Vec<u8> {
        self.rows.pack(&[row, col])
    }

    /// Set a cell inside an open transaction.
    ///
    /// # Errors
    /// TX size limits.
    pub fn set_cell(
        &self,
        tr: &mut Transaction,
        row: &[u8],
        col: &[u8],
        value: &[u8],
    ) -> Result<()> {
        tr.set(self.cell_key(row, col), value)
    }

    /// Get a cell at snapshot.
    ///
    /// # Errors
    /// Store get.
    pub fn get_cell(
        &self,
        tr: &mut Transaction,
        cluster: &StoreCluster,
        row: &[u8],
        col: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        tr.get(cluster, self.cell_key(row, col))
    }

    /// Upsert one cell in its own TX.
    ///
    /// # Errors
    /// Commit failures.
    pub fn put(
        &self,
        cluster: &mut StoreCluster,
        row: &[u8],
        col: &[u8],
        value: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        self.set_cell(&mut tr, row, col, value)?;
        tr.commit(cluster)
    }

    /// Read one cell (snapshot TX).
    ///
    /// # Errors
    /// Store errors.
    pub fn get(
        &self,
        cluster: &StoreCluster,
        row: &[u8],
        col: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        self.get_cell(&mut tr, cluster, row, col)
    }
}

// ── Simple indexes (FDB simple-indexes recipe) ─────────────────────────────

/// Primary row + secondary index maintained in one TX.
///
/// Primary: `(user, id) = payload`  
/// Index:   `(zip_idx, zip, id) = empty`
pub struct IndexedUsers {
    /// Primary subspace.
    pub user: Subspace,
    /// Zipcode index subspace.
    pub zip_index: Subspace,
}

impl IndexedUsers {
    /// Default subspaces matching the FDB recipe shape.
    #[must_use]
    pub fn new() -> Self {
        Self {
            user: Subspace::new(b"user"),
            zip_index: Subspace::new(b"zipcode_index"),
        }
    }

    fn user_key(&self, id: &[u8]) -> Vec<u8> {
        self.user.pack(&[id])
    }

    fn index_key(&self, zip: &[u8], id: &[u8]) -> Vec<u8> {
        self.zip_index.pack(&[zip, id])
    }

    /// Transactional set: user row + zip index (FDB recipe `set_user`).
    ///
    /// # Errors
    /// Commit / conflict.
    pub fn set_user(
        &self,
        cluster: &mut StoreCluster,
        id: &[u8],
        name: &[u8],
        zipcode: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        // If replacing, drop old index entry when zip changes (read old if present).
        if let Some(old) = tr.get(cluster, self.user_key(id))? {
            // payload layout: zip\0name
            if let Some(sep) = old.iter().position(|&b| b == 0) {
                let old_zip = &old[..sep];
                if old_zip != zipcode {
                    tr.clear(self.index_key(old_zip, id))?;
                }
            }
        }
        let mut payload = zipcode.to_vec();
        payload.push(0);
        payload.extend_from_slice(name);
        tr.set(self.user_key(id), &payload)?;
        // Non-empty marker: Montanha `clear` stages empty values as tombstones;
        // FDB recipe uses '' for index presence — we use \x01 so ranges can skip tombs.
        tr.set(self.index_key(zipcode, id), b"\x01")?;
        tr.commit(cluster)
    }

    /// Lookup user by id.
    ///
    /// # Errors
    /// Store.
    pub fn get_user(
        &self,
        cluster: &StoreCluster,
        id: &[u8],
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let Some(raw) = tr.get(cluster, self.user_key(id))? else {
            return Ok(None);
        };
        let sep = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let zip = raw[..sep].to_vec();
        let name = if sep < raw.len() {
            raw[sep + 1..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Some((zip, name)))
    }

    /// IDs with given zipcode via index range (FDB recipe `get_user_IDs_in_region`).
    ///
    /// # Errors
    /// Range / store.
    pub fn ids_in_zip(&self, cluster: &StoreCluster, zipcode: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let start = self.zip_index.pack(&[zipcode]);
        let mut end = start.clone();
        end.push(0xff);
        let pairs = tr.get_range(cluster, &start, &end)?;
        let mut ids = Vec::new();
        for (k, v) in pairs {
            if !is_live(&v) {
                continue; // tombstone after clear
            }
            // unpack last component as id
            if let Some(pos) = k.iter().rposition(|&b| b == 0) {
                ids.push(k[pos + 1..].to_vec());
            }
        }
        Ok(ids)
    }
}

impl Default for IndexedUsers {
    fn default() -> Self {
        Self::new()
    }
}

// ── Queue (FIFO) ───────────────────────────────────────────────────────────

/// FIFO queue: keys `(q, seq) = value` with meta `(q, meta, head|tail)`.
pub struct Queue {
    data: Subspace,
    meta: Subspace,
}

impl Queue {
    /// Named queue.
    #[must_use]
    pub fn new(name: &[u8]) -> Self {
        let root = Subspace::new(b"queue").sub(name);
        Self {
            data: root.sub(b"d"),
            meta: root.sub(b"m"),
        }
    }

    fn head_key(&self) -> Vec<u8> {
        self.meta.pack(&[b"head"])
    }

    fn tail_key(&self) -> Vec<u8> {
        self.meta.pack(&[b"tail"])
    }

    fn parse_u64(raw: Option<Vec<u8>>) -> u64 {
        raw.and_then(|b| {
            std::str::from_utf8(&b)
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0)
    }

    /// Push to back.
    ///
    /// # Errors
    /// Commit.
    pub fn push(&self, cluster: &mut StoreCluster, value: &[u8]) -> Result<u64> {
        let mut tr = cluster.begin();
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?);
        let seq = tail;
        let item = self.data.pack(&[format!("{seq:020}").as_bytes()]);
        tr.set(&item, value)?;
        tr.set(self.tail_key(), (tail + 1).to_string().into_bytes())?;
        if tr.get(cluster, self.head_key())?.is_none() {
            tr.set(self.head_key(), b"0")?;
        }
        tr.commit(cluster)
    }

    /// Pop from front. Returns `None` if empty.
    ///
    /// # Errors
    /// Commit.
    pub fn pop(&self, cluster: &mut StoreCluster) -> Result<Option<Vec<u8>>> {
        let mut tr = cluster.begin();
        let head = Self::parse_u64(tr.get(cluster, self.head_key())?);
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?);
        if head >= tail {
            // empty — commit empty read TX? no writes; skip commit
            return Ok(None);
        }
        let item = self.data.pack(&[format!("{head:020}").as_bytes()]);
        let val = tr.get(cluster, &item)?;
        tr.clear(&item)?;
        tr.set(self.head_key(), (head + 1).to_string().into_bytes())?;
        tr.commit(cluster)?;
        Ok(val)
    }

    /// Peek front without remove.
    ///
    /// # Errors
    /// Store.
    pub fn peek(&self, cluster: &StoreCluster) -> Result<Option<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let head = Self::parse_u64(tr.get(cluster, self.head_key())?);
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?);
        if head >= tail {
            return Ok(None);
        }
        let item = self.data.pack(&[format!("{head:020}").as_bytes()]);
        tr.get(cluster, &item)
    }
}

// ── Multimap ───────────────────────────────────────────────────────────────

/// Multimap: `(mm, key, value_id) = value` for multi-values per key.
pub struct Multimap {
    root: Subspace,
}

impl Multimap {
    /// Named multimap.
    #[must_use]
    pub fn new(name: &[u8]) -> Self {
        Self {
            root: Subspace::new(b"mm").sub(name),
        }
    }

    /// Insert value under key (value used as tie-break id if unique).
    ///
    /// # Errors
    /// Commit.
    pub fn insert(
        &self,
        cluster: &mut StoreCluster,
        key: &[u8],
        value: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        // key: root/key/value so same value is idempotent
        let k = self.root.pack(&[key, value]);
        tr.set(&k, value)?;
        tr.commit(cluster)
    }

    /// All values for key.
    ///
    /// # Errors
    /// Range.
    pub fn get_all(&self, cluster: &StoreCluster, key: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let start = self.root.pack(&[key]);
        let mut end = start.clone();
        end.push(0xff);
        let pairs = tr.get_range(cluster, &start, &end)?;
        Ok(pairs
            .into_iter()
            .filter(|(_, v)| is_live(v))
            .map(|(_, v)| v)
            .collect())
    }
}

// ── Priority queue (min) ───────────────────────────────────────────────────

/// Min-priority queue: keys `(pq, priority, seq) = value`.
pub struct PriorityQueue {
    data: Subspace,
    seq_meta: Subspace,
}

impl PriorityQueue {
    /// Named PQ.
    #[must_use]
    pub fn new(name: &[u8]) -> Self {
        let root = Subspace::new(b"pq").sub(name);
        Self {
            data: root.sub(b"d"),
            seq_meta: root.sub(b"s"),
        }
    }

    /// Push with priority (lower = higher priority). `prio` is big-endian u64 in key.
    ///
    /// # Errors
    /// Commit.
    pub fn push(
        &self,
        cluster: &mut StoreCluster,
        priority: u64,
        value: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        let seq = tr
            .get(cluster, self.seq_meta.pack(&[b"n"]))?
            .and_then(|b| {
                std::str::from_utf8(&b)
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(0u64);
        let prio_bytes = priority.to_be_bytes();
        let seq_bytes = format!("{seq:020}");
        let k = self
            .data
            .pack(&[prio_bytes.as_slice(), seq_bytes.as_bytes()]);
        tr.set(&k, value)?;
        tr.set(
            self.seq_meta.pack(&[b"n"]),
            (seq + 1).to_string().into_bytes(),
        )?;
        tr.commit(cluster)
    }

    /// Peek min without remove.
    ///
    /// # Errors
    /// Range.
    pub fn peek_min(&self, cluster: &StoreCluster) -> Result<Option<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let start = self.data.range_start();
        let end = self.data.range_end();
        let pairs = tr.get_range(cluster, &start, &end)?;
        Ok(pairs
            .into_iter()
            .find(|(_, v)| is_live(v))
            .map(|(_, v)| v))
    }

    /// Pop min.
    ///
    /// # Errors
    /// Commit.
    pub fn pop_min(&self, cluster: &mut StoreCluster) -> Result<Option<Vec<u8>>> {
        let mut tr = cluster.begin();
        let start = self.data.range_start();
        let end = self.data.range_end();
        let pairs = tr.get_range(cluster, &start, &end)?;
        let Some((k, v)) = pairs.into_iter().find(|(_, v)| is_live(v)) else {
            return Ok(None);
        };
        tr.clear(&k)?;
        tr.commit(cluster)?;
        Ok(Some(v))
    }
}

// ── Tests (adversarial: must fail if Montanha SI/OCC regresses) ────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_store::{StoreCluster, StoreError};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("fdb-recipes-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn open3() -> (std::path::PathBuf, StoreCluster) {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(100).unwrap();
        (dir, c)
    }

    #[test]
    fn table_sparse_cells() {
        let (dir, mut c) = open3();
        let t = Table::new(b"orders");
        t.put(&mut c, b"1", b"sku", b"widget").unwrap();
        t.put(&mut c, b"1", b"qty", b"3").unwrap();
        assert_eq!(
            t.get(&c, b"1", b"sku").unwrap().as_deref(),
            Some(b"widget".as_ref())
        );
        assert_eq!(
            t.get(&c, b"1", b"qty").unwrap().as_deref(),
            Some(b"3".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn simple_index_lookup_and_atomic_with_primary() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        u.set_user(&mut c, b"u1", b"alice", b"22182").unwrap();
        u.set_user(&mut c, b"u2", b"bob", b"22182").unwrap();
        u.set_user(&mut c, b"u3", b"cara", b"10001").unwrap();

        let ids = u.ids_in_zip(&c, b"22182").unwrap();
        assert_eq!(ids.len(), 2, "index must list both users in zip: {ids:?}");
        assert!(ids.iter().any(|i| i == b"u1"));
        assert!(ids.iter().any(|i| i == b"u2"));

        let (zip, name) = u.get_user(&c, b"u1").unwrap().expect("u1");
        assert_eq!(zip, b"22182");
        assert_eq!(name, b"alice");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Concurrent writer must not leave index without primary (OCC / multi-key TX).
    #[test]
    fn simple_index_concurrent_set_no_orphan_index() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        u.set_user(&mut c, b"x", b"old", b"11111").unwrap();

        // TX A: read user, will rewrite zip
        let mut ta = c.begin();
        let _ = ta.get(&c, u.user.pack(&[b"x"])).unwrap();
        // Concurrent B commits new user row+index
        u.set_user(&mut c, b"x", b"new", b"22222").unwrap();

        // A tries stale write of primary+index
        let mut payload = b"11111".to_vec();
        payload.push(0);
        payload.extend_from_slice(b"stale");
        ta.set(u.user.pack(&[b"x"]), &payload).unwrap();
        ta.set(u.zip_index.pack(&[b"11111", b"x"]), b"").unwrap();
        let err = ta.commit(&mut c);
        assert!(
            matches!(err, Err(StoreError::Conflict)),
            "stale index update must Conflict, got {err:?}"
        );

        // Winner is B: zip 22222, no orphan-only consistency break
        let (zip, name) = u.get_user(&c, b"x").unwrap().unwrap();
        assert_eq!(zip, b"22222");
        assert_eq!(name, b"new");
        let in_old = u.ids_in_zip(&c, b"11111").unwrap();
        assert!(
            !in_old.iter().any(|i| i == b"x"),
            "must not list x under old zip after B won"
        );
        let in_new = u.ids_in_zip(&c, b"22222").unwrap();
        assert!(in_new.iter().any(|i| i == b"x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Snapshot isolation: index read must not see concurrent commit mid-TX.
    #[test]
    fn simple_index_snapshot_hides_concurrent_insert() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        u.set_user(&mut c, b"a", b"alice", b"90000").unwrap();

        let mut tr = c.begin();
        let before = tr
            .get_range(
                &c,
                &u.zip_index.pack(&[b"90000"]),
                &{
                    let mut e = u.zip_index.pack(&[b"90000"]);
                    e.push(0xff);
                    e
                },
            )
            .unwrap();
        let n_before = before.len();

        // Concurrent insert same zip
        u.set_user(&mut c, b"b", b"bob", b"90000").unwrap();

        let after = tr
            .get_range(
                &c,
                &u.zip_index.pack(&[b"90000"]),
                &{
                    let mut e = u.zip_index.pack(&[b"90000"]);
                    e.push(0xff);
                    e
                },
            )
            .unwrap();
        assert_eq!(
            after.len(),
            n_before,
            "SI: range inside TX must not grow after concurrent insert (got {} vs {})",
            after.len(),
            n_before
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_fifo() {
        let (dir, mut c) = open3();
        let q = Queue::new(b"jobs");
        q.push(&mut c, b"first").unwrap();
        q.push(&mut c, b"second").unwrap();
        assert_eq!(q.peek(&c).unwrap().as_deref(), Some(b"first".as_ref()));
        assert_eq!(q.pop(&mut c).unwrap().as_deref(), Some(b"first".as_ref()));
        assert_eq!(q.pop(&mut c).unwrap().as_deref(), Some(b"second".as_ref()));
        assert!(q.pop(&mut c).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multimap_multi_values() {
        let (dir, mut c) = open3();
        let m = Multimap::new(b"tags");
        m.insert(&mut c, b"post1", b"rust").unwrap();
        m.insert(&mut c, b"post1", b"db").unwrap();
        m.insert(&mut c, b"post2", b"rust").unwrap();
        let v = m.get_all(&c, b"post1").unwrap();
        assert_eq!(v.len(), 2);
        assert!(v.iter().any(|x| x == b"rust"));
        assert!(v.iter().any(|x| x == b"db"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn priority_queue_min_order() {
        let (dir, mut c) = open3();
        let pq = PriorityQueue::new(b"tasks");
        pq.push(&mut c, 10, b"low").unwrap();
        pq.push(&mut c, 1, b"high").unwrap();
        pq.push(&mut c, 5, b"mid").unwrap();
        assert_eq!(pq.peek_min(&c).unwrap().as_deref(), Some(b"high".as_ref()));
        assert_eq!(pq.pop_min(&mut c).unwrap().as_deref(), Some(b"high".as_ref()));
        assert_eq!(pq.pop_min(&mut c).unwrap().as_deref(), Some(b"mid".as_ref()));
        assert_eq!(pq.pop_min(&mut c).unwrap().as_deref(), Some(b"low".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_concurrent_push_pop_conflict_or_serialize() {
        let (dir, mut c) = open3();
        let q = Queue::new(b"c");
        q.push(&mut c, b"only").unwrap();

        let mut t1 = c.begin();
        let mut t2 = c.begin();
        // both try to pop
        let head_k = q.meta.pack(&[b"head"]);
        let h1 = Queue::parse_u64(t1.get(&c, &head_k).unwrap());
        let h2 = Queue::parse_u64(t2.get(&c, &head_k).unwrap());
        assert_eq!(h1, h2);
        let item = q.data.pack(&[format!("{h1:020}").as_bytes()]);
        let _ = t1.get(&c, &item).unwrap();
        let _ = t2.get(&c, &item).unwrap();
        t1.clear(&item).unwrap();
        t1.set(&head_k, (h1 + 1).to_string().into_bytes()).unwrap();
        t2.clear(&item).unwrap();
        t2.set(&head_k, (h2 + 1).to_string().into_bytes()).unwrap();
        let r1 = t1.commit(&mut c);
        let r2 = t2.commit(&mut c);
        let ok = r1.is_ok() as u8 + r2.is_ok() as u8;
        assert_eq!(ok, 1, "exactly one pop TX must commit, r1={r1:?} r2={r2:?}");
        // queue empty
        assert!(q.pop(&mut c).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
