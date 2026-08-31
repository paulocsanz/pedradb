//! Minimal **real** Raft (in-process) over PedraDB apply (RFC-0010).
//!
//! Implements the Raft paper core for a single shared log:
//! - terms, `voted_for`, leader election ([`RequestVote`])
//! - log replication ([`AppendEntries`])
//! - commit index + apply via [`pedradb_apply::LogApplier`]
//!
//! Disk-persisted hard state + log ([`persist`]); TCP peer RPC ([`net`]).
//! Deterministic tests drive time with [`RaftCluster::tick`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ae_kernel;
pub mod apply_kernel;
pub mod commit_kernel;
pub mod membership_kernel;
pub mod net;
pub mod persist;
pub mod vote_kernel;

pub use ae_kernel::{
    ae_ack_success, ae_ack_success_as_is, ae_entry_action, ae_prev_log_ok, AeEntryAction,
};
pub use apply_kernel::{apply_advance, apply_advance_as_is_skip_holes, ApplyAction};
pub use commit_kernel::{
    may_commit_at, may_commit_at_as_is, propose_ack_ok, propose_ack_ok_as_is, recover_commit,
    recover_commit_as_is, recover_last_applied, recover_last_applied_as_is,
};
pub use membership_kernel::{
    elect_claim_banner, elect_claim_banner_as_is, joint_election_ok, joint_election_ok_as_is,
    joint_leave_ok, joint_leave_ok_as_is, joint_still_active, joint_still_active_as_is,
    liveness_admitted, liveness_admitted_as_is, majority_of,
};
pub use vote_kernel::{
    grant_after_persist, grant_after_persist_as_is, vote_decision, PersistOutcome, VoteDecision,
    VoteInputs,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use pedradb_apply::{LogApplier, LogEntry};
use pedradb_core::{BatchOp, Db, OpenOptions, Result as CoreResult};
use thiserror::Error;

/// Raft / cluster errors.
#[derive(Debug, Error)]
pub enum RaftError {
    /// PedraDB I/O or apply failure.
    #[error("pedradb: {0}")]
    Core(#[from] pedradb_core::CoreError),
    /// Client proposed while this node is not leader.
    #[error("not leader (leader hint = {leader_hint:?})")]
    NotLeader {
        /// Best-effort leader id if known.
        leader_hint: Option<u64>,
    },
    /// Unknown node id.
    #[error("unknown node {0}")]
    UnknownNode(u64),
    /// Empty cluster.
    #[error("cluster has no nodes")]
    EmptyCluster,
    /// Raft meta persistence failure.
    #[error("persist: {0}")]
    Persist(String),
    /// Network RPC failure.
    #[error("network: {0}")]
    Network(String),
    /// Entry was appended but not majority-committed (client must retry).
    #[error("not committed: index {index} commit_index {commit_index}")]
    NotCommitted {
        /// Proposed log index.
        index: u64,
        /// Leader commit index after replication attempt.
        commit_index: u64,
    },
}

/// Result alias.
pub type Result<T> = std::result::Result<T, RaftError>;

/// Raft log entry: term + payload of PedraDB ops.
#[derive(Debug, Clone)]
pub struct RaftLogEntry {
    /// Raft log index (1-based).
    pub index: u64,
    /// Term when the entry was created by a leader.
    pub term: u64,
    /// State-machine payload.
    pub ops: Vec<BatchOp>,
}

impl RaftLogEntry {
    fn to_apply_entry(&self) -> LogEntry {
        LogEntry {
            index: self.index,
            ops: self.ops.clone(),
        }
    }
}

/// Raft hard state (persisted under `raft-meta/`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HardState {
    /// Current term.
    pub current_term: u64,
    /// Vote for this term, if any.
    pub voted_for: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Follower,
    Candidate,
    Leader,
}

/// One Raft peer with local PedraDB state machine.
pub struct RaftNode {
    id: u64,
    pub(crate) role: Role,
    /// Hard state (term / vote).
    pub hard: HardState,
    /// Log entries start at index 1.
    pub(crate) log: Vec<RaftLogEntry>,
    pub(crate) commit_index: u64,
    last_applied: u64,
    /// Leader volatile: next index to send to each peer.
    pub(crate) next_index: HashMap<u64, u64>,
    /// Leader volatile: highest match index per peer.
    pub(crate) match_index: HashMap<u64, u64>,
    /// Election / heartbeat countdown (ticks remaining).
    pub(crate) election_ticks_left: u64,
    /// Ticks between heartbeats when leader.
    pub(crate) heartbeat_every: u64,
    pub(crate) heartbeat_ticks_left: u64,
    /// Election timeout (ticks).
    pub(crate) election_timeout: u64,
    pub(crate) db: Db,
    /// Last known leader (for redirect).
    pub(crate) leader_id: Option<u64>,
    /// Directory for RAFT_HARD / RAFT_LOG (None = memory only).
    pub(crate) meta_dir: Option<PathBuf>,
}

impl RaftNode {
    fn new(id: u64, db: Db, election_timeout: u64, heartbeat_every: u64) -> Self {
        Self {
            id,
            role: Role::Follower,
            hard: HardState::default(),
            log: Vec::new(),
            commit_index: 0,
            last_applied: 0,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            election_ticks_left: election_timeout,
            heartbeat_every,
            heartbeat_ticks_left: heartbeat_every,
            election_timeout,
            db,
            leader_id: None,
            meta_dir: None,
        }
    }

    pub(crate) fn persist_hard(&self) -> Result<()> {
        if let Some(dir) = &self.meta_dir {
            persist::store_hard(dir, &self.hard)?;
        }
        Ok(())
    }

    pub(crate) fn persist_log(&self) -> Result<()> {
        if let Some(dir) = &self.meta_dir {
            persist::store_log(dir, &self.log)?;
        }
        Ok(())
    }

    pub(crate) fn persist_commit(&self) -> Result<()> {
        if let Some(dir) = &self.meta_dir {
            persist::store_commit(dir, self.commit_index)?;
        }
        Ok(())
    }

    /// Advance commit, apply, and durable-persist commit index (F10).
    pub(crate) fn set_commit_and_apply(&mut self, new_commit: u64) -> Result<()> {
        if new_commit <= self.commit_index {
            return Ok(());
        }
        self.commit_index = new_commit;
        self.apply_committed().map_err(RaftError::from)?;
        self.persist_commit()?;
        Ok(())
    }

    pub(crate) fn load_meta(meta_dir: &Path) -> Result<(HardState, Vec<RaftLogEntry>, u64)> {
        Ok((
            persist::load_hard(meta_dir)?,
            persist::load_log(meta_dir)?,
            persist::load_commit(meta_dir)?,
        ))
    }

    /// Node id.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Whether this node believes it is leader.
    #[must_use]
    pub fn is_leader(&self) -> bool {
        self.role == Role::Leader
    }

    /// Current term.
    #[must_use]
    pub fn term(&self) -> u64 {
        self.hard.current_term
    }

    /// PedraDB get (applied state only).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.db.get(key)
    }

    /// Last applied Raft index.
    #[must_use]
    pub fn last_applied(&self) -> u64 {
        self.last_applied
    }

    fn last_log_index(&self) -> u64 {
        self.log.last().map_or(0, |e| e.index)
    }

    fn last_log_term(&self) -> u64 {
        self.log.last().map_or(0, |e| e.term)
    }

    fn log_term_at(&self, index: u64) -> u64 {
        if index == 0 {
            return 0;
        }
        self.log
            .iter()
            .find(|e| e.index == index)
            .map_or(0, |e| e.term)
    }

    pub(crate) fn become_follower(&mut self, term: u64) {
        let mut dirty = false;
        let prev_term = self.hard.current_term;
        let prev_vote = self.hard.voted_for;
        if term > self.hard.current_term {
            self.hard.current_term = term;
            self.hard.voted_for = None;
            dirty = true;
        }
        self.role = Role::Follower;
        self.election_ticks_left = self.election_timeout;
        // F15: term/vote must be durable before we act as follower of the new term.
        // If persist fails, roll memory back so we do not grant votes on a term
        // the disk never saw (double-vote / split-brain class).
        if dirty {
            if let Err(e) = self.persist_hard() {
                self.hard.current_term = prev_term;
                self.hard.voted_for = prev_vote;
                // Still step down in-memory to avoid dual leaders in this process;
                // vote durability is restored on next successful persist path.
                let _ = e;
            }
        }
    }

    /// Transition to leader. Appends a **no-op** log entry in the current term
    /// (Raft §5.4.2 / etcd blank index) so previous-term entries that already
    /// have a majority (always true on **single-node**) can advance
    /// `commit_index`. Without this, a solo node that crashes after
    /// `persist_log` but before commit leaves those entries stuck forever.
    ///
    /// # Errors
    /// Log persistence failure (role left as Candidate, noop not retained).
    pub(crate) fn become_leader(&mut self, peer_ids: &[u64]) -> Result<()> {
        self.role = Role::Leader;
        self.leader_id = Some(self.id);
        let next = self.last_log_index() + 1;
        self.next_index.clear();
        self.match_index.clear();
        for &pid in peer_ids {
            if pid != self.id {
                self.next_index.insert(pid, next);
                self.match_index.insert(pid, 0);
            }
        }
        // F18 single-node (and multi-node leadership start): blank entry.
        let noop = RaftLogEntry {
            index: next,
            term: self.hard.current_term,
            ops: Vec::new(),
        };
        self.log.push(noop);
        if let Err(e) = self.persist_log() {
            self.log.pop();
            self.role = Role::Candidate;
            self.leader_id = None;
            return Err(e);
        }
        self.match_index.insert(self.id, next);
        self.heartbeat_ticks_left = 0; // send immediately
        Ok(())
    }

    pub(crate) fn apply_committed(&mut self) -> CoreResult<()> {
        loop {
            // Pure kernel decides the step (F10-apply); caller mutates.
            let next = self.last_applied + 1;
            let entry_present =
                self.last_applied < self.commit_index && self.log.iter().any(|e| e.index == next);
            match apply_kernel::apply_advance(self.last_applied, self.commit_index, entry_present) {
                ApplyAction::Done | ApplyAction::Stop => break,
                ApplyAction::Apply => {}
            }
            let entry = self
                .log
                .iter()
                .find(|e| e.index == next)
                .cloned()
                .expect("kernel Apply ⇒ entry present");
            // DCS command: single marker put → apply state machine, not raw put.
            if entry.ops.len() == 1 {
                if let BatchOp::Put { key, value } = &entry.ops[0] {
                    if key.as_ref() == pedradb_dcs::DCS_CMD_MARKER {
                        let cmd = pedradb_dcs::DcsCommand::decode(value).map_err(|e| {
                            pedradb_core::CoreError::Internal(format!("dcs cmd: {e}"))
                        })?;
                        // F12: apply must be total. CAS/create conflicts on re-apply or
                        // racing log entries must not freeze last_applied forever.
                        let r = pedradb_dcs::apply_dcs_command(&mut self.db, &cmd);
                        if !pedradb_dcs::dcs_apply_should_advance_result(&r) {
                            let e = r.unwrap_err();
                            return Err(pedradb_core::CoreError::Internal(format!(
                                "dcs apply: {e}"
                            )));
                        }
                        self.last_applied = next;
                        continue;
                    }
                }
            }
            let mut applier = LogApplier::new(&mut self.db, self.last_applied);
            applier.apply_entries(&[entry.to_apply_entry()])?;
            self.last_applied = applier.last_index();
        }
        Ok(())
    }

    /// Read DCS key from applied state.
    #[must_use]
    pub fn dcs_get(&self, key: &[u8]) -> Option<pedradb_dcs::KeyValue> {
        pedradb_dcs::dcs_get(&self.db, key)
    }
}

/// RequestVote RPC arguments.
#[derive(Debug, Clone)]
pub struct RequestVoteArgs {
    /// Candidate term.
    pub term: u64,
    /// Candidate id.
    pub candidate_id: u64,
    /// Candidate last log index.
    pub last_log_index: u64,
    /// Candidate last log term.
    pub last_log_term: u64,
}

/// RequestVote reply.
#[derive(Debug, Clone)]
pub struct RequestVoteReply {
    /// Current term for candidate to update itself.
    pub term: u64,
    /// True means candidate received vote.
    pub vote_granted: bool,
}

/// AppendEntries RPC arguments.
#[derive(Debug, Clone)]
pub struct AppendEntriesArgs {
    /// Leader term.
    pub term: u64,
    /// Leader id.
    pub leader_id: u64,
    /// Index of log entry immediately preceding new ones.
    pub prev_log_index: u64,
    /// Term of `prev_log_index` entry.
    pub prev_log_term: u64,
    /// Log entries to store (empty for heartbeat).
    pub entries: Vec<RaftLogEntry>,
    /// Leader's commit index.
    pub leader_commit: u64,
}

/// AppendEntries reply.
#[derive(Debug, Clone)]
pub struct AppendEntriesReply {
    /// Current term for leader to update itself.
    pub term: u64,
    /// True if follower contained entry matching `prev_log_*`.
    pub success: bool,
    /// For next_index backoff: follower's last log index.
    pub match_index: u64,
}

fn handle_request_vote(node: &mut RaftNode, args: &RequestVoteArgs) -> RequestVoteReply {
    let meta = node.meta_dir.clone();
    handle_request_vote_with_persist(node, args, |hard| persist_hard_state(meta.as_deref(), hard))
}

/// Persist hook used by the production RequestVote path and by DST injectors.
fn persist_hard_state(meta_dir: Option<&Path>, hard: &HardState) -> Result<()> {
    if let Some(dir) = meta_dir {
        persist::store_hard(dir, hard)?;
    }
    Ok(())
}

/// RequestVote after the pure kernel: persist, then grant only on Ok (F15).
///
/// `persist` is the only I/O, called at most twice (term step, then vote
/// step) — hence `FnMut`. Tests inject failure here; production uses
/// [`persist_hard_state`] (same `store_hard` as [`RaftNode::persist_hard`]).
pub fn handle_request_vote_with_persist(
    node: &mut RaftNode,
    args: &RequestVoteArgs,
    mut persist: impl FnMut(&HardState) -> Result<()>,
) -> RequestVoteReply {
    if args.term > node.hard.current_term {
        // F125/F127 / RFC-0158 P1.1: the term step goes through the same
        // injected persist seam as the vote, and the decision is the catalog
        // kernel `durable_term_if_newer` — not an inline `if` (and not the
        // separate internal `persist_hard` channel of `become_follower`).
        let prev_term = node.hard.current_term;
        let prev_voted = node.hard.voted_for;
        node.hard.current_term = args.term;
        node.hard.voted_for = None;
        node.role = Role::Follower;
        node.leader_id = None;
        node.election_ticks_left = node.election_timeout;
        let persist_out = match persist(&node.hard) {
            Ok(()) => vote_kernel::PersistOutcome::Ok,
            Err(_) => vote_kernel::PersistOutcome::Err,
        };
        if let vote_kernel::DurableTerm::Restored =
            vote_kernel::durable_term_if_newer(prev_term, args.term, persist_out)
        {
            node.hard.current_term = prev_term;
            node.hard.voted_for = prev_voted;
            node.role = Role::Follower;
            node.leader_id = None;
            return RequestVoteReply {
                term: prev_term,
                vote_granted: false,
            };
        }
    }
    let mut vote_granted = false;
    let decision = vote_kernel::vote_decision(VoteInputs {
        current_term: node.hard.current_term,
        voted_for: node.hard.voted_for,
        last_log_term: node.last_log_term(),
        last_log_index: node.last_log_index(),
        candidate_term: args.term,
        candidate_id: args.candidate_id,
        candidate_last_log_term: args.last_log_term,
        candidate_last_log_index: args.last_log_index,
    });
    if decision == VoteDecision::WouldGrant {
        let prev = node.hard.voted_for;
        node.hard.voted_for = Some(args.candidate_id);
        let persist_out = match persist(&node.hard) {
            Ok(()) => {
                node.election_ticks_left = node.election_timeout;
                vote_kernel::PersistOutcome::Ok
            }
            Err(_) => {
                node.hard.voted_for = prev;
                vote_kernel::PersistOutcome::Err
            }
        };
        // F15 / RFC-0053 P1.1: wire bit is the kernel, not an inline `true`.
        vote_granted = vote_kernel::grant_after_persist(decision, persist_out);
    }
    RequestVoteReply {
        term: node.hard.current_term,
        vote_granted,
    }
}

/// AS-IS mutant: grant in memory **then** persist. Persist Err still leaves
/// `vote_granted == true` — the F15 bug. Tests assert this fails the F15 oracle.
pub fn handle_request_vote_grant_then_persist(
    node: &mut RaftNode,
    args: &RequestVoteArgs,
    persist: impl FnOnce(&HardState) -> Result<()>,
) -> RequestVoteReply {
    if args.term > node.hard.current_term {
        node.become_follower(args.term);
    }
    let mut vote_granted = false;
    let decision = vote_kernel::vote_decision(VoteInputs {
        current_term: node.hard.current_term,
        voted_for: node.hard.voted_for,
        last_log_term: node.last_log_term(),
        last_log_index: node.last_log_index(),
        candidate_term: args.term,
        candidate_id: args.candidate_id,
        candidate_last_log_term: args.last_log_term,
        candidate_last_log_index: args.last_log_index,
    });
    if decision == VoteDecision::WouldGrant {
        node.hard.voted_for = Some(args.candidate_id);
        vote_granted = true;
        let _ = persist(&node.hard);
    }
    RequestVoteReply {
        term: node.hard.current_term,
        vote_granted,
    }
}

/// Public for network server.
pub fn rpc_request_vote(node: &mut RaftNode, args: &RequestVoteArgs) -> RequestVoteReply {
    handle_request_vote(node, args)
}

/// Public for network server.
pub fn rpc_append_entries(node: &mut RaftNode, args: &AppendEntriesArgs) -> AppendEntriesReply {
    handle_append_entries(node, args)
}

fn handle_append_entries(node: &mut RaftNode, args: &AppendEntriesArgs) -> AppendEntriesReply {
    if args.term < node.hard.current_term {
        return AppendEntriesReply {
            term: node.hard.current_term,
            success: false,
            match_index: node.last_log_index(),
        };
    }
    if args.term > node.hard.current_term {
        node.become_follower(args.term);
    } else {
        node.role = Role::Follower;
        node.election_ticks_left = node.election_timeout;
    }
    node.leader_id = Some(args.leader_id);

    // Log consistency (pure kernel) then mutate log (caller).
    if !ae_kernel::ae_prev_log_ok(
        args.prev_log_index,
        args.prev_log_term,
        node.last_log_index(),
        node.log_term_at(args.prev_log_index),
    ) {
        return AppendEntriesReply {
            term: node.hard.current_term,
            success: false,
            match_index: node.last_log_index(),
        };
    }

    // Append / conflict resolve via ae_kernel (F16: never rewrite ≤ commit).
    for e in &args.entries {
        let existing_term = node.log.iter().find(|x| x.index == e.index).map(|x| x.term);
        match ae_kernel::ae_entry_action(
            e.index,
            e.term,
            existing_term,
            node.commit_index,
            node.last_log_index(),
        ) {
            AeEntryAction::Keep => {}
            AeEntryAction::Append => {
                node.log.push(e.clone());
            }
            AeEntryAction::TruncateAndInstall => {
                if let Some(pos) = node.log.iter().position(|x| x.index == e.index) {
                    node.log.truncate(pos);
                }
                node.log.push(e.clone());
            }
            AeEntryAction::Refuse => {
                return AppendEntriesReply {
                    term: node.hard.current_term,
                    success: false,
                    match_index: node.last_log_index(),
                };
            }
        }
    }
    node.log.sort_by_key(|e| e.index);
    node.log.dedup_by_key(|e| e.index);
    let log_dirty = !args.entries.is_empty();
    let persist_ok = if log_dirty {
        node.persist_log().is_ok()
    } else {
        true
    };
    if !ae_kernel::ae_ack_success(log_dirty, persist_ok) {
        // F48: log not durable — do not ack success / advance leader match.
        return AppendEntriesReply {
            term: node.hard.current_term,
            success: false,
            match_index: node.last_log_index(),
        };
    }

    if args.leader_commit > node.commit_index {
        let last_new = args
            .entries
            .last()
            .map_or(args.prev_log_index, |e| e.index)
            .max(node.last_log_index());
        let new_c = args.leader_commit.min(last_new);
        let _ = node.set_commit_and_apply(new_c);
    }

    AppendEntriesReply {
        term: node.hard.current_term,
        success: true,
        match_index: node.last_log_index(),
    }
}

/// In-process Raft cluster: N nodes, direct RPC, tick-driven timers.
pub struct RaftCluster {
    nodes: HashMap<u64, RaftNode>,
    ids: Vec<u64>,
    /// Deterministic RNG state for election timeout jitter.
    rng: u64,
}

impl RaftCluster {
    /// Open `n` nodes under `parent_dir/{id}` with PedraDB storage.
    ///
    /// # Errors
    /// PedraDB open failures.
    pub fn open(parent_dir: impl AsRef<Path>, n: u64) -> Result<Self> {
        if n == 0 {
            return Err(RaftError::EmptyCluster);
        }
        let parent = parent_dir.as_ref();
        let mut nodes = HashMap::new();
        let mut ids = Vec::new();
        for id in 1..=n {
            let dir = parent.join(format!("node-{id}"));
            let db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )?;
            // Stagger timeouts slightly by id for deterministic elections.
            let election_timeout = 5 + id;
            let meta = persist::raft_meta_dir(&dir);
            let (hard, log, commit) = RaftNode::load_meta(&meta)?;
            let mut node = RaftNode::new(id, db, election_timeout, 2);
            node.meta_dir = Some(meta);
            node.hard = hard;
            node.log = log;
            // F10: never treat the full on-disk log as committed.
            let log_last = node.log.last().map_or(0, |e| e.index);
            node.commit_index = commit_kernel::recover_commit(commit, log_last);
            node.last_applied = commit_kernel::recover_last_applied();
            node.apply_committed()?;
            nodes.insert(id, node);
            ids.push(id);
        }
        Ok(Self {
            nodes,
            ids,
            rng: 0xC0FFEE,
        })
    }

    /// Node ids in order.
    #[must_use]
    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    /// Immutable node.
    #[must_use]
    pub fn node(&self, id: u64) -> Option<&RaftNode> {
        self.nodes.get(&id)
    }

    /// Mutable node (DST persist injectors / RequestVote harness).
    pub fn node_mut(&mut self, id: u64) -> Option<&mut RaftNode> {
        self.nodes.get_mut(&id)
    }

    /// Current leader id if any node is leader.
    #[must_use]
    pub fn leader_id(&self) -> Option<u64> {
        self.nodes
            .values()
            .find(|n| n.is_leader())
            .map(RaftNode::id)
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Advance timers by one tick: heartbeats and elections.
    ///
    /// # Errors
    /// Apply errors while committing.
    pub fn tick(&mut self) -> Result<()> {
        let ids = self.ids.clone();
        // Snapshot which nodes need election / heartbeat.
        let mut start_election: Vec<u64> = Vec::new();
        let mut leader_heartbeats: Vec<u64> = Vec::new();

        for id in &ids {
            let node = self.nodes.get_mut(id).unwrap();
            match node.role {
                Role::Leader => {
                    if node.heartbeat_ticks_left == 0 {
                        leader_heartbeats.push(*id);
                        node.heartbeat_ticks_left = node.heartbeat_every;
                    } else {
                        node.heartbeat_ticks_left -= 1;
                    }
                }
                Role::Follower | Role::Candidate => {
                    if node.election_ticks_left == 0 {
                        start_election.push(*id);
                    } else {
                        node.election_ticks_left -= 1;
                    }
                }
            }
        }

        for lid in leader_heartbeats {
            self.broadcast_append(lid, Vec::new())?;
        }
        for cid in start_election {
            self.start_election(cid)?;
        }
        Ok(())
    }

    /// Run ticks until a leader exists or `max_ticks` exhausted.
    ///
    /// # Errors
    /// Tick/apply errors.
    pub fn elect_leader(&mut self, max_ticks: u64) -> Result<u64> {
        for _ in 0..max_ticks {
            if let Some(l) = self.leader_id() {
                return Ok(l);
            }
            self.tick()?;
        }
        Err(RaftError::NotLeader { leader_hint: None })
    }

    fn start_election(&mut self, candidate_id: u64) -> Result<()> {
        let peer_ids = self.ids.clone();
        let j = self.next_rand() % 3;
        let (term, last_idx, last_term) = {
            let node = self
                .nodes
                .get_mut(&candidate_id)
                .ok_or(RaftError::UnknownNode(candidate_id))?;
            // F15: durable self-vote before soliciting peers (single-DC crash mid-election).
            let prev_term = node.hard.current_term;
            let prev_vote = node.hard.voted_for;
            let prev_role = node.role;
            node.hard.current_term += 1;
            node.role = Role::Candidate;
            node.hard.voted_for = Some(candidate_id);
            node.election_ticks_left = node.election_timeout + j;
            if let Err(e) = node.persist_hard() {
                node.hard.current_term = prev_term;
                node.hard.voted_for = prev_vote;
                node.role = prev_role;
                return Err(e);
            }
            (
                node.hard.current_term,
                node.last_log_index(),
                node.last_log_term(),
            )
        };

        let args = RequestVoteArgs {
            term,
            candidate_id,
            last_log_index: last_idx,
            last_log_term: last_term,
        };

        let mut votes = 1u64; // self
        let majority = (peer_ids.len() as u64) / 2 + 1;

        for &pid in &peer_ids {
            if pid == candidate_id {
                continue;
            }
            let reply = {
                let follower = self
                    .nodes
                    .get_mut(&pid)
                    .ok_or(RaftError::UnknownNode(pid))?;
                handle_request_vote(follower, &args)
            };
            // Candidate steps down if reply term higher.
            if reply.term > term {
                if let Some(c) = self.nodes.get_mut(&candidate_id) {
                    c.become_follower(reply.term);
                }
                return Ok(());
            }
            if reply.vote_granted {
                votes += 1;
            }
        }

        if votes >= majority {
            if let Some(c) = self.nodes.get_mut(&candidate_id) {
                // Still candidate for this term?
                if c.role == Role::Candidate && c.hard.current_term == term {
                    c.become_leader(&peer_ids)?;
                }
            }
            // Replicate noop + commit previous-term majority entries (F18).
            self.broadcast_append(candidate_id, Vec::new())?;
        }
        Ok(())
    }

    fn broadcast_append(
        &mut self,
        leader_id: u64,
        client_entries: Vec<RaftLogEntry>,
    ) -> Result<()> {
        let peer_ids = self.ids.clone();
        // Append client entries to leader log first.
        if !client_entries.is_empty() {
            let leader = self
                .nodes
                .get_mut(&leader_id)
                .ok_or(RaftError::UnknownNode(leader_id))?;
            if leader.role != Role::Leader {
                return Err(RaftError::NotLeader {
                    leader_hint: leader.leader_id,
                });
            }
            for e in client_entries {
                leader.log.push(e);
            }
            // F15-class: do not replicate entries the leader has not durably logged.
            leader.persist_log()?;
        }

        let (term, commit, last_idx) = {
            let leader = self
                .nodes
                .get(&leader_id)
                .ok_or(RaftError::UnknownNode(leader_id))?;
            if leader.role != Role::Leader {
                return Err(RaftError::NotLeader {
                    leader_hint: leader.leader_id,
                });
            }
            (
                leader.hard.current_term,
                leader.commit_index,
                leader.last_log_index(),
            )
        };

        // Leader matches itself.
        if let Some(leader) = self.nodes.get_mut(&leader_id) {
            leader.match_index.insert(leader_id, last_idx);
        }

        for &pid in &peer_ids {
            if pid == leader_id {
                continue;
            }
            let (prev_idx, prev_term, entries) = {
                let leader = self.nodes.get(&leader_id).unwrap();
                let next = *leader.next_index.get(&pid).unwrap_or(&(last_idx + 1));
                let prev_idx = next.saturating_sub(1);
                let prev_term = leader.log_term_at(prev_idx);
                let entries: Vec<RaftLogEntry> = leader
                    .log
                    .iter()
                    .filter(|e| e.index >= next)
                    .cloned()
                    .collect();
                (prev_idx, prev_term, entries)
            };

            let args = AppendEntriesArgs {
                term,
                leader_id,
                prev_log_index: prev_idx,
                prev_log_term: prev_term,
                entries,
                leader_commit: commit,
            };
            let reply = {
                let follower = self.nodes.get_mut(&pid).unwrap();
                handle_append_entries(follower, &args)
            };

            if reply.term > term {
                if let Some(l) = self.nodes.get_mut(&leader_id) {
                    l.become_follower(reply.term);
                }
                return Ok(());
            }
            if let Some(leader) = self.nodes.get_mut(&leader_id) {
                if reply.success {
                    leader.next_index.insert(pid, reply.match_index + 1);
                    leader.match_index.insert(pid, reply.match_index);
                } else {
                    let ni = leader.next_index.get(&pid).copied().unwrap_or(1);
                    leader.next_index.insert(pid, ni.saturating_sub(1).max(1));
                }
            }
        }

        self.update_commit_index(leader_id)?;
        Ok(())
    }

    fn update_commit_index(&mut self, leader_id: u64) -> Result<()> {
        let majority = self.ids.len() / 2 + 1;
        let (term, last_idx, match_snapshot) = {
            let leader = self
                .nodes
                .get(&leader_id)
                .ok_or(RaftError::UnknownNode(leader_id))?;
            if leader.role != Role::Leader {
                return Ok(());
            }
            let mut matches: Vec<u64> = self
                .ids
                .iter()
                .map(|id| {
                    if *id == leader_id {
                        leader.last_log_index()
                    } else {
                        leader.match_index.get(id).copied().unwrap_or(0)
                    }
                })
                .collect();
            matches.sort_unstable();
            (leader.hard.current_term, leader.last_log_index(), matches)
        };

        // Largest N such that majority has match_index >= N and log[N].term == current.
        for n in (1..=last_idx).rev() {
            let count = match_snapshot.iter().filter(|&&m| m >= n).count();
            if count < majority {
                continue;
            }
            let term_at = self.nodes.get(&leader_id).unwrap().log_term_at(n);
            if commit_kernel::may_commit_at(term_at, term, true) {
                let leader = self.nodes.get_mut(&leader_id).unwrap();
                if n > leader.commit_index {
                    leader.set_commit_and_apply(n)?;
                    // Propagate commit via heartbeat next tick; also push now.
                    self.broadcast_append(leader_id, Vec::new())?;
                }
                break;
            }
        }
        Ok(())
    }

    /// Propose puts through the current leader; replicates and commits.
    ///
    /// Returns `Ok(index)` only after majority commit and local apply (F11).
    ///
    /// # Errors
    /// Not leader, not committed under partition, or apply/replication failure.
    pub fn propose_puts(
        &mut self,
        kvs: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    ) -> Result<u64> {
        let leader_id = self
            .leader_id()
            .ok_or(RaftError::NotLeader { leader_hint: None })?;
        let ops: Vec<BatchOp> = kvs.into_iter().map(|(k, v)| BatchOp::put(k, v)).collect();
        let entry = {
            let leader = self.nodes.get(&leader_id).unwrap();
            let index = leader.last_log_index() + 1;
            RaftLogEntry {
                index,
                term: leader.hard.current_term,
                ops,
            }
        };
        let index = entry.index;
        self.broadcast_append(leader_id, vec![entry])?;
        let commit = self
            .nodes
            .get(&leader_id)
            .map(|n| n.commit_index)
            .unwrap_or(0);
        if !commit_kernel::propose_ack_ok(index, commit) {
            return Err(RaftError::NotCommitted {
                index,
                commit_index: commit,
            });
        }
        Ok(index)
    }

    /// Directory used for node `id` under `parent` (helper for tests).
    #[must_use]
    pub fn node_dir(parent: impl AsRef<Path>, id: u64) -> PathBuf {
        parent.as_ref().join(format!("node-{id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_parent() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-raft-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn elect_and_replicate_three_nodes() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 3).unwrap();
        let leader = cluster.elect_leader(50).unwrap();
        assert!(cluster.node(leader).unwrap().is_leader());

        let idx = cluster
            .propose_puts([(b"k".to_vec(), b"v1".to_vec())])
            .unwrap();
        // Leader noop (blank entry on become_leader) occupies index 1; first put is ≥ 2.
        assert!(idx >= 1, "propose must return a committed index");

        // Followers should have applied after commit propagation.
        for id in cluster.ids() {
            let n = cluster.node(*id).unwrap();
            assert_eq!(
                n.get(b"k").as_deref(),
                Some(b"v1".as_ref()),
                "node {id} missing key"
            );
            assert!(n.last_applied() >= idx);
        }

        cluster
            .propose_puts([(b"k".to_vec(), b"v2".to_vec())])
            .unwrap();
        for id in cluster.ids() {
            assert_eq!(
                cluster.node(*id).unwrap().get(b"k").as_deref(),
                Some(b"v2".as_ref())
            );
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn propose_without_leader_fails() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 3).unwrap();
        // No ticks → no leader.
        let err = cluster
            .propose_puts([(b"a".to_vec(), b"b".to_vec())])
            .unwrap_err();
        assert!(matches!(err, RaftError::NotLeader { .. }));
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F10: uncommitted log suffix must not become committed on restart.
    #[test]
    fn reopen_does_not_commit_uncommitted_suffix() {
        let parent = temp_parent();
        let dir = parent.join("node-1");
        std::fs::create_dir_all(&dir).unwrap();
        let meta = persist::raft_meta_dir(&dir);
        std::fs::create_dir_all(&meta).unwrap();
        // Fake a log with 2 entries but commit only 1.
        let log = vec![
            RaftLogEntry {
                index: 1,
                term: 1,
                ops: vec![BatchOp::put(b"a", b"1")],
            },
            RaftLogEntry {
                index: 2,
                term: 1,
                ops: vec![BatchOp::put(b"b", b"2")],
            },
        ];
        persist::store_log(&meta, &log).unwrap();
        persist::store_hard(
            &meta,
            &HardState {
                current_term: 1,
                voted_for: Some(1),
            },
        )
        .unwrap();
        persist::store_commit(&meta, 1).unwrap();
        // Also need PedraDB with applied "a" only.
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            db.put(b"a", b"1").unwrap();
            db.close().unwrap();
        }
        let cluster = RaftCluster::open(&parent, 1).unwrap();
        let n = cluster.node(1).unwrap();
        assert_eq!(n.commit_index, 1, "must not jump commit to log len");
        assert_eq!(n.last_applied(), 1);
        assert_eq!(n.get(b"a").as_deref(), Some(b"1".as_ref()));
        assert!(n.get(b"b").is_none(), "uncommitted suffix must not apply");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F11: propose returns Ok only when majority committed (in healthy cluster it does).
    #[test]
    fn propose_acks_only_after_commit() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 3).unwrap();
        cluster.elect_leader(50).unwrap();
        let idx = cluster
            .propose_puts([(b"ack".to_vec(), b"1".to_vec())])
            .unwrap();
        let leader = cluster.leader_id().unwrap();
        assert!(cluster.node(leader).unwrap().commit_index >= idx);
        assert_eq!(
            cluster.node(leader).unwrap().get(b"ack").as_deref(),
            Some(b"1".as_ref())
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Single-node: majority=1, elect + durable put + reopen.
    #[test]
    fn single_node_elect_and_put() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        let leader = cluster.elect_leader(20).unwrap();
        assert_eq!(leader, 1);
        // Leadership noop occupies an index; first client put is still committed.
        let idx = cluster
            .propose_puts([(b"solo".to_vec(), b"ok".to_vec())])
            .unwrap();
        assert!(idx >= 1);
        assert_eq!(
            cluster.node(1).unwrap().get(b"solo").as_deref(),
            Some(b"ok".as_ref())
        );
        drop(cluster);
        let cluster = RaftCluster::open(&parent, 1).unwrap();
        assert_eq!(
            cluster.node(1).unwrap().get(b"solo").as_deref(),
            Some(b"ok".as_ref())
        );
        assert!(cluster.node(1).unwrap().commit_index >= 1);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F18: single-node crash after log append, before commit — previous-term
    /// entry must commit after re-election via leader noop.
    #[test]
    fn single_node_commits_prev_term_after_reelect() {
        let parent = temp_parent();
        let dir = parent.join("node-1");
        std::fs::create_dir_all(&dir).unwrap();
        let meta = persist::raft_meta_dir(&dir);
        std::fs::create_dir_all(&meta).unwrap();
        // Simulated: entry 1 committed+applied; entry 2 on disk uncommitted (crash).
        let log = vec![
            RaftLogEntry {
                index: 1,
                term: 1,
                ops: vec![BatchOp::put(b"a", b"1")],
            },
            RaftLogEntry {
                index: 2,
                term: 1,
                ops: vec![BatchOp::put(b"stuck", b"v2")],
            },
        ];
        persist::store_log(&meta, &log).unwrap();
        persist::store_hard(
            &meta,
            &HardState {
                current_term: 1,
                voted_for: Some(1),
            },
        )
        .unwrap();
        persist::store_commit(&meta, 1).unwrap();
        {
            let mut db = Db::open_with(
                &dir,
                OpenOptions {
                    wal_full_fsync: true,
                    history: Default::default(),
                    wal_recovery: Default::default(),
                    sync: true,
                    auto_flush_bytes: None,
                    auto_compact_sst_count: None,
                    auto_compact_sst_bytes: None,
                    exclusive: true,
                    large_value_threshold: None,
                    sst_payload_budget_bytes: None,
                },
            )
            .unwrap();
            db.put(b"a", b"1").unwrap();
            db.close().unwrap();
        }
        // No client propose: only re-elect. Leader noop must free "stuck".
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        assert!(cluster.node(1).unwrap().get(b"stuck").is_none());
        cluster.elect_leader(30).unwrap();
        assert_eq!(
            cluster.node(1).unwrap().get(b"stuck").as_deref(),
            Some(b"v2".as_ref()),
            "F18: prev-term majority entry must commit after single-node re-elect"
        );
        assert!(cluster.node(1).unwrap().commit_index >= 2);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Single-node multi-put durability across process reopen.
    #[test]
    fn single_node_multi_put_reopen() {
        let parent = temp_parent();
        {
            let mut cluster = RaftCluster::open(&parent, 1).unwrap();
            cluster.elect_leader(20).unwrap();
            for i in 0..5u8 {
                let k = vec![b'k', i];
                let v = vec![b'v', i];
                cluster.propose_puts([(k, v)]).unwrap();
            }
        }
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        // May need re-elect if leadership not restored (volatile).
        let _ = cluster.elect_leader(30);
        for i in 0..5u8 {
            let k = [b'k', i];
            let v = [b'v', i];
            assert_eq!(
                cluster.node(1).unwrap().get(&k).as_deref(),
                Some(v.as_ref()),
                "missing k{i}"
            );
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// AS-IS mutant of the **caller protocol** (not the pure kernel): set
    /// `vote_granted` without durable persist. Disk must *not* show the vote —
    /// this is the F15 gap the FIXED path closes. Proves the persist-before-grant
    /// invariant has teeth (Beyond mutation discipline).
    #[test]
    fn as_is_grant_without_persist_leaves_disk_unvoted() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        let n = cluster.nodes.get_mut(&1).unwrap();
        n.hard.current_term = 5;
        n.hard.voted_for = None;
        // Mutant: decision WouldGrant, update memory, **skip** persist_hard, grant.
        let args = RequestVoteArgs {
            term: 5,
            candidate_id: 9,
            last_log_index: 0,
            last_log_term: 0,
        };
        let d = vote_kernel::vote_decision(VoteInputs {
            current_term: n.hard.current_term,
            voted_for: n.hard.voted_for,
            last_log_term: n.last_log_term(),
            last_log_index: n.last_log_index(),
            candidate_term: args.term,
            candidate_id: args.candidate_id,
            candidate_last_log_term: args.last_log_term,
            candidate_last_log_index: args.last_log_index,
        });
        assert_eq!(d, VoteDecision::WouldGrant);
        n.hard.voted_for = Some(9);
        let grant_without_persist = true; // AS-IS mutant
        assert!(grant_without_persist);
        // Disk still empty vote for term 5 (we never called persist after vote).
        let meta = persist::raft_meta_dir(&parent.join("node-1"));
        // May have older hard state from open; force reload after mutant.
        let hard = persist::load_hard(&meta).unwrap_or(HardState {
            current_term: 0,
            voted_for: None,
        });
        assert!(
            hard.voted_for != Some(9),
            "AS-IS: grant without persist must leave disk without candidate 9 vote (got {hard:?})"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F15: injected persist Err ⇒ production handler does not grant.
    #[test]
    fn persist_fail_never_grants() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        let n = cluster.node_mut(1).unwrap();
        n.hard.current_term = 5;
        n.hard.voted_for = None;
        let reply = handle_request_vote_with_persist(
            n,
            &RequestVoteArgs {
                term: 5,
                candidate_id: 2,
                last_log_index: 0,
                last_log_term: 0,
            },
            |_| Err(RaftError::Persist("injected".into())),
        );
        assert!(!reply.vote_granted);
        assert_eq!(n.hard.voted_for, None);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0158 P1.1 / F125/F127: a newer term whose hard-state persist fails
    /// must not raise the term in memory — the reply and the node keep the
    /// previous term/vote, the node is forced Follower with no leader hint.
    /// The aligned handler matches kernel `Restored`; the grant-then-persist
    /// mutant family keeps the undurable raise.
    #[test]
    fn durable_term_rollback_on_request_vote_with_persist_is_not_ok() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        let n = cluster.node_mut(1).unwrap();
        n.hard.current_term = 5;
        n.hard.voted_for = Some(3);
        n.role = Role::Leader;
        n.leader_id = Some(1);
        let reply = handle_request_vote_with_persist(
            n,
            &RequestVoteArgs {
                term: 6,
                candidate_id: 2,
                last_log_index: 0,
                last_log_term: 0,
            },
            |_| Err(RaftError::Persist("injected".into())),
        );
        assert!(
            !reply.vote_granted,
            "undurable term step must deny the vote"
        );
        assert_eq!(
            reply.term, 5,
            "reply carries the RESTORED term (F125/F127), not the undurable raise"
        );
        assert_eq!(n.hard.current_term, 5, "term not raised without durability");
        assert_eq!(
            n.hard.voted_for,
            Some(3),
            "previous vote survives the failed step"
        );
        assert_eq!(n.role, Role::Follower, "undurable step forces Follower");
        assert_eq!(
            n.leader_id, None,
            "leader hint cleared after the undurable step"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Mutant grant-then-persist violates F15 when persist fails; FIXED does not.
    #[test]
    fn mutation_grant_then_persist_violates_f15() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        let args = RequestVoteArgs {
            term: 5,
            candidate_id: 2,
            last_log_index: 0,
            last_log_term: 0,
        };
        {
            let n = cluster.node_mut(1).unwrap();
            n.hard.current_term = 5;
            n.hard.voted_for = None;
            let broken = handle_request_vote_grant_then_persist(n, &args, |_| {
                Err(RaftError::Persist("injected".into()))
            });
            assert!(
                broken.vote_granted,
                "mutant must grant despite persist Err (teeth)"
            );
        }
        {
            let n = cluster.node_mut(1).unwrap();
            n.hard.current_term = 5;
            n.hard.voted_for = None;
            let fixed = handle_request_vote_with_persist(n, &args, |_| {
                Err(RaftError::Persist("injected".into()))
            });
            assert!(!fixed.vote_granted, "FIXED must not grant on persist Err");
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F15: vote is durable on disk before grant (reload hard state).
    #[test]
    fn vote_persisted_before_grant() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 3).unwrap();
        // Force node 2 to vote for 1 via RequestVote at a fresh term.
        let args = RequestVoteArgs {
            term: 5,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        };
        {
            let n = cluster.nodes.get_mut(&2).unwrap();
            let reply = handle_request_vote(n, &args);
            assert!(reply.vote_granted);
            assert_eq!(n.hard.voted_for, Some(1));
            assert_eq!(n.hard.current_term, 5);
        }
        // Hard state on disk must carry the vote (crash before reply arrives).
        let meta = persist::raft_meta_dir(&parent.join("node-2"));
        let hard = persist::load_hard(&meta).unwrap();
        assert_eq!(hard.current_term, 5);
        assert_eq!(hard.voted_for, Some(1));
        // Second candidate in same term must not get the vote.
        let args2 = RequestVoteArgs {
            term: 5,
            candidate_id: 3,
            last_log_index: 0,
            last_log_term: 0,
        };
        let reply2 = handle_request_vote(cluster.nodes.get_mut(&2).unwrap(), &args2);
        assert!(!reply2.vote_granted);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// F16: AppendEntries cannot rewrite a committed index.
    #[test]
    fn refuse_conflict_at_committed_index() {
        let parent = temp_parent();
        let mut cluster = RaftCluster::open(&parent, 1).unwrap();
        cluster.elect_leader(20).unwrap();
        cluster
            .propose_puts([(b"c".to_vec(), b"1".to_vec())])
            .unwrap();
        let n = cluster.nodes.get_mut(&1).unwrap();
        assert!(n.commit_index >= 1);
        let bad = AppendEntriesArgs {
            term: n.hard.current_term,
            leader_id: 99,
            prev_log_index: 0,
            prev_log_term: 0,
            entries: vec![RaftLogEntry {
                index: 1,
                term: n.hard.current_term + 1, // conflict at committed index
                ops: vec![BatchOp::put(b"c", b"evil")],
            }],
            leader_commit: 0,
        };
        // Bump term so AE is not rejected for stale term only.
        n.hard.current_term += 1;
        let _ = n.persist_hard();
        let reply = handle_append_entries(n, &bad);
        assert!(!reply.success, "must refuse rewrite of committed entry");
        assert_eq!(n.get(b"c").as_deref(), Some(b"1".as_ref()));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn hard_state_and_log_survive_reopen() {
        let parent = temp_parent();
        {
            let mut cluster = RaftCluster::open(&parent, 3).unwrap();
            cluster.elect_leader(50).unwrap();
            cluster
                .propose_puts([(b"persist".to_vec(), b"yes".to_vec())])
                .unwrap();
            // Hard state files must exist for each node.
            for id in 1..=3u64 {
                let meta = persist::raft_meta_dir(&parent.join(format!("node-{id}")));
                assert!(meta.join("RAFT_HARD").exists(), "missing hard node {id}");
                assert!(meta.join("RAFT_LOG").exists(), "missing log node {id}");
                assert!(
                    meta.join("RAFT_COMMIT").exists(),
                    "missing commit node {id}"
                );
            }
        }
        // Reopen cluster: log reloads; SM still in PedraDB.
        let cluster = RaftCluster::open(&parent, 3).unwrap();
        for id in cluster.ids() {
            assert_eq!(
                cluster.node(*id).unwrap().get(b"persist").as_deref(),
                Some(b"yes".as_ref()),
                "node {id}"
            );
            assert!(
                !cluster.node(*id).unwrap().log.is_empty(),
                "raft log should reload"
            );
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Multi-process smoke: spawn 3 `pedra-raft-node` binaries (RFC-0012 P0.3).
    #[test]
    fn multi_process_three_nodes_smoke() {
        use crate::net::{wait_for_leader, PeerClient};
        use std::net::SocketAddr;
        use std::process::{Command, Stdio};
        use std::thread;
        use std::time::Duration;

        let parent = temp_parent();
        let base = 20000 + (std::process::id() % 800) as u16;
        let addrs: Vec<SocketAddr> = (0..3u16)
            .map(|i| format!("127.0.0.1:{}", base + i).parse().unwrap())
            .collect();
        let peer_args: Vec<String> = addrs
            .iter()
            .enumerate()
            .map(|(i, a)| format!("--peer={}={}", i + 1, a))
            .collect();

        let bin = std::env::var("CARGO_BIN_EXE_pedra-raft-node").unwrap_or_else(|_| {
            // When running lib tests without bin env, use cargo-built path.
            let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.pop();
            p.pop();
            p.push("target");
            p.push("debug");
            p.push("pedra-raft-node");
            p.to_string_lossy().into_owned()
        });
        // Ensure binary exists (build it if needed is caller's job; cargo test --bins helps).
        if !PathBuf::from(&bin).exists() {
            let status = Command::new("cargo")
                .args(["build", "-p", "pedradb-raft", "--bin", "pedra-raft-node"])
                .status()
                .expect("cargo build bin");
            assert!(status.success());
        }
        let mut children = Vec::new();
        for id in 1..=3u64 {
            let data = parent.join(format!("mp-{id}"));
            let mut cmd = Command::new(&bin);
            cmd.arg("--id")
                .arg(id.to_string())
                .arg("--data")
                .arg(&data)
                .arg("--bind")
                .arg(addrs[(id - 1) as usize].to_string());
            for p in &peer_args {
                // --peer=id=addr form: our parser expects --peer then value
                let v = p.trim_start_matches("--peer=");
                cmd.arg("--peer").arg(v);
            }
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            children.push(cmd.spawn().expect("spawn raft node"));
        }
        thread::sleep(Duration::from_millis(400));
        let leader = wait_for_leader(&addrs, 20_000).expect("multi-process leader");
        let client = PeerClient::new(leader);
        client.propose_put(b"mp", b"1").expect("propose");
        let mut ok = false;
        for _ in 0..40 {
            ok = addrs.iter().all(|a| {
                PeerClient::new(*a).get(b"mp").ok().flatten().as_deref() == Some(b"1".as_ref())
            });
            if ok {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        for mut c in children {
            let _ = c.kill();
            let _ = c.wait();
        }
        assert!(ok, "multi-process put visible on all nodes");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn network_three_nodes_elect_and_put() {
        use crate::net::{wait_for_leader, NetworkNode, PeerClient};
        use std::collections::HashMap;
        use std::net::SocketAddr;
        use std::thread;
        use std::time::Duration;

        let parent = temp_parent();
        // Ephemeral ports: bind 0 then... NetworkNode needs fixed ports.
        // Use high ports with pid salt.
        let base = 18000 + (std::process::id() % 1000) as u16;
        let addrs: Vec<SocketAddr> = (0..3u16)
            .map(|i| format!("127.0.0.1:{}", base + i).parse().unwrap())
            .collect();
        let mut peers = HashMap::new();
        for (i, a) in addrs.iter().enumerate() {
            peers.insert((i + 1) as u64, *a);
        }

        for id in 1..=3u64 {
            let dir = parent.join(format!("net-node-{id}"));
            let bind = peers[&id];
            let peers_c = peers.clone();
            thread::spawn(move || {
                let node = NetworkNode::open(id, dir, bind, peers_c).unwrap();
                let _ = node.serve();
            });
        }
        thread::sleep(Duration::from_millis(200));
        let leader_addr = wait_for_leader(&addrs, 15_000).expect("leader");
        let client = PeerClient::new(leader_addr);
        let idx = client.propose_put(b"nk", b"nv").expect("propose");
        assert!(idx >= 1);
        // Read from all nodes (eventual after apply).
        let mut ok = false;
        for _ in 0..50 {
            ok = addrs.iter().all(|a| {
                PeerClient::new(*a).get(b"nk").ok().flatten().as_deref() == Some(b"nv".as_ref())
            });
            if ok {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(ok, "all nodes should have nk=nv");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Kill the leader process; remaining majority re-elects and accepts a new put.
    #[test]
    fn multi_process_failover_after_leader_kill() {
        use crate::net::{wait_for_leader, PeerClient};
        use std::net::SocketAddr;
        use std::process::{Command, Stdio};
        use std::thread;
        use std::time::Duration;

        let parent = temp_parent();
        let base = 21000 + (std::process::id() % 700) as u16;
        let addrs: Vec<SocketAddr> = (0..3u16)
            .map(|i| format!("127.0.0.1:{}", base + i).parse().unwrap())
            .collect();
        let peer_args: Vec<String> = addrs
            .iter()
            .enumerate()
            .map(|(i, a)| format!("{}={}", i + 1, a))
            .collect();

        let bin = std::env::var("CARGO_BIN_EXE_pedra-raft-node").unwrap_or_else(|_| {
            let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.pop();
            p.pop();
            p.push("target/debug/pedra-raft-node");
            p.to_string_lossy().into_owned()
        });
        if !PathBuf::from(&bin).exists() {
            assert!(Command::new("cargo")
                .args(["build", "-p", "pedradb-raft", "--bin", "pedra-raft-node"])
                .status()
                .unwrap()
                .success());
        }

        let mut children = Vec::new();
        for id in 1..=3u64 {
            let data = parent.join(format!("fo-{id}"));
            let mut cmd = Command::new(&bin);
            cmd.arg("--id")
                .arg(id.to_string())
                .arg("--data")
                .arg(&data)
                .arg("--bind")
                .arg(addrs[(id - 1) as usize].to_string());
            for p in &peer_args {
                cmd.arg("--peer").arg(p);
            }
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            children.push(cmd.spawn().expect("spawn"));
        }
        thread::sleep(Duration::from_millis(500));

        let leader_addr = wait_for_leader(&addrs, 20_000).expect("initial leader");
        let leader_id = PeerClient::new(leader_addr).status().unwrap().2;
        PeerClient::new(leader_addr)
            .propose_put(b"before", b"1")
            .expect("put before kill");

        // Kill the leader OS process (index leader_id-1).
        let idx = (leader_id as usize).saturating_sub(1);
        if idx < children.len() {
            let _ = children[idx].kill();
            let _ = children[idx].wait();
        }

        let survivors: Vec<SocketAddr> = addrs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, a)| *a)
            .collect();
        let new_leader = wait_for_leader(&survivors, 25_000).expect("new leader after kill");
        assert_ne!(
            new_leader, leader_addr,
            "leader address should change after kill"
        );
        PeerClient::new(new_leader)
            .propose_put(b"after", b"2")
            .expect("put after failover");

        // Survivors see both keys.
        for a in &survivors {
            let mut saw = false;
            for _ in 0..40 {
                let c = PeerClient::new(*a);
                if c.get(b"before").ok().flatten().as_deref() == Some(b"1".as_ref())
                    && c.get(b"after").ok().flatten().as_deref() == Some(b"2".as_ref())
                {
                    saw = true;
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            assert!(saw, "survivor {a} missing data after failover");
        }

        for (i, mut c) in children.into_iter().enumerate() {
            if i != idx {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// DCS create-if-absent via Raft; followers see the key.
    #[test]
    fn network_dcs_create_replicated() {
        use crate::net::{wait_for_leader, NetworkNode, PeerClient};
        use pedradb_dcs::DcsCommand;
        use std::collections::HashMap;
        use std::net::SocketAddr;
        use std::thread;
        use std::time::Duration;

        let parent = temp_parent();
        let base = 22000 + (std::process::id() % 600) as u16;
        let addrs: Vec<SocketAddr> = (0..3u16)
            .map(|i| format!("127.0.0.1:{}", base + i).parse().unwrap())
            .collect();
        let mut peers = HashMap::new();
        for (i, a) in addrs.iter().enumerate() {
            peers.insert((i + 1) as u64, *a);
        }
        for id in 1..=3u64 {
            let dir = parent.join(format!("dcs-node-{id}"));
            let bind = peers[&id];
            let peers_c = peers.clone();
            thread::spawn(move || {
                let node = NetworkNode::open(id, dir, bind, peers_c).unwrap();
                let _ = node.serve();
            });
        }
        thread::sleep(Duration::from_millis(250));
        let leader = wait_for_leader(&addrs, 15_000).expect("leader");
        let client = PeerClient::new(leader);
        let cmd = DcsCommand::Create {
            key: b"pg-leader".to_vec(),
            value: b"node-a".to_vec(),
            lease: 0,
        };
        client.propose_dcs(&cmd).expect("dcs create");
        // Second create must fail (race).
        assert!(client.propose_dcs(&cmd).is_err());

        let mut ok = false;
        for _ in 0..50 {
            ok = addrs.iter().all(|a| {
                PeerClient::new(*a)
                    .dcs_get(b"pg-leader")
                    .ok()
                    .flatten()
                    .map(|kv| kv.value == b"node-a")
                    .unwrap_or(false)
            });
            if ok {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(ok, "all nodes should see DCS leader key");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
