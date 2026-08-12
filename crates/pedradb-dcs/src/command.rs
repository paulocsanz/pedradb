//! Replicated DCS commands for Raft apply (compose DCS + consensus).
//!
//! Leader checks preconditions, then logs a [`DcsCommand`]. Every node applies
//! the same command to its PedraDB via [`apply_dcs_command`] — no lease table
//! required for lease=0 ops (multi-node first ship).

use pedradb_core::Db;

use crate::{
    decode_meta, decode_u64, encode_meta, encode_u64, kv_key, meta_key, DcsError, KeyValue,
    Result, REV_KEY,
};

/// Marker key used inside Raft log entries (not a user DCS key).
pub const DCS_CMD_MARKER: &[u8] = b"\0pedra/dcs/cmd";

/// Deterministic DCS mutation (lease bindings must be explicit in the command).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DcsCommand {
    /// Unconditional put.
    Put {
        /// User key.
        key: Vec<u8>,
        /// Value.
        value: Vec<u8>,
        /// Lease id (`0` = none).
        lease: u64,
    },
    /// Create only if absent.
    Create {
        /// User key.
        key: Vec<u8>,
        /// Value.
        value: Vec<u8>,
        /// Lease id.
        lease: u64,
    },
    /// CAS on `mod_revision` (`0` = create-if-absent).
    Cas {
        /// User key.
        key: Vec<u8>,
        /// Value.
        value: Vec<u8>,
        /// Expected revision.
        expected_rev: u64,
        /// Lease id.
        lease: u64,
    },
    /// Delete if present.
    Delete {
        /// User key.
        key: Vec<u8>,
    },
}

impl DcsCommand {
    /// Encode for Raft log payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Self::Put { key, value, lease } => {
                b.push(1);
                put_bytes(&mut b, key);
                put_bytes(&mut b, value);
                b.extend_from_slice(&lease.to_le_bytes());
            }
            Self::Create { key, value, lease } => {
                b.push(2);
                put_bytes(&mut b, key);
                put_bytes(&mut b, value);
                b.extend_from_slice(&lease.to_le_bytes());
            }
            Self::Cas {
                key,
                value,
                expected_rev,
                lease,
            } => {
                b.push(3);
                put_bytes(&mut b, key);
                put_bytes(&mut b, value);
                b.extend_from_slice(&expected_rev.to_le_bytes());
                b.extend_from_slice(&lease.to_le_bytes());
            }
            Self::Delete { key } => {
                b.push(4);
                put_bytes(&mut b, key);
            }
        }
        b
    }

    /// Decode from Raft log payload.
    ///
    /// # Errors
    /// Corrupt bytes.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.is_empty() {
            return Err(DcsError::Corrupt("empty cmd".into()));
        }
        let mut off = 1usize;
        match buf[0] {
            1 => {
                let key = take_bytes(buf, &mut off)?;
                let value = take_bytes(buf, &mut off)?;
                let lease = take_u64(buf, &mut off)?;
                Ok(Self::Put { key, value, lease })
            }
            2 => {
                let key = take_bytes(buf, &mut off)?;
                let value = take_bytes(buf, &mut off)?;
                let lease = take_u64(buf, &mut off)?;
                Ok(Self::Create { key, value, lease })
            }
            3 => {
                let key = take_bytes(buf, &mut off)?;
                let value = take_bytes(buf, &mut off)?;
                let expected_rev = take_u64(buf, &mut off)?;
                let lease = take_u64(buf, &mut off)?;
                Ok(Self::Cas {
                    key,
                    value,
                    expected_rev,
                    lease,
                })
            }
            4 => {
                let key = take_bytes(buf, &mut off)?;
                Ok(Self::Delete { key })
            }
            t => Err(DcsError::Corrupt(format!("bad cmd tag {t}"))),
        }
    }
}

fn put_bytes(b: &mut Vec<u8>, d: &[u8]) {
    b.extend_from_slice(&(d.len() as u32).to_le_bytes());
    b.extend_from_slice(d);
}

fn take_bytes(buf: &[u8], off: &mut usize) -> Result<Vec<u8>> {
    if *off + 4 > buf.len() {
        return Err(DcsError::Corrupt("eof len".into()));
    }
    let n = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
    *off += 4;
    if *off + n > buf.len() {
        return Err(DcsError::Corrupt("eof bytes".into()));
    }
    let v = buf[*off..*off + n].to_vec();
    *off += n;
    Ok(v)
}

fn take_u64(buf: &[u8], off: &mut usize) -> Result<u64> {
    if *off + 8 > buf.len() {
        return Err(DcsError::Corrupt("eof u64".into()));
    }
    let v = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}

fn get_kv(db: &Db, key: &[u8]) -> Option<KeyValue> {
    let val = db.get(&kv_key(key))?;
    let meta = db.get(&meta_key(key))?;
    let (create, mod_rev, lease) = decode_meta(&meta).ok()?;
    Some(KeyValue {
        key: key.to_vec(),
        value: val.to_vec(),
        mod_revision: mod_rev,
        create_revision: create,
        lease,
    })
}

fn revision(db: &Db) -> u64 {
    db.get(REV_KEY)
        .and_then(|b| decode_u64(&b).ok())
        .unwrap_or(0)
}

/// Read-only precondition check (leader, before propose).
///
/// # Errors
/// CAS / create failure (same as local DCS).
pub fn check_command(db: &Db, cmd: &DcsCommand) -> Result<()> {
    match cmd {
        DcsCommand::Put { .. } => Ok(()),
        DcsCommand::Create { key, .. } => {
            if get_kv(db, key).is_some() {
                Err(DcsError::CasFailed("key exists"))
            } else {
                Ok(())
            }
        }
        DcsCommand::Cas {
            key, expected_rev, ..
        } => {
            let cur = get_kv(db, key);
            match (*expected_rev, cur) {
                (0, Some(_)) => Err(DcsError::CasFailed("expected absent")),
                (0, None) => Ok(()),
                (r, Some(kv)) if kv.mod_revision == r => Ok(()),
                (_, None) => Err(DcsError::CasFailed("key missing")),
                _ => Err(DcsError::CasFailed("revision mismatch")),
            }
        }
        DcsCommand::Delete { .. } => Ok(()),
    }
}

/// Apply a command to `db` (all Raft followers + leader on commit).
///
/// **Raft apply must not permanently stall the apply cursor.** A second
/// [`DcsCommand::Create`] when the key already exists is a no-op that returns
/// the existing revision (dual-append / retry safety). Leader **pre-check**
/// via [`check_command`] still rejects client create races before propose.
///
/// # Errors
/// I/O, or CAS precondition failure (revision mismatch).
pub fn apply_dcs_command(db: &mut Db, cmd: &DcsCommand) -> Result<u64> {
    match cmd {
        DcsCommand::Create { key, value, lease } => {
            if let Some(existing) = get_kv(db, key) {
                let _ = (value, lease);
                return Ok(existing.mod_revision);
            }
            put_new(db, key, value, *lease, None)
        }
        DcsCommand::Put { key, value, lease } => {
            let prev = get_kv(db, key);
            let create_hint = prev.map(|p| p.create_revision);
            put_new(db, key, value, *lease, create_hint)
        }
        DcsCommand::Cas {
            key,
            value,
            expected_rev,
            lease,
        } => {
            // expected_rev == 0 is create-if-absent: idempotent if key already present
            // (same dual-append safety as Create).
            if *expected_rev == 0 {
                if let Some(existing) = get_kv(db, key) {
                    let _ = (value, lease);
                    return Ok(existing.mod_revision);
                }
            } else {
                check_command(
                    db,
                    &DcsCommand::Cas {
                        key: key.clone(),
                        value: value.clone(),
                        expected_rev: *expected_rev,
                        lease: *lease,
                    },
                )?;
            }
            let prev = get_kv(db, key);
            let create_hint = prev.map(|p| p.create_revision);
            put_new(db, key, value, *lease, create_hint)
        }
        DcsCommand::Delete { key } => {
            if get_kv(db, key).is_none() {
                return Ok(revision(db));
            }
            let rev = revision(db) + 1;
            let mut tx = db.begin();
            tx.put(REV_KEY, encode_u64(rev))?;
            tx.delete(kv_key(key))?;
            tx.delete(meta_key(key))?;
            tx.commit()?;
            Ok(rev)
        }
    }
}

fn put_new(
    db: &mut Db,
    key: &[u8],
    value: &[u8],
    lease: u64,
    create_hint: Option<u64>,
) -> Result<u64> {
    let rev = revision(db) + 1;
    let create = create_hint.unwrap_or(rev);
    let mut tx = db.begin();
    tx.put(REV_KEY, encode_u64(rev))?;
    tx.put(kv_key(key), value)?;
    tx.put(meta_key(key), encode_meta(create, rev, lease))?;
    tx.commit()?;
    Ok(rev)
}

/// Point get for followers (raw DCS layout).
#[must_use]
pub fn dcs_get(db: &Db, key: &[u8]) -> Option<KeyValue> {
    get_kv(db, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pedradb_core::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_db() -> (std::path::PathBuf, Db) {
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-dcs-cmd-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        let db = Db::open_with(
            &d,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                exclusive: true,
            },
        )
        .unwrap();
        (d, db)
    }

    #[test]
    fn encode_round_trip_and_apply_cas() {
        let (dir, mut db) = temp_db();
        let cmd = DcsCommand::Cas {
            key: b"leader".to_vec(),
            value: b"n1".to_vec(),
            expected_rev: 0,
            lease: 0,
        };
        let bytes = cmd.encode();
        let decoded = DcsCommand::decode(&bytes).unwrap();
        assert_eq!(cmd, decoded);
        let rev = apply_dcs_command(&mut db, &cmd).unwrap();
        assert_eq!(rev, 1);
        assert_eq!(dcs_get(&db, b"leader").unwrap().value, b"n1");
        // second Create on apply is idempotent (must not brick Raft apply cursor)
        let rev2 = apply_dcs_command(&mut db, &cmd).unwrap();
        assert_eq!(rev2, 1);
        // Leader pre-check still rejects create-if-absent race for clients.
        assert!(check_command(&db, &cmd).is_err());
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
