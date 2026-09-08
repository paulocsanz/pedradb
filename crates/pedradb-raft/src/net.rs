//! TCP transport for Raft RPCs (RFC-0012 P0.1).
//!
//! Framing: `u32 LE length` + body. Body:
//! - tag `1` = RequestVote
//! - tag `2` = AppendEntries
//! - tag `3` = ClientPropose (puts only, to leader)
//! - tag `4` = ClientGet
//! - tag `5` = Status
//! - tag `6` = DcsCommand propose
//! - tag `7` = DcsGet
//! - tag `0x80` = Auth (shared-secret handshake; first frame when auth enabled)
//!
//! Replies: same framing with type-specific payloads.
//!
//! # Auth
//! When [`NetworkNode`] / [`PeerClient`] are configured with a non-empty shared
//! secret, each TCP connection must begin with an auth frame (`0x80` + secret).
//! Empty secret = open bind (lab only; **do not** expose publicly).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pedradb_core::{BatchOp, Db, OpenOptions};

use crate::persist;
use crate::{
    rpc_append_entries, rpc_request_vote, AppendEntriesArgs, AppendEntriesReply, RaftError,
    RaftLogEntry, RaftNode, RequestVoteArgs, RequestVoteReply, Result,
};

fn io_net(e: std::io::Error) -> RaftError {
    RaftError::Network(e.to_string())
}

fn write_frame(stream: &mut impl Write, body: &[u8]) -> Result<()> {
    let len = body.len() as u32;
    stream.write_all(&len.to_le_bytes()).map_err(io_net)?;
    stream.write_all(body).map_err(io_net)?;
    stream.flush().map_err(io_net)?;
    Ok(())
}

fn read_frame(stream: &mut impl Read) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).map_err(io_net)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(RaftError::Network("frame too large".into()));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).map_err(io_net)?;
    Ok(body)
}

fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_bytes(b: &mut Vec<u8>, d: &[u8]) {
    put_u32(b, d.len() as u32);
    b.extend_from_slice(d);
}

fn take_u64(buf: &[u8], off: &mut usize) -> Result<u64> {
    if *off + 8 > buf.len() {
        return Err(RaftError::Network("eof u64".into()));
    }
    let v = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}
fn take_u32(buf: &[u8], off: &mut usize) -> Result<u32> {
    if *off + 4 > buf.len() {
        return Err(RaftError::Network("eof u32".into()));
    }
    let v = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(v)
}
fn take_bytes(buf: &[u8], off: &mut usize) -> Result<Vec<u8>> {
    let n = take_u32(buf, off)? as usize;
    if *off + n > buf.len() {
        return Err(RaftError::Network("eof bytes".into()));
    }
    let v = buf[*off..*off + n].to_vec();
    *off += n;
    Ok(v)
}

fn encode_rv(a: &RequestVoteArgs) -> Vec<u8> {
    let mut b = vec![1u8];
    put_u64(&mut b, a.term);
    put_u64(&mut b, a.candidate_id);
    put_u64(&mut b, a.last_log_index);
    put_u64(&mut b, a.last_log_term);
    b
}

fn decode_rv(buf: &[u8], off: &mut usize) -> Result<RequestVoteArgs> {
    Ok(RequestVoteArgs {
        term: take_u64(buf, off)?,
        candidate_id: take_u64(buf, off)?,
        last_log_index: take_u64(buf, off)?,
        last_log_term: take_u64(buf, off)?,
    })
}

fn encode_rv_reply(r: &RequestVoteReply) -> Vec<u8> {
    let mut b = Vec::new();
    put_u64(&mut b, r.term);
    b.push(u8::from(r.vote_granted));
    b
}

fn decode_rv_reply2(buf: &[u8]) -> Result<RequestVoteReply> {
    if buf.len() < 9 {
        return Err(RaftError::Network("rv reply short".into()));
    }
    Ok(RequestVoteReply {
        term: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
        vote_granted: buf[8] != 0,
    })
}

fn encode_op(b: &mut Vec<u8>, op: &BatchOp) {
    match op {
        BatchOp::Put { key, value } => {
            b.push(1);
            put_bytes(b, key);
            put_bytes(b, value);
        }
        BatchOp::Delete { key } => {
            b.push(2);
            put_bytes(b, key);
        }
        BatchOp::DeleteRange { start, end } => {
            b.push(3);
            put_bytes(b, start);
            put_bytes(b, end);
        }
    }
}

fn decode_op(buf: &[u8], off: &mut usize) -> Result<BatchOp> {
    let tag = {
        if *off >= buf.len() {
            return Err(RaftError::Network("op tag".into()));
        }
        let t = buf[*off];
        *off += 1;
        t
    };
    match tag {
        1 => Ok(BatchOp::Put {
            key: bytes::Bytes::from(take_bytes(buf, off)?),
            value: bytes::Bytes::from(take_bytes(buf, off)?),
        }),
        2 => Ok(BatchOp::Delete {
            key: bytes::Bytes::from(take_bytes(buf, off)?),
        }),
        3 => Ok(BatchOp::DeleteRange {
            start: bytes::Bytes::from(take_bytes(buf, off)?),
            end: bytes::Bytes::from(take_bytes(buf, off)?),
        }),
        _ => Err(RaftError::Network("bad op".into())),
    }
}

fn encode_ae(a: &AppendEntriesArgs) -> Vec<u8> {
    let mut b = vec![2u8];
    put_u64(&mut b, a.term);
    put_u64(&mut b, a.leader_id);
    put_u64(&mut b, a.prev_log_index);
    put_u64(&mut b, a.prev_log_term);
    put_u64(&mut b, a.leader_commit);
    put_u32(&mut b, a.entries.len() as u32);
    for e in &a.entries {
        put_u64(&mut b, e.index);
        put_u64(&mut b, e.term);
        put_u32(&mut b, e.ops.len() as u32);
        for op in &e.ops {
            encode_op(&mut b, op);
        }
    }
    b
}

fn decode_ae(buf: &[u8], off: &mut usize) -> Result<AppendEntriesArgs> {
    let term = take_u64(buf, off)?;
    let leader_id = take_u64(buf, off)?;
    let prev_log_index = take_u64(buf, off)?;
    let prev_log_term = take_u64(buf, off)?;
    let leader_commit = take_u64(buf, off)?;
    let n_raw = take_u32(buf, off)?;
    // F9 (network path): untrusted entry count must not with_capacity multi-GiB
    // inside a ≤16MiB frame (same class as persist decode_log).
    const MIN_ENTRY: usize = 8 + 8 + 4;
    const MIN_OP: usize = 1 + 4;
    let rem = buf.len().saturating_sub(*off);
    let max_n = rem / MIN_ENTRY;
    if n_raw as usize > max_n {
        return Err(RaftError::Network(format!(
            "ae entry count {n_raw} exceeds remaining {rem}"
        )));
    }
    let n = n_raw as usize;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        let index = take_u64(buf, off)?;
        let eterm = take_u64(buf, off)?;
        let nop_raw = take_u32(buf, off)?;
        let rem_ops = buf.len().saturating_sub(*off);
        if nop_raw as usize > rem_ops / MIN_OP {
            return Err(RaftError::Network(format!(
                "ae op count {nop_raw} exceeds remaining {rem_ops}"
            )));
        }
        let nop = nop_raw as usize;
        let mut ops = Vec::with_capacity(nop);
        for _ in 0..nop {
            ops.push(decode_op(buf, off)?);
        }
        entries.push(RaftLogEntry {
            index,
            term: eterm,
            ops,
        });
    }
    Ok(AppendEntriesArgs {
        term,
        leader_id,
        prev_log_index,
        prev_log_term,
        entries,
        leader_commit,
    })
}

fn encode_ae_reply(r: &AppendEntriesReply) -> Vec<u8> {
    let mut b = Vec::new();
    put_u64(&mut b, r.term);
    b.push(u8::from(r.success));
    put_u64(&mut b, r.match_index);
    b
}

fn decode_ae_reply(buf: &[u8]) -> Result<AppendEntriesReply> {
    if buf.len() < 17 {
        return Err(RaftError::Network("ae reply short".into()));
    }
    Ok(AppendEntriesReply {
        term: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
        success: buf[8] != 0,
        match_index: u64::from_le_bytes(buf[9..17].try_into().unwrap()),
    })
}

/// Auth frame tag (connection preamble).
const AUTH_TAG: u8 = 0x80;

fn write_auth(stream: &mut impl Write, secret: &[u8]) -> Result<()> {
    let mut b = vec![AUTH_TAG];
    put_bytes(&mut b, secret);
    write_frame(stream, &b)
}

fn expect_auth(stream: &mut impl Read, secret: &[u8]) -> Result<()> {
    let body = read_frame(stream)?;
    if body.first().copied() != Some(AUTH_TAG) {
        return Err(RaftError::Network("auth required".into()));
    }
    let mut off = 1usize;
    let got = take_bytes(&body, &mut off)?;
    if got.as_slice() != secret {
        return Err(RaftError::Network("auth rejected".into()));
    }
    Ok(())
}

/// RPC client to one peer.
pub struct PeerClient {
    addr: SocketAddr,
    /// Shared secret; empty = no auth handshake.
    auth: Vec<u8>,
}

impl PeerClient {
    /// Peer at `addr` (no auth).
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            auth: Vec::new(),
        }
    }

    /// Peer with shared-secret auth (must match server).
    #[must_use]
    pub fn with_auth(addr: SocketAddr, secret: impl AsRef<[u8]>) -> Self {
        Self {
            addr,
            auth: secret.as_ref().to_vec(),
        }
    }

    fn connect(&self) -> Result<TcpStream> {
        let mut s =
            TcpStream::connect_timeout(&self.addr, Duration::from_secs(2)).map_err(io_net)?;
        if !self.auth.is_empty() {
            write_auth(&mut s, &self.auth)?;
        }
        Ok(s)
    }

    /// RequestVote.
    ///
    /// # Errors
    /// Network.
    pub fn request_vote(&self, args: &RequestVoteArgs) -> Result<RequestVoteReply> {
        let mut s = self.connect()?;
        write_frame(&mut s, &encode_rv(args))?;
        let body = read_frame(&mut s)?;
        decode_rv_reply2(&body)
    }

    /// AppendEntries.
    ///
    /// # Errors
    /// Network.
    pub fn append_entries(&self, args: &AppendEntriesArgs) -> Result<AppendEntriesReply> {
        let mut s = self.connect()?;
        write_frame(&mut s, &encode_ae(args))?;
        let body = read_frame(&mut s)?;
        decode_ae_reply(&body)
    }

    /// Client put (leader only).
    ///
    /// # Errors
    /// Network / not leader.
    pub fn propose_put(&self, key: &[u8], value: &[u8]) -> Result<u64> {
        let mut b = vec![3u8];
        put_bytes(&mut b, key);
        put_bytes(&mut b, value);
        let mut s = self.connect()?;
        write_frame(&mut s, &b)?;
        let body = read_frame(&mut s)?;
        if pedradb_core::write_admission_kernel::batch_is_empty(body.len() as u64) {
            return Err(RaftError::Network("empty propose reply".into()));
        }
        if body[0] == 0 {
            return Err(RaftError::NotLeader { leader_hint: None });
        }
        if body.len() < 9 {
            return Err(RaftError::Network("propose reply short".into()));
        }
        Ok(u64::from_le_bytes(body[1..9].try_into().unwrap()))
    }

    /// Propose a DCS command through the leader (replicated SM).
    ///
    /// # Errors
    /// Network / not leader / CAS reject.
    pub fn propose_dcs(&self, cmd: &pedradb_dcs::DcsCommand) -> Result<u64> {
        let payload = cmd.encode();
        let mut b = vec![6u8];
        put_bytes(&mut b, &payload);
        let mut s = self.connect()?;
        write_frame(&mut s, &b)?;
        let body = read_frame(&mut s)?;
        if pedradb_core::write_admission_kernel::batch_is_empty(body.len() as u64) {
            return Err(RaftError::Network("empty dcs reply".into()));
        }
        match body[0] {
            1 if body.len() >= 9 => Ok(u64::from_le_bytes(body[1..9].try_into().unwrap())),
            2 => Err(RaftError::NotLeader { leader_hint: None }),
            3 => Err(RaftError::Network(
                String::from_utf8_lossy(&body[1..]).into_owned(),
            )),
            _ => Err(RaftError::Network("bad dcs reply".into())),
        }
    }

    /// DCS get on this node (applied state).
    ///
    /// # Errors
    /// Network.
    pub fn dcs_get(&self, key: &[u8]) -> Result<Option<pedradb_dcs::KeyValue>> {
        let mut b = vec![7u8];
        put_bytes(&mut b, key);
        let mut s = self.connect()?;
        write_frame(&mut s, &b)?;
        let body = read_frame(&mut s)?;
        if pedradb_core::write_admission_kernel::batch_is_empty(body.len() as u64) || body[0] == 0 {
            return Ok(None);
        }
        let mut off = 1;
        let key = take_bytes(&body, &mut off)?;
        let value = take_bytes(&body, &mut off)?;
        let mod_revision = take_u64(&body, &mut off)?;
        let create_revision = take_u64(&body, &mut off)?;
        let lease = take_u64(&body, &mut off)?;
        Ok(Some(pedradb_dcs::KeyValue {
            key,
            value,
            mod_revision,
            create_revision,
            lease,
        }))
    }

    /// Client get.
    ///
    /// # Errors
    /// Network.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut b = vec![4u8];
        put_bytes(&mut b, key);
        let mut s = self.connect()?;
        write_frame(&mut s, &b)?;
        let body = read_frame(&mut s)?;
        if pedradb_core::write_admission_kernel::batch_is_empty(body.len() as u64) {
            return Err(RaftError::Network("empty get".into()));
        }
        if body[0] == 0 {
            return Ok(None);
        }
        let mut off = 1;
        Ok(Some(take_bytes(&body, &mut off)?))
    }

    /// Status: (is_leader, term, id).
    ///
    /// # Errors
    /// Network.
    pub fn status(&self) -> Result<(bool, u64, u64)> {
        let mut s = self.connect()?;
        write_frame(&mut s, &[5u8])?;
        let body = read_frame(&mut s)?;
        if body.len() < 17 {
            return Err(RaftError::Network("status short".into()));
        }
        Ok((
            body[0] != 0,
            u64::from_le_bytes(body[1..9].try_into().unwrap()),
            u64::from_le_bytes(body[9..17].try_into().unwrap()),
        ))
    }
}

/// Networked Raft node: TCP server + tick loop.
pub struct NetworkNode {
    node: Arc<Mutex<RaftNode>>,
    peers: HashMap<u64, SocketAddr>,
    peer_ids: Vec<u64>,
    bind: SocketAddr,
    /// Shared secret for peer/client connections; empty = open (lab only).
    auth: Vec<u8>,
}

impl NetworkNode {
    /// Open data dir, bind `bind`, map of peer id → address (including self).
    ///
    /// # Errors
    /// Open / bind.
    pub fn open(
        id: u64,
        data_dir: impl AsRef<Path>,
        bind: SocketAddr,
        peers: HashMap<u64, SocketAddr>,
    ) -> Result<Self> {
        Self::open_with_auth(id, data_dir, bind, peers, &[][..])
    }

    /// Like [`Self::open`] with a shared secret (required for public binds).
    ///
    /// # Errors
    /// Open / bind.
    pub fn open_with_auth(
        id: u64,
        data_dir: impl AsRef<Path>,
        bind: SocketAddr,
        peers: HashMap<u64, SocketAddr>,
        secret: impl AsRef<[u8]>,
    ) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).map_err(io_net)?;
        let db = Db::open_with(
            data_dir,
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
        let meta = persist::raft_meta_dir(data_dir);
        let (hard, log, commit) = RaftNode::load_meta(&meta)?;
        let mut node = RaftNode::new(id, db, 8 + id, 2);
        node.meta_dir = Some(meta);
        node.hard = hard;
        node.log = log;
        // F10: durable commit only; re-apply committed prefix.
        let log_last = node.log.last().map_or(0, |e| e.index);
        node.commit_index = crate::commit_kernel::recover_commit(commit, log_last);
        node.last_applied = crate::commit_kernel::recover_last_applied();
        node.apply_committed().map_err(RaftError::from)?;
        let peer_ids: Vec<u64> = peers.keys().copied().collect();
        Ok(Self {
            node: Arc::new(Mutex::new(node)),
            peers,
            peer_ids,
            bind,
            auth: secret.as_ref().to_vec(),
        })
    }

    /// Serve forever (blocks): accept + tick thread.
    ///
    /// # Errors
    /// Bind failure.
    pub fn serve(self) -> Result<()> {
        let listener = TcpListener::bind(self.bind).map_err(io_net)?;
        listener.set_nonblocking(false).map_err(io_net)?;

        let node = Arc::clone(&self.node);
        let peers = self.peers.clone();
        let peer_ids = self.peer_ids.clone();
        let auth = self.auth.clone();
        let id = {
            let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
            n.id()
        };

        // Ticker + election/heartbeat using network RPC.
        let tick_node = Arc::clone(&self.node);
        let tick_auth = auth.clone();
        let tick_peers = peers.clone();
        let tick_ids = peer_ids.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(50));
            let _ = network_tick(&tick_node, id, &tick_peers, &tick_ids, &tick_auth);
        });

        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let node = Arc::clone(&self.node);
            let peers = self.peers.clone();
            let peer_ids = self.peer_ids.clone();
            let auth = auth.clone();
            thread::spawn(move || {
                let _ = handle_conn(&mut stream, &node, id, &peers, &peer_ids, &auth);
            });
        }
        Ok(())
    }
}

fn handle_conn(
    stream: &mut TcpStream,
    node: &Arc<Mutex<RaftNode>>,
    self_id: u64,
    peers: &HashMap<u64, SocketAddr>,
    peer_ids: &[u64],
    auth: &[u8],
) -> Result<()> {
    if !auth.is_empty() {
        expect_auth(stream, auth)?;
    }
    let body = read_frame(stream)?;
    if body.is_empty() {
        return Ok(());
    }
    match body[0] {
        1 => {
            let mut off = 1;
            let args = decode_rv(&body, &mut off)?;
            let reply = {
                let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
                rpc_request_vote(&mut n, &args)
            };
            write_frame(stream, &encode_rv_reply(&reply))?;
        }
        2 => {
            let mut off = 1;
            let args = decode_ae(&body, &mut off)?;
            let reply = {
                let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
                rpc_append_entries(&mut n, &args)
            };
            write_frame(stream, &encode_ae_reply(&reply))?;
        }
        3 => {
            let mut off = 1;
            let key = take_bytes(&body, &mut off)?;
            let value = take_bytes(&body, &mut off)?;
            let res = network_propose(node, self_id, peers, peer_ids, auth, &key, &value);
            match res {
                Ok(idx) => {
                    let mut b = vec![1u8];
                    put_u64(&mut b, idx);
                    write_frame(stream, &b)?;
                }
                Err(_) => write_frame(stream, &[0u8])?,
            }
        }
        4 => {
            let mut off = 1;
            let key = take_bytes(&body, &mut off)?;
            let val = {
                let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
                n.get(&key).map(|b| b.to_vec())
            };
            match val {
                Some(v) => {
                    let mut b = vec![1u8];
                    put_bytes(&mut b, &v);
                    write_frame(stream, &b)?;
                }
                None => write_frame(stream, &[0u8])?,
            }
        }
        5 => {
            let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
            let mut b = vec![u8::from(n.is_leader())];
            put_u64(&mut b, n.term());
            put_u64(&mut b, n.id());
            write_frame(stream, &b)?;
        }
        6 => {
            let mut off = 1;
            let payload = take_bytes(&body, &mut off)?;
            let cmd = pedradb_dcs::DcsCommand::decode(&payload)
                .map_err(|e| RaftError::Network(e.to_string()))?;
            match network_propose_dcs(node, self_id, peers, peer_ids, auth, &cmd) {
                Ok(idx) => {
                    let mut b = vec![1u8];
                    put_u64(&mut b, idx);
                    write_frame(stream, &b)?;
                }
                Err(RaftError::NotLeader { .. }) => write_frame(stream, &[2u8])?,
                Err(e) => {
                    let msg = e.to_string();
                    let mut b = vec![3u8];
                    b.extend_from_slice(msg.as_bytes());
                    write_frame(stream, &b)?;
                }
            }
        }
        7 => {
            let mut off = 1;
            let key = take_bytes(&body, &mut off)?;
            let kv = {
                let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
                n.dcs_get(&key)
            };
            match kv {
                Some(kv) => {
                    let mut b = vec![1u8];
                    put_bytes(&mut b, &kv.key);
                    put_bytes(&mut b, &kv.value);
                    put_u64(&mut b, kv.mod_revision);
                    put_u64(&mut b, kv.create_revision);
                    put_u64(&mut b, kv.lease);
                    write_frame(stream, &b)?;
                }
                None => write_frame(stream, &[0u8])?,
            }
        }
        _ => {}
    }
    Ok(())
}

fn network_propose_dcs(
    node: &Arc<Mutex<RaftNode>>,
    self_id: u64,
    peers: &HashMap<u64, SocketAddr>,
    peer_ids: &[u64],
    auth: &[u8],
    cmd: &pedradb_dcs::DcsCommand,
) -> Result<u64> {
    let bound = {
        let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        if !n.is_leader() {
            return Err(RaftError::NotLeader {
                leader_hint: n.leader_id,
            });
        }
        pedradb_dcs::check_command(&n.db, cmd).map_err(|e| RaftError::Network(e.to_string()))?;
        // now_ms=0: this path is immortal-lease today; bind is a no-op unless a
        // corpse is already expired at 0. Same function as the store propose.
        pedradb_dcs::bind_absent_create(&n.db, cmd.clone(), 0)
    };
    let entry = {
        let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        let index = n.log.last().map_or(0, |e| e.index) + 1;
        let term = n.hard.current_term;
        let e = RaftLogEntry {
            index,
            term,
            ops: vec![BatchOp::put(pedradb_dcs::DCS_CMD_MARKER, bound.encode())],
        };
        n.log.push(e.clone());
        n.persist_log()?;
        e
    };
    network_broadcast_append(node, self_id, peers, peer_ids, auth, vec![entry.clone()])?;
    // F11: only ACK after majority commit.
    let commit = {
        let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        n.commit_index
    };
    if !crate::commit_kernel::propose_ack_ok(entry.index, commit) {
        return Err(RaftError::NotCommitted {
            index: entry.index,
            commit_index: commit,
        });
    }
    Ok(entry.index)
}

fn network_propose(
    node: &Arc<Mutex<RaftNode>>,
    self_id: u64,
    peers: &HashMap<u64, SocketAddr>,
    peer_ids: &[u64],
    auth: &[u8],
    key: &[u8],
    value: &[u8],
) -> Result<u64> {
    {
        let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        if !n.is_leader() {
            return Err(RaftError::NotLeader {
                leader_hint: n.leader_id,
            });
        }
    }
    let entry = {
        let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        let index = n.log.last().map_or(0, |e| e.index) + 1;
        let term = n.hard.current_term;
        let e = RaftLogEntry {
            index,
            term,
            ops: vec![BatchOp::put(key, value)],
        };
        n.log.push(e.clone());
        n.persist_log()?;
        e
    };
    network_broadcast_append(node, self_id, peers, peer_ids, auth, vec![entry.clone()])?;
    // F11: only ACK after majority commit.
    let commit = {
        let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        n.commit_index
    };
    if !crate::commit_kernel::propose_ack_ok(entry.index, commit) {
        return Err(RaftError::NotCommitted {
            index: entry.index,
            commit_index: commit,
        });
    }
    Ok(entry.index)
}

fn network_tick(
    node: &Arc<Mutex<RaftNode>>,
    self_id: u64,
    peers: &HashMap<u64, SocketAddr>,
    peer_ids: &[u64],
    auth: &[u8],
) -> Result<()> {
    let action = {
        let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        if n.is_leader() {
            if n.heartbeat_ticks_left == 0 {
                n.heartbeat_ticks_left = n.heartbeat_every;
                Some(1u8) // heartbeat
            } else {
                n.heartbeat_ticks_left -= 1;
                None
            }
        } else if n.election_ticks_left == 0 {
            Some(2u8) // election
        } else {
            n.election_ticks_left -= 1;
            None
        }
    };
    match action {
        Some(1) => network_broadcast_append(node, self_id, peers, peer_ids, auth, Vec::new()),
        Some(2) => network_start_election(node, self_id, peers, peer_ids, auth),
        _ => Ok(()),
    }
}

fn network_start_election(
    node: &Arc<Mutex<RaftNode>>,
    self_id: u64,
    peers: &HashMap<u64, SocketAddr>,
    peer_ids: &[u64],
    auth: &[u8],
) -> Result<()> {
    let args = {
        let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        // F15: durable self-vote before RequestVote RPCs.
        let prev_term = n.hard.current_term;
        let prev_vote = n.hard.voted_for;
        let prev_role = n.role;
        n.hard.current_term += 1;
        n.role = crate::Role::Candidate;
        n.hard.voted_for = Some(self_id);
        n.election_ticks_left = n.election_timeout;
        if let Err(e) = n.persist_hard() {
            n.hard.current_term = prev_term;
            n.hard.voted_for = prev_vote;
            n.role = prev_role;
            return Err(e);
        }
        RequestVoteArgs {
            term: n.hard.current_term,
            candidate_id: self_id,
            last_log_index: n.log.last().map_or(0, |e| e.index),
            last_log_term: n.log.last().map_or(0, |e| e.term),
        }
    };

    let mut votes = 1u64;
    let majority = peer_ids.len() as u64 / 2 + 1;
    for &pid in peer_ids {
        if pid == self_id {
            continue;
        }
        let Some(addr) = peers.get(&pid) else {
            continue;
        };
        let client = if auth.is_empty() {
            PeerClient::new(*addr)
        } else {
            PeerClient::with_auth(*addr, auth)
        };
        if let Ok(reply) = client.request_vote(&args) {
            if reply.term > args.term {
                let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
                n.become_follower(reply.term);
                return Ok(());
            }
            if reply.vote_granted {
                votes += 1;
            }
        }
    }
    if votes >= majority {
        {
            let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
            if n.hard.current_term == args.term {
                n.become_leader(peer_ids)?;
            }
        }
        // F18: replicate leader noop and advance commit (single-node critical).
        network_broadcast_append(node, self_id, peers, peer_ids, auth, Vec::new())?;
    }
    Ok(())
}

fn network_broadcast_append(
    node: &Arc<Mutex<RaftNode>>,
    self_id: u64,
    peers: &HashMap<u64, SocketAddr>,
    peer_ids: &[u64],
    auth: &[u8],
    new_entries: Vec<RaftLogEntry>,
) -> Result<()> {
    // Attach new entries already on leader log if any; broadcast current tail.
    let (term, commit, last_idx, snapshot_log) = {
        let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        if !n.is_leader() && !new_entries.is_empty() {
            return Err(RaftError::NotLeader {
                leader_hint: n.leader_id,
            });
        }
        (
            n.hard.current_term,
            n.commit_index,
            n.log.last().map_or(0, |e| e.index),
            n.log.clone(),
        )
    };
    let _ = new_entries;

    for &pid in peer_ids {
        if pid == self_id {
            continue;
        }
        let Some(addr) = peers.get(&pid) else {
            continue;
        };
        let next = {
            let n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
            *n.next_index.get(&pid).unwrap_or(&(last_idx + 1))
        };
        let prev_idx = next.saturating_sub(1);
        let prev_term = snapshot_log
            .iter()
            .find(|e| e.index == prev_idx)
            .map_or(0, |e| e.term);
        let entries: Vec<RaftLogEntry> = snapshot_log
            .iter()
            .filter(|e| e.index >= next)
            .cloned()
            .collect();
        let args = AppendEntriesArgs {
            term,
            leader_id: self_id,
            prev_log_index: prev_idx,
            prev_log_term: prev_term,
            entries,
            leader_commit: commit,
        };
        let client = if auth.is_empty() {
            PeerClient::new(*addr)
        } else {
            PeerClient::with_auth(*addr, auth)
        };
        if let Ok(reply) = client.append_entries(&args) {
            let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
            if reply.term > term {
                n.become_follower(reply.term);
                return Ok(());
            }
            if reply.success {
                n.next_index.insert(pid, reply.match_index + 1);
                n.match_index.insert(pid, reply.match_index);
            } else {
                let ni = n.next_index.get(&pid).copied().unwrap_or(1);
                n.next_index.insert(pid, ni.saturating_sub(1).max(1));
            }
        }
    }

    // Update commit index (majority of match_index).
    {
        let mut n = node.lock().map_err(|e| RaftError::Network(e.to_string()))?;
        if !n.is_leader() {
            return Ok(());
        }
        let majority = peer_ids.len() / 2 + 1;
        let last = n.log.last().map_or(0, |e| e.index);
        for cand in (1..=last).rev() {
            let count = peer_ids
                .iter()
                .filter(|&&pid| {
                    if pid == self_id {
                        true
                    } else {
                        n.match_index.get(&pid).copied().unwrap_or(0) >= cand
                    }
                })
                .count();
            if count >= majority {
                let term_at = n.log.iter().find(|e| e.index == cand).map_or(0, |e| e.term);
                if term_at == n.hard.current_term && cand > n.commit_index {
                    let _ = n.set_commit_and_apply(cand);
                }
                break;
            }
        }
    }
    Ok(())
}

/// Wait until some peer reports leader (for tests).
///
/// # Errors
/// Timeout.
pub fn wait_for_leader(addrs: &[SocketAddr], max_ms: u64) -> Result<SocketAddr> {
    wait_for_leader_auth(addrs, max_ms, &[])
}

/// Like [`wait_for_leader`] when the cluster requires a shared secret.
///
/// # Errors
/// Timeout.
pub fn wait_for_leader_auth(
    addrs: &[SocketAddr],
    max_ms: u64,
    secret: &[u8],
) -> Result<SocketAddr> {
    let steps = max_ms / 50;
    for _ in 0..steps {
        for &a in addrs {
            let c = if secret.is_empty() {
                PeerClient::new(a)
            } else {
                PeerClient::with_auth(a, secret)
            };
            if let Ok((is_leader, _, _)) = c.status() {
                if is_leader {
                    return Ok(a);
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(RaftError::NotLeader { leader_hint: None })
}

/// Helper for tests: data dir path.
#[must_use]
pub fn node_data_dir(parent: impl AsRef<Path>, id: u64) -> PathBuf {
    parent.as_ref().join(format!("net-node-{id}"))
}

#[cfg(test)]
mod auth_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn auth_frame_roundtrip() {
        let mut buf = Vec::new();
        write_auth(&mut buf, b"s3cret").unwrap();
        let mut cur = Cursor::new(buf);
        expect_auth(&mut cur, b"s3cret").unwrap();
        assert!(expect_auth(&mut Cursor::new(vec![]), b"s3cret").is_err());
    }

    #[test]
    fn auth_rejects_wrong_secret() {
        let mut buf = Vec::new();
        write_auth(&mut buf, b"good").unwrap();
        let mut cur = Cursor::new(buf);
        assert!(expect_auth(&mut cur, b"bad").is_err());
    }
}
