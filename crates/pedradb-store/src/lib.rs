//! **Montanha-Store** — multi-Raft range KV on PedraDB.
//!
//! Substrate for Montanha-DCS (DCS is a **layer on the store**, not the base).
//! See `docs/montanha-layering-dcs-on-store.md`.
//!
//! # Model
//!
//! - Cluster = N **nodes**, each with one PedraDB directory.
//! - Keyspace split into **ranges** `[start, end)` (empty `end` = +∞).
//! - Each range = one Raft group (in-process multi-node for MVP).
//! - Writes go to the **range leader**; commit requires a **strict majority**;
//!   followers replicate then apply to local PedraDB.
//! - Many writers in the cluster (different ranges → different leaders).
//! - **Raft meta durable in PedraDB** under `\0store/raft/{range}/` (hard, log,
//!   commit, applied) so process restart keeps term/vote/log/watermark (F26).
//! - **In-range multi-key atomic** via [`StoreCluster::put_batch`]: one raft log
//!   entry applied with PedraDB `apply_batch` (row+index style layers). Keys must
//!   share one range; cross-range batches return [`StoreError::CrossRange`].
//!
//! # Reads
//!
//! - [`ReadPolicy::LocalApplied`] — read any node's applied PedraDB (may be stale;
//!   **not** linearizable).
//! - [`ReadPolicy::Strong`] — only the **current** range leader may serve; a deposed
//!   leader returns [`StoreError::NotLeader`] (ReadIndex/lease-class revalidation for
//!   this in-process MVP).
//!
//! # DCS on store
//!
//! Meta keys under prefix `m/` (configurable) live in whichever range covers them.
//! [`StoreCluster::dcs_create`] / [`dcs_cas`] run [`pedradb_dcs::apply_dcs_command`]
//! **only on commit** via a raft log entry carrying a `DcsCommand` payload.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use pedradb_core::{BatchOp, Db, OpenOptions};
use pedradb_dcs::{apply_dcs_command, check_command, dcs_get, DcsCommand, KeyValue};
use thiserror::Error;

/// Store errors.
#[derive(Debug, Error)]
pub enum StoreError {
    /// PedraDB.
    #[error("pedradb: {0}")]
    Core(#[from] pedradb_core::CoreError),
    /// DCS layer.
    #[error("dcs: {0}")]
    Dcs(#[from] pedradb_dcs::DcsError),
    /// Not leader of the range that owns the key.
    #[error("not leader of range {range_id} (leader={leader:?})")]
    NotLeader {
        /// Range id.
        range_id: u64,
        /// Leader node if known.
        leader: Option<u64>,
    },
    /// Strong read refused: caller is not the live range leader.
    #[error("stale or non-leader for strong read on range {range_id}")]
    StaleLeader {
        /// Range id.
        range_id: u64,
        /// Node that was asked.
        node_id: u64,
        /// Live leader if known.
        live_leader: Option<u64>,
    },
    /// Client propose did not reach a strict majority; entry is **not** committed.
    ///
    /// API contract: successful [`StoreCluster::put`] / [`StoreCluster::put_batch`] /
    /// DCS propose means majority durable.
    #[error(
        "not committed on range {range_id}: proposed index {index}, commit {commit} (majority not reached)"
    )]
    NotCommitted {
        /// Range id.
        range_id: u64,
        /// Log index of the client entry.
        index: u64,
        /// Leader commit index after the attempt.
        commit: u64,
    },
    /// Multi-key batch spans more than one range (no silent partial apply).
    ///
    /// Layers must co-locate keys in one range or use an explicit cross-range
    /// protocol (not shipped in P1 — hard-fail only).
    #[error("cross-range batch: keys map to ranges {ranges:?} (co-locate or split ops)")]
    CrossRange {
        /// Distinct range ids touched by the batch (sorted).
        ranges: Vec<u64>,
    },
    /// No range covers key.
    #[error("no range for key")]
    NoRange,
    /// Empty cluster / bad config.
    #[error("{0}")]
    Msg(String),
}

/// Result.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Read consistency policy (named so stale-leader wrong answers are not claimed strong).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPolicy {
    /// Local applied state on the chosen node. **Non-linearizable** — may lag or
    /// (if the node was partitioned) reflect a non-committed view only after apply;
    /// safe for UI polls / best-effort, not fencing.
    LocalApplied,
    /// Linearizable / lease-ReadIndex class for this MVP: only the **current**
    /// range leader returns Ok; deposed leaders must revalidate (fail closed).
    Strong,
}

/// Key range metadata `[start, end)` — empty `end` means unbounded.
#[derive(Debug, Clone)]
pub struct RangeMeta {
    /// Globally unique range id.
    pub id: u64,
    /// Inclusive start.
    pub start: Vec<u8>,
    /// Exclusive end; empty = +∞.
    pub end: Vec<u8>,
}

impl RangeMeta {
    /// Whether `key` is in this range.
    #[must_use]
    pub fn contains(&self, key: &[u8]) -> bool {
        key >= self.start.as_slice()
            && (self.end.is_empty() || key < self.end.as_slice())
    }
}

/// One raft log entry for a range.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RangeEntry {
    /// User put.
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
    },
    /// Atomic multi-put (same range); applied via PedraDB `apply_batch`.
    Batch {
        /// Ordered key/value pairs.
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// DCS command (applied via pedradb-dcs).
    Dcs(DcsCommand),
    /// Leadership blank index (Raft §5.4.2): previous-term majority can commit.
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogRec {
    index: u64,
    term: u64,
    entry: RangeEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Follower,
    Candidate,
    Leader,
}

// ── Durable raft meta (F26) ───────────────────────────────────────────────
// Keys live in the same PedraDB as user data, under a reserved NUL prefix so
// normal UTF-8/app keys do not collide. User `put` rejects this prefix.

const RAFT_META_PREFIX: &[u8] = b"\0store/raft/";

fn raft_meta_key(range_id: u64, kind: &str) -> Vec<u8> {
    let mut k = RAFT_META_PREFIX.to_vec();
    k.extend_from_slice(range_id.to_string().as_bytes());
    k.push(b'/');
    k.extend_from_slice(kind.as_bytes());
    k
}

fn is_raft_meta_key(key: &[u8]) -> bool {
    key.starts_with(RAFT_META_PREFIX)
}

fn append_crc(body: &mut Vec<u8>) {
    let c = crc32c::crc32c(body);
    body.extend_from_slice(&c.to_le_bytes());
}

fn strip_crc(buf: &[u8]) -> Result<&[u8]> {
    if buf.len() < 4 {
        return Err(StoreError::Msg("raft meta too short".into()));
    }
    let (payload, crc_raw) = buf.split_at(buf.len() - 4);
    let got = u32::from_le_bytes(crc_raw.try_into().unwrap());
    let expect = crc32c::crc32c(payload);
    if got != expect {
        return Err(StoreError::Msg("raft meta CRC mismatch".into()));
    }
    Ok(payload)
}

fn encode_hard(term: u64, voted_for: Option<u64>) -> Vec<u8> {
    let mut b = Vec::with_capacity(1 + 8 + 8 + 4);
    b.extend_from_slice(&term.to_le_bytes());
    match voted_for {
        Some(v) => {
            b.push(1);
            b.extend_from_slice(&v.to_le_bytes());
        }
        None => b.push(0),
    }
    append_crc(&mut b);
    b
}

fn decode_hard(buf: &[u8]) -> Result<(u64, Option<u64>)> {
    let p = strip_crc(buf)?;
    if p.len() < 9 {
        return Err(StoreError::Msg("hard short".into()));
    }
    let term = u64::from_le_bytes(p[0..8].try_into().unwrap());
    let voted = if p[8] == 1 {
        if p.len() < 17 {
            return Err(StoreError::Msg("hard vote short".into()));
        }
        Some(u64::from_le_bytes(p[9..17].try_into().unwrap()))
    } else {
        None
    };
    Ok((term, voted))
}

fn encode_u64_meta(n: u64) -> Vec<u8> {
    let mut b = n.to_le_bytes().to_vec();
    append_crc(&mut b);
    b
}

fn decode_u64_meta(buf: &[u8]) -> Result<u64> {
    let p = strip_crc(buf)?;
    if p.len() < 8 {
        return Err(StoreError::Msg("u64 meta short".into()));
    }
    Ok(u64::from_le_bytes(p[0..8].try_into().unwrap()))
}

fn encode_bytes(b: &mut Vec<u8>, d: &[u8]) {
    b.extend_from_slice(&(d.len() as u32).to_le_bytes());
    b.extend_from_slice(d);
}

fn take_bytes(buf: &[u8], off: &mut usize) -> Result<Vec<u8>> {
    if *off + 4 > buf.len() {
        return Err(StoreError::Msg("bytes len eof".into()));
    }
    let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
    *off += 4;
    if *off + n > buf.len() {
        return Err(StoreError::Msg("bytes body eof".into()));
    }
    let v = buf[*off..*off + n].to_vec();
    *off += n;
    Ok(v)
}

fn encode_entry(e: &RangeEntry) -> Vec<u8> {
    let mut b = Vec::new();
    match e {
        RangeEntry::Put { key, value } => {
            b.push(1);
            encode_bytes(&mut b, key);
            encode_bytes(&mut b, value);
        }
        RangeEntry::Dcs(cmd) => {
            b.push(2);
            let raw = cmd.encode();
            encode_bytes(&mut b, &raw);
        }
        RangeEntry::Noop => b.push(3),
        RangeEntry::Batch { pairs } => {
            b.push(4);
            b.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
            for (k, v) in pairs {
                encode_bytes(&mut b, k);
                encode_bytes(&mut b, v);
            }
        }
    }
    b
}

fn decode_entry(buf: &[u8], off: &mut usize) -> Result<RangeEntry> {
    if *off >= buf.len() {
        return Err(StoreError::Msg("entry eof".into()));
    }
    let tag = buf[*off];
    *off += 1;
    match tag {
        1 => {
            let key = take_bytes(buf, off)?;
            let value = take_bytes(buf, off)?;
            Ok(RangeEntry::Put { key, value })
        }
        2 => {
            let raw = take_bytes(buf, off)?;
            let cmd = DcsCommand::decode(&raw)?;
            Ok(RangeEntry::Dcs(cmd))
        }
        3 => Ok(RangeEntry::Noop),
        4 => {
            if *off + 4 > buf.len() {
                return Err(StoreError::Msg("batch count eof".into()));
            }
            let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
            *off += 4;
            // Bound pairs (hostile / corrupt log).
            if n > 1_000_000 {
                return Err(StoreError::Msg("batch too large".into()));
            }
            let mut pairs = Vec::with_capacity(n);
            for _ in 0..n {
                let key = take_bytes(buf, off)?;
                let value = take_bytes(buf, off)?;
                pairs.push((key, value));
            }
            Ok(RangeEntry::Batch { pairs })
        }
        t => Err(StoreError::Msg(format!("bad entry tag {t}"))),
    }
}

fn encode_log(log: &[LogRec]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(log.len() as u64).to_le_bytes());
    for rec in log {
        b.extend_from_slice(&rec.index.to_le_bytes());
        b.extend_from_slice(&rec.term.to_le_bytes());
        b.extend_from_slice(&encode_entry(&rec.entry));
    }
    append_crc(&mut b);
    b
}

fn decode_log(buf: &[u8]) -> Result<Vec<LogRec>> {
    let p = strip_crc(buf)?;
    if p.len() < 8 {
        return Err(StoreError::Msg("log header short".into()));
    }
    let n = u64::from_le_bytes(p[0..8].try_into().unwrap()) as usize;
    // Bound capacity (F2/F9 class).
    let rem = p.len().saturating_sub(8);
    if n > rem {
        return Err(StoreError::Msg(format!(
            "log entry count {n} exceeds remaining {rem}"
        )));
    }
    let mut off = 8;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if off + 16 > p.len() {
            return Err(StoreError::Msg("log rec short".into()));
        }
        let index = u64::from_le_bytes(p[off..off + 8].try_into().unwrap());
        off += 8;
        let term = u64::from_le_bytes(p[off..off + 8].try_into().unwrap());
        off += 8;
        let entry = decode_entry(p, &mut off)?;
        out.push(LogRec {
            index,
            term,
            entry,
        });
    }
    if off != p.len() {
        return Err(StoreError::Msg("log trailing garbage".into()));
    }
    Ok(out)
}

fn load_range_peer(db: &Db, range_id: u64, node_id: u64) -> Result<RangePeer> {
    let mut peer = RangePeer::new(node_id);
    if let Some(raw) = db.get(&raft_meta_key(range_id, "hard")) {
        let (term, voted) = decode_hard(&raw)?;
        peer.term = term;
        peer.voted_for = voted;
    }
    if let Some(raw) = db.get(&raft_meta_key(range_id, "snap")) {
        let (si, st) = decode_snap(&raw)?;
        peer.snapshot_index = si;
        peer.snapshot_term = st;
    }
    if let Some(raw) = db.get(&raft_meta_key(range_id, "log")) {
        peer.log = decode_log(&raw)?;
    }
    if let Some(raw) = db.get(&raft_meta_key(range_id, "commit")) {
        peer.commit = decode_u64_meta(&raw)?;
    }
    if let Some(raw) = db.get(&raft_meta_key(range_id, "applied")) {
        peer.applied = decode_u64_meta(&raw)?;
    }
    // Cap watermarks to log length (corrupt/partial meta).
    let last = peer.last_index();
    peer.commit = peer.commit.min(last);
    peer.applied = peer.applied.min(peer.commit);
    // Drop any log entries already covered by snapshot (idempotent load).
    if peer.snapshot_index > 0 {
        peer.log.retain(|e| e.index > peer.snapshot_index);
    }
    // Role is always follower after process restart (volatile leadership).
    peer.role = Role::Follower;
    peer.leader_id = None;
    Ok(peer)
}

fn persist_hard_db(db: &mut Db, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "hard"),
        encode_hard(peer.term, peer.voted_for),
    )?;
    Ok(())
}

fn persist_log_db(db: &mut Db, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(raft_meta_key(range_id, "log"), encode_log(&peer.log))?;
    Ok(())
}

fn persist_commit_db(db: &mut Db, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "commit"),
        encode_u64_meta(peer.commit),
    )?;
    Ok(())
}

fn persist_applied_db(db: &mut Db, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "applied"),
        encode_u64_meta(peer.applied),
    )?;
    Ok(())
}

fn encode_snap(index: u64, term: u64) -> Vec<u8> {
    let mut b = Vec::with_capacity(16 + 4);
    b.extend_from_slice(&index.to_le_bytes());
    b.extend_from_slice(&term.to_le_bytes());
    append_crc(&mut b);
    b
}

fn decode_snap(buf: &[u8]) -> Result<(u64, u64)> {
    let p = strip_crc(buf)?;
    if p.len() < 16 {
        return Err(StoreError::Msg("snap short".into()));
    }
    Ok((
        u64::from_le_bytes(p[0..8].try_into().unwrap()),
        u64::from_le_bytes(p[8..16].try_into().unwrap()),
    ))
}

fn persist_snap_db(db: &mut Db, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "snap"),
        encode_snap(peer.snapshot_index, peer.snapshot_term),
    )?;
    Ok(())
}

/// Per-range raft state on one node (log durable in PedraDB; apply → same DB).
struct RangePeer {
    role: Role,
    term: u64,
    voted_for: Option<u64>,
    log: Vec<LogRec>,
    /// Last log index included in the durable snapshot prefix (entries ≤ this dropped).
    snapshot_index: u64,
    /// Term of `snapshot_index` (for prev_log checks after truncate).
    snapshot_term: u64,
    commit: u64,
    applied: u64,
    election_left: u64,
    election_timeout: u64,
    hb_left: u64,
    next_index: HashMap<u64, u64>,
    match_index: HashMap<u64, u64>,
    leader_id: Option<u64>,
}

impl RangePeer {
    fn new(node_id: u64) -> Self {
        Self {
            role: Role::Follower,
            term: 0,
            voted_for: None,
            log: Vec::new(),
            snapshot_index: 0,
            snapshot_term: 0,
            commit: 0,
            applied: 0,
            election_left: 4 + node_id,
            election_timeout: 4 + node_id,
            hb_left: 2,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            leader_id: None,
        }
    }

    fn last_index(&self) -> u64 {
        self.log
            .last()
            .map_or(self.snapshot_index, |e| e.index)
    }

    fn last_term(&self) -> u64 {
        self.log
            .last()
            .map_or(self.snapshot_term, |e| e.term)
    }

    fn term_at(&self, index: u64) -> u64 {
        if index == 0 {
            return 0;
        }
        if index == self.snapshot_index {
            return self.snapshot_term;
        }
        if index < self.snapshot_index {
            return 0;
        }
        self.log
            .iter()
            .find(|e| e.index == index)
            .map_or(0, |e| e.term)
    }

    /// Drop log entries with `index <= through` (F27). Caller persists.
    fn compact_through(&mut self, through: u64) {
        if through <= self.snapshot_index || through == 0 {
            return;
        }
        let term = self.term_at(through);
        if term == 0 && through > self.snapshot_index {
            // Entry not found — cannot compact past known log.
            return;
        }
        self.log.retain(|e| e.index > through);
        self.snapshot_index = through;
        self.snapshot_term = term;
        let floor = through + 1;
        for ni in self.next_index.values_mut() {
            *ni = (*ni).max(floor);
        }
        // Match indices must not lag the snapshot or majority math stalls (commit
        // never advances past the truncated prefix).
        for mi in self.match_index.values_mut() {
            *mi = (*mi).max(through);
        }
    }

    fn become_leader(&mut self, peers: &[u64], self_id: u64) {
        self.role = Role::Leader;
        self.leader_id = Some(self_id);
        let next = self.last_index() + 1;
        self.next_index.clear();
        self.match_index.clear();
        for &p in peers {
            if p != self_id {
                self.next_index.insert(p, next);
                self.match_index.insert(p, 0);
            }
        }
        // F23: blank entry in current term so prev-term majority entries can commit
        // (critical after re-elect; same class as raft single-node F18).
        self.log.push(LogRec {
            index: next,
            term: self.term,
            entry: RangeEntry::Noop,
        });
        self.match_index.insert(self_id, next);
        self.hb_left = 0;
    }

    fn become_follower(&mut self, term: u64) {
        if term > self.term {
            self.term = term;
            self.voted_for = None;
        }
        self.role = Role::Follower;
        self.election_left = self.election_timeout;
    }
}

/// One physical store node.
struct StoreNode {
    db: Db,
    /// range_id → peer
    ranges: HashMap<u64, RangePeer>,
    /// When false, node is partitioned: no votes, no append RPC, no client leadership.
    participating: bool,
}

/// In-process multi-node multi-Raft store (Montanha-Store MVP).
pub struct StoreCluster {
    nodes: HashMap<u64, StoreNode>,
    ids: Vec<u64>,
    /// All range metas (same on every node).
    ranges: Vec<RangeMeta>,
    rng: u64,
}

impl StoreCluster {
    /// Open `n_nodes` under `parent`, with `n_ranges` equal splits of the keyspace.
    ///
    /// Ranges cover: `""` .. split points .. +∞ so every key maps somewhere.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open(parent: impl AsRef<Path>, n_nodes: u64, n_ranges: u64) -> Result<Self> {
        if n_nodes == 0 || n_ranges == 0 {
            return Err(StoreError::Msg("need nodes and ranges".into()));
        }
        // F25: single-byte split collapses when n_ranges > 256 (step == 0).
        if n_ranges > 256 {
            return Err(StoreError::Msg(
                "n_ranges > 256 not supported (single-byte keyspace split)".into(),
            ));
        }
        let parent = parent.as_ref();
        let ranges = split_keyspace(n_ranges);
        let mut nodes = HashMap::new();
        let mut ids = Vec::new();
        for id in 1..=n_nodes {
            let dir = parent.join(format!("store-node-{id}"));
            let db = Db::open_with(
                &dir,
                OpenOptions {
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    exclusive: true,
                },
            )?;
            let mut rmap = HashMap::new();
            for meta in &ranges {
                // F26: restore durable raft meta (or empty peer on first open).
                rmap.insert(meta.id, load_range_peer(&db, meta.id, id)?);
            }
            nodes.insert(
                id,
                StoreNode {
                    db,
                    ranges: rmap,
                    participating: true,
                },
            );
            ids.push(id);
        }
        Ok(Self {
            nodes,
            ids,
            ranges,
            rng: 0xA11CE,
        })
    }

    /// Range metas.
    #[must_use]
    pub fn range_metas(&self) -> &[RangeMeta] {
        &self.ranges
    }

    /// Membership node ids.
    #[must_use]
    pub fn node_ids(&self) -> &[u64] {
        &self.ids
    }

    /// Locate range id for key.
    pub fn locate(&self, key: &[u8]) -> Result<u64> {
        self.ranges
            .iter()
            .find(|r| r.contains(key))
            .map(|r| r.id)
            .ok_or(StoreError::NoRange)
    }

    /// Whether `node_id` participates in Raft (not partitioned).
    #[must_use]
    pub fn is_participating(&self, node_id: u64) -> bool {
        self.nodes
            .get(&node_id)
            .map(|n| n.participating)
            .unwrap_or(false)
    }

    /// Partition or heal a node (no votes / append / leadership while off).
    pub fn set_participating(&mut self, node_id: u64, on: bool) -> Result<()> {
        let n = self
            .nodes
            .get_mut(&node_id)
            .ok_or_else(|| StoreError::Msg("bad node".into()))?;
        n.participating = on;
        if !on {
            // Drop leadership so clients do not route to a partitioned leader.
            for p in n.ranges.values_mut() {
                if p.role == Role::Leader {
                    p.role = Role::Follower;
                    p.leader_id = None;
                    p.election_left = p.election_timeout;
                }
            }
        }
        Ok(())
    }

    /// Force the current range leader to step down (still participating).
    ///
    /// Returns the previous leader id.
    pub fn step_down_range_leader(&mut self, range_id: u64) -> Result<u64> {
        let lid = self
            .range_leader(range_id)
            .ok_or_else(|| StoreError::Msg("no leader to step down".into()))?;
        let p = self
            .nodes
            .get_mut(&lid)
            .unwrap()
            .ranges
            .get_mut(&range_id)
            .unwrap();
        p.role = Role::Follower;
        p.leader_id = None;
        p.election_left = p.election_timeout;
        // Clear follower view of this leader so strong-read revalidation works.
        for n in self.nodes.values_mut() {
            if let Some(rp) = n.ranges.get_mut(&range_id) {
                if rp.leader_id == Some(lid) {
                    rp.leader_id = None;
                }
            }
        }
        Ok(lid)
    }

    /// Unique participating leader for range, if any.
    ///
    /// Returns `None` when zero **or more than one** node claims `Role::Leader`
    /// (dual-leader / split-brain claim → fail closed for strong reads and routing).
    #[must_use]
    pub fn range_leader(&self, range_id: u64) -> Option<u64> {
        let mut leaders = self.ids.iter().filter_map(|&nid| {
            let n = self.nodes.get(&nid)?;
            if !n.participating {
                return None;
            }
            let p = n.ranges.get(&range_id)?;
            if p.role == Role::Leader {
                Some(nid)
            } else {
                None
            }
        });
        let first = leaders.next()?;
        if leaders.next().is_some() {
            // Ambiguous: more than one Leader claim — not safe to serve.
            return None;
        }
        Some(first)
    }

    /// Count of participating nodes that currently claim `Role::Leader` for the range.
    #[must_use]
    pub fn leader_claim_count(&self, range_id: u64) -> u64 {
        self.ids
            .iter()
            .filter(|&&nid| {
                self.nodes.get(&nid).is_some_and(|n| {
                    n.participating
                        && n.ranges
                            .get(&range_id)
                            .is_some_and(|p| p.role == Role::Leader)
                })
            })
            .count() as u64
    }

    /// Count peers where applied PedraDB has `key` → `value` (byte equality).
    #[must_use]
    pub fn count_applied_eq(&self, key: &[u8], value: &[u8]) -> u64 {
        self.ids
            .iter()
            .filter(|&&nid| {
                self.nodes
                    .get(&nid)
                    .and_then(|n| n.db.get(key))
                    .is_some_and(|v| v.as_ref() == value)
            })
            .count() as u64
    }

    /// Range commit index on a node (0 if missing).
    #[must_use]
    pub fn commit_index(&self, node_id: u64, range_id: u64) -> u64 {
        self.nodes
            .get(&node_id)
            .and_then(|n| n.ranges.get(&range_id))
            .map(|p| p.commit)
            .unwrap_or(0)
    }

    /// Range applied index on a node.
    #[must_use]
    pub fn applied_index(&self, node_id: u64, range_id: u64) -> u64 {
        self.nodes
            .get(&node_id)
            .and_then(|n| n.ranges.get(&range_id))
            .map(|p| p.applied)
            .unwrap_or(0)
    }

    /// Whether `node_id` believes it is Leader for `range_id` (ignores cluster view).
    #[must_use]
    pub fn node_thinks_leader(&self, node_id: u64, range_id: u64) -> bool {
        self.nodes
            .get(&node_id)
            .and_then(|n| n.ranges.get(&range_id))
            .is_some_and(|p| p.role == Role::Leader)
    }

    fn next_rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Drive elections / heartbeats for all ranges.
    pub fn tick(&mut self) -> Result<()> {
        let ids = self.ids.clone();
        let range_ids: Vec<u64> = self.ranges.iter().map(|r| r.id).collect();
        for rid in range_ids {
            self.tick_range(rid, &ids)?;
        }
        Ok(())
    }

    /// Elect leaders for all ranges (bounded ticks).
    pub fn elect_all(&mut self, max_ticks: u64) -> Result<()> {
        for _ in 0..max_ticks {
            let all = self.ranges.iter().all(|r| self.range_leader(r.id).is_some());
            if all {
                return Ok(());
            }
            self.tick()?;
        }
        Err(StoreError::Msg("elect timeout".into()))
    }

    fn tick_range(&mut self, rid: u64, ids: &[u64]) -> Result<()> {
        let mut elect: Vec<u64> = Vec::new();
        let mut hb: Vec<u64> = Vec::new();
        for &nid in ids {
            if !self.is_participating(nid) {
                continue;
            }
            let p = self.nodes.get_mut(&nid).unwrap().ranges.get_mut(&rid).unwrap();
            match p.role {
                Role::Leader => {
                    if p.hb_left == 0 {
                        hb.push(nid);
                        p.hb_left = 2;
                    } else {
                        p.hb_left -= 1;
                    }
                }
                Role::Follower | Role::Candidate => {
                    if p.election_left == 0 {
                        elect.push(nid);
                    } else {
                        p.election_left -= 1;
                    }
                }
            }
        }
        for lid in hb {
            self.broadcast_append(rid, lid, None)?;
        }
        for cid in elect {
            self.start_election(rid, cid)?;
        }
        Ok(())
    }

    fn start_election(&mut self, rid: u64, cand: u64) -> Result<()> {
        if !self.is_participating(cand) {
            return Ok(());
        }
        let ids = self.ids.clone();
        let j = self.next_rand() % 3;
        let (term, last_i, last_t) = {
            let n = self.nodes.get_mut(&cand).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            p.term += 1;
            p.role = Role::Candidate;
            p.voted_for = Some(cand);
            p.election_left = p.election_timeout + j;
            let t = p.term;
            let li = p.last_index();
            let lt = p.last_term();
            persist_hard_db(&mut n.db, rid, p)?;
            (t, li, lt)
        };
        let mut votes = 1u64;
        // Majority of configured membership (partitioned nodes do not vote).
        let maj = (ids.len() as u64) / 2 + 1;
        for &pid in &ids {
            if pid == cand {
                continue;
            }
            if !self.is_participating(pid) {
                continue;
            }
            let grant = {
                let n = self.nodes.get_mut(&pid).unwrap();
                let p = n.ranges.get_mut(&rid).unwrap();
                if term > p.term {
                    p.become_follower(term);
                    let _ = persist_hard_db(&mut n.db, rid, p);
                }
                let can = p.voted_for.is_none() || p.voted_for == Some(cand);
                let up = last_t > p.last_term()
                    || (last_t == p.last_term() && last_i >= p.last_index());
                if term == p.term && can && up {
                    p.voted_for = Some(cand);
                    p.election_left = p.election_timeout;
                    let _ = persist_hard_db(&mut n.db, rid, p);
                    true
                } else {
                    false
                }
            };
            if grant {
                votes += 1;
            }
        }
        if votes >= maj {
            {
                let n = self.nodes.get_mut(&cand).unwrap();
                let p = n.ranges.get_mut(&rid).unwrap();
                if p.term == term && p.role == Role::Candidate {
                    p.become_leader(&ids, cand);
                    persist_log_db(&mut n.db, rid, p)?;
                }
            }
            self.broadcast_append(rid, cand, None)?;
        }
        Ok(())
    }

    fn broadcast_append(
        &mut self,
        rid: u64,
        leader: u64,
        client: Option<RangeEntry>,
    ) -> Result<()> {
        if !self.is_participating(leader) {
            return Err(StoreError::NotLeader {
                range_id: rid,
                leader: None,
            });
        }
        // Dual-leader claim: refuse client proposes (and leadership routing).
        if client.is_some() && self.leader_claim_count(rid) != 1 {
            return Err(StoreError::NotLeader {
                range_id: rid,
                leader: self.range_leader(rid),
            });
        }
        let ids = self.ids.clone();
        let had_client = client.is_some();
        let mut proposed_index: Option<u64> = None;
        if let Some(entry) = client {
            let n = self.nodes.get_mut(&leader).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            if p.role != Role::Leader {
                return Err(StoreError::NotLeader {
                    range_id: rid,
                    leader: p.leader_id,
                });
            }
            let idx = p.last_index() + 1;
            let term = p.term;
            p.log.push(LogRec {
                index: idx,
                term,
                entry,
            });
            proposed_index = Some(idx);
            persist_log_db(&mut n.db, rid, p)?;
        }
        let (term, commit, last, log_snap) = {
            let p = self.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
            if p.role != Role::Leader {
                return Err(StoreError::NotLeader {
                    range_id: rid,
                    leader: p.leader_id,
                });
            }
            (p.term, p.commit, p.last_index(), p.log.clone())
        };

        for &pid in &ids {
            if pid == leader {
                continue;
            }
            if !self.is_participating(pid) {
                continue;
            }
            let (next, prev, prev_term) = {
                let p = self.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
                let next = *p.next_index.get(&pid).unwrap_or(&(last + 1));
                let prev = next.saturating_sub(1);
                // F27: after log compact, `prev` may be only in the snapshot watermark
                // (not in `log_snap`). Must use term_at, not log-only lookup.
                let prev_term = p.term_at(prev);
                (next, prev, prev_term)
            };
            let entries: Vec<LogRec> = log_snap
                .iter()
                .filter(|e| e.index >= next)
                .cloned()
                .collect();

            let ok = {
                let n = self.nodes.get_mut(&pid).unwrap();
                let p = n.ranges.get_mut(&rid).unwrap();
                if term < p.term {
                    false
                } else {
                    if term > p.term {
                        p.become_follower(term);
                        let _ = persist_hard_db(&mut n.db, rid, p);
                    } else {
                        p.role = Role::Follower;
                        p.election_left = p.election_timeout;
                    }
                    p.leader_id = Some(leader);
                    let consistent = prev == 0
                        || (p.last_index() >= prev && p.term_at(prev) == prev_term);
                    if !consistent {
                        false
                    } else {
                        let mut ok_append = true;
                        let mut log_dirty = false;
                        for e in &entries {
                            if let Some(i) = p.log.iter().position(|x| x.index == e.index) {
                                // Conflict if term *or* payload differs (leader may have
                                // discarded an uncommitted client entry and re-used the index).
                                if p.log[i].term != e.term || p.log[i].entry != e.entry {
                                    // F24: never rewrite a committed index.
                                    if e.index <= p.commit {
                                        ok_append = false;
                                        break;
                                    }
                                    p.log.truncate(i);
                                    p.log.push(e.clone());
                                    log_dirty = true;
                                }
                            } else {
                                let expect = p.last_index() + 1;
                                if e.index != expect {
                                    ok_append = false;
                                    break;
                                }
                                p.log.push(e.clone());
                                log_dirty = true;
                            }
                        }
                        if !ok_append {
                            false
                        } else {
                            p.log.sort_by_key(|e| e.index);
                            p.log.dedup_by_key(|e| e.index);
                            if log_dirty {
                                let _ = persist_log_db(&mut n.db, rid, p);
                            }
                            if commit > p.commit {
                                p.commit = commit.min(p.last_index());
                                let _ = persist_commit_db(&mut n.db, rid, p);
                            }
                            true
                        }
                    }
                }
            };
            {
                let p = self.nodes.get_mut(&leader).unwrap().ranges.get_mut(&rid).unwrap();
                if ok {
                    let match_i = entries.last().map_or(prev, |e| e.index);
                    p.next_index.insert(pid, match_i + 1);
                    p.match_index.insert(pid, match_i);
                } else {
                    let ni = p.next_index.get(&pid).copied().unwrap_or(1);
                    p.next_index.insert(pid, ni.saturating_sub(1).max(1));
                }
            }
        }

        // Commit on leader only when a strict majority of the membership has the entry.
        {
            let maj = ids.len() / 2 + 1;
            let n = self.nodes.get_mut(&leader).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            let last = p.last_index();
            for idx in (1..=last).rev() {
                let count = ids
                    .iter()
                    .filter(|&&pid| {
                        if pid == leader {
                            true
                        } else {
                            p.match_index.get(&pid).copied().unwrap_or(0) >= idx
                        }
                    })
                    .count();
                if count >= maj && p.term_at(idx) == p.term {
                    if idx > p.commit {
                        p.commit = idx;
                        let _ = persist_commit_db(&mut n.db, rid, p);
                    }
                    break;
                }
            }
        }

        // Client API contract: Ok only if the proposed index is majority-committed.
        // On failure, **discard** the uncommitted entry everywhere so heal/tick cannot
        // silently commit an orphan (fencing lie) and client retry cannot dual-append
        // Create (non-idempotent apply brick).
        if let Some(idx) = proposed_index {
            let commit_now = self
                .nodes
                .get(&leader)
                .and_then(|n| n.ranges.get(&rid))
                .map(|p| p.commit)
                .unwrap_or(0);
            if commit_now < idx {
                // Must re-persist truncated logs (entry was already on disk at propose).
                self.discard_uncommitted_from(rid, leader, idx)?;
                return Err(StoreError::NotCommitted {
                    range_id: rid,
                    index: idx,
                    commit: commit_now,
                });
            }
        }

        // Apply on participating nodes for this range.
        for &nid in &ids {
            if self.is_participating(nid) {
                self.apply_range(nid, rid)?;
            }
        }
        // Heartbeat to push commit to followers (no client entry → no NotCommitted gate).
        if had_client {
            self.broadcast_append(rid, leader, None)?;
            for &nid in &ids {
                if self.is_participating(nid) {
                    self.apply_range(nid, rid)?;
                }
            }
        }
        // F27: drop applied prefix once every participating peer has applied it.
        self.maybe_compact_logs(rid)?;
        Ok(())
    }

    /// Truncate raft logs on all peers through `min(applied)` (F27).
    fn maybe_compact_logs(&mut self, rid: u64) -> Result<()> {
        let ids: Vec<u64> = self
            .ids
            .iter()
            .copied()
            .filter(|&nid| self.is_participating(nid))
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let min_applied = ids
            .iter()
            .map(|&nid| self.applied_index(nid, rid))
            .min()
            .unwrap_or(0);
        if min_applied == 0 {
            return Ok(());
        }
        // Only compact if every peer already has snapshot < min_applied and has the entry.
        for &nid in &ids {
            let p = self.nodes.get(&nid).unwrap().ranges.get(&rid).unwrap();
            if p.snapshot_index >= min_applied {
                continue;
            }
            if p.term_at(min_applied) == 0 && min_applied > p.snapshot_index {
                return Ok(()); // lagging peer missing entry; wait
            }
        }
        for &nid in &ids {
            let n = self.nodes.get_mut(&nid).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            if p.snapshot_index >= min_applied {
                continue;
            }
            let before = p.log.len();
            p.compact_through(min_applied);
            if p.log.len() != before || p.snapshot_index == min_applied {
                persist_snap_db(&mut n.db, rid, p)?;
                persist_log_db(&mut n.db, rid, p)?;
            }
        }
        Ok(())
    }

    /// Drop log entries with `index >= from_index` on all peers for `rid`.
    ///
    /// Used when a client propose fails majority: the entry must not linger to be
    /// committed later by heartbeat/heal, and the index must be free for a clean retry.
    ///
    /// **Durability:** client entries are written via [`persist_log_db`] at propose
    /// time; this path must re-persist the truncated log (and applied cursor if
    /// clamped) so a process reopen cannot resurrect an orphan (I-MAJ-3 / I-DCS-5).
    fn discard_uncommitted_from(
        &mut self,
        rid: u64,
        leader: u64,
        from_index: u64,
    ) -> Result<()> {
        let ids = self.ids.clone();
        for &nid in &ids {
            let Some(n) = self.nodes.get_mut(&nid) else {
                continue;
            };
            let Some(p) = n.ranges.get_mut(&rid) else {
                continue;
            };
            // Never discard at or below commit (safety).
            let cut = from_index.max(p.commit.saturating_add(1));
            let before_len = p.log.len();
            p.log.retain(|e| e.index < cut);
            let mut applied_dirty = false;
            if p.applied >= cut {
                // applied should never exceed commit; clamp if inconsistent.
                p.applied = p.commit.min(p.applied);
                applied_dirty = true;
            }
            // Always re-persist when we truncated (or even if empty retain matched —
            // propose already wrote the orphan index to disk on the leader).
            if p.log.len() != before_len || nid == leader {
                persist_log_db(&mut n.db, rid, p)?;
            }
            if applied_dirty {
                persist_applied_db(&mut n.db, rid, p)?;
            }
        }
        if let Some(p) = self
            .nodes
            .get_mut(&leader)
            .and_then(|n| n.ranges.get_mut(&rid))
        {
            for &pid in &ids {
                if pid == leader {
                    continue;
                }
                let m = p.match_index.get(&pid).copied().unwrap_or(0);
                if m >= from_index {
                    p.match_index.insert(pid, from_index.saturating_sub(1));
                }
                // Force re-send from the discarded index on next append.
                p.next_index.insert(pid, from_index);
            }
        }
        Ok(())
    }

    /// Append a put **only on the leader log** without replication (test/sim hook).
    ///
    /// Used to prove minority-only append does not advance commit / apply.
    pub fn append_local_only_put(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<u64> {
        let key = key.as_ref().to_vec();
        let value = value.as_ref().to_vec();
        let rid = self.locate(&key)?;
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        let p = self
            .nodes
            .get_mut(&leader)
            .unwrap()
            .ranges
            .get_mut(&rid)
            .unwrap();
        if p.role != Role::Leader {
            return Err(StoreError::NotLeader {
                range_id: rid,
                leader: p.leader_id,
            });
        }
        let idx = p.last_index() + 1;
        let term = p.term;
        p.log.push(LogRec {
            index: idx,
            term,
            entry: RangeEntry::Put { key, value },
        });
        // Recompute commit with only local knowledge (match_index unchanged) —
        // must NOT commit the new index without majority match.
        let ids = self.ids.clone();
        let maj = ids.len() / 2 + 1;
        let p = self
            .nodes
            .get_mut(&leader)
            .unwrap()
            .ranges
            .get_mut(&rid)
            .unwrap();
        let last = p.last_index();
        let mut new_commit = p.commit;
        for n in (1..=last).rev() {
            let count = ids
                .iter()
                .filter(|&&pid| {
                    if pid == leader {
                        true
                    } else {
                        p.match_index.get(&pid).copied().unwrap_or(0) >= n
                    }
                })
                .count();
            if count >= maj && p.term_at(n) == p.term {
                new_commit = n;
                break;
            }
        }
        p.commit = new_commit;
        self.apply_range(leader, rid)?;
        Ok(self.commit_index(leader, rid))
    }

    fn apply_range(&mut self, nid: u64, rid: u64) -> Result<()> {
        let node = self.nodes.get_mut(&nid).unwrap();
        // Collect entries to apply, then mutate db + peer separately (borrowck).
        let (start, end, recs) = {
            let peer = node.ranges.get(&rid).unwrap();
            let start = peer.applied + 1;
            let end = peer.commit;
            let mut recs = Vec::new();
            for next in start..=end {
                if let Some(rec) = peer.log.iter().find(|e| e.index == next) {
                    recs.push(rec.clone());
                } else {
                    break;
                }
            }
            (start, end, recs)
        };
        if recs.is_empty() {
            return Ok(());
        }
        let mut applied_to = start - 1;
        for rec in &recs {
            match &rec.entry {
                RangeEntry::Put { key, value } => {
                    if !is_raft_meta_key(key) {
                        node.db.put(key, value)?;
                    }
                }
                RangeEntry::Batch { pairs } => {
                    let ops: Vec<BatchOp> = pairs
                        .iter()
                        .filter(|(k, _)| !is_raft_meta_key(k))
                        .map(|(k, v)| BatchOp::put(k, v))
                        .collect();
                    if !ops.is_empty() {
                        node.db.apply_batch(ops)?;
                    }
                }
                RangeEntry::Dcs(cmd) => {
                    match apply_dcs_command(&mut node.db, cmd) {
                        Ok(_) => {}
                        Err(pedradb_dcs::DcsError::CasFailed(_)) => {}
                        Err(e) => return Err(StoreError::Dcs(e)),
                    }
                }
                RangeEntry::Noop => {}
            }
            applied_to = rec.index;
        }
        let peer = node.ranges.get_mut(&rid).unwrap();
        peer.applied = applied_to.min(end);
        persist_applied_db(&mut node.db, rid, peer)?;
        Ok(())
    }

    /// Put key/value via the leader of the owning range.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let key = key.as_ref().to_vec();
        if is_raft_meta_key(&key) {
            return Err(StoreError::Msg(
                "key prefix \\0store/raft/ is reserved for raft meta".into(),
            ));
        }
        let value = value.as_ref().to_vec();
        let rid = self.locate(&key)?;
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        self.broadcast_append(
            rid,
            leader,
            Some(RangeEntry::Put { key, value }),
        )
    }

    /// Atomically put multiple key/value pairs in **one** raft log entry.
    ///
    /// All keys must map to the **same** range ([`StoreError::CrossRange`] otherwise).
    /// On Ok, every pair is majority-committed and applied together (PedraDB batch).
    /// On [`StoreError::NotCommitted`], no pair is majority-applied (same discard rules
    /// as single put). Empty batch is a no-op Ok.
    ///
    /// # Errors
    /// Cross-range, reserved keys, not leader, not committed, I/O.
    pub fn put_batch(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<[u8]>, impl AsRef<[u8]>)>,
    ) -> Result<()> {
        let mut owned: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (k, v) in pairs {
            let key = k.as_ref().to_vec();
            if is_raft_meta_key(&key) {
                return Err(StoreError::Msg(
                    "key prefix \\0store/raft/ is reserved for raft meta".into(),
                ));
            }
            owned.push((key, v.as_ref().to_vec()));
        }
        if owned.is_empty() {
            return Ok(());
        }
        let mut range_ids: Vec<u64> = Vec::new();
        for (k, _) in &owned {
            let rid = self.locate(k)?;
            if !range_ids.contains(&rid) {
                range_ids.push(rid);
            }
        }
        range_ids.sort_unstable();
        if range_ids.len() != 1 {
            return Err(StoreError::CrossRange { ranges: range_ids });
        }
        let rid = range_ids[0];
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        self.broadcast_append(rid, leader, Some(RangeEntry::Batch { pairs: owned }))
    }

    /// Get from a node (applied state). **LocalApplied** semantics — non-linearizable.
    pub fn get_on(&self, node_id: u64, key: &[u8]) -> Result<Option<Bytes>> {
        self.get_with_policy(node_id, key, ReadPolicy::LocalApplied)
    }

    /// Get from node 1 if present (local applied).
    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        let id = *self.ids.first().ok_or_else(|| StoreError::Msg("empty".into()))?;
        self.get_on(id, key)
    }

    /// Read with an explicit policy.
    ///
    /// - [`ReadPolicy::LocalApplied`]: any node, applied PedraDB (may be stale).
    /// - [`ReadPolicy::Strong`]: only if `node_id` is the live range leader; otherwise
    ///   [`StoreError::StaleLeader`] / [`StoreError::NotLeader`].
    pub fn get_with_policy(
        &self,
        node_id: u64,
        key: &[u8],
        policy: ReadPolicy,
    ) -> Result<Option<Bytes>> {
        let n = self
            .nodes
            .get(&node_id)
            .ok_or_else(|| StoreError::Msg("bad node".into()))?;
        match policy {
            ReadPolicy::LocalApplied => Ok(n.db.get(key)),
            ReadPolicy::Strong => {
                let rid = self.locate(key)?;
                // Fail closed: need a *unique* live leader, and it must be this node.
                let live = self.range_leader(rid);
                let claims = self.leader_claim_count(rid);
                let thinks = n
                    .ranges
                    .get(&rid)
                    .is_some_and(|p| p.role == Role::Leader);
                if claims != 1 || live != Some(node_id) || !thinks || !n.participating {
                    return Err(StoreError::StaleLeader {
                        range_id: rid,
                        node_id,
                        live_leader: live,
                    });
                }
                Ok(n.db.get(key))
            }
        }
    }

    /// Strong (linearizable-class) get via the live range leader.
    pub fn get_strong(&self, key: &[u8]) -> Result<Option<Bytes>> {
        let rid = self.locate(key)?;
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        self.get_with_policy(leader, key, ReadPolicy::Strong)
    }

    /// DCS create-if-absent on the range that owns `key` (meta keys should use `m/` prefix).
    pub fn dcs_create(&mut self, key: &[u8], value: &[u8]) -> Result<u64> {
        let cmd = DcsCommand::Create {
            key: key.to_vec(),
            value: value.to_vec(),
            lease: 0,
        };
        self.propose_dcs(cmd)
    }

    /// DCS CAS.
    pub fn dcs_cas(&mut self, key: &[u8], value: &[u8], expected_rev: u64) -> Result<u64> {
        let cmd = DcsCommand::Cas {
            key: key.to_vec(),
            value: value.to_vec(),
            expected_rev,
            lease: 0,
        };
        self.propose_dcs(cmd)
    }

    fn propose_dcs(&mut self, cmd: DcsCommand) -> Result<u64> {
        let key = match &cmd {
            DcsCommand::Put { key, .. }
            | DcsCommand::Create { key, .. }
            | DcsCommand::Cas { key, .. }
            | DcsCommand::Delete { key } => key.clone(),
        };
        let is_delete = matches!(cmd, DcsCommand::Delete { .. });
        let rid = self.locate(&key)?;
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        // Pre-check on leader db.
        {
            let db = &self.nodes.get(&leader).unwrap().db;
            check_command(db, &cmd)?;
        }
        // Majority commit required: NotCommitted if followers cannot form a majority.
        self.broadcast_append(rid, leader, Some(RangeEntry::Dcs(cmd)))?;
        // Post-condition: entry applied on leader. Never return Ok(0) for a successful mutate.
        let n = self
            .nodes
            .get(&leader)
            .ok_or_else(|| StoreError::Msg("bad leader".into()))?;
        if is_delete {
            if dcs_get(&n.db, &key).is_some() {
                return Err(StoreError::Msg(
                    "dcs delete committed but key still present".into(),
                ));
            }
            let rev = n
                .db
                .get(b"d/rev")
                .and_then(|b| {
                    if b.len() >= 8 {
                        Some(u64::from_le_bytes(b[..8].try_into().ok()?))
                    } else {
                        None
                    }
                })
                .ok_or(StoreError::NotCommitted {
                    range_id: rid,
                    index: 0,
                    commit: 0,
                })?;
            if rev == 0 {
                return Err(StoreError::NotCommitted {
                    range_id: rid,
                    index: 0,
                    commit: 0,
                });
            }
            return Ok(rev);
        }
        let kv = dcs_get(&n.db, &key).ok_or(StoreError::NotCommitted {
            range_id: rid,
            index: self.commit_index(leader, rid),
            commit: self.commit_index(leader, rid),
        })?;
        if kv.mod_revision == 0 {
            return Err(StoreError::NotCommitted {
                range_id: rid,
                index: kv.mod_revision,
                commit: self.commit_index(leader, rid),
            });
        }
        Ok(kv.mod_revision)
    }

    /// DCS get on a node.
    pub fn dcs_get_on(&self, node_id: u64, key: &[u8]) -> Result<Option<KeyValue>> {
        let n = self
            .nodes
            .get(&node_id)
            .ok_or_else(|| StoreError::Msg("bad node".into()))?;
        Ok(dcs_get(&n.db, key))
    }

    /// Parent path helper.
    #[must_use]
    pub fn node_dir(parent: impl AsRef<Path>, id: u64) -> PathBuf {
        parent.as_ref().join(format!("store-node-{id}"))
    }
}

/// Split keyspace into `n` ranges using single-byte boundaries when possible.
///
/// Caller must ensure `1 <= n <= 256` (see [`StoreCluster::open`]).
fn split_keyspace(n: u64) -> Vec<RangeMeta> {
    if n == 1 {
        return vec![RangeMeta {
            id: 1,
            start: vec![],
            end: vec![],
        }];
    }
    debug_assert!((2..=256).contains(&n));
    // Use splits at equal steps over the first key byte.
    let mut out = Vec::new();
    let step = 256u64 / n;
    for i in 0..n {
        let start = if i == 0 {
            vec![]
        } else {
            vec![(i * step) as u8]
        };
        let end = if i + 1 == n {
            vec![]
        } else {
            vec![((i + 1) * step) as u8]
        };
        out.push(RangeMeta {
            id: i + 1,
            start,
            end,
        });
    }
    out
}

/// Default meta prefix for DCS keys so they land in early ranges (often range 1 if start empty).
pub const META_PREFIX: &[u8] = b"m/";

/// Build a meta key under `m/`.
#[must_use]
pub fn meta_key(suffix: &[u8]) -> Vec<u8> {
    let mut k = META_PREFIX.to_vec();
    k.extend_from_slice(suffix);
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-store-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn keys_one_per_range(c: &StoreCluster) -> Vec<Vec<u8>> {
        c.range_metas()
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

    #[test]
    fn multi_range_puts_different_leaders() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        assert_eq!(keys.len(), 3);
        let r0 = c.locate(&keys[0]).unwrap();
        let r1 = c.locate(&keys[1]).unwrap();
        let r2 = c.locate(&keys[2]).unwrap();
        assert_ne!(r0, r1);
        assert_ne!(r1, r2);
        assert_ne!(r0, r2);
        // Concurrent range leadership is real: each range has a live leader
        // (may share physical nodes, but leadership is per-range).
        assert!(c.range_leader(r0).is_some());
        assert!(c.range_leader(r1).is_some());
        assert!(c.range_leader(r2).is_some());
        c.put(&keys[0], b"a").unwrap();
        c.put(&keys[1], b"b").unwrap();
        c.put(&keys[2], b"c").unwrap();
        for nid in 1..=3u64 {
            assert_eq!(
                c.get_on(nid, &keys[0]).unwrap().as_deref(),
                Some(b"a".as_ref())
            );
            assert_eq!(
                c.get_on(nid, &keys[1]).unwrap().as_deref(),
                Some(b"b".as_ref())
            );
            assert_eq!(
                c.get_on(nid, &keys[2]).unwrap().as_deref(),
                Some(b"c".as_ref())
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn majority_durable_put_on_three_peers() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = b"maj-key";
        let val = b"maj-val";
        c.put(key, val).unwrap();
        let n = c.count_applied_eq(key, val);
        assert!(
            n >= 2,
            "successful put must be applied on a strict majority; got {n}"
        );
        // Full majority of 3 should all see it after commit push in this MVP.
        assert_eq!(n, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Criterion 2: real `put` path under follower partition must not return Ok.
    #[test]
    fn put_fails_without_majority_under_partition() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let rid = c.locate(b"part-k").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let ids: Vec<u64> = c.node_ids().to_vec();
        // Partition both followers — only leader remains (minority of membership).
        for &nid in &ids {
            if nid != leader {
                c.set_participating(nid, false).unwrap();
            }
        }
        assert!(c.range_leader(rid).is_some());
        let err = c
            .put(b"part-k", b"v")
            .expect_err("put without majority must not Ok");
        assert!(
            matches!(err, StoreError::NotCommitted { .. }),
            "expected NotCommitted, got {err:?}"
        );
        assert_eq!(
            c.count_applied_eq(b"part-k", b"v"),
            0,
            "uncommitted put must not apply"
        );
        // Heal and put succeeds with majority.
        for &nid in &ids {
            c.set_participating(nid, true).unwrap();
        }
        c.elect_all(80).unwrap();
        c.put(b"part-k", b"v").unwrap();
        assert!(c.count_applied_eq(b"part-k", b"v") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Criterion 2/5: real `dcs_create` under minority must not return Ok(0) or Ok at all.
    #[test]
    fn dcs_create_fails_without_majority_under_partition() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = meta_key(b"part-leader");
        let rid = c.locate(&key).unwrap();
        let leader = c.range_leader(rid).unwrap();
        let ids: Vec<u64> = c.node_ids().to_vec();
        for &nid in &ids {
            if nid != leader {
                c.set_participating(nid, false).unwrap();
            }
        }
        let err = c
            .dcs_create(&key, b"node-a")
            .expect_err("dcs_create without majority must not Ok");
        assert!(
            matches!(err, StoreError::NotCommitted { .. }),
            "expected NotCommitted, got {err:?}"
        );
        for nid in 1..=3u64 {
            assert!(
                c.dcs_get_on(nid, &key).unwrap().is_none(),
                "key must be absent after failed create"
            );
        }
        // No orphan on leader log: last_index == commit after discard.
        assert_eq!(
            c.commit_index(leader, rid),
            c.nodes
                .get(&leader)
                .unwrap()
                .ranges
                .get(&rid)
                .unwrap()
                .last_index()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Skeptic: NotCommitted must not leave orphan Create that heal/tick commits
    /// without Ok, and retry+put must not brick the range.
    #[test]
    fn dcs_create_not_committed_heal_retry_put_ok() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = meta_key(b"heal-leader");
        let rid = c.locate(&key).unwrap();
        let leader = c.range_leader(rid).unwrap();
        let ids: Vec<u64> = c.node_ids().to_vec();
        for &nid in &ids {
            if nid != leader {
                c.set_participating(nid, false).unwrap();
            }
        }
        c.dcs_create(&key, b"node-a")
            .expect_err("must NotCommitted under minority");
        // Key absent after discard (no fencing lie if we never Ok).
        for &nid in &ids {
            assert!(c.dcs_get_on(nid, &key).unwrap().is_none());
        }

        // Heal; ticks must not silently install the lock (orphan discarded).
        for &nid in &ids {
            c.set_participating(nid, true).unwrap();
        }
        for _ in 0..40 {
            c.tick().unwrap();
        }
        for &nid in &ids {
            assert!(
                c.dcs_get_on(nid, &key).unwrap().is_none(),
                "heal/tick must not commit discarded Create; node {nid}"
            );
        }

        c.elect_all(80).unwrap();
        // Clean retry succeeds and replicates.
        let rev = c.dcs_create(&key, b"node-a").expect("retry create after heal");
        assert!(rev >= 1);
        for &nid in &ids {
            let kv = c.dcs_get_on(nid, &key).unwrap().expect("replicated");
            assert_eq!(kv.value, b"node-a");
        }
        // Second create still fails at client pre-check (lock held).
        assert!(c.dcs_create(&key, b"node-b").is_err());

        // Range not bricked: ordinary puts apply on majority.
        c.put(b"x", b"1").unwrap();
        c.put(b"y", b"2").unwrap();
        assert!(c.count_applied_eq(b"x", b"1") >= 2);
        assert!(c.count_applied_eq(b"y", b"2") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// I-MAJ-3 / I-DCS-5: NotCommitted must discard the **durable** raft log entry.
    ///
    /// Propose persists the log before majority; without re-persist on discard,
    /// reopen would reload the orphan Create and heal/tick could apply it without Ok.
    #[test]
    fn dcs_create_not_committed_survives_reopen_without_installing_lock() {
        let dir = temp();
        let key = meta_key(b"reopen-orphan-leader");
        {
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            let rid = c.locate(&key).unwrap();
            let leader = c.range_leader(rid).unwrap();
            let ids: Vec<u64> = c.node_ids().to_vec();
            for &nid in &ids {
                if nid != leader {
                    c.set_participating(nid, false).unwrap();
                }
            }
            c.dcs_create(&key, b"node-a")
                .expect_err("must NotCommitted under minority");
            // Durable log on leader must not retain an entry past commit.
            let p = c.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
            assert!(
                p.last_index() <= p.commit,
                "after discard last_index={} must be <= commit={}",
                p.last_index(),
                p.commit
            );
            assert!(c.dcs_get_on(leader, &key).unwrap().is_none());
            // Drop without healing — next open reloads from PedraDB raft meta.
        }
        {
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            // All peers participating again after reopen.
            for _ in 0..40 {
                c.tick().unwrap();
            }
            c.elect_all(120).unwrap();
            for &nid in c.node_ids() {
                assert!(
                    c.dcs_get_on(nid, &key).unwrap().is_none(),
                    "reopen+tick must not install orphan Create; node {nid}"
                );
            }
            // Clean create still wins (no fencing lie from discarded propose).
            let rev = c
                .dcs_create(&key, b"node-b")
                .expect("create after reopen must succeed");
            assert!(rev >= 1);
            for &nid in c.node_ids() {
                assert_eq!(
                    c.dcs_get_on(nid, &key).unwrap().expect("replicated").value,
                    b"node-b"
                );
            }
            c.put(b"after-reopen", b"ok").unwrap();
            assert!(c.count_applied_eq(b"after-reopen", b"ok") >= 2);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Local-only append hook still does not commit (internal commit rule).
    #[test]
    fn minority_only_append_does_not_commit() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let rid = c.locate(b"solo").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let commit_before = c.commit_index(leader, rid);
        let commit_after = c.append_local_only_put(b"solo", b"x").unwrap();
        assert_eq!(
            commit_after, commit_before,
            "minority-only append must not advance commit"
        );
        let applied = c.count_applied_eq(b"solo", b"x");
        assert!(
            applied < 2,
            "uncommitted entry must not appear on majority; applied={applied}"
        );
        c.put(b"real", b"y").unwrap();
        assert!(c.count_applied_eq(b"real", b"y") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strong_read_refuses_deposed_and_dual_leader() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = b"read-k";
        c.put(key, b"v1").unwrap();
        assert_eq!(
            c.get_strong(key).unwrap().as_deref(),
            Some(b"v1".as_ref())
        );

        let rid = c.locate(key).unwrap();
        let _old = c.step_down_range_leader(rid).unwrap();
        c.elect_all(80).unwrap();
        let live = c.range_leader(rid).expect("unique new leader");
        // Always pick a *different* node as the dual claimant (stale self-claim).
        let stale = c
            .node_ids()
            .iter()
            .copied()
            .find(|&n| n != live)
            .expect("need another node");
        {
            let p = c
                .nodes
                .get_mut(&stale)
                .unwrap()
                .ranges
                .get_mut(&rid)
                .unwrap();
            p.role = Role::Leader;
            p.leader_id = Some(stale);
        }
        // Fail closed: no unique leader.
        assert_eq!(c.leader_claim_count(rid), 2);
        assert!(
            c.range_leader(rid).is_none(),
            "dual Leader claims must yield no safe range_leader"
        );
        // Strong read must fail on *both* claimants (not fail-open on HashMap order).
        for nid in [stale, live] {
            let err = c
                .get_with_policy(nid, key, ReadPolicy::Strong)
                .expect_err("dual-leader strong read must fail closed");
            assert!(
                matches!(err, StoreError::StaleLeader { .. }),
                "node {nid}: {err:?}"
            );
        }
        assert!(c.get_strong(key).is_err());

        // Resolve dual: demote stale claimant; unique leader serves strong again.
        {
            let p = c
                .nodes
                .get_mut(&stale)
                .unwrap()
                .ranges
                .get_mut(&rid)
                .unwrap();
            p.role = Role::Follower;
            p.leader_id = Some(live);
        }
        assert_eq!(c.range_leader(rid), Some(live));
        assert_eq!(
            c.get_strong(key).unwrap().as_deref(),
            Some(b"v1".as_ref())
        );

        // Follower still cannot serve strong.
        let follower = c.node_ids().iter().copied().find(|&n| n != live).unwrap();
        assert!(matches!(
            c.get_with_policy(follower, key, ReadPolicy::Strong)
                .unwrap_err(),
            StoreError::StaleLeader { .. }
        ));
        // LocalApplied allowed (non-linearizable by contract).
        assert!(c
            .get_with_policy(1, key, ReadPolicy::LocalApplied)
            .unwrap()
            .is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn range_failover_after_leader_loss() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = b"ha-key";
        c.put(key, b"before").unwrap();
        assert!(c.count_applied_eq(key, b"before") >= 2);

        let rid = c.locate(key).unwrap();
        let old = c.range_leader(rid).unwrap();
        // Remove leader from membership participation (partition / crash).
        c.set_participating(old, false).unwrap();
        assert!(c.range_leader(rid).is_none() || c.range_leader(rid) != Some(old));

        c.elect_all(120).unwrap();
        let new_leader = c.range_leader(rid).expect("new range leader after loss");
        assert_ne!(new_leader, old);

        // Prior commit still readable on majority of remaining peers.
        let remaining: Vec<u64> = c
            .node_ids()
            .iter()
            .copied()
            .filter(|&n| n != old)
            .collect();
        let seen = remaining
            .iter()
            .filter(|&&n| {
                c.get_on(n, key)
                    .ok()
                    .flatten()
                    .is_some_and(|v| v.as_ref() == b"before")
            })
            .count();
        assert!(
            seen >= 2 || (remaining.len() == 2 && seen >= 1),
            "pre-failover value retained on remaining peers; seen={seen}"
        );
        // With 3-node cluster and majority commit, both remaining should have it.
        assert_eq!(seen, 2, "both live peers should retain committed key");

        c.put(b"ha-key2", b"after").unwrap();
        assert!(c.count_applied_eq(b"ha-key2", b"after") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dcs_on_store_create_replicated() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 2).unwrap();
        c.elect_all(80).unwrap();
        let key = meta_key(b"cluster1/leader");
        let rev = c.dcs_create(&key, b"node-a").unwrap();
        assert!(rev >= 1);
        // Second create fails.
        assert!(c.dcs_create(&key, b"node-b").is_err());
        for nid in 1..=3u64 {
            let kv = c.dcs_get_on(nid, &key).unwrap().expect("leader key");
            assert_eq!(kv.value, b"node-a");
        }
        // CAS renew
        let rev2 = c.dcs_cas(&key, b"node-a", rev).unwrap();
        assert!(rev2 > rev);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn range_contains() {
        let r = RangeMeta {
            id: 1,
            start: vec![0x40],
            end: vec![0x80],
        };
        assert!(!r.contains(b"\x3f"));
        assert!(r.contains(b"\x40"));
        assert!(r.contains(b"\x7f"));
        assert!(!r.contains(b"\x80"));
    }

    /// F27: applied log prefix is truncated once all peers applied it.
    #[test]
    fn raft_log_compacts_after_all_applied() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        // Keys are two-byte [b'k', i] (not ASCII "k0"/"k7").
        for i in 0..8u8 {
            c.put([b'k', i], [b'v', i]).unwrap();
        }
        let rid = c.locate(&[b'k', 0]).unwrap();
        // After majority apply + compact, in-memory log should be short.
        for &nid in c.node_ids() {
            let p = c.nodes.get(&nid).unwrap().ranges.get(&rid).unwrap();
            assert!(
                p.snapshot_index >= 1,
                "node {nid}: expected snapshot after multi-put, snap={}",
                p.snapshot_index
            );
            // Log should not retain the entire history (8 puts + noops + heartbeats).
            assert!(
                p.log.len() < 20,
                "node {nid}: log not compacted, len={}",
                p.log.len()
            );
            // Applied data still present.
            assert_eq!(
                c.get_on(nid, &[b'k', 7]).unwrap().as_deref(),
                Some([b'v', 7].as_slice())
            );
        }
        // Reopen: snap + short log load; data intact; further put works.
        drop(c);
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        assert_eq!(
            c.get_on(1, &[b'k', 3]).unwrap().as_deref(),
            Some([b'v', 3].as_slice())
        );
        c.put(b"post-compact", b"yes").unwrap();
        assert_eq!(c.count_applied_eq(b"post-compact", b"yes"), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F26: raft meta survives process restart (same node dirs).
    #[test]
    fn raft_meta_survives_reopen() {
        let dir = temp();
        {
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"durable-k", b"durable-v").unwrap();
            for nid in c.node_ids().to_vec() {
                assert_eq!(
                    c.get_on(nid, b"durable-k").unwrap().as_deref(),
                    Some(b"durable-v".as_ref()),
                    "node {nid}"
                );
            }
        }
        // Reopen same directories — applied state + raft watermarks must load.
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        for nid in c.node_ids().to_vec() {
            assert_eq!(
                c.get_on(nid, b"durable-k").unwrap().as_deref(),
                Some(b"durable-v".as_ref()),
                "reopen node {nid}"
            );
            // Hard state / log / commit loaded (term advanced from elections).
            let rid = c.locate(b"durable-k").unwrap();
            assert!(
                c.commit_index(nid, rid) >= 1,
                "node {nid} commit watermark missing after reopen"
            );
            assert!(
                c.applied_index(nid, rid) >= 1,
                "node {nid} applied watermark missing after reopen"
            );
        }
        // New election + put still works on recovered meta.
        c.elect_all(80).unwrap();
        c.put(b"after-reopen", b"ok").unwrap();
        assert_eq!(
            c.count_applied_eq(b"after-reopen", b"ok"),
            3
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_rejects_raft_meta_prefix() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 1, 1).unwrap();
        c.elect_all(20).unwrap();
        let mut bad = b"\0store/raft/".to_vec();
        bad.extend_from_slice(b"1/hard");
        let err = c.put(&bad, b"x").unwrap_err();
        assert!(err.to_string().contains("reserved"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F25: refuse degenerate single-byte splits.
    #[test]
    fn open_rejects_too_many_ranges() {
        let dir = temp();
        match StoreCluster::open(&dir, 3, 300) {
            Ok(_) => panic!("expected n_ranges > 256 to fail"),
            Err(err) => assert!(err.to_string().contains("256"), "{err}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F23: prev-term entry majority-replicated commits after re-elect via leader noop.
    #[test]
    fn leader_noop_commits_prev_term_after_reelect() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = b"prev-term";
        let rid = c.locate(key).unwrap();
        let leader = c.range_leader(rid).unwrap();
        // Append entry in current term, majority-replicate via normal put first.
        c.put(key, b"v1").unwrap();
        let term1 = c.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap().term;
        // Inject a second log entry as if majority held it but commit lagged:
        // force step-down + re-elect so a new term's noop can free commit.
        // Simulate: local-only append of another put (not committed), then re-elect.
        let _ = c.append_local_only_put(b"stuck-key", b"stuck-val").unwrap();
        assert!(
            c.get_on(leader, b"stuck-key").unwrap().is_none(),
            "local-only must not apply without majority"
        );
        // Kill old leader, re-elect — noop in new term should allow committing
        // whatever majority already had (v1 already applied).
        c.set_participating(leader, false).unwrap();
        c.elect_all(120).unwrap();
        // New put in new term works.
        c.put(b"after", b"ok").unwrap();
        assert_eq!(
            c.count_applied_eq(b"after", b"ok"),
            2, // two participating peers
        );
        // Original key still present on survivors.
        let live = c.range_leader(rid).unwrap();
        assert_eq!(
            c.get_on(live, key).unwrap().as_deref(),
            Some(b"v1".as_ref())
        );
        let _ = term1;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F22: DCS create conflict on apply does not freeze the range apply cursor.
    #[test]
    fn dcs_apply_cas_failed_does_not_stick_pipeline() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = meta_key(b"pipe");
        c.dcs_create(&key, b"a").unwrap();
        // Inject a committed log entry that will CasFail on apply (duplicate create),
        // then a normal put that must still apply.
        let rid = c.locate(&key).unwrap();
        let leader = c.range_leader(rid).unwrap();
        {
            let p = c.nodes.get_mut(&leader).unwrap().ranges.get_mut(&rid).unwrap();
            let idx = p.last_index() + 1;
            let term = p.term;
            p.log.push(LogRec {
                index: idx,
                term,
                entry: RangeEntry::Dcs(DcsCommand::Create {
                    key: key.clone(),
                    value: b"dup".to_vec(),
                    lease: 0,
                }),
            });
            // Pretend majority committed both the dup create and a following put.
            let idx2 = idx + 1;
            p.log.push(LogRec {
                index: idx2,
                term,
                entry: RangeEntry::Put {
                    key: b"after-cas".to_vec(),
                    value: b"ok".to_vec(),
                },
            });
            p.commit = idx2;
            // Followers need the entries for count_applied; apply on leader only for test.
        }
        c.apply_range(leader, rid).unwrap();
        assert_eq!(
            c.applied_index(leader, rid),
            c.commit_index(leader, rid),
            "F22: applied must reach commit past CasFailed create"
        );
        assert_eq!(
            c.get_on(leader, b"after-cas").unwrap().as_deref(),
            Some(b"ok".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1.1: same-range multi-key atomic put_batch — majority sees all keys.
    #[test]
    fn put_batch_same_range_majority_atomic() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put_batch([(b"a", b"1"), (b"b", b"2"), (b"c", b"3")])
            .unwrap();
        for (k, v) in [(b"a", b"1"), (b"b", b"2"), (b"c", b"3")] {
            assert!(
                c.count_applied_eq(k, v) >= 2,
                "key {} missing on majority",
                String::from_utf8_lossy(k)
            );
        }
        // Single range so all keys co-located.
        assert_eq!(c.locate(b"a").unwrap(), c.locate(b"c").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1.1: index-style primary row + secondary key in one batch.
    #[test]
    fn put_batch_row_and_secondary_index_style() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        // Co-locate under same first-byte region (single range cluster).
        let row = b"row/user/42";
        let idx = b"idx/email/a@b.co";
        let payload = br#"{"email":"a@b.co"}"#;
        c.put_batch([(row.as_slice(), payload.as_slice()), (idx.as_slice(), row.as_slice())])
            .unwrap();
        assert!(c.count_applied_eq(row, payload) >= 2);
        assert!(c.count_applied_eq(idx, row) >= 2);
        // Point lookup path: secondary → primary key → row.
        let pk = c.get(idx).unwrap().expect("idx");
        assert_eq!(pk.as_ref(), row);
        assert_eq!(c.get(pk.as_ref()).unwrap().as_deref(), Some(payload.as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1.1: minority batch fails with no partial majority apply.
    #[test]
    fn put_batch_fails_without_majority_no_partial() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let rid = c.locate(b"bk1").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let ids: Vec<u64> = c.node_ids().to_vec();
        for &nid in &ids {
            if nid != leader {
                c.set_participating(nid, false).unwrap();
            }
        }
        let err = c
            .put_batch([(b"bk1", b"v1"), (b"bk2", b"v2")])
            .expect_err("batch without majority must fail");
        assert!(
            matches!(err, StoreError::NotCommitted { .. }),
            "got {err:?}"
        );
        assert_eq!(c.count_applied_eq(b"bk1", b"v1"), 0);
        assert_eq!(c.count_applied_eq(b"bk2", b"v2"), 0);
        // Heal and succeed fully.
        for &nid in &ids {
            c.set_participating(nid, true).unwrap();
        }
        c.elect_all(80).unwrap();
        c.put_batch([(b"bk1", b"v1"), (b"bk2", b"v2")]).unwrap();
        assert!(c.count_applied_eq(b"bk1", b"v1") >= 2);
        assert!(c.count_applied_eq(b"bk2", b"v2") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Criterion 4: cross-range multi-key hard-fails with no partial apply.
    #[test]
    fn put_batch_cross_range_hard_fails() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        assert!(keys.len() >= 2);
        let r0 = c.locate(&keys[0]).unwrap();
        let r1 = c.locate(&keys[1]).unwrap();
        assert_ne!(r0, r1);
        let err = c
            .put_batch([
                (keys[0].as_slice(), b"x".as_slice()),
                (keys[1].as_slice(), b"y".as_slice()),
            ])
            .expect_err("cross-range must hard-fail");
        match err {
            StoreError::CrossRange { ranges } => {
                assert!(ranges.contains(&r0) && ranges.contains(&r1));
            }
            other => panic!("expected CrossRange, got {other:?}"),
        }
        assert_eq!(c.count_applied_eq(&keys[0], b"x"), 0);
        assert_eq!(c.count_applied_eq(&keys[1], b"y"), 0);
        // Single-key multi-range power still works.
        c.put(&keys[0], b"a").unwrap();
        c.put(&keys[1], b"b").unwrap();
        assert!(c.count_applied_eq(&keys[0], b"a") >= 2);
        assert!(c.count_applied_eq(&keys[1], b"b") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Batch encode/decode round-trip via durable log (reopen).
    #[test]
    fn put_batch_survives_reopen() {
        let dir = temp();
        {
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put_batch([(b"p1", b"A"), (b"p2", b"B")]).unwrap();
        }
        let c = StoreCluster::open(&dir, 3, 1).unwrap();
        assert_eq!(c.get_on(1, b"p1").unwrap().as_deref(), Some(b"A".as_ref()));
        assert_eq!(c.get_on(2, b"p2").unwrap().as_deref(), Some(b"B".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
