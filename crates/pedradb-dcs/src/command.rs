//! Replicated DCS commands for Raft apply (compose DCS + consensus).
//!
//! Leader checks preconditions, then logs a [`DcsCommand`]. Every node applies
//! the same command to its PedraDB via [`apply_dcs_command`].
//!
//! # Lease field (multi-node / store path)
//!
//! - `lease == 0` — no TTL (immortal until delete).
//! - `lease != 0` — **absolute deadline in logical milliseconds** (cluster clock).
//!   Baked into the log at propose time so apply is deterministic; reads use
//!   [`dcs_get_at`] with the caller's current logical time. This is the durable
//!   multi-node alternative to the process-local lease table in [`crate::Dcs`].

use pedradb_core::{Db, Env};

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

fn get_kv<E: Env>(db: &Db<E>, key: &[u8]) -> Option<KeyValue> {
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

/// Whether a binding is still live at `now_ms` (`lease==0` never expires).
#[must_use]
pub fn lease_live(lease: u64, now_ms: u64) -> bool {
    lease == 0 || now_ms < lease
}

fn get_kv_at<E: Env>(db: &Db<E>, key: &[u8], now_ms: u64) -> Option<KeyValue> {
    let kv = get_kv(db, key)?;
    if lease_live(kv.lease, now_ms) {
        Some(kv)
    } else {
        None
    }
}

fn revision<E: Env>(db: &Db<E>) -> u64 {
    db.get(REV_KEY)
        .and_then(|b| decode_u64(&b).ok())
        .unwrap_or(0)
}

/// Read-only precondition check (leader, before propose).
///
/// Uses raw presence (no clock). Prefer [`check_command_at`] when leases use
/// absolute deadlines.
///
/// # Errors
/// CAS / create failure (same as local DCS).
pub fn check_command<E: Env>(db: &Db<E>, cmd: &DcsCommand) -> Result<()> {
    check_command_at(db, cmd, 0)
}

/// Precondition check with logical clock (`now_ms`).
///
/// Expired leased keys (`lease != 0 && now_ms >= lease`) count as **absent**
/// for create/CAS (F7 multi-node: lock liveness after TTL).
///
/// # Errors
/// CAS / create failure.
pub fn check_command_at<E: Env>(db: &Db<E>, cmd: &DcsCommand, now_ms: u64) -> Result<()> {
    match cmd {
        DcsCommand::Put { .. } => Ok(()),
        DcsCommand::Create { key, .. } => {
            if get_kv_at(db, key, now_ms).is_some() {
                Err(DcsError::CasFailed("key exists"))
            } else {
                Ok(())
            }
        }
        DcsCommand::Cas {
            key, expected_rev, ..
        } => {
            let cur = get_kv_at(db, key, now_ms);
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
pub fn apply_dcs_command<E: Env>(db: &mut Db<E>, cmd: &DcsCommand) -> Result<u64> {
    match cmd {
        DcsCommand::Create { key, value, lease } => {
            if let Some(existing) = get_kv(db, key) {
                // Dual-append same create (same absolute lease deadline): idempotent.
                if existing.lease == *lease && existing.value == *value {
                    return Ok(existing.mod_revision);
                }
                // Immortal key already present (lease=0): keep dual-append safety.
                if existing.lease == 0 {
                    return Ok(existing.mod_revision);
                }
                // Leased binding with different deadline/value: leader re-created after
                // expiry (or CAS path) — overwrite so apply cursor never stalls.
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
            // expected_rev == 0 is create-if-absent: dual-append safety when same binding.
            if *expected_rev == 0 {
                if let Some(existing) = get_kv(db, key) {
                    if existing.lease == *lease && existing.value == *value {
                        return Ok(existing.mod_revision);
                    }
                    if existing.lease == 0 {
                        return Ok(existing.mod_revision);
                    }
                    // else fall through to overwrite (re-create after expiry)
                }
            } else if let Some(existing) = get_kv(db, key) {
                if existing.mod_revision != *expected_rev {
                    // Stale CAS on apply: no-op keep cursor (leader pre-checked).
                    return Ok(existing.mod_revision);
                }
            } else {
                // Key missing on apply — no-op revision.
                return Ok(revision(db));
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

fn put_new<E: Env>(
    db: &mut Db<E>,
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

/// Point get for followers (raw DCS layout — **ignores** lease expiry).
#[must_use]
pub fn dcs_get<E: Env>(db: &Db<E>, key: &[u8]) -> Option<KeyValue> {
    get_kv(db, key)
}

/// Point get with absolute-deadline lease check (`now_ms` = cluster logical time).
#[must_use]
pub fn dcs_get_at<E: Env>(db: &Db<E>, key: &[u8], now_ms: u64) -> Option<KeyValue> {
    get_kv_at(db, key, now_ms)
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
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
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

    #[test]
    fn absolute_lease_deadline_expires_for_get_and_create() {
        let (dir, mut db) = temp_db();
        let expiry = 1_000u64;
        let cmd = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-a".to_vec(),
            lease: expiry,
        };
        apply_dcs_command(&mut db, &cmd).unwrap();
        assert!(dcs_get_at(&db, b"lock", 999).is_some());
        assert!(dcs_get_at(&db, b"lock", 1_000).is_none());
        // After expiry, create-if-absent is allowed again.
        assert!(check_command_at(&db, &cmd, 1_000).is_ok());
        let cmd2 = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-b".to_vec(),
            lease: 5_000,
        };
        apply_dcs_command(&mut db, &cmd2).unwrap();
        assert_eq!(
            dcs_get_at(&db, b"lock", 1_001).unwrap().value,
            b"holder-b"
        );
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
