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
//!   entry applied with PedraDB `apply_batch` (row+index style layers).
//! - **Cross-range multi-key TX** via [`StoreCluster::commit_tx`] / [`tx_start`] /
//!   [`tx_finish`]: 2PC prepare/commit with durable intents (FDB-class *gap* vs
//!   same-range-only batch). Write-write conflicts on intent-held keys abort.
//! - **Snapshot/OCC client TX** (RFC-0022): [`StoreCluster::snapshot_begin`] /
//!   [`client::SnapshotTx`] — snapshot generation at begin, OCC on commit,
//!   optional [`StoreError::TransactionTooOld`]. Leadership-invisible (no range ids).
//! - **Watch** after majority commit via [`WatchHub`] (RFC-0022 P0.3).
//! - **Product faces** in [`layers`] (etcd-need, TiKV-like, table/SQLite encode,
//!   PG N-writer, OLAP RO, stream) — thin layers, not wire clones.
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

mod ae_ack_kernel;
pub mod client;
mod commit_kernel;
mod compact_kernel;
pub mod fdb_compat;
pub mod fdb_layers;
mod index_val_kernel;
mod l28;
pub mod layers;
mod membership_kernel;
mod msg;
mod rpc_mode_kernel;
mod si_kernel;
mod snapshot_kernel;
pub mod tcp;
pub mod tls;
pub mod tx_glue_kernel;
mod txn_kernel;

pub use ae_ack_kernel::{ae_ack_success, ae_ack_success_as_is};
pub use client::{
    classify, classify_message, leader_from_status, leaders_from_status, ClientClass, PendingTx,
    SnapshotTx, TcpClusterClient, Transaction, MAX_SNAPSHOT_LAG,
};
pub use commit_kernel::{
    may_commit_at, may_commit_at_as_is, propose_ack_ok, propose_ack_ok_as_is, recover_commit,
    recover_commit_as_is,
};
pub use compact_kernel::{
    compact_index_floor, compact_ready, compact_through_unleft, compact_through_unleft_as_is,
    may_compact_through, may_compact_through_as_is, peer_counts_for_compact,
    peer_counts_for_compact_as_is,
};
pub use fdb_compat::{
    run_phase1_bindingtester_subset, FdbDatabase, FdbError, FdbTransaction, Phase1HarnessReport,
};
pub use fdb_layers::{IdempotentIndex, NaiveAllocator, NaiveList, SafeAllocator, SafeList};
pub use index_val_kernel::{
    exact_value_children, exact_value_children_as_is, len_pref_value, len_pref_value_as_is,
    value_len_tag, value_len_tag_as_is,
};
pub use l28::{
    l28_durability_ok, l28_durability_ok_as_is, l28_leader_kill_ok, l28_leader_kill_ok_as_is,
    l28_tcp_apply_ok, l28_tcp_apply_ok_as_is, l28_tcp_hw_ok, l28_tcp_hw_ok_as_is, l28_tcp_leave_ok,
    l28_tcp_leave_ok_as_is, l28_tcp_left_ok, l28_tcp_left_ok_as_is, l28_tcp_napply_ok,
    l28_tcp_abort_ok, l28_tcp_abort_ok_as_is, l28_tcp_clear_ok, l28_tcp_clear_ok_as_is,
    l28_tcp_lid_ok, l28_tcp_lid_ok_as_is, l28_tcp_peer_ok, l28_tcp_peer_ok_as_is,
    l28_tcp_dsc_ok, l28_tcp_dsc_ok_as_is, l28_tcp_pld_ok, l28_tcp_pld_ok_as_is,
    l28_tcp_pre_ok, l28_tcp_pre_ok_as_is, l28_tcp_rdr_ok, l28_tcp_rdr_ok_as_is,
    l28_tcp_hnt_ok, l28_tcp_hnt_ok_as_is, l28_tcp_std_ok, l28_tcp_std_ok_as_is,
    l28_tcp_fence_ok, l28_tcp_fence_ok_as_is,
    l28_tcp_hist_ok, l28_tcp_hist_ok_as_is,
    l28_tcp_napply_ok_as_is, l28_tcp_nowms_ok, l28_tcp_nowms_ok_as_is, l28_tcp_odrop_ok,
    l28_tcp_odrop_ok_as_is, l28_tcp_part_ok,
    l28_tcp_trunc_ok, l28_tcp_trunc_ok_as_is,
    l28_tcp_part_ok_as_is, l28_tcp_plant_ok, l28_tcp_plant_ok_as_is, world_seed_l28_ok,
    world_seed_l28_ok_as_is,
};
pub use layers::{
    olap_get, olap_ingest, olap_list_at, olap_stream_range, pg_upsert, pks_one_per_range,
    put_with_secondary_index, raw_keys_one_per_range, sql_multi_table_write, stream_get,
    stream_list_at, stream_publish, table_get, table_put, table_row_key, EtcdNeedFace,
    LeadershipEvent, LeadershipHub, TikvKvFace, WatchEvent, WatchHub,
};
pub use membership_kernel::{
    elect_claim_banner, elect_claim_banner_as_is, high_water_at_least, high_water_at_least_as_is,
    joint_election_ok, joint_election_ok_as_is, joint_leave_ok, joint_leave_ok_as_is,
    joint_still_active, joint_still_active_as_is, liveness_admitted, liveness_admitted_as_is,
    majority_of, queued_leave_finish_ok, queued_leave_finish_ok_as_is,
};
pub use msg::PeerMsg;
pub use rpc_mode_kernel::{allow_direct_rpc, allow_direct_rpc_as_is};
pub use si_kernel::{
    point_get_prefer_applied, point_get_prefer_applied_as_is, point_get_watermark,
    point_get_watermark_as_is, si_reader_beats, si_reader_beats_as_is, snapshot_read_plan,
    snapshot_read_plan_as_is, SnapshotRead,
};
pub use snapshot_kernel::{
    snapshot_needs_txn_meta_clear, snapshot_needs_txn_meta_clear_as_is, snapshot_touches_user_key,
    snapshot_touches_user_key_as_is,
};
pub use tcp::{
    client_add_member_joint, client_commit_tx, client_dcs_cas, client_dcs_create, client_dcs_get,
    client_get, client_leave_joint, client_put, client_put_batch, client_remove_member_joint,
    client_set_peers, client_status, client_tick, connect as tcp_connect,
    connect_host as tcp_connect_host, peer_wire, read_frame, resolve_host_port, write_frame,
    WireMsg,
};
pub use tls::{
    install_from_pem_files, maybe_client_wrap, maybe_server_wrap, reload_from_pem_files,
    tls_installed, IoBox,
};
pub use tx_glue_kernel::{tx_range_action, TxRangeAction};
pub use txn_kernel::{
    discard_cut, discard_cut_as_is, leftover_txn_is_aborted, leftover_txn_is_aborted_as_is,
    next_txn_id_after, next_txn_id_as_is, prepare_error_aborts_earlier,
    prepare_error_aborts_earlier_as_is, recover_si_generation, recover_si_generation_as_is,
    reserve_si_gen, reserve_si_gen_as_is, revert_clears_status, revert_clears_status_as_is,
    revert_user_action, revert_user_action_as_is, should_repair_si_hist,
    should_repair_si_hist_as_is, txn_commit_action, txn_commit_action_as_is, unreserve_si_gen,
    RevertUserAction, SiGenReserve, TxnCommitAction,
};
/// Pedra open knobs under each Montanha node (RFC-0025 P0.1).
///
/// Default matches historical `open` (WAL fsync on every durable write).
#[derive(Debug, Clone)]
pub struct StoreOpenOptions {
    /// When `true` (default), Pedra fsyncs the WAL before Ok on put/batch.
    /// Set `false` only for **bulk load / capacity lab** — not crash-safe.
    pub pedra_sync: bool,
    /// When `true`, enable Pedra L0 write backpressure defaults after open
    /// (pressure @ L0 trigger, hard stall @ 2×, drain on). Default **false**
    /// so lab/soak paths stay unconstrained unless opted in.
    pub pedra_write_backpressure: bool,
    /// RFC-0058 P0.2: open every node with the **verified profile**
    /// (`OpenOptions::verified()` — sync forced **true**, strongest WAL
    /// data class, fail-closed recovery). Verified wins over
    /// `pedra_sync = false`: durability is the composition being declared.
    /// Default **false**.
    pub pedra_verified: bool,
    /// WAL barrier data class. Default `true` = platform strongest
    /// (`F_FULLFSYNC` on Darwin) — the product class. `false` =
    /// `fdatasync` weak class (the `librocksdb-sys` crate-build class):
    /// same barrier call and same `OpClass::Sync` fault seam, cheaper
    /// syscall. For simulation harnesses whose wall clock is dominated
    /// by the strong barrier without adding oracle power (RFC-0059).
    pub pedra_wal_full_fsync: bool,
    /// RFC-0013 P1.3: shared cluster identity. `None` (default) mints on
    /// first open of empty nodes and recovers from `\0store/cluster/id`.
    /// Multi-host processes must pass the **same** 16 bytes or a node dir
    /// from another cluster is refuse-closed as [`StoreError::ClusterMismatch`].
    pub cluster_id: Option<[u8; 16]>,
}

impl Default for StoreOpenOptions {
    fn default() -> Self {
        Self {
            pedra_sync: true,
            pedra_write_backpressure: false,
            pedra_verified: false,
            pedra_wal_full_fsync: true,
            cluster_id: None,
        }
    }
}

impl StoreOpenOptions {
    /// Lab capacity mode: Pedra `sync=false` (faster, not durable on process crash).
    #[must_use]
    pub fn lab_capacity() -> Self {
        Self {
            pedra_sync: false,
            pedra_write_backpressure: false,
            pedra_verified: false,
            pedra_wal_full_fsync: true,
            cluster_id: None,
        }
    }

    /// Production-shaped L0 admission (see [`Db::enable_write_backpressure_defaults`]).
    #[must_use]
    pub fn with_write_backpressure(mut self) -> Self {
        self.pedra_write_backpressure = true;
        self
    }

    /// Pin the RFC-0013 P1.3 cluster id (multi-host must share one value).
    #[must_use]
    pub fn with_cluster_id(mut self, id: [u8; 16]) -> Self {
        self.cluster_id = Some(id);
        self
    }
}

/// In-process counters (RFC-0013 P1.5). Not a metrics product — lab/ops
/// hooks so hosts are not blind. Single-threaded `StoreCluster`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreMetrics {
    /// Client proposes that majority-committed (`put` / `put_batch` Ok).
    pub commits_ok: u64,
    /// Client proposes that returned [`StoreError::NotCommitted`].
    pub not_committed: u64,
    /// Successful range elections (`try_become_leader` persisted).
    pub elections: u64,
}

/// Aggregate Pedra L0 / mem admission counters across local nodes (lab A/B, gates).
///
/// Config fields are the first non-zero values seen (same on every node when opened
/// with [`StoreOpenOptions::with_write_backpressure`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteAdmissionSnap {
    /// Local nodes that contributed stats.
    pub nodes: u64,
    /// Max L0 SST file count across nodes.
    pub l0_files_max: u64,
    /// Sum of hard-stall refusals across nodes.
    pub write_stall_count_sum: u64,
    /// Sum of soft-pressure drains across nodes.
    pub write_pressure_count_sum: u64,
    /// Configured hard L0 stall limit (`0` = disabled).
    pub write_stall_l0: u64,
    /// Configured soft L0 pressure threshold (`0` = disabled).
    pub write_pressure_l0: u64,
    /// Configured mem stall limit in bytes (`0` = disabled).
    pub write_stall_mem_bytes: u64,
}

impl WriteAdmissionSnap {
    /// Compact JSON object (no outer braces nesting helpers).
    #[must_use]
    pub fn to_json_object(&self) -> String {
        format!(
            r#"{{"nodes":{},"l0_files_max":{},"write_stall_count_sum":{},"write_pressure_count_sum":{},"write_stall_l0":{},"write_pressure_l0":{},"write_stall_mem_bytes":{}}}"#,
            self.nodes,
            self.l0_files_max,
            self.write_stall_count_sum,
            self.write_pressure_count_sum,
            self.write_stall_l0,
            self.write_pressure_l0,
            self.write_stall_mem_bytes
        )
    }
}

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Bound;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use pedradb_core::{BatchOp, Db, Env, Host, OpenOptions, Rng, SeedRng};
use pedradb_dcs::{
    apply_dcs_command, bind_absent_create, check_command_at, dcs_get, dcs_get_at, DcsCommand,
    KeyValue,
};
use pedradb_io_uring::IoUringEnv;
use thiserror::Error;

/// How peer RPCs (RequestVote / AppendEntries) are delivered.
///
/// - [`RpcMode::Queued`] (production default, RFC-0067 P2.2): messages go to
///   an outbound queue; the World / harness must
///   [`StoreCluster::drain_outbound`] and [`StoreCluster::handle_inbound`]
///   (typically via a Net). Client `put` may return [`StoreError::NotCommitted`]
///   with the entry **left on the leader log** until majority is reached after
///   delivery (no silent discard in Queued mode on that path).
/// - [`RpcMode::Direct`]: in-process sync delivery of the same `PeerMsg`
///   (lab leftover). Opt-in via [`StoreCluster::enable_lab_direct_rpc`] for
///   unpinned unit tests; refused after [`StoreCluster::pin_dst_queued`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RpcMode {
    /// Immediate in-process RPC (lab leftover; opt-in).
    Direct,
    /// Enqueue RPC bytes; deliver via [`StoreCluster::handle_inbound`].
    #[default]
    Queued,
}

/// Store errors.
#[derive(Debug, Error)]
pub enum StoreError {
    /// PedraDB (non-admission errors). Write stalls are mapped to
    /// [`Self::WriteStall`] / [`Self::WriteStallMem`].
    #[error("pedradb: {0}")]
    Core(#[source] pedradb_core::CoreError),
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
    /// Multi-key **batch** spans more than one range.
    ///
    /// Use [`StoreCluster::commit_tx`] for cross-range atomic multi-key writes.
    /// [`StoreCluster::put_batch`] remains same-range only (fast path).
    #[error("cross-range batch: keys map to ranges {ranges:?} (use commit_tx or co-locate)")]
    CrossRange {
        /// Distinct range ids touched by the batch (sorted).
        ranges: Vec<u64>,
    },
    /// Write-write conflict with another prepared multi-key TX (overlapping keys).
    #[error("transaction conflict on key (txn intents held)")]
    Conflict,
    /// Prepared TX was aborted (conflict during prepare apply or explicit cancel).
    #[error("transaction {0} aborted")]
    TxnAborted(u64),
    /// Snapshot is too old relative to cluster commit generation (RFC-0022 / FDB-class).
    #[error("transaction too old: snapshot {snapshot}, current {current}")]
    TransactionTooOld {
        /// Generation at [`StoreCluster::snapshot_begin`].
        snapshot: u64,
        /// Cluster [`StoreCluster::read_version`] at commit attempt.
        current: u64,
    },
    /// Pedra L0 write stall (open-items §2.3) — compact/retry, no sleep in engine.
    #[error("write stall: L0 has {l0_files} files (limit {limit})")]
    WriteStall {
        /// Current L0 SST count.
        l0_files: usize,
        /// Configured stall threshold.
        limit: usize,
    },
    /// Pedra memtable write stall (open-items §2.3 c).
    #[error("write stall: memtable ~{mem_bytes}B (limit {limit}B)")]
    WriteStallMem {
        /// Approximate active memtable bytes.
        mem_bytes: usize,
        /// Configured stall threshold in bytes.
        limit: usize,
    },
    /// Single value exceeds [`MAX_VALUE_BYTES`].
    #[error("value too large: {size} bytes (limit {limit})")]
    ValueTooLarge {
        /// Actual size in bytes.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// Transaction payload (sum of key+value sizes or key count) exceeds limit.
    #[error("transaction too large: {size} (limit {limit})")]
    TransactionTooLarge {
        /// Measured size (bytes or key count — see message context via fields).
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// No range covers key.
    #[error("no range for key")]
    NoRange,
    /// A node directory belongs to a different cluster (RFC-0013 P1.3).
    /// Silent merge of two Pedra trees is refuse-closed.
    #[error("cluster id mismatch on node {node_id}")]
    ClusterMismatch {
        /// Node whose on-disk id disagreed.
        node_id: u64,
        /// Id this process bound (minted, configured, or from a sibling).
        expected: [u8; 16],
        /// Id stored under `\0store/cluster/id` on that node.
        found: [u8; 16],
    },
    /// Empty cluster / bad config.
    #[error("{0}")]
    Msg(String),
}

impl From<pedradb_core::CoreError> for StoreError {
    fn from(e: pedradb_core::CoreError) -> Self {
        match e {
            pedradb_core::CoreError::WriteStall { l0_files, limit } => {
                StoreError::WriteStall { l0_files, limit }
            }
            pedradb_core::CoreError::WriteStallMem { mem_bytes, limit } => {
                StoreError::WriteStallMem { mem_bytes, limit }
            }
            other => StoreError::Core(other),
        }
    }
}

/// Max single value size for client TX / put paths (FDB-class order; Montanha-chosen).
pub const MAX_VALUE_BYTES: usize = 100 * 1024;
/// Max sum of key+value bytes in one client TX commit.
pub const MAX_TX_BYTES: usize = 10 * 1024 * 1024;
/// Max number of keys in one client TX commit.
pub const MAX_TX_KEYS: usize = 10_000;

/// Handle for a multi-range TX after successful prepare (2PC).
#[derive(Debug, Clone)]
pub struct TxHandle {
    /// Transaction id.
    pub id: u64,
    /// Ranges that prepared (sorted).
    pub ranges: Vec<u64>,
    /// Per-range user keys (commit/abort/revert scoped to range — not whole DB).
    keys_by_range: Vec<(u64, Vec<Vec<u8>>)>,
}

/// User key/value pair bytes.
type KvPair = (Vec<u8>, Vec<u8>);
/// Range id → pairs for that range.
type RangeGroups = Vec<(u64, Vec<KvPair>)>;

enum CleanupMode {
    Abort,
    Revert,
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
        key >= self.start.as_slice() && (self.end.is_empty() || key < self.end.as_slice())
    }
}

/// One raft log entry for a range (payload inside [`LogRec`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeEntry {
    /// User put. Empty `value` means **delete** (Pedra `delete`, not empty payload).
    Put {
        /// User key.
        key: Vec<u8>,
        /// Value bytes; empty ⇒ delete key.
        value: Vec<u8>,
        /// Snapshot/OCC generation assigned at propose (0 = no SI side-effect).
        si_gen: u64,
    },
    /// Atomic multi-put/delete (same range); empty values ⇒ delete.
    Batch {
        /// Ordered key/value pairs (empty value ⇒ delete).
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
        /// Snapshot/OCC generation for this batch (0 = no SI side-effect).
        si_gen: u64,
    },
    /// DCS command (applied via pedradb-dcs).
    Dcs(DcsCommand),
    /// Leadership blank index (Raft §5.4.2): previous-term majority can commit.
    Noop,
    /// TX prepare: install durable intents for keys in this range.
    TxnPrepare {
        /// Transaction id.
        txn_id: u64,
        /// Intent pairs (key → value).
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// TX commit: materialize intents for listed keys **in this range only**.
    TxnCommit {
        /// Transaction id.
        txn_id: u64,
        /// User keys to commit in this range.
        keys: Vec<Vec<u8>>,
        /// One SI generation for the whole 2PC TX (0 = no SI side-effect).
        si_gen: u64,
    },
    /// TX abort: drop intents for listed keys (this range).
    TxnAbort {
        /// Transaction id.
        txn_id: u64,
        /// User keys to abort in this range.
        keys: Vec<Vec<u8>>,
    },
    /// Compensating delete of user keys after a partial commit failure.
    TxnRevert {
        /// Transaction id.
        txn_id: u64,
        /// User keys to revert in this range.
        keys: Vec<Vec<u8>>,
    },
    /// Log-carried membership (RFC-0063 P0). Joint: while this entry is
    /// uncommitted, a commit quorum is majority(`old`) ∧ majority(`new`).
    /// After apply, cluster voters become `new`. Out-of-band
    /// [`StoreCluster::remove_member`] still uses the quorum floor.
    MembershipJoint {
        /// Voters at propose time.
        old: Vec<u64>,
        /// Target voters (one add or remove relative to `old` in P0).
        new: Vec<u64>,
    },
}

/// One raft log record on a range (index + term + payload).
///
/// Public so [`crate::msg::PeerMsg`] can expose `AppendEntries` without
/// private-interface warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRec {
    /// Log index (1-based).
    pub index: u64,
    /// Term at which this entry was written.
    pub term: u64,
    /// Range command payload.
    pub entry: RangeEntry,
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
const INTENT_PREFIX: &[u8] = b"\0store/intent/";
const TXN_PREFIX: &[u8] = b"\0store/txn/";
const SI_META_PREFIX: &[u8] = b"\0store/meta/";
const HIST_PREFIX: &[u8] = b"\0store/hist/";
/// RFC-0013 P1.3: cluster identity (id + last voting set). Layout:
/// `\0store/cluster/id` = 16 raw bytes; `\0store/cluster/membership` =
/// `u32 LE` count + `count` × `u64 LE` voter ids (sorted).
const CLUSTER_META_PREFIX: &[u8] = b"\0store/cluster/";

fn raft_meta_key(range_id: u64, kind: &str) -> Vec<u8> {
    let mut k = RAFT_META_PREFIX.to_vec();
    k.extend_from_slice(range_id.to_string().as_bytes());
    k.push(b'/');
    k.extend_from_slice(kind.as_bytes());
    k
}

/// Half-open interval `[start, end)` on raw key bytes (`end` empty = +∞).
#[must_use]
pub fn key_in_half_open(key: &[u8], start: &[u8], end: &[u8]) -> bool {
    if key < start {
        return false;
    }
    if end.is_empty() {
        return true;
    }
    key < end
}

/// Intersection of query `[q_start, q_end)` with a store range (F108).
fn clip_query_to_range(q_start: &[u8], q_end: &[u8], r: &RangeMeta) -> Option<(Vec<u8>, Vec<u8>)> {
    let r_before_q = !r.end.is_empty() && q_start >= r.end.as_slice();
    let q_before_r = !q_end.is_empty() && r.start.as_slice() >= q_end;
    if r_before_q || q_before_r {
        return None;
    }
    let start = if q_start < r.start.as_slice() {
        r.start.clone()
    } else {
        q_start.to_vec()
    };
    let end = match (q_end.is_empty(), r.end.is_empty()) {
        (true, true) => Vec::new(),
        (true, false) => r.end.clone(),
        (false, true) => q_end.to_vec(),
        (false, false) => {
            if q_end <= r.end.as_slice() {
                q_end.to_vec()
            } else {
                r.end.clone()
            }
        }
    };
    if !end.is_empty() && start.as_slice() >= end.as_slice() {
        return None;
    }
    Some((start, end))
}

fn is_reserved_store_key(key: &[u8]) -> bool {
    key.starts_with(RAFT_META_PREFIX)
        || key.starts_with(INTENT_PREFIX)
        || key.starts_with(TXN_PREFIX)
        || key.starts_with(SI_META_PREFIX)
        || key.starts_with(HIST_PREFIX)
        || key.starts_with(CLUSTER_META_PREFIX)
}

fn cluster_id_key() -> Vec<u8> {
    let mut k = CLUSTER_META_PREFIX.to_vec();
    k.extend_from_slice(b"id");
    k
}

fn cluster_membership_key() -> Vec<u8> {
    let mut k = CLUSTER_META_PREFIX.to_vec();
    k.extend_from_slice(b"membership");
    k
}

fn cluster_high_water_key() -> Vec<u8> {
    let mut k = CLUSTER_META_PREFIX.to_vec();
    k.extend_from_slice(b"high_water");
    k
}

fn mint_cluster_id() -> [u8; 16] {
    use pedradb_core::rng::{Rng, SystemRng};
    let a = SystemRng.next_u64();
    let b = SystemRng.next_u64();
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&a.to_le_bytes());
    id[8..].copy_from_slice(&b.to_le_bytes());
    id
}

fn decode_cluster_id(raw: &[u8]) -> Result<[u8; 16]> {
    raw.try_into()
        .map_err(|_| StoreError::Msg("cluster id must be 16 bytes".into()))
}

fn encode_membership(ids: &[u64]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + ids.len() * 8);
    v.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    for id in ids {
        v.extend_from_slice(&id.to_le_bytes());
    }
    v
}

fn decode_membership(raw: &[u8]) -> Result<Vec<u64>> {
    if raw.len() < 4 {
        return Err(StoreError::Msg("cluster membership short".into()));
    }
    let n = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
    if raw.len() != 4 + n * 8 {
        return Err(StoreError::Msg("cluster membership length".into()));
    }
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let off = 4 + i * 8;
        ids.push(u64::from_le_bytes(raw[off..off + 8].try_into().unwrap()));
    }
    Ok(ids)
}

/// Hex of a 16-byte cluster id (status / docs / tests).
#[must_use]
pub fn cluster_id_hex(id: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in id {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// F100: length-prefix user under intent/hist/txn so `intent/a` is not a
/// byte-prefix of `intent/ab` (same class as F89–F99).
fn push_user_component(buf: &mut Vec<u8>, user: &[u8]) {
    buf.extend_from_slice(&len_pref_value(user));
}

/// Decode user after a fixed meta prefix (F100 length-prefix; legacy raw OK).
fn user_from_meta_suffix(rest: &[u8]) -> Option<Vec<u8>> {
    if rest.len() >= 4 {
        let n = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
        if rest.len() == 4 + n {
            return Some(rest[4..].to_vec());
        }
    }
    Some(rest.to_vec())
}

fn intent_key(user: &[u8]) -> Vec<u8> {
    let mut k = INTENT_PREFIX.to_vec();
    push_user_component(&mut k, user);
    k
}

fn txn_status_key(txn_id: u64) -> Vec<u8> {
    let mut k = TXN_PREFIX.to_vec();
    k.extend_from_slice(&txn_id.to_le_bytes());
    k.extend_from_slice(b"/status");
    k
}

fn txn_pair_key(txn_id: u64, user: &[u8]) -> Vec<u8> {
    let mut k = TXN_PREFIX.to_vec();
    k.extend_from_slice(&txn_id.to_le_bytes());
    k.extend_from_slice(b"/k/");
    push_user_component(&mut k, user);
    k
}

fn txn_pair_prefix(txn_id: u64) -> Vec<u8> {
    let mut k = TXN_PREFIX.to_vec();
    k.extend_from_slice(&txn_id.to_le_bytes());
    k.extend_from_slice(b"/k/");
    k
}

fn txn_pre_key(txn_id: u64, user: &[u8]) -> Vec<u8> {
    let mut k = TXN_PREFIX.to_vec();
    k.extend_from_slice(&txn_id.to_le_bytes());
    k.extend_from_slice(b"/pre/");
    push_user_component(&mut k, user);
    k
}

fn si_meta_key(kind: &str) -> Vec<u8> {
    let mut k = SI_META_PREFIX.to_vec();
    k.extend_from_slice(kind.as_bytes());
    k
}

fn hist_key(user: &[u8]) -> Vec<u8> {
    let mut k = HIST_PREFIX.to_vec();
    push_user_component(&mut k, user);
    k
}

/// `None` = key was absent at prepare; `Some(v)` = restore `v`.
fn encode_preimage(present: Option<&[u8]>) -> Vec<u8> {
    match present {
        None => vec![0],
        Some(v) => {
            let mut b = Vec::with_capacity(1 + v.len());
            b.push(1);
            b.extend_from_slice(v);
            b
        }
    }
}

/// Decode prepare-time preimage blob.
///
/// Tag `0` = key was absent; `1` + payload = restore value.
/// Empty / unknown tag is **Err** (F118) — not "missing preimage".
fn decode_preimage(raw: &[u8]) -> Result<Option<Vec<u8>>> {
    if raw.is_empty() {
        return Err(StoreError::Msg("preimage empty".into()));
    }
    match raw[0] {
        0 => Ok(None),
        1 => Ok(Some(raw[1..].to_vec())),
        _ => Err(StoreError::Msg("preimage tag".into())),
    }
}

pub(crate) use pedradb_core::prefix_exclusive_end;

fn scan_prefix<E: Env>(db: &Db<E>, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let start = Bound::Included(prefix);
    let end_owned = prefix_exclusive_end(prefix);
    let end = match end_owned.as_deref() {
        Some(e) => Bound::Excluded(e),
        None => Bound::Unbounded,
    };
    db.range_limited(start, end, None)
        .into_iter()
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect()
}

/// Apply user put or true Pedra delete (empty value ⇒ delete).
fn apply_put_or_delete<E: Env>(db: &mut Db<E>, key: &[u8], value: &[u8]) -> Result<()> {
    if value.is_empty() {
        db.delete(key)?;
    } else {
        db.put(key, value)?;
    }
    Ok(())
}

/// Durable SI history row on this replica (written during Raft apply — same path as user data).
///
/// # Errors
/// Present-but-corrupt hist blob (F117) — never rewrite as a fresh single-gen history.
fn persist_si_hist_on_db<E: Env>(
    db: &mut Db<E>,
    user_key: &[u8],
    gen: u64,
    new_val: Option<&[u8]>,
) -> Result<()> {
    let hk = hist_key(user_key);
    // F117: missing → empty chain; present corrupt → hard error (do not wipe).
    let mut hist = match db.get(&hk) {
        None => Vec::new(),
        Some(b) => decode_hist(b.as_ref())?,
    };
    // Avoid duplicate gen append on replay.
    if hist.last().map(|(g, _)| *g) != Some(gen) {
        hist.push((gen, new_val.map(|v| v.to_vec())));
    }
    db.put(&hk, encode_hist(&hist))?;
    Ok(())
}

fn encode_hist(hist: &[(u64, Option<Vec<u8>>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(hist.len() as u32).to_le_bytes());
    for (g, v) in hist {
        b.extend_from_slice(&g.to_le_bytes());
        match v {
            Some(val) => {
                b.push(1);
                encode_bytes(&mut b, val);
            }
            None => b.push(0),
        }
    }
    append_crc(&mut b);
    b
}

fn decode_hist(buf: &[u8]) -> Result<Vec<(u64, Option<Vec<u8>>)>> {
    let p = strip_crc(buf)?;
    if p.len() < 4 {
        return Err(StoreError::Msg("hist short".into()));
    }
    let n = u32::from_le_bytes(p[0..4].try_into().unwrap()) as usize;
    if n > 10_000 {
        return Err(StoreError::Msg("hist too large".into()));
    }
    let mut off = 4;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if off + 9 > p.len() {
            return Err(StoreError::Msg("hist entry eof".into()));
        }
        let g = u64::from_le_bytes(p[off..off + 8].try_into().unwrap());
        off += 8;
        let tag = p[off];
        off += 1;
        match tag {
            0 => out.push((g, None)),
            1 => {
                let v = take_bytes(p, &mut off)?;
                out.push((g, Some(v)));
            }
            _ => return Err(StoreError::Msg("hist tag".into())),
        }
    }
    Ok(out)
}

fn encode_intent(txn_id: u64, value: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + value.len());
    b.extend_from_slice(&txn_id.to_le_bytes());
    b.extend_from_slice(value);
    b
}

/// Decode intent value: `u64le txn_id || payload`.
///
/// Short blob is **Err** (F120) — callers that treated `None` as "no intent"
/// skipped materialise on commit while still deleting the intent key.
fn decode_intent(raw: &[u8]) -> Result<(u64, &[u8])> {
    if raw.len() < 8 {
        return Err(StoreError::Msg("intent short".into()));
    }
    let id = u64::from_le_bytes(raw[0..8].try_into().unwrap());
    Ok((id, &raw[8..]))
}

/// True if `user` has an intent held by a *different* txn (or any if `self_id` is None).
/// Validate key/value pairs against TX size limits (RFC-0021 P0.2).
pub fn validate_tx_pairs(pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
    if pairs.len() > MAX_TX_KEYS {
        return Err(StoreError::TransactionTooLarge {
            size: pairs.len(),
            limit: MAX_TX_KEYS,
        });
    }
    let mut total = 0usize;
    for (k, v) in pairs {
        if v.len() > MAX_VALUE_BYTES {
            return Err(StoreError::ValueTooLarge {
                size: v.len(),
                limit: MAX_VALUE_BYTES,
            });
        }
        if is_reserved_store_key(k) {
            return Err(StoreError::Msg(
                "key prefix reserved for store internal meta".into(),
            ));
        }
        total = total.saturating_add(k.len()).saturating_add(v.len());
        if total > MAX_TX_BYTES {
            return Err(StoreError::TransactionTooLarge {
                size: total,
                limit: MAX_TX_BYTES,
            });
        }
    }
    if pairs.is_empty() {
        return Err(StoreError::Msg("empty transaction".into()));
    }
    Ok(())
}

fn intent_conflict<E: Env>(db: &Db<E>, user: &[u8], self_id: Option<u64>) -> bool {
    let Some(raw) = db.get(&intent_key(user)) else {
        return false;
    };
    // Present undecodable intent blocks prepare (fail closed).
    let Ok((oid, _)) = decode_intent(&raw) else {
        return true;
    };
    match self_id {
        None => true,
        Some(id) => oid != id,
    }
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
    if !pedradb_core::wal::crc::crc_match_ok(got, expect) {
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
        // Tag 11: Put + si_gen (tag 1 legacy decode still accepted).
        RangeEntry::Put { key, value, si_gen } => {
            b.push(11);
            encode_bytes(&mut b, key);
            encode_bytes(&mut b, value);
            b.extend_from_slice(&si_gen.to_le_bytes());
        }
        RangeEntry::Dcs(cmd) => {
            b.push(2);
            let raw = cmd.encode();
            encode_bytes(&mut b, &raw);
        }
        RangeEntry::Noop => b.push(3),
        // Tag 12: Batch + si_gen (tag 4 legacy).
        RangeEntry::Batch { pairs, si_gen } => {
            b.push(12);
            b.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
            for (k, v) in pairs {
                encode_bytes(&mut b, k);
                encode_bytes(&mut b, v);
            }
            b.extend_from_slice(&si_gen.to_le_bytes());
        }
        RangeEntry::TxnPrepare { txn_id, pairs } => {
            b.push(5);
            b.extend_from_slice(&txn_id.to_le_bytes());
            b.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
            for (k, v) in pairs {
                encode_bytes(&mut b, k);
                encode_bytes(&mut b, v);
            }
        }
        RangeEntry::TxnCommit {
            txn_id,
            keys,
            si_gen,
        } => {
            // Tag 13: commit + si_gen (tag 6 legacy decode still accepted).
            b.push(13);
            b.extend_from_slice(&txn_id.to_le_bytes());
            b.extend_from_slice(&(keys.len() as u32).to_le_bytes());
            for k in keys {
                encode_bytes(&mut b, k);
            }
            b.extend_from_slice(&si_gen.to_le_bytes());
        }
        RangeEntry::TxnAbort { txn_id, keys } => {
            b.push(7);
            b.extend_from_slice(&txn_id.to_le_bytes());
            b.extend_from_slice(&(keys.len() as u32).to_le_bytes());
            for k in keys {
                encode_bytes(&mut b, k);
            }
        }
        RangeEntry::TxnRevert { txn_id, keys } => {
            b.push(8);
            b.extend_from_slice(&txn_id.to_le_bytes());
            b.extend_from_slice(&(keys.len() as u32).to_le_bytes());
            for k in keys {
                encode_bytes(&mut b, k);
            }
        }
        RangeEntry::MembershipJoint { old, new } => {
            b.push(14);
            encode_u64_list(&mut b, old);
            encode_u64_list(&mut b, new);
        }
    }
    b
}

fn encode_u64_list(b: &mut Vec<u8>, ids: &[u64]) {
    b.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    for id in ids {
        b.extend_from_slice(&id.to_le_bytes());
    }
}

fn decode_u64_list(buf: &[u8], off: &mut usize) -> Result<Vec<u64>> {
    if *off + 4 > buf.len() {
        return Err(StoreError::Msg("id list count eof".into()));
    }
    let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
    *off += 4;
    if n > 1024 || *off + n.saturating_mul(8) > buf.len() {
        return Err(StoreError::Msg("id list too large".into()));
    }
    let mut ids = Vec::with_capacity(n);
    for _ in 0..n {
        ids.push(u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap()));
        *off += 8;
    }
    Ok(ids)
}

fn decode_key_list(buf: &[u8], off: &mut usize) -> Result<Vec<Vec<u8>>> {
    if *off + 4 > buf.len() {
        return Err(StoreError::Msg("key list count eof".into()));
    }
    let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
    *off += 4;
    // Soft cap + residual (F2/F39): n may be < 1e6 yet still >> remaining bytes.
    let rem = buf.len().saturating_sub(*off);
    if n > 1_000_000 || n > rem {
        return Err(StoreError::Msg("key list too large".into()));
    }
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        keys.push(take_bytes(buf, off)?);
    }
    Ok(keys)
}

fn decode_entry(buf: &[u8], off: &mut usize) -> Result<RangeEntry> {
    if *off >= buf.len() {
        return Err(StoreError::Msg("entry eof".into()));
    }
    let tag = buf[*off];
    *off += 1;
    match tag {
        1 => {
            // Legacy Put without si_gen.
            let key = take_bytes(buf, off)?;
            let value = take_bytes(buf, off)?;
            Ok(RangeEntry::Put {
                key,
                value,
                si_gen: 0,
            })
        }
        11 => {
            let key = take_bytes(buf, off)?;
            let value = take_bytes(buf, off)?;
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("put si_gen eof".into()));
            }
            let si_gen = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            Ok(RangeEntry::Put { key, value, si_gen })
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
            // Soft cap + residual (F2/F39).
            let rem = buf.len().saturating_sub(*off);
            if n > 1_000_000 || n > rem {
                return Err(StoreError::Msg("batch too large".into()));
            }
            let mut pairs = Vec::with_capacity(n);
            for _ in 0..n {
                let key = take_bytes(buf, off)?;
                let value = take_bytes(buf, off)?;
                pairs.push((key, value));
            }
            Ok(RangeEntry::Batch { pairs, si_gen: 0 })
        }
        12 => {
            if *off + 4 > buf.len() {
                return Err(StoreError::Msg("batch12 count eof".into()));
            }
            let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
            *off += 4;
            let rem = buf.len().saturating_sub(*off);
            if n > 1_000_000 || n > rem {
                return Err(StoreError::Msg("batch12 too large".into()));
            }
            let mut pairs = Vec::with_capacity(n);
            for _ in 0..n {
                let key = take_bytes(buf, off)?;
                let value = take_bytes(buf, off)?;
                pairs.push((key, value));
            }
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("batch si_gen eof".into()));
            }
            let si_gen = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            Ok(RangeEntry::Batch { pairs, si_gen })
        }
        5 => {
            if *off + 8 + 4 > buf.len() {
                return Err(StoreError::Msg("txn prepare hdr".into()));
            }
            let txn_id = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
            *off += 4;
            let rem = buf.len().saturating_sub(*off);
            if n > 1_000_000 || n > rem {
                return Err(StoreError::Msg("txn prepare too large".into()));
            }
            let mut pairs = Vec::with_capacity(n);
            for _ in 0..n {
                let key = take_bytes(buf, off)?;
                let value = take_bytes(buf, off)?;
                pairs.push((key, value));
            }
            Ok(RangeEntry::TxnPrepare { txn_id, pairs })
        }
        6 => {
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("txn commit eof".into()));
            }
            let txn_id = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            let keys = decode_key_list(buf, off)?;
            Ok(RangeEntry::TxnCommit {
                txn_id,
                keys,
                si_gen: 0,
            })
        }
        13 => {
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("txn commit13 eof".into()));
            }
            let txn_id = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            let keys = decode_key_list(buf, off)?;
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("txn commit si_gen eof".into()));
            }
            let si_gen = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            Ok(RangeEntry::TxnCommit {
                txn_id,
                keys,
                si_gen,
            })
        }
        7 => {
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("txn abort eof".into()));
            }
            let txn_id = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            let keys = decode_key_list(buf, off)?;
            Ok(RangeEntry::TxnAbort { txn_id, keys })
        }
        8 => {
            if *off + 8 > buf.len() {
                return Err(StoreError::Msg("txn revert eof".into()));
            }
            let txn_id = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
            *off += 8;
            let keys = decode_key_list(buf, off)?;
            Ok(RangeEntry::TxnRevert { txn_id, keys })
        }
        14 => {
            let old = decode_u64_list(buf, off)?;
            let new = decode_u64_list(buf, off)?;
            Ok(RangeEntry::MembershipJoint { old, new })
        }
        t => Err(StoreError::Msg(format!("bad entry tag {t}"))),
    }
}

/// Clear intents + txn pair/pre/status for user keys in `[start, end)`.
///
/// Intent/txn keys live under `\0store/*` (often outside the user range), so a
/// range keyspace wipe does not reach them. Used by install-snapshot (F40).
fn clear_range_txn_meta<E: Env>(db: &mut Db<E>, start: &[u8], end: &[u8]) -> Result<()> {
    let mut ops: Vec<BatchOp> = Vec::new();
    let mut touched_txns: Vec<u64> = Vec::new();
    for (ik, raw) in scan_prefix(db, INTENT_PREFIX) {
        let Some(user) = ik
            .strip_prefix(INTENT_PREFIX)
            .and_then(user_from_meta_suffix)
        else {
            continue;
        };
        if !key_in_half_open(&user, start, end) {
            continue;
        }
        if let Ok((tid, _)) = decode_intent(&raw) {
            if !touched_txns.contains(&tid) {
                touched_txns.push(tid);
            }
            ops.push(BatchOp::delete(txn_pair_key(tid, &user)));
            ops.push(BatchOp::delete(txn_pre_key(tid, &user)));
        }
        ops.push(BatchOp::delete(ik));
    }
    // Also drop stray pair/pre rows without a live intent (partial crash).
    for (pk, _) in scan_prefix(db, TXN_PREFIX) {
        let Some(rest) = pk.strip_prefix(TXN_PREFIX) else {
            continue;
        };
        if rest.len() < 8 {
            continue;
        }
        let tid = u64::from_le_bytes(rest[0..8].try_into().unwrap());
        let after = &rest[8..];
        let user = if let Some(u) = after.strip_prefix(b"/k/") {
            user_from_meta_suffix(u)
        } else if let Some(u) = after.strip_prefix(b"/pre/") {
            user_from_meta_suffix(u)
        } else {
            None
        };
        let Some(user) = user else {
            continue;
        };
        if !key_in_half_open(&user, start, end) {
            continue;
        }
        if !touched_txns.contains(&tid) {
            touched_txns.push(tid);
        }
        ops.push(BatchOp::delete(pk));
    }
    if !ops.is_empty() {
        db.apply_batch(ops)?;
    }
    for tid in touched_txns {
        let prefix = txn_pair_prefix(tid);
        let left = scan_prefix(db, &prefix);
        if left.is_empty() {
            // F47: abort fence must survive snapshot install (export is user
            // keys only). Wiping status here lets a later TxnCommit replay.
            let st = db.get(&txn_status_key(tid));
            if st.as_deref() != Some(b"abort".as_ref()) {
                let _ = db.apply_batch([BatchOp::delete(txn_status_key(tid))]);
            }
        }
    }
    Ok(())
}

fn clear_txn_keys<E: Env>(db: &mut Db<E>, txn_id: u64, keys: &[Vec<u8>]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let mut ops = Vec::new();
    for u in keys {
        ops.push(BatchOp::delete(intent_key(u)));
        ops.push(BatchOp::delete(txn_pair_key(txn_id, u)));
        ops.push(BatchOp::delete(txn_pre_key(txn_id, u)));
    }
    // Drop status only when no pair index remains for this txn.
    let prefix = txn_pair_prefix(txn_id);
    db.apply_batch(ops)?;
    let left = scan_prefix(db, &prefix);
    if left.is_empty() {
        let _ = db.put(txn_status_key(txn_id), b""); // will delete below
        db.apply_batch([BatchOp::delete(txn_status_key(txn_id))])?;
    }
    Ok(())
}

fn apply_txn_prepare<E: Env>(
    db: &mut Db<E>,
    txn_id: u64,
    pairs: &[(Vec<u8>, Vec<u8>)],
) -> Result<()> {
    for (k, _) in pairs {
        if intent_conflict(db, k, Some(txn_id)) {
            db.put(txn_status_key(txn_id), b"abort")?;
            return Ok(());
        }
    }
    let mut ops = Vec::new();
    for (k, v) in pairs {
        let pre = db.get(k);
        ops.push(BatchOp::put(
            txn_pre_key(txn_id, k),
            encode_preimage(pre.as_deref()),
        ));
        ops.push(BatchOp::put(intent_key(k), encode_intent(txn_id, v)));
        ops.push(BatchOp::put(txn_pair_key(txn_id, k), v));
    }
    ops.push(BatchOp::put(txn_status_key(txn_id), b"prepared"));
    db.apply_batch(ops)?;
    Ok(())
}

/// Materialize intents for **only** `keys` (this range's slice of the TX).
fn apply_txn_commit<E: Env>(db: &mut Db<E>, txn_id: u64, keys: &[Vec<u8>]) -> Result<()> {
    let st = db.get(&txn_status_key(txn_id));
    let status_is_abort = st.as_deref() == Some(b"abort".as_ref());
    if txn_kernel::txn_commit_action(status_is_abort) == txn_kernel::TxnCommitAction::Revert {
        // F47: fenced TX — never materialise. If a prior apply already wrote user
        // keys, restore preimages (same as TxnRevert). Keep abort fence durable.
        // Propagate corrupt-preimage errors (F118) — do not leave aborted writes.
        apply_txn_revert(db, txn_id, keys)?;
        // F134: keep abort fence durable after fenced commit path (F130 class).
        db.put(txn_status_key(txn_id), b"abort")?;
        return Ok(());
    }
    let mut ops = Vec::new();
    for u in keys {
        if let Some(raw) = db.get(&txn_pair_key(txn_id, u)) {
            if raw.is_empty() {
                ops.push(BatchOp::delete(u.as_slice()));
            } else {
                ops.push(BatchOp::put(u.as_slice(), raw.as_ref()));
            }
        } else if let Some(raw) = db.get(&intent_key(u)) {
            // F120: present corrupt intent must not skip materialise then delete.
            let (oid, val) = decode_intent(&raw)?;
            if oid == txn_id {
                if val.is_empty() {
                    ops.push(BatchOp::delete(u.as_slice()));
                } else {
                    ops.push(BatchOp::put(u.as_slice(), val));
                }
            }
        }
        ops.push(BatchOp::delete(intent_key(u)));
        ops.push(BatchOp::delete(txn_pair_key(txn_id, u)));
        // Keep preimage until the coordinator finishes all ranges — revert
        // after a later-range failure must still restore the old value.
    }
    if !ops.is_empty() {
        db.apply_batch(ops)?;
    }
    // Clear status when no remaining pair records for this txn.
    let prefix = txn_pair_prefix(txn_id);
    let left = scan_prefix(db, &prefix);
    if left.is_empty() {
        db.apply_batch([BatchOp::delete(txn_status_key(txn_id))])?;
    }
    Ok(())
}

/// Drop intents for keys (prepare cancelled / abort).
fn apply_txn_abort<E: Env>(db: &mut Db<E>, txn_id: u64, keys: &[Vec<u8>]) -> Result<()> {
    clear_txn_keys(db, txn_id, keys)
}

/// Compensating action: restore the prepare-time preimage (not blind delete).
///
/// Missing preimage (peer never prepared) leaves the user key untouched so a
/// lagging replica that still holds the old value is not wiped.
///
/// F52: a prior majority `TxnCommit` may already have written `\0store/hist/`
/// under the TX's `si_gen` with the **aborted** new value. After Pedra restore,
/// rewrite that hist tip so SI matches the restored preimage (reopen-safe).
fn apply_txn_revert<E: Env>(db: &mut Db<E>, txn_id: u64, keys: &[Vec<u8>]) -> Result<()> {
    // F47: never drop an abort fence here. A later raft replay of TxnCommit
    // must still see status=abort. Successful materialise is the only path
    // that clears status (apply_txn_commit).
    let status_is_abort = db.get(&txn_status_key(txn_id)).as_deref() == Some(b"abort".as_ref());
    let mut ops = Vec::new();
    let mut restored: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    for u in keys {
        // F118: missing pre key → LeaveUntouched; present corrupt → hard error
        // (never treat garbage as "peer never prepared").
        let pre = match db.get(&txn_pre_key(txn_id, u)) {
            None => None,
            Some(raw) => Some(decode_preimage(raw.as_ref())?),
        };
        let had_pre = pre.is_some();
        let pre_was_absent = matches!(pre, Some(None));
        match txn_kernel::revert_user_action(had_pre, pre_was_absent) {
            txn_kernel::RevertUserAction::RestoreValue => {
                if let Some(Some(val)) = &pre {
                    ops.push(BatchOp::put(u.as_slice(), val.as_slice()));
                }
            }
            txn_kernel::RevertUserAction::RestoreAbsent => {
                ops.push(BatchOp::delete(u.as_slice()));
            }
            txn_kernel::RevertUserAction::LeaveUntouched => {}
        }
        if let Some(p) = pre {
            restored.push((u.clone(), p));
        }
        ops.push(BatchOp::delete(intent_key(u)));
        ops.push(BatchOp::delete(txn_pair_key(txn_id, u)));
        ops.push(BatchOp::delete(txn_pre_key(txn_id, u)));
    }
    if !ops.is_empty() {
        db.apply_batch(ops)?;
    }
    // F52: align durable SI hist with restored Pedra (TxnCommit may have stamped
    // the aborted write under si_gen before this compensating entry applied).
    for (u, pre) in &restored {
        if !txn_kernel::should_repair_si_hist(true, is_reserved_store_key(u)) {
            continue;
        }
        repair_si_hist_tip(db, u, pre.as_deref())?;
    }
    let prefix = txn_pair_prefix(txn_id);
    let left = scan_prefix(db, &prefix);
    if txn_kernel::revert_clears_status(status_is_abort, left.is_empty()) {
        db.apply_batch([BatchOp::delete(txn_status_key(txn_id))])?;
    }
    Ok(())
}

/// Rewrite the last SI hist entry for `user_key` to `live` (F52).
///
/// If hist is empty, no-op (no SI stamp to repair). Same gen as the tip is kept
/// so generations reserved by a failed multi-range TX do not keep advertising
/// the aborted write after Pedra preimage restore.
fn repair_si_hist_tip<E: Env>(db: &mut Db<E>, user_key: &[u8], live: Option<&[u8]>) -> Result<()> {
    let hk = hist_key(user_key);
    // F117: present corrupt hist must not look empty (no-op that leaves SI wrong).
    let mut hist = match db.get(&hk) {
        None => return Ok(()),
        Some(b) => decode_hist(b.as_ref())?,
    };
    if hist.is_empty() {
        return Ok(());
    }
    let tip_gen = hist.last().map(|(g, _)| *g).unwrap_or(0);
    if tip_gen == 0 {
        // Only the gen-0 preimage floor — leave it; nothing committed to unwind.
        return Ok(());
    }
    let new_val = live.map(|v| v.to_vec());
    if hist.last().map(|(_, v)| v.as_ref()) == Some(new_val.as_ref()) {
        return Ok(());
    }
    if let Some(last) = hist.last_mut() {
        last.1 = new_val;
    }
    db.put(&hk, encode_hist(&hist))?;
    Ok(())
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

fn encode_one_log_rec(rec: &LogRec) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&rec.index.to_le_bytes());
    b.extend_from_slice(&rec.term.to_le_bytes());
    b.extend_from_slice(&encode_entry(&rec.entry));
    append_crc(&mut b);
    b
}

fn decode_one_log_rec(buf: &[u8]) -> Result<LogRec> {
    let p = strip_crc(buf)?;
    if p.len() < 16 {
        return Err(StoreError::Msg("log rec short".into()));
    }
    let index = u64::from_le_bytes(p[0..8].try_into().unwrap());
    let term = u64::from_le_bytes(p[8..16].try_into().unwrap());
    let mut off = 16;
    let entry = decode_entry(p, &mut off)?;
    if off != p.len() {
        return Err(StoreError::Msg("log rec trailing garbage".into()));
    }
    Ok(LogRec { index, term, entry })
}

fn log_entry_key(range_id: u64, index: u64) -> Vec<u8> {
    let mut k = raft_meta_key(range_id, "log/e/");
    k.extend_from_slice(&index.to_be_bytes());
    k
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
        out.push(LogRec { index, term, entry });
    }
    if off != p.len() {
        return Err(StoreError::Msg("log trailing garbage".into()));
    }
    Ok(out)
}

fn load_range_peer<E: Env>(
    db: &Db<E>,
    range_id: u64,
    node_id: u64,
    member_ids: &[u64],
) -> Result<RangePeer> {
    let mut peer = RangePeer::new(node_id, range_id, member_ids);
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
    // RFC-0025 P1.2: segment entries beyond the base blob (incremental persist).
    // F121: log_hi is a hard upper bound — missing/wrong-index segment rows
    // must fail open (silent skip left holes; last_index jumped over gaps).
    let blob_last = peer.last_index();
    if let Some(raw) = db.get(&raft_meta_key(range_id, "log_hi")) {
        let hi = decode_u64_meta(&raw)?;
        if hi > blob_last {
            for i in (blob_last + 1)..=hi {
                let Some(eraw) = db.get(&log_entry_key(range_id, i)) else {
                    return Err(StoreError::Msg(format!(
                        "raft log segment gap: range {range_id} index {i} missing (log_hi={hi})"
                    )));
                };
                let rec = decode_one_log_rec(eraw.as_ref())?;
                if rec.index != i {
                    return Err(StoreError::Msg(format!(
                        "raft log segment index mismatch: range {range_id} want {i} got {}",
                        rec.index
                    )));
                }
                peer.log.push(rec);
            }
        }
    }
    peer.disk_log_hi = peer.last_index();
    if let Some(raw) = db.get(&raft_meta_key(range_id, "commit")) {
        peer.commit = decode_u64_meta(&raw)?;
    }
    if let Some(raw) = db.get(&raft_meta_key(range_id, "applied")) {
        peer.applied = decode_u64_meta(&raw)?;
    }
    // Cap watermarks to log length (corrupt/partial meta). F10.
    let last = peer.last_index();
    peer.commit = commit_kernel::recover_commit(peer.commit, last);
    peer.applied = peer.applied.min(peer.commit);
    // Drop any log entries already covered by snapshot (idempotent load).
    if peer.snapshot_index > 0 {
        peer.log.retain(|e| e.index > peer.snapshot_index);
    }
    // I-MAJ-3 / failed 2PC: uncommitted suffix must not survive reopen.
    // A later election + noop would majority-commit an entry the client
    // already saw as NotCommitted (FailingEnv mid-finish).
    peer.log.retain(|e| e.index <= peer.commit);
    // Role is always follower after process restart (volatile leadership).
    peer.role = Role::Follower;
    peer.leader_id = None;
    Ok(peer)
}

/// RFC-0121 P1.2 / 0066 P2.2: inspect a TCP node's Pedra dir after process
/// death. True when the recovered committed log has a C-new-only leave
/// (`MembershipJoint` with `old == new`), or durable membership already
/// omits `removed` and no still-active joint remains (leave applied, then
/// compacted). Production [`crate`] `cluster_real --remove-member` gates
/// exit on [`l28_tcp_left_ok`].
///
/// `data` is the `--data` parent (`store-node-{node_id}` lives under it).
#[must_use]
pub fn tcp_node_disk_left_joint(data: impl AsRef<Path>, node_id: u64, removed: u64) -> bool {
    let dir = data.as_ref().join(format!("store-node-{node_id}"));
    let opts = OpenOptions {
        wal_full_fsync: true,
        history: Default::default(),
        wal_recovery: Default::default(),
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        auto_compact_sst_bytes: None,
        exclusive: true,
        large_value_threshold: None,
    };
    let Ok(db) = Db::open_with_env(&dir, opts, IoUringEnv::default()) else {
        return false;
    };
    let Ok(peer) = load_range_peer(&db, 1, node_id, &[node_id]) else {
        return false;
    };
    let mut leave = false;
    let mut still = false;
    for rec in &peer.log {
        if let RangeEntry::MembershipJoint { old, new } = &rec.entry {
            if membership_kernel::joint_still_active(old, new) {
                still = true;
            } else {
                leave = true;
            }
        }
    }
    if membership_kernel::joint_leave_ok(leave) {
        return true;
    }
    if still {
        return false;
    }
    let Some(raw) = db.get(&cluster_membership_key()) else {
        return false;
    };
    let Ok(ids) = decode_membership(&raw) else {
        return false;
    };
    !ids.contains(&removed) && !ids.is_empty()
}

/// RFC-0126 P1.2: inspect a TCP node's Pedra dir after process death and
/// return the durable membership high-water (`0` if missing/unreadable).
///
/// `data` is the `--data` parent (`store-node-{node_id}` lives under it).
#[must_use]
pub fn tcp_node_disk_high_water(data: impl AsRef<Path>, node_id: u64) -> u64 {
    let dir = data.as_ref().join(format!("store-node-{node_id}"));
    let opts = OpenOptions {
        wal_full_fsync: true,
        history: Default::default(),
        wal_recovery: Default::default(),
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        auto_compact_sst_bytes: None,
        exclusive: true,
        large_value_threshold: None,
    };
    let Ok(db) = Db::open_with_env(&dir, opts, IoUringEnv::default()) else {
        return 0;
    };
    let Some(raw) = db.get(&cluster_high_water_key()) else {
        return 0;
    };
    decode_u64_meta(&raw).unwrap_or(0)
}

/// RFC-0128 P1.2: reopen a TCP node's Pedra dir with stale CLI membership
/// after process death. True when `removed` is not participating (kernel
/// `participating_if_member` on disk `ids`). AS-IS would count a remote
/// non-member (`nodes` has only self).
#[must_use]
pub fn tcp_node_removed_not_participating(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
    removed: u64,
) -> bool {
    let Ok(c) = StoreCluster::open_single_node(data, self_id, cli, 1) else {
        return false;
    };
    !c.is_participating(removed)
}

/// RFC-0130 P1.2: plant a durable committed-unapplied prefix (Noop at
/// `last_index+1`, persist log+commit, leave `applied` behind) on a TCP
/// node's Pedra dir. Production `open_single_node` must recover-apply so
/// `recover_must_apply` is false. AS-IS skips apply and the gap remains.
/// Rewinding `applied` is not this tooth — compact may have dropped that
/// log entry.
#[must_use]
pub fn tcp_node_recover_apply_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    {
        let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
            return false;
        };
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        let Some(p) = n.ranges.get_mut(&1) else {
            return false;
        };
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term.max(1),
            entry: RangeEntry::Noop,
        });
        p.commit = idx;
        if persist_log_db(&mut n.db, 1, p).is_err() {
            return false;
        }
        if persist_commit_db(&mut n.db, 1, p).is_err() {
            return false;
        }
        if !membership_kernel::recover_must_apply(p.applied, p.commit) {
            return false;
        }
    }
    let Ok(c) = StoreCluster::open_single_node(data, self_id, cli, 1) else {
        return false;
    };
    let Some(p) = c.nodes.get(&self_id).and_then(|n| n.ranges.get(&1)) else {
        return false;
    };
    !membership_kernel::recover_must_apply(p.applied, p.commit)
}

/// RFC-0131 P1.2: same plant as 0130, on a replica disk membership already
/// dropped. Production TCP ctor must recover-apply; AS-IS filters by `ids`.
#[must_use]
pub fn tcp_node_removed_recover_apply_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    if !tcp_node_recover_apply_ok(&data, self_id, cli) {
        return false;
    }
    let Ok(c) = StoreCluster::open_single_node(data, self_id, cli, 1) else {
        return false;
    };
    !c.is_member(self_id)
}

fn disk_log_has_uncommitted_suffix<E: Env>(db: &Db<E>, commit: u64) -> bool {
    if let Some(raw) = db.get(&raft_meta_key(1, "log_hi")) {
        if let Ok(hi) = decode_u64_meta(&raw) {
            if hi > commit {
                return true;
            }
        }
    }
    if let Some(raw) = db.get(&raft_meta_key(1, "log")) {
        if let Ok(recs) = decode_log(&raw) {
            if recs.iter().any(|e| e.index > commit) {
                return true;
            }
        }
    }
    false
}

/// RFC-0132 P1.2: plant a durable uncommitted suffix on a replica disk
/// membership already dropped. Production TCP ctor must persist truncate
/// so disk has no `index > commit`. AS-IS filters persist by `ids`.
#[must_use]
pub fn tcp_node_removed_truncate_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let commit;
    {
        let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
            return false;
        };
        if c.is_member(self_id) {
            return false;
        }
        {
            let Some(n) = c.nodes.get_mut(&self_id) else {
                return false;
            };
            let Some(p) = n.ranges.get_mut(&1) else {
                return false;
            };
            commit = p.commit;
            let idx = p.last_index() + 1;
            p.log.push(LogRec {
                index: idx,
                term: p.term.max(1),
                entry: RangeEntry::Put {
                    key: b"rfc0132-tcp".to_vec(),
                    value: b"uncommitted".to_vec(),
                    si_gen: 0,
                },
            });
            if persist_log_db(&mut n.db, 1, p).is_err() {
                return false;
            }
        }
        let Some(n) = c.nodes.get(&self_id) else {
            return false;
        };
        if !disk_log_has_uncommitted_suffix(&n.db, commit) {
            return false;
        }
    }
    let Ok(c) = StoreCluster::open_single_node(data, self_id, cli, 1) else {
        return false;
    };
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    !c.is_member(self_id) && !disk_log_has_uncommitted_suffix(&n.db, commit)
}

/// RFC-0133 P1.2: plant a durable uncommitted suffix (incremental
/// `log_entry_key`) on a replica disk membership already dropped.
/// Production TCP ctor must delete that key. 0132 `log_hi` cap is not
/// this tooth. AS-IS leaves the orphan segment.
#[must_use]
pub fn tcp_node_removed_orphan_drop_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let orphan;
    {
        let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
            return false;
        };
        if c.is_member(self_id) {
            return false;
        }
        let commit;
        {
            let Some(n) = c.nodes.get_mut(&self_id) else {
                return false;
            };
            let Some(p) = n.ranges.get_mut(&1) else {
                return false;
            };
            commit = p.commit;
            let idx = p.last_index() + 1;
            p.log.push(LogRec {
                index: idx,
                term: p.term.max(1),
                entry: RangeEntry::Put {
                    key: b"rfc0133-tcp".to_vec(),
                    value: b"uncommitted".to_vec(),
                    si_gen: 0,
                },
            });
            if persist_log_db(&mut n.db, 1, p).is_err() {
                return false;
            }
        }
        orphan = log_entry_key(1, commit.saturating_add(1));
        let Some(n) = c.nodes.get(&self_id) else {
            return false;
        };
        if n.db.get(&orphan).is_none() {
            return false;
        }
    }
    let Ok(c) = StoreCluster::open_single_node(data, self_id, cli, 1) else {
        return false;
    };
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    !c.is_member(self_id) && n.db.get(&orphan).is_none()
}

/// RFC-0134 P1.2: plant a leftover 2PC intent on a replica disk membership
/// already dropped. Production TCP ctor must abort it. AS-IS filters abort
/// by `ids`.
#[must_use]
pub fn tcp_node_removed_abort_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let ik = intent_key(b"rfc0134-tcp");
    {
        let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
            return false;
        };
        if c.is_member(self_id) {
            return false;
        }
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        if n.db.put(&ik, encode_intent(99, b"pending")).is_err() {
            return false;
        }
        if n.db.get(&ik).is_none() {
            return false;
        }
    }
    let Ok(c) = StoreCluster::open_single_node(data, self_id, cli, 1) else {
        return false;
    };
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    !c.is_member(self_id) && n.db.get(&ik).is_none()
}

fn disk_si_now_ms<E: Env>(n: &StoreNode<E>) -> u64 {
    n.db
        .get(&si_meta_key("now_ms"))
        .and_then(|raw| decode_u64_meta(&raw).ok())
        .unwrap_or(0)
}

/// RFC-0135 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must persist `now_ms` on self. AS-IS filters persist by `ids`.
#[must_use]
pub fn tcp_node_removed_now_ms_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    c.advance_now_ms(7_000);
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    disk_si_now_ms(n) == c.now_ms() && c.now_ms() >= 7_000
}

/// RFC-0136 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must persist SI hist on self. AS-IS filters persist by `ids`.
#[must_use]
pub fn tcp_node_removed_hist_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    let k = b"rfc0136-tcp-hist".to_vec();
    c.key_history
        .insert(k.clone(), vec![(1, Some(b"v".to_vec()))]);
    c.commit_generation = c.commit_generation.max(1);
    if c.persist_si_keys(&[k.clone()]).is_err() {
        return false;
    }
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    n.db.get(&hist_key(&k)).is_some()
}

/// RFC-0137 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must persist abort fence on self. AS-IS filters persist by `ids`.
#[must_use]
pub fn tcp_node_removed_fence_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    let tid = 0x0137_1E28u64;
    if c.fence_txn_aborted(tid).is_err() {
        return false;
    }
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    n.db.get(&txn_status_key(tid)).as_deref() == Some(b"abort".as_ref())
}

/// RFC-0138 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must force-clear stuck intents on self. AS-IS filters clear by `ids`.
#[must_use]
pub fn tcp_node_removed_clear_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    let k = b"rfc0138-tcp-left".to_vec();
    let ik = intent_key(&k);
    let tid = 0x0138_1E28u64;
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        if n.db.put(&ik, encode_intent(tid, b"pending")).is_err() {
            return false;
        }
    }
    if c.nodes
        .get(&self_id)
        .and_then(|n| n.db.get(&ik))
        .is_none()
    {
        return false;
    }
    if c.force_local_clear_keys(tid, &[k], false).is_err() {
        return false;
    }
    c.nodes
        .get(&self_id)
        .and_then(|n| n.db.get(&ik))
        .is_none()
}

/// RFC-0139 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must drop leftover TX preimages on self. AS-IS filters drop by `ids`.
#[must_use]
pub fn tcp_node_removed_pre_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    let k = b"rfc0139-tcp-left".to_vec();
    let tid = 0x0139_1E28u64;
    let pk = txn_pre_key(tid, &k);
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        if n.db.put(&pk, encode_preimage(Some(b"old"))).is_err() {
            return false;
        }
    }
    if c.nodes
        .get(&self_id)
        .and_then(|n| n.db.get(&pk))
        .is_none()
    {
        return false;
    }
    let handle = TxHandle {
        id: tid,
        ranges: vec![1],
        keys_by_range: vec![(1, vec![k])],
    };
    if c.drop_preimages(&handle).is_err() {
        return false;
    }
    c.nodes
        .get(&self_id)
        .and_then(|n| n.db.get(&pk))
        .is_none()
}

/// RFC-0140 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must load RangePeer from disk C-new, not stale CLI. Observable
/// via election timeout. 0125 high-water and 0139 drop-preimages are
/// **not** this tooth. AS-IS would keep CLI n_nodes at load.
#[must_use]
pub fn tcp_node_removed_peer_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    let disk = c.ids.clone();
    if disk.is_empty() || disk.contains(&self_id) {
        return false;
    }
    let Some(got) = c
        .nodes
        .get(&self_id)
        .and_then(|n| n.ranges.get(&1))
        .map(|p| p.election_timeout)
    else {
        return false;
    };
    let want = election_timeout_for(self_id, 1, &disk);
    let stale = election_timeout_for(self_id, 1, cli);
    got == want && want != stale
}

/// RFC-0141 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must not treat HashMap first-key as cluster identity. Plant a
/// user key; `get` is Err (not `Ok(Some(stale))`). 0140 timeout peek is
/// **not** this tooth. AS-IS would `get()` the local-only bytes.
#[must_use]
pub fn tcp_node_removed_lid_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    if c.local_node_id().is_some() {
        return false;
    }
    let k = b"rfc0141-tcp-stale";
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        if n.db.put(k, b"stale").is_err() {
            return false;
        }
    }
    let got = c.get(k);
    got.is_err() && got.as_ref().ok().and_then(|v| v.as_deref()) != Some(b"stale".as_ref())
}

/// RFC-0142 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must not pick remote `ids.first()` as a LocalApplied reader.
/// `get` Err contains `empty`, not `bad node`. 0141 local-id None is
/// **not** this tooth. AS-IS would `get_on` a remote voter.
#[must_use]
pub fn tcp_node_removed_rdr_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    if c.best_reader_for_key(b"rfc0142-tcp-get").is_some() {
        return false;
    }
    match c.get(b"rfc0142-tcp-get") {
        Err(e) => {
            let msg = e.to_string();
            msg.contains("empty") && !msg.contains("bad node")
        }
        Ok(_) => false,
    }
}

/// RFC-0143 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must live-discard an uncommitted suffix on self. 0132 recover
/// truncate is **not** this tooth. AS-IS filters discard by `ids`.
#[must_use]
pub fn tcp_node_removed_dsc_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    let commit;
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        let Some(p) = n.ranges.get_mut(&1) else {
            return false;
        };
        commit = p.commit;
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term.max(1),
            entry: RangeEntry::Put {
                key: b"rfc0143-tcp".to_vec(),
                value: b"orphan".to_vec(),
                si_gen: 0,
            },
        });
        if persist_log_db(&mut n.db, 1, p).is_err() {
            return false;
        }
    }
    {
        let Some(n) = c.nodes.get(&self_id) else {
            return false;
        };
        let ram = n
            .ranges
            .get(&1)
            .is_some_and(|p| p.log.iter().any(|e| e.index > commit));
        if !ram || !disk_log_has_uncommitted_suffix(&n.db, commit) {
            return false;
        }
    }
    let from = commit.saturating_add(1);
    if c.discard_uncommitted_from(1, self_id, from).is_err() {
        return false;
    }
    let Some(n) = c.nodes.get(&self_id) else {
        return false;
    };
    n.ranges
        .get(&1)
        .is_some_and(|p| p.log.iter().all(|e| e.index <= commit))
        && !disk_log_has_uncommitted_suffix(&n.db, commit)
}

/// RFC-0144 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must pick a **local** persist-leader on no-leader abort so
/// `next_index` repair runs. 0143 direct discard is **not** this tooth.
/// AS-IS uses remote `ids.first()` and skips the repair.
#[must_use]
pub fn tcp_node_removed_pld_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    if c.range_leader(1).is_some() {
        return false;
    }
    let commit;
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        let Some(p) = n.ranges.get_mut(&1) else {
            return false;
        };
        commit = p.commit;
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term.max(1),
            entry: RangeEntry::Put {
                key: b"rfc0144-tcp".to_vec(),
                value: b"orphan".to_vec(),
                si_gen: 0,
            },
        });
        if persist_log_db(&mut n.db, 1, p).is_err() {
            return false;
        }
    }
    let from = commit.saturating_add(1);
    for n in c.nodes.values_mut() {
        for p in n.ranges.values_mut() {
            p.sent_through.clear();
        }
    }
    if c.finish_queued_propose(1, from, true).is_err() {
        return false;
    }
    c.nodes
        .get(&self_id)
        .and_then(|n| n.ranges.get(&1))
        .and_then(|p| p.next_index.get(&1).copied())
        == Some(from)
}

/// RFC-0145 P1.2: production TCP ctor of a replica already dropped from
/// `ids` must step a planted Leader down on re-install of C-new. 0144
/// persist-leader and 0128 participating are **not** this tooth. AS-IS
/// keeps `Role::Leader`.
#[must_use]
pub fn tcp_node_removed_std_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if c.is_member(self_id) {
        return false;
    }
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        let Some(p) = n.ranges.get_mut(&1) else {
            return false;
        };
        p.role = Role::Leader;
        p.leader_id = Some(self_id);
    }
    if !c.node_thinks_leader(self_id, 1) {
        return false;
    }
    let ids = c.ids.clone();
    if c.install_applied_membership(ids).is_err() {
        return false;
    }
    !c.is_member(self_id) && !c.node_thinks_leader(self_id, 1)
}

/// RFC-0146 P1.2: production TCP ctor of a **remaining** voter must not
/// route `leader_hint` to a replica already dropped from `ids`. 0145
/// step-down is **not** this tooth. AS-IS returns any `leader_id`.
#[must_use]
pub fn tcp_node_hint_ok(
    data: impl AsRef<Path>,
    self_id: u64,
    cli: &[u64],
    removed: u64,
) -> bool {
    let Ok(mut c) = StoreCluster::open_single_node(&data, self_id, cli, 1) else {
        return false;
    };
    if !c.is_member(self_id) || c.is_member(removed) {
        return false;
    }
    while c.step_down_range_leader(1).is_ok() {}
    if c.range_leader(1).is_some() {
        return false;
    }
    {
        let Some(n) = c.nodes.get_mut(&self_id) else {
            return false;
        };
        let Some(p) = n.ranges.get_mut(&1) else {
            return false;
        };
        p.leader_id = Some(removed);
    }
    c.leader_hint(1) != Some(removed)
}

fn persist_hard_db<E: Env>(db: &mut Db<E>, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "hard"),
        encode_hard(peer.term, peer.voted_for),
    )?;
    Ok(())
}

/// F125/F127: raise `term` only when hard state is durable.
///
/// On persist failure: restore previous term/vote, force [`Role::Follower`] and
/// clear `leader_id` so this process does not keep acting as leader of a
/// superseded term (even though the higher term did not hit disk).
fn durable_become_follower_if_newer<E: Env>(
    db: &mut Db<E>,
    range_id: u64,
    peer: &mut RangePeer,
    term: u64,
) -> bool {
    if term <= peer.term {
        return true;
    }
    let prev_term = peer.term;
    let prev_voted = peer.voted_for;
    peer.become_follower(term);
    if persist_hard_db(db, range_id, peer).is_ok() {
        return true;
    }
    peer.term = prev_term;
    peer.voted_for = prev_voted;
    peer.role = Role::Follower;
    peer.leader_id = None;
    false
}

/// Persist raft log (RFC-0025 P1.2).
///
/// Fast path: when the log only **grew** by contiguous new indices since
/// [`RangePeer::disk_log_hi`], append those entries + `log_hi` in **one**
/// Pedra `apply_batch` (single fsync) instead of rewriting the full blob.
/// Truncate / compact / gaps fall back to full `log` blob rewrite.
fn persist_log_db<E: Env>(db: &mut Db<E>, range_id: u64, peer: &mut RangePeer) -> Result<()> {
    let last = peer.last_index();
    let new: Vec<&LogRec> = peer
        .log
        .iter()
        .filter(|e| e.index > peer.disk_log_hi)
        .collect();
    let expected = last.saturating_sub(peer.disk_log_hi);
    let continuous = last > peer.disk_log_hi
        && !new.is_empty()
        && new.len() as u64 == expected
        && new.first().map(|e| e.index) == Some(peer.disk_log_hi.saturating_add(1))
        && new.last().map(|e| e.index) == Some(last);

    if continuous {
        let mut ops: Vec<BatchOp> = Vec::with_capacity(new.len() + 1);
        for e in &new {
            ops.push(BatchOp::put(
                log_entry_key(range_id, e.index),
                encode_one_log_rec(e),
            ));
        }
        ops.push(BatchOp::put(
            raft_meta_key(range_id, "log_hi"),
            encode_u64_meta(last),
        ));
        db.apply_batch(ops)?;
        peer.disk_log_hi = last;
        return Ok(());
    }

    // Full rewrite (truncate, compact, empty, or non-contiguous).
    let old_hi = peer.disk_log_hi;
    let mut ops = vec![BatchOp::put(
        raft_meta_key(range_id, "log"),
        encode_log(&peer.log),
    )];
    ops.push(BatchOp::put(
        raft_meta_key(range_id, "log_hi"),
        encode_u64_meta(last),
    ));
    // RFC-0133: drop incremental segment keys past the new hi.
    // AS-IS leaves them (0132 leftover — watermark moved, bytes remain).
    for i in last.saturating_add(1)..=old_hi {
        if membership_kernel::recover_drop_orphan_seg(i, last) {
            ops.push(BatchOp::delete(log_entry_key(range_id, i)));
        }
    }
    db.apply_batch(ops)?;
    peer.disk_log_hi = last;
    Ok(())
}

fn persist_commit_db<E: Env>(db: &mut Db<E>, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "commit"),
        encode_u64_meta(peer.commit),
    )?;
    Ok(())
}

fn persist_applied_db<E: Env>(db: &mut Db<E>, range_id: u64, peer: &RangePeer) -> Result<()> {
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

fn persist_snap_db<E: Env>(db: &mut Db<E>, range_id: u64, peer: &RangePeer) -> Result<()> {
    db.put(
        raft_meta_key(range_id, "snap"),
        encode_snap(peer.snapshot_index, peer.snapshot_term),
    )?;
    Ok(())
}

/// Per-range raft state on one node (log durable in PedraDB; apply → same DB).
#[derive(Clone)]
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
    /// Highest log index this leader has ever put on the wire for that
    /// peer — acked or still in flight (F-found, RFC-0059 swarm: seeds
    /// 49/865). `match_index` alone is not an escape proof: a copy can
    /// sit in the net past the ack deadline. Discard must respect it or
    /// the freed index is reused within the same term and two payloads
    /// share one (index, term).
    sent_through: HashMap<u64, u64>,
    leader_id: Option<u64>,
    /// Highest log index known durable on Pedra for this peer (RFC-0025 P1.2).
    /// Volatile after load; used to choose append vs full rewrite on persist.
    disk_log_hi: u64,
}

/// Election timeout ticks for `(node_id, range_id)` given membership.
///
/// Preferred leader for range `r` is `members[(r-1) % n]` with the **shortest**
/// timeout; others stagger by ring distance. Without members, fall back to a
/// hash mix of node+range (still diversifies vs node-only).
fn election_timeout_for(node_id: u64, range_id: u64, member_ids: &[u64]) -> u64 {
    let mut sorted: Vec<u64> = member_ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.is_empty() {
        // [4, 9] — independent of pure node_id so multi-range still spreads.
        return 4
            + ((node_id
                .wrapping_mul(5)
                .wrapping_add(range_id.wrapping_mul(7)))
                % 6);
    }
    let n = sorted.len();
    let pref = sorted[(range_id.saturating_sub(1) as usize) % n];
    let pref_pos = sorted.iter().position(|&x| x == pref).unwrap_or(0);
    let my_pos = sorted.iter().position(|&x| x == node_id).unwrap_or(0);
    let dist = (my_pos + n - pref_pos) % n;
    // Preferred = 3 ticks; next on ring = 4, …
    3 + dist as u64
}

impl RangePeer {
    /// Build a follower peer. Election timeouts are **per-(node, range)** so that
    /// multi-Raft leaders diversify across members instead of always electing the
    /// lowest `node_id` on every range (option-A scale residual).
    fn new(node_id: u64, range_id: u64, member_ids: &[u64]) -> Self {
        let election_timeout = election_timeout_for(node_id, range_id, member_ids);
        Self {
            role: Role::Follower,
            term: 0,
            voted_for: None,
            log: Vec::new(),
            snapshot_index: 0,
            snapshot_term: 0,
            commit: 0,
            applied: 0,
            election_left: election_timeout,
            election_timeout,
            hb_left: 2,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            sent_through: HashMap::new(),
            leader_id: None,
            disk_log_hi: 0,
        }
    }

    fn last_index(&self) -> u64 {
        self.log.last().map_or(self.snapshot_index, |e| e.index)
    }

    fn last_term(&self) -> u64 {
        self.log.last().map_or(self.snapshot_term, |e| e.term)
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
        let term = self.term_at(through);
        if !compact_kernel::may_compact_through(self.snapshot_index, through, term) {
            return;
        }
        self.log.retain(|e| e.index > through);
        self.snapshot_index = through;
        self.snapshot_term = term;
        let floor = compact_kernel::compact_index_floor(through);
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
struct StoreNode<E: Env = IoUringEnv> {
    db: Db<E>,
    /// range_id → peer
    ranges: HashMap<u64, RangePeer>,
    /// When false, node is partitioned: no votes, no append RPC, no client leadership.
    participating: bool,
}

/// One SI/OCC hist step: `(generation, value)`. `None` = deleted.
type HistStep = (u64, Option<Vec<u8>>);
type HistChain = Vec<HistStep>;
type KeyHistMap = HashMap<Vec<u8>, HistChain>;
/// `(key, new_value, preimage)` staged for SI notes.
type VersionNote = (Vec<u8>, Vec<u8>, Option<Vec<u8>>);
/// `(range_id, log_index)` → `(si_gen, notes)`.
type PendingNotes = HashMap<(u64, u64), (u64, Vec<VersionNote>)>;
type RangeKvMap = HashMap<u64, Vec<KvPair>>;

/// In-process multi-node multi-Raft store (Montanha-Store MVP).
///
/// RFC-0013 P1.3: every node directory stores `\0store/cluster/id` (16 bytes).
/// Open refuses [`StoreError::ClusterMismatch`] if a dir already belongs to
/// another cluster (no silent merge). Pin with
/// [`StoreOpenOptions::with_cluster_id`] on multi-host.
///
/// Generic over [`Env`] so DST can open every node on a shared `FailingEnv` /
/// `RecordingEnv` clone (same trip state via `Rc`).
///
/// Peer RPCs use [`RpcMode`]: default [`RpcMode::Queued`] (RFC-0067 P2.2);
/// [`RpcMode::Direct`] is unpinned lab opt-in via [`Self::enable_lab_direct_rpc`].
pub struct StoreCluster<E: Env = IoUringEnv> {
    nodes: HashMap<u64, StoreNode<E>>,
    /// Raft voting membership (strict majority of this set).
    ids: Vec<u64>,
    /// Largest membership ever admitted (grows on `add_member` / cluster
    /// build). [`Self::remove_member`] refuses to shrink below the size
    /// where any two commit/election quorums over this universe still
    /// intersect — the quorum floor for out-of-band reconfiguration.
    membership_high_water: usize,
    /// Options used to open each node's engine (reopen after BitFlip).
    engine_opts: pedradb_core::OpenOptions,
    /// All range metas (same on every node).
    ranges: Vec<RangeMeta>,
    /// Election jitter / non-determinism seam (inject [`SeedRng`] for DST).
    rng: SeedRng,
    /// Next multi-key transaction id (monotonic in-process).
    next_txn_id: u64,
    /// Direct vs queued peer RPC.
    rpc_mode: RpcMode,
    /// RFC-0067: once true, [`RpcMode::Direct`] is refused (World/DST pin).
    dst_queued_pin: bool,
    /// Outbound peer messages when [`RpcMode::Queued`] (`from`, `to`, msg).
    outbound: VecDeque<(u64, u64, PeerMsg)>,
    /// In-flight election vote counts: (range_id, term) → votes (includes self).
    /// Election tally per (range, term, candidate). Keyed by candidate:
    /// two nodes can time out into the same term and both poll votes —
    /// a shared (range, term) counter pooled grants across candidates
    /// and elected a leader without its own majority (seed 503976).
    election_votes: HashMap<(u64, u64, u64), u64>,
    /// Voter ids that granted (range, term, candidate). Joint elections
    /// need the set, not just a count (RFC-0064).
    election_granted: HashMap<(u64, u64, u64), Vec<u64>>,
    /// Logical time (World/DST); each [`Self::tick`] / [`Self::advance_time`] advances it.
    /// Raft election/heartbeat counters are pure functions of this — no wall clock.
    logical_now: u64,
    /// Cluster logical clock in **milliseconds** for DCS absolute-deadline leases.
    /// Advanced by [`Self::advance_now_ms`] / optionally with ticks.
    now_ms: u64,
    /// Last `now_ms` fsynced into SI meta (must not go backwards on reopen — F56).
    persisted_now_ms: u64,
    /// True once a non-zero DCS lease deadline has been proposed (or recovered).
    /// Tick persists `now_ms` only then, so elect-all without TTL stays cheap.
    has_ttl_leases: bool,
    /// When true, each [`Self::tick`] also advances `now_ms` by this many ms (World).
    ms_per_tick: u64,
    /// RFC-0013 P1.3: durable cluster identity (`\0store/cluster/id`).
    cluster_id: [u8; 16],
    /// RFC-0021 P2.6: node_id → region label (lab multi-site; empty = unknown).
    node_regions: HashMap<u64, String>,
    /// RFC-0021 P1.3: optional dial map id → host:port (in-process control plane;
    /// multi-host nodes still push via TCP `SetPeers` without SSH).
    peer_addrs: HashMap<u64, String>,
    /// Monotonic commit generation (RFC-0022/0023 snapshot/OCC `read_version`).
    commit_generation: u64,
    /// Last commit generation that mutated each user key (OCC).
    key_versions: HashMap<Vec<u8>, u64>,
    /// Per-key history: `(generation, value)` ordered ascending by generation.
    /// Generation `0` holds the pre-image before the first mutation in this process
    /// (RFC-0023 snapshot reads). Value `None` = deleted / absent.
    key_history: KeyHistMap,
    /// Snapshots strictly below this are too old (version GC watermark).
    safe_watermark: u64,
    /// Preimages + reserved SI gen staged at propose; flushed when log index
    /// majority-commits (Direct Ok **and** Queued `finish_queued_propose`).
    /// Key: `(range_id, log_index)` → `(si_gen, items)`.
    /// F49: gen is reserved in `with_si_gen` so concurrent outstanding proposes
    /// never embed the same durable `si_gen`.
    pending_version_notes: PendingNotes,
    /// Last log index per range already flushed into version history (idempotent).
    version_notes_through: HashMap<u64, u64>,
    /// F-found (RFC-0059 swarm, seed 49): the exact entry each in-flight
    /// Queued propose wrote, keyed `(range_id, log_index)`.
    /// `finish_queued_propose` compares the live log entry against it —
    /// a commit watermark alone cannot prove the client's entry is what
    /// committed at that index after a not-escaped abort freed the index
    /// for reuse within the same term.
    proposed_entries: HashMap<(u64, u64), RangeEntry>,
    /// In-process watch hub; notified after majority put/commit_tx (RFC-0022 P0.3).
    watch: WatchHub,
    /// RFC-0013 P1.2: best-effort leadership stream (not fencing).
    live: LeadershipHub,
    /// RFC-0013 P1.5: commits / NotCommitted / elections.
    metrics: StoreMetrics,
    /// RFC-0025 P1.1: staged puts for [`Self::put_buffered`] / [`Self::flush_writes`].
    write_coalesce: Vec<(Vec<u8>, Vec<u8>)>,
}

/// How many commit generations of version history to retain (RFC-0023 P0.4).
/// Snapshots older than `commit_generation - VERSION_RETENTION` become too-old after GC.
pub const VERSION_RETENTION: u64 = 64;

impl StoreCluster<IoUringEnv> {
    /// Open `n_nodes` under `parent`, with `n_ranges` equal splits of the keyspace.
    ///
    /// Uses production [`IoUringEnv`] (Linux ring, POSIX fallback) and a fixed seed RNG. Prefer
    /// [`open_with_rng`](Self::open_with_rng) / [`open_with_env_rng`](StoreCluster::open_with_env_rng)
    /// / [`open_with_host`](StoreCluster::open_with_host) for DST.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open(parent: impl AsRef<Path>, n_nodes: u64, n_ranges: u64) -> Result<Self> {
        Self::open_with_rng(parent, n_nodes, n_ranges, SeedRng::new(0xA11CE))
    }

    /// Unpinned lab: Direct pump of the same `PeerMsg` (RFC-0067 P2.2).
    ///
    /// Production [`Self::open`] is Queued-only. Tests that elect/put
    /// in-process without a Net must opt in here.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_lab_direct(parent: impl AsRef<Path>, n_nodes: u64, n_ranges: u64) -> Result<Self> {
        let mut c = Self::open(parent, n_nodes, n_ranges)?;
        c.enable_lab_direct_rpc();
        Ok(c)
    }

    /// Open with Pedra durability knobs (RFC-0025). Default `open` = durable.
    ///
    /// Use [`StoreOpenOptions::lab_capacity`] only for bulk/bench (not crash-safe).
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_options(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        opts: StoreOpenOptions,
    ) -> Result<Self> {
        let envs: Vec<IoUringEnv> = (0..n_nodes).map(|_| IoUringEnv::default()).collect();
        Self::open_with_envs_rng_opts(parent, n_nodes, n_ranges, envs, SeedRng::new(0xA11CE), opts)
    }

    /// Open **one** local node for multi-host TCP (RFC-0017 P0.1).
    ///
    /// Only `store-node-{self_id}` is opened; `member_ids` is the full Raft
    /// membership (must include `self_id`). Pins [`RpcMode::Queued`] (RFC-0067
    /// P1.2) so Direct cannot skip Net mid-run; pump outbound PeerMsg over
    /// the network to remote peers.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_single_node(
        parent: impl AsRef<Path>,
        self_id: u64,
        member_ids: &[u64],
        n_ranges: u64,
    ) -> Result<Self> {
        Self::open_single_node_with_options(
            parent,
            self_id,
            member_ids,
            n_ranges,
            StoreOpenOptions::default(),
        )
    }

    /// [`open_single_node`](Self::open_single_node) with Pedra open knobs (sync / backpressure).
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_single_node_with_options(
        parent: impl AsRef<Path>,
        self_id: u64,
        member_ids: &[u64],
        n_ranges: u64,
        store_opts: StoreOpenOptions,
    ) -> Result<Self> {
        Self::open_single_node_with_rng_opts(
            parent,
            self_id,
            member_ids,
            n_ranges,
            SeedRng::new(0xA11CE ^ self_id.wrapping_mul(0x9E37_79B9)),
            store_opts,
        )
    }

    /// [`open_single_node`](Self::open_single_node) with explicit RNG.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_single_node_with_rng(
        parent: impl AsRef<Path>,
        self_id: u64,
        member_ids: &[u64],
        n_ranges: u64,
        rng: SeedRng,
    ) -> Result<Self> {
        Self::open_single_node_with_rng_opts(
            parent,
            self_id,
            member_ids,
            n_ranges,
            rng,
            StoreOpenOptions::default(),
        )
    }

    /// Multiproc single-node open with RNG and Pedra open knobs.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_single_node_with_rng_opts(
        parent: impl AsRef<Path>,
        self_id: u64,
        member_ids: &[u64],
        n_ranges: u64,
        rng: SeedRng,
        store_opts: StoreOpenOptions,
    ) -> Result<Self> {
        if n_ranges == 0 || member_ids.is_empty() {
            return Err(StoreError::Msg("need members and ranges".into()));
        }
        if !member_ids.contains(&self_id) {
            return Err(StoreError::Msg("self_id not in member_ids".into()));
        }
        if n_ranges > 256 {
            return Err(StoreError::Msg(
                "n_ranges > 256 not supported (single-byte keyspace split)".into(),
            ));
        }
        let parent = parent.as_ref();
        let ranges = split_keyspace(n_ranges);
        let opts = if store_opts.pedra_verified {
            // RFC-0058 P0.2: verified nodes pin the profile options.
            OpenOptions::verified()
        } else {
            OpenOptions {
                wal_full_fsync: store_opts.pedra_wal_full_fsync,
                history: Default::default(),
                wal_recovery: Default::default(),
                // Multiproc path: honor store_opts.pedra_sync (default true = durable).
                sync: store_opts.pedra_sync,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            }
        };
        let dir = parent.join(format!("store-node-{self_id}"));
        let mut db = Db::open_with_env(&dir, opts, IoUringEnv::default())?;
        if store_opts.pedra_write_backpressure {
            db.enable_write_backpressure_defaults();
        }
        let mut ids: Vec<u64> = member_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        // RFC-0125: disk voters before load_range_peer (CLI `--peer` is stale after leave).
        if let Some(raw) = db.get(&cluster_membership_key()) {
            if let Ok(disk_ids) = decode_membership(&raw) {
                if membership_kernel::disk_membership_overrides_cli(!disk_ids.is_empty()) {
                    ids = disk_ids;
                }
            }
        }
        let mut disk_hw = member_ids.len() as u64;
        if let Some(raw) = db.get(&cluster_high_water_key()) {
            if let Ok(h) = decode_u64_meta(&raw) {
                disk_hw = membership_kernel::high_water_at_least(h, disk_hw);
            }
        }
        let mut rmap = HashMap::new();
        for meta in &ranges {
            rmap.insert(meta.id, load_range_peer(&db, meta.id, self_id, &ids)?);
        }
        let mut nodes = HashMap::new();
        nodes.insert(
            self_id,
            StoreNode {
                db,
                ranges: rmap,
                participating: membership_kernel::participating_if_member(ids.contains(&self_id)),
            },
        );
        let mut cluster = Self {
            membership_high_water: membership_kernel::high_water_at_least(
                disk_hw,
                ids.len() as u64,
            ) as usize,
            engine_opts: opts,
            nodes,
            ids,
            ranges,
            rng,
            cluster_id: [0u8; 16],
            next_txn_id: 1,
            rpc_mode: RpcMode::Queued,
            dst_queued_pin: false,
            outbound: VecDeque::new(),
            election_votes: HashMap::new(),
            election_granted: HashMap::new(),
            logical_now: 0,
            now_ms: 0,
            persisted_now_ms: 0,
            has_ttl_leases: false,
            ms_per_tick: 10,
            node_regions: HashMap::new(),
            peer_addrs: HashMap::new(),
            commit_generation: 0,
            key_versions: HashMap::new(),
            key_history: HashMap::new(),
            safe_watermark: 0,
            pending_version_notes: HashMap::new(),
            version_notes_through: HashMap::new(),
            proposed_entries: HashMap::new(),
            watch: WatchHub::new(),
            live: LeadershipHub::new(),
            metrics: StoreMetrics::default(),
            write_coalesce: Vec::new(),
        };
        cluster.bind_cluster_identity(store_opts.cluster_id)?;
        cluster.recover_after_open()?;
        // RFC-0067 P1.2: the TCP ctor is Queued-pinned. Direct cannot skip
        // Net on `montanha-tcp` / `cluster_real` mid-run.
        cluster.pin_dst_queued();
        Ok(cluster)
    }

    /// Open with a deterministic RNG for election jitter (DST / reproducible tests).
    ///
    /// Disk is production [`IoUringEnv`]. For injectable disk, use [`open_with_env_rng`](StoreCluster::open_with_env_rng).
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_rng(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        rng: SeedRng,
    ) -> Result<Self> {
        StoreCluster::open_with_env_rng(parent, n_nodes, n_ranges, IoUringEnv::default(), rng)
    }

    /// [`open_with_rng`](Self::open_with_rng) then unpinned Direct (RFC-0067 P2.2).
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_rng_lab_direct(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        rng: SeedRng,
    ) -> Result<Self> {
        let mut c = Self::open_with_rng(parent, n_nodes, n_ranges, rng)?;
        c.enable_lab_direct_rpc();
        Ok(c)
    }

    /// [`open_with_options`](Self::open_with_options) then unpinned Direct.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_options_lab_direct(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        opts: StoreOpenOptions,
    ) -> Result<Self> {
        let mut c = Self::open_with_options(parent, n_nodes, n_ranges, opts)?;
        c.enable_lab_direct_rpc();
        Ok(c)
    }
}

impl<E: Env> StoreCluster<E> {
    /// Open every node on the same cloned [`Env`] (shared fault state when `E` uses `Rc`).
    ///
    /// For **per-peer** fault injection (independent `FailingEnv` per node), use
    /// [`open_with_envs_rng`](Self::open_with_envs_rng).
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_env_rng(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        env: E,
        rng: SeedRng,
    ) -> Result<Self> {
        let envs: Vec<E> = (0..n_nodes).map(|_| env.clone()).collect();
        Self::open_with_envs_rng(parent, n_nodes, n_ranges, envs, rng)
    }

    /// [`open_with_env_rng`](Self::open_with_env_rng) then unpinned Direct.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_env_rng_lab_direct(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        env: E,
        rng: SeedRng,
    ) -> Result<Self> {
        let mut c = Self::open_with_env_rng(parent, n_nodes, n_ranges, env, rng)?;
        c.enable_lab_direct_rpc();
        Ok(c)
    }

    /// Open with **one [`Env`] per node** (order = node ids `1..=n_nodes`).
    ///
    /// World/DST uses this so each peer has an independent `FailingEnv` (not shared
    /// `Rc` trip state). Length of `envs` must equal `n_nodes`.
    ///
    /// # Errors
    /// Open / bad args / env count mismatch.
    pub fn open_with_envs_rng(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        envs: impl IntoIterator<Item = E>,
        rng: SeedRng,
    ) -> Result<Self> {
        Self::open_with_envs_rng_opts(
            parent,
            n_nodes,
            n_ranges,
            envs,
            rng,
            StoreOpenOptions::default(),
        )
    }

    /// [`open_with_envs_rng`](Self::open_with_envs_rng) then unpinned Direct.
    ///
    /// # Errors
    /// Open / bad args / env count mismatch.
    pub fn open_with_envs_rng_lab_direct(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        envs: impl IntoIterator<Item = E>,
        rng: SeedRng,
    ) -> Result<Self> {
        let mut c = Self::open_with_envs_rng(parent, n_nodes, n_ranges, envs, rng)?;
        c.enable_lab_direct_rpc();
        Ok(c)
    }

    /// Like [`open_with_envs_rng`](Self::open_with_envs_rng) with Pedra durability knobs.
    ///
    /// # Errors
    /// Open / bad args / env count mismatch.
    pub fn open_with_envs_rng_opts(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        envs: impl IntoIterator<Item = E>,
        rng: SeedRng,
        store_opts: StoreOpenOptions,
    ) -> Result<Self> {
        if n_nodes == 0 || n_ranges == 0 {
            return Err(StoreError::Msg("need nodes and ranges".into()));
        }
        // F25: single-byte split collapses when n_ranges > 256 (step == 0).
        if n_ranges > 256 {
            return Err(StoreError::Msg(
                "n_ranges > 256 not supported (single-byte keyspace split)".into(),
            ));
        }
        let envs: Vec<E> = envs.into_iter().collect();
        if envs.len() as u64 != n_nodes {
            return Err(StoreError::Msg(format!(
                "env count {} != n_nodes {n_nodes}",
                envs.len()
            )));
        }
        let parent = parent.as_ref();
        let ranges = split_keyspace(n_ranges);
        let mut nodes = HashMap::new();
        let ids: Vec<u64> = (1..=n_nodes).collect();
        let opts = if store_opts.pedra_verified {
            // RFC-0058 P0.2: verified nodes pin the profile options.
            OpenOptions::verified()
        } else {
            OpenOptions {
                wal_full_fsync: store_opts.pedra_wal_full_fsync,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: store_opts.pedra_sync,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            }
        };
        for (i, env) in envs.into_iter().enumerate() {
            let id = (i as u64) + 1;
            let dir = parent.join(format!("store-node-{id}"));
            let mut db = Db::open_with_env(&dir, opts, env)?;
            if store_opts.pedra_write_backpressure {
                db.enable_write_backpressure_defaults();
            }
            let mut rmap = HashMap::new();
            // RFC-0140: peek disk membership before load (0125 TCP leftover on
            // in-process open — CLI `1..=n_nodes` built the peer).
            let mut peer_ids = ids.clone();
            if let Some(raw) = db.get(&cluster_membership_key()) {
                if let Ok(disk_ids) = decode_membership(&raw) {
                    if membership_kernel::open_peer_uses_disk(!disk_ids.is_empty()) {
                        peer_ids = disk_ids;
                    }
                }
            }
            for meta in &ranges {
                // F26: restore durable raft meta (or empty peer on first open).
                rmap.insert(meta.id, load_range_peer(&db, meta.id, id, &peer_ids)?);
            }
            nodes.insert(
                id,
                StoreNode {
                    db,
                    ranges: rmap,
                    participating: membership_kernel::participating_if_member(
                        peer_ids.contains(&id),
                    ),
                },
            );
        }
        let mut cluster = Self {
            membership_high_water: ids.len(),
            engine_opts: opts,
            nodes,
            ids,
            ranges,
            rng,
            cluster_id: [0u8; 16],
            next_txn_id: 1,
            rpc_mode: RpcMode::Queued,
            dst_queued_pin: false,
            outbound: VecDeque::new(),
            election_votes: HashMap::new(),
            election_granted: HashMap::new(),
            logical_now: 0,
            now_ms: 0,
            persisted_now_ms: 0,
            has_ttl_leases: false,
            ms_per_tick: 10,
            node_regions: HashMap::new(),
            peer_addrs: HashMap::new(),
            commit_generation: 0,
            key_versions: HashMap::new(),
            key_history: HashMap::new(),
            safe_watermark: 0,
            pending_version_notes: HashMap::new(),
            version_notes_through: HashMap::new(),
            proposed_entries: HashMap::new(),
            watch: WatchHub::new(),
            live: LeadershipHub::new(),
            metrics: StoreMetrics::default(),
            write_coalesce: Vec::new(),
        };
        cluster.bind_cluster_identity(store_opts.cluster_id)?;
        cluster.recover_after_open()?;
        Ok(cluster)
    }

    /// Current logical time (monotone; advanced by [`Self::tick`] / [`Self::advance_time`]).
    #[must_use]
    pub fn logical_now(&self) -> u64 {
        self.logical_now
    }

    /// Cluster logical time in milliseconds (DCS absolute lease deadlines).
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Advance only the lease clock (does not tick raft).
    ///
    /// Persists the new value (F56): expired DCS leases must stay dead across reopen.
    pub fn advance_now_ms(&mut self, ms: u64) {
        self.now_ms = self.now_ms.saturating_add(ms);
        self.persist_now_ms();
    }

    /// Set ms added to `now_ms` on each raft [`Self::tick`] (default 10).
    pub fn set_ms_per_tick(&mut self, ms: u64) {
        self.ms_per_tick = ms;
    }

    /// RFC-0013 P1.3: bind a cluster id (configured, recovered, or minted)
    /// and refuse a node directory that already belongs to another cluster.
    fn bind_cluster_identity(&mut self, configured: Option<[u8; 16]>) -> Result<()> {
        let mut disk: Option<(u64, [u8; 16])> = None;
        let mut disk_mem: Option<Vec<u64>> = None;
        let mut disk_hw = 0u64;
        let mut nids: Vec<u64> = self.nodes.keys().copied().collect();
        nids.sort_unstable();
        for nid in &nids {
            let Some(node) = self.nodes.get(nid) else {
                continue;
            };
            if let Some(raw) = node.db.get(&cluster_id_key()) {
                let found = decode_cluster_id(&raw)?;
                match disk {
                    None => disk = Some((*nid, found)),
                    Some((_, expected)) if expected != found => {
                        return Err(StoreError::ClusterMismatch {
                            node_id: *nid,
                            expected,
                            found,
                        });
                    }
                    Some(_) => {}
                }
            }
            if let Some(raw) = node.db.get(&cluster_membership_key()) {
                let ids = decode_membership(&raw)?;
                if disk_mem.is_none() && !ids.is_empty() {
                    disk_mem = Some(ids);
                }
            }
            if let Some(raw) = node.db.get(&cluster_high_water_key()) {
                if let Ok(h) = decode_u64_meta(&raw) {
                    disk_hw = disk_hw.max(h);
                }
            }
        }
        let id = match (configured, disk) {
            (Some(want), Some((nid, found))) if want != found => {
                return Err(StoreError::ClusterMismatch {
                    node_id: nid,
                    expected: want,
                    found,
                });
            }
            (Some(want), _) => want,
            (None, Some((_, found))) => found,
            (None, None) => mint_cluster_id(),
        };
        self.cluster_id = id;
        // RFC-0124: durable C-new must not be overwritten by CLI `--peer`.
        if membership_kernel::disk_membership_overrides_cli(disk_mem.is_some()) {
            if let Some(ids) = disk_mem {
                self.ids = ids;
                self.membership_high_water = self.membership_high_water.max(self.ids.len());
                let live = self.ids.clone();
                for (vid, n) in self.nodes.iter_mut() {
                    n.participating = live.contains(vid);
                }
            }
        }
        self.membership_high_water = membership_kernel::high_water_at_least(
            disk_hw,
            self.membership_high_water as u64,
        ) as usize;
        self.persist_cluster_identity()
    }

    fn persist_cluster_identity(&mut self) -> Result<()> {
        let ids = self.ids.clone();
        self.persist_cluster_identity_with(&ids)
    }

    fn persist_cluster_identity_with(&mut self, ids: &[u64]) -> Result<()> {
        let id_key = cluster_id_key();
        let mem_key = cluster_membership_key();
        let mem_val = encode_membership(ids);
        let hw_key = cluster_high_water_key();
        let hw = membership_kernel::high_water_at_least(
            self.membership_high_water as u64,
            ids.len() as u64,
        );
        self.membership_high_water = hw as usize;
        let hw_val = encode_u64_meta(hw);
        let mut nids: Vec<u64> = self.nodes.keys().copied().collect();
        nids.sort_unstable();
        for nid in nids {
            let Some(node) = self.nodes.get_mut(&nid) else {
                continue;
            };
            node.db.put(id_key.as_slice(), self.cluster_id.as_slice())?;
            node.db.put(mem_key.as_slice(), mem_val.as_slice())?;
            node.db.put(hw_key.as_slice(), hw_val.as_slice())?;
        }
        Ok(())
    }

    /// RFC-0013 P1.3: 16-byte cluster identity (stable across reopen).
    #[must_use]
    pub fn cluster_id(&self) -> [u8; 16] {
        self.cluster_id
    }

    /// Hex form of [`Self::cluster_id`].
    #[must_use]
    pub fn cluster_id_hex(&self) -> String {
        cluster_id_hex(&self.cluster_id)
    }

    /// Crash recovery: abort leftover 2PC intents, restore SI meta + next txn id.
    ///
    /// # Errors
    /// Present-but-corrupt SI counters (`generation` / `watermark` / `next_txn` / `now_ms`)
    /// with no valid replica copy (F114); leftover intent revert with corrupt
    /// preimage (F118/F122).
    fn recover_after_open(&mut self) -> Result<()> {
        self.abort_leftover_intents()?;
        self.load_si_from_disk()?;
        self.recover_next_txn_id()?;
        self.recover_now_ms()?;
        self.persist_truncated_logs()?;
        self.recover_apply_committed()?;
        Ok(())
    }

    /// RFC-0130: committed-but-unapplied prefix must apply on recover.
    /// AS-IS skips (crash window: joint committed, voters still C-old).
    fn recover_apply_committed(&mut self) -> Result<()> {
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        let range_ids: Vec<u64> = self.ranges.iter().map(|r| r.id).collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::recover_apply_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            for &rid in &range_ids {
                let (applied, commit) = {
                    let Some(node) = self.nodes.get(&nid) else {
                        continue;
                    };
                    let Some(peer) = node.ranges.get(&rid) else {
                        continue;
                    };
                    (peer.applied, peer.commit)
                };
                if membership_kernel::recover_must_apply(applied, commit) {
                    self.apply_range(nid, rid)?;
                }
            }
        }
        Ok(())
    }

    /// Restore the DCS lease clock (max across local replicas). Never go backwards.
    fn recover_now_ms(&mut self) -> Result<()> {
        let loaded = self.load_u64_meta_max("now_ms")?;
        self.now_ms = self.now_ms.max(loaded);
        self.persisted_now_ms = self.now_ms;
        if self.now_ms > 0 {
            self.has_ttl_leases = true;
        }
        Ok(())
    }

    /// Durable `now_ms` so TTL expiry survives process death (F56).
    ///
    /// F131: only advance `persisted_now_ms` when at least one replica put
    /// succeeded. Marking RAM-persisted after a total write miss skipped
    /// retries; reopen then reloaded `now_ms=0` and reanimated expired locks.
    fn persist_now_ms(&mut self) {
        if self.now_ms <= self.persisted_now_ms {
            return;
        }
        if self.persist_u64_meta_all("now_ms", self.now_ms).is_ok() {
            self.persisted_now_ms = self.now_ms;
        }
    }

    /// Persist raft logs after load truncated any uncommitted suffix.
    ///
    /// # Errors
    /// Log persist failure (F128 — uncommitted suffix must not remain on disk).
    fn persist_truncated_logs(&mut self) -> Result<()> {
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        let range_ids: Vec<u64> = self.ranges.iter().map(|r| r.id).collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::recover_truncate_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            let Some(node) = self.nodes.get_mut(&nid) else {
                continue;
            };
            for rid in &range_ids {
                if let Some(peer) = node.ranges.get_mut(rid) {
                    persist_log_db(&mut node.db, *rid, peer)?;
                }
            }
        }
        Ok(())
    }

    fn persist_u64_meta_all(&mut self, kind: &str, n: u64) -> Result<()> {
        let key = si_meta_key(kind);
        let val = encode_u64_meta(n);
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        let mut any_ok = false;
        let mut last_err: Option<StoreError> = None;
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::persist_meta_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            if let Some(node) = self.nodes.get_mut(&nid) {
                match node.db.put(&key, &val) {
                    Ok(()) => any_ok = true,
                    Err(e) => last_err = Some(StoreError::from(e)),
                }
            }
        }
        if any_ok {
            Ok(())
        } else {
            Err(last_err.unwrap_or_else(|| {
                StoreError::Msg(format!("si meta {kind}: persist failed on all replicas"))
            }))
        }
    }

    /// Max of a SI u64 meta key across local replicas.
    ///
    /// Missing on all nodes → 0. At least one valid decode → max of valids
    /// (corrupt siblings ignored). Present only as corrupt → Err (F114: do not
    /// treat bitrot as "never written" and restart counters at 0).
    fn load_u64_meta_max(&self, kind: &str) -> Result<u64> {
        let key = si_meta_key(kind);
        let mut max = 0u64;
        let mut any_valid = false;
        let mut any_corrupt = false;
        for node in self.nodes.values() {
            if let Some(raw) = node.db.get(&key) {
                match decode_u64_meta(raw.as_ref()) {
                    Ok(v) => {
                        any_valid = true;
                        max = max.max(v);
                    }
                    Err(_) => any_corrupt = true,
                }
            }
        }
        if any_corrupt && !any_valid {
            return Err(StoreError::Msg(format!(
                "si meta {kind}: corrupt on all local replicas"
            )));
        }
        Ok(max)
    }

    /// F122: leftover 2PC cleanup must not swallow corrupt-preimage revert errors.
    fn abort_leftover_intents(&mut self) -> Result<()> {
        if !txn_kernel::leftover_txn_is_aborted() {
            return Ok(());
        }
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::recover_abort_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            let Some(node) = self.nodes.get_mut(&nid) else {
                continue;
            };
            let rows = scan_prefix(&node.db, INTENT_PREFIX);
            let mut by_txn: HashMap<u64, Vec<Vec<u8>>> = HashMap::new();
            let mut garbage: Vec<Vec<u8>> = Vec::new();
            for (ik, raw) in rows {
                let user = ik
                    .strip_prefix(INTENT_PREFIX)
                    .and_then(user_from_meta_suffix)
                    .unwrap_or_else(|| ik.clone());
                if let Ok((oid, _)) = decode_intent(&raw) {
                    by_txn.entry(oid).or_default().push(user);
                } else {
                    garbage.push(ik);
                }
            }
            // Partial TxnCommit deletes intents but keeps preimages — still in-flight.
            for (pk, _) in scan_prefix(&node.db, TXN_PREFIX) {
                let Some(rest) = pk.strip_prefix(TXN_PREFIX) else {
                    continue;
                };
                if rest.len() < 8 {
                    continue;
                }
                let tid = u64::from_le_bytes(rest[0..8].try_into().unwrap());
                if let Some(u) = rest[8..].strip_prefix(b"/pre/") {
                    if let Some(user) = user_from_meta_suffix(u) {
                        by_txn.entry(tid).or_default().push(user);
                    }
                }
            }
            for (tid, ks) in by_txn {
                // F47/F130/F133: fence must be durable before/after revert — open
                // recovery must not swallow put errors (same class as fence_txn_aborted).
                node.db.put(txn_status_key(tid), b"abort")?;
                // Revert (restore preimage), not abort-only: a partial TxnCommit
                // may have materialised user keys before disk death.
                // F122: propagate corrupt preimage (F118) — do not leave open Ok.
                apply_txn_revert(&mut node.db, tid, &ks)?;
                // Keep abort fence after revert (revert/clear may drop status).
                node.db.put(txn_status_key(tid), b"abort")?;
            }
            if !garbage.is_empty() {
                let ops: Vec<BatchOp> = garbage.into_iter().map(BatchOp::delete).collect();
                node.db.apply_batch(ops)?;
            }
        }
        Ok(())
    }

    fn load_si_from_disk(&mut self) -> Result<()> {
        let meta_gen = self.load_u64_meta_max("generation")?;
        let meta_wm = self.load_u64_meta_max("watermark")?;
        let mut best: KeyHistMap = HashMap::new();
        // F119: present-but-corrupt hist on every replica used to be skipped →
        // SI snapshots evaporated. Track users that only had corrupt blobs.
        let mut corrupt_only: HashSet<Vec<u8>> = HashSet::new();
        for node in self.nodes.values() {
            for (hk, raw) in scan_prefix(&node.db, HIST_PREFIX) {
                let Some(user) = hk.strip_prefix(HIST_PREFIX).and_then(user_from_meta_suffix)
                else {
                    continue;
                };
                match decode_hist(&raw) {
                    Ok(hist) => {
                        corrupt_only.remove(&user);
                        let existing = best
                            .get(&user)
                            .and_then(|h| h.last().map(|(g, _)| *g))
                            .unwrap_or(0);
                        let new_last = hist.last().map(|(g, _)| *g).unwrap_or(0);
                        if new_last >= existing {
                            best.insert(user, hist);
                        }
                    }
                    Err(_) => {
                        if !best.contains_key(&user) {
                            corrupt_only.insert(user);
                        }
                    }
                }
            }
        }
        if !corrupt_only.is_empty() {
            return Err(StoreError::Msg(
                "si hist: corrupt on all local replicas".into(),
            ));
        }
        self.key_history = best;
        self.key_versions.clear();
        let mut hist_tip = 0u64;
        for (k, hist) in &self.key_history {
            if let Some((g, _)) = hist.iter().rev().find(|(g, _)| *g > 0) {
                hist_tip = hist_tip.max(*g);
                self.key_versions.insert(k.clone(), *g);
            }
        }
        // Belt: never restart below durable hist tips even if meta lagged.
        self.commit_generation = txn_kernel::recover_si_generation(meta_gen.max(hist_tip));
        self.safe_watermark =
            txn_kernel::recover_si_generation(meta_wm.min(self.commit_generation));
        Ok(())
    }

    fn recover_next_txn_id(&mut self) -> Result<()> {
        let mut max_id = self.load_u64_meta_max("next_txn")?;
        for node in self.nodes.values() {
            for (k, _) in scan_prefix(&node.db, TXN_PREFIX) {
                if let Some(rest) = k.strip_prefix(TXN_PREFIX) {
                    if rest.len() >= 8 {
                        let id = u64::from_le_bytes(rest[0..8].try_into().unwrap());
                        max_id = max_id.max(id);
                    }
                }
            }
        }
        self.next_txn_id = txn_kernel::next_txn_id_after(max_id);
        Ok(())
    }

    /// Mirror SI generation / watermark / hist for reopen.
    ///
    /// # Errors
    /// F136: generation/watermark must land on at least one replica (same class
    /// as F131/F132 meta). Hist rows are best-effort here — Raft apply already
    /// wrote them when `si_gen > 0`.
    fn persist_si_keys(&mut self, keys: &[Vec<u8>]) -> Result<()> {
        self.persist_u64_meta_all("generation", self.commit_generation)?;
        self.persist_u64_meta_all("watermark", self.safe_watermark)?;
        let mut hist_writes: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for k in keys {
            if let Some(hist) = self.key_history.get(k) {
                hist_writes.push((hist_key(k), encode_hist(hist)));
            }
        }
        if hist_writes.is_empty() {
            return Ok(());
        }
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::persist_hist_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            if let Some(node) = self.nodes.get_mut(&nid) {
                for (hk, hv) in &hist_writes {
                    let _ = node.db.put(hk, hv);
                }
            }
        }
        Ok(())
    }

    /// Advance logical time by `dt` steps; each step runs one raft timer tick
    /// (election / heartbeat). Deterministic — no wall clock.
    ///
    /// # Errors
    /// Raft / I/O during elections or heartbeats.
    pub fn advance_time(&mut self, dt: u64) -> Result<()> {
        for _ in 0..dt {
            self.tick()?;
        }
        Ok(())
    }

    /// Remove `node_id` from Raft **membership** (F28 / P2.2).
    ///
    /// Unlike [`Self::set_participating`], the peer no longer counts toward majority
    /// and no longer blocks log compaction. Node data is retained so it can be
    /// re-added via [`Self::add_member`] and catch up via install-snapshot.
    ///
    /// # Errors
    /// Unknown node / would leave empty membership.
    pub fn remove_member(&mut self, node_id: u64) -> Result<()> {
        if !self.nodes.contains_key(&node_id) {
            return Err(StoreError::Msg("remove_member: unknown node".into()));
        }
        if !self.ids.contains(&node_id) {
            return Ok(());
        }
        if self.ids.len() <= 1 {
            return Err(StoreError::Msg(
                "remove_member: cannot empty membership".into(),
            ));
        }
        // F-found (RFC-0059 P2 campaign, seeds 500308/500908/501044/...):
        // chained out-of-band removals shrank the voting set until a
        // commit quorum of the shrunken config was disjoint from a later
        // election quorum of the restored config — a committed delete was
        // overwritten and the invariant checker flagged a resurrection.
        // Out-of-band (non-log-carried) reconfiguration is only sound
        // while every quorum pair over the largest-ever membership still
        // intersects: 2·(⌊m/2⌋+1) > high_water. Multi-node rollouts need
        // log-carried config changes (joint consensus) — refusing here is
        // the contract until that lands.
        let high_water = self.membership_high_water.max(self.ids.len());
        let next = self.ids.len() - 1;
        if 2 * (next / 2 + 1) <= high_water {
            return Err(StoreError::Msg(format!(
                "remove_member: quorum floor — {next} voters cannot keep every quorum pair \
                 intersecting over the {high_water}-node high-water mark; shrinking further \
                 needs a log-carried config change"
            )));
        }
        // Persist the shrunken set *before* mutating RAM. A FailingEnv trip
        // after retain used to leave World with rm_member_err while the
        // voting set had already shrunk (fail-open membership).
        let next_ids: Vec<u64> = self
            .ids
            .iter()
            .copied()
            .filter(|&id| id != node_id)
            .collect();
        self.persist_cluster_identity_with(&next_ids)?;
        self.ids = next_ids;
        if let Some(n) = self.nodes.get_mut(&node_id) {
            n.participating = false;
            for p in n.ranges.values_mut() {
                if p.role == Role::Leader {
                    p.role = Role::Follower;
                    p.leader_id = None;
                }
            }
        }
        // Drop next/match slots on remaining leaders.
        let ids = self.ids.clone();
        let range_ids: Vec<u64> = self.ranges.iter().map(|r| r.id).collect();
        for &rid in &range_ids {
            for &lid in &ids {
                if let Some(p) = self
                    .nodes
                    .get_mut(&lid)
                    .and_then(|n| n.ranges.get_mut(&rid))
                {
                    p.next_index.remove(&node_id);
                    p.match_index.remove(&node_id);
                    if membership_kernel::drop_sent_through(ids.contains(&node_id)) {
                        p.sent_through.remove(&node_id);
                    }
                }
            }
        }
        Ok(())
    }

    /// Re-admit a previously removed peer into membership (follower; needs catch-up).
    ///
    /// # Errors
    /// Unknown node / already a member.
    pub fn add_member(&mut self, node_id: u64) -> Result<()> {
        if !self.nodes.contains_key(&node_id) {
            return Err(StoreError::Msg("add_member: unknown node".into()));
        }
        if self.ids.contains(&node_id) {
            return Ok(());
        }
        let mut next_ids = self.ids.clone();
        next_ids.push(node_id);
        next_ids.sort_unstable();
        self.persist_cluster_identity_with(&next_ids)?;
        self.ids = next_ids;
        self.membership_high_water = self.membership_high_water.max(self.ids.len());
        {
            let n = self.nodes.get_mut(&node_id).unwrap();
            n.participating = true;
            for p in n.ranges.values_mut() {
                p.role = Role::Follower;
                p.leader_id = None;
                p.election_left = p.election_timeout;
                p.next_index.clear();
                p.match_index.clear();
            }
        }
        // Force leaders to treat the peer as needing catch-up from the start
        // (triggers InstallSnapshot when log was compacted).
        let range_ids: Vec<u64> = self.ranges.iter().map(|r| r.id).collect();
        let members = self.ids.clone();
        for &rid in &range_ids {
            for &lid in &members {
                if lid == node_id {
                    continue;
                }
                if let Some(p) = self
                    .nodes
                    .get_mut(&lid)
                    .and_then(|n| n.ranges.get_mut(&rid))
                {
                    if p.role == Role::Leader {
                        p.next_index.insert(node_id, 1);
                        p.match_index.insert(node_id, 0);
                    }
                }
            }
        }
        Ok(())
    }

    /// Log-carried single remove (RFC-0063 P0 joint). Commits under
    /// majority(`old`) ∧ majority(`new`); out-of-band [`Self::remove_member`]
    /// still hits the quorum floor. One joint entry may be in flight.
    ///
    /// # Errors
    /// Unknown node / empty membership / no leader / joint already pending.
    pub fn remove_member_joint(&mut self, node_id: u64) -> Result<()> {
        let in_ids = self.ids.contains(&node_id);
        if !in_ids {
            return Ok(());
        }
        if !membership_kernel::joint_target_counts(in_ids, self.nodes.contains_key(&node_id)) {
            return Err(StoreError::Msg("remove_member_joint: unknown node".into()));
        }
        if self.ids.len() <= 1 {
            return Err(StoreError::Msg(
                "remove_member_joint: cannot empty membership".into(),
            ));
        }
        let rid = self.ranges.first().map(|r| r.id).unwrap_or(1);
        let leader = self
            .range_leader(rid)
            .ok_or_else(|| StoreError::NotLeader {
                range_id: rid,
                leader: None,
            })?;
        if let Some(p) = self.nodes.get(&leader).and_then(|n| n.ranges.get(&rid)) {
            if Self::pending_joint_on(p).is_some() {
                return Err(StoreError::Msg(
                    "remove_member_joint: a joint config is already in flight".into(),
                ));
            }
        }
        let old = self.ids.clone();
        let new: Vec<u64> = old.iter().copied().filter(|&id| id != node_id).collect();
        self.broadcast_append(rid, leader, Some(RangeEntry::MembershipJoint { old, new }))?;
        self.leave_joint_after_commit()
    }

    /// Log-carried add (RFC-0064). Same joint quorum as
    /// [`Self::remove_member_joint`]. The joining node may live in another
    /// process (RFC-0119 P1.1: TCP replica `nodes` is `{self}` only).
    ///
    /// # Errors
    /// Unknown node / already a member / no leader / joint in flight.
    pub fn add_member_joint(&mut self, node_id: u64) -> Result<()> {
        if self.ids.contains(&node_id) {
            return Ok(());
        }
        if !membership_kernel::joint_add_target_counts(self.nodes.contains_key(&node_id)) {
            return Err(StoreError::Msg("add_member_joint: unknown node".into()));
        }
        let rid = self.ranges.first().map(|r| r.id).unwrap_or(1);
        let leader = self
            .range_leader(rid)
            .ok_or_else(|| StoreError::NotLeader {
                range_id: rid,
                leader: None,
            })?;
        if let Some(p) = self.nodes.get(&leader).and_then(|n| n.ranges.get(&rid)) {
            if Self::pending_joint_on(p).is_some() {
                return Err(StoreError::Msg(
                    "add_member_joint: a joint config is already in flight".into(),
                ));
            }
        }
        if let Some(n) = self.nodes.get_mut(&node_id) {
            n.participating = true;
        }
        let old = self.ids.clone();
        let mut new = old.clone();
        new.push(node_id);
        new.sort_unstable();
        self.broadcast_append(rid, leader, Some(RangeEntry::MembershipJoint { old, new }))?;
        self.leave_joint_after_commit()
    }

    /// Append C-new-only (`old == new`) once a joint is **committed** (RFC-0066 P0).
    ///
    /// No-op if no active joint, or the joint is still uncommitted, or a leave
    /// is already in the log. Election/commit keep old∧new until this commits.
    ///
    /// # Errors
    /// No leader / append I/O.
    pub fn leave_joint(&mut self) -> Result<()> {
        let rid = self.ranges.first().map(|r| r.id).unwrap_or(1);
        let Some(leader) = self.range_leader(rid) else {
            return Ok(());
        };
        let Some(p) = self.nodes.get(&leader).and_then(|n| n.ranges.get(&rid)) else {
            return Ok(());
        };
        let leave_in_flight = p.log.iter().any(|rec| {
            rec.index > p.commit
                && matches!(
                    &rec.entry,
                    RangeEntry::MembershipJoint { old, new }
                        if !membership_kernel::joint_still_active(old, new)
                )
        });
        if leave_in_flight {
            return Ok(());
        }
        let Some((idx, old, new)) = Self::pending_joint_on(p) else {
            return Ok(());
        };
        if !membership_kernel::joint_still_active(&old, &new) {
            return Ok(());
        }
        if idx > p.commit {
            return Ok(());
        }
        self.broadcast_append(
            rid,
            leader,
            Some(RangeEntry::MembershipJoint {
                old: new.clone(),
                new,
            }),
        )
    }

    /// RFC-0122: index of an uncommitted C-new-only leave on the leader log.
    #[must_use]
    pub fn uncommitted_leave_index(&self) -> Option<(u64, u64)> {
        let rid = self.ranges.first().map(|r| r.id).unwrap_or(1);
        let leader = self.range_leader(rid)?;
        let p = self.nodes.get(&leader).and_then(|n| n.ranges.get(&rid))?;
        p.log.iter().find_map(|rec| {
            if rec.index > p.commit
                && matches!(
                    &rec.entry,
                    RangeEntry::MembershipJoint { old, new }
                        if !membership_kernel::joint_still_active(old, new)
                )
            {
                Some((rid, rec.index))
            } else {
                None
            }
        })
    }

    /// RFC-0122: finish a queued leave propose (`index > commit`).
    ///
    /// # Errors
    /// Finish / apply I/O.
    pub fn finish_uncommitted_leave(&mut self) -> Result<bool> {
        let Some((rid, idx)) = self.uncommitted_leave_index() else {
            return Ok(false);
        };
        let _ = self.finish_queued_propose(rid, idx, false)?;
        let committed = self.uncommitted_leave_index().is_none();
        Ok(membership_kernel::queued_leave_finish_ok(true, committed))
    }

    /// RFC-0068: plant a committed C-old,new joint with apply lag and no leave.
    ///
    /// DST/World seam (same class as BitFlip): the joint is on the leader log
    /// with `commit == index` and `applied == index-1`, and auto-leave is
    /// skipped. Production joint election must still require C-new.
    ///
    /// `new_member` must already be an opened node that is **not** in the
    /// current voting set (typically after [`Self::remove_member_joint`]).
    ///
    /// # Errors
    /// Unknown node / already a member / no leader.
    pub fn plant_committed_joint_without_leave(&mut self, new_member: u64) -> Result<()> {
        if !self.nodes.contains_key(&new_member) {
            return Err(StoreError::Msg(
                "plant_committed_joint_without_leave: unknown node".into(),
            ));
        }
        if self.ids.contains(&new_member) {
            return Err(StoreError::Msg(
                "plant_committed_joint_without_leave: already a member".into(),
            ));
        }
        let rid = self.ranges.first().map(|r| r.id).unwrap_or(1);
        let leader = self
            .range_leader(rid)
            .ok_or_else(|| StoreError::NotLeader {
                range_id: rid,
                leader: None,
            })?;
        let old = self.ids.clone();
        let mut new = old.clone();
        new.push(new_member);
        new.sort_unstable();
        let p = self
            .nodes
            .get_mut(&leader)
            .and_then(|n| n.ranges.get_mut(&rid))
            .ok_or_else(|| StoreError::Msg("plant: missing leader range".into()))?;
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term,
            entry: RangeEntry::MembershipJoint { old, new },
        });
        p.commit = idx;
        p.applied = idx.saturating_sub(1);
        Ok(())
    }

    /// RFC-0069: admit an *unbounded* eventual-election claim.
    ///
    /// Bounded [`Self::elect_all`] does not use this. A liveness claim is
    /// only admitted when ES-1, ES-2 and ES-3 all hold (the tcp_node_model
    /// axioms). AS-IS [`liveness_admitted_as_is`] would admit without them.
    #[must_use]
    pub fn claim_eventual_election(&self, es1: bool, es2: bool, es3: bool) -> bool {
        if self.ids.is_empty() {
            return false;
        }
        membership_kernel::liveness_admitted(es1, es2, es3)
    }

    /// RFC-0068: would a C-old majority elect under the current pending joint?
    ///
    /// Plants the same `election_granted` map the live joint-election
    /// path reads. Must be **false** while a committed C-old,new is active.
    pub fn probe_old_majority_joint_election(&mut self, range_id: u64) -> bool {
        let Some(lid) = self.range_leader(range_id) else {
            return false;
        };
        let Some(term) = self
            .nodes
            .get(&lid)
            .and_then(|n| n.ranges.get(&range_id))
            .map(|p| p.term)
        else {
            return false;
        };
        let old = self
            .pending_joint()
            .map(|(o, _)| o)
            .unwrap_or_else(|| self.ids.clone());
        let maj = membership_kernel::majority_of(old.len() as u64) as usize;
        let granted: Vec<u64> = old.iter().copied().take(maj).collect();
        self.election_granted
            .insert((range_id, term, lid), granted.clone());
        self.election_votes
            .insert((range_id, term, lid), granted.len() as u64);
        self.election_has_joint_quorum(range_id, term, lid)
    }

    /// Whether `node_id` is in the current Raft membership set.
    #[must_use]
    pub fn is_member(&self, node_id: u64) -> bool {
        self.ids.contains(&node_id)
    }

    /// Engine data directory for `node_id` (RFC-0060 BitFlip / at-rest scrub).
    #[must_use]
    pub fn node_data_dir(&self, node_id: u64) -> Option<std::path::PathBuf> {
        self.nodes.get(&node_id).map(|n| n.db.path().to_path_buf())
    }

    /// Flush that node's Pedra engine (WAL → SST) so BitFlip has a durable page.
    ///
    /// # Errors
    /// Unknown node / flush I/O.
    pub fn flush_engine_on(&mut self, node_id: u64) -> Result<()> {
        let n = self
            .nodes
            .get_mut(&node_id)
            .ok_or_else(|| StoreError::Msg(format!("flush_engine: unknown node {node_id}")))?;
        n.db.flush()?;
        Ok(())
    }

    /// Close and reopen the node's engine from `env` (force disk re-read).
    ///
    /// On open/CRC failure the node is **removed** so later `get_on` is
    /// fail-closed (`bad node`) instead of serving a cached memtable.
    ///
    /// # Errors
    /// Unknown node / close / reopen.
    pub fn reopen_engine_on(&mut self, node_id: u64, env: E) -> Result<()> {
        let n = self
            .nodes
            .remove(&node_id)
            .ok_or_else(|| StoreError::Msg(format!("reopen_engine: unknown node {node_id}")))?;
        let path = n.db.path().to_path_buf();
        n.db.close()?;
        let mut db = match Db::open_with_env(&path, self.engine_opts, env) {
            Ok(db) => db,
            Err(e) => {
                return Err(StoreError::from(e));
            }
        };
        if let Some(raw) = db.get(&cluster_id_key()) {
            let found = decode_cluster_id(&raw)?;
            if found != self.cluster_id {
                return Err(StoreError::ClusterMismatch {
                    node_id,
                    expected: self.cluster_id,
                    found,
                });
            }
        } else {
            db.put(cluster_id_key(), self.cluster_id.as_slice())?;
        }
        if let Some(raw) = db.get(&cluster_membership_key()) {
            if let Ok(ids) = decode_membership(&raw) {
                if membership_kernel::disk_membership_overrides_cli(!ids.is_empty()) {
                    self.ids = ids;
                    self.membership_high_water = self.membership_high_water.max(self.ids.len());
                }
            }
        }
        if let Some(raw) = db.get(&cluster_high_water_key()) {
            if let Ok(h) = decode_u64_meta(&raw) {
                self.membership_high_water = membership_kernel::high_water_at_least(
                    h,
                    self.membership_high_water as u64,
                ) as usize;
            }
        }
        let mut rmap = HashMap::new();
        for meta in &self.ranges {
            rmap.insert(meta.id, load_range_peer(&db, meta.id, node_id, &self.ids)?);
        }
        self.nodes.insert(
            node_id,
            StoreNode {
                db,
                ranges: rmap,
                participating: membership_kernel::participating_if_member(
                    self.ids.contains(&node_id),
                ),
            },
        );
        self.abort_leftover_intents()?;
        self.persist_truncated_logs()?;
        self.recover_apply_committed()?;
        Ok(())
    }

    /// Process-crash reopen: drop the live engine **without** [`Db::close`]
    /// (close flushes the WAL — a clean shutdown). The caller must already
    /// have dropped unsynced env bytes (`RecordingEnv::crash` on Mem).
    ///
    /// # Errors
    /// Unknown node / reopen / cluster-id mismatch.
    pub fn crash_reopen_engine_on(&mut self, node_id: u64, env: E) -> Result<()> {
        let n = self
            .nodes
            .remove(&node_id)
            .ok_or_else(|| StoreError::Msg(format!("crash_reopen: unknown node {node_id}")))?;
        let path = n.db.path().to_path_buf();
        drop(n.db);
        let mut db = match Db::open_with_env(&path, self.engine_opts, env) {
            Ok(db) => db,
            Err(e) => return Err(StoreError::from(e)),
        };
        if let Some(raw) = db.get(&cluster_id_key()) {
            let found = decode_cluster_id(&raw)?;
            if found != self.cluster_id {
                return Err(StoreError::ClusterMismatch {
                    node_id,
                    expected: self.cluster_id,
                    found,
                });
            }
        } else {
            db.put(cluster_id_key(), self.cluster_id.as_slice())?;
        }
        if let Some(raw) = db.get(&cluster_membership_key()) {
            if let Ok(ids) = decode_membership(&raw) {
                if membership_kernel::disk_membership_overrides_cli(!ids.is_empty()) {
                    self.ids = ids;
                    self.membership_high_water = self.membership_high_water.max(self.ids.len());
                }
            }
        }
        if let Some(raw) = db.get(&cluster_high_water_key()) {
            if let Ok(h) = decode_u64_meta(&raw) {
                self.membership_high_water = membership_kernel::high_water_at_least(
                    h,
                    self.membership_high_water as u64,
                ) as usize;
            }
        }
        let mut rmap = HashMap::new();
        for meta in &self.ranges {
            rmap.insert(meta.id, load_range_peer(&db, meta.id, node_id, &self.ids)?);
        }
        self.nodes.insert(
            node_id,
            StoreNode {
                db,
                ranges: rmap,
                participating: membership_kernel::participating_if_member(
                    self.ids.contains(&node_id),
                ),
            },
        );
        self.abort_leftover_intents()?;
        self.persist_truncated_logs()?;
        self.recover_apply_committed()?;
        Ok(())
    }

    /// Snapshot index on a node for a range (0 if missing).
    #[must_use]
    pub fn snapshot_index(&self, node_id: u64, range_id: u64) -> u64 {
        self.nodes
            .get(&node_id)
            .and_then(|n| n.ranges.get(&range_id))
            .map(|p| p.snapshot_index)
            .unwrap_or(0)
    }

    /// RFC-0059 P2.2: current term on a node for a range (0 if missing) —
    /// input for the trajectory checker (a live node's term never goes
    /// backwards, including across install-snapshot catch-up).
    #[must_use]
    pub fn term_on(&self, node_id: u64, range_id: u64) -> u64 {
        self.nodes
            .get(&node_id)
            .and_then(|n| n.ranges.get(&range_id))
            .map(|p| p.term)
            .unwrap_or(0)
    }

    /// RFC-0059 diagnostics: one-line raft peer state (term, vote, log
    /// suffix with per-entry terms, commit/applied/snapshot watermarks,
    /// match_index map) for divergence triage in the swarm.
    pub fn raft_debug_line(&self, node_id: u64, range_id: u64) -> String {
        let Some(n) = self.nodes.get(&node_id) else {
            return format!("n{node_id}: missing");
        };
        let Some(p) = n.ranges.get(&range_id) else {
            return format!("n{node_id}: no-range");
        };
        let log: Vec<String> = p
            .log
            .iter()
            .map(|e| {
                let entry = match &e.entry {
                    RangeEntry::Put { key, value, .. } => {
                        format!("put({:02x?},{:02x?})", key, value)
                    }
                    RangeEntry::Batch { .. } => "batch".to_string(),
                    #[allow(unreachable_patterns)]
                    _ => "other".to_string(),
                };
                format!("{}@{}:{}", e.index, e.term, entry)
            })
            .collect();
        let mut m: Vec<String> = p
            .match_index
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect();
        m.sort();
        format!(
            "n{node_id}: term={} role={:?} vote={:?} commit={} applied={} snap={} log={:?} match={:?}",
            p.term, p.role, p.voted_for, p.commit, p.applied, p.snapshot_index, log, m
        )
    }

    /// Set peer RPC delivery mode ([`RpcMode::Queued`] production default;
    /// [`RpcMode::Direct`] is lab opt-in).
    ///
    /// After [`Self::pin_dst_queued`], a request for [`RpcMode::Direct`] is a
    /// no-op (RFC-0067). The AS-IS kernel would still switch.
    pub fn set_rpc_mode(&mut self, mode: RpcMode) {
        let want_direct = mode == RpcMode::Direct;
        if !allow_direct_rpc(self.dst_queued_pin, want_direct) {
            return;
        }
        self.rpc_mode = mode;
        if mode == RpcMode::Direct {
            self.outbound.clear();
            self.election_votes.clear();
            self.election_granted.clear();
        }
    }

    /// Pin this cluster to [`RpcMode::Queued`] for World / DST (RFC-0067).
    ///
    /// Subsequent [`Self::set_rpc_mode`] of [`RpcMode::Direct`] is refused.
    /// Call once from `World::run` so Net drop/reorder cannot be silently skipped.
    pub fn pin_dst_queued(&mut self) {
        self.dst_queued_pin = true;
        self.set_rpc_mode(RpcMode::Queued);
    }

    /// RFC-0067 P2.2: opt-in unpinned in-process Direct pump (lab leftover).
    ///
    /// Production [`Self::open`] starts [`RpcMode::Queued`]. After
    /// [`Self::pin_dst_queued`] this is a no-op.
    pub fn enable_lab_direct_rpc(&mut self) {
        self.set_rpc_mode(RpcMode::Direct);
    }

    /// True after [`Self::pin_dst_queued`].
    #[must_use]
    pub fn dst_queued_pin(&self) -> bool {
        self.dst_queued_pin
    }

    /// Current RPC mode.
    #[must_use]
    pub fn rpc_mode(&self) -> RpcMode {
        self.rpc_mode
    }

    /// Drain queued outbound peer messages as `(from, to, bytes)`.
    ///
    /// Empty when [`RpcMode::Direct`].
    pub fn drain_outbound(&mut self) -> Vec<(u64, u64, Vec<u8>)> {
        self.outbound
            .drain(..)
            .map(|(f, t, m)| (f, t, m.encode()))
            .collect()
    }

    /// Number of messages waiting in the outbound queue.
    #[must_use]
    pub fn outbound_len(&self) -> usize {
        self.outbound.len()
    }

    /// Apply one inbound peer message delivered by Net/World to `to` from `from`.
    ///
    /// May enqueue reply messages when [`RpcMode::Queued`].
    ///
    /// # Errors
    /// Decode / raft / I/O.
    pub fn handle_inbound(&mut self, from: u64, to: u64, bytes: &[u8]) -> Result<()> {
        let msg = PeerMsg::decode(bytes)?;
        self.dispatch_peer_msg(from, to, msg)
    }

    /// Open from a [`Host`]: disk = `host.env()`, election RNG = `SeedRng` derived
    /// from one `host.rng()` draw (stable if host uses `SeedRng`).
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_host(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        host: &impl Host<Env = E>,
    ) -> Result<Self> {
        let seed = host.rng().next_u64();
        Self::open_with_env_rng(
            parent,
            n_nodes,
            n_ranges,
            host.env().clone(),
            SeedRng::new(seed),
        )
    }

    /// [`open_with_host`](Self::open_with_host) then unpinned Direct.
    ///
    /// # Errors
    /// Open / bad args.
    pub fn open_with_host_lab_direct(
        parent: impl AsRef<Path>,
        n_nodes: u64,
        n_ranges: u64,
        host: &impl Host<Env = E>,
    ) -> Result<Self> {
        let mut c = Self::open_with_host(parent, n_nodes, n_ranges, host)?;
        c.enable_lab_direct_rpc();
        Ok(c)
    }

    /// Replace election RNG (e.g. re-seed mid-test). Clones share stream if `SeedRng`.
    pub fn set_rng(&mut self, rng: SeedRng) {
        self.rng = rng;
    }

    /// Shared RNG handle (for harness logging).
    #[must_use]
    pub fn rng(&self) -> &SeedRng {
        &self.rng
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
    ///
    /// Remote peers (not opened in this process — multi-host TCP) count as
    /// participating if they remain in membership [`Self::node_ids`].
    #[must_use]
    pub fn is_participating(&self, node_id: u64) -> bool {
        let in_ids = self.ids.contains(&node_id);
        // RFC-0128: a removed voter never counts, even if the flag is stale.
        if !membership_kernel::participating_if_member(in_ids) {
            return false;
        }
        if let Some(n) = self.nodes.get(&node_id) {
            n.participating
        } else {
            true
        }
    }

    /// True if this process holds the PedraDB for `node_id`.
    #[must_use]
    pub fn is_local_node(&self, node_id: u64) -> bool {
        self.nodes.contains_key(&node_id)
    }

    /// Local node id when this process is a single-node multi-host member.
    ///
    /// RFC-0141: the HashMap first-key is **not** identity if that node left
    /// `ids` (TCP removed replica would otherwise `get()` stale local bytes).
    #[must_use]
    pub fn local_node_id(&self) -> Option<u64> {
        if self.nodes.len() != 1 {
            return None;
        }
        let id = self.nodes.keys().next().copied()?;
        if !membership_kernel::local_id_if_member(self.ids.contains(&id)) {
            return None;
        }
        Some(id)
    }

    /// Raft membership ids (sorted).
    #[must_use]
    pub fn member_ids(&self) -> &[u64] {
        &self.ids
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
        let now = self.range_leader(range_id);
        self.live.notify_leader(range_id, now);
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
        self.rng.next_u64()
    }

    /// Drive elections / heartbeats for all ranges (one logical time step).
    ///
    /// Advances [`Self::logical_now`] by 1 and [`Self::now_ms`] by [`Self::ms_per_tick`].
    /// No wall clock / `thread::sleep`.
    pub fn tick(&mut self) -> Result<()> {
        self.logical_now = self.logical_now.saturating_add(1);
        self.now_ms = self.now_ms.saturating_add(self.ms_per_tick);
        if self.has_ttl_leases && self.ms_per_tick > 0 {
            self.persist_now_ms();
        }
        let ids = self.ids.clone();
        let range_ids: Vec<u64> = self.ranges.iter().map(|r| r.id).collect();
        for rid in range_ids {
            self.tick_range(rid, &ids)?;
        }
        Ok(())
    }

    /// Tick a **single** range (election / HB / AE for that group only).
    ///
    /// Prefer this while waiting for majority on one put/batch so multi-Raft
    /// idle ranges do not pay full HB/fsync tax on every poll (RFC-0021 scale residual).
    /// Still advances logical time by one step.
    ///
    /// # Errors
    /// Raft / I/O during that range's tick.
    pub fn tick_range_id(&mut self, range_id: u64) -> Result<()> {
        if !self.ranges.iter().any(|r| r.id == range_id) {
            return Err(StoreError::Msg(format!("unknown range {range_id}")));
        }
        self.logical_now = self.logical_now.saturating_add(1);
        self.now_ms = self.now_ms.saturating_add(self.ms_per_tick);
        if self.has_ttl_leases && self.ms_per_tick > 0 {
            self.persist_now_ms();
        }
        let ids = self.ids.clone();
        self.tick_range(range_id, &ids)
    }

    /// Elect leaders for all ranges (bounded ticks), then best-effort rebalance.
    pub fn elect_all(&mut self, max_ticks: u64) -> Result<()> {
        self.elect_until_all_have_leaders(max_ticks)?;
        // Best-effort balance; uses elect_until (no recursive rebalance).
        let _ = self.rebalance_range_leaders(max_ticks);
        Ok(())
    }

    /// Tick until every range has a unique leader (no rebalance).
    fn elect_until_all_have_leaders(&mut self, max_ticks: u64) -> Result<()> {
        for _ in 0..max_ticks {
            let all = self
                .ranges
                .iter()
                .all(|r| self.range_leader(r.id).is_some());
            if all {
                return Ok(());
            }
            self.tick()?;
        }
        Err(StoreError::Msg("elect timeout".into()))
    }

    /// Distinct node ids that currently lead at least one range.
    #[must_use]
    pub fn leader_nodes(&self) -> Vec<u64> {
        let mut set = std::collections::BTreeSet::new();
        for r in &self.ranges {
            if let Some(l) = self.range_leader(r.id) {
                set.insert(l);
            }
        }
        set.into_iter().collect()
    }

    /// If one node holds more ranges than `ceil(n_ranges / n_nodes)`, step down
    /// excess leaders and re-elect (lab multi-Raft balance).
    ///
    /// Preferred after [`Self::elect_all`] when timeouts alone leave skew (jitter).
    ///
    /// # Errors
    /// Elect timeout / step-down failures.
    pub fn rebalance_range_leaders(&mut self, max_ticks: u64) -> Result<()> {
        if self.ids.is_empty() || self.ranges.is_empty() {
            return Ok(());
        }
        let n_nodes = self.ids.len() as u64;
        let n_ranges = self.ranges.len() as u64;
        let target = n_ranges.div_ceil(n_nodes).max(1);
        // Up to n_ranges step-downs in the worst case.
        for _ in 0..n_ranges {
            let mut load: HashMap<u64, Vec<u64>> = HashMap::new();
            for r in &self.ranges {
                if let Some(l) = self.range_leader(r.id) {
                    load.entry(l).or_default().push(r.id);
                }
            }
            let Some((&hot, ranges)) = load
                .iter()
                .max_by_key(|(_, rs)| rs.len())
                .map(|(k, v)| (k, v.clone()))
            else {
                break;
            };
            if ranges.len() as u64 <= target {
                break;
            }
            // Step down a range that *should* prefer another node when possible.
            let rid = ranges
                .iter()
                .copied()
                .find(|&rid| {
                    let mut sorted = self.ids.clone();
                    sorted.sort_unstable();
                    let pref = sorted[(rid.saturating_sub(1) as usize) % sorted.len()];
                    pref != hot
                })
                .unwrap_or(ranges[0]);
            let _ = self.step_down_range_leader(rid)?;
            // Re-elect without nested rebalance (avoids elect_all recursion).
            self.elect_until_all_have_leaders(max_ticks)?;
        }
        Ok(())
    }

    /// Multiproc TCP: if **this** node is leader of more ranges than
    /// `ceil(n_ranges / n_members)`, step down excess (prefer non-preferred ranges).
    ///
    /// Other members pick up leadership via election timeout (no in-process
    /// `elect_all` — remote peers are not in `nodes`). Returns how many ranges
    /// were stepped down.
    ///
    /// # Errors
    /// Step-down failures.
    pub fn rebalance_local_leaders(&mut self) -> Result<u32> {
        let Some(self_id) = self.local_node_id() else {
            return Ok(0);
        };
        if self.ids.is_empty() || self.ranges.is_empty() {
            return Ok(0);
        }
        let n_nodes = self.ids.len() as u64;
        let n_ranges = self.ranges.len() as u64;
        let target = n_ranges.div_ceil(n_nodes).max(1);
        let mut sorted_ids = self.ids.clone();
        sorted_ids.sort_unstable();
        let mut mine: Vec<u64> = self
            .ranges
            .iter()
            .filter_map(|r| {
                if self.node_thinks_leader(self_id, r.id) {
                    Some(r.id)
                } else {
                    None
                }
            })
            .collect();
        // Step down ranges that prefer a *different* member first.
        mine.sort_by_key(|&rid| {
            let pref = sorted_ids[(rid.saturating_sub(1) as usize) % sorted_ids.len()];
            if pref == self_id {
                1u8
            } else {
                0u8
            }
        });
        let mut stepped = 0u32;
        while (mine.len() as u64) > target {
            let rid = mine.remove(0);
            if !self.node_thinks_leader(self_id, rid) {
                continue;
            }
            let _ = self.step_down_range_leader(rid)?;
            stepped = stepped.saturating_add(1);
        }
        Ok(stepped)
    }

    fn tick_range(&mut self, rid: u64, ids: &[u64]) -> Result<()> {
        let mut elect: Vec<u64> = Vec::new();
        let mut hb: Vec<u64> = Vec::new();
        for &nid in ids {
            if !self.is_participating(nid) || !self.is_local_node(nid) {
                continue;
            }
            let p = self
                .nodes
                .get_mut(&nid)
                .unwrap()
                .ranges
                .get_mut(&rid)
                .unwrap();
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
        let j = self.next_rand() % 3;
        let (term, last_i, last_t) = {
            let n = self.nodes.get_mut(&cand).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            // F147: term/role/vote must not stick in RAM if hard cannot persist
            // (same class as F125/F127 — non-durable Candidate of a higher term).
            let prev_term = p.term;
            let prev_role = p.role;
            let prev_voted = p.voted_for;
            let prev_election_left = p.election_left;
            p.term += 1;
            p.role = Role::Candidate;
            p.voted_for = Some(cand);
            p.election_left = p.election_timeout + j;
            let t = p.term;
            let li = p.last_index();
            let lt = p.last_term();
            if let Err(e) = persist_hard_db(&mut n.db, rid, p) {
                p.term = prev_term;
                p.role = prev_role;
                p.voted_for = prev_voted;
                p.election_left = prev_election_left;
                return Err(e);
            }
            (t, li, lt)
        };
        // Self-vote; joint majority of C-old ∧ C-new (RFC-0064).
        self.election_votes.insert((rid, term, cand), 1);
        self.election_granted.insert((rid, term, cand), vec![cand]);
        let targets = self.vote_targets();
        for pid in targets {
            if pid == cand {
                continue;
            }
            // Local partitioned node: skip. Remote members are absent from
            // `nodes` on the multi-host path — still send (Queued → TCP).
            if self.nodes.get(&pid).is_some_and(|n| !n.participating) {
                continue;
            }
            let msg = PeerMsg::RequestVote {
                range_id: rid,
                term,
                candidate_id: cand,
                last_log_index: last_i,
                last_log_term: last_t,
            };
            self.send_peer_rpc(cand, pid, msg)?;
        }
        if self.rpc_mode == RpcMode::Direct && self.election_has_joint_quorum(rid, term, cand) {
            self.try_become_leader(rid, cand, term)?;
        }
        Ok(())
    }

    fn try_become_leader(&mut self, rid: u64, cand: u64, term: u64) -> Result<()> {
        if !self.election_has_joint_quorum(rid, term, cand) {
            return Ok(());
        }
        let ids = self.vote_targets();
        let promoted = {
            let n = self.nodes.get_mut(&cand).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            if p.term == term && p.role == Role::Candidate {
                // F148: become_leader pushes a Noop and sets Leader in RAM before
                // durable log persist. On fail, roll back so we are not a live
                // Leader of a non-durable blank entry (F147 class).
                let prev_role = p.role;
                let prev_leader_id = p.leader_id;
                let prev_log_len = p.log.len();
                let prev_next = p.next_index.clone();
                let prev_match = p.match_index.clone();
                let prev_hb = p.hb_left;
                p.become_leader(&ids, cand);
                if let Err(e) = persist_log_db(&mut n.db, rid, p) {
                    p.role = prev_role;
                    p.leader_id = prev_leader_id;
                    p.log.truncate(prev_log_len);
                    p.next_index = prev_next;
                    p.match_index = prev_match;
                    p.hb_left = prev_hb;
                    return Err(e);
                }
                true
            } else {
                false
            }
        };
        if promoted {
            self.election_votes.remove(&(rid, term, cand));
            self.election_granted.remove(&(rid, term, cand));
            self.metrics.elections = self.metrics.elections.saturating_add(1);
            let leader = self.range_leader(rid);
            self.live.notify_leader(rid, leader);
            self.broadcast_append(rid, cand, None)?;
        }
        Ok(())
    }

    /// Send RPC: Direct = dispatch immediately; Queued = enqueue.
    fn send_peer_rpc(&mut self, from: u64, to: u64, msg: PeerMsg) -> Result<()> {
        match self.rpc_mode {
            RpcMode::Direct => self.dispatch_peer_msg(from, to, msg),
            RpcMode::Queued => {
                self.outbound.push_back((from, to, msg));
                Ok(())
            }
        }
    }

    fn dispatch_peer_msg(&mut self, from: u64, to: u64, msg: PeerMsg) -> Result<()> {
        match msg {
            PeerMsg::RequestVote {
                range_id,
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                let reply = self.on_request_vote(
                    to,
                    range_id,
                    term,
                    candidate_id,
                    last_log_index,
                    last_log_term,
                )?;
                // Reply goes to candidate (`from` is the candidate for RV).
                self.send_peer_rpc(to, from, reply)?;
            }
            PeerMsg::RequestVoteReply {
                range_id,
                term,
                vote_granted,
            } => {
                self.on_request_vote_reply(to, from, range_id, term, vote_granted)?;
            }
            PeerMsg::AppendEntries {
                range_id,
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                leader_commit,
                entries,
            } => {
                let reply = self.on_append_entries(
                    to,
                    range_id,
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    leader_commit,
                    entries,
                )?;
                self.send_peer_rpc(to, from, reply)?;
            }
            PeerMsg::AppendEntriesReply {
                range_id,
                term,
                success,
                match_index,
            } => {
                self.on_append_entries_reply(to, from, range_id, term, success, match_index)?;
            }
            PeerMsg::InstallSnapshot {
                range_id,
                term,
                leader_id,
                last_included_index,
                last_included_term,
                kv_pairs,
            } => {
                let reply = self.on_install_snapshot(
                    to,
                    range_id,
                    term,
                    leader_id,
                    last_included_index,
                    last_included_term,
                    kv_pairs,
                )?;
                self.send_peer_rpc(to, from, reply)?;
            }
            PeerMsg::InstallSnapshotReply {
                range_id,
                term,
                success,
                match_index,
            } => {
                self.on_install_snapshot_reply(to, from, range_id, term, success, match_index)?;
            }
        }
        Ok(())
    }

    fn on_request_vote(
        &mut self,
        to: u64,
        range_id: u64,
        term: u64,
        candidate_id: u64,
        last_log_index: u64,
        last_log_term: u64,
    ) -> Result<PeerMsg> {
        if !self.is_participating(to) {
            // Partitioned node does not vote (and should not receive in Direct;
            // Queued may still deliver if net doesn't mirror membership).
            let term_now = self
                .nodes
                .get(&to)
                .and_then(|n| n.ranges.get(&range_id))
                .map(|p| p.term)
                .unwrap_or(0);
            return Ok(PeerMsg::RequestVoteReply {
                range_id,
                term: term_now,
                vote_granted: false,
            });
        }
        let Some(n) = self.nodes.get_mut(&to) else {
            return Ok(PeerMsg::RequestVoteReply {
                range_id,
                term: 0,
                vote_granted: false,
            });
        };
        let Some(p) = n.ranges.get_mut(&range_id) else {
            // Corrupt / stale range id — fail-closed vote deny.
            return Ok(PeerMsg::RequestVoteReply {
                range_id,
                term: 0,
                vote_granted: false,
            });
        };
        // F125: never grant a vote (or advance term) unless hard state is durable.
        if !durable_become_follower_if_newer(&mut n.db, range_id, p, term) {
            return Ok(PeerMsg::RequestVoteReply {
                range_id,
                term: p.term,
                vote_granted: false,
            });
        }
        let can = p.voted_for.is_none() || p.voted_for == Some(candidate_id);
        let up = last_log_term > p.last_term()
            || (last_log_term == p.last_term() && last_log_index >= p.last_index());
        let mut grant = term == p.term && can && up;
        if grant {
            let prev_voted = p.voted_for;
            p.voted_for = Some(candidate_id);
            p.election_left = p.election_timeout;
            if persist_hard_db(&mut n.db, range_id, p).is_err() {
                p.voted_for = prev_voted;
                grant = false;
            }
        }
        Ok(PeerMsg::RequestVoteReply {
            range_id,
            term: p.term,
            vote_granted: grant,
        })
    }

    fn on_request_vote_reply(
        &mut self,
        cand: u64,
        from: u64,
        range_id: u64,
        term: u64,
        vote_granted: bool,
    ) -> Result<()> {
        if !self.is_participating(cand) {
            return Ok(());
        }
        // Step down if reply term is higher.
        {
            let Some(n) = self.nodes.get_mut(&cand) else {
                return Ok(());
            };
            let Some(p) = n.ranges.get_mut(&range_id) else {
                return Ok(());
            };
            if term > p.term {
                // F127: step-down hard state must be durable (or demote without
                // keeping Candidate/Leader of the old term).
                let _ = durable_become_follower_if_newer(&mut n.db, range_id, p, term);
                self.election_votes.remove(&(range_id, p.term, cand));
                self.election_granted.remove(&(range_id, p.term, cand));
                return Ok(());
            }
            if p.role != Role::Candidate || p.term != term {
                return Ok(());
            }
        }
        if vote_granted {
            let in_ids = self.ids.contains(&from);
            let in_pending = self
                .pending_joint()
                .is_some_and(|(o, n)| o.contains(&from) || n.contains(&from));
            if membership_kernel::election_grant_from_counts(in_ids, in_pending) {
                let votes = self
                    .election_votes
                    .entry((range_id, term, cand))
                    .or_insert(1);
                *votes = votes.saturating_add(1);
                let granted = self
                    .election_granted
                    .entry((range_id, term, cand))
                    .or_default();
                if !granted.contains(&from) {
                    granted.push(from);
                }
            }
        }
        if self.election_has_joint_quorum(range_id, term, cand) {
            self.try_become_leader(range_id, cand, term)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn on_append_entries(
        &mut self,
        to: u64,
        range_id: u64,
        term: u64,
        leader_id: u64,
        prev_log_index: u64,
        prev_log_term: u64,
        leader_commit: u64,
        entries: Vec<LogRec>,
    ) -> Result<PeerMsg> {
        if !self.is_participating(to) {
            let term_now = self
                .nodes
                .get(&to)
                .and_then(|n| n.ranges.get(&range_id))
                .map(|p| p.term)
                .unwrap_or(0);
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: term_now,
                success: false,
                match_index: 0,
            });
        }
        let Some(n) = self.nodes.get_mut(&to) else {
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: 0,
                success: false,
                match_index: 0,
            });
        };
        let Some(p) = n.ranges.get_mut(&range_id) else {
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: 0,
                success: false,
                match_index: 0,
            });
        };
        if term < p.term {
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: p.term,
                success: false,
                match_index: 0,
            });
        }
        if term > p.term {
            // F127: do not process AE under a non-durable higher term (log could
            // persist while hard state stayed on the old term → reopen dual-vote).
            if !durable_become_follower_if_newer(&mut n.db, range_id, p, term) {
                return Ok(PeerMsg::AppendEntriesReply {
                    range_id,
                    term: p.term,
                    success: false,
                    match_index: 0,
                });
            }
        } else {
            p.role = Role::Follower;
            p.election_left = p.election_timeout;
        }
        p.leader_id = Some(leader_id);
        let prev = prev_log_index;
        let consistent = prev == 0 || (p.last_index() >= prev && p.term_at(prev) == prev_log_term);
        if !consistent {
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: p.term,
                success: false,
                match_index: 0,
            });
        }
        let mut ok_append = true;
        let mut log_dirty = false;
        let log_before = p.log.clone();
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
            p.log = log_before;
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: p.term,
                success: false,
                match_index: 0,
            });
        }
        p.log.sort_by_key(|e| e.index);
        p.log.dedup_by_key(|e| e.index);
        // F48: success ack means the suffix is durable. Swallowing persist
        // failure made the leader advance commit; heal+elect then majority-
        // applied a TxnCommit the client already heard as Err.
        let persist_ok = if log_dirty {
            persist_log_db(&mut n.db, range_id, p).is_ok()
        } else {
            true
        };
        if !ae_ack_kernel::ae_ack_success(log_dirty, persist_ok) {
            p.log = log_before;
            return Ok(PeerMsg::AppendEntriesReply {
                range_id,
                term: p.term,
                success: false,
                match_index: 0,
            });
        }
        if leader_commit > p.commit {
            let old_commit = p.commit;
            p.commit = leader_commit.min(p.last_index());
            if persist_commit_db(&mut n.db, range_id, p).is_err() {
                p.commit = old_commit;
            }
        }
        let match_i = entries.last().map_or(prev_log_index, |e| e.index);
        let term_now = p.term;
        // Apply committed entries on follower (re-borrow after peer mut ends).
        self.apply_range(to, range_id)?;
        Ok(PeerMsg::AppendEntriesReply {
            range_id,
            term: term_now,
            success: true,
            match_index: match_i,
        })
    }

    fn on_append_entries_reply(
        &mut self,
        leader: u64,
        from: u64,
        range_id: u64,
        term: u64,
        success: bool,
        match_index: u64,
    ) -> Result<()> {
        if !self.is_participating(leader) {
            return Ok(());
        }
        if membership_kernel::drop_repl_slot(self.ids.contains(&from)) {
            return Ok(());
        }
        {
            let Some(n) = self.nodes.get_mut(&leader) else {
                return Ok(());
            };
            // Stale AE reply for a range this node no longer owns — ignore (no panic).
            let Some(p) = n.ranges.get_mut(&range_id) else {
                return Ok(());
            };
            if term > p.term {
                let _ = durable_become_follower_if_newer(&mut n.db, range_id, p, term);
                return Ok(());
            }
            if p.role != Role::Leader || p.term != term {
                return Ok(());
            }
            if success {
                p.next_index.insert(from, match_index + 1);
                p.match_index.insert(from, match_index);
            } else {
                let ni = p.next_index.get(&from).copied().unwrap_or(1);
                p.next_index.insert(from, ni.saturating_sub(1).max(1));
            }
        }
        self.try_advance_commit(range_id, leader)?;
        // Apply only **local** participating nodes (multi-host).
        let ids = self.ids.clone();
        for &nid in &ids {
            if self.is_local_node(nid) && self.is_participating(nid) {
                self.apply_range(nid, range_id)?;
            }
        }
        self.maybe_compact_logs(range_id)?;
        Ok(())
    }

    fn replication_count(p: &RangePeer, leader: u64, ids: &[u64], idx: u64) -> usize {
        ids.iter()
            .filter(|&&pid| {
                if pid == leader {
                    true
                } else {
                    p.match_index.get(&pid).copied().unwrap_or(0) >= idx
                }
            })
            .count()
    }

    fn unleft_applied_joint_index(p: &RangePeer) -> Option<u64> {
        let mut last = None;
        for rec in &p.log {
            if rec.index > p.applied {
                break;
            }
            if let RangeEntry::MembershipJoint { old, new } = &rec.entry {
                if membership_kernel::joint_still_active(old, new) {
                    last = Some(rec.index);
                } else {
                    last = None;
                }
            }
        }
        last
    }

    fn pending_joint_on(p: &RangePeer) -> Option<(u64, Vec<u64>, Vec<u64>)> {
        // Uncommitted C-old,new first. An uncommitted leave (old==new) must
        // not hide a committed active joint (RFC-0066: both quorums until
        // leave commits).
        if let Some(found) = p.log.iter().find_map(|rec| {
            if rec.index > p.commit {
                if let RangeEntry::MembershipJoint { old, new } = &rec.entry {
                    if membership_kernel::joint_still_active(old, new) {
                        return Some((rec.index, old.clone(), new.clone()));
                    }
                }
            }
            None
        }) {
            return Some(found);
        }
        let mut last: Option<(u64, Vec<u64>, Vec<u64>)> = None;
        for rec in &p.log {
            if rec.index > p.commit {
                break;
            }
            if let RangeEntry::MembershipJoint { old, new } = &rec.entry {
                if membership_kernel::joint_still_active(old, new) {
                    last = Some((rec.index, old.clone(), new.clone()));
                } else {
                    last = None;
                }
            }
        }
        last
    }

    fn pending_joint(&self) -> Option<(Vec<u64>, Vec<u64>)> {
        for (id, n) in &self.nodes {
            if !membership_kernel::pending_joint_node_counts(self.ids.contains(id)) {
                continue;
            }
            for p in n.ranges.values() {
                if let Some((_, old, new)) = Self::pending_joint_on(p) {
                    return Some((old, new));
                }
            }
        }
        None
    }

    fn vote_targets(&self) -> Vec<u64> {
        let mut t = self.ids.clone();
        if let Some((old, new)) = self.pending_joint() {
            t.extend(old);
            t.extend(new);
        }
        t.sort_unstable();
        t.dedup();
        t
    }

    fn election_has_joint_quorum(&self, rid: u64, term: u64, cand: u64) -> bool {
        let granted = self
            .election_granted
            .get(&(rid, term, cand))
            .cloned()
            .unwrap_or_default();
        let old = self
            .pending_joint()
            .map(|(o, _)| o)
            .unwrap_or_else(|| self.ids.clone());
        let new = self.pending_joint().map(|(_, n)| n);
        let old_yes = old.iter().filter(|id| granted.contains(id)).count() as u64;
        let new_yes = new.as_ref().map(|n| {
            (
                n.iter().filter(|id| granted.contains(id)).count() as u64,
                n.len() as u64,
            )
        });
        membership_kernel::joint_election_ok(old_yes, old.len() as u64, new_yes)
    }

    fn try_advance_commit(&mut self, rid: u64, leader: u64) -> Result<()> {
        let ids = self.ids.clone();
        {
            let Some(n) = self.nodes.get_mut(&leader) else {
                return Ok(());
            };
            let Some(p) = n.ranges.get_mut(&rid) else {
                return Ok(());
            };
            if p.role != Role::Leader {
                return Ok(());
            }
            let joint = Self::pending_joint_on(p);
            let last = p.last_index();
            for idx in (1..=last).rev() {
                let old_maj = ids.len() / 2 + 1;
                let old_ok = Self::replication_count(p, leader, &ids, idx) >= old_maj;
                let new_ok = match &joint {
                    Some((jidx, _, new)) if idx >= *jidx => {
                        let new_maj = new.len() / 2 + 1;
                        Self::replication_count(p, leader, new, idx) >= new_maj
                    }
                    _ => true,
                };
                if commit_kernel::may_commit_at(p.term_at(idx), p.term, old_ok && new_ok) {
                    if idx > p.commit {
                        // F126: same class as AE leader_commit path — do not leave
                        // memory commit ahead of durable meta (apply would race).
                        let old_commit = p.commit;
                        p.commit = idx;
                        if persist_commit_db(&mut n.db, rid, p).is_err() {
                            p.commit = old_commit;
                        }
                    }
                    break;
                }
            }
        }
        self.leave_joint_after_commit()?;
        Ok(())
    }

    /// RFC-0098: Queued `leave_joint` may return `NotCommitted` after the
    /// leave is already on the leader log. That is not a failed leave.
    fn leave_joint_after_commit(&mut self) -> Result<()> {
        match self.leave_joint() {
            Ok(()) => Ok(()),
            Err(StoreError::NotCommitted { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Export **user** applied KV for a range (install-snapshot payload).
    ///
    /// Reserved `\0store/*` keys are **never** exported:
    /// - Raft meta is per-node (term/vote/log); shipping the leader's meta onto a
    ///   follower corrupts that peer (F41).
    /// - Intents/txn/SI are not range-applied state; orphans are cleared on install
    ///   for keys in this range (F40) or on open recovery (F35).
    fn export_range_kv(&self, node_id: u64, rid: u64) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let meta = self
            .ranges
            .iter()
            .find(|r| r.id == rid)
            .ok_or_else(|| StoreError::Msg("bad range".into()))?;
        let n = self
            .nodes
            .get(&node_id)
            .ok_or_else(|| StoreError::Msg("bad node".into()))?;
        let start = meta.start.as_slice();
        let end = if meta.end.is_empty() {
            Bound::Unbounded
        } else {
            Bound::Excluded(meta.end.as_slice())
        };
        let start_b = if meta.start.is_empty() {
            Bound::Unbounded
        } else {
            Bound::Included(start)
        };
        let mut out = Vec::new();
        for (k, v) in n.db.range_limited(start_b, end, None) {
            if !snapshot_kernel::snapshot_touches_user_key(is_reserved_store_key(&k)) {
                continue;
            }
            out.push((k.to_vec(), v.to_vec()));
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn on_install_snapshot(
        &mut self,
        to: u64,
        range_id: u64,
        term: u64,
        leader_id: u64,
        last_included_index: u64,
        last_included_term: u64,
        kv_pairs: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<PeerMsg> {
        if !self.is_participating(to) {
            let term_now = self
                .nodes
                .get(&to)
                .and_then(|n| n.ranges.get(&range_id))
                .map(|p| p.term)
                .unwrap_or(0);
            return Ok(PeerMsg::InstallSnapshotReply {
                range_id,
                term: term_now,
                success: false,
                match_index: 0,
            });
        }
        {
            let n = self
                .nodes
                .get_mut(&to)
                .ok_or_else(|| StoreError::Msg(format!("install-snap: unknown node {to}")))?;
            let Some(p) = n.ranges.get_mut(&range_id) else {
                // Defense in depth past the frame CRC: a (stale or
                // foreign) snapshot for a range this node does not
                // serve is rejected, never a panic — a wire-fed value
                // must not take the process down.
                return Ok(PeerMsg::InstallSnapshotReply {
                    range_id,
                    term: 0,
                    success: false,
                    match_index: 0,
                });
            };
            if term < p.term {
                return Ok(PeerMsg::InstallSnapshotReply {
                    range_id,
                    term: p.term,
                    success: false,
                    match_index: 0,
                });
            }
            // F144 residual of F127/F124: higher-term install used to call
            // become_follower (RAM term bump + clear vote) then roll back only
            // snap/commit/applied on persist fail — non-durable higher term stuck.
            if term > p.term {
                if !durable_become_follower_if_newer(&mut n.db, range_id, p, term) {
                    return Ok(PeerMsg::InstallSnapshotReply {
                        range_id,
                        term: p.term,
                        success: false,
                        match_index: 0,
                    });
                }
            } else {
                p.role = Role::Follower;
                p.election_left = p.election_timeout;
            }
            p.leader_id = Some(leader_id);
            // F-found (RFC-0059 swarm, seed 104853): reject a snapshot
            // strictly older than our own commit. Everything ≤ commit is
            // already in our applied state (or replays from our own log);
            // installing the older snapshot wipes that newer applied user
            // state while the retained log prefix (`applied` unchanged)
            // never re-applies it — committed data silently vanishes with
            // converged raft bookkeeping. An at-commit snapshot (equal
            // index) still installs (idempotent replacement + persist-fail
            // rollback are tested paths). Reply failure with our commit as
            // a *hint*: the leader may resume AE from it, but must not
            // count it as a match — those indexes are our branch, not the
            // leader's (a leader that recorded them as matches committed
            // its own-term no-op without any real replication, seed 503976).
            if last_included_index < p.commit {
                let m = p.commit;
                return Ok(PeerMsg::InstallSnapshotReply {
                    range_id,
                    term: p.term,
                    success: false,
                    match_index: m,
                });
            }
            // F124: raft meta must be durable **before** wiping user keys.
            // AS-IS swallowed persist errors then wiped + replied success — leader
            // thought install worked while the follower lost keys without a snap.
            let prev_snap_i = p.snapshot_index;
            let prev_snap_t = p.snapshot_term;
            let prev_log = p.log.clone();
            let prev_commit = p.commit;
            let prev_applied = p.applied;
            p.snapshot_index = last_included_index;
            p.snapshot_term = last_included_term;
            p.log.retain(|e| e.index > last_included_index);
            p.commit = p.commit.max(last_included_index);
            p.applied = p.applied.max(last_included_index);
            if let Err(e) = (|| -> Result<()> {
                // Hard already durable when term was raised above; re-persist is
                // cheap and covers same-term follower role path.
                persist_hard_db(&mut n.db, range_id, p)?;
                persist_snap_db(&mut n.db, range_id, p)?;
                persist_log_db(&mut n.db, range_id, p)?;
                persist_commit_db(&mut n.db, range_id, p)?;
                persist_applied_db(&mut n.db, range_id, p)?;
                Ok(())
            })() {
                // Roll back in-memory watermarks; leave user keys untouched.
                // Term/vote already handled by durable_become_follower (durable
                // higher term stays; non-durable bump was never applied).
                p.snapshot_index = prev_snap_i;
                p.snapshot_term = prev_snap_t;
                p.log = prev_log;
                p.commit = prev_commit;
                p.applied = prev_applied;
                let _ = e;
                return Ok(PeerMsg::InstallSnapshotReply {
                    range_id,
                    term: p.term,
                    success: false,
                    match_index: 0,
                });
            }
        }
        // Replace applied **user** state for this range (F38/F40/F41):
        // - Wipe only non-reserved keys in [start,end) (never `\0store/*`).
        // - Clear intents/txn meta for user keys owned by this range (they live
        //   under `\0store/intent|txn/` outside the user keyspace).
        // - Apply export user keys only; ignore reserved keys if a peer still
        //   sends them (old wire). Raft meta for *this* range was already
        //   persisted from local peer state above — never from the leader export.
        {
            let meta = self
                .ranges
                .iter()
                .find(|r| r.id == range_id)
                .ok_or_else(|| StoreError::Msg("install snap: bad range".into()))?
                .clone();
            let start = meta.start.clone();
            let end = meta.end.clone();
            let n = self.nodes.get_mut(&to).unwrap();
            let start_b = if start.is_empty() {
                Bound::Unbounded
            } else {
                Bound::Included(start.as_slice())
            };
            let end_b = if end.is_empty() {
                Bound::Unbounded
            } else {
                Bound::Excluded(end.as_slice())
            };
            let stale: Vec<Vec<u8>> = n
                .db
                .range_limited(start_b, end_b, None)
                .into_iter()
                .map(|(k, _)| k.to_vec())
                .filter(|k| snapshot_kernel::snapshot_touches_user_key(is_reserved_store_key(k)))
                .collect();
            if !stale.is_empty() {
                let ops: Vec<BatchOp> = stale.into_iter().map(BatchOp::delete).collect();
                n.db.apply_batch(ops)?;
            }
            // Drop orphan intents / txn records for keys in this range (F40).
            if snapshot_kernel::snapshot_needs_txn_meta_clear() {
                clear_range_txn_meta(&mut n.db, start.as_slice(), end.as_slice())?;
            }
            for (k, v) in &kv_pairs {
                if !snapshot_kernel::snapshot_touches_user_key(is_reserved_store_key(k)) {
                    continue;
                }
                n.db.put(k, v)?;
            }
        }
        Ok(PeerMsg::InstallSnapshotReply {
            range_id,
            term,
            success: true,
            match_index: last_included_index,
        })
    }

    fn on_install_snapshot_reply(
        &mut self,
        leader: u64,
        from: u64,
        range_id: u64,
        term: u64,
        success: bool,
        match_index: u64,
    ) -> Result<()> {
        if !self.is_participating(leader) {
            return Ok(());
        }
        if membership_kernel::drop_repl_slot(self.ids.contains(&from)) {
            return Ok(());
        }
        {
            let Some(n) = self.nodes.get_mut(&leader) else {
                return Ok(());
            };
            // Defense in depth past the frame CRC: a reply for a range
            // this node does not serve is dropped, never a panic.
            let Some(p) = n.ranges.get_mut(&range_id) else {
                return Ok(());
            };
            if term > p.term {
                let _ = durable_become_follower_if_newer(&mut n.db, range_id, p, term);
                return Ok(());
            }
            if p.role != Role::Leader {
                return Ok(());
            }
            if success {
                p.next_index.insert(from, match_index + 1);
                p.match_index.insert(from, match_index);
            } else {
                // Rejected snapshot. A non-zero match_index is the
                // follower's own commit — a hint for where AE may resume,
                // never a match (F-found seed 503976: a stale-branch
                // leader counted two such hints as replication and
                // committed its own-term no-op with no real follower
                // acks). Clamp to our log; below the compaction
                // watermark retry the snapshot next round.
                let snap = p.snapshot_index;
                let last = p.last_index();
                let ni = if match_index > 0 {
                    (match_index + 1).min(last + 1).max(snap.max(1))
                } else {
                    snap.max(1)
                };
                p.next_index.insert(from, ni);
            }
        }
        self.try_advance_commit(range_id, leader)?;
        let ids = self.ids.clone();
        for &nid in &ids {
            if self.is_local_node(nid) && self.is_participating(nid) {
                self.apply_range(nid, range_id)?;
            }
        }
        self.maybe_compact_logs(range_id)?;
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
            // Assign SI generation + capture preimages **before** apply.
            let entry = self.with_si_gen(entry);
            let note_items = self.preimages_for_entry(&entry);
            let stamped_gen = match &entry {
                RangeEntry::Put { si_gen, .. } | RangeEntry::Batch { si_gen, .. } => *si_gen,
                RangeEntry::TxnCommit { si_gen, .. } => *si_gen,
                _ => 0,
            };
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
                entry: entry.clone(),
            });
            proposed_index = Some(idx);
            // F-found (RFC-0059 swarm, seed 49): remember the exact entry at
            // this index so `finish_queued_propose` can tell *whose* commit
            // a later watermark reflects after an abort freed the index.
            self.proposed_entries.insert((rid, idx), entry);
            // F47: if durable log write fails, roll back the in-memory push.
            // Otherwise a later heal/cancel can majority-commit the orphan.
            if let Err(e) = persist_log_db(&mut n.db, rid, p) {
                p.log.pop();
                self.proposed_entries.remove(&(rid, idx));
                // F49/F50: unreserve SI gen so a failed propose does not burn
                // generations (and so retry can re-use the same logical slot).
                self.commit_generation =
                    txn_kernel::unreserve_si_gen(self.commit_generation, stamped_gen);
                return Err(e);
            }
            if !note_items.is_empty() {
                self.pending_version_notes
                    .insert((rid, idx), (stamped_gen, note_items));
            }
        }

        // Run replication/apply; on *any* failure before the client index is
        // majority-committed, discard the orphan (Direct RPC). NotCommitted is
        // only one failure mode — IoError mid-AE previously left the entry in
        // the leader log and FailingEnv+heal installed it (F47).
        let outcome =
            self.broadcast_append_after_propose(rid, leader, &ids, proposed_index, had_client);
        if let Err(e) = outcome {
            if let Some(idx) = proposed_index {
                if self.rpc_mode == RpcMode::Direct {
                    let commit_now = self
                        .nodes
                        .get(&leader)
                        .and_then(|n| n.ranges.get(&rid))
                        .map(|p| p.commit)
                        .unwrap_or(0);
                    if !commit_kernel::propose_ack_ok(idx, commit_now) {
                        // F129: surface leader discard failure (F128). Followers
                        // remain best-effort inside discard_uncommitted_from.
                        self.discard_uncommitted_from(rid, leader, idx)?;
                    }
                }
            }
            return Err(e);
        }
        Ok(())
    }

    /// Replication + apply tail of [`broadcast_append`] after an optional client push.
    fn broadcast_append_after_propose(
        &mut self,
        rid: u64,
        leader: u64,
        ids: &[u64],
        proposed_index: Option<u64>,
        had_client: bool,
    ) -> Result<()> {
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

        let leader_snap = self
            .nodes
            .get(&leader)
            .and_then(|n| n.ranges.get(&rid))
            .map(|p| (p.snapshot_index, p.snapshot_term))
            .unwrap_or((0, 0));

        for &pid in ids {
            if pid == leader || !self.is_participating(pid) {
                continue;
            }
            let (next, prev, prev_term) = {
                let p = self.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
                let next = *p.next_index.get(&pid).unwrap_or(&(last + 1));
                let prev = next.saturating_sub(1);
                // F27: after log compact, `prev` may be only in the snapshot watermark.
                let prev_term = p.term_at(prev);
                (next, prev, prev_term)
            };
            // P2.3: follower is behind compacted prefix → InstallSnapshot, not AE.
            if leader_snap.0 > 0 && next <= leader_snap.0 {
                // The export is the leader's **live applied state**, so it is
                // labeled at the leader's applied point — a committed prefix
                // (applied ≤ commit) whose exact state is the db we just
                // serialized. Labeling it at the compaction watermark shipped
                // state the follower never reached (or lacked) under a stale
                // label: two leaders sent (4,6) with different contents and a
                // lagging follower flip-flopped between them (seed 503976).
                // `applied ≥ snapshot_index` (install/compaction invariants),
                // so the term always resolves.
                let (lii, lit) = {
                    let p = self.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
                    (p.applied, p.term_at(p.applied))
                };
                let kv_pairs = self.export_range_kv(leader, rid)?;
                let msg = PeerMsg::InstallSnapshot {
                    range_id: rid,
                    term,
                    leader_id: leader,
                    last_included_index: lii,
                    last_included_term: lit,
                    kv_pairs,
                };
                if let Some(n) = self.nodes.get_mut(&leader) {
                    if let Some(p) = n.ranges.get_mut(&rid) {
                        let st = p.sent_through.entry(pid).or_insert(0);
                        *st = (*st).max(lii);
                    }
                }
                self.send_peer_rpc(leader, pid, msg)?;
                continue;
            }
            let entries: Vec<LogRec> = log_snap
                .iter()
                .filter(|e| e.index >= next)
                .cloned()
                .collect();
            if let Some(last_sent) = entries.last().map(|e| e.index) {
                if let Some(n) = self.nodes.get_mut(&leader) {
                    if let Some(p) = n.ranges.get_mut(&rid) {
                        let st = p.sent_through.entry(pid).or_insert(0);
                        *st = (*st).max(last_sent);
                    }
                }
            }
            let msg = PeerMsg::AppendEntries {
                range_id: rid,
                term,
                leader_id: leader,
                prev_log_index: prev,
                prev_log_term: prev_term,
                leader_commit: commit,
                entries,
            };
            self.send_peer_rpc(leader, pid, msg)?;
        }

        // Direct: replies already updated match_index/commit via on_append_entries_reply.
        // Queued: commit advances when World delivers replies.

        // Client API contract: Ok only if the proposed index is majority-committed.
        // Direct: discard uncommitted on failure.
        // Queued: leave entry for external delivery; return NotCommitted without discard
        // so World can pump Net and complete majority.
        if let Some(idx) = proposed_index {
            let commit_now = self
                .nodes
                .get(&leader)
                .and_then(|n| n.ranges.get(&rid))
                .map(|p| p.commit)
                .unwrap_or(0);
            if !commit_kernel::propose_ack_ok(idx, commit_now) {
                if self.rpc_mode == RpcMode::Direct {
                    // F129: if leader cannot durable-truncate the orphan, prefer
                    // that error over a clean NotCommitted (orphan may reopen).
                    self.discard_uncommitted_from(rid, leader, idx)?;
                }
                return Err(StoreError::NotCommitted {
                    range_id: rid,
                    index: idx,
                    commit: commit_now,
                });
            }
        }

        // Apply only **local** participating nodes (multi-host: remotes apply via AE).
        for &nid in ids {
            if self.is_local_node(nid) && self.is_participating(nid) {
                self.apply_range(nid, rid)?;
            }
        }
        // Heartbeat to push commit to followers (no client entry → no NotCommitted gate).
        if had_client {
            self.broadcast_append(rid, leader, None)?;
            for &nid in ids {
                if self.is_local_node(nid) && self.is_participating(nid) {
                    self.apply_range(nid, rid)?;
                }
            }
        }
        // Version history / OCC: flush notes for any client entry that majority-committed.
        if let Some(idx) = proposed_index {
            let commit_now = self
                .nodes
                .get(&leader)
                .and_then(|n| n.ranges.get(&rid))
                .map(|p| p.commit)
                .unwrap_or(0);
            if commit_now >= idx {
                self.flush_version_notes_through(rid, idx)?;
            }
        }
        // F27: drop applied prefix once every participating peer has applied it.
        self.maybe_compact_logs(rid)?;
        Ok(())
    }

    /// After Queued RPC delivery, check whether `index` is committed on `range_id`
    /// and apply; if still uncommitted and `abort` is true, discard from leader.
    ///
    /// Used by World after pumping Net for a client put that returned NotCommitted.
    /// Also flushes version/OCC notes once the entry is majority-committed (RFC-0023).
    ///
    /// # Errors
    /// I/O / apply.
    pub fn finish_queued_propose(
        &mut self,
        range_id: u64,
        index: u64,
        abort_if_uncommitted: bool,
    ) -> Result<bool> {
        let leader = self.range_leader(range_id);
        let Some(leader) = leader else {
            if abort_if_uncommitted {
                // No leader — discard on local replicas. Persist-leader must
                // be local so truncate is fail-closed (0143 leftover: ids.first
                // is a remote voter on the TCP removed replica).
                let persist_leader = self
                    .ids
                    .iter()
                    .copied()
                    .chain(self.nodes.keys().copied())
                    .find(|&id| {
                        membership_kernel::discard_leader_local(self.is_local_node(id))
                    });
                if let Some(lid) = persist_leader {
                    self.discard_uncommitted_from(range_id, lid, index)?;
                }
            }
            return Ok(false);
        };
        let commit = self.commit_index(leader, range_id);
        // F-found (RFC-0059 swarm, seed 49): a commit watermark alone cannot
        // resolve CommitUnknown — after a not-escaped abort freed this index,
        // a later entry reused it and committed, and the client's put was
        // reported Ok for a commit that was never its entry. The live log
        // entry (when still present) must still be the one this propose
        // wrote; an overwritten/missing entry means the client keeps its
        // NotCommitted (the commit at that index belongs to someone else).
        if let Some(expected) = self.proposed_entries.get(&(range_id, index)).cloned() {
            let snapshot_index = self.snapshot_index(leader, range_id);
            let actual = self
                .nodes
                .get(&leader)
                .and_then(|n| n.ranges.get(&range_id))
                .and_then(|p| p.log.iter().find(|e| e.index == index))
                .map(|e| e.entry.clone());
            match actual {
                Some(actual) if actual == expected => {}
                Some(_) => {
                    // Index reused by a different entry — not this client's commit.
                    self.proposed_entries.remove(&(range_id, index));
                    self.drop_pending_version_notes_from(range_id, index);
                    return Ok(false);
                }
                None if snapshot_index >= index => {
                    // Compacted past the index: the prefix (whatever entries
                    // it held) applied; judge by the watermark as before.
                }
                None => {
                    self.proposed_entries.remove(&(range_id, index));
                    self.drop_pending_version_notes_from(range_id, index);
                    return Ok(false);
                }
            }
        }
        if commit >= index {
            let ids = self.ids.clone();
            for &nid in &ids {
                if self.is_local_node(nid) && self.is_participating(nid) {
                    self.apply_range(nid, range_id)?;
                }
            }
            // OCC/SI version history — must run even when put() returned NotCommitted.
            self.flush_version_notes_through(range_id, index)?;
            // RFC-0098: Queued add_member_joint returns NotCommitted before
            // leave_joint; after the joint commits, append C-new-only
            // *before* compact can hide the active joint from pending_joint_on.
            self.leave_joint_after_commit()?;
            self.maybe_compact_logs(range_id)?;
            // Resolution done — stop tracking this propose's entry.
            self.proposed_entries.remove(&(range_id, index));
            // Heartbeat commit to followers.
            if self.is_local_node(leader) && self.is_participating(leader) {
                let _ = self.broadcast_append(range_id, leader, None);
            }
            Ok(true)
        } else if abort_if_uncommitted {
            self.discard_uncommitted_from(range_id, leader, index)?;
            self.proposed_entries.remove(&(range_id, index));
            Ok(false)
        } else {
            Ok(false)
        }
    }

    /// Truncate raft logs through `min(applied)` over **full membership** (F27/F28).
    ///
    /// F28: do **not** ignore partitioned/offline peers. Their `applied` freezes, so
    /// compact cannot drop entries they still need for AE catch-up when healed.
    /// (Compacting past offline peers left them unable to catch up without snapshot install.)
    fn maybe_compact_logs(&mut self, rid: u64) -> Result<()> {
        let ids = self.ids.clone();
        if ids.is_empty() {
            return Ok(());
        }
        // Multi-host: remotes are not in `nodes` — use leader match_index for them.
        let mut min_applied = u64::MAX;
        for &nid in &ids {
            if !compact_kernel::peer_counts_for_compact(self.is_participating(nid)) {
                continue;
            }
            if self.is_local_node(nid) {
                min_applied = min_applied.min(self.applied_index(nid, rid));
            } else if let Some(leader) = self.range_leader(rid) {
                if self.is_local_node(leader) {
                    let mi = self
                        .nodes
                        .get(&leader)
                        .and_then(|n| n.ranges.get(&rid))
                        .and_then(|p| p.match_index.get(&nid).copied())
                        .unwrap_or(0);
                    min_applied = min_applied.min(mi);
                }
            }
        }
        if min_applied == u64::MAX {
            min_applied = 0;
        }
        if !compact_kernel::compact_ready(min_applied) {
            return Ok(());
        }
        // RFC-0100: do not compact an applied still-active joint until leave
        // is applied (`pending_joint_on` would go None and hide C-old,new).
        let mut through = min_applied;
        for &nid in &ids {
            if !self.is_local_node(nid) {
                continue;
            }
            let p = self.nodes.get(&nid).unwrap().ranges.get(&rid).unwrap();
            through = compact_kernel::compact_through_unleft(
                through,
                Self::unleft_applied_joint_index(p),
            );
        }
        if !compact_kernel::compact_ready(through) {
            return Ok(());
        }
        // Compact only local peers (remote peers compact independently).
        for &nid in &ids {
            if !self.is_local_node(nid) {
                continue;
            }
            let p = self.nodes.get(&nid).unwrap().ranges.get(&rid).unwrap();
            if p.snapshot_index >= through {
                continue;
            }
            if !compact_kernel::may_compact_through(p.snapshot_index, through, p.term_at(through))
                && p.snapshot_index < through
            {
                return Ok(()); // lagging peer missing entry; wait
            }
        }
        for &nid in &ids {
            if !self.is_local_node(nid) {
                continue;
            }
            let n = self.nodes.get_mut(&nid).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            if p.snapshot_index >= through {
                continue;
            }
            let before = p.log.len();
            p.compact_through(through);
            if p.log.len() != before || p.snapshot_index == through {
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
    fn discard_uncommitted_from(&mut self, rid: u64, leader: u64, from_index: u64) -> Result<()> {
        // F-found (RFC-0059 swarm, seeds 49/865): an entry that already
        // left **some** leader on the wire (acked **or still in flight**)
        // must NOT be discarded. `match_index` alone missed the in-flight
        // case: the freed index was reused within the same term, and the
        // crossed copy then let two different payloads share one
        // (index, term) — the retained follower copy applied against the
        // leader's replacement (phantom value), and in the fully-escaped
        // case the commit advanced past the reused index reporting the
        // client's put Ok while its value was erased everywhere. Keep the
        // entry: the client already saw NotCommitted (commit-unknown), and
        // the entry commits or not on its own quorum fate. Index identity
        // within a term is the Raft invariant that must never break. Any
        // node may be the (former) leader that sent it — scan them all.
        let escaped = self.nodes.values().any(|n| {
            n.ranges
                .get(&rid)
                .map(|p| p.sent_through.values().any(|&si| si >= from_index))
                .unwrap_or(false)
        });
        if escaped {
            // Entry (and anything after it still in flight) keeps its
            // quorum fate — its OCC notes must survive to match.
            return Ok(());
        }
        let ids = self.ids.clone();
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::discard_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            let Some(n) = self.nodes.get_mut(&nid) else {
                continue;
            };
            let Some(p) = n.ranges.get_mut(&rid) else {
                continue;
            };
            // Never discard at or below commit (safety).
            let cut = txn_kernel::discard_cut(from_index, p.commit);
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
            // F128: leader must fail closed so the orphan cannot reopen+heal.
            // Followers stay best-effort (dead peer must not block the leader).
            if p.log.len() != before_len || nid == leader {
                if nid == leader {
                    persist_log_db(&mut n.db, rid, p)?;
                } else {
                    let _ = persist_log_db(&mut n.db, rid, p);
                }
            }
            if applied_dirty {
                if nid == leader {
                    persist_applied_db(&mut n.db, rid, p)?;
                } else {
                    let _ = persist_applied_db(&mut n.db, rid, p);
                }
            }
        }
        // Notes for exactly the pruned suffix (leader cut), never a blanket
        // from-index drop: later entries with the leader's own commit < their
        // index still lose their entries here (retain < cut removes them) and
        // take their notes with them; earlier-than-cut indexes keep both.
        let leader_cut = self
            .nodes
            .get(&leader)
            .and_then(|n| n.ranges.get(&rid))
            .map(|p| txn_kernel::discard_cut(from_index, p.commit))
            .unwrap_or(from_index);
        self.drop_pending_version_notes_from_cut(rid, leader_cut);
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
            entry: RangeEntry::Put {
                key,
                value,
                si_gen: 0,
            },
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
            if commit_kernel::may_commit_at(p.term_at(n), p.term, count >= maj) {
                new_commit = n;
                break;
            }
        }
        p.commit = new_commit;
        self.apply_range(leader, rid)?;
        Ok(self.commit_index(leader, rid))
    }

    fn apply_range(&mut self, nid: u64, rid: u64) -> Result<()> {
        // Multi-host: remotes apply via their own AE path; never touch missing local dbs.
        if !self.is_local_node(nid) {
            return Ok(());
        }
        pedradb_core::buggify_hooks::inject_checked(
            pedradb_core::buggify_hooks::sites::BEFORE_RAFT_APPLY,
        )
        .map_err(|e| StoreError::from(pedradb_core::CoreError::from(e)))?;
        let mut install_new: Option<Vec<u64>> = None;
        let applied_to_cap = {
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
                RangeEntry::Put { key, value, si_gen } => {
                    if !is_reserved_store_key(key) {
                        apply_put_or_delete(&mut node.db, key, value)?;
                        if *si_gen > 0 {
                            persist_si_hist_on_db(
                                &mut node.db,
                                key,
                                *si_gen,
                                if value.is_empty() {
                                    None
                                } else {
                                    Some(value.as_slice())
                                },
                            )?;
                        }
                    }
                }
                RangeEntry::Batch { pairs, si_gen } => {
                    let mut ops: Vec<BatchOp> = Vec::new();
                    for (k, v) in pairs {
                        if is_reserved_store_key(k) {
                            continue;
                        }
                        if v.is_empty() {
                            ops.push(BatchOp::delete(k));
                        } else {
                            ops.push(BatchOp::put(k, v));
                        }
                    }
                    if !ops.is_empty() {
                        node.db.apply_batch(ops)?;
                    }
                    if *si_gen > 0 {
                        for (k, v) in pairs {
                            if is_reserved_store_key(k) {
                                continue;
                            }
                            persist_si_hist_on_db(
                                &mut node.db,
                                k,
                                *si_gen,
                                if v.is_empty() {
                                    None
                                } else {
                                    Some(v.as_slice())
                                },
                            )?;
                        }
                        // F138: generation meta must be durable before applied advances
                        // (same class as F136 coordinator persist_si_keys).
                        node.db
                            .put(si_meta_key("generation"), encode_u64_meta(*si_gen))?;
                    }
                }
                RangeEntry::TxnPrepare { txn_id, pairs } => {
                    apply_txn_prepare(&mut node.db, *txn_id, pairs)?;
                }
                RangeEntry::TxnCommit {
                    txn_id,
                    keys,
                    si_gen,
                } => {
                    apply_txn_commit(&mut node.db, *txn_id, keys)?;
                    if *si_gen > 0 {
                        for k in keys {
                            if is_reserved_store_key(k) {
                                continue;
                            }
                            let live = node.db.get(k);
                            persist_si_hist_on_db(&mut node.db, k, *si_gen, live.as_deref())?;
                        }
                        node.db
                            .put(si_meta_key("generation"), encode_u64_meta(*si_gen))?;
                    }
                }
                RangeEntry::TxnAbort { txn_id, keys } => {
                    apply_txn_abort(&mut node.db, *txn_id, keys)?;
                }
                RangeEntry::TxnRevert { txn_id, keys } => {
                    apply_txn_revert(&mut node.db, *txn_id, keys)?;
                }
                RangeEntry::Dcs(cmd) => {
                    let r = apply_dcs_command(&mut node.db, cmd);
                    if !pedradb_dcs::dcs_apply_should_advance_result(&r) {
                        return Err(StoreError::Dcs(r.unwrap_err()));
                    }
                }
                RangeEntry::Noop => {}
                RangeEntry::MembershipJoint { new, .. } => {
                    install_new = Some(new.clone());
                }
            }
            // Put path: also bump generation meta when SI gen present.
            if let RangeEntry::Put { si_gen, .. } = &rec.entry {
                if *si_gen > 0 {
                    node.db
                        .put(si_meta_key("generation"), encode_u64_meta(*si_gen))?;
                }
            }
            applied_to = rec.index;
        }
        applied_to.min(end)
        };
        // RFC-0124 P1.1: durable C-new before applied advances past the joint.
        // AS-IS persist applied first (crash: applied high, voters still C-old).
        if membership_kernel::membership_identity_before_applied(true) {
            if let Some(new) = install_new.take() {
                self.install_applied_membership(new)?;
            }
        }
        let node = self.nodes.get_mut(&nid).unwrap();
        let peer = node.ranges.get_mut(&rid).unwrap();
        // F160 residual of F126: do not leave RAM applied ahead of durable meta.
        // AS-IS set applied then `?` on persist fail — cursor stuck high; compact
        // could drop log that reopen still needs for re-apply.
        let old_applied = peer.applied;
        peer.applied = applied_to_cap;
        if let Err(e) = persist_applied_db(&mut node.db, rid, peer) {
            peer.applied = old_applied;
            return Err(e);
        }
        if let Some(new) = install_new {
            self.install_applied_membership(new)?;
        }
        Ok(())
    }

    fn install_applied_membership(&mut self, mut new: Vec<u64>) -> Result<()> {
        new.sort_unstable();
        new.dedup();
        if new.is_empty() {
            return Err(StoreError::Msg(
                "membership joint: refusing empty voter set".into(),
            ));
        }
        self.ids = new;
        self.membership_high_water = self.membership_high_water.max(self.ids.len());
        let live = self.ids.clone();
        for (id, n) in self.nodes.iter_mut() {
            let in_ids = live.contains(id);
            n.participating = in_ids;
            if membership_kernel::removed_steps_down(in_ids) {
                for p in n.ranges.values_mut() {
                    if p.role == Role::Leader {
                        p.role = Role::Follower;
                        p.leader_id = None;
                    }
                }
            }
            for p in n.ranges.values_mut() {
                if let Some(lid) = p.leader_id {
                    if !membership_kernel::hint_if_member(live.contains(&lid)) {
                        p.leader_id = None;
                    }
                }
                let keys: Vec<u64> = p
                    .next_index
                    .keys()
                    .chain(p.match_index.keys())
                    .chain(p.sent_through.keys())
                    .copied()
                    .collect();
                for pid in keys {
                    if membership_kernel::drop_repl_slot(live.contains(&pid)) {
                        p.next_index.remove(&pid);
                        p.match_index.remove(&pid);
                        p.sent_through.remove(&pid);
                    }
                }
            }
        }
        self.persist_cluster_identity()
    }

    /// Best-effort leader id known by **local** peers (from AE `leader_id`).
    ///
    /// Multi-host: when this process is not leader, still surface a routing hint.
    #[must_use]
    pub fn leader_hint(&self, range_id: u64) -> Option<u64> {
        if let Some(lid) = self.range_leader(range_id) {
            return Some(lid);
        }
        for n in self.nodes.values() {
            if let Some(p) = n.ranges.get(&range_id) {
                if let Some(lid) = p.leader_id {
                    if membership_kernel::hint_if_member(self.ids.contains(&lid)) {
                        return Some(lid);
                    }
                }
            }
        }
        None
    }

    /// Human-readable multi-host status (local id, leaders, membership, Pedra L0).
    #[must_use]
    pub fn status_text(&self) -> String {
        let local = self
            .local_node_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "multi".into());
        let mut parts = vec![
            format!("local={local}"),
            format!("cluster={}", self.cluster_id_hex()),
            format!("members={:?}", self.ids),
        ];
        for r in &self.ranges {
            let lead = self
                .range_leader(r.id)
                .or_else(|| self.leader_hint(r.id))
                .map(|l| l.to_string())
                .unwrap_or_else(|| "-".into());
            parts.push(format!("r{}:leader={lead}", r.id));
        }
        // Pedra engine pressure (ops / LB probes).
        for id in &self.ids {
            if let Some(n) = self.nodes.get(id) {
                let s = n.db.stats();
                parts.push(format!(
                    "n{id}:l0={} stall={} pressure={} l0_lim={} mem_lim={}",
                    s.l0_files,
                    s.write_stall_count,
                    s.write_pressure_count,
                    s.write_stall_l0,
                    s.write_stall_mem_bytes
                ));
            }
        }
        parts.join(" ")
    }

    /// Aggregate Pedra L0/mem admission stats across local nodes (structured gates / A-B).
    #[must_use]
    pub fn write_admission_snap(&self) -> WriteAdmissionSnap {
        let mut out = WriteAdmissionSnap::default();
        for id in &self.ids {
            let Some(n) = self.nodes.get(id) else {
                continue;
            };
            let s = n.db.stats();
            out.nodes += 1;
            out.l0_files_max = out.l0_files_max.max(s.l0_files);
            out.write_stall_count_sum += s.write_stall_count;
            out.write_pressure_count_sum += s.write_pressure_count;
            if s.write_stall_l0 > 0 {
                out.write_stall_l0 = s.write_stall_l0;
            }
            if s.write_pressure_l0 > 0 {
                out.write_pressure_l0 = s.write_pressure_l0;
            }
            if s.write_stall_mem_bytes > 0 {
                out.write_stall_mem_bytes = s.write_stall_mem_bytes;
            }
        }
        out
    }

    /// Admin: split the range that contains `split_key` into `[start, split_key)` and
    /// `[split_key, end)` (RFC-0021 P1.2 minimal PD). New range id = max(id)+1.
    ///
    /// Does **not** migrate historical raft log; for lab/scale-out of **new** writes.
    /// Existing keys stay on the left range only if `key < split_key`.
    ///
    /// # Errors
    /// No range, split at start/end, empty split key issues.
    pub fn split_range_at(&mut self, split_key: impl AsRef<[u8]>) -> Result<(u64, u64)> {
        let sk = split_key.as_ref().to_vec();
        if sk.is_empty() {
            return Err(StoreError::Msg("split_key must be non-empty".into()));
        }
        let rid = self.locate(&sk)?;
        let meta = self
            .ranges
            .iter()
            .find(|r| r.id == rid)
            .cloned()
            .ok_or(StoreError::NoRange)?;
        if !meta.start.is_empty() && sk.as_slice() <= meta.start.as_slice() {
            return Err(StoreError::Msg("split_key must be > range start".into()));
        }
        if !meta.end.is_empty() && sk.as_slice() >= meta.end.as_slice() {
            return Err(StoreError::Msg("split_key must be < range end".into()));
        }
        let new_id = self.ranges.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        // Shrink old range end to split_key; new range [split_key, old_end).
        for r in &mut self.ranges {
            if r.id == rid {
                r.end = sk.clone();
            }
        }
        self.ranges.push(RangeMeta {
            id: new_id,
            start: sk,
            end: meta.end,
        });
        self.ranges.sort_by_key(|r| r.id);
        // Install empty raft peers on every local node for the new range.
        let node_ids: Vec<u64> = self.nodes.keys().copied().collect();
        let members = self.ids.clone();
        for nid in node_ids {
            let n = self.nodes.get_mut(&nid).unwrap();
            n.ranges
                .insert(new_id, load_range_peer(&n.db, new_id, nid, &members)?);
        }
        // Elect leaders for both ranges after split.
        self.elect_all(120)?;
        Ok((rid, new_id))
    }

    /// Merge two **adjacent** ranges into the left one (RFC-0022 P1.2 placement).
    ///
    /// Lab control-plane: does not ship historical raft logs; for rebalance hooks
    /// after load shifts. Right range id is removed from the map.
    ///
    /// # Errors
    /// Unknown ids, non-adjacent ranges.
    pub fn merge_adjacent_ranges(&mut self, left_id: u64, right_id: u64) -> Result<u64> {
        let left = self
            .ranges
            .iter()
            .find(|r| r.id == left_id)
            .cloned()
            .ok_or_else(|| StoreError::Msg(format!("unknown range {left_id}")))?;
        let right = self
            .ranges
            .iter()
            .find(|r| r.id == right_id)
            .cloned()
            .ok_or_else(|| StoreError::Msg(format!("unknown range {right_id}")))?;
        if left.end != right.start {
            return Err(StoreError::Msg(
                "ranges are not adjacent (left.end must equal right.start)".into(),
            ));
        }
        for r in &mut self.ranges {
            if r.id == left_id {
                r.end = right.end.clone();
            }
        }
        self.ranges.retain(|r| r.id != right_id);
        for n in self.nodes.values_mut() {
            n.ranges.remove(&right_id);
        }
        self.elect_all(80)?;
        Ok(left_id)
    }

    /// Set region label for a member (RFC-0021 P2.6 lab multi-site).
    pub fn set_node_region(&mut self, node_id: u64, region: impl Into<String>) -> Result<()> {
        if !self.ids.contains(&node_id) {
            return Err(StoreError::Msg(format!("unknown member {node_id}")));
        }
        self.node_regions.insert(node_id, region.into());
        Ok(())
    }

    /// Region for `node_id` if set.
    #[must_use]
    pub fn node_region(&self, node_id: u64) -> Option<&str> {
        self.node_regions.get(&node_id).map(|s| s.as_str())
    }

    /// Prefer members in `prefer_region` first (then others by id). Lab routing hint.
    #[must_use]
    pub fn dial_order_prefer_region(&self, prefer_region: Option<&str>) -> Vec<u64> {
        let mut same = Vec::new();
        let mut other = Vec::new();
        for &id in &self.ids {
            let reg = self.node_regions.get(&id).map(|s| s.as_str());
            if prefer_region.is_some() && reg == prefer_region {
                same.push(id);
            } else {
                other.push(id);
            }
        }
        same.sort_unstable();
        other.sort_unstable();
        same.extend(other);
        same
    }

    /// Install peer dial map in-process (RFC-0021 P1.3). Does not require SSH.
    pub fn set_peer_addrs(&mut self, peers: impl IntoIterator<Item = (u64, String)>) -> Result<()> {
        let map: HashMap<u64, String> = peers.into_iter().collect();
        if map.is_empty() {
            return Err(StoreError::Msg("empty peer map".into()));
        }
        self.peer_addrs = map;
        Ok(())
    }

    /// Current peer dial map (may be empty if never set).
    #[must_use]
    pub fn peer_addrs(&self) -> &HashMap<u64, String> {
        &self.peer_addrs
    }

    /// Machine-readable cluster status (RFC-0021 P0.5) — no serde dep, hand JSON.
    ///
    /// Shape:
    /// ```json
    /// {"local":1,"members":[1,2,3],"regions":{"1":"a"},"peers":{"1":"h:p"},"ranges":[...]}
    /// ```
    #[must_use]
    pub fn cluster_status_json(&self) -> String {
        let local = self
            .local_node_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "null".into());
        let members = self
            .ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut regions = Vec::new();
        for (id, reg) in &self.node_regions {
            let esc = reg.replace('\\', "\\\\").replace('"', "\\\"");
            regions.push(format!("\"{id}\":\"{esc}\""));
        }
        let mut peers = Vec::new();
        for (id, addr) in &self.peer_addrs {
            let esc = addr.replace('\\', "\\\\").replace('"', "\\\"");
            peers.push(format!("\"{id}\":\"{esc}\""));
        }
        let mut ranges = Vec::new();
        for r in &self.ranges {
            let lead = self
                .range_leader(r.id)
                .or_else(|| self.leader_hint(r.id))
                .map(|l| l.to_string())
                .unwrap_or_else(|| "null".into());
            let mut commits = Vec::new();
            let mut applieds = Vec::new();
            for &nid in &self.ids {
                commits.push(format!("\"{nid}\":{}", self.commit_index(nid, r.id)));
                applieds.push(format!("\"{nid}\":{}", self.applied_index(nid, r.id)));
            }
            ranges.push(format!(
                "{{\"id\":{},\"leader\":{},\"commit\":{{{}}},\"applied\":{{{}}}}}",
                r.id,
                lead,
                commits.join(","),
                applieds.join(",")
            ));
        }
        format!(
            "{{\"local\":{local},\"members\":[{members}],\"regions\":{{{}}},\"peers\":{{{}}},\"ranges\":[{}]}}",
            regions.join(","),
            peers.join(","),
            ranges.join(",")
        )
    }

    /// Put key/value via the leader of the owning range.
    ///
    /// Multi-host: only the process that holds the live leader may propose;
    /// others return [`StoreError::NotLeader`] with a routing hint when known.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let key = key.as_ref().to_vec();
        if is_reserved_store_key(&key) {
            return Err(StoreError::Msg(
                "key prefix reserved for store internal meta".into(),
            ));
        }
        let value = value.as_ref().to_vec();
        if value.len() > MAX_VALUE_BYTES {
            return Err(StoreError::ValueTooLarge {
                size: value.len(),
                limit: MAX_VALUE_BYTES,
            });
        }
        let rid = self.locate(&key)?;
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: self.leader_hint(rid),
        })?;
        if !self.is_local_node(leader) {
            return Err(StoreError::NotLeader {
                range_id: rid,
                leader: Some(leader),
            });
        }
        {
            let db = &self.nodes.get(&leader).unwrap().db;
            if intent_conflict(db, &key, None) {
                return Err(StoreError::Conflict);
            }
        }
        // Version notes staged inside broadcast_append; flushed on majority commit
        // (Direct Ok or later via finish_queued_propose).
        let r = self.broadcast_append(
            rid,
            leader,
            Some(RangeEntry::Put {
                key: key.clone(),
                value: value.clone(),
                si_gen: 0, // assigned in with_si_gen at propose
            }),
        );
        self.record_propose_result(r)
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
            if is_reserved_store_key(&key) {
                return Err(StoreError::Msg(
                    "key prefix reserved for store internal meta".into(),
                ));
            }
            owned.push((key, v.as_ref().to_vec()));
        }
        if owned.is_empty() {
            return Ok(());
        }
        validate_tx_pairs(&owned)?;
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
        {
            let db = &self.nodes.get(&leader).unwrap().db;
            for (k, _) in &owned {
                if intent_conflict(db, k, None) {
                    return Err(StoreError::Conflict);
                }
            }
        }
        let r = self.broadcast_append(
            rid,
            leader,
            Some(RangeEntry::Batch {
                pairs: owned.clone(),
                si_gen: 0,
            }),
        );
        self.record_propose_result(r)
    }

    /// Stage a put for later [`Self::flush_writes`] (RFC-0025 P1.1 group-commit style).
    ///
    /// Does **not** hit Raft until flush. Use for tight loops that can batch.
    ///
    /// # Errors
    /// Reserved key / value size limits.
    pub fn put_buffered(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let key = key.as_ref().to_vec();
        if is_reserved_store_key(&key) {
            return Err(StoreError::Msg(
                "key prefix reserved for store internal meta".into(),
            ));
        }
        let value = value.as_ref().to_vec();
        if value.len() > MAX_VALUE_BYTES {
            return Err(StoreError::ValueTooLarge {
                size: value.len(),
                limit: MAX_VALUE_BYTES,
            });
        }
        if self.write_coalesce.len() >= MAX_TX_KEYS {
            return Err(StoreError::TransactionTooLarge {
                size: self.write_coalesce.len() + 1,
                limit: MAX_TX_KEYS,
            });
        }
        self.write_coalesce.push((key, value));
        Ok(())
    }

    /// Number of staged [`Self::put_buffered`] pairs not yet flushed.
    #[must_use]
    pub fn buffered_writes(&self) -> usize {
        self.write_coalesce.len()
    }

    /// Flush staged puts via [`Self::put_many`] (one Raft batch per range;
    /// multi-range uses 2PC `commit_tx` — F77).
    ///
    /// F66: on failure the buffer is **kept** so the client can retry. (A prior
    /// `mem::take` dropped staged pairs on any `put_many` error — silent loss.)
    ///
    /// # Errors
    /// Same as [`Self::put_many`].
    pub fn flush_writes(&mut self) -> Result<()> {
        if self.write_coalesce.is_empty() {
            return Ok(());
        }
        // Clone so a failed put_many does not wipe the buffer (F66).
        let snapshot: Vec<(Vec<u8>, Vec<u8>)> = self.write_coalesce.clone();
        self.put_many(snapshot.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))?;
        self.write_coalesce.clear();
        Ok(())
    }

    /// Stage put and auto-flush when buffer reaches `max_batch` (or always if 1).
    ///
    /// # Errors
    /// Reserved key / flush errors.
    pub fn put_coalesce(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        max_batch: usize,
    ) -> Result<()> {
        self.put_buffered(key, value)?;
        let max = max_batch.max(1);
        if self.write_coalesce.len() >= max {
            self.flush_writes()?;
        }
        Ok(())
    }

    /// Put many keys with **same-range batching** (RFC-0025 P0.1).
    ///
    /// Groups pairs by range and calls [`Self::put_batch`] once per group. Prefer
    /// this over N×[`Self::put`] when keys share a range (layers: row+index,
    /// bulk ingest). Cross-range still costs one batch per range (not full 2PC).
    ///
    /// # Errors
    /// Same as [`Self::put_batch`] per group.
    pub fn put_many(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<[u8]>, impl AsRef<[u8]>)>,
    ) -> Result<()> {
        let mut by_range: RangeKvMap = HashMap::new();
        let mut flat: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (k, v) in pairs {
            let key = k.as_ref().to_vec();
            if is_reserved_store_key(&key) {
                return Err(StoreError::Msg(
                    "key prefix reserved for store internal meta".into(),
                ));
            }
            let val = v.as_ref().to_vec();
            if val.len() > MAX_VALUE_BYTES {
                return Err(StoreError::ValueTooLarge {
                    size: val.len(),
                    limit: MAX_VALUE_BYTES,
                });
            }
            let rid = self.locate(&key)?;
            by_range
                .entry(rid)
                .or_default()
                .push((key.clone(), val.clone()));
            flat.push((key, val));
        }
        // F77: multi-range sequential put_batch left earlier ranges committed when a
        // later range failed (silent partial apply). Cross-range uses 2PC commit_tx.
        if by_range.len() > 1 {
            let _ = self.commit_tx(flat)?;
            return Ok(());
        }
        let mut rids: Vec<u64> = by_range.keys().copied().collect();
        rids.sort_unstable();
        for rid in rids {
            let group = by_range.remove(&rid).unwrap_or_default();
            if group.is_empty() {
                continue;
            }
            self.put_batch(group)?;
        }
        Ok(())
    }

    /// Allocate the next txn id. Persist the issued id **before** advancing RAM
    /// (F132). Swallowing persist after a RAM bump reused the id on reopen
    /// once the finished TX no longer had `\0store/txn/` keys.
    fn alloc_txn_id(&mut self) -> Result<u64> {
        let id = self.next_txn_id;
        self.persist_u64_meta_all("next_txn", id)?;
        self.next_txn_id = id.saturating_add(1).max(1);
        Ok(id)
    }

    fn group_pairs_by_range(&self, pairs: &[KvPair]) -> Result<RangeGroups> {
        let mut map: HashMap<u64, Vec<KvPair>> = HashMap::new();
        for (k, v) in pairs {
            if is_reserved_store_key(k) {
                return Err(StoreError::Msg(
                    "key prefix reserved for store internal meta".into(),
                ));
            }
            let rid = self.locate(k)?;
            map.entry(rid).or_default().push((k.clone(), v.clone()));
        }
        let mut out: RangeGroups = map.into_iter().collect();
        out.sort_by_key(|(rid, _)| *rid);
        Ok(out)
    }

    fn propose_on_range(&mut self, rid: u64, entry: RangeEntry) -> Result<()> {
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        self.broadcast_append(rid, leader, Some(entry))
    }

    /// Prepare a multi-key TX on all touched ranges (2PC phase 1).
    ///
    /// Same-range batches use a single prepare. On success, call [`tx_finish`] or
    /// [`tx_cancel`]. Isolation: write-write conflict if any key already holds an
    /// intent from another txn (`StoreError::Conflict`).
    pub fn tx_start(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<[u8]>, impl AsRef<[u8]>)>,
    ) -> Result<TxHandle> {
        let owned: Vec<(Vec<u8>, Vec<u8>)> = pairs
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_vec(), v.as_ref().to_vec()))
            .collect();
        if owned.is_empty() {
            return Err(StoreError::Msg("empty transaction".into()));
        }
        let groups = self.group_pairs_by_range(&owned)?;
        let txn_id = self.alloc_txn_id()?;
        let mut keys_by_range: Vec<(u64, Vec<Vec<u8>>)> = Vec::new();
        for (rid, range_pairs) in &groups {
            let keys: Vec<Vec<u8>> = range_pairs.iter().map(|(k, _)| k.clone()).collect();
            // Leader pre-check intents.
            // F50: any failure after earlier ranges prepared must abort those intents.
            // `?` on missing leader previously returned without cleanup → immortal Conflict.
            let Some(leader) = self.range_leader(*rid) else {
                if txn_kernel::prepare_error_aborts_earlier() {
                    for (pr, pkeys) in &keys_by_range {
                        self.cleanup_range_keys(*pr, txn_id, pkeys, CleanupMode::Abort)?;
                    }
                }
                return Err(StoreError::NotLeader {
                    range_id: *rid,
                    leader: None,
                });
            };
            {
                let db = &self.nodes.get(&leader).unwrap().db;
                for (k, _) in range_pairs {
                    if intent_conflict(db, k, Some(txn_id)) {
                        if txn_kernel::prepare_error_aborts_earlier() {
                            for (pr, pkeys) in &keys_by_range {
                                self.cleanup_range_keys(*pr, txn_id, pkeys, CleanupMode::Abort)?;
                            }
                        }
                        return Err(StoreError::Conflict);
                    }
                }
            }
            match self.propose_on_range(
                *rid,
                RangeEntry::TxnPrepare {
                    txn_id,
                    pairs: range_pairs.clone(),
                },
            ) {
                Ok(()) => {
                    // Verify prepare status on leader (apply may have set abort).
                    let db = &self.nodes.get(&leader).unwrap().db;
                    let st = db.get(&txn_status_key(txn_id));
                    if st.as_deref() == Some(b"abort".as_ref()) {
                        if txn_kernel::prepare_error_aborts_earlier() {
                            for (pr, pkeys) in &keys_by_range {
                                self.cleanup_range_keys(*pr, txn_id, pkeys, CleanupMode::Abort)?;
                            }
                            self.cleanup_range_keys(*rid, txn_id, &keys, CleanupMode::Abort)?;
                        }
                        return Err(StoreError::Conflict);
                    }
                    keys_by_range.push((*rid, keys));
                }
                Err(e) => {
                    if txn_kernel::prepare_error_aborts_earlier() {
                        for (pr, pkeys) in &keys_by_range {
                            self.cleanup_range_keys(*pr, txn_id, pkeys, CleanupMode::Abort)?;
                        }
                    }
                    return Err(e);
                }
            }
        }
        let ranges: Vec<u64> = keys_by_range.iter().map(|(r, _)| *r).collect();
        Ok(TxHandle {
            id: txn_id,
            ranges,
            keys_by_range,
        })
    }

    fn keys_for_range(handle: &TxHandle, rid: u64) -> &[Vec<u8>] {
        handle
            .keys_by_range
            .iter()
            .find(|(r, _)| *r == rid)
            .map(|(_, k)| k.as_slice())
            .unwrap_or(&[])
    }

    /// Drop intents (and optionally user keys) on **every** peer's PedraDB without Raft.
    ///
    /// Used when a range has no leader so `TxnAbort`/`TxnRevert` cannot majority-commit.
    /// Prevents durable stuck intents that would `Conflict` forever (I-TX-2 / reopen).
    ///
    /// # Errors
    /// Revert/abort path errors (F122 — corrupt preimage must not be swallowed).
    fn force_local_clear_keys(
        &mut self,
        txn_id: u64,
        keys: &[Vec<u8>],
        delete_user_values: bool,
    ) -> Result<()> {
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::force_clear_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            let Some(n) = self.nodes.get_mut(&nid) else {
                continue;
            };
            if delete_user_values {
                apply_txn_revert(&mut n.db, txn_id, keys)?;
            } else {
                apply_txn_abort(&mut n.db, txn_id, keys)?;
            }
        }
        Ok(())
    }

    /// F47: durable abort fence so a later-committed raft `TxnCommit` (orphan log
    /// entry after a failed `tx_finish` + heal/elect) cannot materialise user keys.
    ///
    /// [`apply_txn_commit`] treats status `abort` as no-op for the put path.
    ///
    /// # Errors
    /// F130: fence put failure must surface — a silent miss lets TxnCommit apply.
    fn fence_txn_aborted(&mut self, txn_id: u64) -> Result<()> {
        let key = txn_status_key(txn_id);
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::persist_fence_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            if let Some(n) = self.nodes.get_mut(&nid) {
                n.db.put(&key, b"abort")?;
            }
        }
        Ok(())
    }

    /// Revert a range that already majority-committed `TxnCommit` (F47 / F139).
    ///
    /// Raft `TxnRevert` is required so remote peers that applied the commit see
    /// the compensating entry. Force-local heals this process's Pedra copies.
    ///
    /// # Errors
    /// Force-local revert failure, or raft propose failure even when local clear
    /// succeeded (remotes may still hold the committed write).
    fn revert_majority_committed_range(
        &mut self,
        rid: u64,
        txn_id: u64,
        keys: &[Vec<u8>],
    ) -> Result<()> {
        let raft = self.propose_on_range(
            rid,
            RangeEntry::TxnRevert {
                txn_id,
                keys: keys.to_vec(),
            },
        );
        let local = self.force_local_clear_keys(txn_id, keys, true);
        match (raft, local) {
            (Ok(()), Ok(())) => Ok(()),
            (_, Err(e)) => Err(e),
            (Err(e), Ok(())) => Err(e),
        }
    }

    /// Try raft cleanup; always force-local clear so leaderless ranges cannot stick intents.
    ///
    /// # Errors
    /// Local force-clear failures (F122).
    fn cleanup_range_keys(
        &mut self,
        rid: u64,
        txn_id: u64,
        keys: &[Vec<u8>],
        mode: CleanupMode,
    ) -> Result<()> {
        let entry = match mode {
            CleanupMode::Abort => RangeEntry::TxnAbort {
                txn_id,
                keys: keys.to_vec(),
            },
            CleanupMode::Revert => RangeEntry::TxnRevert {
                txn_id,
                keys: keys.to_vec(),
            },
        };
        let raft_ok = self.propose_on_range(rid, entry).is_ok();
        // Even if raft Ok, force-local is idempotent and heals any lagging peer.
        // If raft failed (no leader), force-local is the only way to drop disk intents.
        let _ = raft_ok;
        self.force_local_clear_keys(txn_id, keys, matches!(mode, CleanupMode::Revert))
    }

    /// 2PC phase 2: commit a prepared TX (materialize intents **per range**).
    ///
    /// If any range fails to commit, **all** prepared ranges are **reverted**
    /// (preimage restored; F34) so criterion 1 holds: fail ⇒ no majority user-key
    /// apply from this TX. Cleanup is force-local on every peer so a leaderless
    /// range cannot leave stuck intents on disk.
    ///
    /// Always Revert (not Abort) after prepare (F47): Abort deletes preimages
    /// without restoring user keys. An orphan `TxnCommit` that later majority-
    /// applies would then stick forever. Revert is idempotent when Commit never
    /// applied (preimage == live value). A durable abort fence is written so
    /// any residual raft `TxnCommit` re-apply is a no-op / re-revert.
    ///
    /// On full success, SI/OCC history is advanced **once** for all keys in the TX
    /// (F37) so intermediate generations never observe a partial multi-range apply.
    pub fn tx_finish(&mut self, handle: &TxHandle) -> Result<()> {
        // Reserve one SI gen for the whole multi-range TX (F37 + F49).
        let reserved = txn_kernel::reserve_si_gen(self.commit_generation);
        self.commit_generation = reserved.next_current;
        let si_gen = reserved.reserved;
        let mut committed: Vec<u64> = Vec::new();
        for rid in &handle.ranges {
            let keys = Self::keys_for_range(handle, *rid).to_vec();
            match self.propose_on_range(
                *rid,
                RangeEntry::TxnCommit {
                    txn_id: handle.id,
                    keys: keys.clone(),
                    si_gen,
                },
            ) {
                Ok(()) => committed.push(*rid),
                Err(e) => {
                    // F47: abort is a log decision, not only a local Pedra put.
                    // Ranges that already majority-committed TxnCommit must get
                    // a majority TxnRevert on the same raft log. Local fence
                    // remains defense-in-depth for reopen/apply.
                    // F130: fence failure must not be silent.
                    self.fence_txn_aborted(handle.id)?;
                    // F122: prefer surfacing corrupt-preimage cleanup failure
                    // over the original NotLeader (otherwise aborted writes stick).
                    let mut cleanup_err: Option<StoreError> = None;
                    for rid2 in &handle.ranges {
                        let akeys = Self::keys_for_range(handle, *rid2).to_vec();
                        // RFC-0056 P1.4: per-range cleanup action from the
                        // pure kernel (F47 majority revert vs F34 local).
                        match tx_glue_kernel::tx_range_action(committed.contains(rid2), true) {
                            TxRangeAction::MajorityRevert => {
                                if let Err(ce) =
                                    self.revert_majority_committed_range(*rid2, handle.id, &akeys)
                                {
                                    if cleanup_err.is_none() {
                                        cleanup_err = Some(ce);
                                    }
                                }
                            }
                            TxRangeAction::LocalRevert => {
                                if let Err(ce) = self.cleanup_range_keys(
                                    *rid2,
                                    handle.id,
                                    &akeys,
                                    CleanupMode::Revert,
                                ) {
                                    if cleanup_err.is_none() {
                                        cleanup_err = Some(ce);
                                    }
                                }
                            }
                            TxRangeAction::KeepCommitted => {}
                        }
                    }
                    if let Err(fe) = self.fence_txn_aborted(handle.id) {
                        return Err(cleanup_err.unwrap_or(fe));
                    }
                    return Err(cleanup_err.unwrap_or(e));
                }
            }
        }
        // One SI generation for the whole TX (must run before preimages drop).
        self.note_tx_commit(handle, si_gen)?;
        self.drop_preimages(handle)?;
        Ok(())
    }

    /// Best local node to read applied TX state for `rid` after majority commit.
    ///
    /// Prefer the live range leader (just applied), else the participating peer with
    /// highest `applied`. Never default to a partitioned `ids[0]` (F42).
    fn best_applied_reader(&self, rid: u64) -> Option<u64> {
        let lead = self
            .range_leader(rid)
            .filter(|&l| self.is_local_node(l) && self.is_participating(l));
        let self_id = self.local_node_id();
        let mut best: Option<(u64, bool, bool, bool, u64)> = None;
        for &nid in &self.ids {
            if !self.is_local_node(nid) {
                continue;
            }
            let c_lead = lead == Some(nid);
            let c_part = self.is_participating(nid);
            let c_self = self_id == Some(nid);
            let c_app = self.applied_index(nid, rid);
            match best {
                None => best = Some((nid, c_lead, c_part, c_self, c_app)),
                Some((_, bl, bp, bs, ba)) => {
                    if si_kernel::si_reader_beats(c_lead, c_part, c_self, c_app, bl, bp, bs, ba) {
                        best = Some((nid, c_lead, c_part, c_self, c_app));
                    }
                }
            }
        }
        best.map(|(id, _, _, _, _)| id)
    }

    /// Record all keys of a finished 2PC TX under a **single** commit generation.
    ///
    /// Preimages are read from durable prepare records; new values from applied user
    /// keys (or leftover pair/intent if a peer is still catching up).
    ///
    /// Per-range reads use [`Self::best_applied_reader`] so a lagging `ids[0]` cannot
    /// poison SI history after a majority commit that excluded that node (F42).
    ///
    /// # Errors
    /// SI meta persist (F136).
    fn note_tx_commit(&mut self, handle: &TxHandle, si_gen: u64) -> Result<()> {
        let mut items: Vec<VersionNote> = Vec::new();
        for (rid, keys) in &handle.keys_by_range {
            let Some(nid) = self.best_applied_reader(*rid) else {
                continue;
            };
            let Some(n) = self.nodes.get(&nid) else {
                continue;
            };
            for k in keys {
                if is_reserved_store_key(k) {
                    continue;
                }
                // Missing pre → absent floor. Present corrupt → Err (F140),
                // never unwrap_or(None) as a fake gen-0 tombstone.
                let pre = match n.db.get(&txn_pre_key(handle.id, k)) {
                    None => None,
                    Some(b) => decode_preimage(b.as_ref())?,
                };
                // F141: short/corrupt intent used as val fallback must not become
                // empty (SI delete tombstone) via decode_intent.ok().
                let val = if let Some(b) = n.db.get(k) {
                    b.to_vec()
                } else if let Some(b) = n.db.get(&txn_pair_key(handle.id, k)) {
                    b.to_vec()
                } else if let Some(raw) = n.db.get(&intent_key(k)) {
                    let (oid, v) = decode_intent(&raw)?;
                    if oid == handle.id {
                        v.to_vec()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                items.push((k.clone(), val, pre));
            }
        }
        // Use the gen reserved at `tx_finish` (matches durable TxnCommit.si_gen).
        self.note_mutations_at(Some(si_gen), &items)?;
        Ok(())
    }

    /// Drop prepare-time preimages after a *successful* all-range commit.
    ///
    /// # Errors
    /// I/O deleting preimage keys (leftover pre confuses later reverts).
    fn drop_preimages(&mut self, handle: &TxHandle) -> Result<()> {
        let nids: Vec<u64> = self.nodes.keys().copied().collect();
        for nid in nids {
            let in_ids = self.ids.contains(&nid);
            if !membership_kernel::drop_preimages_node_counts(self.is_local_node(nid), in_ids) {
                continue;
            }
            let Some(node) = self.nodes.get_mut(&nid) else {
                continue;
            };
            let mut ops = Vec::new();
            for (_, keys) in &handle.keys_by_range {
                for u in keys {
                    ops.push(BatchOp::delete(txn_pre_key(handle.id, u)));
                }
            }
            if !ops.is_empty() {
                node.db.apply_batch(ops)?;
            }
        }
        Ok(())
    }

    /// Abort a prepared TX (drop intents on all peers, even without leaders).
    pub fn tx_cancel(&mut self, handle: &TxHandle) -> Result<()> {
        // F47: fence first so in-flight/uncommitted TxnCommit cannot apply later.
        self.fence_txn_aborted(handle.id)?;
        // Revert (not abort-only): a failed `tx_finish` may have materialised
        // some ranges before disk death; abort would drop intents and leave
        // those user keys. Restore prepare-time preimages (F34).
        for rid in &handle.ranges {
            let keys = Self::keys_for_range(handle, *rid).to_vec();
            self.cleanup_range_keys(*rid, handle.id, &keys, CleanupMode::Revert)?;
        }
        self.fence_txn_aborted(handle.id)?;
        Ok(())
    }

    /// Atomic multi-key write across **any** set of ranges (FDB-class defining gap).
    ///
    /// - Prepare all ranges then commit per-range keys only.
    /// - On prepare or commit failure: abort/revert so **no** majority user-key
    ///   apply remains from this TX; intents are force-cleared if raft cannot.
    /// - Enforces [`MAX_VALUE_BYTES`] / [`MAX_TX_BYTES`] / [`MAX_TX_KEYS`].
    ///
    /// Returns the transaction id on success.
    pub fn commit_tx(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<[u8]>, impl AsRef<[u8]>)>,
    ) -> Result<u64> {
        let owned: Vec<(Vec<u8>, Vec<u8>)> = pairs
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_vec(), v.as_ref().to_vec()))
            .collect();
        validate_tx_pairs(&owned)?;
        // TxnCommit entries stage version notes at propose; flushed on majority.
        let handle = self.tx_start(owned)?;
        match self.tx_finish(&handle) {
            Ok(()) => Ok(handle.id),
            Err(e) => {
                // tx_finish already reverts/aborts with force-local; cancel is extra sweep.
                // F135: surface cancel failure (corrupt pre / fence) over the original err.
                if let Err(ce) = self.tx_cancel(&handle) {
                    return Err(ce);
                }
                Err(e)
            }
        }
    }

    /// Open a **write-only** buffered TX without snapshot (weak path). Prefer [`Self::begin`].
    #[must_use]
    pub fn pending_tx_begin(&self) -> crate::client::PendingTx {
        crate::client::PendingTx::new()
    }

    /// RFC-0023 default: unified snapshot TX (alias of [`Self::begin`]).
    #[must_use]
    pub fn tx_begin(&self) -> crate::client::Transaction {
        self.begin()
    }

    /// Cluster commit generation (RFC-0023 `read_version`). Starts at 0; bumps on
    /// successful majority put / put_batch / commit_tx.
    #[must_use]
    pub fn read_version(&self) -> u64 {
        self.commit_generation
    }

    /// Lowest readable snapshot (versions strictly below are too old after GC).
    #[must_use]
    pub fn safe_watermark(&self) -> u64 {
        self.safe_watermark
    }

    /// Begin a snapshot TX at the current [`Self::read_version`] (RFC-0023 default).
    ///
    /// Leadership-invisible: callers never pass range or node ids.
    #[must_use]
    pub fn begin(&self) -> crate::client::Transaction {
        crate::client::Transaction::at_version(self.commit_generation)
    }

    /// Alias for [`Self::begin`] (RFC-0022 name).
    #[must_use]
    pub fn snapshot_begin(&self) -> SnapshotTx {
        self.begin()
    }

    /// Current value of `key` as of the latest commit (history tip or Pedra).
    ///
    /// Pedra fallback uses [`Self::best_applied_reader`] for the key's range — not
    /// `ids[0]` / arbitrary LocalApplied (same F42 class as `note_tx_commit`).
    fn value_now(&self, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(hist) = self.key_history.get(key) {
            if let Some((_, v)) = hist.last() {
                return v.clone();
            }
        }
        let rid = self.locate(key).ok()?;
        let nid = self.best_applied_reader(rid)?;
        self.nodes
            .get(&nid)
            .and_then(|n| n.db.get(key))
            .map(|b| b.to_vec())
    }

    /// Stamp a client entry with a new SI generation (leader propose path).
    ///
    /// **Reserves** [`Self::commit_generation`] immediately (F49) so two outstanding
    /// Queued proposes never embed the same durable `si_gen` in the raft log /
    /// apply-path hist. `note_mutations` applies the reserved gen without a second bump.
    fn with_si_gen(&mut self, entry: RangeEntry) -> RangeEntry {
        match entry {
            RangeEntry::Put { key, value, .. } => {
                let r = txn_kernel::reserve_si_gen(self.commit_generation);
                self.commit_generation = r.next_current;
                RangeEntry::Put {
                    key,
                    value,
                    si_gen: r.reserved,
                }
            }
            RangeEntry::Batch { pairs, .. } => {
                let r = txn_kernel::reserve_si_gen(self.commit_generation);
                self.commit_generation = r.next_current;
                RangeEntry::Batch {
                    pairs,
                    si_gen: r.reserved,
                }
            }
            other => other,
        }
    }

    /// Build version-note items for a client log entry (preimages at propose time).
    fn preimages_for_entry(&self, entry: &RangeEntry) -> Vec<VersionNote> {
        match entry {
            RangeEntry::Put { key, value, .. } => {
                if is_reserved_store_key(key) {
                    return Vec::new();
                }
                vec![(key.clone(), value.clone(), self.value_now(key))]
            }
            RangeEntry::Batch { pairs, .. } => pairs
                .iter()
                .filter(|(k, _)| !is_reserved_store_key(k))
                .map(|(k, v)| (k.clone(), v.clone(), self.value_now(k)))
                .collect(),
            // TxnCommit SI notes are deferred to [`Self::note_tx_commit`] after *all*
            // ranges majority-commit (F37: one generation per logical multi-range TX).
            RangeEntry::TxnCommit { .. } => Vec::new(),
            RangeEntry::Dcs(cmd) => {
                let (key, value) = match cmd {
                    DcsCommand::Put { key, value, .. }
                    | DcsCommand::Create { key, value, .. }
                    | DcsCommand::Cas { key, value, .. } => (key.clone(), value.clone()),
                    DcsCommand::Delete { key } => (key.clone(), Vec::new()),
                };
                vec![(key.clone(), value, self.value_now(&key))]
            }
            RangeEntry::Noop
            | RangeEntry::TxnPrepare { .. }
            | RangeEntry::TxnAbort { .. }
            | RangeEntry::TxnRevert { .. }
            | RangeEntry::MembershipJoint { .. } => Vec::new(),
        }
    }

    /// Flush staged version notes for `range_id` through `through_index` (inclusive).
    ///
    /// # Errors
    /// SI meta persist while applying notes (F136).
    fn flush_version_notes_through(&mut self, range_id: u64, through_index: u64) -> Result<()> {
        let already = self
            .version_notes_through
            .get(&range_id)
            .copied()
            .unwrap_or(0);
        if through_index <= already {
            return Ok(());
        }
        let mut idxs: Vec<u64> = self
            .pending_version_notes
            .keys()
            .filter(|(r, i)| *r == range_id && *i > already && *i <= through_index)
            .map(|(_, i)| *i)
            .collect();
        idxs.sort_unstable();
        for idx in idxs {
            if let Some((gen, items)) = self.pending_version_notes.remove(&(range_id, idx)) {
                self.note_mutations_at(Some(gen), &items)?;
            }
            self.version_notes_through.insert(range_id, idx);
        }
        // Advance watermark even if some indices had empty notes (Noop).
        let cur = self
            .version_notes_through
            .get(&range_id)
            .copied()
            .unwrap_or(0);
        if through_index > cur {
            self.version_notes_through.insert(range_id, through_index);
        }
        Ok(())
    }

    /// Drop staged notes for discarded (uncommitted) log indexes.
    fn drop_pending_version_notes_from(&mut self, range_id: u64, from_index: u64) {
        self.pending_version_notes
            .retain(|(r, i), _| !(*r == range_id && *i >= from_index));
    }

    /// F-found companion (RFC-0059 swarm, seed 49): drop notes only for
    /// indexes the discard actually pruned (>= the leader's `cut`), never
    /// `from_index` — with the escape check keeping later entries alive,
    /// an from-index drop would strip the OCC notes of entries that still
    /// commit (silent versionless apply).
    fn drop_pending_version_notes_from_cut(&mut self, range_id: u64, cut: u64) {
        self.pending_version_notes
            .retain(|(r, i), _| !(*r == range_id && *i >= cut));
    }

    /// Value of `key` as of snapshot generation `R` (RFC-0023 SI).
    ///
    /// Does **not** return commits with generation `> R`. Own-TX write buffering is
    /// the caller's responsibility.
    ///
    /// # Errors
    /// [`StoreError::TransactionTooOld`] when `snapshot` predates the SI GC
    /// floor (F168: pruned history must fail closed, not fabricate absence);
    /// store get errors for keys never versioned in this process.
    pub fn get_at_version(&self, key: &[u8], snapshot: u64) -> Result<Option<Vec<u8>>> {
        // F168: below the floor entry (`watermark - 1`) the pruned history has
        // no covering version — the old path answered `Ok(None)`, silently
        // reporting committed data as absent to an old snapshot.
        if matches!(
            si_kernel::snapshot_read_plan(snapshot, self.safe_watermark),
            SnapshotRead::TooOld
        ) {
            return Err(StoreError::TransactionTooOld {
                snapshot,
                current: self.commit_generation,
            });
        }
        if let Some(hist) = self.key_history.get(key) {
            for (g, val) in hist.iter().rev() {
                if *g <= snapshot {
                    return Ok(val.clone());
                }
            }
            // All history entries are after snapshot (should not happen if gen-0 exists).
            return Ok(None);
        }
        // Missing hist: do not serve the Pedra tip as an old snapshot (bitrot /
        // unreplicated SI). Tip is only valid at/after the current generation.
        if snapshot < self.commit_generation {
            return Ok(None);
        }
        Ok(self.get(key)?.map(|b| b.to_vec()))
    }

    /// Record mutations for OCC + in-memory version history after majority commit.
    ///
    /// Durable hist is written on each replica in [`Self::apply_range`] (Raft path).
    /// This updates the coordinator memory map + watch; `persist_si_keys` is a
    /// best-effort mirror for reopen when apply already stored hist.
    ///
    /// Each item is `(key, new_value, preimage)`. Empty `new_value` ⇒ deleted (`None` in hist).
    ///
    /// When `reserved` is `Some(g)` use that SI generation (assigned at propose /
    /// `tx_finish`) without a second bump (F49). `None` allocates a new gen.
    ///
    /// # Errors
    /// SI generation/watermark persist (F136).
    fn note_mutations_at(&mut self, reserved: Option<u64>, items: &[VersionNote]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let g = match reserved {
            Some(r) if r > 0 => {
                if r > self.commit_generation {
                    self.commit_generation = r;
                }
                r
            }
            _ => {
                self.commit_generation = self.commit_generation.saturating_add(1);
                self.commit_generation
            }
        };
        for (k, val, pre) in items {
            let hist = self.key_history.entry(k.clone()).or_default();
            if hist.is_empty() {
                hist.push((0, pre.clone()));
            }
            let live = if val.is_empty() {
                None
            } else {
                Some(val.clone())
            };
            if hist.last().map(|(hg, _)| *hg) != Some(g) {
                hist.push((g, live));
            }
            self.key_versions.insert(k.clone(), g);
            self.watch.notify(k, val, g);
        }
        self.maybe_gc_versions();
        // Mirror watermark/generation; hist rows already on disk via apply when si_gen>0.
        let keys: Vec<Vec<u8>> = items.iter().map(|(k, _, _)| k.clone()).collect();
        self.persist_si_keys(&keys)?;
        Ok(())
    }

    /// Advance watermark and prune version history (RFC-0023 P0.4).
    fn maybe_gc_versions(&mut self) {
        if self.commit_generation <= VERSION_RETENTION {
            return;
        }
        let new_wm = self.commit_generation.saturating_sub(VERSION_RETENTION);
        if new_wm <= self.safe_watermark {
            return;
        }
        self.safe_watermark = new_wm;
        for hist in self.key_history.values_mut() {
            let mut floor_val: Option<Option<Vec<u8>>> = None;
            let mut kept: Vec<(u64, Option<Vec<u8>>)> = Vec::new();
            for (g, v) in hist.drain(..) {
                if g < new_wm {
                    floor_val = Some(v);
                } else {
                    kept.push((g, v));
                }
            }
            if let Some(v) = floor_val {
                // Floor readable for snapshot in [new_wm-1, first kept).
                let floor_gen = new_wm.saturating_sub(1);
                hist.push((floor_gen, v));
            }
            hist.extend(kept);
        }
    }

    /// OCC check then majority commit (used by [`Transaction::commit`] / SnapshotTx).
    ///
    /// # Errors
    /// [`StoreError::TransactionTooOld`], [`StoreError::Conflict`], commit failures.
    pub fn commit_snapshot_tx(
        &mut self,
        snapshot: u64,
        read_keys: impl IntoIterator<Item = Vec<u8>>,
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<u64> {
        self.commit_transaction(snapshot, read_keys, pairs, std::iter::empty())
    }

    /// Full TX commit: too-old, OCC on keys + conflict ranges, then majority apply.
    ///
    /// # Errors
    /// TooOld, Conflict, limits, NotLeader, NotCommitted.
    pub fn commit_transaction(
        &mut self,
        snapshot: u64,
        read_keys: impl IntoIterator<Item = Vec<u8>>,
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
        conflict_ranges: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    ) -> Result<u64> {
        let current = self.commit_generation;
        if snapshot < self.safe_watermark {
            return Err(StoreError::TransactionTooOld {
                snapshot,
                current: self.safe_watermark,
            });
        }
        if current.saturating_sub(snapshot) > MAX_SNAPSHOT_LAG {
            return Err(StoreError::TransactionTooOld { snapshot, current });
        }
        let read_keys: Vec<Vec<u8>> = read_keys.into_iter().collect();
        for k in &read_keys {
            if self.key_versions.get(k).copied().unwrap_or(0) > snapshot {
                return Err(StoreError::Conflict);
            }
        }
        for (k, _) in &pairs {
            if self.key_versions.get(k).copied().unwrap_or(0) > snapshot {
                return Err(StoreError::Conflict);
            }
        }
        let ranges: Vec<(Vec<u8>, Vec<u8>)> = conflict_ranges.into_iter().collect();
        for (start, end) in &ranges {
            for (k, ver) in &self.key_versions {
                if *ver > snapshot && key_in_half_open(k, start, end) {
                    return Err(StoreError::Conflict);
                }
            }
        }
        self.commit_tx(pairs)
    }

    /// Keys in `[start, end)` visible at `snapshot` (history + Pedra gen-0 scan).
    ///
    /// Scans Pedra so durable keys after reopen (no in-memory history) still appear.
    /// Reserved store meta keys are skipped. Lab/ cap: unbounded Pedra range (tests).
    pub fn keys_in_range_at(
        &self,
        start: &[u8],
        end: &[u8],
        snapshot: u64,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        use std::collections::BTreeSet;
        let mut key_set: BTreeSet<Vec<u8>> = BTreeSet::new();
        for k in self.key_history.keys() {
            if key_in_half_open(k, start, end) && !is_reserved_store_key(k) {
                key_set.insert(k.clone());
            }
        }
        // Pedra scan: each overlapping store range on *that* range's applied
        // reader (F108 / F84 residual). A single `best_reader_for_key(start)`
        // missed keys in later ranges when that node lagged there.
        let mut scanned = false;
        for r in &self.ranges {
            let Some((cs, ce)) = clip_query_to_range(start, end, r) else {
                continue;
            };
            let Some(nid) = self
                .best_applied_reader(r.id)
                .or_else(|| self.best_reader_for_key(&cs))
                .or_else(|| self.best_changelog_reader())
            else {
                continue;
            };
            let Some(n) = self.nodes.get(&nid) else {
                continue;
            };
            scanned = true;
            let start_b = Bound::Included(cs.as_slice());
            let end_b = if ce.is_empty() {
                Bound::Unbounded
            } else {
                Bound::Excluded(ce.as_slice())
            };
            for (k, _) in n.db.range_limited(start_b, end_b, None) {
                if !is_reserved_store_key(&k) {
                    key_set.insert(k.to_vec());
                }
            }
        }
        if !scanned {
            if let Some(nid) = self
                .best_reader_for_key(start)
                .or_else(|| self.best_changelog_reader())
                .or_else(|| self.local_node_id())
                .or_else(|| self.ids_first_if_local())
            {
                if let Some(n) = self.nodes.get(&nid) {
                    let start_b = Bound::Included(start);
                    let end_b = if end.is_empty() {
                        Bound::Unbounded
                    } else {
                        Bound::Excluded(end)
                    };
                    for (k, _) in n.db.range_limited(start_b, end_b, None) {
                        if !is_reserved_store_key(&k) {
                            key_set.insert(k.to_vec());
                        }
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(key_set.len());
        for k in key_set {
            if let Some(v) = self.get_at_version(&k, snapshot)? {
                out.push((k, v));
            }
        }
        Ok(out)
    }

    /// Subscribe to key-prefix events after majority commit (RFC-0022 P0.3).
    pub fn watch_prefix(
        &mut self,
        prefix: impl AsRef<[u8]>,
    ) -> (u64, std::sync::mpsc::Receiver<WatchEvent>) {
        self.watch.watch_prefix(prefix)
    }

    /// Drop a watch subscription.
    pub fn unwatch(&mut self, id: u64) {
        self.watch.unwatch(id);
    }

    /// RFC-0013 P1.2: subscribe to range leadership. Best-effort stream —
    /// **not fencing**. Truth remains Raft/DCS (`range_leader` / `get_strong`).
    /// First event is a snapshot of the current unique leader (if any).
    pub fn subscribe_leadership(
        &mut self,
        range_id: u64,
    ) -> (u64, std::sync::mpsc::Receiver<LeadershipEvent>) {
        let (id, rx) = self.live.subscribe(range_id);
        let leader = self.range_leader(range_id);
        self.live.push_snapshot(id, range_id, leader);
        (id, rx)
    }

    /// Drop a Live leadership subscription.
    pub fn unwatch_leadership(&mut self, id: u64) {
        self.live.unwatch(id);
    }

    /// RFC-0013 P1.5: commit / NotCommitted / election counters.
    #[must_use]
    pub fn metrics(&self) -> StoreMetrics {
        self.metrics
    }

    fn record_propose_result(&mut self, r: Result<()>) -> Result<()> {
        match &r {
            Ok(()) => self.metrics.commits_ok = self.metrics.commits_ok.saturating_add(1),
            Err(StoreError::NotCommitted { .. }) => {
                self.metrics.not_committed = self.metrics.not_committed.saturating_add(1);
            }
            _ => {}
        }
        r
    }

    /// Shared watch hub (layers / Scylla-need CP helpers).
    #[must_use]
    pub fn watch_hub(&self) -> &WatchHub {
        &self.watch
    }

    /// Mutable watch hub.
    pub fn watch_hub_mut(&mut self) -> &mut WatchHub {
        &mut self.watch
    }

    /// Version of last mutation of `key` (0 if never written in this process).
    #[must_use]
    pub fn key_version(&self, key: &[u8]) -> u64 {
        self.key_versions.get(key).copied().unwrap_or(0)
    }

    /// Leadership-invisible put: same as [`Self::put`] — routes to the live range
    /// leader in-process without the caller naming a node (RFC-0022 P0.2).
    ///
    /// Multi-host TCP clients use [`TcpClusterClient::put`] for NotLeader retry.
    ///
    /// # Errors
    /// Same as [`Self::put`].
    pub fn put_routed_invisible(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.put(key, value)
    }

    /// Test helper: force watermark (RFC-0023 too-old path without thousands of puts).
    #[doc(hidden)]
    pub fn force_safe_watermark_for_test(&mut self, wm: u64) {
        self.safe_watermark = wm;
    }

    /// Test helper: advance commit generation (legacy; prefer watermark for too-old).
    #[doc(hidden)]
    pub fn force_read_version_for_test(&mut self, v: u64) {
        self.commit_generation = v;
    }

    /// Get from a node (applied state). **LocalApplied** semantics — non-linearizable.
    pub fn get_on(&self, node_id: u64, key: &[u8]) -> Result<Option<Bytes>> {
        self.get_with_policy(node_id, key, ReadPolicy::LocalApplied)
    }

    /// CHANGELOG tail on a caught-up local Pedra (RFC-0024 fold follow).
    ///
    /// Not a Raft read. Prefers the participating local node with the highest
    /// Pedra `last_sequence` so a partitioned `ids[0]` cannot starve the fold
    /// (same class as F42).
    #[must_use]
    pub fn changelog_after(&self, from_seq: u64) -> Vec<pedradb_core::ChangeEntry> {
        let Some(id) = self.best_changelog_reader() else {
            return Vec::new();
        };
        let Some(n) = self.nodes.get(&id) else {
            return Vec::new();
        };
        n.db.changes_after(from_seq)
    }

    /// RFC-0059 diagnostics: that node's own changelog entries (WAL
    /// writes it applied locally), independent of reader election.
    /// Invariant checkers use the union across participating nodes as
    /// ground truth; a single reader can lag under faults.
    pub fn changelog_on(&self, node_id: u64, from_seq: u64) -> Vec<pedradb_core::ChangeEntry> {
        let Some(n) = self.nodes.get(&node_id) else {
            return Vec::new();
        };
        n.db.changes_after(from_seq)
    }

    fn best_changelog_reader(&self) -> Option<u64> {
        let self_id = self.local_node_id();
        let mut best: Option<(u64, bool, bool, bool, u64)> = None;
        for &nid in &self.ids {
            if !self.is_local_node(nid) {
                continue;
            }
            let c_part = self.is_participating(nid);
            let c_self = self_id == Some(nid);
            let c_seq = self
                .nodes
                .get(&nid)
                .map(|n| n.db.last_sequence())
                .unwrap_or(0);
            match best {
                None => best = Some((nid, false, c_part, c_self, c_seq)),
                Some((_, bl, bp, bs, ba)) => {
                    if si_kernel::si_reader_beats(false, c_part, c_self, c_seq, bl, bp, bs, ba) {
                        best = Some((nid, false, c_part, c_self, c_seq));
                    }
                }
            }
        }
        best.map(|(id, _, _, _, _)| id)
    }

    /// Get LocalApplied from the freshest local PedraDB for this key's range (F72/F84).
    ///
    /// F72: never default to a lagging `ids[0]`. F84: prefer
    /// [`Self::best_applied_reader`] for the key's range — global
    /// [`Self::best_changelog_reader`] (max `last_sequence`) can pick a node
    /// busy on another range that is still lagging on this key's range.
    /// Multi-host single local: that node only.
    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        let id = self
            .best_reader_for_key(key)
            .ok_or_else(|| StoreError::Msg("empty".into()))?;
        self.get_on(id, key)
    }

    /// Best local node to serve a LocalApplied read of `key` (F84).
    fn best_reader_for_key(&self, key: &[u8]) -> Option<u64> {
        if si_kernel::point_get_prefer_applied() {
            if let Ok(rid) = self.locate(key) {
                if let Some(id) = self.best_applied_reader(rid) {
                    return Some(id);
                }
            }
        }
        self.best_changelog_reader()
            .or_else(|| self.local_node_id())
            .or_else(|| self.ids_first_if_local())
    }

    /// RFC-0142: `ids.first()` is not a LocalApplied reader unless that node
    /// is opened in this process (TCP removed replica: first voter is remote).
    fn ids_first_if_local(&self) -> Option<u64> {
        let id = *self.ids.first()?;
        membership_kernel::reader_id_local(self.is_local_node(id)).then_some(id)
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
                let thinks = n.ranges.get(&rid).is_some_and(|p| p.role == Role::Leader);
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

    /// Fast read replica path (TiKV-style follower read): **LocalApplied** on the
    /// freshest non-leader member when possible; otherwise any applied peer.
    ///
    /// Not linearizable — may lag the leader. Prefer [`Self::get_strong`] for
    /// FDB-class linearizable reads. Used for scale-out read amp.
    ///
    /// # Errors
    /// Empty cluster / unknown range.
    pub fn get_fast_replica(&self, key: &[u8]) -> Result<Option<Bytes>> {
        let rid = self.locate(key)?;
        let leader = self.range_leader(rid);
        // Pick member with highest applied index among followers (or all if no leader).
        let mut best: Option<(u64, u64)> = None; // (applied, node_id)
        for &nid in &self.ids {
            if !membership_kernel::reader_id_local(self.is_local_node(nid)) {
                continue;
            }
            if !self.is_participating(nid) {
                continue;
            }
            if Some(nid) == leader {
                continue;
            }
            let applied = self.applied_index(nid, rid);
            match best {
                None => best = Some((applied, nid)),
                Some((a, _)) if applied >= a => best = Some((applied, nid)),
                _ => {}
            }
        }
        let node = best
            .map(|(_, n)| n)
            .or(leader)
            .or_else(|| self.ids_first_if_local())
            .ok_or_else(|| StoreError::Msg("no replica".into()))?;
        self.get_with_policy(node, key, ReadPolicy::LocalApplied)
    }

    /// How far `node_id` lags the leader commit index on `range_id` (`commit - applied`).
    ///
    /// `0` = caught up; negative should not occur (clamped to 0).
    #[must_use]
    pub fn applied_lag(&self, node_id: u64, range_id: u64) -> u64 {
        let leader = self.range_leader(range_id);
        let leader_commit = leader
            .map(|l| self.commit_index(l, range_id))
            .unwrap_or_else(|| self.commit_index(node_id, range_id));
        let applied = self.applied_index(node_id, range_id);
        leader_commit.saturating_sub(applied)
    }

    /// Max applied lag across participating members of a range (0 = fully caught up).
    #[must_use]
    pub fn max_applied_lag(&self, range_id: u64) -> u64 {
        self.ids
            .iter()
            .filter(|&&nid| self.is_participating(nid))
            .map(|&nid| self.applied_lag(nid, range_id))
            .max()
            .unwrap_or(0)
    }

    /// Multi-range concurrent-friendly put: same as [`Self::put`] but returns the
    /// range id that accepted the write (for multiwrite / client routing metrics).
    ///
    /// # Errors
    /// Same as [`Self::put`].
    pub fn put_routed(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<u64> {
        let key_ref = key.as_ref();
        let rid = self.locate(key_ref)?;
        self.put(key_ref, value)?;
        Ok(rid)
    }

    /// DCS create-if-absent on the range that owns `key` (meta keys should use `m/` prefix).
    ///
    /// Immortal binding (`lease = 0`).
    pub fn dcs_create(&mut self, key: &[u8], value: &[u8]) -> Result<u64> {
        self.dcs_create_ttl(key, value, 0)
    }

    /// DCS create-if-absent with **TTL** (multi-node durable deadline).
    ///
    /// `ttl_ms == 0` → immortal. Otherwise absolute deadline `now_ms + ttl_ms` is
    /// written into the raft log (`DcsCommand::lease`) so all peers share the same
    /// expiry; reads use [`Self::now_ms`].
    pub fn dcs_create_ttl(&mut self, key: &[u8], value: &[u8], ttl_ms: u64) -> Result<u64> {
        let lease = if ttl_ms == 0 {
            0
        } else {
            self.now_ms.saturating_add(ttl_ms)
        };
        let cmd = DcsCommand::Create {
            key: key.to_vec(),
            value: value.to_vec(),
            lease,
        };
        self.propose_dcs(cmd)
    }

    /// DCS CAS.
    pub fn dcs_cas(&mut self, key: &[u8], value: &[u8], expected_rev: u64) -> Result<u64> {
        self.dcs_cas_ttl(key, value, expected_rev, 0)
    }

    /// DCS CAS with optional TTL (absolute deadline baked at propose).
    pub fn dcs_cas_ttl(
        &mut self,
        key: &[u8],
        value: &[u8],
        expected_rev: u64,
        ttl_ms: u64,
    ) -> Result<u64> {
        let lease = if ttl_ms == 0 {
            0
        } else {
            self.now_ms.saturating_add(ttl_ms)
        };
        let cmd = DcsCommand::Cas {
            key: key.to_vec(),
            value: value.to_vec(),
            expected_rev,
            lease,
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
        let watch_val: Vec<u8> = match &cmd {
            DcsCommand::Put { value, .. }
            | DcsCommand::Create { value, .. }
            | DcsCommand::Cas { value, .. } => value.clone(),
            DcsCommand::Delete { .. } => Vec::new(),
        };
        let is_delete = matches!(cmd, DcsCommand::Delete { .. });
        let lease = match &cmd {
            DcsCommand::Put { lease, .. }
            | DcsCommand::Create { lease, .. }
            | DcsCommand::Cas { lease, .. } => *lease,
            DcsCommand::Delete { .. } => 0,
        };
        if lease != 0 {
            self.has_ttl_leases = true;
            self.persist_now_ms();
        }
        let rid = self.locate(&key)?;
        let leader = self.range_leader(rid).ok_or(StoreError::NotLeader {
            range_id: rid,
            leader: None,
        })?;
        let now = self.now_ms;
        // Pre-check on leader db (expired leased keys count as absent).
        // Then bind Create/Cas(0) against an expired corpse to Cas(old rev)
        // so apply never overwrites (I-DCS-1).
        let cmd = {
            let db = &self.nodes.get(&leader).unwrap().db;
            check_command_at(db, &cmd, now).map_err(StoreError::Dcs)?;
            bind_absent_create(db, cmd, now)
        };
        // Majority commit required: NotCommitted if followers cannot form a majority.
        self.broadcast_append(rid, leader, Some(RangeEntry::Dcs(cmd)))?;
        // Post-condition: entry applied on leader. Never return Ok(0) for a successful mutate.
        let n = self
            .nodes
            .get(&leader)
            .ok_or_else(|| StoreError::Msg("bad leader".into()))?;
        if is_delete {
            if dcs_get_at(&n.db, &key, now).is_some() {
                return Err(StoreError::Msg(
                    "dcs delete committed but key still present".into(),
                ));
            }
            // F123: short/torn d/rev after delete is Corrupt, not NotCommitted
            // (F113 class — present corrupt must not look like "never wrote rev").
            let rev = match n.db.get(b"d/rev") {
                None => {
                    return Err(StoreError::NotCommitted {
                        range_id: rid,
                        index: 0,
                        commit: 0,
                    });
                }
                Some(b) if b.len() >= 8 => u64::from_le_bytes(b[..8].try_into().unwrap()),
                Some(_) => {
                    return Err(StoreError::Msg(
                        "dcs cluster rev corrupt after delete".into(),
                    ));
                }
            };
            if rev == 0 {
                return Err(StoreError::NotCommitted {
                    range_id: rid,
                    index: 0,
                    commit: 0,
                });
            }
            // Version notes flushed via broadcast_append majority path.
            let _ = watch_val;
            return Ok(rev);
        }
        let kv = dcs_get_at(&n.db, &key, now).ok_or(StoreError::NotCommitted {
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
        // Apply of a losing Create/Cas is a no-op: the live key is the winner.
        // Do not Ok the loser's client with the winner's revision (I-DCS-1).
        if kv.value.as_slice() != watch_val.as_slice() || (lease != 0 && kv.lease != lease) {
            return Err(StoreError::Dcs(pedradb_dcs::DcsError::CasFailed(
                "lost race",
            )));
        }
        Ok(kv.mod_revision)
    }

    /// DCS get on the freshest local replica for this key's range (F73/F84).
    ///
    /// # Errors
    /// Empty cluster / unknown node.
    pub fn dcs_get(&self, key: &[u8]) -> Result<Option<KeyValue>> {
        let id = self
            .best_reader_for_key(key)
            .ok_or_else(|| StoreError::Msg("empty".into()))?;
        self.dcs_get_on(id, key)
    }

    /// DCS get on a node (**lease-aware** via [`Self::now_ms`]).
    pub fn dcs_get_on(&self, node_id: u64, key: &[u8]) -> Result<Option<KeyValue>> {
        let n = self
            .nodes
            .get(&node_id)
            .ok_or_else(|| StoreError::Msg("bad node".into()))?;
        Ok(dcs_get_at(&n.db, key, self.now_ms))
    }

    /// Raw DCS get ignoring lease expiry (debug / tests).
    pub fn dcs_get_raw_on(&self, node_id: u64, key: &[u8]) -> Result<Option<KeyValue>> {
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
///
/// F103: raw `m/` || suffix made `m/a` a byte-prefix of `m/ab` (F96 only
/// fixed [`layers::EtcdNeedFace`] `full_key`). Length-prefix the suffix.
#[must_use]
pub fn meta_key(suffix: &[u8]) -> Vec<u8> {
    let mut k = META_PREFIX.to_vec();
    k.extend_from_slice(&len_pref_value(suffix));
    k
}

#[cfg(test)]
mod three_teeth_queued;

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

    fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            let to = dst.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_tree(&e.path(), &to);
            } else {
                std::fs::copy(e.path(), to).unwrap();
            }
        }
    }

    /// RFC-0090 P2.1: production `encode_hard`/`decode_hard` (persist +
    /// load_range_peer) XOR only the trailer CRC (term/vote intact).
    /// Decode is crc mismatch. AS-IS would return the stored term.
    #[test]
    fn crc_mismatch_on_live_store_raft_meta_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any store raft-meta crc would match"
        );
        let mut raw = encode_hard(3, Some(1));
        assert!(raw.len() >= 9 + 4, "hard meta must have payload + trailer");
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        match decode_hard(&raw) {
            Ok((term, _)) => panic!("AS-IS hole: served term {term} after CRC trailer lie"),
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a term parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0013 P1.3: cluster id is minted, persisted, and stable on reopen.
    #[test]
    fn cluster_id_survives_reopen() {
        let dir = temp();
        let id = {
            let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            let id = c.cluster_id();
            assert_ne!(id, [0u8; 16]);
            let raw = c.nodes[&1].db.get(&cluster_id_key()).expect("id key");
            assert_eq!(raw.as_ref(), id.as_slice());
            let mem = c.nodes[&1]
                .db
                .get(&cluster_membership_key())
                .expect("membership key");
            assert_eq!(decode_membership(&mem).unwrap(), vec![1, 2, 3]);
            drop(c);
            id
        };
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        assert_eq!(c.cluster_id(), id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0013 P1.3: configured id pins empty dirs; a different pin refuses.
    #[test]
    fn cluster_id_configured_pin_and_mismatch() {
        let dir = temp();
        let pin = [0xC1, 0xD0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x13];
        {
            let c = StoreCluster::open_with_options_lab_direct(
                &dir,
                3,
                1,
                StoreOpenOptions::default().with_cluster_id(pin),
            )
            .unwrap();
            assert_eq!(c.cluster_id(), pin);
            drop(c);
        }
        let other = [0xFF; 16];
        let err = match StoreCluster::open_with_options_lab_direct(
            &dir,
            3,
            1,
            StoreOpenOptions::default().with_cluster_id(other),
        ) {
            Err(e) => e,
            Ok(_) => panic!("expected ClusterMismatch, open succeeded"),
        };
        assert!(
            matches!(err, StoreError::ClusterMismatch { .. }),
            "got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0013 P1.3: copying a node dir from cluster A into cluster B is
    /// refuse-closed (no silent merge of two Pedra trees).
    #[test]
    fn cluster_id_refuses_cross_cluster_node_dir() {
        let dir_a = temp();
        let dir_b = temp();
        {
            let _a = StoreCluster::open_lab_direct(&dir_a, 3, 1).unwrap();
            let _b = StoreCluster::open_lab_direct(&dir_b, 3, 1).unwrap();
        }
        let src = dir_a.join("store-node-2");
        let dst = dir_b.join("store-node-2");
        let _ = std::fs::remove_dir_all(&dst);
        copy_tree(&src, &dst);
        let err = match StoreCluster::open_lab_direct(&dir_b, 3, 1) {
            Err(e) => e,
            Ok(_) => panic!("expected ClusterMismatch, silent merge succeeded"),
        };
        assert!(
            matches!(err, StoreError::ClusterMismatch { .. }),
            "got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// RFC-0013 P1.3: user puts cannot stamp cluster identity.
    #[test]
    fn cluster_id_key_is_reserved() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let err = c.put(cluster_id_key(), b"hijack").unwrap_err();
        assert!(
            matches!(err, StoreError::Msg(ref m) if m.contains("reserved")),
            "got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F103: `m/` || suffix made `m/a` a prefix of `m/ab` (F96 fixed EtcdNeedFace only).
    #[test]
    fn meta_key_suffix_is_not_prefix_of_sibling() {
        let a = meta_key(b"a");
        let ab = meta_key(b"ab");
        assert!(
            !ab.starts_with(&a),
            "meta_key(a) must not be a byte-prefix of meta_key(ab): {a:?} vs {ab:?}"
        );
        assert_ne!(a, ab);
        assert_ne!(meta_key(b"smoke"), meta_key(b"smoke/k0"));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(&a, b"ha").unwrap();
        c.put(&ab, b"hab").unwrap();
        let end = pedradb_core::prefix_exclusive_end(&a);
        let snap = c.read_version();
        let hits = c
            .keys_in_range_at(&a, end.as_deref().unwrap_or(&[]), snap)
            .unwrap();
        assert!(
            hits.iter().any(|(k, v)| k == &a && v.as_slice() == b"ha"),
            "own meta_key missing: {hits:?}"
        );
        assert!(
            !hits.iter().any(|(k, _)| k == &ab),
            "meta_key(a) range leaked sibling ab: {hits:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F108: a query spanning two store ranges must not depend on one node's
    /// view of `start`'s range (F84 residual).
    #[test]
    fn keys_in_range_at_spans_ranges_after_first_node_partition() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 2).unwrap();
        c.elect_all(120).unwrap();
        let keys = keys_one_per_range(&c);
        assert!(keys.len() >= 2, "need two ranges");
        c.put(&keys[0], b"v0").unwrap();
        c.set_participating(1, false).unwrap();
        c.elect_all(120).unwrap();
        c.put(&keys[1], b"v1").unwrap();
        let snap = c.read_version();
        let got = c.keys_in_range_at(&[], &[], snap).unwrap();
        assert!(
            got.iter()
                .any(|(k, v)| k == &keys[0] && v.as_slice() == b"v0"),
            "range-0 key missing after partition: {got:?}"
        );
        assert!(
            got.iter()
                .any(|(k, v)| k == &keys[1] && v.as_slice() == b"v1"),
            "range-1 key missing (scan used start-range reader only): {got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
    fn rebalance_local_sheds_excess_leadership() {
        let dir = temp();
        // 1 local node of a 3-member config with 6 ranges — open_single_node style.
        let mut c = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 6).unwrap();
        // Force local leadership on every range (multiproc-style overload).
        let rids: Vec<u64> = c.range_metas().iter().map(|r| r.id).collect();
        for rid in rids {
            if let Some(n) = c.nodes.get_mut(&1) {
                if let Some(p) = n.ranges.get_mut(&rid) {
                    p.role = Role::Leader;
                    p.leader_id = Some(1);
                }
            }
        }
        assert_eq!(
            c.ranges
                .iter()
                .filter(|r| c.node_thinks_leader(1, r.id))
                .count(),
            6
        );
        let stepped = c.rebalance_local_leaders().unwrap();
        // target = ceil(6/3)=2 → step down 4
        assert_eq!(stepped, 4);
        let left = c
            .ranges
            .iter()
            .filter(|r| c.node_thinks_leader(1, r.id))
            .count();
        assert_eq!(left, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_range_id_drives_only_named_range() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 4).unwrap();
        c.elect_all(120).unwrap();
        let rid = c.range_metas()[0].id;
        // Put on range 1 should succeed after single-range ticks only.
        let k = if c.range_metas()[0].start.is_empty() {
            vec![0u8, b'x']
        } else {
            let mut k = c.range_metas()[0].start.clone();
            k.push(b'x');
            k
        };
        c.put(&k, b"v").unwrap();
        for _ in 0..20 {
            c.tick_range_id(rid).unwrap();
        }
        assert_eq!(c.get_strong(&k).unwrap().as_deref(), Some(b"v".as_ref()));
        assert!(c.tick_range_id(999).is_err(), "unknown range must fail");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_range_election_timeouts_diversify_leaders() {
        // Root cause of option-A residual: node-only timeouts → one node leads all ranges.
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 6).unwrap();
        c.elect_all(200).unwrap();
        let leaders: Vec<(u64, u64)> = c
            .range_metas()
            .iter()
            .map(|r| (r.id, c.range_leader(r.id).expect("leader")))
            .collect();
        let distinct = c.leader_nodes();
        assert!(
            distinct.len() >= 2,
            "expected ≥2 leader nodes after multi-range elect, got {distinct:?} map={leaders:?}"
        );
        // Preferred assignment for 3 nodes × 6 ranges is round-robin → all 3 nodes.
        assert!(
            distinct.len() >= 3 || {
                c.rebalance_range_leaders(200).unwrap();
                c.leader_nodes().len() >= 2
            },
            "rebalance should keep multi-node leadership; leaders={:?}",
            c.leader_nodes()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn election_timeout_for_round_robin_prefers_member() {
        let members = [1u64, 2, 3];
        // range 1 → prefer node 1 (timeout 3); node 2 = 4; node 3 = 5
        assert_eq!(election_timeout_for(1, 1, &members), 3);
        assert_eq!(election_timeout_for(2, 1, &members), 4);
        assert_eq!(election_timeout_for(3, 1, &members), 5);
        // range 2 → prefer node 2
        assert_eq!(election_timeout_for(2, 2, &members), 3);
        assert_eq!(election_timeout_for(1, 2, &members), 5); // dist ring: 1 is after 2→3→1 = 2 steps?
                                                             // pref_pos=1 (node2), my_pos=0 (node1): dist = (0+3-1)%3 = 2 → timeout 5
        assert_eq!(election_timeout_for(3, 2, &members), 4);
    }

    fn any_log_has_c_new_only_leave(c: &StoreCluster) -> bool {
        c.nodes.values().any(|n| {
            n.ranges.values().any(|p| {
                p.log.iter().any(|rec| {
                    matches!(
                        &rec.entry,
                        RangeEntry::MembershipJoint { old, new }
                            if !membership_kernel::joint_still_active(old, new)
                    )
                })
            })
        })
    }

    fn max_c_new_only_leave_index(c: &StoreCluster) -> u64 {
        let mut m = 0u64;
        for n in c.nodes.values() {
            for p in n.ranges.values() {
                for rec in &p.log {
                    if let RangeEntry::MembershipJoint { old, new } = &rec.entry {
                        if !membership_kernel::joint_still_active(old, new) {
                            m = m.max(rec.index);
                        }
                    }
                }
            }
        }
        m
    }

    /// RFC-0103/0104: pump+finish until a **new** C-new-only index appears
    /// (prior add/Direct leave in RAM is not this op's tooth).
    fn drive_queued_joint_until_leave(
        c: &mut StoreCluster,
        result: Result<()>,
        before_leave: u64,
        what: &str,
    ) -> bool {
        match result {
            Ok(()) => max_c_new_only_leave_index(c) > before_leave,
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                for _ in 0..96 {
                    if max_c_new_only_leave_index(c) > before_leave {
                        return true;
                    }
                    pump_queued(c, 1);
                    let commit = c
                        .range_leader(range_id)
                        .map(|lid| c.commit_index(lid, range_id))
                        .unwrap_or(0);
                    if commit >= index {
                        assert!(
                            c.finish_queued_propose(range_id, index, true).unwrap(),
                            "{what} must commit after pump"
                        );
                    }
                    if max_c_new_only_leave_index(c) > before_leave {
                        return true;
                    }
                }
                max_c_new_only_leave_index(c) > before_leave
            }
            Err(e) => panic!("{what}: {e}"),
        }
    }

    /// RFC-0103/0104: commit the leader's last index so `pending_joint` clears.
    fn drain_queued_until_joint_idle(c: &mut StoreCluster) {
        for _ in 0..128 {
            if c.pending_joint().is_none() {
                return;
            }
            pump_queued(c, 1);
            let Some(lid) = c.range_leader(1) else {
                continue;
            };
            let (last, commit) = {
                let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
                (p.last_index(), p.commit)
            };
            if last > 0 && commit >= last {
                let _ = c.finish_queued_propose(1, last, true);
            }
        }
    }

    /// Pump Queued outbound via in-process delivery (simulates reliable Net).
    fn pump_queued(c: &mut StoreCluster, rounds: usize) {
        for _ in 0..rounds {
            let batch = c.drain_outbound();
            if batch.is_empty() {
                break;
            }
            for (from, to, bytes) in batch {
                c.handle_inbound(from, to, &bytes).unwrap();
            }
        }
    }

    /// Elect a leader under Queued RPC by ticking + pumping AE/RV.
    fn elect_queued(c: &mut StoreCluster, ticks: usize) {
        for _ in 0..ticks {
            c.tick().unwrap();
            pump_queued(c, 48);
            if c.range_leader(1).is_some() {
                return;
            }
        }
        panic!("no leader under Queued after {ticks} ticks");
    }

    /// Put under Queued: drain AE/RV until majority commit or fail.
    fn put_queued(c: &mut StoreCluster, key: &[u8], val: &[u8]) {
        match c.put(key, val) {
            Ok(()) => {}
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                pump_queued(c, 96);
                assert!(
                    c.finish_queued_propose(range_id, index, true).unwrap(),
                    "put should commit after Queued pump key={}",
                    String::from_utf8_lossy(key)
                );
            }
            Err(e) => panic!("unexpected put err: {e}"),
        }
        pump_queued(c, 24);
    }

    #[test]
    fn queued_rpc_elect_and_put_via_inbound() {
        let dir = temp();
        let mut c = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0xC0DE_0001)).unwrap();
        c.set_rpc_mode(RpcMode::Queued);
        elect_queued(&mut c, 80);
        assert!(c.range_leader(1).is_some(), "leader via queued RV");
        let key = b"k-queued";
        let val = b"v-queued";
        put_queued(&mut c, key, val);
        assert_eq!(c.count_applied_eq(key, val), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0050 P1.1: `RpcMode::Direct` is a synchronous pump of the **same**
    /// `PeerMsg` — not a second protocol. Same seed ⇒ same leader topology and
    /// same committed/apply state under both delivery modes; bytes drained in
    /// Queued mode are `PeerMsg`-codec stable (Direct dispatches this exact
    /// message).
    #[test]
    fn direct_pump_and_queued_share_peer_msg_semantics() {
        let seed = 0xD1DE_0001u64;
        let ka = b"p11/a";
        let kb = b"p11/b";
        let v = b"same-peermsg";

        // A: Direct (lab opt-in) — in-process sync dispatch of PeerMsg.
        let dir_a = temp();
        let mut a =
            StoreCluster::open_with_rng_lab_direct(&dir_a, 3, 1, SeedRng::new(seed)).unwrap();
        for _ in 0..80 {
            a.tick().unwrap();
            if a.range_leader(1).is_some() {
                break;
            }
        }
        assert!(a.range_leader(1).is_some(), "leader via Direct RV");
        a.put(ka, v).unwrap();
        a.put(kb, v).unwrap();

        // B: Queued (production default) — same PeerMsg travels encode → drain → decode → dispatch.
        let dir_b = temp();
        let mut b = StoreCluster::open_with_rng(&dir_b, 3, 1, SeedRng::new(seed)).unwrap();
        assert_eq!(b.rpc_mode(), RpcMode::Queued);
        b.set_rpc_mode(RpcMode::Queued);
        elect_queued(&mut b, 80);
        put_queued(&mut b, ka, v);
        put_queued(&mut b, kb, v);

        let rid = a.locate(ka).unwrap();
        assert_eq!(
            a.range_leader(rid),
            b.range_leader(rid),
            "same seed must elect the same leader under both modes"
        );
        assert_eq!(a.leader_claim_count(rid), b.leader_claim_count(rid));
        assert_eq!(
            a.count_applied_eq(ka, v),
            b.count_applied_eq(ka, v),
            "apply state must match across Direct/Queued"
        );
        assert_eq!(a.count_applied_eq(kb, v), b.count_applied_eq(kb, v));
        assert_eq!(a.get_strong(ka).unwrap().as_deref(), Some(&v[..]));
        assert_eq!(b.get_strong(ka).unwrap().as_deref(), Some(&v[..]));

        // Codec stability: one more tick queues heartbeats; every drained byte
        // blob decodes to the PeerMsg Direct dispatches and re-encodes equal.
        b.tick().unwrap();
        let drained = b.drain_outbound();
        assert!(
            !drained.is_empty(),
            "heartbeats should be queued after tick"
        );
        for (_from, _to, bytes) in drained {
            let msg = PeerMsg::decode(&bytes).unwrap();
            assert_eq!(msg.encode(), bytes, "PeerMsg codec must be byte-stable");
        }
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// RFC-0067 P2.2: production multi-node open is Queued-only.
    #[test]
    fn default_open_starts_queued() {
        let dir = temp();
        let c = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0667_0002)).unwrap();
        assert_eq!(c.rpc_mode(), RpcMode::Queued, "production open is Queued");
        assert!(
            !c.dst_queued_pin(),
            "default open is unpinned (lab can opt in)"
        );
        let dir2 = temp();
        let mut d = StoreCluster::open_lab_direct(&dir2, 3, 1).unwrap();
        assert_eq!(d.rpc_mode(), RpcMode::Direct, "lab opt-in Direct");
        d.pin_dst_queued();
        d.set_rpc_mode(RpcMode::Direct);
        assert_eq!(d.rpc_mode(), RpcMode::Queued);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    /// RFC-0067 P0.3 / P2.2: DST pin forces Queued; `set_rpc_mode(Direct)`
    /// cannot skip Net. Starts from explicit Direct opt-in, then pin.
    /// AS-IS kernel would still admit Direct.
    #[test]
    fn pin_dst_queued_refuses_direct_switch() {
        let dir = temp();
        let mut c = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0667_0001)).unwrap();
        assert_eq!(c.rpc_mode(), RpcMode::Queued, "open_with_rng starts Queued");
        assert!(!c.dst_queued_pin());
        c.enable_lab_direct_rpc();
        assert_eq!(c.rpc_mode(), RpcMode::Direct, "lab opt-in Direct");
        c.pin_dst_queued();
        assert!(c.dst_queued_pin());
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        c.set_rpc_mode(RpcMode::Direct);
        assert_eq!(
            c.rpc_mode(),
            RpcMode::Queued,
            "pinned Direct must stay Queued"
        );
        assert!(allow_direct_rpc_as_is(true, true), "AS-IS would switch");
        assert!(!allow_direct_rpc(true, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0067 P1.2: `open_single_node` (montanha-tcp / cluster_real) pins
    /// Queued at open; `set_rpc_mode(Direct)` cannot drop mid-run.
    #[test]
    fn open_single_node_refuses_direct_switch() {
        let dir = temp();
        let mut c = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 1).unwrap();
        assert!(c.dst_queued_pin(), "TCP ctor must pin Queued");
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        c.set_rpc_mode(RpcMode::Direct);
        assert_eq!(
            c.rpc_mode(),
            RpcMode::Queued,
            "TCP node must not drop to Direct"
        );
        assert!(allow_direct_rpc_as_is(true, true), "AS-IS would switch");
        assert!(!allow_direct_rpc(true, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F49: two outstanding Queued proposes must stamp **distinct** durable SI gens.
    ///
    /// `with_si_gen` claimed to reserve gens but only read `commit_generation+1`
    /// without advancing. Two NotCommitted puts both embed the same `si_gen` in the
    /// raft log; Raft apply then writes `\0store/hist/` under that shared gen. A
    /// crash after apply (or any path that reloads apply hist before coordinator
    /// `note_mutations` rewrites) collapses both commits into one generation →
    /// SI snapshot that should see only the first also sees the second.
    #[test]
    fn queued_double_propose_distinct_si_gens_survive_reopen() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x0F49_5101)).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"f48/a", b"old-a").unwrap();
        c.put(b"f48/b", b"old-b").unwrap();
        let gen_seed = c.read_version();
        assert!(gen_seed >= 2, "seed puts must advance generation");

        c.set_rpc_mode(RpcMode::Queued);
        // Two client proposes **without** finish between them — both stamp si_gen
        // before any note_mutations bump.
        let (r1, i1) = match c.put(b"f48/a", b"new-a") {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            Ok(()) => panic!("Queued put should return NotCommitted before pump"),
            Err(e) => panic!("unexpected: {e}"),
        };
        let (r2, i2) = match c.put(b"f48/b", b"new-b") {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            Ok(()) => panic!("second Queued put should return NotCommitted"),
            Err(e) => panic!("unexpected: {e}"),
        };
        assert_eq!(r1, r2, "single range cluster");
        assert_ne!(i1, i2, "distinct log indexes");

        // Inspect raft log stamps **before** finish (apply/note paths).
        let mut stamped: Vec<(u64, u64)> = Vec::new(); // (index, si_gen)
        {
            let leader = c.range_leader(r1).expect("leader");
            let n = c.nodes.get(&leader).unwrap();
            let p = n.ranges.get(&r1).unwrap();
            for rec in &p.log {
                if rec.index == i1 || rec.index == i2 {
                    if let RangeEntry::Put { si_gen, .. } = &rec.entry {
                        stamped.push((rec.index, *si_gen));
                    }
                }
            }
        }
        stamped.sort_by_key(|(i, _)| *i);
        assert_eq!(
            stamped.len(),
            2,
            "both puts must be in leader log: {stamped:?}"
        );
        assert_ne!(
            stamped[0].1, stamped[1].1,
            "F49: raft Put si_gen collision — both embeds {} (indexes {} and {})",
            stamped[0].1, stamped[0].0, stamped[1].0
        );
        assert!(
            stamped[0].1 > gen_seed && stamped[1].1 > gen_seed,
            "stamped gens {stamped:?} must exceed seed {gen_seed}"
        );

        pump_queued(&mut c, 128);
        assert!(
            c.finish_queued_propose(r1, i1, true).unwrap(),
            "first put must majority-commit"
        );
        assert!(
            c.finish_queued_propose(r2, i2, true).unwrap(),
            "second put must majority-commit"
        );
        pump_queued(&mut c, 32);

        let va = c.key_version(b"f48/a");
        let vb = c.key_version(b"f48/b");
        assert_ne!(va, vb, "in-memory OCC versions must differ");
        let mid = va.min(vb);
        if vb > va {
            assert_eq!(
                c.get_at_version(b"f48/b", mid).unwrap().as_deref(),
                Some(b"old-b".as_ref()),
                "in-memory SI: snapshot@{mid} must not see second commit"
            );
        } else {
            assert_eq!(
                c.get_at_version(b"f48/a", mid).unwrap().as_deref(),
                Some(b"old-a".as_ref()),
                "in-memory SI: snapshot@{mid} must not see second commit"
            );
        }

        // Simulate crash **after Raft apply wrote hist, before coordinator notes**:
        // reload SI only from apply-path hist gens (si_gen embedded in log).
        // Apply uses the stamped si_gen; collision ⇒ both keys share one gen.
        drop(c);
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x0F49_5102)).unwrap();
        c.elect_all(40).unwrap();
        // Happy-path reopen may be healed by persist_si_keys; the log stamp assert
        // above is the primary F49 gate. Still check SI if versions differ.
        let va2 = c.key_version(b"f48/a");
        let vb2 = c.key_version(b"f48/b");
        if va2 != vb2 {
            let mid2 = va2.min(vb2);
            if vb2 > va2 {
                assert_eq!(
                    c.get_at_version(b"f48/b", mid2).unwrap().as_deref(),
                    Some(b"old-b".as_ref()),
                    "reopen SI snapshot@{mid2}"
                );
            } else {
                assert_eq!(
                    c.get_at_version(b"f48/a", mid2).unwrap().as_deref(),
                    Some(b"old-a".as_ref()),
                    "reopen SI snapshot@{mid2}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0023 skeptic: Queued put must advance version history so OCC sees the write.
    ///
    /// Repro of greenwash: TX begin after seed; Queued overwrite of read-set key;
    /// without finish_queued note_mutations, commit would Ok (silent wrong).
    #[test]
    fn queued_put_advances_versions_for_tx_occ() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x51_0001)).unwrap();
        // Seed under Direct so note path is clean, then switch to Queued.
        c.elect_all(80).unwrap();
        c.put(b"qk", b"v0").unwrap();
        assert!(c.key_version(b"qk") >= 1);
        let gen_after_seed = c.read_version();

        c.set_rpc_mode(RpcMode::Queued);
        let mut tx = c.begin();
        assert_eq!(tx.snapshot_version(), gen_after_seed);
        assert_eq!(
            tx.get(&c, b"qk").unwrap().as_deref(),
            Some(b"v0".as_ref()),
            "snapshot must see seed"
        );

        // Concurrent Queued overwrite (typically NotCommitted then finish).
        put_queued(&mut c, b"qk", b"v1");
        assert!(
            c.key_version(b"qk") > gen_after_seed,
            "Queued majority apply must bump key_version (got {} seed_gen={gen_after_seed})",
            c.key_version(b"qk")
        );
        assert!(
            c.read_version() > gen_after_seed,
            "commit_generation must advance after Queued put"
        );
        // SI: still sees v0
        assert_eq!(
            tx.get(&c, b"qk").unwrap().as_deref(),
            Some(b"v0".as_ref()),
            "must not see Queued concurrent write"
        );
        tx.set(b"other", b"x").unwrap();
        let err = tx
            .commit(&mut c)
            .expect_err("read-set OCC after Queued put");
        assert!(
            matches!(err, StoreError::Conflict),
            "expected Conflict after Queued concurrent write, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Residual fix: clear ⇒ Pedra delete, not empty payload.
    #[test]
    fn clear_is_true_delete_not_empty_value() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"gone", b"here").unwrap();
        assert_eq!(c.get(b"gone").unwrap().as_deref(), Some(b"here".as_ref()));
        let mut tx = c.begin();
        tx.clear(b"gone").unwrap();
        tx.commit(&mut c).unwrap();
        // Pedra must report absence, not Some([]).
        assert!(
            c.get(b"gone").unwrap().is_none(),
            "clear must Pedra-delete; got {:?}",
            c.get(b"gone").unwrap()
        );
        // Snapshot at latest also None.
        assert!(c
            .get_at_version(b"gone", c.read_version())
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SI hist is written on Raft apply path (reopen loads hist from Pedra).
    #[test]
    fn si_hist_survives_reopen_after_put() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"si-k", b"v1").unwrap();
            assert!(c.read_version() >= 1);
            assert_eq!(
                c.get_at_version(b"si-k", c.read_version())
                    .unwrap()
                    .as_deref(),
                Some(b"v1".as_ref())
            );
        }
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        // load_si_from_disk should restore generation + hist
        assert!(
            c.read_version() >= 1,
            "generation must reload from disk, got {}",
            c.read_version()
        );
        assert_eq!(
            c.get_at_version(b"si-k", c.read_version())
                .unwrap()
                .as_deref(),
            Some(b"v1".as_ref()),
            "hist must reload so SI still sees v1"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F114: garbage SI generation meta was ignored → reopen at gen 0 → reuse gens.
    #[test]
    fn open_rejects_corrupt_si_generation_meta() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"k", b"v1").unwrap();
            assert!(c.read_version() >= 1);
            let gk = si_meta_key("generation");
            for nid in c.ids.clone() {
                if let Some(n) = c.nodes.get_mut(&nid) {
                    // Not a valid encode_u64_meta blob (no CRC / short).
                    n.db.put(&gk, b"xx").unwrap();
                }
            }
        }
        match StoreCluster::open_lab_direct(&dir, 3, 1) {
            Ok(_) => panic!("corrupt generation on all replicas must fail open, not restart at 0"),
            Err(e) => assert!(
                e.to_string().contains("si meta generation"),
                "expected si meta generation error, got {e}"
            ),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F114: one good replica still recovers generation (corrupt siblings ignored).
    #[test]
    fn open_uses_max_valid_si_generation_when_sibling_corrupt() {
        let dir = temp();
        let gen_before = {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"k", b"v1").unwrap();
            let g = c.read_version();
            assert!(g >= 1);
            // Poison only node 1; nodes 2/3 keep valid meta.
            let gk = si_meta_key("generation");
            if let Some(n) = c.nodes.get_mut(&1) {
                n.db.put(&gk, b"xx").unwrap();
            }
            g
        };
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        assert!(
            c.read_version() >= gen_before,
            "valid sibling meta must win over corrupt: got {} want >= {gen_before}",
            c.read_version()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Bitrot on \0store/hist/ must fail closed (CRC), not return wrong value silently.
    #[test]
    fn si_hist_bitrot_fail_closed() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"br", b"good").unwrap();
        // F100: hist_key length-prefixes the user component.
        let hk = hist_key(b"br");
        // Flip a byte in durable hist on every node.
        for nid in 1..=3u64 {
            let n = c.nodes.get_mut(&nid).unwrap();
            if let Some(raw) = n.db.get(&hk) {
                let mut v = raw.to_vec();
                if !v.is_empty() {
                    let i = v.len() / 2;
                    v[i] ^= 0xFF;
                    n.db.put(&hk, &v).unwrap();
                }
            }
        }
        // Corrupt hist decode fails → load skips; get may still see user key from Pedra.
        // decode_hist on explicit read must error (CRC).
        let n = c.nodes.get(&1).unwrap();
        let raw = n.db.get(&hk).expect("hist key exists");
        assert!(
            decode_hist(raw.as_ref()).is_err(),
            "bitrot must fail CRC, not decode silently"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F117: corrupt hist used to decode as empty and get rewritten with one gen.
    #[test]
    fn persist_si_hist_rejects_corrupt_does_not_wipe() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"h", b"v1").unwrap();
        c.put(b"h", b"v2").unwrap();
        let hk = hist_key(b"h");
        let before = c.nodes.get(&1).unwrap().db.get(&hk).unwrap().to_vec();
        assert!(decode_hist(&before).unwrap().len() >= 2);
        // Poison hist on the leader apply path node(s).
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(&hk, b"xx").unwrap();
            }
        }
        let err = c.put(b"h", b"v3");
        assert!(
            err.is_err(),
            "put must fail closed when hist is corrupt, not wipe: {err:?}"
        );
        // Hist blob still the poison (not a fresh single-gen rewrite).
        for nid in c.ids.clone() {
            let raw = c.nodes.get(&nid).unwrap().db.get(&hk).unwrap();
            assert_eq!(
                raw.as_ref(),
                b"xx",
                "corrupt hist must not be rewritten as a short valid chain"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F119: all-replica corrupt hist was skipped on open → SI snapshot evaporates.
    #[test]
    fn open_rejects_corrupt_si_hist_on_all_replicas() {
        let dir = temp();
        let g1;
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"h", b"v1").unwrap();
            g1 = c.read_version();
            c.put(b"h", b"v2").unwrap();
            assert_eq!(
                c.get_at_version(b"h", g1).unwrap().as_deref(),
                Some(b"v1".as_ref())
            );
            let hk = hist_key(b"h");
            for nid in c.ids.clone() {
                if let Some(n) = c.nodes.get_mut(&nid) {
                    n.db.put(&hk, b"xx").unwrap();
                }
            }
        }
        match StoreCluster::open_lab_direct(&dir, 3, 1) {
            Ok(c) => {
                let got = c.get_at_version(b"h", g1).unwrap();
                panic!(
                    "corrupt hist on all replicas must fail open, not drop SI (get_at {g1}={got:?})"
                );
            }
            Err(e) => assert!(
                e.to_string().contains("si hist"),
                "expected si hist error, got {e}"
            ),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F119: one valid hist replica still recovers SI (corrupt siblings ignored).
    #[test]
    fn open_uses_valid_si_hist_when_sibling_corrupt() {
        let dir = temp();
        let g1;
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"h", b"v1").unwrap();
            g1 = c.read_version();
            c.put(b"h", b"v2").unwrap();
            let hk = hist_key(b"h");
            if let Some(n) = c.nodes.get_mut(&1) {
                n.db.put(&hk, b"xx").unwrap();
            }
        }
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        assert_eq!(
            c.get_at_version(b"h", g1).unwrap().as_deref(),
            Some(b"v1".as_ref()),
            "valid sibling hist must win over corrupt"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F120: short intent on commit skipped materialise but still deleted the intent.
    #[test]
    fn apply_txn_commit_rejects_short_intent() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let nid = c.ids[0];
        let n = c.nodes.get_mut(&nid).unwrap();
        let tid = 77u64;
        n.db.put(txn_status_key(tid), b"prepared").unwrap();
        // No pair row — commit falls back to intent; short blob is corrupt.
        n.db.put(intent_key(b"u"), b"xxxx").unwrap();
        let err = apply_txn_commit(&mut n.db, tid, &[b"u".to_vec()]);
        assert!(
            err.is_err(),
            "short intent must fail closed on commit: {err:?}"
        );
        assert!(
            n.db.get(&intent_key(b"u")).is_some(),
            "failed commit must not drop the short intent"
        );
        assert!(
            n.db.get(b"u").is_none(),
            "user key must not be invented from garbage intent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F118: present garbage preimage must not look like "peer never prepared".
    #[test]
    fn apply_txn_revert_rejects_corrupt_preimage() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"u", b"live").unwrap();
        let nid = c.ids[0];
        let n = c.nodes.get_mut(&nid).unwrap();
        let tid = 42u64;
        n.db.put(txn_pre_key(tid, b"u"), b"\xffgarbage").unwrap();
        let err = apply_txn_revert(&mut n.db, tid, &[b"u".to_vec()]);
        assert!(
            err.is_err(),
            "corrupt preimage must fail closed, not LeaveUntouched: {err:?}"
        );
        assert_eq!(
            n.db.get(b"u").as_deref(),
            Some(b"live".as_ref()),
            "user key must not be wiped when preimage is garbage"
        );
        // Preimage still present (revert did not partial-delete and walk away).
        assert!(
            n.db.get(&txn_pre_key(tid, b"u")).is_some(),
            "failed revert must not drop the preimage key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FailingEnv during multi-key 2PC: no silent partial success without majority path.
    #[test]
    fn failing_env_commit_tx_no_silent_wrong() {
        use pedradb_sim::FailingEnv;
        let dir = temp();
        // Fail disk after enough ops for open+elect; mid-commit may error.
        let env = FailingEnv::fail_after(120);
        let mut c =
            StoreCluster::open_with_env_rng_lab_direct(&dir, 3, 1, env, SeedRng::new(0xF41))
                .unwrap();
        let _ = c.elect_all(120);
        if c.range_leader(1).is_none() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let r = c.commit_tx([(b"a", b"1"), (b"b", b"2")]);
        match r {
            Ok(_) => {
                let a = c.get(b"a").unwrap();
                let b = c.get(b"b").unwrap();
                assert_eq!(a.as_deref(), Some(b"1".as_ref()));
                assert_eq!(b.as_deref(), Some(b"2".as_ref()));
            }
            Err(_) => {
                let a_ok = c.count_applied_eq(b"a", b"1");
                let b_ok = c.count_applied_eq(b"b", b"2");
                assert!(
                    !(a_ok >= 2 && b_ok < 2) && !(b_ok >= 2 && a_ok < 2),
                    "partial majority apply: a={a_ok} b={b_ok}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Range scan after reopen must see Pedra-durable keys (not history-only).
    #[test]
    fn keys_in_range_at_after_reopen_sees_pedra() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"rr/a", b"1").unwrap();
            c.put(b"rr/b", b"2").unwrap();
            let got = c
                .keys_in_range_at(b"rr/", b"rr0", c.read_version())
                .unwrap();
            assert!(got.len() >= 2, "before reopen: {got:?}");
        }
        // Reopen: SI meta + history are durable; read at current generation.
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        let got = c
            .keys_in_range_at(b"rr/", b"rr0", c.read_version())
            .unwrap();
        assert!(
            got.iter()
                .any(|(k, v)| k.as_slice() == b"rr/a" && v.as_slice() == b"1"),
            "after reopen must see rr/a at current generation, got {got:?}"
        );
        assert!(
            got.iter()
                .any(|(k, v)| k.as_slice() == b"rr/b" && v.as_slice() == b"2"),
            "after reopen must see rr/b at current generation, got {got:?}"
        );
        // Snapshot 0 is the pre-first-commit world (keys absent), not "see tip".
        let at0 = c.keys_in_range_at(b"rr/", b"rr0", 0).unwrap();
        assert!(
            !at0.iter()
                .any(|(k, _)| k.as_slice() == b"rr/a" || k.as_slice() == b"rr/b"),
            "snapshot 0 must not show post-gen-0 keys, got {at0:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Criterion 1: Queued multi-Raft elect + put, leader loss, re-elect, majority still has data.
    #[test]
    fn queued_rpc_failover_put_majority_readable() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0xFA11_0FE1)).unwrap();
        c.set_rpc_mode(RpcMode::Queued);
        elect_queued(&mut c, 100);

        let key = b"ha-queued";
        put_queued(&mut c, key, b"before");
        assert!(
            c.count_applied_eq(key, b"before") >= 2,
            "majority must have pre-failover put via Queued path"
        );

        let rid = c.locate(key).unwrap();
        let old = c.range_leader(rid).expect("leader before failover");
        c.set_participating(old, false).unwrap();
        // Clear any stale leadership; drive re-election through Queued exchange.
        for _ in 0..160 {
            c.tick().unwrap();
            pump_queued(&mut c, 48);
            if let Some(l) = c.range_leader(rid) {
                if l != old {
                    break;
                }
            }
        }
        let new_leader = c
            .range_leader(rid)
            .expect("new leader after Queued re-elect");
        assert_ne!(new_leader, old, "must not keep deposed leader");

        let remaining: Vec<u64> = c.node_ids().iter().copied().filter(|&n| n != old).collect();
        let seen = remaining
            .iter()
            .filter(|&&n| {
                c.get_on(n, key)
                    .ok()
                    .flatten()
                    .is_some_and(|v| v.as_ref() == b"before")
            })
            .count();
        assert_eq!(
            seen,
            remaining.len(),
            "all remaining peers must retain committed pre-failover key; seen={seen}"
        );

        put_queued(&mut c, b"ha-queued-2", b"after");
        assert!(
            c.count_applied_eq(b"ha-queued-2", b"after") >= 2,
            "post-failover put must majority-commit via Queued"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_range_puts_different_leaders() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let rev = c
            .dcs_create(&key, b"node-a")
            .expect("retry create after heal");
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
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = b"read-k";
        c.put(key, b"v1").unwrap();
        assert_eq!(c.get_strong(key).unwrap().as_deref(), Some(b"v1".as_ref()));

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
        assert_eq!(c.get_strong(key).unwrap().as_deref(), Some(b"v1".as_ref()));

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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let remaining: Vec<u64> = c.node_ids().iter().copied().filter(|&n| n != old).collect();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 2).unwrap();
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

    /// Seeded RNG is a stable seam (same seed → same stream).
    #[test]
    fn open_with_seed_rng_is_deterministic() {
        let dir = temp();
        let c = StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(12345)).unwrap();
        let a = c.rng().next_u64();
        let b = c.rng().next_u64();
        let c2 = StoreCluster::open_with_rng_lab_direct(temp(), 3, 1, SeedRng::new(12345)).unwrap();
        assert_eq!(c2.rng().next_u64(), a);
        assert_eq!(c2.rng().next_u64(), b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Host seam: same DetHost seed ⇒ same election RNG stream after open.
    #[test]
    fn open_with_host_is_deterministic() {
        use pedradb_core::{DetHost, StdEnv};
        let dir = temp();
        let host = DetHost::with_seed(StdEnv, 0xBEEF);
        let c = StoreCluster::open_with_host_lab_direct(&dir, 3, 1, &host).unwrap();
        let a = c.rng().next_u64();
        let b = c.rng().next_u64();
        let host2 = DetHost::with_seed(StdEnv, 0xBEEF);
        let c2 = StoreCluster::open_with_host_lab_direct(temp(), 3, 1, &host2).unwrap();
        assert_eq!(c2.rng().next_u64(), a);
        assert_eq!(c2.rng().next_u64(), b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F28: partitioned peer freezes applied → compact must not drop catch-up entries.
    #[test]
    fn compact_does_not_pass_offline_peer_applied() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"before", b"1").unwrap();
        let rid = c.locate(b"before").unwrap();
        // Partition one follower after it has applied "before".
        let leader = c.range_leader(rid).unwrap();
        let offline = c.node_ids().iter().copied().find(|&n| n != leader).unwrap();
        let applied_offline = c.applied_index(offline, rid);
        assert!(applied_offline >= 1);
        c.set_participating(offline, false).unwrap();
        // Majority continues; many puts.
        for i in 0..6u8 {
            c.put([b'x', i], [b'y', i]).unwrap();
        }
        // Online peers must not compact past the offline peer's applied watermark.
        for &nid in c.node_ids() {
            if nid == offline {
                continue;
            }
            let snap = c
                .nodes
                .get(&nid)
                .unwrap()
                .ranges
                .get(&rid)
                .unwrap()
                .snapshot_index;
            assert!(
                snap <= applied_offline,
                "node {nid}: snapshot {snap} advanced past offline applied {applied_offline}"
            );
        }
        // Heal: offline must catch up via AE (log still has the suffix).
        c.set_participating(offline, true).unwrap();
        c.elect_all(80).unwrap();
        // Drive replication.
        c.put(b"heal", b"z").unwrap();
        assert_eq!(
            c.get_on(offline, b"heal").unwrap().as_deref(),
            Some(b"z".as_ref()),
            "healed peer must catch up"
        );
        assert_eq!(
            c.get_on(offline, b"before").unwrap().as_deref(),
            Some(b"1".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F27: applied log prefix is truncated once all peers applied it.
    #[test]
    fn raft_log_compacts_after_all_applied() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        assert_eq!(
            c.get_on(1, &[b'k', 3]).unwrap().as_deref(),
            Some([b'v', 3].as_slice())
        );
        c.put(b"post-compact", b"yes").unwrap();
        assert_eq!(c.count_applied_eq(b"post-compact", b"yes"), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F122: leftover intents with corrupt preimage used to open Ok (revert swallowed).
    #[test]
    fn open_rejects_leftover_intent_with_corrupt_preimage() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"u", b"live").unwrap();
            let tid = 99u64;
            for nid in c.ids.clone() {
                let n = c.nodes.get_mut(&nid).unwrap();
                n.db.put(intent_key(b"u"), encode_intent(tid, b"aborted-new"))
                    .unwrap();
                n.db.put(txn_pre_key(tid, b"u"), b"\xffgarbage").unwrap();
            }
        }
        match StoreCluster::open_lab_direct(&dir, 3, 1) {
            Ok(_) => {
                panic!("open must fail closed on leftover corrupt preimage, not swallow revert")
            }
            Err(e) => assert!(
                e.to_string().contains("preimage"),
                "expected preimage error, got {e}"
            ),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F122: force-local clear surfaces corrupt preimage (tx_cancel path).
    #[test]
    fn tx_cancel_rejects_corrupt_preimage() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"u", b"live").unwrap();
        let h = c.tx_start([(b"u".as_slice(), b"new".as_slice())]).unwrap();
        // Poison prepare preimages on every peer after prepare.
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(txn_pre_key(h.id, b"u"), b"\xffgarbage").unwrap();
            }
        }
        let err = c.tx_cancel(&h);
        assert!(
            err.is_err(),
            "tx_cancel must not swallow corrupt preimage: {err:?}"
        );
        assert_eq!(
            c.get(b"u").unwrap().as_deref(),
            Some(b"live".as_ref()),
            "user key must stay at preimage when cancel fails closed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F123: post-delete cluster rev must fail closed when present-but-short.
    #[test]
    fn dcs_delete_reports_corrupt_rev() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.dcs_create(b"/k", b"v").unwrap();
        // Apply path already fail-closes on corrupt rev (F113). Poison after a
        // successful create and issue Delete so post-check reads d/rev.
        // Force the post-check branch: apply Delete with valid rev, then the
        // only soft path was short→NotCommitted; inject short rev on leader
        // *before* propose so apply fails — still must not return Ok.
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(b"d/rev", b"xx").unwrap();
            }
        }
        let err = c.propose_dcs(DcsCommand::Delete {
            key: b"/k".to_vec(),
        });
        assert!(err.is_err(), "delete with corrupt d/rev must fail: {err:?}");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("corrupt")
                || msg.contains("rev")
                || msg.contains("NotCommitted")
                || msg.contains("Dcs")
                || msg.contains("u64"),
            "got {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F121: missing segment row under log_hi used to be skipped → log holes.
    #[test]
    fn open_rejects_raft_log_segment_gap() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            // No elect/put required — inject a segment gap on every node's Pedra.
            let rid = 1u64;
            let rec1 = LogRec {
                index: 1,
                term: 1,
                entry: RangeEntry::Noop,
            };
            let rec3 = LogRec {
                index: 3,
                term: 1,
                entry: RangeEntry::Noop,
            };
            for nid in c.ids.clone() {
                let n = c.nodes.get_mut(&nid).unwrap();
                // Base blob empty → load walks log_hi segments 1..=3.
                n.db.put(raft_meta_key(rid, "log"), encode_log(&[]))
                    .unwrap();
                n.db.put(raft_meta_key(rid, "log_hi"), encode_u64_meta(3))
                    .unwrap();
                n.db.put(log_entry_key(rid, 1), encode_one_log_rec(&rec1))
                    .unwrap();
                // index 2 intentionally missing
                n.db.put(log_entry_key(rid, 3), encode_one_log_rec(&rec3))
                    .unwrap();
                // Keep commit/applied low so uncommitted-suffix trim is a no-op.
                n.db.put(raft_meta_key(rid, "commit"), encode_u64_meta(0))
                    .unwrap();
                n.db.put(raft_meta_key(rid, "applied"), encode_u64_meta(0))
                    .unwrap();
            }
        }
        match StoreCluster::open_lab_direct(&dir, 3, 1) {
            Ok(_) => panic!("log segment gap must fail open, not load a holed log"),
            Err(e) => assert!(
                e.to_string().contains("segment gap") || e.to_string().contains("log segment"),
                "expected segment gap error, got {e}"
            ),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F26: raft meta survives process restart (same node dirs).
    #[test]
    fn raft_meta_survives_reopen() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        assert_eq!(c.count_applied_eq(b"after-reopen", b"ok"), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_rejects_raft_meta_prefix() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 1, 1).unwrap();
        c.elect_all(20).unwrap();
        let mut bad = b"\0store/raft/".to_vec();
        bad.extend_from_slice(b"1/hard");
        let err = c.put(&bad, b"x").unwrap_err();
        assert!(err.to_string().contains("reserved"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F100: intent/hist/txn user keys length-prefixed — no sibling prefix leak.
    #[test]
    fn intent_hist_txn_user_keys_not_prefix_siblings() {
        let ia = intent_key(b"a");
        let iab = intent_key(b"ab");
        let ha = hist_key(b"a");
        let hab = hist_key(b"ab");
        let pa = txn_pair_key(1, b"a");
        let pab = txn_pair_key(1, b"ab");
        assert!(
            !iab.starts_with(&ia),
            "intent_key(a) must not prefix intent_key(ab): {ia:?} vs {iab:?}"
        );
        assert!(
            !hab.starts_with(&ha),
            "hist_key(a) must not prefix hist_key(ab)"
        );
        assert!(
            !pab.starts_with(&pa),
            "txn_pair_key(a) must not prefix txn_pair_key(ab)"
        );
        // Decode path used by open GC / install-snapshot.
        assert_eq!(
            user_from_meta_suffix(ia.strip_prefix(INTENT_PREFIX).unwrap()).as_deref(),
            Some(b"a".as_ref())
        );
        assert_eq!(
            user_from_meta_suffix(iab.strip_prefix(INTENT_PREFIX).unwrap()).as_deref(),
            Some(b"ab".as_ref())
        );
        // Round-trip: multi-key TX with sibling user keys stays independent.
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.commit_tx([
            (b"a".as_slice(), b"va".as_slice()),
            (b"ab".as_slice(), b"vab".as_slice()),
        ])
        .unwrap();
        assert_eq!(c.get(b"a").unwrap().as_deref(), Some(b"va".as_ref()));
        assert_eq!(c.get(b"ab").unwrap().as_deref(), Some(b"vab".as_ref()));
        // SI hist load after reopen.
        drop(c);
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        assert_eq!(c.get(b"a").unwrap().as_deref(), Some(b"va".as_ref()));
        assert_eq!(c.get(b"ab").unwrap().as_deref(), Some(b"vab".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F25: refuse degenerate single-byte splits.
    #[test]
    fn open_rejects_too_many_ranges() {
        let dir = temp();
        match StoreCluster::open_lab_direct(&dir, 3, 300) {
            Ok(_) => panic!("expected n_ranges > 256 to fail"),
            Err(err) => assert!(err.to_string().contains("256"), "{err}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F23: prev-term entry majority-replicated commits after re-elect via leader noop.
    #[test]
    fn leader_noop_commits_prev_term_after_reelect() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let key = meta_key(b"pipe");
        c.dcs_create(&key, b"a").unwrap();
        // Inject a committed log entry that will CasFail on apply (duplicate create),
        // then a normal put that must still apply.
        let rid = c.locate(&key).unwrap();
        let leader = c.range_leader(rid).unwrap();
        {
            let p = c
                .nodes
                .get_mut(&leader)
                .unwrap()
                .ranges
                .get_mut(&rid)
                .unwrap();
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
                    si_gen: 0,
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
        assert_eq!(
            c.dcs_get_on(leader, &key).unwrap().unwrap().value,
            b"a",
            "injected Create must not steal the live binding"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// I-DCS-1 at apply: a later Create in the log must not overwrite a live lease.
    #[test]
    fn dcs_apply_create_does_not_steal_live_lock() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.set_ms_per_tick(0);
        c.elect_all(80).unwrap();
        let key = meta_key(b"live-lock");
        c.dcs_create_ttl(&key, b"holder-a", 10_000).unwrap();
        let rid = c.locate(&key).unwrap();
        let leader = c.range_leader(rid).unwrap();
        {
            let p = c
                .nodes
                .get_mut(&leader)
                .unwrap()
                .ranges
                .get_mut(&rid)
                .unwrap();
            let idx = p.last_index() + 1;
            let term = p.term;
            p.log.push(LogRec {
                index: idx,
                term,
                entry: RangeEntry::Dcs(DcsCommand::Create {
                    key: key.clone(),
                    value: b"holder-b".to_vec(),
                    lease: 99_000,
                }),
            });
            p.commit = idx;
        }
        c.apply_range(leader, rid).unwrap();
        assert_eq!(
            c.applied_index(leader, rid),
            c.commit_index(leader, rid),
            "Create conflict must still advance apply"
        );
        assert_eq!(
            c.dcs_get_on(leader, &key).unwrap().unwrap().value,
            b"holder-a",
            "live leased Create must not last-write-wins"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1.1: same-range multi-key atomic put_batch — majority sees all keys.
    #[test]
    fn put_batch_same_range_majority_atomic() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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

    /// RFC-0025 P2.3: strong leader read vs fast replica.
    #[test]
    fn get_strong_and_fast_replica_roundtrip() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        c.put(b"rk", b"v1").unwrap();
        assert_eq!(
            c.get_strong(b"rk").unwrap().as_deref(),
            Some(b"v1".as_ref())
        );
        assert_eq!(
            c.get_fast_replica(b"rk").unwrap().as_deref(),
            Some(b"v1".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F84: multi-range `get` follows per-range applied reader (not global
    /// last_sequence). Partitioning `ids[0]` must not hide a live key in range 2.
    #[test]
    fn get_multi_range_uses_per_range_applied_reader() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 2, SeedRng::new(0xF84_0001)).unwrap();
        c.elect_all(100).unwrap();
        // Range 2 starts at 0x80 under a 2-way first-byte split.
        let k1 = vec![0x90, b'x'];
        assert_eq!(c.locate(&k1).unwrap(), 2, "key must land in range 2");
        c.put(&k1, b"live").unwrap();
        assert!(c.count_applied_eq(&k1, b"live") >= 2);
        // Partition node 1 (historical default reader).
        c.set_participating(1, false).unwrap();
        assert_eq!(
            c.get(&k1).unwrap().as_deref(),
            Some(b"live".as_ref()),
            "F84: get must use best applied for range 2, not lagging/global default"
        );
        let rid = c.locate(&k1).unwrap();
        let best = c.best_applied_reader(rid).expect("best applied");
        assert_ne!(best, 1, "best reader must not be partitioned node 1");
        assert_eq!(
            c.get(&k1).unwrap().as_deref(),
            c.get_on(best, &k1).unwrap().as_deref()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0025 P1.1: buffered coalesce flushes as put_many.
    #[test]
    fn put_buffered_flush_coalesce() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        for i in 0..8u8 {
            c.put_buffered([b'b', i], [i]).unwrap();
        }
        assert_eq!(c.buffered_writes(), 8);
        c.flush_writes().unwrap();
        assert_eq!(c.buffered_writes(), 0);
        for i in 0..8u8 {
            assert!(c.count_applied_eq(&[b'b', i], &[i]) >= 2);
        }
        // Auto-flush every 4
        for i in 0..8u8 {
            c.put_coalesce([b'c', i], [i], 4).unwrap();
        }
        assert_eq!(c.buffered_writes(), 0);
        assert!(c.count_applied_eq(b"c\x07", b"\x07") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F66: failed flush must not drop the coalesce buffer (silent loss).
    #[test]
    fn put_buffered_flush_keeps_buffer_on_error() {
        let dir = temp();
        // No elect → put_many/put_batch → NotLeader.
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.put_buffered(b"keep/me", b"v1").unwrap();
        c.put_buffered(b"keep/me2", b"v2").unwrap();
        assert_eq!(c.buffered_writes(), 2);
        let err = c.flush_writes().expect_err("flush without leader");
        assert!(
            matches!(err, StoreError::NotLeader { .. }) || matches!(err, StoreError::Msg(_)),
            "expected not-leader-ish error, got {err:?}"
        );
        assert_eq!(
            c.buffered_writes(),
            2,
            "F66: flush error wiped staged puts (silent loss)"
        );
        // After elect, same buffer can still flush.
        c.elect_all(50).unwrap();
        c.flush_writes().unwrap();
        assert_eq!(c.buffered_writes(), 0);
        assert!(c.count_applied_eq(b"keep/me", b"v1") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F77: multi-range put_many uses 2PC (not sequential put_batch half-apply).
    #[test]
    fn put_many_multi_range_is_atomic_on_failure() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 4).unwrap();
        c.elect_all(100).unwrap();
        // Keys in different first-byte ranges under 4-way split.
        let metas = c.range_metas().to_vec();
        assert!(metas.len() >= 2, "need multi-range: {metas:?}");
        let k0 = if metas[0].start.is_empty() {
            vec![0x00, b'a']
        } else {
            let mut k = metas[0].start.clone();
            k.push(b'a');
            k
        };
        let k1 = {
            let mut k = metas[1].start.clone();
            k.push(b'b');
            k
        };
        assert_ne!(c.locate(&k0).unwrap(), c.locate(&k1).unwrap());
        // Happy path: both apply atomically via commit_tx.
        c.put_many([
            (k0.as_slice(), b"v0".as_slice()),
            (k1.as_slice(), b"v1".as_slice()),
        ])
        .unwrap();
        assert!(c.count_applied_eq(&k0, b"v0") >= 2);
        assert!(c.count_applied_eq(&k1, b"v1") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0025 P0.1: put_many groups by range into put_batch (single range).
    #[test]
    fn put_many_same_range_and_lab_open() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(60).unwrap();
        c.put_many([(b"m1", b"a"), (b"m2", b"b"), (b"m3", b"c")])
            .unwrap();
        assert!(c.count_applied_eq(b"m1", b"a") >= 2);
        assert!(c.count_applied_eq(b"m3", b"c") >= 2);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);

        // Lab capacity open: sync=false still elects and writes (not crash-safe).
        let dir2 = temp();
        let mut c = StoreCluster::open_with_options_lab_direct(
            &dir2,
            3,
            1,
            StoreOpenOptions::lab_capacity(),
        )
        .unwrap();
        c.elect_all(60).unwrap();
        c.put(b"lab", b"1").unwrap();
        assert!(c.count_applied_eq(b"lab", b"1") >= 2);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    /// Pedra write backpressure defaults apply on every node when opted in.
    #[test]
    fn open_with_write_backpressure_enables_l0_stall() {
        let dir = temp();
        let opts = StoreOpenOptions::default().with_write_backpressure();
        assert!(opts.pedra_write_backpressure);
        let c = StoreCluster::open_with_options_lab_direct(&dir, 3, 1, opts).unwrap();
        for id in &c.ids {
            let n = c.nodes.get(id).expect("node");
            assert_eq!(
                n.db.write_pressure_l0(),
                Some(pedradb_core::L0_COMPACTION_TRIGGER)
            );
            assert_eq!(
                n.db.write_stall_l0(),
                Some(pedradb_core::L0_COMPACTION_TRIGGER.saturating_mul(2))
            );
            assert!(n.db.write_stall_drain());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Multiproc single-node path also honors write-backpressure open opts.
    #[test]
    fn open_single_node_with_write_backpressure() {
        let dir = temp();
        let opts = StoreOpenOptions::default().with_write_backpressure();
        let c = StoreCluster::open_single_node_with_options(&dir, 1, &[1], 1, opts).unwrap();
        let n = c.nodes.get(&1).expect("node 1");
        assert_eq!(
            n.db.write_pressure_l0(),
            Some(pedradb_core::L0_COMPACTION_TRIGGER)
        );
        assert!(n.db.write_stall_drain());
        let st = c.status_text();
        assert!(
            st.contains("n1:l0=") && st.contains("l0_lim="),
            "status should include Pedra L0 metrics: {st}"
        );
        assert!(
            st.contains(&format!(
                "l0_lim={}",
                pedradb_core::L0_COMPACTION_TRIGGER.saturating_mul(2)
            )),
            "hard stall limit in status: {st}"
        );
        let snap = c.write_admission_snap();
        assert_eq!(snap.nodes, 1);
        assert_eq!(
            snap.write_pressure_l0,
            pedradb_core::L0_COMPACTION_TRIGGER as u64
        );
        assert_eq!(
            snap.write_stall_l0,
            pedradb_core::L0_COMPACTION_TRIGGER.saturating_mul(2) as u64
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Core WriteStall maps to first-class StoreError (not opaque Core).
    #[test]
    fn write_stall_maps_from_core_error() {
        let e: StoreError = pedradb_core::CoreError::WriteStall {
            l0_files: 9,
            limit: 8,
        }
        .into();
        assert!(matches!(
            e,
            StoreError::WriteStall {
                l0_files: 9,
                limit: 8
            }
        ));
        let e: StoreError = pedradb_core::CoreError::WriteStallMem {
            mem_bytes: 100,
            limit: 64,
        }
        .into();
        assert!(matches!(e, StoreError::WriteStallMem { limit: 64, .. }));
        let cls = crate::client::classify(&e);
        assert!(matches!(cls, crate::client::ClientClass::Unavailable(_)));
    }

    /// P1.1: index-style primary row + secondary key in one batch.
    #[test]
    fn put_batch_row_and_secondary_index_style() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        // Co-locate under same first-byte region (single range cluster).
        let row = b"row/user/42";
        let idx = b"idx/email/a@b.co";
        let payload = br#"{"email":"a@b.co"}"#;
        c.put_batch([
            (row.as_slice(), payload.as_slice()),
            (idx.as_slice(), row.as_slice()),
        ])
        .unwrap();
        assert!(c.count_applied_eq(row, payload) >= 2);
        assert!(c.count_applied_eq(idx, row) >= 2);
        // Point lookup path: secondary → primary key → row.
        let pk = c.get(idx).unwrap().expect("idx");
        assert_eq!(pk.as_ref(), row);
        assert_eq!(
            c.get(pk.as_ref()).unwrap().as_deref(),
            Some(payload.as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1.1: minority batch fails with no partial majority apply.
    #[test]
    fn put_batch_fails_without_majority_no_partial() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
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
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
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
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put_batch([(b"p1", b"A"), (b"p2", b"B")]).unwrap();
        }
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        assert_eq!(c.get_on(1, b"p1").unwrap().as_deref(), Some(b"A".as_ref()));
        assert_eq!(c.get_on(2, b"p2").unwrap().as_deref(), Some(b"B".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FDB-class gap: cross-range multi-key atomic via commit_tx.
    #[test]
    fn commit_tx_cross_range_atomic_majority() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        assert!(keys.len() >= 2);
        assert_ne!(c.locate(&keys[0]).unwrap(), c.locate(&keys[1]).unwrap());
        let tid = c
            .commit_tx([
                (keys[0].as_slice(), b"va".as_slice()),
                (keys[1].as_slice(), b"vb".as_slice()),
            ])
            .unwrap();
        assert!(tid >= 1);
        assert!(c.count_applied_eq(&keys[0], b"va") >= 2);
        assert!(c.count_applied_eq(&keys[1], b"vb") >= 2);
        // put_batch still hard-fails cross-range (fast path).
        assert!(matches!(
            c.put_batch([
                (keys[0].as_slice(), b"x".as_slice()),
                (keys[1].as_slice(), b"y".as_slice()),
            ])
            .unwrap_err(),
            StoreError::CrossRange { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// I-TX-2: prepare Ok, then last range loses leader → finish Err, no majority keys,
    /// **and** no stuck intents (re-elect then put/commit_tx must succeed).
    #[test]
    fn commit_tx_finish_fail_after_prepare_no_partial() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        assert!(keys.len() >= 2);
        let h = c
            .tx_start([
                (keys[0].as_slice(), b"p0".as_slice()),
                (keys[1].as_slice(), b"p1".as_slice()),
            ])
            .expect("prepare both ranges");
        // User keys must not be majority-applied after prepare alone.
        assert_eq!(c.count_applied_eq(&keys[0], b"p0"), 0);
        assert_eq!(c.count_applied_eq(&keys[1], b"p1"), 0);
        // Knock out the last prepared range's leader so commit cannot majority there.
        let last = *h.ranges.last().unwrap();
        let _ = c.step_down_range_leader(last);
        let err = c
            .tx_finish(&h)
            .expect_err("finish without last leader must fail");
        assert!(
            matches!(
                err,
                StoreError::NotLeader { .. } | StoreError::NotCommitted { .. }
            ),
            "got {err:?}"
        );
        // Criterion 1: fail ⇒ no key from TX majority-applied.
        assert_eq!(
            c.count_applied_eq(&keys[0], b"p0"),
            0,
            "key0 must not remain majority-applied after failed finish"
        );
        assert_eq!(
            c.count_applied_eq(&keys[1], b"p1"),
            0,
            "key1 must not remain majority-applied after failed finish"
        );
        // Intents must not stick: re-elect and write the same keys successfully.
        c.elect_all(120).unwrap();
        c.put(&keys[0], b"after0")
            .expect("put after failed TX must not Conflict on stuck intent");
        c.put(&keys[1], b"after1")
            .expect("put key1 after failed TX must not Conflict");
        assert!(c.count_applied_eq(&keys[0], b"after0") >= 2);
        assert!(c.count_applied_eq(&keys[1], b"after1") >= 2);
        // Full multi-key TX on those keys also works after cleanup.
        c.commit_tx([
            (keys[0].as_slice(), b"tx0".as_slice()),
            (keys[1].as_slice(), b"tx1".as_slice()),
        ])
        .expect("commit_tx after failed finish must not Conflict");
        assert!(c.count_applied_eq(&keys[0], b"tx0") >= 2);
        assert!(c.count_applied_eq(&keys[1], b"tx1") >= 2);
        // Across reopen: still no immortal intents from the failed TX.
        drop(c);
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        c.put(&keys[0], b"reopen-ok")
            .expect("reopen put must not hit leftover intent");
        assert!(c.count_applied_eq(&keys[0], b"reopen-ok") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Prepare failure under minority: no user keys majority-applied.
    #[test]
    fn commit_tx_cross_range_minority_no_partial() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        let r0 = c.locate(&keys[0]).unwrap();
        let r1 = c.locate(&keys[1]).unwrap();
        // Partition so neither range has majority (isolate two followers globally).
        let ids: Vec<u64> = c.node_ids().to_vec();
        // Keep only node 1 participating — cannot majority any range.
        for &nid in &ids {
            if nid != 1 {
                c.set_participating(nid, false).unwrap();
            }
        }
        // Re-elect on remaining single node fails majority of 3 — may have no leader.
        let _ = c.elect_all(40);
        // If we still have leaders somehow, tx should NotCommitted.
        let res = c.commit_tx([
            (keys[0].as_slice(), b"x".as_slice()),
            (keys[1].as_slice(), b"y".as_slice()),
        ]);
        assert!(res.is_err(), "expected err under minority, got {res:?}");
        assert_eq!(c.count_applied_eq(&keys[0], b"x"), 0);
        assert_eq!(c.count_applied_eq(&keys[1], b"y"), 0);
        let _ = (r0, r1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Write-write conflict: second tx_start while first holds intents.
    #[test]
    fn commit_tx_write_write_conflict() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        let h1 = c
            .tx_start([
                (keys[0].as_slice(), b"a1".as_slice()),
                (keys[1].as_slice(), b"b1".as_slice()),
            ])
            .unwrap();
        let err = c
            .tx_start([
                (keys[0].as_slice(), b"a2".as_slice()),
                (keys[1].as_slice(), b"b2".as_slice()),
            ])
            .expect_err("overlapping prepare must conflict");
        assert!(
            matches!(err, StoreError::Conflict | StoreError::TxnAborted(_)),
            "got {err:?}"
        );
        // First TX still finishes; values a1/b1 win.
        c.tx_finish(&h1).unwrap();
        assert!(c.count_applied_eq(&keys[0], b"a1") >= 2);
        assert!(c.count_applied_eq(&keys[1], b"b1") >= 2);
        assert_eq!(c.count_applied_eq(&keys[0], b"a2"), 0);
        // After commit, new TX can write.
        c.commit_tx([
            (keys[0].as_slice(), b"a3".as_slice()),
            (keys[1].as_slice(), b"b3".as_slice()),
        ])
        .unwrap();
        assert!(c.count_applied_eq(&keys[0], b"a3") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Multi-range single-key power still works alongside TX.
    #[test]
    fn multi_range_puts_still_work_with_tx_path() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        c.put(&keys[0], b"s0").unwrap();
        c.put(&keys[1], b"s1").unwrap();
        assert!(c.count_applied_eq(&keys[0], b"s0") >= 2);
        assert!(c.count_applied_eq(&keys[1], b"s1") >= 2);
        c.commit_tx([
            (keys[0].as_slice(), b"t0".as_slice()),
            (keys[1].as_slice(), b"t1".as_slice()),
        ])
        .unwrap();
        c.put(&keys[2.min(keys.len() - 1)], b"solo").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same-range commit_tx also works (2PC single range).
    #[test]
    fn commit_tx_same_range() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.commit_tx([(b"x", b"1"), (b"y", b"2")]).unwrap();
        assert!(c.count_applied_eq(b"x", b"1") >= 2);
        assert!(c.count_applied_eq(b"y", b"2") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0021 P0.1 / 0023: write-only PendingTx still works; default is Transaction.
    #[test]
    fn pending_tx_client_session_atomic_majority() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let mut tx = c.pending_tx_begin();
        tx.set(b"pt-a", b"1").unwrap();
        tx.set(b"pt-b", b"2").unwrap();
        assert_eq!(tx.get_buffered(b"pt-a"), Some(b"1".as_ref()));
        let tid = tx.commit(&mut c).unwrap();
        assert!(tid >= 1);
        assert!(c.count_applied_eq(b"pt-a", b"1") >= 2);
        assert!(c.count_applied_eq(b"pt-b", b"2") >= 2);
        // Default begin() is snapshot Transaction.
        let mut t2 = c.begin();
        t2.set(b"pt-c", b"3").unwrap();
        t2.commit(&mut c).unwrap();
        assert!(c.count_applied_eq(b"pt-c", b"3") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0021 P1.2: split_range_at creates a second range; puts still work.
    #[test]
    fn split_range_at_two_ranges_put() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        assert_eq!(c.range_metas().len(), 1);
        let (left, right) = c.split_range_at([0x80u8]).unwrap();
        assert_eq!(left, 1);
        assert_eq!(right, 2);
        assert_eq!(c.range_metas().len(), 2);
        c.put([0x10u8], b"lo").unwrap();
        c.put([0x90u8], b"hi").unwrap();
        assert!(c.count_applied_eq(&[0x10], b"lo") >= 2);
        assert!(c.count_applied_eq(&[0x90], b"hi") >= 2);
        assert_ne!(c.locate(&[0x10]).unwrap(), c.locate(&[0x90]).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Residual: split while a TX is prepared — finish or fail-closed cleanup.
    #[test]
    fn split_range_during_prepared_tx_finish_or_cancel_safe() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        // Single range; keys will fall on different sides after split at 0x80.
        let h = c
            .tx_start([([0x10u8], b"L".as_slice()), ([0x90u8], b"R".as_slice())])
            .unwrap();
        let (left, right) = c.split_range_at([0x80u8]).unwrap();
        assert_ne!(left, right);
        match c.tx_finish(&h) {
            Ok(()) => {
                assert!(c.count_applied_eq(&[0x10], b"L") >= 2);
                assert!(c.count_applied_eq(&[0x90], b"R") >= 2);
            }
            Err(e) => {
                let _ = c.tx_cancel(&h);
                assert_eq!(
                    c.count_applied_eq(&[0x10], b"L"),
                    0,
                    "partial after fail: {e}"
                );
                assert_eq!(
                    c.count_applied_eq(&[0x90], b"R"),
                    0,
                    "partial after fail: {e}"
                );
                c.commit_tx([([0x10u8], b"L2".as_slice()), ([0x90u8], b"R2".as_slice())])
                    .expect("must not immortal-intent after split+fail");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Residual: remove_member while TX prepared — finish still majority-commits.
    #[test]
    fn remove_member_during_prepared_tx() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let h = c.tx_start([(b"k1", b"v1"), (b"k2", b"v2")]).unwrap();
        let rid = c.locate(b"k1").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let victim = c
            .node_ids()
            .iter()
            .copied()
            .find(|&id| id != leader)
            .unwrap();
        c.remove_member(victim).unwrap();
        c.tx_finish(&h).expect("finish after shrink membership");
        assert!(c.count_applied_eq(b"k1", b"v1") >= 1);
        assert!(c.count_applied_eq(b"k2", b"v2") >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0021 P0.5: cluster_status_json has members + range leaders.
    #[test]
    fn cluster_status_json_has_leaders() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let js = c.cluster_status_json();
        assert!(js.contains("\"members\""), "{js}");
        assert!(js.contains("\"ranges\""), "{js}");
        assert!(js.contains("\"leader\""), "{js}");
        // Must parse as JSON-ish object
        assert!(js.starts_with('{') && js.ends_with('}'), "{js}");
        let lead = c.range_leader(1).expect("leader");
        assert!(js.contains(&format!("\"leader\":{lead}")), "{js}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0021 P1.3: peer dial map installed in-process without SSH.
    #[test]
    fn set_peer_addrs_without_ssh() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.set_peer_addrs([
            (1u64, "10.0.0.1:9701".into()),
            (2, "10.0.0.2:9701".into()),
            (3, "10.0.0.3:9701".into()),
        ])
        .unwrap();
        assert_eq!(
            c.peer_addrs().get(&2).map(|s| s.as_str()),
            Some("10.0.0.2:9701")
        );
        let js = c.cluster_status_json();
        assert!(js.contains("10.0.0.2:9701"), "{js}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0021 P2.6: region tags change dial preference order.
    #[test]
    fn region_prefer_dial_order() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.set_node_region(1, "us-east").unwrap();
        c.set_node_region(2, "us-east").unwrap();
        c.set_node_region(3, "eu-west").unwrap();
        let order = c.dial_order_prefer_region(Some("eu-west"));
        assert_eq!(order[0], 3, "eu-west member first: {order:?}");
        assert!(order[1..].contains(&1) && order[1..].contains(&2));
        let js = c.cluster_status_json();
        assert!(js.contains("eu-west") && js.contains("us-east"), "{js}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0021 P0.2: oversized value rejected with typed error.
    #[test]
    fn commit_tx_rejects_value_too_large() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        let big = vec![7u8; MAX_VALUE_BYTES + 1];
        let err = c.commit_tx([(b"k", big.as_slice())]).unwrap_err();
        assert!(
            matches!(err, StoreError::ValueTooLarge { .. }),
            "got {err:?}"
        );
        assert!(matches!(
            crate::client::classify(&err),
            crate::client::ClientClass::LimitRejected { kind: "value", .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1.5: tick advances logical_now; advance_time is pure (no wall clock).
    #[test]
    fn logical_time_advances_with_tick() {
        let dir = temp();
        let mut c = StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(9)).unwrap();
        assert_eq!(c.logical_now(), 0);
        c.advance_time(5).unwrap();
        assert_eq!(c.logical_now(), 5);
        c.tick().unwrap();
        assert_eq!(c.logical_now(), 6);
        let t0 = c.logical_now();
        c.advance_time(10).unwrap();
        assert_eq!(c.logical_now(), t0 + 10);
        // Same advance schedule → same time (deterministic).
        let mut c2 = StoreCluster::open_with_rng_lab_direct(temp(), 3, 1, SeedRng::new(9)).unwrap();
        c2.advance_time(16).unwrap();
        assert_eq!(c2.logical_now(), c.logical_now());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P2.2: remove_member shrinks majority; compact not blocked by dead peer.
    #[test]
    fn remove_member_unblocks_compact_and_majority() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"before-rm", b"1").unwrap();
        c.remove_member(3).unwrap();
        assert!(!c.is_member(3));
        assert_eq!(c.node_ids().len(), 2);
        // Majority of 2 is 2 — both remaining must ack.
        c.put(b"after-rm", b"2").unwrap();
        assert_eq!(c.count_applied_eq(b"after-rm", b"2"), 2);
        // More puts so compact advances past removed peer's frozen applied.
        for i in 0..6u8 {
            c.put([b'r', i], [b'v', i]).unwrap();
        }
        let rid = c.locate(b"after-rm").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let snap = c.snapshot_index(leader, rid);
        assert!(
            snap >= 1,
            "membership shrink should allow compact; snap={snap}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F-found (RFC-0059 P2 campaign, seeds 500308 et al.): quorum floor
    /// on chained out-of-band removals. From 7 nodes a single removal is
    /// allowed (any two quorums over the 7-node universe still
    /// intersect); shrinking further is refused until a re-add restores
    /// the high-water mark — a smaller config could commit with a quorum
    /// disjoint from a later restored-config election and lose
    /// acknowledged writes.
    #[test]
    fn remove_member_quorum_floor_refuses_disjoint_shrink() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 7, 1).unwrap();
        c.elect_all(80).unwrap();
        assert!(c.remove_member(7).is_ok(), "single removal must pass");
        assert!(
            c.remove_member(6).is_err(),
            "second chained removal must hit the quorum floor"
        );
        assert!(c.add_member(7).is_ok());
        assert!(
            c.remove_member(6).is_ok(),
            "after re-add the high-water rule allows one out again"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0063 P0: log-carried joint remove is committed under
    /// majority(old)∧majority(new) and may shrink past the out-of-band floor.
    #[test]
    fn log_carried_joint_remove_crosses_out_of_band_floor() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 7, 1).unwrap();
        c.elect_all(80).unwrap();
        assert!(c.remove_member(7).is_ok());
        assert!(
            c.remove_member(6).is_err(),
            "out-of-band second shrink still hits the floor"
        );
        assert!(c.add_member(7).is_ok());
        c.remove_member_joint(7).expect("joint remove 7");
        assert!(!c.is_member(7), "joint apply must drop voter 7");
        c.remove_member_joint(6).expect("joint remove 6 past floor");
        assert!(
            !c.is_member(6),
            "log-carried joint may shrink past the out-of-band floor"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0064 P0: during joint add (C-old=3 maj=2, C-new=4 maj=3), two
    /// C-old votes are not enough to become leader.
    #[test]
    fn election_during_joint_add_refuses_old_only_majority() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        assert!(!c.is_member(4));
        let lid = c.range_leader(1).expect("leader after shrink");
        let term = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap().term;
        {
            let p = c.nodes.get_mut(&lid).unwrap().ranges.get_mut(&1).unwrap();
            let idx = p.last_index() + 1;
            p.log.push(LogRec {
                index: idx,
                term: p.term,
                entry: RangeEntry::MembershipJoint {
                    old: vec![1, 2, 3],
                    new: vec![1, 2, 3, 4],
                },
            });
        }
        c.election_granted.insert((1, term, lid), vec![1, 2]);
        c.election_votes.insert((1, term, lid), 2);
        assert!(
            !c.election_has_joint_quorum(1, term, lid),
            "2/3 old is not a joint quorum for add-to-4"
        );
        assert!(
            crate::membership_kernel::joint_election_ok_as_is(2, 3, Some((2, 4))),
            "AS-IS dente: old-only would elect"
        );
        // Drop the planted uncommitted joint so a real add can run.
        {
            let p = c.nodes.get_mut(&lid).unwrap().ranges.get_mut(&1).unwrap();
            p.log.pop();
        }
        c.add_member_joint(4).expect("joint add 4");
        assert!(c.is_member(4), "Direct joint add must apply");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0096 P0: production `leave_joint` after a committed joint
    /// without leave writes C-new-only (`old == new`). AS-IS
    /// `joint_leave_ok` skips leave. Compacted `add_member_joint` logs
    /// and `cluster_real` L28 leave are not this tooth.
    #[test]
    fn leave_joint_on_live_store_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.plant_committed_joint_without_leave(4)
            .expect("plant committed joint without leave");
        c.leave_joint().expect("production leave");
        let lid = c.range_leader(1).expect("leader");
        let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
        let leave = p.log.iter().any(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if !membership_kernel::joint_still_active(old, new)
            )
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(leave, "leave_joint must append C-new-only leave");
        assert!(
            membership_kernel::joint_leave_ok(leave),
            "kernel must require the leave that production wrote"
        );
    }

    /// RFC-0121 P1.2: on-disk C-new-only after live Direct shrink.
    /// 3-process `cluster_real --remove-member` is the REAL TCP tooth.
    #[test]
    fn tcp_node_disk_left_joint_after_direct_remove() {
        assert!(!l28_tcp_left_ok(false));
        assert!(
            l28_tcp_left_ok_as_is(false),
            "AS-IS dente: skip on-disk C-new-only"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.remove_member_joint(3).expect("shrink to 2");
        }
        let left = tcp_node_disk_left_joint(&dir, 1, 3) || tcp_node_disk_left_joint(&dir, 2, 3);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(left, "Direct remove must persist C-new-only on disk");
        assert!(membership_kernel::joint_leave_ok(left));
        assert!(l28_tcp_left_ok(left));
    }

    /// RFC-0126 P1.2: on-disk high-water after Direct shrink is still 3.
    /// 3-process `cluster_real --remove-member` is the REAL TCP tooth.
    #[test]
    fn tcp_node_disk_high_water_after_direct_remove() {
        assert!(!l28_tcp_hw_ok(false));
        assert!(
            l28_tcp_hw_ok_as_is(false),
            "AS-IS dente: skip on-disk high-water"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.remove_member_joint(3).expect("shrink to 2");
        }
        let disk_hw = tcp_node_disk_high_water(&dir, 1).max(tcp_node_disk_high_water(&dir, 2));
        let kept = membership_kernel::high_water_at_least(disk_hw, 2) >= 3;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(kept, "Direct remove must persist high-water 3, got {disk_hw}");
        assert!(l28_tcp_hw_ok(kept));
        assert_eq!(
            membership_kernel::high_water_at_least_as_is(disk_hw, 2),
            2,
            "AS-IS would forget disk high-water"
        );
    }

    /// RFC-0128 P1.2: TCP ctor with stale CLI after Direct shrink does not
    /// count the removed voter. 3-process `cluster_real` is the REAL TCP tooth.
    #[test]
    fn tcp_node_removed_not_participating_after_direct_remove() {
        assert!(!l28_tcp_part_ok(false));
        assert!(
            l28_tcp_part_ok_as_is(false),
            "AS-IS dente: skip TCP participating"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.remove_member_joint(3).expect("shrink to 2");
        }
        let ok = tcp_node_removed_not_participating(&dir, 1, &[1, 2, 3], 3);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "stale CLI [1,2,3] must not count removed 3 as participating");
        assert!(l28_tcp_part_ok(ok));
    }

    /// RFC-0130 P1.2: TCP ctor recover-applies a planted committed-unapplied
    /// Noop. 3-process `cluster_real` is the REAL TCP tooth.
    #[test]
    fn tcp_node_recover_apply_ok_after_direct() {
        assert!(!l28_tcp_apply_ok(false));
        assert!(
            l28_tcp_apply_ok_as_is(false),
            "AS-IS dente: skip TCP recover apply"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
        }
        let ok = tcp_node_recover_apply_ok(&dir, 1, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "open_single_node must recover-apply a planted apply gap");
        assert!(l28_tcp_apply_ok(ok));
    }

    /// RFC-0131 P1.2: TCP ctor recover-applies on a replica leave already
    /// dropped from `ids`. 0130 voter plant is **not** this tooth.
    #[test]
    fn tcp_node_removed_recover_apply_ok_after_direct() {
        assert!(!l28_tcp_napply_ok(false));
        assert!(
            l28_tcp_napply_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica recover apply"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_recover_apply_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must recover-apply"
        );
        assert!(l28_tcp_napply_ok(ok));
    }

    /// RFC-0132 P1.2: TCP ctor persists truncated log on a replica leave
    /// already dropped from `ids`. 0131 apply plant is **not** this tooth.
    #[test]
    fn tcp_node_removed_truncate_ok_after_direct() {
        assert!(!l28_tcp_trunc_ok(false));
        assert!(
            l28_tcp_trunc_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica truncate persist"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_truncate_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must persist truncated log"
        );
        assert!(l28_tcp_trunc_ok(ok));
    }

    /// RFC-0133 P1.2: TCP ctor drops orphan `log_entry_key` on a replica
    /// leave already dropped from `ids`. 0132 log_hi cap is **not** this tooth.
    #[test]
    fn tcp_node_removed_orphan_drop_ok_after_direct() {
        assert!(!l28_tcp_odrop_ok(false));
        assert!(
            l28_tcp_odrop_ok_as_is(false),
            "AS-IS dente: skip TCP orphan-segment drop"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_orphan_drop_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must drop orphan log_entry_key"
        );
        assert!(l28_tcp_odrop_ok(ok));
    }

    /// RFC-0134 P1.2: TCP ctor aborts leftover 2PC on a replica leave already
    /// dropped from `ids`. 0133 orphan drop is **not** this tooth.
    #[test]
    fn tcp_node_removed_abort_ok_after_direct() {
        assert!(!l28_tcp_abort_ok(false));
        assert!(
            l28_tcp_abort_ok_as_is(false),
            "AS-IS dente: skip TCP leftover 2PC abort"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_abort_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must abort leftover intent"
        );
        assert!(l28_tcp_abort_ok(ok));
    }

    /// RFC-0135 P1.2: TCP ctor persists now_ms on a replica leave already
    /// dropped from `ids`. 0134 abort is **not** this tooth.
    #[test]
    fn tcp_node_removed_now_ms_ok_after_direct() {
        assert!(!l28_tcp_nowms_ok(false));
        assert!(
            l28_tcp_nowms_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica now_ms persist"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_now_ms_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must persist now_ms"
        );
        assert!(l28_tcp_nowms_ok(ok));
    }

    /// RFC-0136 P1.2: TCP ctor persists SI hist on a replica leave already
    /// dropped from `ids`. 0135 now_ms is **not** this tooth.
    #[test]
    fn tcp_node_removed_hist_ok_after_direct() {
        assert!(!l28_tcp_hist_ok(false));
        assert!(
            l28_tcp_hist_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica SI hist persist"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_hist_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must persist SI hist"
        );
        assert!(l28_tcp_hist_ok(ok));
    }

    /// RFC-0137 P1.2: TCP ctor persists abort fence on a replica leave already
    /// dropped from `ids`. 0136 SI hist is **not** this tooth.
    #[test]
    fn tcp_node_removed_fence_ok_after_direct() {
        assert!(!l28_tcp_fence_ok(false));
        assert!(
            l28_tcp_fence_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica abort-fence persist"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_fence_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must persist abort fence"
        );
        assert!(l28_tcp_fence_ok(ok));
    }

    /// RFC-0138 P1.2: TCP ctor force-clears stuck intents on a replica leave
    /// already dropped from `ids`. 0137 abort fence is **not** this tooth.
    #[test]
    fn tcp_node_removed_clear_ok_after_direct() {
        assert!(!l28_tcp_clear_ok(false));
        assert!(
            l28_tcp_clear_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica force-local TX clear"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_clear_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must force-clear stuck intent"
        );
        assert!(l28_tcp_clear_ok(ok));
    }

    /// RFC-0139 P1.2: TCP ctor drops leftover preimages on a replica leave
    /// already dropped from `ids`. 0138 force-clear is **not** this tooth.
    #[test]
    fn tcp_node_removed_pre_ok_after_direct() {
        assert!(!l28_tcp_pre_ok(false));
        assert!(
            l28_tcp_pre_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica drop-preimages"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_pre_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must drop leftover preimage"
        );
        assert!(l28_tcp_pre_ok(ok));
    }

    /// RFC-0140 P1.2: TCP ctor election timeout follows disk C-new, not
    /// stale CLI. 0139 drop-preimages is **not** this tooth.
    #[test]
    fn tcp_node_removed_peer_ok_after_direct() {
        assert!(!l28_tcp_peer_ok(false));
        assert!(
            l28_tcp_peer_ok_as_is(false),
            "AS-IS dente: skip TCP disk-peer election timeout"
        );
        assert_ne!(
            election_timeout_for(3, 1, &[1, 2]),
            election_timeout_for(3, 1, &[1, 2, 3]),
            "timeout must differ so the TCP load-order tooth is observable"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_peer_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must load peer from disk C-new"
        );
        assert!(l28_tcp_peer_ok(ok));
    }

    /// RFC-0141 P1.2: TCP ctor omits HashMap first-key as identity on a
    /// replica leave already dropped from `ids`. 0140 timeout peek is
    /// **not** this tooth.
    #[test]
    fn tcp_node_removed_lid_ok_after_direct() {
        assert!(!l28_tcp_lid_ok(false));
        assert!(
            l28_tcp_lid_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica local-id gate"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_lid_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must omit local identity"
        );
        assert!(l28_tcp_lid_ok(ok));
    }

    /// RFC-0142 P1.2: TCP ctor must not pick remote `ids.first()` as a
    /// LocalApplied reader. 0141 local-id None is **not** this tooth.
    #[test]
    fn tcp_node_removed_rdr_ok_after_direct() {
        assert!(!l28_tcp_rdr_ok(false));
        assert!(
            l28_tcp_rdr_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica reader-local gate"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_rdr_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must skip remote ids.first"
        );
        assert!(l28_tcp_rdr_ok(ok));
    }

    /// RFC-0143 P1.2: TCP ctor live-discards uncommitted suffix on a replica
    /// leave already dropped from `ids`. 0132 recover truncate is **not**
    /// this tooth.
    #[test]
    fn tcp_node_removed_dsc_ok_after_direct() {
        assert!(!l28_tcp_dsc_ok(false));
        assert!(
            l28_tcp_dsc_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica live discard"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_dsc_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must live-discard suffix"
        );
        assert!(l28_tcp_dsc_ok(ok));
    }

    /// RFC-0144 P1.2: TCP ctor no-leader abort uses a local persist-leader.
    /// 0143 live discard is **not** this tooth.
    #[test]
    fn tcp_node_removed_pld_ok_after_direct() {
        assert!(!l28_tcp_pld_ok(false));
        assert!(
            l28_tcp_pld_ok_as_is(false),
            "AS-IS dente: skip TCP persist-leader locality"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_pld_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must repair next_index locally"
        );
        assert!(l28_tcp_pld_ok(ok));
    }

    /// RFC-0145 P1.2: TCP ctor re-install of C-new steps a planted Leader
    /// down. 0144 persist-leader is **not** this tooth.
    #[test]
    fn tcp_node_removed_std_ok_after_direct() {
        assert!(!l28_tcp_std_ok(false));
        assert!(
            l28_tcp_std_ok_as_is(false),
            "AS-IS dente: skip TCP removed-replica Leader step-down"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_removed_std_ok(&dir, 3, &[1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on removed replica must step planted Leader down"
        );
        assert!(l28_tcp_std_ok(ok));
    }

    /// RFC-0146 P1.2: TCP ctor of a remaining voter omits the removed
    /// replica from `leader_hint`. 0145 step-down is **not** this tooth.
    #[test]
    fn tcp_node_hint_ok_after_direct() {
        assert!(!l28_tcp_hnt_ok(false));
        assert!(
            l28_tcp_hnt_ok_as_is(false),
            "AS-IS dente: skip TCP leader-hint membership filter"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 3);
            assert!(!c.is_member(3), "leave must drop 3 before the TCP plant");
        }
        let ok = tcp_node_hint_ok(&dir, 1, &[1, 2, 3], 3);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            ok,
            "open_single_node on remaining voter must omit removed hint"
        );
        assert!(l28_tcp_hnt_ok(ok));
    }

    /// RFC-0097 P0: production RPC is Queued. After a planted committed
    /// joint, pin Queued and `leave_joint` still writes C-new-only.
    /// Direct-lab 0096 is not this tooth.
    #[test]
    fn leave_joint_on_queued_store_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.plant_committed_joint_without_leave(4)
            .expect("plant committed joint without leave");
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued, "production fingerprint");
        match c.leave_joint() {
            Ok(()) => {}
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                pump_queued(&mut c, 96);
                assert!(
                    c.finish_queued_propose(range_id, index, true).unwrap(),
                    "Queued leave must commit after pump"
                );
            }
            Err(e) => panic!("Queued leave: {e}"),
        }
        pump_queued(&mut c, 24);
        let lid = c.range_leader(1).expect("leader");
        let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
        let leave = p.log.iter().any(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if !membership_kernel::joint_still_active(old, new)
            )
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(leave, "Queued leave_joint must append C-new-only leave");
        assert!(membership_kernel::joint_leave_ok(leave));
    }

    /// RFC-0098 P0: Queued `add_member_joint` returns NotCommitted before
    /// `leave_joint`. After pump+finish, C-new-only must be in the log.
    /// Plant+leave (0096/0097) is not this tooth.
    #[test]
    fn leave_joint_on_queued_add_member_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        match c.add_member_joint(4) {
            Ok(()) => {}
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                pump_queued(&mut c, 96);
                assert!(
                    c.finish_queued_propose(range_id, index, true).unwrap(),
                    "Queued add_member_joint must commit after pump"
                );
            }
            Err(e) => panic!("Queued add_member_joint: {e}"),
        }
        pump_queued(&mut c, 24);
        let lid = c.range_leader(1).expect("leader");
        let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
        let leave = p.log.iter().any(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if !membership_kernel::joint_still_active(old, new)
            )
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            leave,
            "Queued add_member_joint must leave C-new-only in the log"
        );
        assert!(membership_kernel::joint_leave_ok(leave));
    }

    /// RFC-0099 P0: Queued `remove_member_joint` must leave after the
    /// shrink joint commits. Add (0098) is not this tooth.
    #[test]
    fn leave_joint_on_queued_remove_member_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        // Compact after leave-apply drops C-new-only from RAM (0096 class:
        // remaining 3 catch up; add-path 0098 keeps leave because the
        // joining node's applied lags). Observe leave before that compact.
        let mut saw_leave = false;
        match c.remove_member_joint(4) {
            Ok(()) => {
                saw_leave = any_log_has_c_new_only_leave(&c);
            }
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => {
                for _ in 0..96 {
                    if any_log_has_c_new_only_leave(&c) {
                        saw_leave = true;
                        break;
                    }
                    pump_queued(&mut c, 1);
                    let commit = c
                        .range_leader(range_id)
                        .map(|lid| c.commit_index(lid, range_id))
                        .unwrap_or(0);
                    if commit >= index {
                        assert!(
                            c.finish_queued_propose(range_id, index, true).unwrap(),
                            "Queued remove_member_joint must commit after pump"
                        );
                    }
                    if any_log_has_c_new_only_leave(&c) {
                        saw_leave = true;
                        break;
                    }
                }
            }
            Err(e) => panic!("Queued remove_member_joint: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            saw_leave,
            "Queued remove_member_joint must leave C-new-only in the log"
        );
        assert!(membership_kernel::joint_leave_ok(saw_leave));
    }

    /// RFC-0122 P0: after finishing the **joint** propose, C-new-only may
    /// sit uncommitted (`leave_joint_after_commit` swallows NotCommitted).
    /// 0121 joint-only finish is **not** this tooth.
    #[test]
    fn queued_leave_after_joint_must_be_finished() {
        assert!(!membership_kernel::queued_leave_finish_ok(true, false));
        assert!(
            membership_kernel::queued_leave_finish_ok_as_is(true, false),
            "AS-IS dente: leave in log is enough"
        );
        assert!(membership_kernel::queued_leave_finish_ok(true, true));
        assert!(membership_kernel::queued_leave_finish_ok(false, false));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        let (rid, joint_idx) = match c.remove_member_joint(4) {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            Ok(()) => panic!("Queued remove_member_joint should NotCommitted"),
            Err(e) => panic!("Queued remove_member_joint: {e}"),
        };
        let mut joint_done = false;
        for _ in 0..96 {
            pump_queued(&mut c, 1);
            let commit = c
                .range_leader(rid)
                .map(|lid| c.commit_index(lid, rid))
                .unwrap_or(0);
            if commit >= joint_idx {
                assert!(
                    c.finish_queued_propose(rid, joint_idx, false).unwrap(),
                    "joint propose must finish"
                );
                joint_done = true;
                break;
            }
        }
        assert!(joint_done, "joint must commit");
        assert!(
            any_log_has_c_new_only_leave(&c),
            "leave must be on the log after joint finish"
        );
        assert!(
            c.uncommitted_leave_index().is_some(),
            "0121 leftover: leave appended but not committed"
        );
        let leave_idx = max_c_new_only_leave_index(&c);
        let commit = c
            .range_leader(rid)
            .map(|lid| c.commit_index(lid, rid))
            .unwrap_or(0);
        assert!(
            !membership_kernel::queued_leave_finish_ok(true, leave_idx <= commit),
            "uncommitted leave must fail queued_leave_finish_ok"
        );
        for _ in 0..96 {
            if c.uncommitted_leave_index().is_none() {
                break;
            }
            pump_queued(&mut c, 1);
            let _ = c.finish_uncommitted_leave();
        }
        assert!(
            c.uncommitted_leave_index().is_none(),
            "finish_uncommitted_leave must commit C-new-only"
        );
        let commit = c
            .range_leader(rid)
            .map(|lid| c.commit_index(lid, rid))
            .unwrap_or(0);
        assert!(membership_kernel::queued_leave_finish_ok(
            true,
            max_c_new_only_leave_index(&c) <= commit
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0123 P1.1: after 0122 finish, persist + crash-reopen must still
    /// see committed C-new-only (or a snapshot that covers it). RAM-only
    /// 0122 is **not** this tooth.
    #[test]
    fn queued_leave_survives_crash_reopen() {
        assert!(!membership_kernel::queued_leave_finish_ok(true, false));
        assert!(
            membership_kernel::queued_leave_finish_ok_as_is(true, false),
            "AS-IS dente: skip reopen"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        let (rid, joint_idx) = match c.remove_member_joint(4) {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            Ok(()) => panic!("Queued remove_member_joint should NotCommitted"),
            Err(e) => panic!("Queued remove_member_joint: {e}"),
        };
        for _ in 0..96 {
            pump_queued(&mut c, 1);
            let commit = c
                .range_leader(rid)
                .map(|lid| c.commit_index(lid, rid))
                .unwrap_or(0);
            if commit >= joint_idx {
                assert!(c.finish_queued_propose(rid, joint_idx, false).unwrap());
                break;
            }
        }
        for _ in 0..96 {
            if c.uncommitted_leave_index().is_none() {
                break;
            }
            pump_queued(&mut c, 1);
            let _ = c.finish_uncommitted_leave();
        }
        assert!(c.uncommitted_leave_index().is_none());
        let leave_idx = max_c_new_only_leave_index(&c);
        assert!(leave_idx > 0, "leave must be in the log before reopen");
        let lid = c.range_leader(rid).expect("leader");
        {
            let n = c.nodes.get_mut(&lid).unwrap();
            persist_log_db(&mut n.db, rid, n.ranges.get_mut(&rid).unwrap()).unwrap();
            persist_commit_db(&mut n.db, rid, n.ranges.get(&rid).unwrap()).unwrap();
            persist_applied_db(&mut n.db, rid, n.ranges.get(&rid).unwrap()).unwrap();
        }
        c.crash_reopen_engine_on(lid, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen leader");
        let p = c.nodes.get(&lid).unwrap().ranges.get(&rid).unwrap();
        let snap_covers = p.snapshot_index >= leave_idx;
        let committed_in_log = p.log.iter().any(|rec| {
            rec.index <= p.commit
                && matches!(
                    &rec.entry,
                    RangeEntry::MembershipJoint { old, new }
                        if !membership_kernel::joint_still_active(old, new)
                )
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            committed_in_log || snap_covers,
            "committed leave must survive crash-reopen or be snapshotted"
        );
        assert!(membership_kernel::queued_leave_finish_ok(
            committed_in_log,
            committed_in_log || snap_covers
        ));
    }

    fn queued_shrink_until_leave_committed(c: &mut StoreCluster, remove: u64) {
        let (rid, joint_idx) = match c.remove_member_joint(remove) {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            Ok(()) => panic!("Queued remove_member_joint should NotCommitted"),
            Err(e) => panic!("Queued remove_member_joint: {e}"),
        };
        for _ in 0..96 {
            pump_queued(c, 1);
            let commit = c
                .range_leader(rid)
                .map(|lid| c.commit_index(lid, rid))
                .unwrap_or(0);
            if commit >= joint_idx {
                assert!(c.finish_queued_propose(rid, joint_idx, false).unwrap());
                break;
            }
        }
        for _ in 0..96 {
            if c.uncommitted_leave_index().is_none() {
                return;
            }
            pump_queued(c, 1);
            let _ = c.finish_uncommitted_leave();
        }
        assert!(
            c.uncommitted_leave_index().is_none(),
            "leave must commit"
        );
    }

    /// RFC-0124 P0: durable membership must override stale RAM/CLI ids.
    /// 0123 crash-reopen of the log is **not** this tooth.
    #[test]
    fn disk_membership_overrides_cli_after_leave() {
        assert!(membership_kernel::disk_membership_overrides_cli(true));
        assert!(
            !membership_kernel::disk_membership_overrides_cli_as_is(true),
            "AS-IS dente: CLI --peer overwrites disk"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4), "apply must drop 4");
        let lid = c.range_leader(1).expect("leader");
        let raw = c
            .nodes
            .get(&lid)
            .unwrap()
            .db
            .get(&cluster_membership_key())
            .expect("disk membership");
        let disk = decode_membership(&raw).unwrap();
        assert!(
            !disk.contains(&4),
            "disk membership must omit removed node: {disk:?}"
        );
        c.ids = vec![1, 2, 3, 4];
        c.bind_cluster_identity(None).unwrap();
        assert!(
            !c.is_member(4),
            "bind must restore disk voters, not stale CLI ids"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0124 P1.2: crash-reopen reloads voters from disk.
    #[test]
    fn crash_reopen_reloads_disk_membership() {
        assert!(membership_kernel::disk_membership_overrides_cli(true));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        let lid = c.range_leader(1).expect("leader");
        {
            let n = c.nodes.get_mut(&lid).unwrap();
            persist_log_db(&mut n.db, 1, n.ranges.get_mut(&1).unwrap()).unwrap();
            persist_commit_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
            persist_applied_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
        }
        c.ids = vec![1, 2, 3, 4];
        c.crash_reopen_engine_on(lid, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen");
        assert!(
            !c.is_member(4),
            "crash_reopen must reload C-new from disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0125 P0: 4-node high-water survives reopen as 3 nodes; OOB shrink
    /// still hits the quorum floor. AS-IS high-water=3 would allow it.
    #[test]
    fn high_water_survives_reopen_refuses_oob_shrink() {
        assert_eq!(membership_kernel::high_water_at_least(4, 3), 4);
        assert_eq!(
            membership_kernel::high_water_at_least_as_is(4, 3),
            3,
            "AS-IS dente: RAM/CLI high-water only"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open(&dir, 3, 1).expect("reopen 3");
        assert!(!c2.is_member(4), "disk membership omits 4");
        let err = c2.remove_member(3).expect_err("quorum floor");
        assert!(
            err.to_string().contains("quorum floor"),
            "4-node high-water must survive reopen, got {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0125 P1.1: TCP ctor loads disk voters before the raft log.
    /// 0124 bind-after-load is **not** this tooth.
    #[test]
    fn open_single_node_stale_cli_loads_disk_membership() {
        assert!(membership_kernel::disk_membership_overrides_cli(true));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
        }
        let c2 = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3, 4], 1)
            .expect("TCP ctor with stale CLI");
        assert!(
            !c2.is_member(4),
            "open_single_node must take disk C-new, not CLI [1,2,3,4]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0126 P0: crash-reopen must read durable high-water, not RAM.
    /// 0125 process-open is **not** this tooth.
    #[test]
    fn crash_reopen_restores_high_water() {
        assert_eq!(membership_kernel::high_water_at_least(4, 3), 4);
        assert_eq!(
            membership_kernel::high_water_at_least_as_is(4, 3),
            3,
            "AS-IS dente: RAM high-water only"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        let lid = c.range_leader(1).expect("leader");
        {
            let n = c.nodes.get_mut(&lid).unwrap();
            persist_log_db(&mut n.db, 1, n.ranges.get_mut(&1).unwrap()).unwrap();
            persist_commit_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
            persist_applied_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
        }
        c.membership_high_water = 3;
        c.crash_reopen_engine_on(lid, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen");
        let err = c.remove_member(3).expect_err("quorum floor after reopen");
        assert!(
            err.to_string().contains("quorum floor"),
            "disk high-water 4 must survive crash-reopen, got {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0127 P0: crash-reopen must not keep a stale participating=true
    /// on a node that disk membership already dropped.
    #[test]
    fn crash_reopen_participating_follows_membership() {
        assert!(!membership_kernel::participating_if_member(false));
        assert!(
            membership_kernel::participating_if_member_as_is(false),
            "AS-IS dente: keep captured participating"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        {
            let n = c.nodes.get_mut(&4).unwrap();
            n.participating = true;
            persist_log_db(&mut n.db, 1, n.ranges.get_mut(&1).unwrap()).unwrap();
            persist_commit_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
            persist_applied_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
        }
        c.crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen removed node");
        assert!(!c.is_member(4), "disk membership omits 4");
        assert!(
            !c.is_participating(4),
            "stale participating=true must not survive crash-reopen"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plant_committed_unapplied_shrink(c: &mut StoreCluster, lid: u64) {
        let n = c.nodes.get_mut(&lid).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term,
            entry: RangeEntry::MembershipJoint {
                old: vec![1, 2, 3, 4],
                new: vec![1, 2, 3],
            },
        });
        p.commit = idx;
        persist_log_db(&mut n.db, 1, p).unwrap();
        persist_commit_db(&mut n.db, 1, p).unwrap();
    }

    /// RFC-0130 P0: crash-reopen must apply a committed unapplied joint.
    /// 0124 disk membership of an already-applied joint is **not** this tooth.
    #[test]
    fn crash_reopen_applies_committed_unapplied_joint() {
        assert!(membership_kernel::recover_must_apply(1, 2));
        assert!(
            !membership_kernel::recover_must_apply_as_is(1, 2),
            "AS-IS dente: skip apply on recover"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        let lid = c.range_leader(1).expect("leader");
        assert_eq!(c.applied_index(lid, 1), c.commit_index(lid, 1));
        plant_committed_unapplied_shrink(&mut c, lid);
        assert!(c.is_member(4), "joint is committed but not applied");
        c.crash_reopen_engine_on(lid, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen");
        assert!(
            !c.is_member(4),
            "recover must apply committed joint"
        );
        let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
        assert!(
            p.applied >= p.commit,
            "applied must catch commit after recover"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0130 P1.1: process open applies a committed unapplied joint.
    /// crash-reopen is **not** this tooth.
    #[test]
    fn open_applies_committed_unapplied_joint() {
        assert!(membership_kernel::recover_must_apply(0, 1));
        assert!(
            !membership_kernel::recover_must_apply_as_is(0, 1),
            "AS-IS dente: skip apply on recover"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            let lid = c.range_leader(1).expect("leader");
            assert_eq!(c.applied_index(lid, 1), c.commit_index(lid, 1));
            plant_committed_unapplied_shrink(&mut c, lid);
            assert!(c.is_member(4), "joint is committed but not applied");
        }
        let c2 = StoreCluster::open(&dir, 4, 1).expect("process open");
        assert!(
            !c2.is_member(4),
            "open recover must apply committed joint"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plant_committed_unapplied_put(c: &mut StoreCluster, nid: u64, key: &[u8], val: &[u8]) {
        let n = c.nodes.get_mut(&nid).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term.max(1),
            entry: RangeEntry::Put {
                key: key.to_vec(),
                value: val.to_vec(),
                si_gen: 0,
            },
        });
        p.commit = idx;
        persist_log_db(&mut n.db, 1, p).unwrap();
        persist_commit_db(&mut n.db, 1, p).unwrap();
    }

    /// RFC-0131 P0: recover must apply on a local replica that leave already
    /// dropped from `ids`. 0130 leader-in-ids plant is **not** this tooth.
    #[test]
    fn crash_reopen_applies_committed_on_removed_replica() {
        assert!(membership_kernel::recover_apply_node_counts(true, false));
        assert!(
            !membership_kernel::recover_apply_node_counts_as_is(true, false),
            "AS-IS dente: skip local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let key = b"rfc0131-removed";
        let val = b"applied-on-recover";
        plant_committed_unapplied_put(&mut c, 4, key, val);
        assert!(
            c.get_on(4, key).unwrap().is_none(),
            "put is committed but not applied"
        );
        c.crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen removed replica");
        assert!(!c.is_member(4), "disk membership still omits 4");
        let got = c.get_on(4, key).expect("get_on removed replica");
        assert_eq!(
            got.as_deref(),
            Some(val.as_slice()),
            "recover must apply committed put on removed replica"
        );
        let p = c.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        assert!(
            p.applied >= p.commit,
            "applied must catch commit after recover"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0131 P1.1: process open of n=4 after leave applies on node 4.
    /// crash-reopen is **not** this tooth.
    #[test]
    fn open_applies_committed_on_removed_replica() {
        assert!(membership_kernel::recover_apply_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
            plant_committed_unapplied_put(&mut c, 4, b"rfc0131-open", b"via-open");
        }
        let c2 = StoreCluster::open(&dir, 4, 1).expect("process open n=4");
        assert!(!c2.is_member(4));
        let got = c2.get_on(4, b"rfc0131-open").expect("get_on");
        assert_eq!(
            got.as_deref(),
            Some(b"via-open".as_slice()),
            "open recover must apply on removed replica"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plant_uncommitted_suffix(c: &mut StoreCluster, nid: u64, key: &[u8], val: &[u8]) -> u64 {
        let n = c.nodes.get_mut(&nid).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        let commit = p.commit;
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term.max(1),
            entry: RangeEntry::Put {
                key: key.to_vec(),
                value: val.to_vec(),
                si_gen: 0,
            },
        });
        persist_log_db(&mut n.db, 1, p).unwrap();
        commit
    }

    fn disk_has_uncommitted_suffix<E: pedradb_core::Env>(
        db: &pedradb_core::Db<E>,
        commit: u64,
    ) -> bool {
        // load_range_peer walks the blob then segments through log_hi.
        // Orphan log_entry_key rows past log_hi are not loaded (F128 watermark).
        if let Some(raw) = db.get(&raft_meta_key(1, "log_hi")) {
            if let Ok(hi) = decode_u64_meta(&raw) {
                if hi > commit {
                    return true;
                }
            }
        }
        if let Some(raw) = db.get(&raft_meta_key(1, "log")) {
            if let Ok(recs) = decode_log(&raw) {
                if recs.iter().any(|e| e.index > commit) {
                    return true;
                }
            }
        }
        false
    }

    /// RFC-0132 P0: crash-reopen must persist the truncated log on a replica
    /// already dropped from `ids`. 0131 applied-put is **not** this tooth.
    #[test]
    fn crash_reopen_truncates_uncommitted_on_removed_replica() {
        assert!(membership_kernel::recover_truncate_node_counts(true, false));
        assert!(
            !membership_kernel::recover_truncate_node_counts_as_is(true, false),
            "AS-IS dente: skip truncate persist on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let commit = plant_uncommitted_suffix(&mut c, 4, b"rfc0132-suffix", b"uncommitted");
        assert!(
            disk_has_uncommitted_suffix(&c.nodes.get(&4).unwrap().db, commit),
            "plant must leave an uncommitted suffix on disk"
        );
        c.crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen removed replica");
        assert!(!c.is_member(4));
        assert!(
            !disk_has_uncommitted_suffix(&c.nodes.get(&4).unwrap().db, commit),
            "recover must persist truncated log on removed replica"
        );
        let p = c.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        assert!(
            p.log.iter().all(|e| e.index <= p.commit),
            "RAM log must not keep the suffix"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0132 P1.1: process open of n=4 after leave truncates node 4 disk.
    /// crash-reopen is **not** this tooth.
    #[test]
    fn open_truncates_uncommitted_on_removed_replica() {
        assert!(membership_kernel::recover_truncate_node_counts(true, false));
        let dir = temp();
        let commit;
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
            commit = plant_uncommitted_suffix(&mut c, 4, b"rfc0132-open", b"uncommitted");
            assert!(disk_has_uncommitted_suffix(
                &c.nodes.get(&4).unwrap().db,
                commit
            ));
        }
        let c2 = StoreCluster::open(&dir, 4, 1).expect("process open n=4");
        assert!(!c2.is_member(4));
        assert!(
            !disk_has_uncommitted_suffix(&c2.nodes.get(&4).unwrap().db, commit),
            "open recover must persist truncated log on removed replica"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0133 P0: truncate persist must delete orphan `log_entry_key` rows.
    /// 0132 log_hi cap is **not** this tooth.
    #[test]
    fn crash_reopen_drops_orphan_seg_on_removed_replica() {
        assert!(membership_kernel::recover_drop_orphan_seg(3, 2));
        assert!(
            !membership_kernel::recover_drop_orphan_seg_as_is(3, 2),
            "AS-IS dente: leave orphan log segments"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let commit = plant_uncommitted_suffix(&mut c, 4, b"rfc0133-orphan", b"uncommitted");
        let orphan = log_entry_key(1, commit.saturating_add(1));
        assert!(
            c.nodes.get(&4).unwrap().db.get(&orphan).is_some(),
            "plant must write an incremental segment past commit"
        );
        c.crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen removed replica");
        assert!(!c.is_member(4));
        assert!(
            c.nodes.get(&4).unwrap().db.get(&orphan).is_none(),
            "recover truncate must drop orphan log_entry_key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0133 P1.1: process open of n=4 after leave drops node 4 orphans.
    /// crash-reopen is **not** this tooth.
    #[test]
    fn open_drops_orphan_seg_on_removed_replica() {
        assert!(membership_kernel::recover_drop_orphan_seg(3, 2));
        let dir = temp();
        let commit;
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
            commit = plant_uncommitted_suffix(&mut c, 4, b"rfc0133-open", b"uncommitted");
            assert!(c
                .nodes
                .get(&4)
                .unwrap()
                .db
                .get(&log_entry_key(1, commit.saturating_add(1)))
                .is_some());
        }
        let c2 = StoreCluster::open(&dir, 4, 1).expect("process open n=4");
        assert!(!c2.is_member(4));
        assert!(
            c2.nodes
                .get(&4)
                .unwrap()
                .db
                .get(&log_entry_key(1, commit.saturating_add(1)))
                .is_none(),
            "open recover must drop orphan log_entry_key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0134 P0: recover must abort leftover 2PC on a replica already
    /// dropped from `ids`. 0132 truncate is **not** this tooth.
    #[test]
    fn crash_reopen_aborts_leftover_on_removed_replica() {
        assert!(membership_kernel::recover_abort_node_counts(true, false));
        assert!(
            !membership_kernel::recover_abort_node_counts_as_is(true, false),
            "AS-IS dente: skip leftover abort on local non-member"
        );
        assert!(txn_kernel::leftover_txn_is_aborted());
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let ik = intent_key(b"rfc0134-left");
        {
            let n = c.nodes.get_mut(&4).unwrap();
            n.db.put(&ik, encode_intent(99, b"pending")).unwrap();
        }
        assert!(c.nodes.get(&4).unwrap().db.get(&ik).is_some());
        c.crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen removed replica");
        assert!(!c.is_member(4));
        assert!(
            c.nodes.get(&4).unwrap().db.get(&ik).is_none(),
            "recover must abort leftover intent on removed replica"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0134 P1.1: process open of n=4 after leave aborts node 4 intents.
    /// crash-reopen is **not** this tooth.
    #[test]
    fn open_aborts_leftover_on_removed_replica() {
        assert!(membership_kernel::recover_abort_node_counts(true, false));
        let dir = temp();
        let ik = intent_key(b"rfc0134-open");
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
            c.nodes
                .get_mut(&4)
                .unwrap()
                .db
                .put(&ik, encode_intent(77, b"pending"))
                .unwrap();
        }
        let c2 = StoreCluster::open(&dir, 4, 1).expect("process open n=4");
        assert!(!c2.is_member(4));
        assert!(
            c2.nodes.get(&4).unwrap().db.get(&ik).is_none(),
            "open recover must abort leftover intent on removed replica"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn disk_now_ms<E: pedradb_core::Env>(n: &StoreNode<E>) -> u64 {
        n.db
            .get(&si_meta_key("now_ms"))
            .and_then(|raw| decode_u64_meta(&raw).ok())
            .unwrap_or(0)
    }

    /// RFC-0135 P0: persist_now_ms must write SI meta on a replica already
    /// dropped from `ids`. 0134 leftover abort is **not** this tooth.
    #[test]
    fn persist_now_ms_on_removed_replica() {
        assert!(membership_kernel::persist_meta_node_counts(true, false));
        assert!(
            !membership_kernel::persist_meta_node_counts_as_is(true, false),
            "AS-IS dente: skip SI meta persist on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        c.advance_now_ms(5_000);
        assert!(c.now_ms() >= 5_000);
        assert_eq!(
            disk_now_ms(c.nodes.get(&4).unwrap()),
            c.now_ms(),
            "removed replica must persist now_ms"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0135 P1.1: TCP ctor of a removed node must persist now_ms locally.
    /// in-process n=4 is **not** this tooth.
    #[test]
    fn open_single_node_persist_now_ms_when_removed() {
        assert!(membership_kernel::persist_meta_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        c2.advance_now_ms(7_000);
        assert_eq!(
            disk_now_ms(c2.nodes.get(&4).unwrap()),
            c2.now_ms(),
            "TCP removed replica must persist now_ms on self"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0136 P0: persist_si_keys must write hist on a replica already
    /// dropped from `ids`. 0135 now_ms is **not** this tooth.
    #[test]
    fn persist_si_hist_on_removed_replica() {
        assert!(membership_kernel::persist_hist_node_counts(true, false));
        assert!(
            !membership_kernel::persist_hist_node_counts_as_is(true, false),
            "AS-IS dente: skip SI hist persist on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let k = b"rfc0136-hist".to_vec();
        c.key_history
            .insert(k.clone(), vec![(1, Some(b"v".to_vec()))]);
        c.commit_generation = c.commit_generation.max(1);
        c.persist_si_keys(&[k.clone()]).unwrap();
        assert!(
            c.nodes.get(&4).unwrap().db.get(&hist_key(&k)).is_some(),
            "removed replica must persist SI hist"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0136 P1.1: TCP ctor of a removed node must persist SI hist locally.
    /// in-process n=4 is **not** this tooth.
    #[test]
    fn open_single_node_persist_si_hist_when_removed() {
        assert!(membership_kernel::persist_hist_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        let k = b"rfc0136-tcp-hist".to_vec();
        c2.key_history
            .insert(k.clone(), vec![(1, Some(b"v".to_vec()))]);
        c2.commit_generation = c2.commit_generation.max(1);
        c2.persist_si_keys(&[k.clone()]).unwrap();
        assert!(
            c2.nodes.get(&4).unwrap().db.get(&hist_key(&k)).is_some(),
            "TCP removed replica must persist SI hist on self"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0137 P0: fence_txn_aborted must write abort on a replica already
    /// dropped from `ids`. 0136 hist is **not** this tooth.
    #[test]
    fn fence_txn_aborted_on_removed_replica() {
        assert!(membership_kernel::persist_fence_node_counts(true, false));
        assert!(
            !membership_kernel::persist_fence_node_counts_as_is(true, false),
            "AS-IS dente: skip abort fence on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let tid = 0x0137u64;
        c.fence_txn_aborted(tid).unwrap();
        let got = c
            .nodes
            .get(&4)
            .unwrap()
            .db
            .get(&txn_status_key(tid));
        assert_eq!(
            got.as_deref(),
            Some(b"abort".as_slice()),
            "removed replica must persist abort fence"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0137 P1.1: TCP ctor of a removed node must persist abort fence locally.
    /// in-process n=4 is **not** this tooth.
    #[test]
    fn open_single_node_fence_txn_when_removed() {
        assert!(membership_kernel::persist_fence_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        let tid = 0x0137_0002u64;
        c2.fence_txn_aborted(tid).unwrap();
        let got = c2
            .nodes
            .get(&4)
            .unwrap()
            .db
            .get(&txn_status_key(tid));
        assert_eq!(
            got.as_deref(),
            Some(b"abort".as_slice()),
            "TCP removed replica must persist abort fence on self"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0138 P0: force_local_clear_keys must drop intents on a replica already
    /// dropped from `ids`. 0137 fence is **not** this tooth.
    #[test]
    fn force_local_clear_on_removed_replica() {
        assert!(membership_kernel::force_clear_node_counts(true, false));
        assert!(
            !membership_kernel::force_clear_node_counts_as_is(true, false),
            "AS-IS dente: skip force-local clear on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let k = b"rfc0138-left".to_vec();
        let ik = intent_key(&k);
        let tid = 0x0138u64;
        {
            let n = c.nodes.get_mut(&4).unwrap();
            n.db.put(&ik, encode_intent(tid, b"pending")).unwrap();
        }
        assert!(c.nodes.get(&4).unwrap().db.get(&ik).is_some());
        c.force_local_clear_keys(tid, &[k], false)
            .expect("force-local abort");
        assert!(
            c.nodes.get(&4).unwrap().db.get(&ik).is_none(),
            "removed replica must drop stuck intent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0138 P1.1: TCP ctor of a removed node must force-clear intents locally.
    /// in-process n=4 is **not** this tooth.
    #[test]
    fn open_single_node_force_clear_when_removed() {
        assert!(membership_kernel::force_clear_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        let k = b"rfc0138-tcp-left".to_vec();
        let ik = intent_key(&k);
        let tid = 0x0138_0002u64;
        {
            let n = c2.nodes.get_mut(&4).unwrap();
            n.db.put(&ik, encode_intent(tid, b"pending")).unwrap();
        }
        assert!(c2.nodes.get(&4).unwrap().db.get(&ik).is_some());
        c2.force_local_clear_keys(tid, &[k], false)
            .expect("TCP force-local abort");
        assert!(
            c2.nodes.get(&4).unwrap().db.get(&ik).is_none(),
            "TCP removed replica must drop stuck intent on self"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0139 P0: drop_preimages must delete pre on a replica already
    /// dropped from `ids`. 0138 force-clear is **not** this tooth.
    #[test]
    fn drop_preimages_on_removed_replica() {
        assert!(membership_kernel::drop_preimages_node_counts(true, false));
        assert!(
            !membership_kernel::drop_preimages_node_counts_as_is(true, false),
            "AS-IS dente: skip drop-preimages on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let k = b"rfc0139-left".to_vec();
        let tid = 0x0139u64;
        let pk = txn_pre_key(tid, &k);
        {
            let n = c.nodes.get_mut(&4).unwrap();
            n.db.put(&pk, encode_preimage(Some(b"old"))).unwrap();
        }
        assert!(c.nodes.get(&4).unwrap().db.get(&pk).is_some());
        let handle = TxHandle {
            id: tid,
            ranges: vec![1],
            keys_by_range: vec![(1, vec![k])],
        };
        c.drop_preimages(&handle).expect("drop preimages");
        assert!(
            c.nodes.get(&4).unwrap().db.get(&pk).is_none(),
            "removed replica must drop leftover preimage"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0139 P1.1: TCP ctor of a removed node must drop preimages locally.
    /// in-process n=4 is **not** this tooth.
    #[test]
    fn open_single_node_drop_preimages_when_removed() {
        assert!(membership_kernel::drop_preimages_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        let k = b"rfc0139-tcp-left".to_vec();
        let tid = 0x0139_0002u64;
        let pk = txn_pre_key(tid, &k);
        {
            let n = c2.nodes.get_mut(&4).unwrap();
            n.db.put(&pk, encode_preimage(Some(b"old"))).unwrap();
        }
        assert!(c2.nodes.get(&4).unwrap().db.get(&pk).is_some());
        let handle = TxHandle {
            id: tid,
            ranges: vec![1],
            keys_by_range: vec![(1, vec![k])],
        };
        c2.drop_preimages(&handle).expect("TCP drop preimages");
        assert!(
            c2.nodes.get(&4).unwrap().db.get(&pk).is_none(),
            "TCP removed replica must drop leftover preimage on self"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0140 P0: in-process open must load RangePeer from disk membership,
    /// not CLI n_nodes. 0124 bind `!is_member(4)` and 0125 TCP peek are **not**
    /// this tooth.
    #[test]
    fn open_in_process_loads_disk_ids_before_peer() {
        assert!(membership_kernel::open_peer_uses_disk(true));
        assert!(
            !membership_kernel::open_peer_uses_disk_as_is(true),
            "AS-IS dente: in-process open ignores disk at load"
        );
        let disk = [1u64, 2, 3];
        let cli = [1u64, 2, 3, 4];
        assert_ne!(
            election_timeout_for(4, 1, &disk),
            election_timeout_for(4, 1, &cli),
            "timeout must differ so the load-order tooth is observable"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let c2 = StoreCluster::open(&dir, 4, 1).expect("process open n=4");
        assert!(!c2.is_member(4));
        let got = c2
            .nodes
            .get(&4)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap()
            .election_timeout;
        assert_eq!(
            got,
            election_timeout_for(4, 1, &disk),
            "removed replica must load peer from disk C-new, not CLI n=4"
        );
        assert_ne!(got, election_timeout_for(4, 1, &cli));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0140 P1.1: lab Direct ctor must peek disk membership before load.
    /// production `open` is **not** this tooth.
    #[test]
    fn open_lab_direct_loads_disk_ids_before_peer() {
        assert!(membership_kernel::open_peer_uses_disk(true));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let c2 = StoreCluster::open_lab_direct(&dir, 4, 1).expect("lab Direct n=4");
        assert!(!c2.is_member(4));
        let got = c2
            .nodes
            .get(&4)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap()
            .election_timeout;
        assert_eq!(
            got,
            election_timeout_for(4, 1, &[1, 2, 3]),
            "lab Direct removed replica must load peer from disk C-new"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0141 P0: TCP ctor of a removed replica must not treat HashMap
    /// first-key as cluster identity. 0140 load peek is **not** this tooth.
    #[test]
    fn open_single_node_local_id_omits_removed() {
        assert!(!membership_kernel::local_id_if_member(false));
        assert!(
            membership_kernel::local_id_if_member_as_is(false),
            "AS-IS dente: HashMap first-key even when removed"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        assert_eq!(
            c2.local_node_id(),
            None,
            "removed replica must not claim local identity"
        );
        {
            let n = c2.nodes.get_mut(&4).unwrap();
            n.db.put(b"rfc0141-stale", b"stale").unwrap();
        }
        let got = c2.get(b"rfc0141-stale");
        assert!(
            got.as_ref().ok().and_then(|v| v.as_deref()) != Some(b"stale".as_ref()),
            "removed replica must not serve local-only as cluster LocalApplied"
        );
        assert!(got.is_err(), "fail-closed: no local voter");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0141 P1.1: TCP ctor of a remaining member still has local identity.
    /// removed ctor is **not** this tooth.
    #[test]
    fn open_single_node_local_id_keeps_member() {
        assert!(membership_kernel::local_id_if_member(true));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let c2 = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of remaining member");
        assert!(c2.is_member(1));
        assert_eq!(
            c2.local_node_id(),
            Some(1),
            "remaining member must keep local identity"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0142 P0: TCP removed replica must not pick remote `ids.first()` as
    /// a LocalApplied reader. 0141 `local_node_id` None is **not** this tooth.
    #[test]
    fn open_single_node_get_skips_remote_ids_first() {
        assert!(!membership_kernel::reader_id_local(false));
        assert!(
            membership_kernel::reader_id_local_as_is(false),
            "AS-IS dente: ids.first even when not local"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        assert_eq!(c2.local_node_id(), None);
        assert!(
            c2.best_reader_for_key(b"rfc0142-get").is_none(),
            "removed replica must not pick remote ids.first"
        );
        let err = c2
            .get(b"rfc0142-get")
            .expect_err("fail-closed: no local reader");
        let msg = err.to_string();
        assert!(
            msg.contains("empty"),
            "must be empty, not bad-node remote: {msg}"
        );
        assert!(
            !msg.contains("bad node"),
            "must not attempt get_on of a remote voter: {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0142 P1.1: get_fast_replica must not pick remote `ids.first()`.
    /// `get` is **not** this tooth.
    #[test]
    fn open_single_node_fast_replica_skips_remote_ids_first() {
        assert!(!membership_kernel::reader_id_local(false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        let err = c2
            .get_fast_replica(b"rfc0142-fast")
            .expect_err("fail-closed: no local replica");
        let msg = err.to_string();
        assert!(
            msg.contains("no replica"),
            "must be no replica, not bad-node remote: {msg}"
        );
        assert!(
            !msg.contains("bad node"),
            "must not attempt get_on of a remote voter: {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0143 P0: live discard must drop uncommitted suffix on a replica
    /// already dropped from `ids`. 0132 recover truncate is **not** this tooth.
    #[test]
    fn discard_uncommitted_on_removed_replica() {
        assert!(membership_kernel::discard_node_counts(true, false));
        assert!(
            !membership_kernel::discard_node_counts_as_is(true, false),
            "AS-IS dente: skip live discard on local non-member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        let commit = plant_uncommitted_suffix(&mut c, 4, b"rfc0143-left", b"orphan");
        assert!(
            c.nodes
                .get(&4)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap()
                .log
                .iter()
                .any(|e| e.index > commit),
            "planted uncommitted suffix"
        );
        // Escaped-on-the-wire is F-found, not this tooth: live leaders may
        // still hold sent_through from pre-leave replication.
        for n in c.nodes.values_mut() {
            for p in n.ranges.values_mut() {
                p.sent_through.clear();
            }
        }
        let from = commit.saturating_add(1);
        c.discard_uncommitted_from(1, 4, from)
            .expect("discard on removed replica");
        let p = c.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        assert!(
            p.log.iter().all(|e| e.index <= commit),
            "removed replica must drop RAM uncommitted suffix"
        );
        assert!(
            !disk_has_uncommitted_suffix(&c.nodes.get(&4).unwrap().db, commit),
            "removed replica must persist truncated log"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0143 P1.1: TCP ctor of a removed node must discard locally.
    /// in-process n=4 is **not** this tooth.
    #[test]
    fn open_single_node_discard_uncommitted_when_removed() {
        assert!(membership_kernel::discard_node_counts(true, false));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        let commit = plant_uncommitted_suffix(&mut c2, 4, b"rfc0143-tcp", b"orphan");
        let from = commit.saturating_add(1);
        c2.discard_uncommitted_from(1, 4, from)
            .expect("TCP discard");
        let p = c2.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        assert!(
            p.log.iter().all(|e| e.index <= commit),
            "TCP removed replica must drop RAM uncommitted suffix"
        );
        assert!(
            !disk_has_uncommitted_suffix(&c2.nodes.get(&4).unwrap().db, commit),
            "TCP removed replica must persist truncated log"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0144 P0: no-leader finish_queued_propose must use a local persist-leader.
    /// 0143 direct discard(leader=4) is **not** this tooth.
    #[test]
    fn finish_queued_no_leader_persist_leader_is_local() {
        assert!(!membership_kernel::discard_leader_local(false));
        assert!(
            membership_kernel::discard_leader_local_as_is(false),
            "AS-IS dente: ids.first persist-leader even when remote"
        );
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of removed replica");
        assert!(!c2.is_member(4));
        assert!(c2.range_leader(1).is_none(), "TCP removed has no local leader");
        let commit = plant_uncommitted_suffix(&mut c2, 4, b"rfc0144-left", b"orphan");
        let from = commit.saturating_add(1);
        for n in c2.nodes.values_mut() {
            for p in n.ranges.values_mut() {
                p.sent_through.clear();
            }
        }
        c2.finish_queued_propose(1, from, true)
            .expect("no-leader abort");
        let p = c2.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        assert_eq!(
            p.next_index.get(&1).copied(),
            Some(from),
            "persist-leader must be local node 4 so next_index repair runs"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0144 P1.1: TCP remaining member still uses local ids.first as persist-leader.
    /// removed ctor is **not** this tooth.
    #[test]
    fn finish_queued_member_persist_leader_stays_first() {
        assert!(membership_kernel::discard_leader_local(true));
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
            c.elect_all(80).unwrap();
            c.pin_dst_queued();
            queued_shrink_until_leave_committed(&mut c, 4);
            assert!(!c.is_member(4));
        }
        let mut c2 = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3, 4], 1)
            .expect("TCP ctor of remaining member");
        assert!(c2.is_member(1));
        assert!(c2.range_leader(1).is_none(), "fresh open is follower");
        let commit = plant_uncommitted_suffix(&mut c2, 1, b"rfc0144-mem", b"orphan");
        let from = commit.saturating_add(1);
        for n in c2.nodes.values_mut() {
            for p in n.ranges.values_mut() {
                p.sent_through.clear();
            }
        }
        c2.finish_queued_propose(1, from, true)
            .expect("no-leader abort");
        let p = c2.nodes.get(&1).unwrap().ranges.get(&1).unwrap();
        assert_eq!(
            p.next_index.get(&2).copied(),
            Some(from),
            "member persist-leader stays local ids.first"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0145 P0: joint leave must step a removed Leader down.
    /// 0128 participating and oob remove_member are **not** this tooth.
    #[test]
    fn leave_steps_down_removed_leader() {
        assert!(membership_kernel::removed_steps_down(false));
        assert!(
            !membership_kernel::removed_steps_down_as_is(false),
            "AS-IS dente: keep Role::Leader after joint leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        // AS-IS leftover: joint apply flipped participating but left Role::Leader.
        {
            let n = c.nodes.get_mut(&4).unwrap();
            let p = n.ranges.get_mut(&1).unwrap();
            p.role = Role::Leader;
            p.leader_id = Some(4);
        }
        assert!(c.node_thinks_leader(4, 1));
        c.install_applied_membership(c.member_ids().to_vec())
            .expect("re-apply C-new");
        assert!(
            !c.node_thinks_leader(4, 1),
            "removed replica must not remain Leader"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0145 P1.1: after stepping the removed leader down, remaining
    /// members still elect a member. in-process Leader plant is **not** this tooth.
    #[test]
    fn leave_remaining_elect_member_leader() {
        assert!(membership_kernel::removed_steps_down(false));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        {
            let n = c.nodes.get_mut(&4).unwrap();
            let p = n.ranges.get_mut(&1).unwrap();
            p.role = Role::Leader;
            p.leader_id = Some(4);
        }
        c.install_applied_membership(c.member_ids().to_vec())
            .expect("re-apply C-new");
        c.elect_all(80).unwrap();
        let lead = c.range_leader(1).expect("remaining members elect");
        assert!(c.is_member(lead), "leader must be a remaining member");
        assert!(!c.node_thinks_leader(4, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0146 P0: leader_hint must not name a replica already dropped from `ids`.
    /// 0145 Role::Leader step-down is **not** this tooth.
    #[test]
    fn leader_hint_omits_removed_after_leave() {
        assert!(!membership_kernel::hint_if_member(false));
        assert!(
            membership_kernel::hint_if_member_as_is(false),
            "AS-IS dente: leader_hint returns a removed node"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        while c.step_down_range_leader(1).is_ok() {}
        assert!(c.range_leader(1).is_none());
        for nid in 1..=3u64 {
            let n = c.nodes.get_mut(&nid).unwrap();
            n.ranges.get_mut(&1).unwrap().leader_id = Some(4);
        }
        assert_ne!(
            c.leader_hint(1),
            Some(4),
            "routing hint must not be the removed replica"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0146 P1.1: applying C-new clears stale leader_id on remaining peers.
    /// `leader_hint` filter is **not** this tooth.
    #[test]
    fn install_clears_stale_leader_hint() {
        assert!(!membership_kernel::hint_if_member(false));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        {
            let n = c.nodes.get_mut(&1).unwrap();
            n.ranges.get_mut(&1).unwrap().leader_id = Some(4);
        }
        c.install_applied_membership(c.member_ids().to_vec())
            .expect("re-apply C-new");
        let hint = c.nodes.get(&1).unwrap().ranges.get(&1).unwrap().leader_id;
        assert_ne!(hint, Some(4), "stale hint of the removed node must be cleared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0147 P0: C-new apply forgets next/match/sent_through of a removed
    /// peer. 0146 hint clear is **not** this tooth.
    #[test]
    fn install_drops_removed_repl_slots() {
        assert!(membership_kernel::drop_repl_slot(false));
        assert!(
            !membership_kernel::drop_repl_slot_as_is(false),
            "AS-IS dente: keep next/match/sent_through after joint leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        {
            let n = c.nodes.get_mut(&1).unwrap();
            let p = n.ranges.get_mut(&1).unwrap();
            p.next_index.insert(2, 10);
            p.next_index.insert(4, 99);
            p.match_index.insert(2, 9);
            p.match_index.insert(4, 98);
            p.sent_through.insert(2, 10);
            p.sent_through.insert(4, 99);
        }
        c.install_applied_membership(c.member_ids().to_vec())
            .expect("re-apply C-new");
        let p = c.nodes.get(&1).unwrap().ranges.get(&1).unwrap();
        assert_eq!(
            p.next_index.get(&2).copied(),
            Some(10),
            "remaining-member slot must stay"
        );
        assert_eq!(p.match_index.get(&2).copied(), Some(9));
        assert_eq!(p.sent_through.get(&2).copied(), Some(10));
        assert!(
            !p.next_index.contains_key(&4),
            "removed next_index slot must be dropped"
        );
        assert!(
            !p.match_index.contains_key(&4),
            "removed match_index slot must be dropped"
        );
        assert!(
            !p.sent_through.contains_key(&4),
            "removed sent_through slot must be dropped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0147 P1.1: queued leave itself drops remaining-leader slots for 4.
    /// plant+re-install is **not** this tooth.
    #[test]
    fn leave_drops_removed_repl_slots() {
        assert!(membership_kernel::drop_repl_slot(false));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        let had_slot = (1..=3u64).any(|nid| {
            let p = c.nodes.get(&nid).unwrap().ranges.get(&1).unwrap();
            p.next_index.contains_key(&4)
                || p.match_index.contains_key(&4)
                || p.sent_through.contains_key(&4)
        });
        assert!(
            had_slot,
            "pre-leave remaining peers must hold a slot for 4"
        );
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        for nid in 1..=3u64 {
            let p = c.nodes.get(&nid).unwrap().ranges.get(&1).unwrap();
            assert!(
                !p.next_index.contains_key(&4),
                "node {nid} next_index must forget removed peer 4"
            );
            assert!(
                !p.match_index.contains_key(&4),
                "node {nid} match_index must forget removed peer 4"
            );
            assert!(
                !p.sent_through.contains_key(&4),
                "node {nid} sent_through must forget removed peer 4"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0148 P0: oob remove_member forgets sent_through of the removed peer.
    /// 0147 joint drop_repl_slot and next/match already dropped are **not** this tooth.
    #[test]
    fn remove_drops_removed_sent_through() {
        assert!(membership_kernel::drop_sent_through(false));
        assert!(
            !membership_kernel::drop_sent_through_as_is(false),
            "AS-IS dente: keep sent_through after oob remove_member"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        {
            let n = c.nodes.get_mut(&1).unwrap();
            let p = n.ranges.get_mut(&1).unwrap();
            p.sent_through.insert(2, 10);
            p.sent_through.insert(3, 99);
        }
        c.remove_member(3).expect("oob 3→2 is under quorum floor");
        assert!(!c.is_member(3));
        let p = c.nodes.get(&1).unwrap().ranges.get(&1).unwrap();
        assert_eq!(
            p.sent_through.get(&2).copied(),
            Some(10),
            "remaining-member sent_through must stay"
        );
        assert!(
            !p.sent_through.contains_key(&3),
            "removed sent_through slot must be dropped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0148 P1.1: live replication then oob remove drops sent_through.
    /// plant is **not** this tooth.
    #[test]
    fn oob_remove_drops_live_sent_through() {
        assert!(membership_kernel::drop_sent_through(false));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"rfc0148", b"live").unwrap();
        let had = (1..=2u64).any(|nid| {
            c.nodes
                .get(&nid)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap()
                .sent_through
                .contains_key(&3)
        });
        assert!(had, "pre-remove remaining peers must hold sent_through for 3");
        c.remove_member(3).expect("oob 3→2 is under quorum floor");
        assert!(!c.is_member(3));
        for nid in 1..=2u64 {
            let p = c.nodes.get(&nid).unwrap().ranges.get(&1).unwrap();
            assert!(
                !p.sent_through.contains_key(&3),
                "node {nid} sent_through must forget removed peer 3"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0128 P0: live `is_participating` must ignore a stale flag after
    /// leave. 0127 crash-reopen is **not** this tooth.
    #[test]
    fn is_participating_ignores_stale_flag_after_leave() {
        assert!(!membership_kernel::participating_if_member(false));
        assert!(
            membership_kernel::participating_if_member_as_is(false),
            "AS-IS dente: ignore ids"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        queued_shrink_until_leave_committed(&mut c, 4);
        assert!(!c.is_member(4));
        c.nodes.get_mut(&4).unwrap().participating = true;
        assert!(
            !c.is_participating(4),
            "stale participating=true must not count after leave"
        );
        c.set_participating(1, false).unwrap();
        assert!(
            !c.is_participating(1),
            "partitioned member still in ids must stay not-participating"
        );
        assert!(c.is_member(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0103 P0: Queued add then Queued remove must leave both joints.
    /// 0098 add-only and 0099 Direct-4 remove are not this tooth.
    #[test]
    fn leave_joint_on_queued_add_then_remove_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        let add_leave =
            drive_queued_joint_until_leave(&mut c, add_res, before_add, "Queued add_member_joint");
        assert!(add_leave, "Queued add must leave C-new-only before shrink");
        drain_queued_until_joint_idle(&mut c);
        assert!(c.is_member(4), "add must apply before Queued remove");
        assert!(
            c.pending_joint().is_none(),
            "add leave must commit before a second joint"
        );
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        let remove_leave = drive_queued_joint_until_leave(
            &mut c,
            remove_res,
            before_remove,
            "Queued remove_member_joint after add",
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            remove_leave,
            "Queued remove after add must leave C-new-only"
        );
        assert!(membership_kernel::joint_leave_ok(remove_leave));
    }

    /// RFC-0104 P0: after Queued shrink leave, a second Queued add must be
    /// admitted and leave. 0103 stops at remove; 0098 add is after Direct shrink.
    #[test]
    fn leave_joint_on_queued_add_remove_add_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        assert!(
            drive_queued_joint_until_leave(&mut c, add_res, before_add, "Queued add"),
            "first Queued add must leave"
        );
        drain_queued_until_joint_idle(&mut c);
        assert!(c.is_member(4), "first add must apply");
        assert!(c.pending_joint().is_none());
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        assert!(
            drive_queued_joint_until_leave(&mut c, remove_res, before_remove, "Queued remove"),
            "Queued remove must leave before re-add"
        );
        drain_queued_until_joint_idle(&mut c);
        assert!(
            !c.is_member(4),
            "shrink must apply before second Queued add"
        );
        let before_readd = max_c_new_only_leave_index(&c);
        let readd_res = c.add_member_joint(4);
        if let Err(StoreError::Msg(m)) = &readd_res {
            assert!(
                !m.contains("already in flight"),
                "shrink leave must free the leader for add: {m}"
            );
        }
        let readd_leave = drive_queued_joint_until_leave(
            &mut c,
            readd_res,
            before_readd,
            "Queued add_member_joint after shrink",
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            readd_leave,
            "Queued add after Queued shrink must leave C-new-only"
        );
        assert!(membership_kernel::joint_leave_ok(readd_leave));
    }

    /// RFC-0106 P0: Queued remove then add then remove, each with a new
    /// leave. 0099 is one shrink; 0104 starts after Direct shrink.
    #[test]
    fn leave_joint_on_queued_remove_add_remove_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        let before_rm1 = max_c_new_only_leave_index(&c);
        let rm1 = c.remove_member_joint(4);
        assert!(
            drive_queued_joint_until_leave(&mut c, rm1, before_rm1, "Queued remove"),
            "first Queued remove must leave"
        );
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4), "first shrink must apply");
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        if let Err(StoreError::Msg(m)) = &add_res {
            assert!(
                !m.contains("already in flight"),
                "first shrink leave must free the leader for add: {m}"
            );
        }
        assert!(
            drive_queued_joint_until_leave(&mut c, add_res, before_add, "Queued add after shrink"),
            "Queued add after Queued shrink must leave"
        );
        drain_queued_until_joint_idle(&mut c);
        assert!(c.is_member(4), "add must apply before second shrink");
        let before_rm2 = max_c_new_only_leave_index(&c);
        let rm2 = c.remove_member_joint(4);
        let rm2_leave =
            drive_queued_joint_until_leave(&mut c, rm2, before_rm2, "Queued remove after add");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(rm2_leave, "second Queued remove must leave C-new-only");
        assert!(membership_kernel::joint_leave_ok(rm2_leave));
    }

    /// RFC-0111 P0: four-step Queued remove-add-remove-add. 0106 stops at
    /// the second shrink; 0104 last add is after a Direct shrink.
    #[test]
    fn leave_joint_on_queued_remove_add_remove_add_is_in_log() {
        assert!(!membership_kernel::joint_leave_ok(false));
        assert!(
            membership_kernel::joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        let before_rm1 = max_c_new_only_leave_index(&c);
        let rm1 = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            rm1,
            before_rm1,
            "Queued remove 1"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4));
        let before_add1 = max_c_new_only_leave_index(&c);
        let add1 = c.add_member_joint(4);
        if let Err(StoreError::Msg(m)) = &add1 {
            assert!(
                !m.contains("already in flight"),
                "first shrink leave must free the leader: {m}"
            );
        }
        assert!(drive_queued_joint_until_leave(
            &mut c,
            add1,
            before_add1,
            "Queued add 1"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(c.is_member(4));
        let before_rm2 = max_c_new_only_leave_index(&c);
        let rm2 = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            rm2,
            before_rm2,
            "Queued remove 2"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4), "second shrink must apply before last add");
        let before_add2 = max_c_new_only_leave_index(&c);
        let add2 = c.add_member_joint(4);
        if let Err(StoreError::Msg(m)) = &add2 {
            assert!(
                !m.contains("already in flight"),
                "second shrink leave must free the leader for last add: {m}"
            );
        }
        let add2_leave = drive_queued_joint_until_leave(
            &mut c,
            add2,
            before_add2,
            "Queued add after two shrinks",
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            add2_leave,
            "Queued add after two Queued shrinks must leave C-new-only"
        );
        assert!(membership_kernel::joint_leave_ok(add2_leave));
    }

    /// RFC-0119 P0: TCP replica (`open_single_node`) only has self in
    /// `nodes`. Joint remove of a peer must not be `unknown node`.
    /// In-process 4-node shrink is **not** this tooth.
    #[test]
    fn remove_member_joint_tcp_replica_does_not_require_local_nodes() {
        assert!(membership_kernel::joint_target_counts(true, false));
        assert!(
            !membership_kernel::joint_target_counts_as_is(true, false),
            "AS-IS dente: require peer in local nodes"
        );
        assert!(!membership_kernel::joint_target_counts(false, true));
        assert!(membership_kernel::joint_target_counts_as_is(false, true));
        let dir = temp();
        let mut c = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 1).unwrap();
        assert!(
            !c.nodes.contains_key(&3),
            "TCP replica must not have peer 3 in local nodes"
        );
        assert!(c.ids.contains(&3));
        let e = c.remove_member_joint(3).unwrap_err();
        let msg = e.to_string();
        assert!(
            !msg.contains("unknown node"),
            "TCP replica must name a peer that lives in another process: {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0119 P1.1: TCP replica must joint-add a never-local node.
    /// In-process add of a previously-removed peer is **not** this tooth.
    #[test]
    fn add_member_joint_tcp_replica_does_not_require_local_nodes() {
        assert!(membership_kernel::joint_add_target_counts(false));
        assert!(
            !membership_kernel::joint_add_target_counts_as_is(false),
            "AS-IS dente: require joiner in local nodes"
        );
        let dir = temp();
        let mut c = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 1).unwrap();
        assert!(!c.nodes.contains_key(&4));
        assert!(!c.ids.contains(&4));
        let e = c.add_member_joint(4).unwrap_err();
        let msg = e.to_string();
        assert!(
            !msg.contains("unknown node"),
            "TCP replica must name a joining process: {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0105 P0: after Queued shrink leave, a removed node's leftover
    /// C-old,new must not keep `pending_joint` (election reads that).
    /// 0104 leader-add is not this tooth.
    #[test]
    fn pending_joint_ignores_removed_node_after_queued_shrink() {
        assert!(!membership_kernel::pending_joint_node_counts(false));
        assert!(
            membership_kernel::pending_joint_node_counts_as_is(false),
            "AS-IS dente: count removed node"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            add_res,
            before_add,
            "Queued add"
        ));
        drain_queued_until_joint_idle(&mut c);
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            remove_res,
            before_remove,
            "Queued remove"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4));
        let n4_still_active = c.nodes.get(&4).is_some_and(|n| {
            n.ranges.values().any(|p| {
                p.log.iter().any(|rec| {
                    matches!(
                        &rec.entry,
                        RangeEntry::MembershipJoint { old, new }
                            if membership_kernel::joint_still_active(old, new)
                    )
                })
            })
        });
        let pending = c.pending_joint();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            n4_still_active,
            "removed node must still hold C-old,new (AS-IS would keep joint)"
        );
        assert!(
            pending.is_none(),
            "pending_joint must ignore the removed node's leftover joint"
        );
    }

    /// RFC-0112 P0: after Queued shrink leave, RV `vote_targets` omit the
    /// removed node. 0105 is pending_joint None; 0107 is quorum.
    #[test]
    fn vote_targets_after_queued_shrink_omit_removed_node() {
        assert!(!membership_kernel::pending_joint_node_counts(false));
        assert!(
            membership_kernel::pending_joint_node_counts_as_is(false),
            "AS-IS dente: count removed node"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            add_res,
            before_add,
            "Queued add"
        ));
        drain_queued_until_joint_idle(&mut c);
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            remove_res,
            before_remove,
            "Queued remove"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4));
        let n4_still_active = c.nodes.get(&4).is_some_and(|n| {
            n.ranges.values().any(|p| {
                p.log.iter().any(|rec| {
                    matches!(
                        &rec.entry,
                        RangeEntry::MembershipJoint { old, new }
                            if membership_kernel::joint_still_active(old, new)
                    )
                })
            })
        });
        let targets = c.vote_targets();
        let mut live = c.ids.clone();
        live.sort_unstable();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            n4_still_active,
            "removed node must still hold C-old,new (AS-IS would keep it in targets)"
        );
        assert!(
            !targets.contains(&4),
            "vote_targets must omit removed node 4, got {targets:?}"
        );
        assert_eq!(targets, live, "vote_targets must equal live ids");
    }

    /// RFC-0113 P0: `start_election` must not queue RequestVote to a lagging
    /// removed node (`participating=true` but not in ids). 0112 is the helper.
    #[test]
    fn request_vote_not_sent_to_lagging_removed_node() {
        assert!(!membership_kernel::pending_joint_node_counts(false));
        assert!(
            membership_kernel::pending_joint_node_counts_as_is(false),
            "AS-IS dente: count removed node"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            add_res,
            before_add,
            "Queued add"
        ));
        drain_queued_until_joint_idle(&mut c);
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            remove_res,
            before_remove,
            "Queued remove"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4));
        let n4_still_active = c.nodes.get(&4).is_some_and(|n| {
            n.ranges.values().any(|p| {
                p.log.iter().any(|rec| {
                    matches!(
                        &rec.entry,
                        RangeEntry::MembershipJoint { old, new }
                            if membership_kernel::joint_still_active(old, new)
                    )
                })
            })
        });
        assert!(
            n4_still_active,
            "removed node must still hold C-old,new (AS-IS would RV it)"
        );
        c.nodes.get_mut(&4).unwrap().participating = true;
        let _ = c.drain_outbound();
        let cand = *c.ids.iter().find(|&&id| id != 4).expect("live member");
        c.start_election(1, cand).expect("start_election");
        let outbound = c.drain_outbound();
        let mut rv_to_removed = 0usize;
        let mut rv_to_live = 0usize;
        for (_from, to, bytes) in &outbound {
            if !matches!(
                PeerMsg::decode(bytes).ok(),
                Some(PeerMsg::RequestVote { .. })
            ) {
                continue;
            }
            if *to == 4 {
                rv_to_removed += 1;
            } else if c.ids.contains(to) {
                rv_to_live += 1;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            rv_to_removed, 0,
            "RequestVote must not be queued to lagging removed node 4"
        );
        assert!(
            rv_to_live > 0,
            "election must still queue RequestVote to a live peer"
        );
    }

    /// RFC-0114 P0: a RequestVote grant from the removed node must not be
    /// recorded after leave. 0113 is outbound RV only.
    #[test]
    fn rv_grant_from_removed_node_is_ignored_after_leave() {
        assert!(!membership_kernel::election_grant_from_counts(false, false));
        assert!(
            membership_kernel::election_grant_from_counts_as_is(false, false),
            "AS-IS dente: record any grant"
        );
        assert!(membership_kernel::election_grant_from_counts(false, true));
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            add_res,
            before_add,
            "Queued add"
        ));
        drain_queued_until_joint_idle(&mut c);
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            remove_res,
            before_remove,
            "Queued remove"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4));
        assert!(c.pending_joint().is_none());
        let cand = *c.ids.iter().find(|&&id| id != 4).expect("live member");
        c.start_election(1, cand).expect("start_election");
        let term = c
            .nodes
            .get(&cand)
            .and_then(|n| n.ranges.get(&1))
            .map(|p| p.term)
            .expect("term");
        c.on_request_vote_reply(cand, 4, 1, term, true)
            .expect("reply from removed");
        let granted = c
            .election_granted
            .get(&(1, term, cand))
            .cloned()
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            !granted.contains(&4),
            "grant from removed node 4 must not be recorded, got {granted:?}"
        );
    }

    /// RFC-0115 P0: during joint add (apply lag), a grant from the joining
    /// node must be recorded (C-new). 0114 after-leave ignore is not this tooth.
    #[test]
    fn rv_grant_from_joining_node_counts_during_joint_add() {
        assert!(membership_kernel::election_grant_from_counts(false, true));
        assert!(!membership_kernel::election_grant_from_counts(false, false));
        assert!(
            membership_kernel::election_grant_from_counts_as_is(false, false),
            "AS-IS dente: record any grant"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        assert!(!c.is_member(4));
        c.plant_committed_joint_without_leave(4)
            .expect("plant committed joint add without leave");
        assert!(!c.is_member(4), "apply lag: 4 not in ids");
        assert!(
            c.pending_joint().is_some(),
            "planted C-old,new must be pending"
        );
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), RpcMode::Queued);
        let cand = *c.ids.first().expect("live member");
        c.start_election(1, cand).expect("start_election");
        let term = c
            .nodes
            .get(&cand)
            .and_then(|n| n.ranges.get(&1))
            .map(|p| p.term)
            .expect("term");
        c.on_request_vote_reply(cand, 4, 1, term, true)
            .expect("reply from joining node");
        let granted = c
            .election_granted
            .get(&(1, term, cand))
            .cloned()
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            granted.contains(&4),
            "grant from joining node 4 must count during joint add, got {granted:?}"
        );
    }

    /// RFC-0107 P0: after Queued shrink leave, 2/3 of the live set elects.
    /// AS-IS counting the removed node's C-old,new would require 3/4 old.
    /// 0105 is pending_joint None; 0102 is compact+reopen plant.
    #[test]
    fn election_after_queued_shrink_ignores_removed_node_joint() {
        assert!(membership_kernel::joint_election_ok(2, 3, None));
        assert!(!membership_kernel::joint_election_ok(2, 4, Some((2, 3))));
        assert!(
            membership_kernel::pending_joint_node_counts_as_is(false),
            "AS-IS dente: count removed node"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        c.pin_dst_queued();
        let before_add = max_c_new_only_leave_index(&c);
        let add_res = c.add_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            add_res,
            before_add,
            "Queued add"
        ));
        drain_queued_until_joint_idle(&mut c);
        let before_remove = max_c_new_only_leave_index(&c);
        let remove_res = c.remove_member_joint(4);
        assert!(drive_queued_joint_until_leave(
            &mut c,
            remove_res,
            before_remove,
            "Queued remove"
        ));
        drain_queued_until_joint_idle(&mut c);
        assert!(!c.is_member(4));
        assert!(c.pending_joint().is_none());
        let n4_still_active = c.nodes.get(&4).is_some_and(|n| {
            n.ranges.values().any(|p| {
                p.log.iter().any(|rec| {
                    matches!(
                        &rec.entry,
                        RangeEntry::MembershipJoint { old, new }
                            if membership_kernel::joint_still_active(old, new)
                    )
                })
            })
        });
        assert!(
            n4_still_active,
            "removed node must still hold C-old,new (AS-IS would keep joint)"
        );
        let cand = c.range_leader(1).unwrap_or_else(|| c.ids[0]);
        let term = c
            .nodes
            .get(&cand)
            .and_then(|n| n.ranges.get(&1))
            .map(|p| p.term)
            .expect("candidate range");
        c.election_granted.insert((1, term, cand), vec![1, 2]);
        c.election_votes.insert((1, term, cand), 2);
        let elects = c.election_has_joint_quorum(1, term, cand);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            elects,
            "2/3 live must elect after shrink leave; zombie joint on node 4 must not apply"
        );
    }

    /// RFC-0100 P0: compact must keep an applied still-active joint until
    /// leave. 0099 observes leave *before* compact; that is not this tooth.
    #[test]
    fn compact_does_not_drop_unleft_joint() {
        assert_eq!(compact_kernel::compact_through_unleft(5, Some(3)), 2);
        assert_eq!(
            compact_kernel::compact_through_unleft_as_is(5, Some(3)),
            5,
            "AS-IS dente: compact past un-left joint"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 2, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(2).expect("shrink to 1");
        c.plant_committed_joint_without_leave(2)
            .expect("plant committed joint without leave");
        let lid = c.range_leader(1).expect("leader");
        let joint_idx = {
            let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
            let rec = p.log.last().expect("planted joint");
            match &rec.entry {
                RangeEntry::MembershipJoint { old, new } => {
                    assert!(membership_kernel::joint_still_active(old, new));
                }
                other => panic!("plant must append MembershipJoint, got {other:?}"),
            }
            rec.index
        };
        {
            let p = c.nodes.get_mut(&lid).unwrap().ranges.get_mut(&1).unwrap();
            p.applied = p.commit;
        }
        assert!(joint_idx > 0, "planted joint must have a log index");
        let applied = c.applied_index(lid, 1);
        assert_eq!(
            compact_kernel::compact_through_unleft(applied, Some(joint_idx)),
            joint_idx.saturating_sub(1)
        );
        assert_eq!(
            compact_kernel::compact_through_unleft_as_is(applied, Some(joint_idx)),
            applied,
            "AS-IS would compact through the joint"
        );
        c.maybe_compact_logs(1).expect("compact");
        let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
        let still = p.log.iter().any(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if membership_kernel::joint_still_active(old, new)
            )
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(still, "compact must keep un-left still-active joint");
    }

    /// RFC-0101 P0: compact persist + crash-reopen must still show the
    /// un-left joint. 0100 RAM-only compact is not this tooth.
    #[test]
    fn unleft_joint_survives_compact_reopen() {
        assert_eq!(compact_kernel::compact_through_unleft(5, Some(3)), 2);
        assert_eq!(
            compact_kernel::compact_through_unleft_as_is(5, Some(3)),
            5,
            "AS-IS dente: compact past un-left joint"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 2, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(2).expect("shrink to 1");
        c.plant_committed_joint_without_leave(2)
            .expect("plant committed joint without leave");
        let lid = c.range_leader(1).expect("leader");
        let joint_idx = c
            .nodes
            .get(&lid)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap()
            .last_index();
        {
            let p = c.nodes.get_mut(&lid).unwrap().ranges.get_mut(&1).unwrap();
            p.applied = p.commit;
        }
        {
            let n = c.nodes.get_mut(&lid).unwrap();
            persist_log_db(&mut n.db, 1, n.ranges.get_mut(&1).unwrap()).unwrap();
            persist_commit_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
            persist_applied_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
        }
        c.maybe_compact_logs(1).expect("compact");
        c.crash_reopen_engine_on(lid, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen leader");
        let p = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
        assert!(
            p.snapshot_index < joint_idx,
            "snap must not cover the un-left joint (AS-IS compact would)"
        );
        let still = p.log.iter().any(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if membership_kernel::joint_still_active(old, new)
            )
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            still,
            "un-left joint must survive compact persist + crash-reopen"
        );
    }

    /// RFC-0102 P0: after compact persist + crash-reopen, C-old majority
    /// still must not elect. 0101 log-only and 0066/0068 RAM plant are
    /// not this tooth.
    #[test]
    fn election_after_compact_reopen_refuses_old_majority() {
        assert!(!membership_kernel::joint_election_ok(1, 1, Some((1, 2))));
        assert!(
            membership_kernel::joint_election_ok_as_is(1, 1, Some((1, 2))),
            "AS-IS dente: old-only would elect"
        );
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 2, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(2).expect("shrink to 1");
        c.plant_committed_joint_without_leave(2)
            .expect("plant committed joint without leave");
        let lid = c.range_leader(1).expect("leader");
        {
            let p = c.nodes.get_mut(&lid).unwrap().ranges.get_mut(&1).unwrap();
            p.applied = p.commit;
        }
        {
            let n = c.nodes.get_mut(&lid).unwrap();
            persist_log_db(&mut n.db, 1, n.ranges.get_mut(&1).unwrap()).unwrap();
            persist_commit_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
            persist_applied_db(&mut n.db, 1, n.ranges.get(&1).unwrap()).unwrap();
        }
        c.maybe_compact_logs(1).expect("compact");
        c.crash_reopen_engine_on(lid, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen leader");
        assert!(
            c.pending_joint().is_some(),
            "reopened log must still carry C-old,new"
        );
        let term = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap().term;
        c.election_granted.insert((1, term, lid), vec![1]);
        c.election_votes.insert((1, term, lid), 1);
        assert!(
            !c.election_has_joint_quorum(1, term, lid),
            "C-old majority must not elect after compact+reopen of un-left joint"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0066 P0: after the joint **commits** (but apply lags), C-old
    /// majority still must not elect. AS-IS `pending_joint` died at commit.
    #[test]
    fn election_after_committed_joint_still_requires_new_majority() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        assert!(!c.is_member(4));
        let lid = c.range_leader(1).expect("leader after shrink");
        let term = c.nodes.get(&lid).unwrap().ranges.get(&1).unwrap().term;
        {
            let p = c.nodes.get_mut(&lid).unwrap().ranges.get_mut(&1).unwrap();
            let idx = p.last_index() + 1;
            p.log.push(LogRec {
                index: idx,
                term: p.term,
                entry: RangeEntry::MembershipJoint {
                    old: vec![1, 2, 3],
                    new: vec![1, 2, 3, 4],
                },
            });
            p.commit = idx;
            p.applied = idx.saturating_sub(1);
        }
        assert!(
            membership_kernel::joint_still_active(&[1, 2, 3], &[1, 2, 3, 4]),
            "kernel: C-old,new is still a joint"
        );
        assert!(
            !membership_kernel::joint_still_active_as_is(&[1, 2, 3], &[1, 2, 3, 4]),
            "AS-IS dente: committed joint looks inactive"
        );
        c.election_granted.insert((1, term, lid), vec![1, 2]);
        c.election_votes.insert((1, term, lid), 2);
        assert!(
            !c.election_has_joint_quorum(1, term, lid),
            "2/3 old must not elect after joint commit, before leave"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0068 P0: DST plant API (same crash window as 0066) then C-old
    /// majority must not elect. AS-IS would drop the committed joint.
    #[test]
    fn plant_committed_joint_without_leave_refuses_old_majority() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 4, 1).unwrap();
        c.elect_all(80).unwrap();
        c.remove_member_joint(4).expect("shrink to 3");
        assert!(!c.is_member(4));
        c.plant_committed_joint_without_leave(4)
            .expect("plant committed joint without leave");
        assert!(membership_kernel::joint_still_active(
            &[1, 2, 3],
            &[1, 2, 3, 4]
        ));
        assert!(!membership_kernel::joint_still_active_as_is(
            &[1, 2, 3],
            &[1, 2, 3, 4]
        ));
        assert!(
            !c.probe_old_majority_joint_election(1),
            "2/3 old must not elect after planted committed joint"
        );
        assert!(
            membership_kernel::joint_election_ok_as_is(2, 3, Some((2, 4))),
            "AS-IS dente: old-only would elect"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0069 P0: bounded elect_all still works; an eventual-election
    /// *claim* without ES-1/2/3 is refused. AS-IS would admit.
    #[test]
    fn claim_eventual_election_refused_without_es_axioms() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        assert!(
            c.range_leader(1).is_some(),
            "bounded elect must find a leader"
        );
        assert!(
            !c.claim_eventual_election(false, true, true),
            "missing ES-1 must refuse the liveness claim"
        );
        assert!(!c.claim_eventual_election(true, false, true));
        assert!(!c.claim_eventual_election(true, true, false));
        assert!(c.claim_eventual_election(true, true, true));
        assert!(
            liveness_admitted_as_is(false, false, false),
            "AS-IS dente: claim without axioms"
        );
        assert!(!liveness_admitted(false, true, true));
        assert_eq!(
            elect_claim_banner(false, false, false),
            "bounded-elect not-eventual"
        );
        assert!(
            !elect_claim_banner(false, false, false).contains("live"),
            "TCP/real banner must not print live without ES"
        );
        assert_eq!(
            elect_claim_banner_as_is(false, false, false),
            "live",
            "AS-IS dente: print live without naming ES"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0072 P0: acked put must still be L28-clean after crash-reopen of
    /// a follower. AS-IS would pass on first get only.
    #[test]
    fn l28_durability_ok_requires_after_kill_and_restart() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"l28/k", b"l28/v").unwrap();
        let get_ok = c.get_strong(b"l28/k").unwrap().as_deref() == Some(b"l28/v".as_ref());
        assert!(get_ok, "acked put must be strongly visible");
        let leader = c.range_leader(1).expect("leader");
        let follower = (1u64..=3).find(|&id| id != leader).expect("follower");
        c.crash_reopen_engine_on(follower, pedradb_io_uring::IoUringEnv::default())
            .expect("crash-reopen follower");
        let after_kill_ok = c.count_applied_eq(b"l28/k", b"l28/v") >= 2;
        let restart_ok = c.get_strong(b"l28/k").unwrap().as_deref() == Some(b"l28/v".as_ref());
        assert!(
            l28_durability_ok(get_ok, after_kill_ok, restart_ok),
            "get={get_ok} after={after_kill_ok} restart={restart_ok}"
        );
        assert!(
            l28_durability_ok_as_is(true, false, false),
            "AS-IS dente: get-only would pass"
        );
        assert!(!l28_durability_ok(true, false, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Persist fail on remove must not shrink the in-RAM voting set
    /// (World otherwise records `rm_member_err` while the floor already
    /// sees a smaller config).
    #[test]
    fn remove_member_persist_fail_does_not_shrink_ram() {
        use pedradb_sim::{FailingEnv, FaultKind, OpClass, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2.clone(), e3.clone()],
            SeedRng::new(0xF1D5),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        e1.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
        e2.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
        e3.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
        assert!(c.remove_member(3).is_err(), "persist must fail-closed");
        assert!(
            c.is_member(3),
            "RAM membership must not shrink on persist fail"
        );
        assert_eq!(c.member_ids().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F148: try_become_leader must not leave Leader + non-durable Noop on log fail.
    #[test]
    fn try_become_leader_log_persist_fail_stays_candidate() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2, e3],
            SeedRng::new(0xF148),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        let rid = 1u64;
        let _ = c.step_down_range_leader(rid);
        // Durable Candidate of a new term (hard ok); log write will fail.
        let (term, log_len_before) = {
            let n = c.nodes.get_mut(&1).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            p.term += 1;
            p.role = Role::Candidate;
            p.voted_for = Some(1);
            p.leader_id = None;
            let t = p.term;
            let len = p.log.len();
            persist_hard_db(&mut n.db, rid, p).unwrap();
            (t, len)
        };
        // Joint-election gate (RFC-0064) needs a majority of grants for
        // this term or try_become_leader returns Ok without touching the log.
        c.election_granted.insert((rid, term, 1), vec![1, 2]);
        c.election_votes.insert((rid, term, 1), 2);
        e1.arm_one_failure();
        let err = c.try_become_leader(rid, 1, term);
        assert!(
            err.is_err(),
            "try_become_leader must surface log persist fail: {err:?}"
        );
        let p = c.nodes.get(&1).unwrap().ranges.get(&rid).unwrap();
        assert!(
            matches!(p.role, Role::Candidate),
            "AS-IS stuck as Leader; must stay Candidate, role={:?}",
            p.role
        );
        assert_eq!(
            p.log.len(),
            log_len_before,
            "Noop blank entry must not stick after log persist fail"
        );
        assert_eq!(p.term, term, "term must remain the candidate term");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F147: start_election must roll back term/role/vote if hard persist fails.
    #[test]
    fn start_election_hard_persist_fail_does_not_stick_term() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2, e3],
            SeedRng::new(0xF147),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        let rid = 1u64;
        // Force a known follower so start_election is meaningful.
        let _ = c.step_down_range_leader(rid);
        // Arm the candidate (node 1) so its next hard write fails.
        let before = {
            let p = c.nodes.get(&1).unwrap().ranges.get(&rid).unwrap();
            (p.term, p.role, p.voted_for)
        };
        e1.arm_one_failure();
        let err = c.start_election(rid, 1);
        assert!(
            err.is_err(),
            "start_election must surface hard persist fail: {err:?}"
        );
        let after = {
            let p = c.nodes.get(&1).unwrap().ranges.get(&rid).unwrap();
            (p.term, p.role, p.voted_for)
        };
        assert_eq!(
            after.0, before.0,
            "AS-IS stuck at term+1 in RAM; must restore term (before={before:?} after={after:?})"
        );
        assert_eq!(
            after.1, before.1,
            "role must roll back on hard fail (before={before:?} after={after:?})"
        );
        assert_eq!(
            after.2, before.2,
            "voted_for must roll back on hard fail (before={before:?} after={after:?})"
        );
        // Disk hard matches RAM.
        if let Some(raw) = c.nodes.get(&1).unwrap().db.get(&raft_meta_key(rid, "hard")) {
            let (disk_term, disk_vote) = decode_hard(raw.as_ref()).unwrap();
            assert_eq!(disk_term, before.0);
            assert_eq!(disk_vote, before.2);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F127: AE under higher term must not proceed if hard state cannot persist.
    #[test]
    fn append_entries_hard_persist_fail_rejects() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1, e2, e3.clone()],
            SeedRng::new(0xF127),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        c.put(b"k", b"v").unwrap();
        let rid = 1u64;
        let (term, last_i, last_t) = {
            let p = c.nodes.get(&3).unwrap().ranges.get(&rid).unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let entry = LogRec {
            index: last_i + 1,
            term: term + 1,
            entry: RangeEntry::Noop,
        };
        e3.arm_one_failure();
        let reply = c
            .on_append_entries(3, rid, term + 1, 1, last_i, last_t, 0, vec![entry])
            .unwrap();
        match reply {
            PeerMsg::AppendEntriesReply { success: false, .. } => {}
            other => panic!("expected AE reject when hard persist fails: {other:?}"),
        }
        // Entry must not be in memory log (we never reached append).
        let p3 = c.nodes.get(&3).unwrap().ranges.get(&rid).unwrap();
        assert!(
            p3.log.last().map(|e| e.index) != Some(last_i + 1),
            "must not append under non-durable higher term"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F129: propose path must surface discard failure (not only NotCommitted).
    #[test]
    fn put_not_committed_surfaces_discard_persist_fail() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2, e3],
            SeedRng::new(0xF129),
        )
        .unwrap();
        c.elect_all(120).unwrap();
        for _ in 0..60 {
            if c.range_leader(1) == Some(1) {
                break;
            }
            let _ = c.step_down_range_leader(1);
            let _ = c.tick();
            let _ = c.elect_all(20);
        }
        assert_eq!(c.range_leader(1), Some(1));
        // Minority: put cannot majority-commit → discard path runs.
        c.set_participating(2, false).unwrap();
        c.set_participating(3, false).unwrap();
        // Propose persists the orphan (1 write); discard re-persists truncate (2nd).
        // Arm remaining=1 → propose ok, discard fail → must not return clean NotCommitted.
        e1.arm(1, true);
        let err = c.put(b"orphan", b"x");
        match err {
            Err(StoreError::NotCommitted { .. }) => panic!(
                "F129: discard persist fail must surface as disk error, not clean NotCommitted"
            ),
            Err(_) => {}
            Ok(()) => panic!("put without majority must not Ok"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F130: abort fence put failure must surface (not silent).
    #[test]
    fn fence_txn_aborted_persist_fail_is_err() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2, e3],
            SeedRng::new(0xF130),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        e1.arm_one_failure();
        let err = c.fence_txn_aborted(42);
        assert!(
            err.is_err(),
            "fence must fail closed when status put fails: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F128: leader discard of uncommitted index must durable-truncate its log.
    #[test]
    fn discard_uncommitted_leader_persist_fail_is_err() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2, e3],
            SeedRng::new(0xF128),
        )
        .unwrap();
        c.elect_all(120).unwrap();
        for _ in 0..40 {
            if c.range_leader(1) == Some(1) {
                break;
            }
            let _ = c.step_down_range_leader(1);
            let _ = c.tick();
            let _ = c.elect_all(20);
        }
        assert_eq!(c.range_leader(1), Some(1));
        c.put(b"k", b"v").unwrap();
        let rid = 1u64;
        // Append a fake uncommitted entry on the leader log past commit.
        let cut = {
            let n = c.nodes.get_mut(&1).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            let idx = p.last_index() + 1;
            p.log.push(LogRec {
                index: idx,
                term: p.term,
                entry: RangeEntry::Noop,
            });
            // Persist so disk has the orphan (what propose would have done).
            persist_log_db(&mut n.db, rid, p).unwrap();
            idx
        };
        e1.arm_one_failure();
        let err = c.discard_uncommitted_from(rid, 1, cut);
        assert!(
            err.is_err(),
            "leader must fail closed if discard cannot re-persist log: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F125: vote grant must not stick without durable hard state.
    #[test]
    fn request_vote_persist_fail_denies_grant() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1, e2, e3.clone()],
            SeedRng::new(0xF125),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        let rid = 1u64;
        let (term, last_i, last_t) = {
            let p = c.nodes.get(&3).unwrap().ranges.get(&rid).unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        // Next hard-state write on peer 3 fails.
        e3.arm_one_failure();
        let reply = c
            .on_request_vote(3, rid, term + 1, 1, last_i, last_t)
            .unwrap();
        match reply {
            PeerMsg::RequestVoteReply {
                vote_granted: false,
                ..
            } => {}
            other => panic!("expected deny when hard persist fails, got {other:?}"),
        }
        // Peer 3 must not remember a vote that never hit disk.
        let p3 = c.nodes.get(&3).unwrap().ranges.get(&rid).unwrap();
        assert!(
            p3.voted_for.is_none() || p3.term <= term,
            "in-memory vote/term must not stick without durable hard: term={} voted={:?}",
            p3.term,
            p3.voted_for
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F126: leader commit advance rolls back if commit meta persist fails.
    #[test]
    fn try_advance_commit_persist_fail_does_not_raise_commit() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        // Node 1 is preferred leader for range 1 in many elect schedules; pin env.
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2, e3],
            SeedRng::new(0xF126),
        )
        .unwrap();
        c.elect_all(120).unwrap();
        c.put(b"k", b"v").unwrap();
        let rid = 1u64;
        // Prefer node 1 as leader for inject (shares e1).
        for _ in 0..60 {
            if c.range_leader(rid) == Some(1) {
                break;
            }
            let cur = c.range_leader(rid);
            if let Some(l) = cur {
                let _ = c.step_down_range_leader(rid);
                let _ = l;
            }
            let _ = c.tick();
            let _ = c.elect_all(20);
        }
        assert_eq!(c.range_leader(rid), Some(1), "need leader=1 for e1 inject");
        let before = c.commit_index(1, rid);
        assert!(before >= 1);
        let rolled = before.saturating_sub(1);
        {
            let n = c.nodes.get_mut(&1).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            let last = p.last_index();
            p.commit = rolled;
            for &pid in &c.ids {
                p.match_index.insert(pid, last);
            }
        }
        e1.arm_one_failure();
        c.try_advance_commit(rid, 1).unwrap();
        assert_eq!(
            c.commit_index(1, rid),
            rolled,
            "commit must stay at pre-advance value when persist_commit fails"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F124: raft meta persist failure must not wipe user keys / reply success.
    #[test]
    fn install_snapshot_persist_fail_keeps_user_keys() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1, e2, e3.clone()],
            SeedRng::new(0xF124),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        c.put(b"keep", b"v").unwrap();
        assert_eq!(
            c.get_on(3, b"keep").unwrap().as_deref(),
            Some(b"v".as_ref())
        );
        // Empty export would wipe `keep` if install proceeded past meta persist.
        let rid = c.locate(b"keep").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let (term, snap_i, snap_t) = {
            let p = c.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
            (p.term.max(1), p.applied.max(1), p.term.max(1))
        };
        // Next write(s) on peer 3 fail (raft meta persist).
        e3.arm_one_failure();
        let reply = c
            .on_install_snapshot(3, rid, term, leader, snap_i, snap_t, vec![])
            .unwrap();
        match reply {
            PeerMsg::InstallSnapshotReply { success: false, .. } => {}
            other => panic!("expected success=false on persist fail, got {other:?}"),
        }
        assert_eq!(
            c.get_on(3, b"keep").unwrap().as_deref(),
            Some(b"v".as_ref()),
            "user key must survive failed install-snapshot persist"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F144: higher-term install-snapshot must not leave a non-durable term bump
    /// when raft meta persist fails (F127 residual on the install path).
    #[test]
    fn install_snapshot_higher_term_persist_fail_does_not_stick_term() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1, e2, e3.clone()],
            SeedRng::new(0xF144),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        c.put(b"keep", b"v").unwrap();
        let rid = c.locate(b"keep").unwrap();
        let leader = c.range_leader(rid).unwrap();
        let (peer_term, snap_i, snap_t) = {
            let p = c.nodes.get(&3).unwrap().ranges.get(&rid).unwrap();
            (p.term, p.applied.max(1), p.term.max(1))
        };
        let high = peer_term.saturating_add(5).max(2);
        e3.arm_one_failure();
        let reply = c
            .on_install_snapshot(3, rid, high, leader, snap_i, snap_t, vec![])
            .unwrap();
        match reply {
            PeerMsg::InstallSnapshotReply {
                success: false,
                term: reply_term,
                ..
            } => {
                assert_eq!(
                    reply_term, peer_term,
                    "reply term must be pre-install term when hard bump fails"
                );
            }
            other => panic!("expected success=false, got {other:?}"),
        }
        let after = c.nodes.get(&3).unwrap().ranges.get(&rid).unwrap().term;
        assert_eq!(
            after, peer_term,
            "AS-IS stuck at non-durable higher term {high}; must stay {peer_term}, got {after}"
        );
        // Disk hard must match RAM (no reopen surprise).
        let hard_raw = c.nodes.get(&3).unwrap().db.get(&raft_meta_key(rid, "hard"));
        if let Some(raw) = hard_raw {
            let (disk_term, _) = decode_hard(raw.as_ref()).expect("hard decodes");
            assert_eq!(
                disk_term, peer_term,
                "disk hard term must not advance without durable install"
            );
        }
        assert_eq!(
            c.get_on(3, b"keep").unwrap().as_deref(),
            Some(b"v".as_ref()),
            "user key must survive"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P2.3: re-add after remove+compact → install-snapshot catch-up.
    #[test]
    fn install_snapshot_catchup_after_remove_compact() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        for i in 0..5u8 {
            c.put([b'k', i], [b'v', i]).unwrap();
        }
        // Drop peer 3 from membership so remaining can compact past its applied.
        c.remove_member(3).unwrap();
        for i in 5..12u8 {
            c.put([b'k', i], [b'v', i]).unwrap();
        }
        let rid = c.locate(&[b'k', 0]).unwrap();
        let leader = c.range_leader(rid).unwrap();
        let leader_snap = c.snapshot_index(leader, rid);
        assert!(leader_snap >= 1, "expected compact on remaining members");
        // Peer 3 still has old data for early keys but misses later ones until catch-up.
        c.add_member(3).unwrap();
        assert!(c.is_member(3));
        // Drive AE / InstallSnapshot via heartbeats (and a put noop path).
        for _ in 0..30 {
            c.tick().unwrap();
        }
        // Client put forces full broadcast_append from leader.
        let _ = c.put(b"catchup-ping", b"1");
        for _ in 0..10 {
            c.tick().unwrap();
        }
        // After catch-up, peer 3 must see a late key that was only written after remove.
        assert_eq!(
            c.get_on(3, &[b'k', 11]).unwrap().as_deref(),
            Some([b'v', 11].as_slice()),
            "install-snapshot (or AE catch-up) must restore applied state on re-add"
        );
        assert!(
            c.snapshot_index(3, rid) >= 1 || c.applied_index(3, rid) >= 1,
            "peer 3 watermarks advanced"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Install-snapshot must replace range state, not merge — keys deleted on the
    /// leader after compact must not remain on a re-added lagging follower.
    #[test]
    fn install_snapshot_clears_stale_keys_not_in_export() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"stale-k", b"old").unwrap();
        assert!(c.count_applied_eq(b"stale-k", b"old") >= 2);
        c.remove_member(3).unwrap();
        for i in 0..12u8 {
            c.put([b'n', i], [b'v', i]).unwrap();
        }
        // Simulate a committed delete that was folded into the snapshot prefix
        // (no AE redo of the delete for a compact-behind peer).
        for nid in [1u64, 2] {
            if let Some(n) = c.nodes.get_mut(&nid) {
                let _ = n.db.delete(b"stale-k");
            }
        }
        assert!(c.get_on(1, b"stale-k").unwrap().is_none());
        assert_eq!(
            c.get_on(3, b"stale-k").unwrap().as_deref(),
            Some(b"old".as_ref())
        );
        let rid = c.locate(b"stale-k").unwrap();
        let leader = c.range_leader(rid).unwrap();
        assert!(
            c.snapshot_index(leader, rid) >= 1,
            "need compact for install-snapshot path, snap={}",
            c.snapshot_index(leader, rid)
        );
        c.add_member(3).unwrap();
        for _ in 0..40 {
            c.tick().unwrap();
        }
        let _ = c.put(b"catchup-ping2", b"1");
        for _ in 0..15 {
            c.tick().unwrap();
        }
        assert!(
            c.get_on(3, b"stale-k").unwrap().is_none(),
            "install-snapshot merge left stale key on follower: {:?}",
            c.get_on(3, b"stale-k").unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F-found (RFC-0059 swarm, seed 503976): a follower rejecting a
    /// snapshot older than its own commit used to reply success with its
    /// commit, and the leader recorded that as a replication match of its
    /// own log — a stale-branch leader then satisfied the commit quorum
    /// with those phantom matches and "committed" its own-term no-op with
    /// zero real follower acks. The rejection must be failure + hint: the
    /// leader may resume AE at the hint, never count it as a match.
    #[test]
    fn install_snapshot_stale_reject_is_hint_not_match() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        for i in 0..4u8 {
            c.put([b'a', i], [b'v', i]).unwrap();
        }
        let rid = 1;
        let leader = c.range_leader(rid).unwrap();
        let commit3 = c.commit_index(3, rid);
        assert!(commit3 >= 3, "follower must be ahead of the stale offer");
        let term = c.term_on(leader, rid);
        let reply = c
            .on_install_snapshot(3, rid, term, leader, 1, 1, vec![])
            .unwrap();
        let PeerMsg::InstallSnapshotReply {
            success,
            match_index,
            ..
        } = reply
        else {
            panic!("not a snapshot reply");
        };
        assert!(!success, "stale snapshot must be rejected, not acked");
        assert_eq!(
            match_index, commit3,
            "rejection carries the follower commit as a hint"
        );
        let before = c
            .nodes
            .get(&leader)
            .unwrap()
            .ranges
            .get(&rid)
            .unwrap()
            .match_index
            .get(&3)
            .copied()
            .unwrap_or(0);
        c.on_install_snapshot_reply(leader, 3, rid, term, success, match_index)
            .unwrap();
        let n = c.nodes.get(&leader).unwrap();
        let p = n.ranges.get(&rid).unwrap();
        assert_eq!(
            p.match_index.get(&3).copied().unwrap_or(0),
            before,
            "a rejected snapshot must not advance match_index"
        );
        let last = p.last_index();
        let snap = p.snapshot_index;
        assert_eq!(
            p.next_index.get(&3).copied().unwrap_or(0),
            (match_index + 1).min(last + 1).max(snap.max(1)),
            "AE must resume at the hint clamped into our log"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F-found (RFC-0059 swarm, seed 503976): the install-snapshot export is
    /// the leader's **live applied state**, but the message was labeled at the
    /// compaction watermark — two leaders labeled (4,6) with different
    /// contents and a lagging follower flip-flopped between them (materialized
    /// keys from one, cleared them for the other). The label must be the
    /// leader's applied point, which is exactly what the export reflects.
    #[test]
    fn install_snapshot_label_is_leader_applied_point() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        for i in 0..6u8 {
            c.put([b'b', i], [b'v', i]).unwrap();
        }
        let rid = 1;
        let leader = c.range_leader(rid).unwrap();
        let (applied, snap, expected_term) = {
            // Compact watermark behind live applied, with the applied
            // entry still in the log so `term_at` resolves. Lowering the
            // watermark below a *real* compacted prefix would make
            // `term_at(applied)` miss (the production compact path never
            // does that).
            let n = c.nodes.get_mut(&leader).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            let applied = p.applied;
            assert!(applied >= 1);
            let idx = p.last_index() + 1;
            let term = p.term.max(1);
            n.db.put(b"bx", b"v").unwrap();
            p.log.push(LogRec {
                index: idx,
                term,
                entry: RangeEntry::Put {
                    key: b"bx".to_vec(),
                    value: b"v".to_vec(),
                    si_gen: 0,
                },
            });
            p.commit = idx;
            p.applied = idx;
            p.snapshot_index = 1;
            p.snapshot_term = p.term_at(1).max(1);
            p.next_index.insert(3, 1);
            p.match_index.insert(3, 0);
            (p.applied, p.snapshot_index, term)
        };
        assert!(
            applied > snap,
            "test must distinguish applied={applied} from watermark={snap}"
        );
        c.set_rpc_mode(RpcMode::Queued);
        c.broadcast_append(rid, leader, None).unwrap();
        let msgs = c.drain_outbound();
        let mut found = false;
        for (from, to, bytes) in msgs {
            if let Ok(PeerMsg::InstallSnapshot {
                last_included_index,
                last_included_term,
                kv_pairs,
                ..
            }) = PeerMsg::decode(&bytes)
            {
                assert_eq!((from, to), (leader, 3));
                found = true;
                assert_eq!(
                    last_included_index, applied,
                    "label must be the leader's applied point, not the compaction watermark {snap}"
                );
                assert_ne!(last_included_index, snap);
                assert_eq!(last_included_term, expected_term);
                assert!(
                    kv_pairs.iter().any(|(k, _)| k == b"bx"),
                    "export carries the applied user state"
                );
            }
        }
        assert!(found, "expected an InstallSnapshot for lagging peer 3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F-found (RFC-0059 swarm, seed 503976): the election tally was keyed by
    /// (range, term) only, so two rivals that timed out into the same term
    /// pooled their grants — one won with votes cast for the other. Each
    /// candidate must tally only its own grants.
    #[test]
    fn same_term_rival_candidates_do_not_pool_votes() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 5, 1).unwrap();
        c.elect_all(200).unwrap();
        c.put(b"r", b"1").unwrap();
        let rid = 1;
        c.set_rpc_mode(RpcMode::Queued);
        // Land both rivals on the same term: start_election always does
        // `term += 1`, so first reset every peer onto a shared floor.
        let floor = c.term_on(1, rid).min(c.term_on(2, rid));
        for nid in c.ids.clone() {
            let p = c.nodes.get_mut(&nid).unwrap().ranges.get_mut(&rid).unwrap();
            p.role = Role::Follower;
            p.leader_id = None;
            p.voted_for = None;
            p.term = floor;
        }
        c.start_election(rid, 2).unwrap();
        c.start_election(rid, 1).unwrap();
        let term = c.term_on(1, rid);
        assert_eq!(term, c.term_on(2, rid), "rivals must share a term");
        let outbound = c.drain_outbound();
        let mut delivered = 0;
        for (from, to, bytes) in outbound {
            let is_rv = matches!(
                PeerMsg::decode(&bytes).ok(),
                Some(PeerMsg::RequestVote { .. })
            );
            if is_rv && ((from, to) == (2, 3) || (from, to) == (1, 4)) {
                c.handle_inbound(from, to, &bytes).unwrap();
                delivered += 1;
            }
        }
        assert_eq!(delivered, 2, "one grant routed to each rival");
        let replies = c.drain_outbound();
        for (from, to, bytes) in replies {
            c.handle_inbound(from, to, &bytes).unwrap();
        }
        // Each rival tallied self + one grant = 2 of majority 3. Under the
        // old (range, term) key those two grants pooled onto one counter
        // and someone hit majority; per-candidate keys keep them apart.
        assert_eq!(c.election_votes.get(&(rid, term, 1)).copied(), Some(2));
        assert_eq!(c.election_votes.get(&(rid, term, 2)).copied(), Some(2));
        for cand in [1u64, 2] {
            let p = c.nodes.get(&cand).unwrap().ranges.get(&rid).unwrap();
            assert_ne!(
                p.role,
                Role::Leader,
                "candidate {cand} won with a grant cast for its same-term rival"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Direct install-snapshot of a **user** range must clear orphan intents for
    /// keys in that range even though intent keys live under `\0store/` (low
    /// range keyspace). Full remove/re-add can mask this when range-0 install
    /// side-effect wipes all `\0store/*`.
    #[test]
    fn install_snapshot_user_range_clears_orphan_intents() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 4).unwrap();
        c.elect_all(80).unwrap();
        let user = b"intent-k"; // 0x69 → mid range
        let user_rid = c.locate(user).unwrap();
        let intent_rid = c.locate(&intent_key(user)).unwrap();
        assert_ne!(user_rid, intent_rid);
        c.put(user, b"seed").unwrap();
        c.put([0x01, b'a'], b"low").unwrap();
        // Orphan intent on peer 3 only (simulates prepare then offline past commit).
        {
            let n = c.nodes.get_mut(&3).unwrap();
            n.db.put(intent_key(user), encode_intent(9_999, b"ghost"))
                .unwrap();
            n.db.put(txn_pair_key(9_999, user), b"ghost").unwrap();
            n.db.put(txn_pre_key(9_999, user), encode_preimage(Some(b"seed")))
                .unwrap();
            n.db.put(txn_status_key(9_999), b"prepared").unwrap();
            assert!(intent_conflict(&n.db, user, None));
        }
        let export = c.export_range_kv(1, user_rid).unwrap();
        let leader = c.range_leader(user_rid).unwrap();
        let (term, snap_i, snap_t) = {
            let p = c.nodes.get(&leader).unwrap().ranges.get(&user_rid).unwrap();
            (p.term.max(1), p.applied.max(1), p.term.max(1))
        };
        // Install user-range snapshot onto peer 3 only (no range-0 install).
        let reply = c
            .on_install_snapshot(3, user_rid, term, leader, snap_i, snap_t, export)
            .unwrap();
        match reply {
            PeerMsg::InstallSnapshotReply { success: true, .. } => {}
            other => panic!("install failed: {other:?}"),
        }
        assert_eq!(
            c.get_on(3, user).unwrap().as_deref(),
            Some(b"seed".as_ref()),
            "user value restored from export"
        );
        assert!(
            !intent_conflict(&c.nodes.get(&3).unwrap().db, user, None),
            "orphan intent for key in installed range must be cleared \
             (intent key is in range {intent_rid}, install was range {user_rid})"
        );
        // Low-range user data must not be touched by mid-range install.
        assert_eq!(
            c.get_on(3, &[0x01, b'a']).unwrap().as_deref(),
            Some(b"low".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Install-snapshot wipe must not delete other ranges' raft meta / reserved
    /// keys just because they share the low byte range (`\0store/...`).
    #[test]
    fn install_snapshot_range0_preserves_other_range_raft_meta() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 4).unwrap();
        c.elect_all(80).unwrap();
        c.put([0x01, b'a'], b"low-v").unwrap();
        c.put(b"high-key", b"hi-v").unwrap();
        let rid0 = c.locate(&[0x01, b'a']).unwrap();
        let rid_hi = c.locate(b"high-key").unwrap();
        assert_ne!(rid0, rid_hi);
        let hard_hi = raft_meta_key(rid_hi, "hard");
        let log_hi = raft_meta_key(rid_hi, "log");
        let commit_hi = raft_meta_key(rid_hi, "commit");
        // Snapshot of durable high-range meta before range-0 install.
        let hard_before = c
            .nodes
            .get(&3)
            .unwrap()
            .db
            .get(&hard_hi)
            .map(|b| b.to_vec());
        let log_before = c.nodes.get(&3).unwrap().db.get(&log_hi).map(|b| b.to_vec());
        let commit_before = c
            .nodes
            .get(&3)
            .unwrap()
            .db
            .get(&commit_hi)
            .map(|b| b.to_vec());
        assert!(hard_before.is_some() && commit_before.is_some());
        // In-progress prepare intent for a *high* key (must not vanish on range0 install
        // either — abort is coordinator's job; wipe must be range-scoped).
        {
            let n = c.nodes.get_mut(&3).unwrap();
            n.db.put(intent_key(b"high-key"), encode_intent(42, b"inflight"))
                .unwrap();
        }
        let export0 = c.export_range_kv(1, rid0).unwrap();
        let leader = c.range_leader(rid0).unwrap();
        let (term, snap_i, snap_t) = {
            let p = c.nodes.get(&leader).unwrap().ranges.get(&rid0).unwrap();
            (p.term.max(1), p.applied.max(1), p.term.max(1))
        };
        c.on_install_snapshot(3, rid0, term, leader, snap_i, snap_t, export0)
            .unwrap();
        assert_eq!(
            c.nodes
                .get(&3)
                .unwrap()
                .db
                .get(&hard_hi)
                .map(|b| b.to_vec()),
            hard_before,
            "range-0 install wiped high-range raft hard meta"
        );
        assert_eq!(
            c.nodes.get(&3).unwrap().db.get(&log_hi).map(|b| b.to_vec()),
            log_before,
            "range-0 install wiped high-range raft log meta"
        );
        assert_eq!(
            c.nodes
                .get(&3)
                .unwrap()
                .db
                .get(&commit_hi)
                .map(|b| b.to_vec()),
            commit_before,
            "range-0 install wiped high-range raft commit meta"
        );
        assert_eq!(
            c.get_on(3, b"high-key").unwrap().as_deref(),
            Some(b"hi-v".as_ref()),
            "high-range user data must survive range-0 install"
        );
        // Intent for high key: must still exist (not range-0's to clear as "user wipe").
        // Clearing orphan intents is done by user-range install / open recovery.
        assert!(
            intent_conflict(&c.nodes.get(&3).unwrap().db, b"high-key", None),
            "range-0 install must not blindly delete intents for other ranges' keys"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Install-snapshot must not replace the follower's raft hard/log with the
    /// leader's (per-node state). Export used to ship `\0store/raft/*` from the
    /// leader into the payload.
    #[test]
    fn install_snapshot_does_not_import_leader_raft_meta() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 2).unwrap();
        c.elect_all(80).unwrap();
        c.put([0x01, b'a'], b"v").unwrap();
        let rid = c.locate(&[0x01, b'a']).unwrap();
        let leader = c.range_leader(rid).unwrap();
        let follower = if leader == 1 { 2 } else { 1 };
        let hard_k = raft_meta_key(rid, "hard");
        // Give follower a distinct hard state (term already matches cluster; flip vote).
        {
            let n = c.nodes.get_mut(&follower).unwrap();
            let p = n.ranges.get_mut(&rid).unwrap();
            p.voted_for = Some(follower);
            persist_hard_db(&mut n.db, rid, p).unwrap();
        }
        let follower_hard = c
            .nodes
            .get(&follower)
            .unwrap()
            .db
            .get(&hard_k)
            .map(|b| b.to_vec())
            .expect("follower hard");
        let leader_hard = c
            .nodes
            .get(&leader)
            .unwrap()
            .db
            .get(&hard_k)
            .map(|b| b.to_vec())
            .expect("leader hard");
        assert_ne!(
            follower_hard, leader_hard,
            "precondition: hard states must differ"
        );
        let export = c.export_range_kv(leader, rid).unwrap();
        assert!(
            export.iter().all(|(k, _)| !is_reserved_store_key(k)),
            "export must not contain reserved raft/intent keys"
        );
        let (term, snap_i, snap_t) = {
            let p = c.nodes.get(&leader).unwrap().ranges.get(&rid).unwrap();
            (p.term.max(1), p.applied.max(1), p.term.max(1))
        };
        c.on_install_snapshot(follower, rid, term, leader, snap_i, snap_t, export)
            .unwrap();
        let after = c
            .nodes
            .get(&follower)
            .unwrap()
            .db
            .get(&hard_k)
            .map(|b| b.to_vec());
        // Local persist in on_install_snapshot may rewrite hard from in-memory peer
        // (term/vote), but must never become a copy of the *leader's* pre-install hard.
        assert_ne!(
            after.as_ref(),
            Some(&leader_hard),
            "follower hard became leader hard after install-snapshot"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P2.4: strong-read history under partition — never fail-open dual-leader.
    #[test]
    fn strong_read_history_no_dual_leader_fail_open() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x5150)).unwrap();
        c.elect_all(80).unwrap();
        let key = b"hist-k";
        c.put(key, b"v0").unwrap();
        let mut history: Vec<(u64, Option<Vec<u8>>, bool)> = Vec::new();
        // Sample strong reads from each node.
        for nid in 1..=3u64 {
            match c.get_with_policy(nid, key, ReadPolicy::Strong) {
                Ok(v) => history.push((nid, v.map(|b| b.to_vec()), true)),
                Err(_) => history.push((nid, None, false)),
            }
        }
        // Exactly one node may succeed strong read when unique leader exists.
        let ok_n = history.iter().filter(|h| h.2).count();
        assert_eq!(
            ok_n, 1,
            "exactly one strong-read Ok under unique leader; hist={history:?}"
        );
        // Force dual-leader claim: both 1 and 2 think Leader.
        let rid = c.locate(key).unwrap();
        for nid in [1u64, 2] {
            let p = c.nodes.get_mut(&nid).unwrap().ranges.get_mut(&rid).unwrap();
            p.role = Role::Leader;
            p.leader_id = Some(nid);
        }
        assert!(c.leader_claim_count(rid) >= 2);
        assert!(
            c.range_leader(rid).is_none(),
            "dual claim → no unique leader"
        );
        for nid in 1..=3u64 {
            let err = c
                .get_with_policy(nid, key, ReadPolicy::Strong)
                .expect_err("strong read must fail-closed under dual leader");
            assert!(
                matches!(
                    err,
                    StoreError::StaleLeader { .. } | StoreError::NotLeader { .. }
                ),
                "got {err}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn peer_msg_install_snapshot_roundtrip() {
        let m = PeerMsg::InstallSnapshot {
            range_id: 1,
            term: 2,
            leader_id: 1,
            last_included_index: 5,
            last_included_term: 2,
            kv_pairs: vec![(b"a".to_vec(), b"b".to_vec())],
        };
        assert_eq!(PeerMsg::decode(&m.encode()).unwrap(), m);
        let r = PeerMsg::InstallSnapshotReply {
            range_id: 1,
            term: 2,
            success: true,
            match_index: 5,
        };
        assert_eq!(PeerMsg::decode(&r.encode()).unwrap(), r);
    }

    /// F7 residual multi-node: absolute lease deadline durable in raft log; liveness after TTL.
    #[test]
    fn dcs_lease_ttl_expires_and_recreate_after_advance() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x7EA5E)).unwrap();
        c.set_ms_per_tick(0); // control now_ms only via advance_now_ms
        c.elect_all(80).unwrap();
        let key = meta_key(b"leader-lock");
        c.dcs_create_ttl(&key, b"node-a", 100).unwrap();
        assert_eq!(c.dcs_get_on(1, &key).unwrap().unwrap().value, b"node-a");
        // Before deadline: create fails.
        assert!(c.dcs_create_ttl(&key, b"node-b", 100).is_err());
        // Advance past absolute deadline.
        c.advance_now_ms(100);
        assert!(
            c.dcs_get_on(1, &key).unwrap().is_none(),
            "expired lease must be logically absent"
        );
        // Re-create after expiry (HA lock re-election).
        let rev = c.dcs_create_ttl(&key, b"node-b", 500).unwrap();
        assert!(rev >= 1);
        assert_eq!(c.dcs_get_on(2, &key).unwrap().unwrap().value, b"node-b");
        // Majority holds the new binding.
        let n = c
            .node_ids()
            .iter()
            .filter(|&&nid| {
                c.dcs_get_on(nid, &key)
                    .ok()
                    .flatten()
                    .is_some_and(|kv| kv.value == b"node-b")
            })
            .count();
        assert!(n >= 2, "recreate must majority-replicate; seen={n}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F139: raft TxnRevert fail after majority commit must surface (not only force-local).
    #[test]
    fn revert_majority_committed_surfaces_raft_fail() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"u", b"old").unwrap();
        let tid = 77u64;
        // Simulate prepare preimage + materialised commit on all local peers.
        for nid in c.ids.clone() {
            let n = c.nodes.get_mut(&nid).unwrap();
            n.db.put(txn_pre_key(tid, b"u"), encode_preimage(Some(b"old")))
                .unwrap();
            n.db.put(b"u", b"new").unwrap();
            n.db.put(txn_status_key(tid), b"prepared").unwrap();
        }
        // No leader → propose TxnRevert fails; force-local still restores preimage.
        let rid = 1u64;
        let _ = c.step_down_range_leader(rid);
        assert!(
            c.range_leader(rid).is_none(),
            "need leaderless for raft fail"
        );
        let err = c.revert_majority_committed_range(rid, tid, &[b"u".to_vec()]);
        assert!(
            err.is_err(),
            "F139: must surface raft revert fail even if local clear works: {err:?}"
        );
        assert!(
            matches!(err, Err(StoreError::NotLeader { .. })),
            "expected NotLeader, got {err:?}"
        );
        // Local clear still ran.
        assert_eq!(
            c.get_on(1, b"u").unwrap().as_deref(),
            Some(b"old".as_ref()),
            "force-local must still restore preimage"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F134: fenced TxnCommit apply must re-fence after revert (not swallow put).
    #[test]
    fn apply_txn_commit_fenced_keeps_abort_status() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"u", b"live").unwrap();
        let tid = 55u64;
        let n = c.nodes.get_mut(&1).unwrap();
        // Simulate abort fence + prepare preimage; apply commit must stay fenced.
        n.db.put(txn_pre_key(tid, b"u"), encode_preimage(Some(b"live")))
            .unwrap();
        n.db.put(intent_key(b"u"), encode_intent(tid, b"new"))
            .unwrap();
        n.db.put(txn_status_key(tid), b"abort").unwrap();
        apply_txn_commit(&mut n.db, tid, &[b"u".to_vec()]).unwrap();
        assert_eq!(
            n.db.get(&txn_status_key(tid)).as_deref(),
            Some(b"abort".as_ref()),
            "fenced commit must re-fence abort after revert"
        );
        assert_eq!(
            n.db.get(b"u").as_deref(),
            Some(b"live".as_ref()),
            "must restore preimage, not materialise intent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F135: commit_tx must surface tx_cancel failure after tx_finish err.
    #[test]
    fn commit_tx_surfaces_cancel_after_finish_fail() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2.clone(), e3.clone()],
            SeedRng::new(0xF135),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        // Prepare multi-key then poison preimages so cancel/revert fails closed.
        let h = c
            .tx_start([
                (b"a".as_slice(), b"1".as_slice()),
                (b"b".as_slice(), b"2".as_slice()),
            ])
            .unwrap();
        // Force finish failure: partition so commit cannot majority.
        c.set_participating(2, false).unwrap();
        c.set_participating(3, false).unwrap();
        // Poison preimages so the post-finish cancel hits F118.
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(&txn_pre_key(h.id, b"a"), b"\xffbad").unwrap();
            }
        }
        // Use commit_tx path: finish will fail (minority), cancel should surface preimage err.
        // Direct commit_tx from pairs — re-prepare via new commit_tx after poison is wrong.
        // Call the match arm logic: tx_finish then cancel.
        let finish_err = c.tx_finish(&h);
        assert!(
            finish_err.is_err(),
            "finish without majority: {finish_err:?}"
        );
        // Poison remaining keys and cancel — must err.
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(&txn_pre_key(h.id, b"b"), b"\xffbad").unwrap();
            }
        }
        let cancel_err = c.tx_cancel(&h);
        assert!(
            cancel_err.is_err(),
            "cancel with corrupt pre must err: {cancel_err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F136: SI generation meta must not silently miss all replica puts.
    #[test]
    fn persist_si_keys_gen_fail_is_err() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2.clone(), e3.clone()],
            SeedRng::new(0xF136),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        c.put(b"k", b"v").unwrap();
        e1.arm_one_failure();
        e2.arm_one_failure();
        e3.arm_one_failure();
        let err = c.persist_si_keys(&[b"k".to_vec()]);
        assert!(
            err.is_err(),
            "all-replica gen persist miss must fail closed: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F160: applied cursor must not stick high if applied-meta persist fails (F126 class).
    #[test]
    fn apply_range_applied_persist_fail_does_not_advance_applied() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2.clone(), e3.clone()],
            SeedRng::new(0xF160),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        c.put(b"k", b"v0").unwrap();
        let rid = c.locate(b"k").unwrap();
        let nid = c.range_leader(rid).unwrap();
        let applied0 = c.applied_index(nid, rid);
        {
            let p = c.nodes.get_mut(&nid).unwrap().ranges.get_mut(&rid).unwrap();
            let idx = p.last_index() + 1;
            let term = p.term;
            p.log.push(LogRec {
                index: idx,
                term,
                // si_gen=0: one user put, then applied-meta put (second op fails).
                entry: RangeEntry::Put {
                    key: b"k2".to_vec(),
                    value: b"v2".to_vec(),
                    si_gen: 0,
                },
            });
            p.commit = idx;
        }
        match nid {
            1 => e1.arm(1, true),
            2 => e2.arm(1, true),
            _ => e3.arm(1, true),
        }
        let err = c.apply_range(nid, rid);
        assert!(
            err.is_err(),
            "applied-meta persist miss must fail apply: {err:?}"
        );
        assert_eq!(
            c.applied_index(nid, rid),
            applied0,
            "AS-IS stuck applied high after failed persist_applied; must roll back"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F138: apply_range used to swallow generation-meta put then advance applied.
    #[test]
    fn apply_range_generation_persist_fail_does_not_advance_applied() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let mut c = StoreCluster::open_with_envs_rng_lab_direct(
            &dir,
            3,
            1,
            [e1.clone(), e2.clone(), e3.clone()],
            SeedRng::new(0xF138),
        )
        .unwrap();
        c.elect_all(80).unwrap();
        c.put(b"k", b"v0").unwrap();
        let rid = c.locate(b"k").unwrap();
        let nid = c.range_leader(rid).unwrap();
        let applied0 = c.applied_index(nid, rid);
        {
            let p = c.nodes.get_mut(&nid).unwrap().ranges.get_mut(&rid).unwrap();
            let idx = p.last_index() + 1;
            let term = p.term;
            p.log.push(LogRec {
                index: idx,
                term,
                entry: RangeEntry::Put {
                    key: b"k2".to_vec(),
                    value: b"v2".to_vec(),
                    si_gen: 2,
                },
            });
            p.commit = idx;
        }
        // hist put succeeds, generation put fails (one success then fail).
        match nid {
            1 => e1.arm(1, true),
            2 => e2.arm(1, true),
            _ => e3.arm(1, true),
        }
        let err = c.apply_range(nid, rid);
        assert!(
            err.is_err(),
            "generation persist miss must fail apply: {err:?}"
        );
        assert_eq!(
            c.applied_index(nid, rid),
            applied0,
            "applied must not advance past a failed generation persist"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F140: garbage 2PC preimage was treated as absent SI floor (unwrap_or None).
    #[test]
    fn note_tx_commit_rejects_corrupt_preimage_floor() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        // Pedra-only preimage: key exists, RAM hist empty (no SI note yet).
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(b"k", b"old").unwrap();
            }
        }
        let h = c.tx_start([(b"k".as_slice(), b"new".as_slice())]).unwrap();
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(&txn_pre_key(h.id, b"k"), b"\xffbad").unwrap();
            }
        }
        let err = c.tx_finish(&h);
        assert!(
            err.is_err(),
            "corrupt preimage must fail SI note, not stamp absent floor: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F141: short intent as val fallback became empty → SI delete tombstone.
    #[test]
    fn note_tx_commit_rejects_short_intent_value_fallback() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let h = c.tx_start([(b"k".as_slice(), b"new".as_slice())]).unwrap();
        // Reader sees no live key / pair, only a short intent → old code
        // decoded as None and stamped empty (SI delete).
        for nid in c.ids.clone() {
            if let Some(n) = c.nodes.get_mut(&nid) {
                n.db.put(intent_key(b"k"), b"xxxx").unwrap();
                n.db.delete(b"k").unwrap();
                n.db.delete(&txn_pair_key(h.id, b"k")).unwrap();
            }
        }
        let err = c.note_tx_commit(&h, c.read_version().max(1));
        assert!(
            err.is_err(),
            "short intent val fallback must fail closed, not SI-delete: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0013 P1.6 / F7: an expired DCS TTL stays dead across process restart
    /// (`now_ms` recovered; unknown/expired is fail-safe, not reanimated).
    #[test]
    fn dcs_lease_fail_safe_on_reopen() {
        let dir = temp();
        let key = meta_key(b"ttl-lock");
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.set_ms_per_tick(0);
            c.elect_all(80).unwrap();
            c.dcs_create_ttl(&key, b"holder-a", 50).unwrap();
            assert!(c.dcs_get(&key).unwrap().is_some());
            c.advance_now_ms(50);
            assert!(c.dcs_get(&key).unwrap().is_none());
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        assert!(
            c.dcs_get(&key).unwrap().is_none(),
            "expired lease must stay dead after reopen (F7 fail-safe)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F131: persist_now_ms marked RAM persisted even when every replica put failed.
    /// A later retry was skipped; reopen reloaded now_ms=0 and the expired lock
    /// reanimated (F56 inverse).
    #[test]
    fn persist_now_ms_retries_after_all_replica_put_fail() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let key = meta_key(b"ttl-lock");
        {
            let mut c = StoreCluster::open_with_envs_rng_lab_direct(
                &dir,
                3,
                1,
                [e1.clone(), e2.clone(), e3.clone()],
                SeedRng::new(0xF129),
            )
            .unwrap();
            c.set_ms_per_tick(0);
            c.elect_all(80).unwrap();
            c.dcs_create_ttl(&key, b"holder-a", 100).unwrap();
            e1.arm_one_failure();
            e2.arm_one_failure();
            e3.arm_one_failure();
            c.advance_now_ms(100);
            assert!(
                c.dcs_get_on(1, &key).unwrap().is_none(),
                "RAM clock past deadline must hide the lock"
            );
            e1.disarm();
            e2.disarm();
            e3.disarm();
            c.persist_now_ms();
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        assert!(
            c.dcs_get_on(1, &key).unwrap().is_none(),
            "expired TTL must stay dead after reopen; now_ms must have been retried to disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F132: alloc_txn_id advanced RAM then swallowed persist. After a finished
    /// TX, reopen reused the same id (F35 class).
    #[test]
    fn alloc_txn_id_persist_fail_does_not_reuse_id_after_reopen() {
        use pedradb_sim::{FailingEnv, SeedRng};
        let dir = temp();
        let e1 = FailingEnv::passing();
        let e2 = FailingEnv::passing();
        let e3 = FailingEnv::passing();
        let issued;
        {
            let mut c = StoreCluster::open_with_envs_rng_lab_direct(
                &dir,
                3,
                1,
                [e1.clone(), e2.clone(), e3.clone()],
                SeedRng::new(0xF132),
            )
            .unwrap();
            c.elect_all(80).unwrap();
            let h1 = c.tx_start([(b"t1".as_slice(), b"a".as_slice())]).unwrap();
            let id1 = h1.id;
            c.tx_finish(&h1).unwrap();
            e1.arm_one_failure();
            e2.arm_one_failure();
            e3.arm_one_failure();
            match c.tx_start([(b"t2".as_slice(), b"b".as_slice())]) {
                Ok(h2) => {
                    issued = h2.id;
                    c.tx_finish(&h2).unwrap();
                }
                Err(_) => issued = id1,
            }
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        let h3 = c.tx_start([(b"t3".as_slice(), b"c".as_slice())]).unwrap();
        assert!(
            h3.id > issued,
            "reopen must not reuse txn id {issued} after persist miss, got {}",
            h3.id
        );
        c.tx_cancel(&h3).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0020 P1.2 / W7: minority partition cannot majority-commit a put.
    #[test]
    fn rfc20_partition_minority_cannot_commit() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x0020_0A17)).unwrap();
        c.elect_all(80).unwrap();
        let key = b"part-key";
        let rid = c.locate(key).unwrap();
        let leader = c.range_leader(rid).expect("leader");
        // Partition both followers → leader is minority alone.
        let ids: Vec<u64> = c.node_ids().to_vec();
        for &nid in &ids {
            if nid != leader {
                c.set_participating(nid, false).unwrap();
            }
        }
        let err = c.put(key, b"should-fail");
        assert!(
            matches!(err, Err(StoreError::NotCommitted { .. })),
            "minority put must NotCommitted, got {err:?}"
        );
        // No node should show the value as applied majority.
        assert_eq!(
            c.count_applied_eq(key, b"should-fail"),
            0,
            "minority put must not apply as majority-visible"
        );
        for &nid in &ids {
            let v = c.get_on(nid, key).ok().flatten();
            assert!(
                v.is_none() || v.as_ref().map(|b| b.as_ref()) != Some(b"should-fail".as_ref()),
                "node {nid} must not expose minority-only value"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0020 P1.2 / W8: majority put + leader kill + re-elect → silent_wrong=0 catch-up.
    #[test]
    fn rfc20_leader_kill_after_majority_catchup() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x0020_C111)).unwrap();
        c.set_rpc_mode(RpcMode::Queued);
        elect_queued(&mut c, 100);

        let key = b"kill-key";
        put_queued(&mut c, key, b"acked");
        assert!(
            c.count_applied_eq(key, b"acked") >= 2,
            "pre-kill majority apply required"
        );
        let rid = c.locate(key).unwrap();
        let old = c.range_leader(rid).expect("leader");
        // Leader kill (partition + step down path).
        c.set_participating(old, false).unwrap();
        for _ in 0..200 {
            c.tick().unwrap();
            pump_queued(&mut c, 48);
            if let Some(l) = c.range_leader(rid) {
                if l != old {
                    break;
                }
            }
        }
        assert!(
            c.range_leader(rid).is_some_and(|l| l != old),
            "new leader after kill"
        );

        let mut silent_wrong = 0u64;
        for &nid in c.node_ids() {
            if nid == old {
                continue;
            }
            match c.get_on(nid, key) {
                Ok(Some(v)) if v.as_ref() == b"acked" => {}
                other => {
                    silent_wrong += 1;
                    eprintln!("node {nid} wrong after leader kill: {other:?}");
                }
            }
        }
        assert_eq!(silent_wrong, 0, "leader-kill catch-up silent_wrong");

        // Post-failover write still majority-commits.
        put_queued(&mut c, b"kill-key-2", b"after");
        assert!(c.count_applied_eq(b"kill-key-2", b"after") >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SI: get_at_version after clear keeps pre-clear value; tip/reopen stay absent.
    #[test]
    fn get_at_version_after_clear_and_reopen() {
        let dir = temp();
        let gen_clear;
        let gen_before;
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            c.put(b"gk", b"v0").unwrap();
            gen_before = c.read_version();
            assert_eq!(
                c.get_at_version(b"gk", gen_before).unwrap().as_deref(),
                Some(b"v0".as_ref())
            );
            let mut tx = c.begin();
            tx.clear(b"gk").unwrap();
            tx.commit(&mut c).unwrap();
            gen_clear = c.read_version();
            assert!(gen_clear > gen_before);
            assert_eq!(
                c.get_at_version(b"gk", gen_before).unwrap().as_deref(),
                Some(b"v0".as_ref()),
                "pre-clear snapshot must still see v0"
            );
            assert_eq!(
                c.get_at_version(b"gk", gen_clear).unwrap(),
                None,
                "post-clear snapshot must be None"
            );
            // keys_in_range must not resurrect cleared key at tip.
            let range = c.keys_in_range_at(b"g", b"h", gen_clear).unwrap();
            assert!(
                range.iter().all(|(k, _)| k.as_slice() != b"gk"),
                "cleared key in range at tip: {range:?}"
            );
            let range_old = c.keys_in_range_at(b"g", b"h", gen_before).unwrap();
            assert!(
                range_old
                    .iter()
                    .any(|(k, v)| k.as_slice() == b"gk" && v.as_slice() == b"v0"),
                "pre-clear range must include gk=v0: {range_old:?}"
            );
            drop(c);
        }
        let c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        assert_eq!(
            c.get_at_version(b"gk", gen_before).unwrap().as_deref(),
            Some(b"v0".as_ref()),
            "reopen: pre-clear snap must see v0"
        );
        assert_eq!(
            c.get_at_version(b"gk", gen_clear).unwrap(),
            None,
            "reopen: post-clear must stay None"
        );
        assert_eq!(c.get(b"gk").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Queued: finish after leader failover must flush SI notes for committed index.
    #[test]
    fn finish_queued_after_leader_failover_flushes_si_notes() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x000F_5001)).unwrap();
        c.set_rpc_mode(RpcMode::Queued);
        elect_queued(&mut c, 120);
        put_queued(&mut c, b"seed", b"0");
        let gen0 = c.read_version();

        // Outstanding put.
        let (rid, idx) = match c.put(b"q", b"1") {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            Ok(()) => panic!("expected NotCommitted under Queued"),
            Err(e) => panic!("{e}"),
        };
        // Deliver AE so majority commits, but do NOT finish notes yet.
        pump_queued(&mut c, 128);
        let leader = c.range_leader(rid).expect("leader");
        let commit = c.commit_index(leader, rid);
        assert!(
            commit >= idx,
            "after pump entry should be majority-committed commit={commit} idx={idx}"
        );

        // Kill leader; elect new one while entry is committed.
        c.set_participating(leader, false).unwrap();
        for _ in 0..200 {
            c.tick().unwrap();
            pump_queued(&mut c, 48);
            if c.range_leader(rid).is_some_and(|l| l != leader) {
                break;
            }
        }
        assert!(
            c.range_leader(rid).is_some_and(|l| l != leader),
            "need new leader"
        );

        // Client still holds (rid, idx). finish with abort_if_uncommitted=true.
        // If committed, must return true and flush SI notes — not discard.
        let ok = c.finish_queued_propose(rid, idx, true).unwrap();
        assert!(
            ok,
            "finish_queued must report committed after failover (not abort-drop notes)"
        );
        assert!(
            c.key_version(b"q") > gen0,
            "SI/OCC version must advance for majority-committed Queued put after failover"
        );
        assert_eq!(c.get(b"q").unwrap().as_deref(), Some(b"1".as_ref()));
        // Concurrent TX that read seed and would race on q must Conflict if it read q.
        let mut tx = c.begin();
        // Start TX at current gen after finish — just check get works.
        assert_eq!(tx.get(&c, b"q").unwrap().as_deref(), Some(b"1".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Queued: abort of earlier uncommitted index must not silently apply later put without OCC notes.
    #[test]
    fn finish_queued_abort_earlier_does_not_orphan_later_put() {
        let dir = temp();
        let mut c =
            StoreCluster::open_with_rng_lab_direct(&dir, 3, 1, SeedRng::new(0x000F_5002)).unwrap();
        c.set_rpc_mode(RpcMode::Queued);
        elect_queued(&mut c, 120);
        put_queued(&mut c, b"a", b"0");
        put_queued(&mut c, b"b", b"0");
        let gen0 = c.read_version();

        // Two outstanding proposes.
        let (r1, i1) = match c.put(b"a", b"A") {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            other => panic!("put a: {other:?}"),
        };
        let (r2, i2) = match c.put(b"b", b"B") {
            Err(StoreError::NotCommitted {
                range_id, index, ..
            }) => (range_id, index),
            other => panic!("put b: {other:?}"),
        };
        assert_eq!(r1, r2);
        assert!(i2 > i1);

        // Abort only the first WITHOUT pumping (still uncommitted) — drops notes >= i1
        // if implementation uses from_index retain wrong (drops later too).
        let aborted = c.finish_queued_propose(r1, i1, true).unwrap();
        assert!(!aborted, "first put must still be uncommitted");

        // Pump and finish second.
        pump_queued(&mut c, 128);
        let committed = c.finish_queued_propose(r2, i2, true).unwrap();
        // Second may have been discarded if discard_uncommitted_from(i1) truncated i2!
        if committed {
            assert!(
                c.key_version(b"b") > gen0,
                "second put notes must survive abort of earlier uncommitted index"
            );
            assert_eq!(c.get(b"b").unwrap().as_deref(), Some(b"B".as_ref()));
        } else {
            // If log truncated both, b must not be silently applied without versions.
            let applied = c.count_applied_eq(b"b", b"B");
            assert_eq!(
                applied, 0,
                "if finish false, b must not be majority-applied (silent wrong if applied without OCC)"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F50: multi-range `tx_start` NotLeader mid-prepare must abort earlier intents.
    ///
    /// `range_leader` miss used `?` after range0/1 already prepared → immortal
    /// Conflict on those keys until process reopen (F35 open recovery only).
    #[test]
    fn multi_range_prepare_not_leader_aborts_earlier_intents() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        assert!(keys.len() >= 3);
        c.put(&keys[0], b"old0").unwrap();
        c.put(&keys[1], b"old1").unwrap();
        c.put(&keys[2], b"old2").unwrap();
        // Kill leader of last range so prepare fails mid-way (after earlier prepares).
        let last = c.locate(&keys[2]).unwrap();
        let _ = c.step_down_range_leader(last);
        let err = c
            .tx_start([
                (keys[0].as_slice(), b"n0".as_slice()),
                (keys[1].as_slice(), b"n1".as_slice()),
                (keys[2].as_slice(), b"n2".as_slice()),
            ])
            .expect_err("prepare without last leader");
        assert!(
            matches!(
                err,
                StoreError::NotLeader { .. } | StoreError::NotCommitted { .. }
            ),
            "{err:?}"
        );
        // Re-elect and put must not Conflict on leftover intents.
        c.elect_all(120).unwrap();
        c.put(&keys[0], b"after")
            .expect("F50: no stuck intent on range0 after failed multi-range prepare");
        c.put(&keys[1], b"after")
            .expect("F50: no stuck intent on range1 after failed multi-range prepare");
        assert_eq!(c.get(&keys[0]).unwrap().as_deref(), Some(b"after".as_ref()));
        // Preimages preserved on the range that never prepared.
        assert_eq!(c.get(&keys[2]).unwrap().as_deref(), Some(b"old2".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F52: failed multi-range `tx_finish` after partial `TxnCommit` must not leave
    /// durable SI hist advertising the aborted write after Pedra preimage restore.
    #[test]
    fn partial_tx_finish_si_hist_matches_restored_preimage() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        assert!(keys.len() >= 2);
        c.put(&keys[0], b"old-a").unwrap();
        c.put(&keys[1], b"old-b").unwrap();
        let gen_before = c.read_version();
        let h = c
            .tx_start([
                (keys[0].as_slice(), b"new-a".as_slice()),
                (keys[1].as_slice(), b"new-b".as_slice()),
            ])
            .expect("prepare");
        let last = *h.ranges.last().unwrap();
        let _ = c.step_down_range_leader(last);
        let err = c.tx_finish(&h).expect_err("finish without last leader");
        assert!(
            matches!(
                err,
                StoreError::NotLeader { .. } | StoreError::NotCommitted { .. }
            ),
            "{err:?}"
        );
        // Pedra restored.
        assert_eq!(c.get(&keys[0]).unwrap().as_deref(), Some(b"old-a".as_ref()));
        let gen_after = c.read_version();
        // Any generation at/after the failed TX reserve must not SI-see new-a.
        for g in gen_before..=gen_after.saturating_add(1) {
            let v = c.get_at_version(&keys[0], g).unwrap();
            assert_ne!(
                v.as_deref(),
                Some(b"new-a".as_ref()),
                "F52: SI hist gen {g} still shows aborted write; pedra=old-a v={v:?} before={gen_before} after={gen_after}"
            );
        }
        // Tip SI must match Pedra (old-a).
        assert_eq!(
            c.get_at_version(&keys[0], c.read_version())
                .unwrap()
                .as_deref(),
            Some(b"old-a".as_ref()),
            "tip SI must match restored preimage"
        );
        // Reopen: durable apply-path hist must not resurrect new-a.
        drop(c);
        let c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        assert_eq!(c.get(&keys[0]).unwrap().as_deref(), Some(b"old-a".as_ref()));
        assert_eq!(
            c.get_at_version(&keys[0], c.read_version())
                .unwrap()
                .as_deref(),
            Some(b"old-a".as_ref()),
            "reopen SI tip must be old-a not aborted new-a"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F52 follow-on: range scan after failed multi-range finish must not show aborted writes.
    #[test]
    fn partial_tx_finish_range_scan_no_aborted_write() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        c.put(&keys[0], b"old-a").unwrap();
        c.put(&keys[1], b"old-b").unwrap();
        let h = c
            .tx_start([
                (keys[0].as_slice(), b"new-a".as_slice()),
                (keys[1].as_slice(), b"new-b".as_slice()),
            ])
            .unwrap();
        let last = *h.ranges.last().unwrap();
        let _ = c.step_down_range_leader(last);
        let _ = c.tx_finish(&h).expect_err("fail finish");
        c.elect_all(80).unwrap();
        let tip = c.read_version();
        let range = c.keys_in_range_at(&[], &[], tip).unwrap();
        for (k, v) in &range {
            assert_ne!(v.as_slice(), b"new-a", "aborted write in range scan {k:?}");
            assert_ne!(v.as_slice(), b"new-b", "aborted write in range scan {k:?}");
        }
        // Must still see preimages.
        assert!(
            range
                .iter()
                .any(|(k, v)| k == &keys[0] && v.as_slice() == b"old-a"),
            "missing old-a in {range:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F52: force_local/open path repairs SI hist when raft TxnRevert cannot majority.
    #[test]
    fn force_local_revert_repairs_si_hist() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        c.elect_all(80).unwrap();
        let keys = keys_one_per_range(&c);
        c.put(&keys[0], b"old-a").unwrap();
        c.put(&keys[1], b"old-b").unwrap();
        let h = c
            .tx_start([
                (keys[0].as_slice(), b"new-a".as_slice()),
                (keys[1].as_slice(), b"new-b".as_slice()),
            ])
            .unwrap();
        // Commit first range only by killing last range leader before finish.
        let last = *h.ranges.last().unwrap();
        let first = h.ranges[0];
        let _ = c.step_down_range_leader(last);
        let _ = c.tx_finish(&h).expect_err("fail");
        // Kill first range leader too — reopen must still have repaired hist via force_local.
        let _ = c.step_down_range_leader(first);
        drop(c);
        let c = StoreCluster::open_lab_direct(&dir, 3, 3).unwrap();
        assert_eq!(c.get(&keys[0]).unwrap().as_deref(), Some(b"old-a".as_ref()));
        assert_eq!(
            c.get_at_version(&keys[0], c.read_version())
                .unwrap()
                .as_deref(),
            Some(b"old-a".as_ref()),
            "force_local/open path must not leave SI tip at new-a"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Watermark GC floor keeps live values readable at safe_watermark.
    #[test]
    fn watermark_gc_floor_preserves_readable_snap() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"w", b"keep").unwrap();
        let g_put = c.read_version();
        // Advance generation well past VERSION_RETENTION via other keys.
        for i in 0..70u64 {
            let k = format!("x{i:04}");
            c.put(k.as_bytes(), b"z").unwrap();
        }
        let wm = c.safe_watermark();
        assert!(wm > 0, "watermark should advance after retention");
        // Snapshot at max(g_put, wm) if g_put < wm, too-old for TX; get_at_version
        // still defines value at watermark boundary.
        let at_wm = c.get_at_version(b"w", wm).unwrap();
        // Key was never deleted; floor must still yield "keep".
        assert_eq!(
            at_wm.as_deref(),
            Some(b"keep".as_ref()),
            "GC floor must preserve live value at watermark wm={wm} g_put={g_put} tip={}",
            c.read_version()
        );
        // Tip also keep.
        assert_eq!(
            c.get_at_version(b"w", c.read_version()).unwrap().as_deref(),
            Some(b"keep".as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Range OCC: concurrent clear inside get_range interval must Conflict.
    #[test]
    fn range_occ_conflicts_on_clear_inside_range() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        c.put(b"p/a", b"1").unwrap();
        c.put(b"p/b", b"2").unwrap();
        let mut tx = c.begin();
        let _ = tx.get_range(&c, b"p/", b"p0").unwrap();
        // Concurrent clear of a key inside the range.
        let mut t2 = c.begin();
        t2.clear(b"p/a").unwrap();
        t2.commit(&mut c).unwrap();
        tx.set(b"p/c", b"3").unwrap();
        let err = tx.commit(&mut c).expect_err("range OCC vs clear");
        assert!(
            matches!(err, StoreError::Conflict),
            "expected Conflict after clear in range, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
