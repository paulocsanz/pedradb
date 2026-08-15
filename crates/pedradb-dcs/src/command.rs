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
//!
//! Apply of Create / Cas(rev=0) is insert-or-no-op (never overwrite). Re-create
//! after TTL is [`bind_absent_create`] → Cas on the expired corpse's revision.

use pedradb_core::{Db, Env};

use crate::{
    decode_meta, decode_u64, encode_meta, encode_u64, kv_key, meta_key, DcsError, KeyValue, Result,
    REV_KEY,
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

/// F115: present-but-undecodable meta is not absence (Create/CAS-0 must not overwrite).
fn reject_undecodable_meta<E: Env>(db: &Db<E>, key: &[u8]) -> Result<()> {
    if let Some(raw) = db.get(&meta_key(key)) {
        decode_meta(&raw)?;
    }
    Ok(())
}

pub use crate::lease_kernel::lease_live;

fn get_kv_at<E: Env>(db: &Db<E>, key: &[u8], now_ms: u64) -> Option<KeyValue> {
    let kv = get_kv(db, key)?;
    if lease_live(kv.lease, now_ms) {
        Some(kv)
    } else {
        None
    }
}

/// Missing `d/rev` → 0; present but unreadable → Err (F113, do not reuse rev 1).
fn revision<E: Env>(db: &Db<E>) -> Result<u64> {
    match db.get(REV_KEY) {
        None => Ok(0),
        Some(b) => decode_u64(&b),
    }
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
            reject_undecodable_meta(db, key)?;
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
                (0, None) => reject_undecodable_meta(db, key),
                (r, Some(kv)) if kv.mod_revision == r => Ok(()),
                (_, None) => Err(DcsError::CasFailed("key missing")),
                _ => Err(DcsError::CasFailed("revision mismatch")),
            }
        }
        DcsCommand::Delete { .. } => Ok(()),
    }
}

/// After a successful [`check_command_at`], rewrite Create / Cas(rev=0) that
/// races an **expired physical corpse** into `Cas(expected_rev = corpse.rev)`.
///
/// Apply of Create / Cas(0) never overwrites (I-DCS-1 at apply). Re-create
/// after TTL must be this CAS so the first binder wins and a later duplicate
/// no-ops. Does **not** rewrite a still-live key — that would turn a TOCTOU
/// into a lock steal.
#[must_use]
pub fn bind_absent_create<E: Env>(db: &Db<E>, cmd: DcsCommand, now_ms: u64) -> DcsCommand {
    match cmd {
        DcsCommand::Create { key, value, lease } => {
            if let Some(rev) = expired_corpse_cas_rev(db, &key, now_ms) {
                DcsCommand::Cas {
                    key,
                    value,
                    expected_rev: rev,
                    lease,
                }
            } else {
                DcsCommand::Create { key, value, lease }
            }
        }
        DcsCommand::Cas {
            key,
            value,
            expected_rev: 0,
            lease,
        } => {
            if let Some(rev) = expired_corpse_cas_rev(db, &key, now_ms) {
                DcsCommand::Cas {
                    key,
                    value,
                    expected_rev: rev,
                    lease,
                }
            } else {
                DcsCommand::Cas {
                    key,
                    value,
                    expected_rev: 0,
                    lease,
                }
            }
        }
        other => other,
    }
}

fn expired_corpse_cas_rev<E: Env>(db: &Db<E>, key: &[u8], now_ms: u64) -> Option<u64> {
    let existing = get_kv(db, key)?;
    if lease_live(existing.lease, now_ms) {
        None
    } else {
        Some(existing.mod_revision)
    }
}

/// Apply a command to `db` (all Raft followers + leader on commit).
///
/// **Raft apply must not permanently stall the apply cursor.** A second
/// [`DcsCommand::Create`] / Cas(rev=0) when the key already exists is a no-op
/// that returns the existing revision (I-DCS-1 at apply; dual-append / retry).
/// Re-create after TTL is **not** Create overwrite: the leader binds to
/// [`bind_absent_create`] (Cas on the physical rev) after pre-check.
/// Leader **pre-check** via [`check_command_at`] still rejects client create
/// races before propose.
///
/// # Errors
/// I/O, or CAS precondition failure (revision mismatch).
pub fn apply_dcs_command<E: Env>(db: &mut Db<E>, cmd: &DcsCommand) -> Result<u64> {
    match cmd {
        DcsCommand::Create { key, value, lease } => {
            reject_undecodable_meta(db, key)?;
            if let Some(existing) = get_kv(db, key) {
                // Physical presence → no-op. Cursor advances. Never overwrite.
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
            // expected_rev == 0 is create-if-absent: same rule as Create (never overwrite).
            if *expected_rev == 0 {
                reject_undecodable_meta(db, key)?;
                if let Some(existing) = get_kv(db, key) {
                    return Ok(existing.mod_revision);
                }
            } else if let Some(existing) = get_kv(db, key) {
                if existing.mod_revision != *expected_rev {
                    // Stale CAS on apply: no-op keep cursor (leader pre-checked).
                    return Ok(existing.mod_revision);
                }
            } else {
                // Key missing on apply — no-op revision.
                return revision(db);
            }
            let prev = get_kv(db, key);
            let create_hint = prev.map(|p| p.create_revision);
            put_new(db, key, value, *lease, create_hint)
        }
        DcsCommand::Delete { key } => {
            if get_kv(db, key).is_none() {
                return revision(db);
            }
            let rev = revision(db)? + 1;
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
    let rev = revision(db)? + 1;
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
    fn apply_create_does_not_overwrite_existing() {
        let (dir, mut db) = temp_db();
        let first = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-a".to_vec(),
            lease: 5_000,
        };
        let rev = apply_dcs_command(&mut db, &first).unwrap();
        let steal = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-b".to_vec(),
            lease: 9_000,
        };
        assert_eq!(apply_dcs_command(&mut db, &steal).unwrap(), rev);
        assert_eq!(dcs_get(&db, b"lock").unwrap().value, b"holder-a");
        let steal0 = DcsCommand::Cas {
            key: b"lock".to_vec(),
            value: b"holder-c".to_vec(),
            expected_rev: 0,
            lease: 9_000,
        };
        assert_eq!(apply_dcs_command(&mut db, &steal0).unwrap(), rev);
        assert_eq!(dcs_get(&db, b"lock").unwrap().value, b"holder-a");
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_then_cas_takes_expired_lock_second_loses() {
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
        // After expiry, create-if-absent is allowed again at pre-check.
        assert!(check_command_at(&db, &cmd, 1_000).is_ok());
        let cmd2 = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-b".to_vec(),
            lease: 5_000,
        };
        // Raw Create apply must not steal (corpse still on disk).
        apply_dcs_command(&mut db, &cmd2).unwrap();
        assert_eq!(dcs_get(&db, b"lock").unwrap().value, b"holder-a");
        let bound = bind_absent_create(&db, cmd2, 1_000);
        assert!(
            matches!(
                &bound,
                DcsCommand::Cas {
                    expected_rev: 1,
                    value,
                    ..
                } if value == b"holder-b"
            ),
            "expired corpse must bind to Cas(rev=1), got {bound:?}"
        );
        let cmd3 = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-c".to_vec(),
            lease: 5_000,
        };
        let bound_lose = bind_absent_create(&db, cmd3, 1_000);
        let rev = apply_dcs_command(&mut db, &bound).unwrap();
        assert_eq!(rev, 2);
        assert_eq!(dcs_get_at(&db, b"lock", 1_001).unwrap().value, b"holder-b");
        // Second binder (same corpse rev) no-ops; first holder stays.
        assert_eq!(apply_dcs_command(&mut db, &bound_lose).unwrap(), 2);
        assert_eq!(dcs_get(&db, b"lock").unwrap().value, b"holder-b");
        // Live key: bind must not rewrite Create into a steal Cas.
        let live_create = DcsCommand::Create {
            key: b"lock".to_vec(),
            value: b"holder-d".to_vec(),
            lease: 9_000,
        };
        assert!(matches!(
            bind_absent_create(&db, live_create, 1_001),
            DcsCommand::Create { .. }
        ));
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F113: garbage `d/rev` must not reuse revision 1 on apply.
    #[test]
    fn apply_rejects_truncated_cluster_revision() {
        let (dir, mut db) = temp_db();
        let cmd = DcsCommand::Put {
            key: b"a".to_vec(),
            value: b"1".to_vec(),
            lease: 0,
        };
        assert_eq!(apply_dcs_command(&mut db, &cmd).unwrap(), 1);
        db.put(REV_KEY, b"xx").unwrap();
        let cmd2 = DcsCommand::Put {
            key: b"b".to_vec(),
            value: b"2".to_vec(),
            lease: 0,
        };
        let err = apply_dcs_command(&mut db, &cmd2);
        assert!(
            err.is_err(),
            "corrupt cluster rev must fail closed on apply: {err:?}"
        );
        assert_eq!(dcs_get(&db, b"a").unwrap().mod_revision, 1);
        assert!(dcs_get(&db, b"b").is_none());
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F115: corrupt meta must not look absent on apply Create.
    #[test]
    fn apply_create_rejects_undecodable_meta() {
        let (dir, mut db) = temp_db();
        let cmd = DcsCommand::Put {
            key: b"a".to_vec(),
            value: b"keep".to_vec(),
            lease: 0,
        };
        apply_dcs_command(&mut db, &cmd).unwrap();
        db.put(meta_key(b"a"), b"xx").unwrap();
        let steal = DcsCommand::Create {
            key: b"a".to_vec(),
            value: b"steal".to_vec(),
            lease: 0,
        };
        let err = apply_dcs_command(&mut db, &steal);
        assert!(
            err.is_err(),
            "apply Create must not overwrite corrupt-meta key: {err:?}"
        );
        assert_eq!(db.get(&kv_key(b"a")).as_deref(), Some(b"keep".as_ref()));
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
