//! etcd-class **DCS state machine** on PedraDB (RFC-0010 P2.1).
//!
//! This is **not** the etcd wire protocol. It implements the **semantics**
//! Patroni (and similar HA managers) need from a Distributed Configuration Store:
//!
//! | Operation | Method |
//! |-----------|--------|
//! | Get / put with revision | [`Dcs::get`], [`Dcs::put`] |
//! | Create-if-absent (leader race) | [`Dcs::create`] |
//! | Compare-and-swap by revision | [`Dcs::cas`] |
//! | Delete | [`Dcs::delete`] |
//! | Leases (TTL) | [`Dcs::grant_lease`], [`Dcs::keepalive`], [`Dcs::revoke_lease`] |
//! | Watch prefix / key | [`Dcs::watch_prefix`], [`Dcs::poll_watch`] |
//! | Leader lock helper | [`Dcs::try_acquire_leader`], [`Dcs::renew_leader`] |
//!
//! Multi-key TX in PedraDB provides atomic CAS. Time is **injected** via
//! [`Clock`] so tests stay deterministic.
//!
//! Layering: Patroni plugin / etcd API adapter → **this crate** → PedraDB.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod apply_kernel;
pub mod command;
pub mod lease_kernel;

pub use apply_kernel::{
    dcs_apply_should_advance, dcs_apply_should_advance_as_is, dcs_apply_should_advance_result,
};
pub use command::{
    apply_dcs_command, bind_absent_create, check_command, check_command_at, dcs_get, dcs_get_at,
    DcsCommand, DCS_CMD_MARKER,
};
pub use lease_kernel::{
    lease_live, lease_table_expired, lease_table_expired_as_is, next_lease_id_after,
    next_lease_id_as_is,
};

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use pedradb_core::{Db, Env, Host, OpenOptions, Result as CoreResult, StdEnv};
use thiserror::Error;

// Re-export core time / host seams so DCS and store share one DST plug surface.
pub use pedradb_core::{Clock, DetHost, ManualClock, SystemClock};

/// DCS errors.
#[derive(Debug, Error)]
pub enum DcsError {
    /// Engine I/O.
    #[error("pedradb: {0}")]
    Core(#[from] pedradb_core::CoreError),
    /// CAS / create precondition failed.
    #[error("cas failed: {0}")]
    CasFailed(&'static str),
    /// Unknown or expired lease.
    #[error("lease {0} not found or expired")]
    LeaseNotFound(u64),
    /// Encoding / corruption of DCS metadata.
    #[error("corrupt dcs record: {0}")]
    Corrupt(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, DcsError>;

/// Key-value entry with etcd-like metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValue {
    /// User key bytes.
    pub key: Vec<u8>,
    /// Value bytes.
    pub value: Vec<u8>,
    /// Monotonic cluster revision when written.
    pub mod_revision: u64,
    /// Revision when the key was first created.
    pub create_revision: u64,
    /// Bound lease id (`0` = none).
    pub lease: u64,
}

/// Watch event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Key put / created.
    Put(KeyValue),
    /// Key deleted.
    Delete {
        /// Key.
        key: Vec<u8>,
        /// Revision of the delete.
        mod_revision: u64,
    },
}

/// Active watch handle (poll-based).
#[derive(Debug, Clone)]
pub struct Watch {
    /// Watch id.
    pub id: u64,
    /// Key prefix (empty = all).
    pub prefix: Vec<u8>,
    /// Report events with `mod_revision > since_revision`.
    pub since_revision: u64,
}

// On-disk layout (byte keys):
//   d/k/{user_key}     → value payload
//   d/m/{user_key}     → meta: create_rev u64 | mod_rev u64 | lease u64  (24 bytes)
//   d/rev              → global revision u64
//   d/lease/{id}       → expiry_ms u64 (manual: stored as duration offset from epoch of lease grant — we store absolute Instant as nanos in process memory only)
//
// Leases are process-local (TTL in memory + optional durability of binding on keys).
// Leases are process-local (not durable). On reopen the table is empty; bindings
// with a non-zero lease id are treated as **expired** on read (fail-safe for HA).
// Orphaned key/meta bytes may still sit on disk until overwritten or GC.

const META_LEN: usize = 24;

fn push_user_component(buf: &mut Vec<u8>, user: &[u8]) {
    let n = u32::try_from(user.len()).expect("dcs user key len fits u32");
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(user);
}

/// Decode user after `d/k/` or `d/m/` (F98 length-prefix; legacy raw accepted).
fn user_from_disk_suffix(rest: &[u8]) -> Option<Vec<u8>> {
    if rest.len() >= 4 {
        let n = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
        if rest.len() == 4 + n {
            return Some(rest[4..].to_vec());
        }
    }
    // Pre-F98: raw user bytes after the fixed prefix.
    Some(rest.to_vec())
}

/// F98: `d/k/ || u32be(len) || user` so `d/k/a` is not a byte-prefix of `d/k/ab`.
pub(crate) fn kv_key(user: &[u8]) -> Vec<u8> {
    let mut k = b"d/k/".to_vec();
    push_user_component(&mut k, user);
    k
}

/// F98: `d/m/ || u32be(len) || user` (paired with [`kv_key`]).
pub(crate) fn meta_key(user: &[u8]) -> Vec<u8> {
    let mut k = b"d/m/".to_vec();
    push_user_component(&mut k, user);
    k
}

pub(crate) const REV_KEY: &[u8] = b"d/rev";

pub(crate) fn encode_meta(create: u64, mod_rev: u64, lease: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(META_LEN);
    v.extend_from_slice(&create.to_le_bytes());
    v.extend_from_slice(&mod_rev.to_le_bytes());
    v.extend_from_slice(&lease.to_le_bytes());
    v
}

pub(crate) fn decode_meta(raw: &[u8]) -> Result<(u64, u64, u64)> {
    if raw.len() < META_LEN {
        return Err(DcsError::Corrupt("meta short".into()));
    }
    let create = u64::from_le_bytes(raw[0..8].try_into().unwrap());
    let mod_rev = u64::from_le_bytes(raw[8..16].try_into().unwrap());
    let lease = u64::from_le_bytes(raw[16..24].try_into().unwrap());
    Ok((create, mod_rev, lease))
}

pub(crate) fn encode_u64(n: u64) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

pub(crate) fn decode_u64(raw: &[u8]) -> Result<u64> {
    if raw.len() < 8 {
        return Err(DcsError::Corrupt("u64 short".into()));
    }
    Ok(u64::from_le_bytes(raw[0..8].try_into().unwrap()))
}

/// Lease table entry.
#[derive(Debug, Clone)]
struct Lease {
    ttl: Duration,
    expiry: Instant,
}

/// DCS facade over a single PedraDB directory (one member's state machine).
///
/// Multi-node agreement is the job of Raft (`pedradb-raft`); this type is the
/// **state machine** + local API.
///
/// Generic over [`Clock`] (lease TTL) and [`Env`] (disk faults via `FailingEnv`).
pub struct Dcs<C: Clock = SystemClock, E: Env = StdEnv> {
    db: Db<E>,
    clock: C,
    next_lease_id: u64,
    leases: HashMap<u64, Lease>,
    next_watch_id: u64,
    /// Buffered events for watches (filled on mutating ops).
    watch_log: Vec<(u64, Event)>, // (revision, event)
}

impl Dcs<SystemClock, StdEnv> {
    /// Open or create a DCS store at `path` (production wall clock + real FS).
    ///
    /// # Errors
    /// PedraDB open.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_env_clock(path, StdEnv, SystemClock)
    }
}

impl<C: Clock> Dcs<C, StdEnv> {
    /// Open with an explicit clock (tests); real filesystem.
    ///
    /// # Errors
    /// PedraDB open.
    pub fn open_with_clock(path: impl AsRef<Path>, clock: C) -> Result<Self> {
        Self::open_with_env_clock(path, StdEnv, clock)
    }
}

impl<C: Clock, E: Env> Dcs<C, E> {
    /// Open with injectable disk + clock (DST: `FailingEnv` + `ManualClock`).
    ///
    /// # Errors
    /// PedraDB open.
    pub fn open_with_env_clock(path: impl AsRef<Path>, env: E, clock: C) -> Result<Self> {
        let db = Db::open_with_env(
            path,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
            env,
        )?;
        Self::from_open_db(db, clock)
    }

    /// Open from a [`Host`] (env + clock; RNG unused at DCS layer).
    ///
    /// # Errors
    /// PedraDB open.
    pub fn open_with_host(
        path: impl AsRef<Path>,
        host: &impl Host<Env = E, Clock = C>,
    ) -> Result<Self> {
        Self::open_with_env_clock(path, host.env().clone(), host.clock().clone())
    }

    fn from_open_db(db: Db<E>, clock: C) -> Result<Self> {
        // F7: leases are process-local. On restart the table is empty. Two
        // hazards if we naively start `next_lease_id` at 1 with orphans on disk:
        //   1) granting reuses id N that is still bound to a leader key → that
        //      key becomes "live" again under a new holder (immortal / reanimated
        //      lease) and create-if-absent fails with "key exists";
        //   2) even without create, get() would re-serve the dead holder's value.
        // Fix: never reuse ids that appear in meta, and drop orphaned leased keys
        // so HA can re-acquire immediately (etcd-class fail-safe for non-durable TTL).
        let metas = db.range(
            std::ops::Bound::Included(b"d/m/".as_slice()),
            std::ops::Bound::Excluded(b"d/m0".as_slice()),
        );
        let mut max_lease = 0u64;
        let mut orphan_users: Vec<Vec<u8>> = Vec::new();
        for (mk, mv) in metas {
            if let Ok((_, _, lease)) = decode_meta(&mv) {
                if lease != 0 {
                    max_lease = max_lease.max(lease);
                    if let Some(rest) = mk.as_ref().strip_prefix(b"d/m/") {
                        if let Some(user) = user_from_disk_suffix(rest) {
                            orphan_users.push(user);
                        }
                    }
                }
            }
        }
        let mut this = Self {
            db,
            clock,
            next_lease_id: lease_kernel::next_lease_id_after(max_lease),
            leases: HashMap::new(),
            next_watch_id: 1,
            watch_log: Vec::new(),
        };
        for user in orphan_users {
            this.delete_raw_orphan(&user)?;
        }
        Ok(this)
    }

    /// Load durable cluster revision. Missing → 0; present but short/corrupt → Err (F113).
    fn load_revision(&self) -> Result<u64> {
        match self.db.get(REV_KEY) {
            None => Ok(0),
            Some(b) => decode_u64(&b),
        }
    }

    /// Global cluster revision (0 if empty or unreadable). Mutating paths use
    /// [`Self::load_revision`] so a torn `d/rev` cannot reuse revision 1 (F113).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.load_revision().unwrap_or(0)
    }

    fn bump_revision(&mut self) -> Result<u64> {
        let r = self.load_revision()? + 1;
        self.db.put(REV_KEY, encode_u64(r))?;
        Ok(r)
    }

    /// Read a key.
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<KeyValue> {
        let val = self.db.get(&kv_key(key))?;
        let meta = self.db.get(&meta_key(key))?;
        let (create, mod_rev, lease) = decode_meta(&meta).ok()?;
        if lease != 0 && self.lease_expired(lease) {
            return None;
        }
        Some(KeyValue {
            key: key.to_vec(),
            value: val.to_vec(),
            mod_revision: mod_rev,
            create_revision: create,
            lease,
        })
    }

    fn lease_expired(&self, id: u64) -> bool {
        // F7: unknown id → expired. Clock compare stays in the caller (Instant).
        let hit = self.leases.get(&id).map(|l| self.clock.now() >= l.expiry);
        lease_kernel::lease_table_expired(hit)
    }

    /// Unconditional put (creates or overwrites). Returns new revision.
    ///
    /// # Errors
    /// I/O or lease missing.
    pub fn put(&mut self, key: &[u8], value: &[u8], lease: u64) -> Result<u64> {
        if lease != 0 && !self.leases.contains_key(&lease) {
            return Err(DcsError::LeaseNotFound(lease));
        }
        let prev = self.get(key);
        let rev = self.bump_revision()?;
        let create = prev.as_ref().map_or(rev, |p| p.create_revision);
        let mut tx = self.db.begin();
        tx.put(kv_key(key), value)?;
        tx.put(meta_key(key), encode_meta(create, rev, lease))?;
        tx.commit()?;
        let kv = KeyValue {
            key: key.to_vec(),
            value: value.to_vec(),
            mod_revision: rev,
            create_revision: create,
            lease,
        };
        self.watch_log.push((rev, Event::Put(kv)));
        Ok(rev)
    }

    /// Create key only if absent (leader race). Returns revision or `CasFailed`.
    ///
    /// Logical absence wins: a raw key whose lease is unknown/expired (e.g. after
    /// process restart wiped the in-memory lease table) does **not** block create —
    /// [`put`] overwrites the orphan. This is required for HA leader re-election (F7).
    ///
    /// # Errors
    /// I/O or key exists (live binding).
    pub fn create(&mut self, key: &[u8], value: &[u8], lease: u64) -> Result<u64> {
        if self.get(key).is_some() {
            return Err(DcsError::CasFailed("key exists"));
        }
        // Logical get is None — drop any orphan raw kv/meta so create-if-absent
        // stays a pure insert when lease=0, and put path stays consistent.
        self.delete_raw_orphan(key)?;
        self.put(key, value, lease)
    }

    /// F115: present meta that does not decode is not "absent" (do not GC / overwrite).
    fn reject_undecodable_meta(&self, key: &[u8]) -> Result<()> {
        if let Some(raw) = self.db.get(&meta_key(key)) {
            decode_meta(&raw)?;
        }
        Ok(())
    }

    /// Remove raw `d/k/` + `d/m/` rows without bumping revision / watch (orphan GC).
    fn delete_raw_orphan(&mut self, key: &[u8]) -> Result<()> {
        self.reject_undecodable_meta(key)?;
        if self.db.get(&kv_key(key)).is_none() && self.db.get(&meta_key(key)).is_none() {
            return Ok(());
        }
        let mut tx = self.db.begin();
        tx.delete(kv_key(key))?;
        tx.delete(meta_key(key))?;
        tx.commit()?;
        Ok(())
    }

    /// Compare-and-swap: succeed only if current `mod_revision == expected_rev`.
    /// Use `expected_rev == 0` for create-if-absent (same as [`Dcs::create`]).
    ///
    /// # Errors
    /// I/O or revision mismatch.
    pub fn cas(&mut self, key: &[u8], value: &[u8], expected_rev: u64, lease: u64) -> Result<u64> {
        let cur = self.get(key);
        match (expected_rev, cur) {
            (0, Some(_)) => return Err(DcsError::CasFailed("expected absent")),
            (0, None) => self.reject_undecodable_meta(key)?,
            (r, Some(kv)) if kv.mod_revision == r => {}
            (_, None) => return Err(DcsError::CasFailed("key missing")),
            _ => return Err(DcsError::CasFailed("revision mismatch")),
        }
        self.put(key, value, lease)
    }

    /// Delete key; returns revision of delete, or `None` if absent.
    ///
    /// # Errors
    /// I/O.
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<u64>> {
        if self.get(key).is_none() {
            return Ok(None);
        }
        let rev = self.bump_revision()?;
        let mut tx = self.db.begin();
        tx.delete(kv_key(key))?;
        tx.delete(meta_key(key))?;
        tx.commit()?;
        self.watch_log.push((
            rev,
            Event::Delete {
                key: key.to_vec(),
                mod_revision: rev,
            },
        ));
        Ok(Some(rev))
    }

    /// Grant a lease with TTL. Returns lease id.
    pub fn grant_lease(&mut self, ttl: Duration) -> u64 {
        let id = self.next_lease_id;
        self.next_lease_id += 1;
        let expiry = self.clock.now() + ttl;
        self.leases.insert(id, Lease { ttl, expiry });
        id
    }

    /// Keepalive: refresh expiry to now+ttl.
    ///
    /// # Errors
    /// Unknown lease.
    pub fn keepalive(&mut self, id: u64) -> Result<()> {
        let now = self.clock.now();
        let lease = self
            .leases
            .get_mut(&id)
            .ok_or(DcsError::LeaseNotFound(id))?;
        lease.expiry = now + lease.ttl;
        Ok(())
    }

    /// Revoke lease and delete bound keys (scan meta — O(n) keys with prefix).
    ///
    /// # Errors
    /// I/O.
    pub fn revoke_lease(&mut self, id: u64) -> Result<()> {
        self.leases.remove(&id);
        // Scan all keys with d/m/ prefix for lease id.
        let metas = self.db.range(
            std::ops::Bound::Included(b"d/m/".as_slice()),
            std::ops::Bound::Excluded(b"d/m0".as_slice()),
        );
        let mut to_delete = Vec::new();
        for (mk, mv) in metas {
            if let Ok((_, _, lease)) = decode_meta(&mv) {
                if lease == id {
                    if let Some(rest) = mk.as_ref().strip_prefix(b"d/m/") {
                        if let Some(user) = user_from_disk_suffix(rest) {
                            to_delete.push(user);
                        }
                    }
                }
            }
        }
        for k in to_delete {
            let _ = self.delete(&k)?;
        }
        Ok(())
    }

    /// Expire leases that are past TTL and drop their keys.
    ///
    /// # Errors
    /// I/O while deleting.
    pub fn expire_leases(&mut self) -> Result<Vec<u64>> {
        let now = self.clock.now();
        let expired: Vec<u64> = self
            .leases
            .iter()
            .filter(|(_, l)| now >= l.expiry)
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            self.revoke_lease(*id)?;
        }
        Ok(expired)
    }

    /// Register a prefix watch from current revision.
    pub fn watch_prefix(&mut self, prefix: &[u8]) -> Watch {
        let id = self.next_watch_id;
        self.next_watch_id += 1;
        Watch {
            id,
            prefix: prefix.to_vec(),
            since_revision: self.revision(),
        }
    }

    /// Poll events for a watch since its cursor; advances `since_revision`.
    pub fn poll_watch(&self, watch: &mut Watch) -> Vec<Event> {
        let mut out = Vec::new();
        let mut max_rev = watch.since_revision;
        for (rev, ev) in &self.watch_log {
            if *rev <= watch.since_revision {
                continue;
            }
            let key = match ev {
                Event::Put(kv) => kv.key.as_slice(),
                Event::Delete { key, .. } => key.as_slice(),
            };
            if key.starts_with(&watch.prefix) {
                out.push(ev.clone());
                max_rev = max_rev.max(*rev);
            }
        }
        watch.since_revision = max_rev;
        out
    }

    /// Try to acquire leader key with a new lease (create-if-absent).
    ///
    /// # Errors
    /// Key held by another / I/O.
    pub fn try_acquire_leader(
        &mut self,
        leader_key: &[u8],
        holder: &[u8],
        ttl: Duration,
    ) -> Result<(u64, u64)> {
        let lease = self.grant_lease(ttl);
        match self.create(leader_key, holder, lease) {
            Ok(rev) => Ok((rev, lease)),
            Err(e) => {
                self.leases.remove(&lease);
                Err(e)
            }
        }
    }

    /// Renew leader: keepalive lease + optional value refresh via CAS.
    ///
    /// # Errors
    /// Lost leadership or I/O.
    pub fn renew_leader(
        &mut self,
        leader_key: &[u8],
        holder: &[u8],
        lease: u64,
        expected_rev: u64,
    ) -> Result<u64> {
        self.keepalive(lease)?;
        let cur = self
            .get(leader_key)
            .ok_or(DcsError::CasFailed("leader key gone"))?;
        if cur.value != holder {
            return Err(DcsError::CasFailed("not holder"));
        }
        self.cas(leader_key, holder, expected_rev, lease)
    }

    /// Close underlying DB.
    ///
    /// # Errors
    /// WAL close.
    pub fn close(self) -> CoreResult<()> {
        self.db.close()
    }

    /// Mutable clock (tests / injected time sources).
    pub fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }

    /// Shared reference to the clock (e.g. [`pedradb_core::ManualClock::advance`] takes `&self`).
    pub fn clock(&self) -> &C {
        &self.clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-dcs-{tag}-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn put_get_cas_delete() {
        let dir = temp_dir("kv");
        let mut dcs = Dcs::open(&dir).unwrap();
        let r1 = dcs.put(b"a", b"1", 0).unwrap();
        assert_eq!(r1, 1);
        let kv = dcs.get(b"a").unwrap();
        assert_eq!(kv.value, b"1");
        assert_eq!(kv.mod_revision, 1);

        assert!(dcs.cas(b"a", b"2", 99, 0).is_err());
        let r2 = dcs.cas(b"a", b"2", 1, 0).unwrap();
        assert_eq!(dcs.get(b"a").unwrap().value, b"2");
        assert_eq!(r2, 2);

        dcs.delete(b"a").unwrap();
        assert!(dcs.get(b"a").is_none());
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F98: raw `d/k/||user` made `d/k/a` a prefix of `d/k/ab`.
    #[test]
    fn kv_meta_keys_not_prefix_of_sibling_user() {
        let ka = kv_key(b"a");
        let kab = kv_key(b"ab");
        let ma = meta_key(b"a");
        let mab = meta_key(b"ab");
        assert!(
            !kab.starts_with(&ka),
            "kv_key(a) must not prefix kv_key(ab): {ka:?} vs {kab:?}"
        );
        assert!(
            !mab.starts_with(&ma),
            "meta_key(a) must not prefix meta_key(ab): {ma:?} vs {mab:?}"
        );
        let dir = temp_dir("pref");
        let mut dcs = Dcs::open(&dir).unwrap();
        dcs.put(b"a", b"va", 0).unwrap();
        dcs.put(b"ab", b"vab", 0).unwrap();
        assert_eq!(dcs.get(b"a").unwrap().value, b"va");
        assert_eq!(dcs.get(b"ab").unwrap().value, b"vab");
        // Disk scan of exact kv_key(a) half-open must not include ab.
        let end = pedradb_core::prefix_exclusive_end(&ka);
        let hits = dcs.db.range(
            std::ops::Bound::Included(ka.as_slice()),
            match end.as_deref() {
                Some(e) => std::ops::Bound::Excluded(e),
                None => std::ops::Bound::Unbounded,
            },
        );
        assert_eq!(hits.len(), 1, "prefix scan of kv_key(a) leaked: {hits:?}");
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_if_absent_leader_race() {
        let dir = temp_dir("lead");
        let mut dcs = Dcs::open(&dir).unwrap();
        dcs.create(b"/leader", b"node-a", 0).unwrap();
        assert!(matches!(
            dcs.create(b"/leader", b"node-b", 0),
            Err(DcsError::CasFailed(_))
        ));
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lease_revoke_deletes_bound_keys() {
        let dir = temp_dir("lease");
        let mut dcs = Dcs::open(&dir).unwrap();
        let lease = dcs.grant_lease(Duration::from_secs(10));
        dcs.put(b"locked", b"x", lease).unwrap();
        assert!(dcs.get(b"locked").is_some());
        dcs.revoke_lease(lease).unwrap();
        assert!(dcs.get(b"locked").is_none());
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lease_expire_advances_manual_clock() {
        let dir = temp_dir("ttl");
        let mut dcs = Dcs::open_with_clock(&dir, ManualClock::new()).unwrap();
        let lease = dcs.grant_lease(Duration::from_millis(5));
        dcs.put(b"k", b"v", lease).unwrap();
        dcs.clock_mut().advance(Duration::from_secs(1));
        let expired = dcs.expire_leases().unwrap();
        assert_eq!(expired, vec![lease]);
        assert!(dcs.get(b"k").is_none());
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_with_host_uses_manual_clock() {
        use pedradb_core::{DetHost, Host, StdEnv};
        let dir = temp_dir("host");
        let host = DetHost::with_seed(StdEnv, 99);
        let mut dcs = Dcs::open_with_host(&dir, &host).unwrap();
        let lease = dcs.grant_lease(Duration::from_millis(1));
        dcs.put(b"k", b"v", lease).unwrap();
        // Advance via host handle (shared ManualClock state).
        host.clock().advance(Duration::from_secs(1));
        let expired = dcs.expire_leases().unwrap();
        assert_eq!(expired, vec![lease]);
        assert!(dcs.get(b"k").is_none());
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_prefix_events() {
        let dir = temp_dir("watch");
        let mut dcs = Dcs::open(&dir).unwrap();
        let mut w = dcs.watch_prefix(b"svc/");
        assert!(dcs.poll_watch(&mut w).is_empty());
        dcs.put(b"svc/a", b"1", 0).unwrap();
        dcs.put(b"other", b"x", 0).unwrap();
        dcs.put(b"svc/b", b"2", 0).unwrap();
        let events = dcs.poll_watch(&mut w);
        assert_eq!(events.len(), 2);
        dcs.delete(b"svc/a").unwrap();
        let events = dcs.poll_watch(&mut w);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Delete { .. }));
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn try_acquire_and_renew_leader() {
        let dir = temp_dir("patroni");
        let mut dcs = Dcs::open(&dir).unwrap();
        let (rev, lease) = dcs
            .try_acquire_leader(b"/pg/leader", b"node-1", Duration::from_secs(30))
            .unwrap();
        assert!(rev >= 1);
        let rev2 = dcs
            .renew_leader(b"/pg/leader", b"node-1", lease, rev)
            .unwrap();
        assert!(rev2 > rev);
        // Other holder fails create.
        assert!(dcs
            .try_acquire_leader(b"/pg/leader", b"node-2", Duration::from_secs(30))
            .is_err());
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F7: after process restart, leased leader keys must not stay held forever.
    ///
    /// Lease table is in-memory only; reopen wipes it. Unknown lease ids are
    /// treated as expired, lease ids are never reused against disk orphans, and
    /// orphaned leased keys are GC'd on open so HA can re-acquire.
    #[test]
    fn leased_key_not_immortal_after_dcs_reopen() {
        let dir = temp_dir("lease-reopen");
        {
            let mut dcs = Dcs::open(&dir).unwrap();
            let lease = dcs.grant_lease(Duration::from_secs(3600));
            dcs.put(b"/leader", b"old-node", lease).unwrap();
            assert!(dcs.get(b"/leader").is_some());
            dcs.close().unwrap();
        }
        let mut dcs = Dcs::open(&dir).unwrap();
        assert!(
            dcs.get(b"/leader").is_none(),
            "stale lease binding must not survive restart as live (F7)"
        );
        // New leader wins create-if-absent after restart (orphan GC + logical hide).
        dcs.create(b"/leader", b"new-node", 0)
            .expect("new leader must win create after DCS restart");
        let kv = dcs
            .get(b"/leader")
            .expect("leader key present after take-over");
        assert_eq!(kv.value, b"new-node");
        dcs.delete(b"/leader").unwrap();
        let (rev, _lease) = dcs
            .try_acquire_leader(b"/pg/leader", b"node-2", Duration::from_secs(30))
            .expect("try_acquire on fresh key");
        assert!(rev >= 1);
        dcs.close().unwrap();
        // Full HA path: acquire → process exit → reopen → other node acquires.
        {
            let mut dcs = Dcs::open(&dir).unwrap();
            dcs.try_acquire_leader(b"/ha/leader", b"node-a", Duration::from_secs(3600))
                .unwrap();
            dcs.close().unwrap();
        }
        {
            let mut dcs = Dcs::open(&dir).unwrap();
            assert!(dcs.get(b"/ha/leader").is_none());
            dcs.try_acquire_leader(b"/ha/leader", b"node-b", Duration::from_secs(30))
                .expect("new leader must win after DCS restart");
            assert_eq!(dcs.get(b"/ha/leader").unwrap().value, b"node-b");
            dcs.close().unwrap();
        }
        // Lease id must not re-animate a different key that still referenced it
        // if GC were skipped — open advances next_lease_id past disk max.
        {
            let mut dcs = Dcs::open(&dir).unwrap();
            let l1 = dcs.grant_lease(Duration::from_secs(60));
            dcs.put(b"/k1", b"v1", l1).unwrap();
            // leave /k1 leased and close without revoke
            dcs.close().unwrap();
        }
        {
            let mut dcs = Dcs::open(&dir).unwrap();
            assert!(dcs.get(b"/k1").is_none());
            let l2 = dcs.grant_lease(Duration::from_secs(60));
            // l2 must not equal any id that was on disk at crash (would reanimate).
            dcs.put(b"/k2", b"v2", l2).unwrap();
            assert!(
                dcs.get(b"/k1").is_none(),
                "grant must not reanimate orphan /k1"
            );
            assert_eq!(dcs.get(b"/k2").unwrap().value, b"v2");
            dcs.close().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F113: garbage `d/rev` parsed as 0 and reused revision 1.
    #[test]
    fn put_rejects_truncated_cluster_revision() {
        let dir = temp_dir("rev-corrupt");
        let mut dcs = Dcs::open(&dir).unwrap();
        let r1 = dcs.put(b"a", b"1", 0).unwrap();
        assert_eq!(r1, 1);
        dcs.db.put(REV_KEY, b"xx").unwrap();
        let err = dcs.put(b"b", b"2", 0);
        assert!(
            err.is_err(),
            "corrupt cluster rev must fail closed, not reuse 1: {err:?}"
        );
        assert_eq!(
            dcs.get(b"a").unwrap().mod_revision,
            1,
            "first key must keep its revision"
        );
        assert!(
            dcs.get(b"b").is_none(),
            "second put must not land under a reused revision"
        );
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F115: corrupt meta made get() miss and create-if-absent overwrite the value.
    #[test]
    fn create_rejects_undecodable_meta() {
        let dir = temp_dir("meta-corrupt");
        let mut dcs = Dcs::open(&dir).unwrap();
        dcs.put(b"a", b"keep", 0).unwrap();
        dcs.db.put(meta_key(b"a"), b"xx").unwrap();
        assert!(
            dcs.get(b"a").is_none(),
            "get stays Option: corrupt meta is not a live binding"
        );
        let err = dcs.create(b"a", b"steal", 0);
        assert!(
            err.is_err(),
            "create must not treat corrupt meta as absent: {err:?}"
        );
        assert_eq!(
            dcs.db.get(&kv_key(b"a")).as_deref(),
            Some(b"keep".as_ref()),
            "raw value must survive failed create"
        );
        dcs.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
