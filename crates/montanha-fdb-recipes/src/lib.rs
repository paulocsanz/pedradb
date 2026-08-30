//! FoundationDB [design recipes](https://apple.github.io/foundationdb/design-recipes.html)
//! ported onto Montanha [`Transaction`](pedradb_store::Transaction).
//!
//! Purpose: **find bugs in Montanha** (SI, OCC, range, multi-key TX) by exercising
//! the same data models FDB layers use — not to reimplement Record Layer.
//!
//! Recipes: subspaces, tables, simple indexes, queues, multimaps, priority queues.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod children_kernel;
mod fields_kernel;
mod pack_kernel;

pub use children_kernel::{
    key_in_half_open, next_byte_in_packed_children, next_byte_in_packed_children_as_is,
    packed_children_end, packed_children_end_as_is, packed_children_start, PACKED_CHILD_END,
    PACKED_CHILD_END_AS_IS, PACKED_CHILD_SEP,
};
pub use fields_kernel::{
    child_bytes_after, child_bytes_after_as_is, decode_fields, decode_pair_first_nul,
    encode_fields, encode_fields_as_is, field_kept, field_kept_as_is,
};
pub use pack_kernel::{pack_cut_tag, pack_cut_tag_as_is, PACK_CUT_SEP};

use pedradb_store::{Result, StoreCluster, StoreError, Transaction};

/// Live value? Empty bytes are treated as **cleared/tombstone** (FDB clear removes
/// the key from ranges; Montanha stages empty puts — recipes filter them).
#[must_use]
fn is_live(v: &[u8]) -> bool {
    !v.is_empty()
}

/// Best-effort decode: length-prefixed first, then legacy `sep=0x00` layout.
#[must_use]
fn decode_pair_compat(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    if let Some(parts) = decode_fields(raw, 2) {
        return (parts[0].clone(), parts[1].clone());
    }
    decode_pair_first_nul(raw)
}

#[must_use]
fn decode_triple_compat(raw: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    if let Some(parts) = decode_fields(raw, 3) {
        return (parts[0].clone(), parts[1].clone(), parts[2].clone());
    }
    let parts: Vec<&[u8]> = raw.split(|&b| b == 0).collect();
    let a = parts.first().map(|p| p.to_vec()).unwrap_or_default();
    let b = parts.get(1).map(|p| p.to_vec()).unwrap_or_default();
    let c = if parts.len() > 2 {
        let mut v = parts[2].to_vec();
        for p in &parts[3..] {
            v.push(0);
            v.extend_from_slice(p);
        }
        v
    } else {
        Vec::new()
    };
    (a, b, c)
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

    /// Nested subspace: `self/part` (length-prefixed component — F62).
    #[must_use]
    pub fn sub(&self, part: impl AsRef<[u8]>) -> Self {
        let mut p = self.prefix.clone();
        Self::push_component(&mut p, part.as_ref());
        Self { prefix: p }
    }

    /// Pack a key under this subspace: `prefix || (0x00 || u32be len || part)*`.
    ///
    /// F62: raw `0x00 || part` collides when a component embeds `0x00`
    /// (`pack([a\\0b, c]) == pack([a, b\\0c])`). Length-prefix each part.
    #[must_use]
    pub fn pack(&self, parts: &[&[u8]]) -> Vec<u8> {
        let mut k = self.prefix.clone();
        for p in parts {
            Self::push_component(&mut k, p);
        }
        k
    }

    fn push_component(buf: &mut Vec<u8>, part: &[u8]) {
        buf.push(PACK_CUT_SEP);
        let n = pack_cut_tag(u32::try_from(part.len()).expect("component len fits u32"));
        buf.extend_from_slice(&n.to_be_bytes());
        buf.extend_from_slice(part);
    }

    /// Decode one length-prefixed component after a `0x00` separator.
    fn take_component(rest: &[u8]) -> Option<(&[u8], &[u8])> {
        if rest.len() < 4 {
            return None;
        }
        let n = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
        if rest.len() < 4 + n {
            return None;
        }
        Some((&rest[4..4 + n], &rest[4 + n..]))
    }

    /// Inclusive start of range for this subspace.
    #[must_use]
    pub fn range_start(&self) -> Vec<u8> {
        packed_children_start(&self.prefix)
    }

    /// Exclusive end of packed children: `prefix || 0x01`.
    ///
    /// Keys are `prefix || 0x00 || rest`. `prefix || 0xff` is **not** that
    /// interval — a longer sibling component (zip `900` vs `90`) sorts inside
    /// `[pack(90), pack(90)||0xff)` (F59).
    #[must_use]
    pub fn range_end(&self) -> Vec<u8> {
        packed_children_end(&self.prefix)
    }

    /// Half-open range of packed children of `self.pack(parts)`:
    /// `[pack || 0x00, pack || 0x01)`.
    #[must_use]
    pub fn children_range(&self, parts: &[&[u8]]) -> (Vec<u8>, Vec<u8>) {
        let packed = self.pack(parts);
        (packed_children_start(&packed), packed_children_end(&packed))
    }

    /// Immediate packed child of `self.pack(parts)` inside `key`.
    ///
    /// F60/F62: does not split on raw `0x00` inside the component; reads the
    /// length-prefixed field after `pack || 0x00`.
    #[must_use]
    pub fn child_suffix(&self, parts: &[&[u8]], key: &[u8]) -> Option<Vec<u8>> {
        let (start, _) = self.children_range(parts);
        let rest = child_bytes_after(key, start.as_slice())?;
        let (comp, _) = Self::take_component(rest)?;
        Some(comp.to_vec())
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
    pub fn get(&self, cluster: &StoreCluster, row: &[u8], col: &[u8]) -> Result<Option<Vec<u8>>> {
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
            // payload: length-prefixed (zip, name) — F60; legacy zip\0name still read.
            let (old_zip, _) = decode_pair_compat(&old);
            if old_zip.as_slice() != zipcode {
                tr.clear(self.index_key(&old_zip, id))?;
            }
        }
        let payload = encode_fields(&[zipcode, name]);
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
        Ok(Some(decode_pair_compat(&raw)))
    }

    /// IDs with given zipcode via index range (FDB recipe `get_user_IDs_in_region`).
    ///
    /// # Errors
    /// Range / store.
    pub fn ids_in_zip(&self, cluster: &StoreCluster, zipcode: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let (start, end) = self.zip_index.children_range(&[zipcode]);
        let pairs = tr.get_range(cluster, &start, &end)?;
        let mut ids = Vec::new();
        for (k, v) in pairs {
            if !is_live(&v) {
                continue; // tombstone after clear
            }
            if let Some(id) = self.zip_index.child_suffix(&[zipcode], &k) {
                ids.push(id);
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

    /// Missing counter → 0; present but non-integer → Err (F112, same class as F109).
    fn parse_u64(raw: Option<Vec<u8>>) -> Result<u64> {
        match raw {
            None => Ok(0),
            Some(raw) => {
                let s = std::str::from_utf8(&raw)
                    .map_err(|_| StoreError::Msg("queue counter: bad utf8".into()))?;
                s.parse::<u64>()
                    .map_err(|_| StoreError::Msg("queue counter: bad integer".into()))
            }
        }
    }

    /// Push to back.
    ///
    /// # Errors
    /// Commit or corrupt head/tail counter (F112).
    pub fn push(&self, cluster: &mut StoreCluster, value: &[u8]) -> Result<u64> {
        let mut tr = cluster.begin();
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?)?;
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
    /// Commit or corrupt head/tail counter (F112).
    pub fn pop(&self, cluster: &mut StoreCluster) -> Result<Option<Vec<u8>>> {
        let mut tr = cluster.begin();
        let head = Self::parse_u64(tr.get(cluster, self.head_key())?)?;
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?)?;
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
    /// Store or corrupt head/tail counter (F112).
    pub fn peek(&self, cluster: &StoreCluster) -> Result<Option<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let head = Self::parse_u64(tr.get(cluster, self.head_key())?)?;
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?)?;
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
    pub fn insert(&self, cluster: &mut StoreCluster, key: &[u8], value: &[u8]) -> Result<u64> {
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
        let (start, end) = self.root.children_range(&[key]);
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
    pub fn push(&self, cluster: &mut StoreCluster, priority: u64, value: &[u8]) -> Result<u64> {
        let mut tr = cluster.begin();
        // F112: corrupt seq counter must not wrap to 0 and collide.
        let seq = Queue::parse_u64(tr.get(cluster, self.seq_meta.pack(&[b"n"]))?)?;
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
        Ok(pairs.into_iter().find(|(_, v)| is_live(v)).map(|(_, v)| v))
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

// ── Phase 3: Record Layer *seed* (not Apple Record Layer) ─────────────────

/// Thin record store: primary row + one secondary index in a single TX.
///
/// Stands in as a **Record Layer seed** for bug-hunt / substitution work.
/// Not a planner, not multi-type indexes, not Java RL.
pub struct RecordTable {
    /// Table name bytes (subspace).
    name: Vec<u8>,
    /// Secondary index column name.
    index_col: Vec<u8>,
}

impl RecordTable {
    /// Create a record table with a named secondary index column.
    #[must_use]
    pub fn new(table: impl AsRef<[u8]>, index_col: impl AsRef<[u8]>) -> Self {
        Self {
            name: table.as_ref().to_vec(),
            index_col: index_col.as_ref().to_vec(),
        }
    }

    fn row_key(&self, pk: &[u8]) -> Vec<u8> {
        Subspace::new(b"rec").sub(&self.name).sub(b"r").pack(&[pk])
    }

    fn idx_key(&self, idx_val: &[u8], pk: &[u8]) -> Vec<u8> {
        Subspace::new(b"rec")
            .sub(&self.name)
            .sub(b"i")
            .sub(&self.index_col)
            .pack(&[idx_val, pk])
    }

    /// Upsert record + maintain secondary index atomically.
    ///
    /// # Errors
    /// Conflict / store.
    pub fn upsert(
        &self,
        cluster: &mut StoreCluster,
        pk: &[u8],
        index_val: &[u8],
        payload: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        let rk = self.row_key(pk);
        // Drop old index entry if index value changed.
        if let Some(old) = tr.get(cluster, &rk)? {
            // payload: length-prefixed (index_val, body) — F60.
            let (old_iv, _) = decode_pair_compat(&old);
            if old_iv.as_slice() != index_val {
                tr.clear(self.idx_key(&old_iv, pk))?;
            }
        }
        let body = encode_fields(&[index_val, payload]);
        tr.set(&rk, &body)?;
        tr.set(self.idx_key(index_val, pk), b"\x01")?;
        tr.commit(cluster)
    }

    /// Unique secondary index: fail if `index_val` already points at a **different** pk.
    ///
    /// # Errors
    /// Conflict, or [`pedradb_store::StoreError::Msg`] when uniqueness is violated.
    pub fn upsert_unique(
        &self,
        cluster: &mut StoreCluster,
        pk: &[u8],
        index_val: &[u8],
        payload: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        let rk = self.row_key(pk);
        // Scan existing pks for this index value (range under index subspace).
        let holders = self.lookup_index_in_tx(&mut tr, cluster, index_val)?;
        for h in &holders {
            if h.as_slice() != pk {
                return Err(pedradb_store::StoreError::Msg(format!(
                    "unique index {}:{} already held by other pk",
                    String::from_utf8_lossy(&self.index_col),
                    String::from_utf8_lossy(index_val)
                )));
            }
        }
        if let Some(old) = tr.get(cluster, &rk)? {
            let (old_iv, _) = decode_pair_compat(&old);
            if old_iv.as_slice() != index_val {
                tr.clear(self.idx_key(&old_iv, pk))?;
            }
        }
        let body = encode_fields(&[index_val, payload]);
        tr.set(&rk, &body)?;
        tr.set(self.idx_key(index_val, pk), b"\x01")?;
        tr.commit(cluster)
    }

    /// Upsert with **two** secondary indexes in one TX (multi-index seed).
    ///
    /// Payload: length-prefixed `(iv1, iv2, body)` (F60). Indexes: primary
    /// `index_col` and `index2_col`.
    ///
    /// # Errors
    /// Conflict / store.
    pub fn upsert_two_indexes(
        &self,
        cluster: &mut StoreCluster,
        index2_col: &[u8],
        pk: &[u8],
        index_val: &[u8],
        index2_val: &[u8],
        payload: &[u8],
    ) -> Result<u64> {
        let mut tr = cluster.begin();
        let rk = self.row_key(pk);
        if let Some(old) = tr.get(cluster, &rk)? {
            let (old_iv1, old_iv2, _) = decode_triple_compat(&old);
            if old_iv1.as_slice() != index_val {
                tr.clear(self.idx_key(&old_iv1, pk))?;
            }
            if old_iv2.as_slice() != index2_val {
                tr.clear(self.idx2_key(index2_col, &old_iv2, pk))?;
            }
        }
        let body = encode_fields(&[index_val, index2_val, payload]);
        tr.set(&rk, &body)?;
        tr.set(self.idx_key(index_val, pk), b"\x01")?;
        tr.set(self.idx2_key(index2_col, index2_val, pk), b"\x01")?;
        tr.commit(cluster)
    }

    fn idx2_key(&self, col: &[u8], idx_val: &[u8], pk: &[u8]) -> Vec<u8> {
        Subspace::new(b"rec")
            .sub(&self.name)
            .sub(b"i")
            .sub(col)
            .pack(&[idx_val, pk])
    }

    fn lookup_index_in_tx(
        &self,
        tr: &mut Transaction,
        cluster: &StoreCluster,
        index_val: &[u8],
    ) -> Result<Vec<Vec<u8>>> {
        let idx = Subspace::new(b"rec")
            .sub(&self.name)
            .sub(b"i")
            .sub(&self.index_col);
        let (prefix, end) = idx.children_range(&[index_val]);
        let pairs = tr.get_range(cluster, &prefix, &end)?;
        let mut pks = Vec::new();
        for (k, v) in pairs {
            if !is_live(&v) {
                continue;
            }
            if let Some(pk) = idx.child_suffix(&[index_val], &k) {
                pks.push(pk);
            }
        }
        Ok(pks)
    }

    /// Lookup by primary key → (index_val, payload).
    ///
    /// # Errors
    /// Store.
    pub fn get_by_pk(
        &self,
        cluster: &StoreCluster,
        pk: &[u8],
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let Some(raw) = tr.get(cluster, self.row_key(pk))? else {
            return Ok(None);
        };
        Ok(Some(decode_pair_compat(&raw)))
    }

    /// Lookup PKs by secondary index value.
    ///
    /// # Errors
    /// Range / store.
    pub fn lookup_index(&self, cluster: &StoreCluster, index_val: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let idx = Subspace::new(b"rec")
            .sub(&self.name)
            .sub(b"i")
            .sub(&self.index_col);
        let (start, end) = idx.children_range(&[index_val]);
        let pairs = tr.get_range(cluster, &start, &end)?;
        let mut pks = Vec::new();
        for (k, v) in pairs {
            if !is_live(&v) {
                continue;
            }
            if let Some(pk) = idx.child_suffix(&[index_val], &k) {
                pks.push(pk);
            }
        }
        Ok(pks)
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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

    /// `pack(zip) || 0xff` is not a tuple prefix: zip `900` sorts inside
    /// `[pack(90), pack(90)||0xff)` and leaks into `ids_in_zip("90")`.
    /// FDB tuple layer terminates each element; this pack does not.
    #[test]
    fn simple_index_does_not_include_prefix_sibling_zip() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        u.set_user(&mut c, b"short", b"a", b"90").unwrap();
        u.set_user(&mut c, b"long", b"b", b"900").unwrap();
        assert_eq!(
            u.get_user(&c, b"long").unwrap().unwrap().0,
            b"900",
            "point get must still see the longer zip"
        );
        let in90 = u.ids_in_zip(&c, b"90").unwrap();
        assert!(
            in90.iter().any(|i| i == b"short"),
            "zip 90 missing own id: {in90:?}"
        );
        assert!(
            !in90.iter().any(|i| i == b"long"),
            "zip 90 range included sibling zip 900 (pack||0xff): {in90:?}"
        );
        let in900 = u.ids_in_zip(&c, b"900").unwrap();
        assert!(
            in900.iter().any(|i| i == b"long") && !in900.iter().any(|i| i == b"short"),
            "zip 900 must be exact, got {in900:?}"
        );
        // User id starting with 0xff is a packed *child* (`\0` then 0xff) — must stay.
        let ff = [0xff, b'z'];
        u.set_user(&mut c, &ff, b"c", b"90").unwrap();
        let in90 = u.ids_in_zip(&c, b"90").unwrap();
        assert!(
            in90.iter().any(|i| i.as_slice() == ff),
            "0xff user id must remain in zip 90: {in90:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F62: raw `0x00||part` pack collides when components embed NUL.
    #[test]
    fn pack_is_injective_when_components_contain_nul() {
        let s = Subspace::new(b"t");
        let a = s.pack(&[b"a\x00b", b"c"]);
        let b = s.pack(&[b"a", b"b\x00c"]);
        assert_ne!(
            a, b,
            "pack collision: [a\\0b,c] vs [a,b\\0c] both encode to {a:?}"
        );
        // Nested sub must also length-prefix.
        let s1 = Subspace::new(b"r").sub(b"a\x00b");
        let s2 = Subspace::new(b"r").sub(b"a").sub(b"b");
        assert_ne!(
            s1.as_bytes(),
            s2.as_bytes(),
            "sub collision for NUL vs nested"
        );
    }

    /// Value payload used to be `zip || 0x00 || name`. A zip containing `0x00`
    /// truncates on re-read so zip change clears the wrong index key and leaves
    /// a stale secondary entry (silent extra id under the old zip).
    #[test]
    fn set_user_zip_with_nul_clears_old_index_on_change() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        let zip_nul = [b'9', 0x00, b'0'];
        u.set_user(&mut c, b"u1", b"alice", &zip_nul).unwrap();
        assert!(
            u.ids_in_zip(&c, &zip_nul)
                .unwrap()
                .iter()
                .any(|i| i == b"u1"),
            "initial nul zip must list u1"
        );
        u.set_user(&mut c, b"u1", b"alice", b"99").unwrap();
        let stale = u.ids_in_zip(&c, &zip_nul).unwrap();
        assert!(
            stale.is_empty(),
            "zip change left stale index under nul zip (value\\0 split): {stale:?}"
        );
        assert!(
            u.ids_in_zip(&c, b"99").unwrap().iter().any(|i| i == b"u1"),
            "new zip must list u1"
        );
        let (z, n) = u.get_user(&c, b"u1").unwrap().expect("u1");
        assert_eq!(z, b"99");
        assert_eq!(n, b"alice");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catalog three-teeth plant. Direct `set_user_zip_with_nul_clears_old_index_on_change` is **not** this tooth.
    #[test]
    fn encode_fields_on_live_directory_is_not_ok() {
        let zip = [b'9', 0x00, b'0'];
        assert_ne!(
            encode_fields(&[&zip, b"alice"]),
            encode_fields_as_is(&[&zip, b"alice"]),
            "AS-IS dente: raw 0x00 join truncates zip"
        );
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        u.set_user(&mut c, b"u1", b"alice", &zip).unwrap();
        let (z, n) = u.get_user(&c, b"u1").unwrap().expect("u1");
        assert_eq!(z.as_slice(), zip.as_slice(), "live set_user must keep NUL in zip");
        assert_eq!(n, b"alice");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Packed children are `pack(zip) || 0x00 || id`. Splitting the key on the
    /// last `0x00` truncates an id that itself contains `0x00`.
    #[test]
    fn simple_index_round_trips_id_containing_nul() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        let id = [b'a', 0x00, b'b'];
        u.set_user(&mut c, &id, b"nul", b"90").unwrap();
        let got = u.get_user(&c, &id).unwrap();
        assert!(got.is_some(), "point get must see nul id");
        let in90 = u.ids_in_zip(&c, b"90").unwrap();
        assert!(
            in90.iter().any(|i| i.as_slice() == id),
            "ids_in_zip truncated nul id (rsplit 0x00): {in90:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Payload `zip || 0x00 || name` truncated a zip that contains NUL, so
    /// `set_user` cleared the wrong index key and left a stale hit.
    #[test]
    fn simple_index_nul_zip_does_not_leave_stale_on_update() {
        let (dir, mut c) = open3();
        let u = IndexedUsers::new();
        let zip = [b'9', 0x00, b'0'];
        u.set_user(&mut c, b"x", b"alice", &zip).unwrap();
        let (got_zip, name) = u.get_user(&c, b"x").unwrap().unwrap();
        assert_eq!(got_zip, zip, "get_user must round-trip zip with NUL");
        assert_eq!(name, b"alice");
        u.set_user(&mut c, b"x", b"alice", b"90").unwrap();
        let old = u.ids_in_zip(&c, &zip).unwrap();
        assert!(
            !old.iter().any(|i| i == b"x"),
            "stale index under NUL zip after move: {old:?}"
        );
        let now = u.ids_in_zip(&c, b"90").unwrap();
        assert!(now.iter().any(|i| i == b"x"), "moved zip missing: {now:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_index_round_trips_pk_containing_nul() {
        let (dir, mut c) = open3();
        let rec = RecordTable::new(b"acct", b"email");
        let pk = [b'p', 0x00, b'k'];
        rec.upsert_unique(&mut c, &pk, b"n@x", b"row").unwrap();
        let got = rec.lookup_index(&c, b"n@x").unwrap();
        assert!(
            got.iter().any(|p| p.as_slice() == pk),
            "record index truncated nul pk: {got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Index value with embedded NUL: change must drop the old index entry.
    #[test]
    fn record_index_val_with_nul_clears_on_change() {
        let (dir, mut c) = open3();
        let rec = RecordTable::new(b"acct2", b"email");
        let iv = [b'a', 0x00, b'@', b'x'];
        rec.upsert(&mut c, b"pk1", &iv, b"row").unwrap();
        assert!(rec
            .lookup_index(&c, &iv)
            .unwrap()
            .iter()
            .any(|p| p == b"pk1"));
        rec.upsert(&mut c, b"pk1", b"b@x", b"row").unwrap();
        let stale = rec.lookup_index(&c, &iv).unwrap();
        assert!(
            stale.is_empty(),
            "index_val with NUL left stale entry after change: {stale:?}"
        );
        assert!(rec
            .lookup_index(&c, b"b@x")
            .unwrap()
            .iter()
            .any(|p| p == b"pk1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multimap_get_all_does_not_include_prefix_sibling_key() {
        let (dir, mut c) = open3();
        let mm = Multimap::new(b"tags");
        mm.insert(&mut c, b"a", b"red").unwrap();
        mm.insert(&mut c, b"aa", b"blue").unwrap();
        let a = mm.get_all(&c, b"a").unwrap();
        assert_eq!(a, vec![b"red".to_vec()], "key a leaked sibling aa: {a:?}");
        let aa = mm.get_all(&c, b"aa").unwrap();
        assert_eq!(aa, vec![b"blue".to_vec()]);
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
        let (z0, z1) = u.zip_index.children_range(&[b"90000"]);
        let before = tr.get_range(&c, &z0, &z1).unwrap();
        let n_before = before.len();

        // Concurrent insert same zip
        u.set_user(&mut c, b"b", b"bob", b"90000").unwrap();

        let after = tr.get_range(&c, &z0, &z1).unwrap();
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

    /// F112: garbage tail/seq parsed as 0 and overwrote the first item.
    #[test]
    fn queue_corrupt_tail_does_not_reuse_seq_zero() {
        let (dir, mut c) = open3();
        let q = Queue::new(b"jobs");
        q.push(&mut c, b"first").unwrap();
        let tail = q.tail_key();
        c.put(&tail, b"xxx").unwrap();
        let err = q.push(&mut c, b"second");
        assert!(
            err.is_err(),
            "corrupt tail must fail closed, not overwrite seq 0: {err:?}"
        );
        let item0 = q.data.pack(&[format!("{:020}", 0).as_bytes()]);
        assert_eq!(
            c.get(&item0).unwrap().as_deref(),
            Some(b"first".as_ref()),
            "first queue item must survive corrupt tail"
        );
        let pq = PriorityQueue::new(b"tasks");
        pq.push(&mut c, 1, b"high").unwrap();
        let seqk = pq.seq_meta.pack(&[b"n"]);
        c.put(&seqk, b"yyy").unwrap();
        let err = pq.push(&mut c, 1, b"also-high");
        assert!(
            err.is_err(),
            "corrupt PQ seq must fail closed, not overwrite seq 0: {err:?}"
        );
        let p0 = pq.data.pack(&[
            1u64.to_be_bytes().as_slice(),
            format!("{:020}", 0).as_bytes(),
        ]);
        assert_eq!(
            c.get(&p0).unwrap().as_deref(),
            Some(b"high".as_ref()),
            "first PQ item must survive corrupt seq"
        );
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
        assert_eq!(
            pq.pop_min(&mut c).unwrap().as_deref(),
            Some(b"high".as_ref())
        );
        assert_eq!(
            pq.pop_min(&mut c).unwrap().as_deref(),
            Some(b"mid".as_ref())
        );
        assert_eq!(
            pq.pop_min(&mut c).unwrap().as_deref(),
            Some(b"low".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 3: Record Layer seed — concurrent upsert Conflict fail-closed.
    #[test]
    fn phase3_record_table_concurrent_conflict() {
        let (dir, mut c) = open3();
        let rec = RecordTable::new(b"users", b"email");
        rec.upsert(&mut c, b"1", b"a@b.c", b"alice").unwrap();

        let mut t1 = c.begin();
        let mut t2 = c.begin();
        let rk = rec.row_key(b"1");
        let _ = t1.get(&c, &rk).unwrap();
        let _ = t2.get(&c, &rk).unwrap();
        // Both try to change email index
        let mut body1 = b"x@y.z".to_vec();
        body1.push(0);
        body1.extend_from_slice(b"A");
        let mut body2 = b"p@q.r".to_vec();
        body2.push(0);
        body2.extend_from_slice(b"B");
        t1.set(&rk, &body1).unwrap();
        t1.set(rec.idx_key(b"x@y.z", b"1"), b"\x01").unwrap();
        t2.set(&rk, &body2).unwrap();
        t2.set(rec.idx_key(b"p@q.r", b"1"), b"\x01").unwrap();
        let r1 = t1.commit(&mut c);
        let r2 = t2.commit(&mut c);
        let ok = r1.is_ok() as u8 + r2.is_ok() as u8;
        assert_eq!(
            ok, 1,
            "exactly one record upsert must win, r1={r1:?} r2={r2:?}"
        );
        let (email, _) = rec.get_by_pk(&c, b"1").unwrap().expect("row");
        let by_email = rec.lookup_index(&c, &email).unwrap();
        assert!(
            by_email.iter().any(|p| p == b"1"),
            "index must match winning row email={email:?} pks={by_email:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phase3_record_table_index_lookup() {
        let (dir, mut c) = open3();
        let rec = RecordTable::new(b"item", b"sku");
        rec.upsert(&mut c, b"pk1", b"SKU-1", b"row1").unwrap();
        rec.upsert(&mut c, b"pk2", b"SKU-1", b"row2").unwrap();
        rec.upsert(&mut c, b"pk3", b"SKU-2", b"row3").unwrap();
        let pks = rec.lookup_index(&c, b"SKU-1").unwrap();
        assert_eq!(pks.len(), 2, "SKU-1 → two pks: {pks:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase C: unique secondary index reject + multi-index atomic maintain.
    #[test]
    fn phase3_record_unique_and_multi_index() {
        let (dir, mut c) = open3();
        let rec = RecordTable::new(b"acct", b"email");
        rec.upsert_unique(&mut c, b"1", b"a@x", b"alice").unwrap();
        let err = rec
            .upsert_unique(&mut c, b"2", b"a@x", b"bob")
            .expect_err("duplicate email must fail");
        assert!(
            err.to_string().contains("unique"),
            "expected unique error, got {err}"
        );
        // same pk may re-upsert unique
        rec.upsert_unique(&mut c, b"1", b"a@x", b"alice2").unwrap();

        rec.upsert_two_indexes(&mut c, b"phone", b"9", b"e@9", b"555", b"z")
            .unwrap();
        let by_email = rec.lookup_index(&c, b"e@9").unwrap();
        assert!(
            by_email.iter().any(|p| p == b"9"),
            "email index {by_email:?}"
        );
        // phone index via subspace pack
        let (phone_prefix, end) = Subspace::new(b"rec")
            .sub(b"acct")
            .sub(b"i")
            .sub(b"phone")
            .children_range(&[b"555"]);
        let mut tr = c.begin();
        let pairs = tr.get_range(&c, &phone_prefix, &end).unwrap();
        assert!(
            pairs
                .iter()
                .any(|(k, v)| !v.is_empty() && k.ends_with(b"9")),
            "phone index missing: {pairs:?}"
        );
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
        let h1 = Queue::parse_u64(t1.get(&c, &head_k).unwrap()).unwrap();
        let h2 = Queue::parse_u64(t2.get(&c, &head_k).unwrap()).unwrap();
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
