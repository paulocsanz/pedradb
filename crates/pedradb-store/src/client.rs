//! Client contract for multi-host Montanha (RFC-0017 P2.2 + RFC-0021 P0 TX).
//!
//! # Error classes
//!
//! | Class | Meaning | Client action |
//! |-------|---------|---------------|
//! | [`ClientClass::NotLeader`] | Contacted non-leader (or no leader) | Retry on `leader` hint if known, else refresh status and retry |
//! | [`ClientClass::StaleLeader`] | Strong read on deposed leader | Same as NotLeader |
//! | [`ClientClass::NotCommitted`] | Majority not reached | Retry with backoff (may elect / heal) |
//! | [`ClientClass::Conflict`] | TX / CAS conflict | App-level retry or abort |
//! | [`ClientClass::LimitRejected`] | Value or TX size over limit | Fix payload; do not retry same size |
//! | [`ClientClass::Unavailable`] | Dial / timeout / busy | Retry other peers |
//! | [`ClientClass::Other`] | Bug / protocol | Fail or log |
//!
//! # Client TX (RFC-0021 P0.1)
//!
//! ```ignore
//! let mut tx = cluster.tx_begin(); // or PendingTx::new()
//! tx.set(b"a", b"1")?;
//! tx.set(b"b", b"2")?;
//! let tid = tx.commit(&mut cluster)?; // majority via commit_tx; no range id
//! ```
//!
//! TCP: [`TcpClusterClient::commit_pairs`] / [`PendingTx`] + [`client_commit_tx`].
//!
//! # Retry policy (writes)
//!
//! 1. Prefer **per-range** last-known leader (multi-Raft), else global prefer.
//! 2. On NotLeader with `leader: Some(id)` → cache that range→leader and retry.
//! 3. Else probe `status` on each peer; parse **all** `r*:leader=N` into the map.
//! 4. Cap attempts; never invent a leader without a hint or status.
//!
//! Reads with linearizability must go to the leader (same routing as put).
//! Stale local reads are out of band of this helper (local Pedra open).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::tcp::{client_commit_tx, client_put, client_put_batch, client_set_peers, client_status};
use crate::{
    validate_tx_pairs, Result, StoreCluster, StoreError, MAX_TX_BYTES, MAX_TX_KEYS, MAX_VALUE_BYTES,
};

/// Max commit-generation lag for a snapshot TX before [`StoreError::TransactionTooOld`]
/// (RFC-0022; FDB-class "too old" at lab scale — generation units, not wall clock).
pub const MAX_SNAPSHOT_LAG: u64 = 1_000_000;

/// Stable classification of store/transport errors for clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientClass {
    /// Not the range leader; optional routing hint.
    NotLeader {
        /// Range id when known.
        range_id: Option<u64>,
        /// Suggested leader node id.
        leader: Option<u64>,
    },
    /// Strong read refused (stale leadership view).
    StaleLeader {
        /// Range id.
        range_id: Option<u64>,
        /// Live leader if known.
        live_leader: Option<u64>,
    },
    /// Majority not reached for a propose.
    NotCommitted {
        /// Range id when known.
        range_id: Option<u64>,
    },
    /// Intent / CAS conflict.
    Conflict,
    /// Value or transaction exceeded size limits (do not blind-retry).
    LimitRejected {
        /// Human-readable limit kind (`value` / `transaction`).
        kind: &'static str,
        /// Observed size.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// Network / timeout / busy.
    Unavailable(String),
    /// Everything else.
    Other(String),
}

/// Classify a [`StoreError`] for client retry logic.
#[must_use]
pub fn classify(err: &StoreError) -> ClientClass {
    match err {
        StoreError::NotLeader { range_id, leader } => ClientClass::NotLeader {
            range_id: Some(*range_id),
            leader: *leader,
        },
        StoreError::StaleLeader {
            range_id,
            live_leader,
            ..
        } => ClientClass::StaleLeader {
            range_id: Some(*range_id),
            live_leader: *live_leader,
        },
        StoreError::NotCommitted { range_id, .. } => ClientClass::NotCommitted {
            range_id: Some(*range_id),
        },
        StoreError::Conflict | StoreError::TxnAborted(_) | StoreError::TransactionTooOld { .. } => {
            ClientClass::Conflict
        }
        StoreError::ValueTooLarge { size, limit } => ClientClass::LimitRejected {
            kind: "value",
            size: *size,
            limit: *limit,
        },
        StoreError::TransactionTooLarge { size, limit } => ClientClass::LimitRejected {
            kind: "transaction",
            size: *size,
            limit: *limit,
        },
        // Retryable after compact / flush — not a permanent limit.
        StoreError::WriteStall { l0_files, limit } => {
            ClientClass::Unavailable(format!("write stall L0={l0_files} limit={limit}"))
        }
        StoreError::WriteStallMem { mem_bytes, limit } => {
            ClientClass::Unavailable(format!("write stall mem={mem_bytes}B limit={limit}B"))
        }
        StoreError::Msg(m) => classify_message(m),
        other => ClientClass::Other(other.to_string()),
    }
}

/// Parse string errors (TCP `RespErr` bodies often use Display of StoreError).
#[must_use]
pub fn classify_message(m: &str) -> ClientClass {
    let lower = m.to_ascii_lowercase();
    if lower.contains("write stall") {
        return ClientClass::Unavailable(m.to_string());
    }
    if lower.contains("not leader") {
        return ClientClass::NotLeader {
            range_id: parse_range_id(m),
            leader: parse_leader_hint(m),
        };
    }
    if lower.contains("stale") && lower.contains("leader") {
        return ClientClass::StaleLeader {
            range_id: parse_range_id(m),
            live_leader: parse_leader_hint(m),
        };
    }
    if lower.contains("not committed") {
        return ClientClass::NotCommitted {
            range_id: parse_range_id(m),
        };
    }
    if lower.contains("conflict")
        || lower.contains("aborted")
        || lower.contains("transaction too old")
    {
        return ClientClass::Conflict;
    }
    if lower.contains("value too large") {
        return ClientClass::LimitRejected {
            kind: "value",
            size: 0,
            limit: MAX_VALUE_BYTES,
        };
    }
    if lower.contains("transaction too large") {
        return ClientClass::LimitRejected {
            kind: "transaction",
            size: 0,
            limit: MAX_TX_BYTES,
        };
    }
    if lower.contains("timeout")
        || lower.contains("connect")
        || lower.contains("busy")
        || lower.contains("connection")
        || lower.contains("broken pipe")
        || lower.contains("reset")
    {
        return ClientClass::Unavailable(m.to_string());
    }
    ClientClass::Other(m.to_string())
}

// ── Pending TX session (RFC-0021 P0.1) ────────────────────────────────────

/// Client-facing pending transaction: buffer writes, atomic commit via store.
///
/// Does **not** require the app to name a range leader. Commit uses
/// [`StoreCluster::commit_tx`] (in-process) or TCP `CommitTx` (multi-host).
#[derive(Debug, Default, Clone)]
pub struct PendingTx {
    /// Key → value (last write wins per key).
    ops: HashMap<Vec<u8>, Vec<u8>>,
}

impl PendingTx {
    /// Empty TX.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ops: HashMap::new(),
        }
    }

    /// Number of distinct keys staged.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether no keys staged.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Stage a put (overwrite if key already staged).
    ///
    /// # Errors
    /// [`StoreError::ValueTooLarge`] / [`StoreError::TransactionTooLarge`].
    pub fn set(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let k = key.as_ref().to_vec();
        let v = value.as_ref().to_vec();
        if v.len() > MAX_VALUE_BYTES {
            return Err(StoreError::ValueTooLarge {
                size: v.len(),
                limit: MAX_VALUE_BYTES,
            });
        }
        // Provisional total with this key replacing prior value if any.
        let mut total = 0usize;
        let mut keys = 0usize;
        let mut replaced = false;
        for (ek, ev) in &self.ops {
            if ek == &k {
                total = total.saturating_add(ek.len()).saturating_add(v.len());
                replaced = true;
            } else {
                total = total.saturating_add(ek.len()).saturating_add(ev.len());
            }
            keys += 1;
        }
        if !replaced {
            total = total.saturating_add(k.len()).saturating_add(v.len());
            keys += 1;
        }
        if keys > MAX_TX_KEYS {
            return Err(StoreError::TransactionTooLarge {
                size: keys,
                limit: MAX_TX_KEYS,
            });
        }
        if total > MAX_TX_BYTES {
            return Err(StoreError::TransactionTooLarge {
                size: total,
                limit: MAX_TX_BYTES,
            });
        }
        self.ops.insert(k, v);
        Ok(())
    }

    /// Read a key from the TX buffer (not the store).
    #[must_use]
    pub fn get_buffered(&self, key: &[u8]) -> Option<&[u8]> {
        self.ops.get(key).map(|v| v.as_slice())
    }

    /// Drop all staged writes.
    pub fn cancel(self) {}

    /// Pairs snapshot for commit / TCP.
    #[must_use]
    pub fn pairs(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.ops
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Commit against an in-process [`StoreCluster`] (majority via `commit_tx`).
    ///
    /// # Errors
    /// Empty TX, limits, Conflict, NotLeader, NotCommitted, I/O.
    pub fn commit(self, cluster: &mut StoreCluster) -> Result<u64> {
        let pairs = self.pairs();
        validate_tx_pairs(&pairs)?;
        cluster.commit_tx(pairs)
    }
}

// ── Unified snapshot TX (RFC-0023 default native API) ─────────────────────

/// Default native client transaction (RFC-0023): snapshot-at-begin + OCC.
///
/// - Captures [`StoreCluster::read_version`] at begin.
/// - [`Self::get`] / [`Self::get_range`] see the **snapshot**, not concurrent commits
///   after begin (plus own write-set).
/// - Commit: Conflict if read/write/range sets race; TransactionTooOld if GC watermark
///   advanced past the snapshot.
/// - Leadership-invisible (no range / node ids).
#[derive(Debug, Clone)]
pub struct Transaction {
    snapshot: u64,
    ops: HashMap<Vec<u8>, Vec<u8>>,
    clears: HashSet<Vec<u8>>,
    read_keys: HashSet<Vec<u8>>,
    /// Half-open `[start, end)` conflict ranges from [`Self::get_range`].
    conflict_ranges: Vec<(Vec<u8>, Vec<u8>)>,
}

/// Back-compat alias (RFC-0022 name).
pub type SnapshotTx = Transaction;

impl Transaction {
    /// Open at an explicit snapshot generation (prefer [`StoreCluster::begin`]).
    #[must_use]
    pub fn at_version(snapshot: u64) -> Self {
        Self {
            snapshot,
            ops: HashMap::new(),
            clears: HashSet::new(),
            read_keys: HashSet::new(),
            conflict_ranges: Vec::new(),
        }
    }

    /// Snapshot generation captured at begin.
    #[must_use]
    pub fn snapshot_version(&self) -> u64 {
        self.snapshot
    }

    /// Number of staged writes (sets; clears counted separately).
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len() + self.clears.len()
    }

    /// Whether no writes staged.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.clears.is_empty()
    }

    /// Snapshot read (tracks OCC read-set). Own write-set wins.
    ///
    /// # Errors
    /// Store errors.
    pub fn get(
        &mut self,
        cluster: &StoreCluster,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        let k = key.as_ref();
        self.read_keys.insert(k.to_vec());
        if self.clears.contains(k) {
            return Ok(None);
        }
        if let Some(v) = self.ops.get(k) {
            return Ok(Some(v.clone()));
        }
        cluster.get_at_version(k, self.snapshot)
    }

    /// Range read at snapshot; registers conflict range `[start, end)` (end empty = +∞).
    ///
    /// # Errors
    /// Store errors.
    pub fn get_range(
        &mut self,
        cluster: &StoreCluster,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let s = start.as_ref().to_vec();
        let e = end.as_ref().to_vec();
        self.conflict_ranges.push((s.clone(), e.clone()));
        let mut out = cluster.keys_in_range_at(&s, &e, self.snapshot)?;
        // Overlay local writes / clears.
        out.retain(|(k, _)| !self.clears.contains(k));
        for (k, v) in &self.ops {
            if crate::key_in_half_open(k, &s, &e) {
                if let Some(pos) = out.iter().position(|(ok, _)| ok == k) {
                    out[pos].1 = v.clone();
                } else {
                    out.push((k.clone(), v.clone()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Stage a put (same limits as [`PendingTx::set`]).
    ///
    /// # Errors
    /// Value / TX size limits.
    pub fn set(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let k = key.as_ref().to_vec();
        let v = value.as_ref().to_vec();
        if v.len() > MAX_VALUE_BYTES {
            return Err(StoreError::ValueTooLarge {
                size: v.len(),
                limit: MAX_VALUE_BYTES,
            });
        }
        let mut total = 0usize;
        let mut keys = 0usize;
        let mut replaced = false;
        for (ek, ev) in &self.ops {
            if ek == &k {
                total = total.saturating_add(ek.len()).saturating_add(v.len());
                replaced = true;
            } else {
                total = total.saturating_add(ek.len()).saturating_add(ev.len());
            }
            keys += 1;
        }
        if !replaced {
            total = total.saturating_add(k.len()).saturating_add(v.len());
            keys += 1;
        }
        keys = keys.saturating_add(self.clears.len());
        if keys > MAX_TX_KEYS {
            return Err(StoreError::TransactionTooLarge {
                size: keys,
                limit: MAX_TX_KEYS,
            });
        }
        if total > MAX_TX_BYTES {
            return Err(StoreError::TransactionTooLarge {
                size: total,
                limit: MAX_TX_BYTES,
            });
        }
        self.clears.remove(&k);
        self.ops.insert(k, v);
        Ok(())
    }

    /// Stage a clear/delete. On commit, Montanha applies Pedra **`delete`**
    /// (empty payload is the wire signal for delete — not a stored empty value).
    ///
    /// # Errors
    /// TX size limits.
    pub fn clear(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        let k = key.as_ref().to_vec();
        self.ops.remove(&k);
        self.clears.insert(k);
        if self.ops.len() + self.clears.len() > MAX_TX_KEYS {
            return Err(StoreError::TransactionTooLarge {
                size: self.ops.len() + self.clears.len(),
                limit: MAX_TX_KEYS,
            });
        }
        Ok(())
    }

    /// Clear every key currently visible in `[start, end)` at the snapshot
    /// (FDB-shaped `clear_range` seed — stages per-key clears + range conflict).
    ///
    /// # Errors
    /// Store / TX size limits.
    pub fn clear_range(
        &mut self,
        cluster: &StoreCluster,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<()> {
        let pairs = self.get_range(cluster, start, end)?;
        for (k, _) in pairs {
            self.clear(k)?;
        }
        Ok(())
    }

    /// Staged set pairs; clears are empty values (**delete** on apply).
    #[must_use]
    pub fn pairs(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out: Vec<(Vec<u8>, Vec<u8>)> = self
            .ops
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for k in &self.clears {
            out.push((k.clone(), Vec::new()));
        }
        out
    }

    /// OCC + majority commit (leadership-invisible).
    ///
    /// # Errors
    /// Empty TX, Conflict, TransactionTooOld, limits, NotLeader, NotCommitted.
    pub fn commit(self, cluster: &mut StoreCluster) -> Result<u64> {
        let pairs = self.pairs();
        if pedradb_core::write_admission_kernel::batch_is_empty(pairs.len() as u64) {
            return Err(StoreError::Msg("empty transaction".into()));
        }
        validate_tx_pairs(&pairs)?;
        cluster.commit_transaction(self.snapshot, self.read_keys, pairs, self.conflict_ranges)
    }
}

fn parse_range_id(m: &str) -> Option<u64> {
    // "not leader of range 1 (...)"
    if let Some(rest) = m.split("range ").nth(1) {
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return num.parse().ok();
    }
    None
}

fn parse_leader_hint(m: &str) -> Option<u64> {
    // leader=Some(2) or leader=2 or leader:Some(2)
    for key in [
        "leader=Some(",
        "leader=",
        "live_leader=Some(",
        "live_leader=",
    ] {
        if let Some(i) = m.find(key) {
            let rest = &m[i + key.len()..];
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !num.is_empty() {
                return num.parse().ok();
            }
        }
    }
    // status text: leader of lowest range id (stable; not HashMap iteration order)
    leader_from_status(m)
}

/// Parse all `rN:leader=M` tokens from status_text into range_id → node_id.
#[must_use]
pub fn leaders_from_status(status: &str) -> HashMap<u64, u64> {
    let mut out = HashMap::new();
    for part in status.split_whitespace() {
        let Some(rest) = part.strip_prefix('r') else {
            continue;
        };
        let Some((rid_s, lead_s)) = rest.split_once(":leader=") else {
            continue;
        };
        if lead_s == "-" {
            continue;
        }
        if let (Ok(rid), Ok(lid)) = (rid_s.parse::<u64>(), lead_s.parse::<u64>()) {
            out.insert(rid, lid);
        }
    }
    out
}

/// Parse leader id from status_text — leader of the **lowest** range id that has one.
#[must_use]
pub fn leader_from_status(status: &str) -> Option<u64> {
    let m = leaders_from_status(status);
    m.keys().min().and_then(|k| m.get(k).copied())
}

impl ClientClass {
    /// Best-effort map back to a [`StoreError`] for TCP string bodies.
    #[must_use]
    pub fn into_store_err(&self, raw: &str) -> StoreError {
        match self {
            ClientClass::NotLeader { range_id, leader } => StoreError::NotLeader {
                range_id: range_id.unwrap_or(0),
                leader: *leader,
            },
            ClientClass::StaleLeader {
                range_id,
                live_leader,
            } => StoreError::StaleLeader {
                range_id: range_id.unwrap_or(0),
                node_id: 0,
                live_leader: *live_leader,
            },
            ClientClass::NotCommitted { range_id } => StoreError::NotCommitted {
                range_id: range_id.unwrap_or(0),
                index: 0,
                commit: 0,
            },
            ClientClass::Conflict => StoreError::Conflict,
            ClientClass::LimitRejected {
                kind: "value",
                size,
                limit,
            } => StoreError::ValueTooLarge {
                size: *size,
                limit: *limit,
            },
            ClientClass::LimitRejected { size, limit, .. } => StoreError::TransactionTooLarge {
                size: *size,
                limit: *limit,
            },
            ClientClass::Unavailable(m) => StoreError::Msg(m.clone()),
            ClientClass::Other(_) => StoreError::Msg(raw.to_string()),
        }
    }
}

/// TCP multi-peer client with NotLeader routing and region-aware dial (RFC-0021).
#[derive(Debug, Clone)]
pub struct TcpClusterClient {
    /// node_id → `host:port`
    peers: HashMap<u64, String>,
    /// node_id → region label (lab multi-site).
    regions: HashMap<u64, String>,
    /// Prefer dialing peers in this region first (P2.6).
    prefer_region: Option<String>,
    /// Last known leader for single-range / unknown-range ops.
    prefer: Option<u64>,
    /// Multi-Raft: range_id → known leader node_id (from status / NotLeader).
    leaders: HashMap<u64, u64>,
    /// Known range for upcoming ops (partitioned clients / lab multi-Raft).
    active_range: Option<u64>,
    /// Max put attempts across peers.
    max_attempts: u32,
}

impl TcpClusterClient {
    /// Build from `(id, host:port)` list.
    #[must_use]
    pub fn new(peers: impl IntoIterator<Item = (u64, String)>) -> Self {
        Self {
            peers: peers.into_iter().collect(),
            regions: HashMap::new(),
            prefer_region: None,
            prefer: None,
            leaders: HashMap::new(),
            active_range: None,
            max_attempts: 8,
        }
    }

    /// Override attempt budget.
    #[must_use]
    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n.max(1);
        self
    }

    /// Pin dial preference to a known range (lab multi-Raft / partitioned clients).
    #[must_use]
    pub fn with_active_range(mut self, range_id: u64) -> Self {
        self.set_active_range(Some(range_id));
        self
    }

    /// Set or clear the active range for subsequent writes.
    pub fn set_active_range(&mut self, range_id: Option<u64>) {
        self.active_range = range_id;
        if let Some(rid) = range_id {
            if let Some(lid) = self.leaders.get(&rid).copied() {
                self.prefer = Some(lid);
            }
        }
    }

    /// Tag a member with a region (RFC-0021 P2.6).
    pub fn set_region(&mut self, node_id: u64, region: impl Into<String>) {
        self.regions.insert(node_id, region.into());
    }

    /// Prefer same-region dials when routing (failover to other regions after).
    pub fn set_prefer_region(&mut self, region: Option<impl Into<String>>) {
        self.prefer_region = region.map(|r| r.into());
    }

    /// Cached preferred leader (global; multi-range see [`Self::leader_for_range`]).
    #[must_use]
    pub fn preferred_leader(&self) -> Option<u64> {
        self.prefer
    }

    /// Cached leader for a range, if known.
    #[must_use]
    pub fn leader_for_range(&self, range_id: u64) -> Option<u64> {
        self.leaders.get(&range_id).copied()
    }

    /// Current peer map (for tests / ops).
    #[must_use]
    pub fn peer_map(&self) -> &HashMap<u64, String> {
        &self.peers
    }

    /// Current per-range leader map (tests / observability).
    #[must_use]
    pub fn range_leaders(&self) -> &HashMap<u64, u64> {
        &self.leaders
    }

    /// RFC-0021 P1.3: push dial map to **every** reachable peer via TCP `SetPeers`
    /// (no host SSH). Updates local map on success to any peer.
    ///
    /// # Errors
    /// If **no** peer accepts the map.
    pub fn rewire_peer_map(&mut self, peers: &[(u64, String)]) -> Result<()> {
        if pedradb_core::write_admission_kernel::batch_is_empty(peers.len() as u64) {
            return Err(StoreError::Msg("empty peer map".into()));
        }
        let list: Vec<(u64, String)> = peers.to_vec();
        let mut ok = 0u32;
        let mut last = StoreError::Msg("no peers contacted".into());
        // Contact current map first, then new map addresses.
        let mut targets: Vec<String> = self.peers.values().cloned().collect();
        for (_, addr) in &list {
            if !targets.contains(addr) {
                targets.push(addr.clone());
            }
        }
        for addr in targets {
            match client_set_peers(&addr, &list) {
                Ok(()) => ok += 1,
                Err(e) => last = e,
            }
        }
        if ok == 0 {
            return Err(last);
        }
        self.peers = list.into_iter().collect();
        Ok(())
    }

    /// Ordered dial list for optional range: that range's leader, else global prefer,
    /// then same-region, then others by id.
    fn dial_order_for(&self, range_id: Option<u64>) -> Vec<u64> {
        let mut ids: Vec<u64> = self.peers.keys().copied().collect();
        ids.sort_unstable();
        // Partition by region preference.
        if let Some(ref pref) = self.prefer_region {
            let mut same = Vec::new();
            let mut other = Vec::new();
            for id in ids {
                if self.regions.get(&id).map(|s| s.as_str()) == Some(pref.as_str()) {
                    same.push(id);
                } else {
                    other.push(id);
                }
            }
            ids = same;
            ids.extend(other);
        }
        let prefer_node = range_id
            .and_then(|r| self.leaders.get(&r).copied())
            .or(self.prefer);
        if let Some(p) = prefer_node {
            if let Some(i) = ids.iter().position(|&x| x == p) {
                ids.remove(i);
                ids.insert(0, p);
            }
        }
        ids
    }

    /// Ordered dial list: global prefer, then same-region, then others by id.
    fn dial_order(&self) -> Vec<u64> {
        self.dial_order_for(None)
    }

    /// First dial candidate after region preference (tests / observability).
    #[must_use]
    pub fn first_dial_id(&self) -> Option<u64> {
        self.dial_order().into_iter().next()
    }

    /// Learn / refresh **all** range leaders from any peer that answers status.
    ///
    /// If [`Self::set_active_range`] is set, also pins global prefer to that range's leader.
    pub fn warm_leaders(&mut self) {
        self.refresh_leaders_from_status();
        if let Some(rid) = self.active_range {
            if let Some(lid) = self.leaders.get(&rid).copied() {
                self.prefer = Some(lid);
            }
        }
    }

    /// Record a range→leader mapping and update global prefer.
    fn note_leader(&mut self, range_id: Option<u64>, leader: u64) {
        if let Some(rid) = range_id {
            self.leaders.insert(rid, leader);
        }
        self.prefer = Some(leader);
    }

    /// Refresh per-range leaders from any peer that answers status.
    fn refresh_leaders_from_status(&mut self) {
        let order: Vec<u64> = self.peers.keys().copied().collect();
        for id in order {
            let Some(addr) = self.peers.get(&id) else {
                continue;
            };
            if let Ok(st) = client_status(addr) {
                let map = leaders_from_status(&st);
                if pedradb_core::write_admission_kernel::batch_is_empty(map.len() as u64) {
                    continue;
                }
                self.leaders.extend(map);
                if self.prefer.is_none() {
                    self.prefer = self.leaders.values().next().copied();
                }
                return;
            }
        }
    }

    /// Apply NotLeader / StaleLeader routing to caches; returns updated range hint.
    fn on_not_leader(
        &mut self,
        range_id: Option<u64>,
        leader: Option<u64>,
        range_hint: Option<u64>,
    ) -> Option<u64> {
        let hint = range_id.or(range_hint);
        if let Some(lid) = leader {
            self.note_leader(hint, lid);
        } else {
            self.refresh_leaders_from_status();
        }
        hint
    }

    /// Put with NotLeader / unavailable retries.
    ///
    /// # Errors
    /// Exhausted attempts or non-retryable error.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut attempts = 0u32;
        let mut last = StoreError::Msg("no peers".into());
        let mut range_hint: Option<u64> = self.active_range;

        while attempts < self.max_attempts && Instant::now() < deadline {
            attempts += 1;
            let order = self.dial_order_for(range_hint);
            for id in order {
                let Some(addr) = self.peers.get(&id).cloned() else {
                    continue;
                };
                match client_put(&addr, key, value) {
                    Ok(()) => {
                        self.prefer = Some(id);
                        if let Some(rid) = range_hint {
                            self.leaders.insert(rid, id);
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        // TCP bodies are often StoreError Display strings.
                        last = e;
                        let class = match &last {
                            StoreError::Msg(m) => classify_message(m),
                            other => classify(other),
                        };
                        match class {
                            ClientClass::NotLeader { range_id, leader }
                            | ClientClass::StaleLeader {
                                range_id,
                                live_leader: leader,
                            } => {
                                range_hint = self.on_not_leader(range_id, leader, range_hint);
                                // Reorder next attempt toward the hinted leader (break inner).
                                break;
                            }
                            ClientClass::Unavailable(_) | ClientClass::NotCommitted { .. } => {
                                continue;
                            }
                            ClientClass::Conflict | ClientClass::LimitRejected { .. } => {
                                return Err(last);
                            }
                            ClientClass::Other(_) => continue,
                        }
                    }
                }
            }
            // brief pause before re-probing
            std::thread::sleep(Duration::from_millis(20));
            self.refresh_leaders_from_status();
            if let Some(rid) = range_hint.or(self.active_range) {
                if let Some(lid) = self.leaders.get(&rid).copied() {
                    self.prefer = Some(lid);
                }
            }
        }
        Err(last)
    }

    /// Begin a buffered TX (no network until [`Self::commit_tx`]).
    #[must_use]
    pub fn begin_tx(&self) -> PendingTx {
        PendingTx::new()
    }

    /// Multi-key put via TCP `PutBatch` → server `put_many` (RFC-0025 P1.3).
    ///
    /// One RTT for the batch (range-grouped on server). Prefer this over N×[`Self::put`].
    ///
    /// # Errors
    /// Exhausted attempts, Conflict, limits, or non-retryable error.
    pub fn put_batch(&mut self, pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
        if pedradb_core::write_admission_kernel::batch_is_empty(pairs.len() as u64) {
            return Ok(());
        }
        validate_tx_pairs(pairs)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut attempts = 0u32;
        let mut last = StoreError::Msg("no peers".into());
        let mut range_hint: Option<u64> = self.active_range;

        while attempts < self.max_attempts && Instant::now() < deadline {
            attempts += 1;
            let order = self.dial_order_for(range_hint);
            for id in order {
                let Some(addr) = self.peers.get(&id).cloned() else {
                    continue;
                };
                match client_put_batch(&addr, pairs) {
                    Ok(()) => {
                        self.prefer = Some(id);
                        if let Some(rid) = range_hint {
                            self.leaders.insert(rid, id);
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        last = e;
                        let class = match &last {
                            StoreError::Msg(m) => classify_message(m),
                            other => classify(other),
                        };
                        match class {
                            ClientClass::NotLeader { range_id, leader }
                            | ClientClass::StaleLeader {
                                range_id,
                                live_leader: leader,
                            } => {
                                range_hint = self.on_not_leader(range_id, leader, range_hint);
                                break;
                            }
                            ClientClass::Unavailable(_) | ClientClass::NotCommitted { .. } => {
                                continue;
                            }
                            ClientClass::Conflict | ClientClass::LimitRejected { .. } => {
                                return Err(last);
                            }
                            ClientClass::Other(_) => continue,
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
            self.refresh_leaders_from_status();
            if let Some(rid) = range_hint.or(self.active_range) {
                if let Some(lid) = self.leaders.get(&rid).copied() {
                    self.prefer = Some(lid);
                }
            }
        }
        Err(last)
    }

    /// Commit staged pairs with NotLeader routing (TCP `CommitTx`).
    ///
    /// # Errors
    /// Exhausted attempts, Conflict, limits, or non-retryable error.
    pub fn commit_tx(&mut self, pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<u64> {
        validate_tx_pairs(pairs)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut attempts = 0u32;
        let mut last = StoreError::Msg("no peers".into());
        let mut range_hint: Option<u64> = self.active_range;

        while attempts < self.max_attempts && Instant::now() < deadline {
            attempts += 1;
            let order = self.dial_order_for(range_hint);
            for id in order {
                let Some(addr) = self.peers.get(&id).cloned() else {
                    continue;
                };
                match client_commit_tx(&addr, pairs) {
                    Ok(tid) => {
                        self.prefer = Some(id);
                        if let Some(rid) = range_hint {
                            self.leaders.insert(rid, id);
                        }
                        return Ok(tid);
                    }
                    Err(e) => {
                        last = e;
                        match classify(&last) {
                            ClientClass::NotLeader { range_id, leader }
                            | ClientClass::StaleLeader {
                                range_id,
                                live_leader: leader,
                            } => {
                                range_hint = self.on_not_leader(range_id, leader, range_hint);
                                break;
                            }
                            ClientClass::Unavailable(_) | ClientClass::NotCommitted { .. } => {
                                continue;
                            }
                            ClientClass::Conflict | ClientClass::LimitRejected { .. } => {
                                return Err(last);
                            }
                            ClientClass::Other(_) => continue,
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
            self.refresh_leaders_from_status();
            if let Some(rid) = range_hint.or(self.active_range) {
                if let Some(lid) = self.leaders.get(&rid).copied() {
                    self.prefer = Some(lid);
                }
            }
        }
        Err(last)
    }

    /// Commit a [`PendingTx`] over TCP.
    ///
    /// # Errors
    /// See [`Self::commit_tx`].
    pub fn commit_pending(&mut self, tx: PendingTx) -> Result<u64> {
        let pairs = tx.pairs();
        self.commit_tx(&pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_not_leader_display() {
        let e = StoreError::NotLeader {
            range_id: 1,
            leader: Some(2),
        };
        assert_eq!(
            classify(&e),
            ClientClass::NotLeader {
                range_id: Some(1),
                leader: Some(2),
            }
        );
        let msg = e.to_string();
        match classify_message(&msg) {
            ClientClass::NotLeader {
                leader: Some(2), ..
            } => {}
            other => panic!("unexpected {other:?} from {msg}"),
        }
    }

    #[test]
    fn leader_from_status_text() {
        assert_eq!(
            leader_from_status("local=3 members=[1, 2, 3] r1:leader=1"),
            Some(1)
        );
        assert_eq!(leader_from_status("r1:leader=-"), None);
    }

    #[test]
    fn leaders_from_status_multi_range() {
        let st = "local=3 members=[1, 2, 3] r1:leader=1 r2:leader=3 r3:leader=- r4:leader=2";
        let m = leaders_from_status(st);
        assert_eq!(m.get(&1), Some(&1));
        assert_eq!(m.get(&2), Some(&3));
        assert_eq!(m.get(&4), Some(&2));
        assert!(!m.contains_key(&3));
        // lowest range id with a leader
        assert_eq!(leader_from_status(st), Some(1));
        assert_eq!(
            leader_from_status("r3:leader=3 r1:leader=2 r2:leader=-"),
            Some(2)
        );
    }

    #[test]
    fn dial_order_prefers_range_leader() {
        let mut c =
            TcpClusterClient::new([(1, "a:1".into()), (2, "b:2".into()), (3, "c:3".into())]);
        c.prefer = Some(1);
        c.leaders.insert(4, 3);
        let order = c.dial_order_for(Some(4));
        assert_eq!(order.first().copied(), Some(3));
        let order_global = c.dial_order_for(None);
        assert_eq!(order_global.first().copied(), Some(1));
    }

    #[test]
    fn classify_unavailable() {
        assert!(matches!(
            classify_message("tcp connect 10.0.0.1:9701: connection timed out"),
            ClientClass::Unavailable(_)
        ));
    }

    #[test]
    fn classify_limits() {
        let e = StoreError::ValueTooLarge {
            size: 200_000,
            limit: MAX_VALUE_BYTES,
        };
        assert!(matches!(
            classify(&e),
            ClientClass::LimitRejected { kind: "value", .. }
        ));
        let e = StoreError::TransactionTooLarge {
            size: 99,
            limit: MAX_TX_KEYS,
        };
        assert!(matches!(
            classify(&e),
            ClientClass::LimitRejected {
                kind: "transaction",
                ..
            }
        ));
    }

    #[test]
    fn pending_tx_set_rejects_huge_value() {
        let mut tx = PendingTx::new();
        let big = vec![0u8; MAX_VALUE_BYTES + 1];
        let err = tx.set(b"k", &big).unwrap_err();
        assert!(matches!(err, StoreError::ValueTooLarge { .. }));
    }
}
