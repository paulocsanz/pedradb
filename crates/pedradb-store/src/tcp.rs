//! Length-prefixed TCP wire for multi-host Montanha (RFC-0017 P0.1).
//!
//! Frame: magic `MTCP` + u32 LE body length + body.
//! Body:
//! - tag 1 Peer: from_u64, to_u64, peer_msg_bytes
//! - tag 2 Put: key, value
//! - tag 3 Get: key
//! - tag 4 RespOk
//! - tag 5 RespErr: message
//! - tag 6 RespValue: optional value (0 empty / 1 + bytes)
//! - tag 7 Tick (admin: one raft tick)
//! - tag 8 Status (admin: request leader map)
//! - tag 9 StatusResp: text summary
//! - tag 10 SetPeers: u32 count + repeated (id u64, host_port bytes) — runtime mesh rewire
//! - tag 11 CommitTx: u32 n + repeated (key, value) — atomic multi-key TX (RFC-0021)
//! - tag 12 RespTxn: u64 txn_id
//! - tag 13 DcsCreate: key, value — etcd-need create-if-absent (returns RespRev)
//! - tag 14 DcsCas: key, value, expected_rev
//! - tag 15 DcsGet: key — returns RespValue (user bytes) or empty
//! - tag 16 RespRev: u64 mod_revision
//! - tag 17 PutBatch: u32 n + repeated (key, value) — range-grouped put_many (RFC-0025 P1.3)

use crate::msg::PeerMsg;
use crate::{validate_tx_pairs, Result, StoreError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

const MAGIC: &[u8; 4] = b"MTCP";

/// Application-level wire message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireMsg {
    /// Raft peer RPC (opaque PeerMsg payload).
    Peer {
        /// Sender node id.
        from: u64,
        /// Destination node id.
        to: u64,
        /// Encoded [`PeerMsg`].
        body: Vec<u8>,
    },
    /// Client put.
    Put {
        /// Key.
        key: Vec<u8>,
        /// Value.
        value: Vec<u8>,
    },
    /// Client get.
    Get {
        /// Key.
        key: Vec<u8>,
    },
    /// Success with no payload.
    RespOk,
    /// Error string.
    RespErr {
        /// Message.
        message: String,
    },
    /// Optional value.
    RespValue {
        /// None = missing key.
        value: Option<Vec<u8>>,
    },
    /// Request one raft tick on the server.
    Tick,
    /// Request status string.
    Status,
    /// Status text.
    StatusResp {
        /// Human-readable status.
        text: String,
    },
    /// Replace peer address map (id → host:port) without process restart.
    SetPeers {
        /// Full membership addresses (must include all voters).
        peers: Vec<(u64, String)>,
    },
    /// Atomic multi-key TX commit (server runs `commit_tx`).
    CommitTx {
        /// Ordered key/value pairs.
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// Successful TX id.
    RespTxn {
        /// Transaction id assigned by the store.
        txn_id: u64,
    },
    /// etcd-need DCS create-if-absent (full key, typically `m/...`).
    DcsCreate {
        /// Full key including coordination prefix.
        key: Vec<u8>,
        /// Value bytes.
        value: Vec<u8>,
    },
    /// etcd-need DCS compare-and-swap by revision.
    DcsCas {
        /// Full key.
        key: Vec<u8>,
        /// New value.
        value: Vec<u8>,
        /// Expected mod_revision.
        expected_rev: u64,
    },
    /// etcd-need DCS get (lease-aware on server).
    DcsGet {
        /// Full key.
        key: Vec<u8>,
    },
    /// Successful DCS create/CAS revision.
    RespRev {
        /// `mod_revision` after the mutation.
        rev: u64,
    },
    /// Multi-key put via server `put_many` (same-range batch; multi-range uses
    /// 2PC `commit_tx` inside `put_many` — F77).
    PutBatch {
        /// Key/value pairs (may span ranges).
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_bytes(b: &mut Vec<u8>, data: &[u8]) {
    put_u32(b, data.len() as u32);
    b.extend_from_slice(data);
}

fn take_u32(buf: &[u8], off: &mut usize) -> Result<u32> {
    if *off + 4 > buf.len() {
        return Err(StoreError::Msg("tcp eof u32".into()));
    }
    let v = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(v)
}
fn take_u64(buf: &[u8], off: &mut usize) -> Result<u64> {
    if *off + 8 > buf.len() {
        return Err(StoreError::Msg("tcp eof u64".into()));
    }
    let v = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}
fn take_bytes(buf: &[u8], off: &mut usize) -> Result<Vec<u8>> {
    let n = take_u32(buf, off)? as usize;
    if *off + n > buf.len() {
        return Err(StoreError::Msg("tcp eof bytes".into()));
    }
    let v = buf[*off..*off + n].to_vec();
    *off += n;
    Ok(v)
}

impl WireMsg {
    /// Encode body (without frame header).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            WireMsg::Peer { from, to, body } => {
                b.push(1);
                put_u64(&mut b, *from);
                put_u64(&mut b, *to);
                put_bytes(&mut b, body);
            }
            WireMsg::Put { key, value } => {
                b.push(2);
                put_bytes(&mut b, key);
                put_bytes(&mut b, value);
            }
            WireMsg::Get { key } => {
                b.push(3);
                put_bytes(&mut b, key);
            }
            WireMsg::RespOk => b.push(4),
            WireMsg::RespErr { message } => {
                b.push(5);
                put_bytes(&mut b, message.as_bytes());
            }
            WireMsg::RespValue { value } => {
                b.push(6);
                match value {
                    None => b.push(0),
                    Some(v) => {
                        b.push(1);
                        put_bytes(&mut b, v);
                    }
                }
            }
            WireMsg::Tick => b.push(7),
            WireMsg::Status => b.push(8),
            WireMsg::StatusResp { text } => {
                b.push(9);
                put_bytes(&mut b, text.as_bytes());
            }
            WireMsg::SetPeers { peers } => {
                b.push(10);
                put_u32(&mut b, peers.len() as u32);
                for (id, hp) in peers {
                    put_u64(&mut b, *id);
                    put_bytes(&mut b, hp.as_bytes());
                }
            }
            WireMsg::CommitTx { pairs } => {
                b.push(11);
                put_u32(&mut b, pairs.len() as u32);
                for (k, v) in pairs {
                    put_bytes(&mut b, k);
                    put_bytes(&mut b, v);
                }
            }
            WireMsg::RespTxn { txn_id } => {
                b.push(12);
                put_u64(&mut b, *txn_id);
            }
            WireMsg::DcsCreate { key, value } => {
                b.push(13);
                put_bytes(&mut b, key);
                put_bytes(&mut b, value);
            }
            WireMsg::DcsCas {
                key,
                value,
                expected_rev,
            } => {
                b.push(14);
                put_bytes(&mut b, key);
                put_bytes(&mut b, value);
                put_u64(&mut b, *expected_rev);
            }
            WireMsg::DcsGet { key } => {
                b.push(15);
                put_bytes(&mut b, key);
            }
            WireMsg::RespRev { rev } => {
                b.push(16);
                put_u64(&mut b, *rev);
            }
            WireMsg::PutBatch { pairs } => {
                b.push(17);
                put_u32(&mut b, pairs.len() as u32);
                for (k, v) in pairs {
                    put_bytes(&mut b, k);
                    put_bytes(&mut b, v);
                }
            }
        }
        b
    }

    /// Decode body.
    ///
    /// # Errors
    /// Truncation / bad tag.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.is_empty() {
            return Err(StoreError::Msg("tcp empty body".into()));
        }
        let mut off = 1usize;
        match buf[0] {
            1 => {
                let from = take_u64(buf, &mut off)?;
                let to = take_u64(buf, &mut off)?;
                let body = take_bytes(buf, &mut off)?;
                Ok(WireMsg::Peer { from, to, body })
            }
            2 => {
                let key = take_bytes(buf, &mut off)?;
                let value = take_bytes(buf, &mut off)?;
                Ok(WireMsg::Put { key, value })
            }
            3 => Ok(WireMsg::Get {
                key: take_bytes(buf, &mut off)?,
            }),
            4 => Ok(WireMsg::RespOk),
            5 => {
                let message = String::from_utf8_lossy(&take_bytes(buf, &mut off)?).into_owned();
                Ok(WireMsg::RespErr { message })
            }
            6 => {
                if off >= buf.len() {
                    return Err(StoreError::Msg("tcp resp value flag".into()));
                }
                let flag = buf[off];
                off += 1;
                let value = if flag == 0 {
                    None
                } else {
                    Some(take_bytes(buf, &mut off)?)
                };
                Ok(WireMsg::RespValue { value })
            }
            7 => Ok(WireMsg::Tick),
            8 => Ok(WireMsg::Status),
            9 => {
                let text = String::from_utf8_lossy(&take_bytes(buf, &mut off)?).into_owned();
                Ok(WireMsg::StatusResp { text })
            }
            10 => {
                let n = take_u32(buf, &mut off)? as usize;
                if n > 256 {
                    return Err(StoreError::Msg("setpeers too many".into()));
                }
                let mut peers = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = take_u64(buf, &mut off)?;
                    let hp = String::from_utf8_lossy(&take_bytes(buf, &mut off)?).into_owned();
                    peers.push((id, hp));
                }
                Ok(WireMsg::SetPeers { peers })
            }
            11 => {
                let n = take_u32(buf, &mut off)? as usize;
                // Soft cap + residual (F2/F39).
                let rem = buf.len().saturating_sub(off);
                if n > 10_000 || n > rem {
                    return Err(StoreError::Msg("commit_tx too many pairs".into()));
                }
                let mut pairs = Vec::with_capacity(n);
                for _ in 0..n {
                    let key = take_bytes(buf, &mut off)?;
                    let value = take_bytes(buf, &mut off)?;
                    pairs.push((key, value));
                }
                Ok(WireMsg::CommitTx { pairs })
            }
            12 => Ok(WireMsg::RespTxn {
                txn_id: take_u64(buf, &mut off)?,
            }),
            13 => {
                let key = take_bytes(buf, &mut off)?;
                let value = take_bytes(buf, &mut off)?;
                Ok(WireMsg::DcsCreate { key, value })
            }
            14 => {
                let key = take_bytes(buf, &mut off)?;
                let value = take_bytes(buf, &mut off)?;
                let expected_rev = take_u64(buf, &mut off)?;
                Ok(WireMsg::DcsCas {
                    key,
                    value,
                    expected_rev,
                })
            }
            15 => Ok(WireMsg::DcsGet {
                key: take_bytes(buf, &mut off)?,
            }),
            16 => Ok(WireMsg::RespRev {
                rev: take_u64(buf, &mut off)?,
            }),
            17 => {
                let n = take_u32(buf, &mut off)? as usize;
                let rem = buf.len().saturating_sub(off);
                if n > 10_000 || n > rem {
                    return Err(StoreError::Msg("put_batch too many pairs".into()));
                }
                let mut pairs = Vec::with_capacity(n);
                for _ in 0..n {
                    let key = take_bytes(buf, &mut off)?;
                    let value = take_bytes(buf, &mut off)?;
                    pairs.push((key, value));
                }
                Ok(WireMsg::PutBatch { pairs })
            }
            t => Err(StoreError::Msg(format!("tcp bad tag {t}"))),
        }
    }
}

/// Write one framed message.
///
/// # Errors
/// I/O.
pub fn write_frame(w: &mut impl Write, msg: &WireMsg) -> Result<()> {
    let body = msg.encode();
    let mut hdr = Vec::with_capacity(8);
    hdr.extend_from_slice(MAGIC);
    put_u32(&mut hdr, body.len() as u32);
    w.write_all(&hdr)
        .map_err(|e| StoreError::Msg(format!("tcp write hdr: {e}")))?;
    w.write_all(&body)
        .map_err(|e| StoreError::Msg(format!("tcp write body: {e}")))?;
    w.flush()
        .map_err(|e| StoreError::Msg(format!("tcp flush: {e}")))?;
    Ok(())
}

/// Read one framed message.
///
/// # Errors
/// I/O / truncated.
pub fn read_frame(r: &mut impl Read) -> Result<WireMsg> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)
        .map_err(|e| StoreError::Msg(format!("tcp read magic: {e}")))?;
    if &magic != MAGIC {
        return Err(StoreError::Msg("tcp bad magic".into()));
    }
    let mut len_b = [0u8; 4];
    r.read_exact(&mut len_b)
        .map_err(|e| StoreError::Msg(format!("tcp read len: {e}")))?;
    let len = u32::from_le_bytes(len_b) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(StoreError::Msg("tcp frame too large".into()));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)
        .map_err(|e| StoreError::Msg(format!("tcp read body: {e}")))?;
    WireMsg::decode(&body)
}

/// Resolve `host:port` (hostname or literal IP) to a [`SocketAddr`].
///
/// # Errors
/// DNS / bad address.
pub fn resolve_host_port(host_port: &str) -> Result<SocketAddr> {
    host_port
        .to_socket_addrs()
        .map_err(|e| StoreError::Msg(format!("resolve {host_port}: {e}")))?
        .next()
        .ok_or_else(|| StoreError::Msg(format!("resolve {host_port}: empty")))
}

/// Connect with a short timeout.
///
/// # Errors
/// Dial / timeout.
pub fn connect(addr: SocketAddr, timeout: Duration) -> Result<TcpStream> {
    TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| StoreError::Msg(format!("tcp connect {addr}: {e}")))
}

/// Connect to `host:port` (re-resolves DNS each call).
///
/// # Errors
/// Resolve / dial.
pub fn connect_host(host_port: &str, timeout: Duration) -> Result<TcpStream> {
    let addr = resolve_host_port(host_port)?;
    connect(addr, timeout)
}

/// Client helper: put via TCP (`host:port` or resolved [`SocketAddr`]).
///
/// # Errors
/// Network / server error.
pub fn client_put(addr: impl AsRef<str>, key: &[u8], value: &[u8]) -> Result<()> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    write_frame(
        &mut s,
        &WireMsg::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        },
    )?;
    match read_frame(&mut s)? {
        WireMsg::RespOk => Ok(()),
        WireMsg::RespErr { message } => Err(StoreError::Msg(message)),
        other => Err(StoreError::Msg(format!("unexpected put resp {other:?}"))),
    }
}

/// Client helper: get via TCP.
///
/// # Errors
/// Network / server error.
pub fn client_get(addr: impl AsRef<str>, key: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    write_frame(&mut s, &WireMsg::Get { key: key.to_vec() })?;
    match read_frame(&mut s)? {
        WireMsg::RespValue { value } => Ok(value),
        WireMsg::RespErr { message } => Err(StoreError::Msg(message)),
        other => Err(StoreError::Msg(format!("unexpected get resp {other:?}"))),
    }
}

/// Client helper: request ticks (election/heartbeat).
///
/// # Errors
/// Network.
pub fn client_tick(addr: impl AsRef<str>, n: u32) -> Result<()> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    for _ in 0..n {
        write_frame(&mut s, &WireMsg::Tick)?;
        match read_frame(&mut s)? {
            WireMsg::RespOk => {}
            WireMsg::RespErr { message } => return Err(StoreError::Msg(message)),
            other => return Err(StoreError::Msg(format!("tick resp {other:?}"))),
        }
    }
    Ok(())
}

/// Client helper: status string.
///
/// # Errors
/// Network / server error.
pub fn client_status(addr: impl AsRef<str>) -> Result<String> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    write_frame(&mut s, &WireMsg::Status)?;
    match read_frame(&mut s)? {
        WireMsg::StatusResp { text } => Ok(text),
        WireMsg::RespErr { message } => Err(StoreError::Msg(message)),
        other => Err(StoreError::Msg(format!("status resp {other:?}"))),
    }
}

/// Client helper: atomic multi-key TX via server `commit_tx` (RFC-0021).
///
/// # Errors
/// Network, NotLeader, Conflict, size limits, protocol.
pub fn client_commit_tx(addr: impl AsRef<str>, pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<u64> {
    validate_tx_pairs(pairs)?;
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    write_frame(
        &mut s,
        &WireMsg::CommitTx {
            pairs: pairs.to_vec(),
        },
    )?;
    match read_frame(&mut s)? {
        WireMsg::RespTxn { txn_id } => Ok(txn_id),
        WireMsg::RespErr { message } => {
            // Re-classify known StoreError displays when possible.
            Err(crate::client::classify_message(&message).into_store_err(&message))
        }
        other => Err(StoreError::Msg(format!(
            "unexpected commit_tx resp {other:?}"
        ))),
    }
}

/// Client helper: multi-key put via TCP `PutBatch` → server `put_many` (RFC-0025 P1.3).
///
/// One RTT for N keys (grouped by range on server). Not a full OCC transaction
/// (use [`client_commit_tx`] for that).
///
/// # Errors
/// Network, NotLeader, NotCommitted, limits.
pub fn client_put_batch(addr: impl AsRef<str>, pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    validate_tx_pairs(pairs)?;
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    write_frame(
        &mut s,
        &WireMsg::PutBatch {
            pairs: pairs.to_vec(),
        },
    )?;
    match read_frame(&mut s)? {
        WireMsg::RespOk => Ok(()),
        WireMsg::RespErr { message } => {
            Err(crate::client::classify_message(&message).into_store_err(&message))
        }
        other => Err(StoreError::Msg(format!(
            "unexpected put_batch resp {other:?}"
        ))),
    }
}

/// Client helper: DCS create-if-absent over TCP (returns mod_revision).
///
/// # Errors
/// Network, NotLeader, key exists, NotCommitted.
pub fn client_dcs_create(addr: impl AsRef<str>, key: &[u8], value: &[u8]) -> Result<u64> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    write_frame(
        &mut s,
        &WireMsg::DcsCreate {
            key: key.to_vec(),
            value: value.to_vec(),
        },
    )?;
    match read_frame(&mut s)? {
        WireMsg::RespRev { rev } => Ok(rev),
        WireMsg::RespErr { message } => {
            Err(crate::client::classify_message(&message).into_store_err(&message))
        }
        other => Err(StoreError::Msg(format!(
            "unexpected dcs_create resp {other:?}"
        ))),
    }
}

/// Client helper: DCS CAS over TCP.
///
/// # Errors
/// Network, NotLeader, revision mismatch, NotCommitted.
pub fn client_dcs_cas(
    addr: impl AsRef<str>,
    key: &[u8],
    value: &[u8],
    expected_rev: u64,
) -> Result<u64> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    write_frame(
        &mut s,
        &WireMsg::DcsCas {
            key: key.to_vec(),
            value: value.to_vec(),
            expected_rev,
        },
    )?;
    match read_frame(&mut s)? {
        WireMsg::RespRev { rev } => Ok(rev),
        WireMsg::RespErr { message } => {
            Err(crate::client::classify_message(&message).into_store_err(&message))
        }
        other => Err(StoreError::Msg(format!(
            "unexpected dcs_cas resp {other:?}"
        ))),
    }
}

/// Client helper: DCS get over TCP (user value only; lease-aware server-side).
///
/// # Errors
/// Network / server error.
pub fn client_dcs_get(addr: impl AsRef<str>, key: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    write_frame(&mut s, &WireMsg::DcsGet { key: key.to_vec() })?;
    match read_frame(&mut s)? {
        WireMsg::RespValue { value } => Ok(value),
        WireMsg::RespErr { message } => Err(StoreError::Msg(message)),
        other => Err(StoreError::Msg(format!(
            "unexpected dcs_get resp {other:?}"
        ))),
    }
}

/// Client helper: replace peer address map on a running node (no restart).
///
/// # Errors
/// Network / server error.
pub fn client_set_peers(addr: impl AsRef<str>, peers: &[(u64, String)]) -> Result<()> {
    let mut s = connect_host(addr.as_ref(), Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    write_frame(
        &mut s,
        &WireMsg::SetPeers {
            peers: peers.to_vec(),
        },
    )?;
    match read_frame(&mut s)? {
        WireMsg::RespOk => Ok(()),
        WireMsg::RespErr { message } => Err(StoreError::Msg(message)),
        other => Err(StoreError::Msg(format!("setpeers resp {other:?}"))),
    }
}

/// Encode peer bytes for wire.
#[must_use]
pub fn peer_wire(from: u64, to: u64, msg: &PeerMsg) -> WireMsg {
    WireMsg::Peer {
        from,
        to,
        body: msg.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrip_put_get() {
        let m = WireMsg::Put {
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        };
        assert_eq!(WireMsg::decode(&m.encode()).unwrap(), m);
        let g = WireMsg::Get { key: b"k".to_vec() };
        assert_eq!(WireMsg::decode(&g.encode()).unwrap(), g);
        let r = WireMsg::RespValue {
            value: Some(b"v".to_vec()),
        };
        assert_eq!(WireMsg::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn wire_put_batch_roundtrip() {
        let m = WireMsg::PutBatch {
            pairs: vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
            ],
        };
        assert_eq!(WireMsg::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn wire_dcs_create_cas_get_rev() {
        let c = WireMsg::DcsCreate {
            key: b"m/lock".to_vec(),
            value: b"n1".to_vec(),
        };
        assert_eq!(WireMsg::decode(&c.encode()).unwrap(), c);
        let cas = WireMsg::DcsCas {
            key: b"m/lock".to_vec(),
            value: b"n2".to_vec(),
            expected_rev: 3,
        };
        assert_eq!(WireMsg::decode(&cas.encode()).unwrap(), cas);
        let g = WireMsg::DcsGet {
            key: b"m/lock".to_vec(),
        };
        assert_eq!(WireMsg::decode(&g.encode()).unwrap(), g);
        let r = WireMsg::RespRev { rev: 9 };
        assert_eq!(WireMsg::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn wire_peer_frame() {
        let pm = PeerMsg::RequestVote {
            range_id: 1,
            term: 2,
            candidate_id: 3,
            last_log_index: 0,
            last_log_term: 0,
        };
        let w = peer_wire(3, 1, &pm);
        let d = WireMsg::decode(&w.encode()).unwrap();
        match d {
            WireMsg::Peer { from, to, body } => {
                assert_eq!(from, 3);
                assert_eq!(to, 1);
                assert_eq!(PeerMsg::decode(&body).unwrap(), pm);
            }
            _ => panic!("not peer"),
        }
    }
}
