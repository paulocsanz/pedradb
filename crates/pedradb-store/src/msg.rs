//! Peer RPC messages for Montanha-Store (Net / World delivery).
//!
//! Framing is self-describing (tag + fields). Production TCP can reuse the same
//! codec; [`crate::StoreCluster::handle_inbound`] applies deliveries.

use super::{decode_entry, encode_bytes, encode_entry, take_bytes, LogRec, Result, StoreError};

/// Raft-ish peer RPC used by Montanha-Store multi-Raft ranges.
///
/// `AppendEntries.entries` uses crate-private [`crate::LogRec`]; external callers
/// should treat messages as opaque bytes via [`Self::encode`] / [`Self::decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(private_interfaces)]
pub enum PeerMsg {
    /// RequestVote (candidate → peer).
    RequestVote {
        /// Range id.
        range_id: u64,
        /// Candidate term.
        term: u64,
        /// Candidate node id.
        candidate_id: u64,
        /// Candidate last log index.
        last_log_index: u64,
        /// Candidate last log term.
        last_log_term: u64,
    },
    /// RequestVote reply (peer → candidate).
    RequestVoteReply {
        /// Range id.
        range_id: u64,
        /// Responder current term.
        term: u64,
        /// Whether vote was granted.
        vote_granted: bool,
    },
    /// AppendEntries / heartbeat (leader → follower).
    AppendEntries {
        /// Range id.
        range_id: u64,
        /// Leader term.
        term: u64,
        /// Leader node id.
        leader_id: u64,
        /// Index of log entry immediately preceding new ones.
        prev_log_index: u64,
        /// Term of `prev_log_index`.
        prev_log_term: u64,
        /// Leader commit index.
        leader_commit: u64,
        /// Entries to append (may be empty = heartbeat).
        entries: Vec<LogRec>,
    },
    /// AppendEntries reply (follower → leader).
    AppendEntriesReply {
        /// Range id.
        range_id: u64,
        /// Responder current term.
        term: u64,
        /// Whether append succeeded.
        success: bool,
        /// On success: highest matching index; on failure: hint (0).
        match_index: u64,
    },
    /// InstallSnapshot (leader → lagging follower after log compact).
    InstallSnapshot {
        /// Range id.
        range_id: u64,
        /// Leader term.
        term: u64,
        /// Leader node id.
        leader_id: u64,
        /// Last log index included in the snapshot.
        last_included_index: u64,
        /// Term of `last_included_index`.
        last_included_term: u64,
        /// Applied key/value pairs (range user data + needed meta).
        kv_pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
    /// InstallSnapshot reply.
    InstallSnapshotReply {
        /// Range id.
        range_id: u64,
        /// Responder term.
        term: u64,
        /// Whether install succeeded.
        success: bool,
        /// `last_included_index` echoed on success.
        match_index: u64,
    },
}

fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn take_u64(buf: &[u8], off: &mut usize) -> Result<u64> {
    if *off + 8 > buf.len() {
        return Err(StoreError::Msg("peer msg eof u64".into()));
    }
    let v = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}

impl PeerMsg {
    /// Encode to opaque bytes for [`crate::net`]-style delivery.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            PeerMsg::RequestVote {
                range_id,
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                b.push(1);
                put_u64(&mut b, *range_id);
                put_u64(&mut b, *term);
                put_u64(&mut b, *candidate_id);
                put_u64(&mut b, *last_log_index);
                put_u64(&mut b, *last_log_term);
            }
            PeerMsg::RequestVoteReply {
                range_id,
                term,
                vote_granted,
            } => {
                b.push(2);
                put_u64(&mut b, *range_id);
                put_u64(&mut b, *term);
                b.push(u8::from(*vote_granted));
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
                b.push(3);
                put_u64(&mut b, *range_id);
                put_u64(&mut b, *term);
                put_u64(&mut b, *leader_id);
                put_u64(&mut b, *prev_log_index);
                put_u64(&mut b, *prev_log_term);
                put_u64(&mut b, *leader_commit);
                put_u64(&mut b, entries.len() as u64);
                for rec in entries {
                    put_u64(&mut b, rec.index);
                    put_u64(&mut b, rec.term);
                    let ent = encode_entry(&rec.entry);
                    encode_bytes(&mut b, &ent);
                }
            }
            PeerMsg::AppendEntriesReply {
                range_id,
                term,
                success,
                match_index,
            } => {
                b.push(4);
                put_u64(&mut b, *range_id);
                put_u64(&mut b, *term);
                b.push(u8::from(*success));
                put_u64(&mut b, *match_index);
            }
            PeerMsg::InstallSnapshot {
                range_id,
                term,
                leader_id,
                last_included_index,
                last_included_term,
                kv_pairs,
            } => {
                b.push(5);
                put_u64(&mut b, *range_id);
                put_u64(&mut b, *term);
                put_u64(&mut b, *leader_id);
                put_u64(&mut b, *last_included_index);
                put_u64(&mut b, *last_included_term);
                put_u64(&mut b, kv_pairs.len() as u64);
                for (k, v) in kv_pairs {
                    encode_bytes(&mut b, k);
                    encode_bytes(&mut b, v);
                }
            }
            PeerMsg::InstallSnapshotReply {
                range_id,
                term,
                success,
                match_index,
            } => {
                b.push(6);
                put_u64(&mut b, *range_id);
                put_u64(&mut b, *term);
                b.push(u8::from(*success));
                put_u64(&mut b, *match_index);
            }
        }
        b
    }

    /// Decode opaque bytes.
    ///
    /// # Errors
    /// Truncated / bad tag / bad entry payload.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.is_empty() {
            return Err(StoreError::Msg("peer msg empty".into()));
        }
        let tag = buf[0];
        let mut off = 1;
        match tag {
            1 => Ok(PeerMsg::RequestVote {
                range_id: take_u64(buf, &mut off)?,
                term: take_u64(buf, &mut off)?,
                candidate_id: take_u64(buf, &mut off)?,
                last_log_index: take_u64(buf, &mut off)?,
                last_log_term: take_u64(buf, &mut off)?,
            }),
            2 => {
                let range_id = take_u64(buf, &mut off)?;
                let term = take_u64(buf, &mut off)?;
                if off >= buf.len() {
                    return Err(StoreError::Msg("rv reply flag".into()));
                }
                let vote_granted = buf[off] != 0;
                Ok(PeerMsg::RequestVoteReply {
                    range_id,
                    term,
                    vote_granted,
                })
            }
            3 => {
                let range_id = take_u64(buf, &mut off)?;
                let term = take_u64(buf, &mut off)?;
                let leader_id = take_u64(buf, &mut off)?;
                let prev_log_index = take_u64(buf, &mut off)?;
                let prev_log_term = take_u64(buf, &mut off)?;
                let leader_commit = take_u64(buf, &mut off)?;
                let n = take_u64(buf, &mut off)? as usize;
                // Soft cap + residual (F2/F9/F39): tiny frames must not allocate for huge n.
                let rem = buf.len().saturating_sub(off);
                if n > 1_000_000 || n > rem {
                    return Err(StoreError::Msg("ae entries too many".into()));
                }
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    let index = take_u64(buf, &mut off)?;
                    let eterm = take_u64(buf, &mut off)?;
                    let raw = take_bytes(buf, &mut off)?;
                    let mut eoff = 0;
                    let entry = decode_entry(&raw, &mut eoff)?;
                    if eoff != raw.len() {
                        return Err(StoreError::Msg("ae entry trailing".into()));
                    }
                    entries.push(LogRec {
                        index,
                        term: eterm,
                        entry,
                    });
                }
                Ok(PeerMsg::AppendEntries {
                    range_id,
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    leader_commit,
                    entries,
                })
            }
            4 => {
                let range_id = take_u64(buf, &mut off)?;
                let term = take_u64(buf, &mut off)?;
                if off >= buf.len() {
                    return Err(StoreError::Msg("ae reply flag".into()));
                }
                let success = buf[off] != 0;
                off += 1;
                let match_index = take_u64(buf, &mut off)?;
                Ok(PeerMsg::AppendEntriesReply {
                    range_id,
                    term,
                    success,
                    match_index,
                })
            }
            5 => {
                let range_id = take_u64(buf, &mut off)?;
                let term = take_u64(buf, &mut off)?;
                let leader_id = take_u64(buf, &mut off)?;
                let last_included_index = take_u64(buf, &mut off)?;
                let last_included_term = take_u64(buf, &mut off)?;
                let n = take_u64(buf, &mut off)? as usize;
                // Soft cap + residual (F2/F9/F39).
                let rem = buf.len().saturating_sub(off);
                if n > 2_000_000 || n > rem {
                    return Err(StoreError::Msg("snapshot kv too many".into()));
                }
                let mut kv_pairs = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = take_bytes(buf, &mut off)?;
                    let v = take_bytes(buf, &mut off)?;
                    kv_pairs.push((k, v));
                }
                Ok(PeerMsg::InstallSnapshot {
                    range_id,
                    term,
                    leader_id,
                    last_included_index,
                    last_included_term,
                    kv_pairs,
                })
            }
            6 => {
                let range_id = take_u64(buf, &mut off)?;
                let term = take_u64(buf, &mut off)?;
                if off >= buf.len() {
                    return Err(StoreError::Msg("snap reply flag".into()));
                }
                let success = buf[off] != 0;
                off += 1;
                let match_index = take_u64(buf, &mut off)?;
                Ok(PeerMsg::InstallSnapshotReply {
                    range_id,
                    term,
                    success,
                    match_index,
                })
            }
            t => Err(StoreError::Msg(format!("bad peer msg tag {t}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RangeEntry;

    #[test]
    fn roundtrip_rv() {
        let m = PeerMsg::RequestVote {
            range_id: 1,
            term: 3,
            candidate_id: 2,
            last_log_index: 9,
            last_log_term: 2,
        };
        assert_eq!(PeerMsg::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn roundtrip_ae_with_put() {
        let m = PeerMsg::AppendEntries {
            range_id: 1,
            term: 2,
            leader_id: 1,
            prev_log_index: 0,
            prev_log_term: 0,
            leader_commit: 0,
            entries: vec![LogRec {
                index: 1,
                term: 2,
                entry: RangeEntry::Put {
                    key: b"k".to_vec(),
                    value: b"v".to_vec(),
                    si_gen: 0,
                },
            }],
        };
        let d = PeerMsg::decode(&m.encode()).unwrap();
        assert_eq!(d, m);
    }

    #[test]
    fn roundtrip_replies() {
        let a = PeerMsg::RequestVoteReply {
            range_id: 1,
            term: 4,
            vote_granted: true,
        };
        let b = PeerMsg::AppendEntriesReply {
            range_id: 1,
            term: 4,
            success: false,
            match_index: 0,
        };
        assert_eq!(PeerMsg::decode(&a.encode()).unwrap(), a);
        assert_eq!(PeerMsg::decode(&b.encode()).unwrap(), b);
    }

    /// Hostile AE count below soft cap but above residual must fail-stop without huge alloc.
    #[test]
    fn ae_rejects_count_past_residual() {
        let mut b = vec![3u8]; // AppendEntries tag
        for v in [1u64, 1, 1, 0, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // n = 100_000 entries claimed, zero body remaining after count.
        b.extend_from_slice(&100_000u64.to_le_bytes());
        let err = PeerMsg::decode(&b).expect_err("must reject");
        assert!(
            err.to_string().contains("too many") || err.to_string().contains("eof"),
            "got {err}"
        );
    }
}
