//! Pedra lease canary (RFC-0020 P0.3–P0.4).
//!
//! Thin product layer over **public** Pedra APIs only (`put_if_absent`,
//! `put_if_eq` / `compare_and_swap`, `get`, `multi_get`, commit seq). Used to
//! pressure CAS + durability under concurrent contenders and crash/reopen.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use bytes::Bytes;
use pedradb_core::{ConcurrentDb, CoreError, Db, Env, OpenOptions, Result, SequenceNumber, StdEnv};
use std::path::Path;

/// Key prefix for lease records.
pub const LEASE_PREFIX: &[u8] = b"lease/";

fn push_len_pref(buf: &mut Vec<u8>, part: &[u8]) {
    let n = u32::try_from(part.len()).expect("lease name len fits u32");
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(part);
}

/// Build the storage key for a lease name.
///
/// F90: raw `lease/` || name made `lease/a` a byte-prefix of `lease/ab`.
/// Length-prefix the name (same framing as `pedradb-index` `row_key`, F89).
#[must_use]
pub fn lease_key(name: impl AsRef<[u8]>) -> Vec<u8> {
    let n = name.as_ref();
    let mut k = Vec::with_capacity(LEASE_PREFIX.len() + 4 + n.len());
    k.extend_from_slice(LEASE_PREFIX);
    push_len_pref(&mut k, n);
    k
}

/// Exclusive lease handle over a [`ConcurrentDb`] (thread-safe CAS path).
#[derive(Clone)]
pub struct LeaseStore<E: Env = StdEnv> {
    db: ConcurrentDb<E>,
}

impl LeaseStore<StdEnv> {
    /// Open a lease store on the real filesystem.
    ///
    /// # Errors
    /// Same as [`ConcurrentDb::open_with`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(
            path,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
    }

    /// Open with options.
    ///
    /// # Errors
    /// Open I/O.
    pub fn open_with(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        Ok(Self {
            db: ConcurrentDb::open_with(path, opts)?,
        })
    }
}

impl<E: Env> LeaseStore<E> {
    /// Wrap an existing concurrent DB (e.g. after `Db::open_with_env` + convert).
    #[must_use]
    pub fn from_concurrent(db: ConcurrentDb<E>) -> Self {
        Self { db }
    }

    /// Wrap a single-threaded [`Db`] (becomes concurrent).
    #[must_use]
    pub fn from_db(db: Db<E>) -> Self {
        Self {
            db: ConcurrentDb::from_db(db),
        }
    }

    /// Underlying concurrent handle (tests / multi_get).
    #[must_use]
    pub fn db(&self) -> &ConcurrentDb<E> {
        &self.db
    }

    /// Current holder of `name`, if any.
    #[must_use]
    pub fn holder(&self, name: impl AsRef<[u8]>) -> Option<Bytes> {
        self.db.get(&lease_key(name))
    }

    /// Try to acquire a free lease (`put_if_absent`). Returns assigned seq on win.
    ///
    /// # Errors
    /// WAL I/O. `CasMismatch` is mapped to `Ok(None)` (lost race, not an error).
    pub fn try_acquire(
        &self,
        name: impl AsRef<[u8]>,
        holder: impl AsRef<[u8]>,
    ) -> Result<Option<SequenceNumber>> {
        let key = lease_key(name);
        match self.db.put_if_absent(&key, holder.as_ref()) {
            Ok(seq) => Ok(Some(seq)),
            Err(CoreError::CasMismatch) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Transfer lease only if still held by `expected` (`put_if_eq`).
    ///
    /// # Errors
    /// WAL I/O. Mismatch → `Ok(None)`.
    pub fn try_transfer(
        &self,
        name: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        new_holder: impl AsRef<[u8]>,
    ) -> Result<Option<SequenceNumber>> {
        let key = lease_key(name);
        match self
            .db
            .put_if_eq(&key, expected.as_ref(), new_holder.as_ref())
        {
            Ok(seq) => Ok(Some(seq)),
            Err(CoreError::CasMismatch) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Release lease if held by `holder` (CAS to empty tombstone via delete after eq check).
    ///
    /// Implemented as: if holder matches, delete. Uses get+delete under ConcurrentDb
    /// write lock via `with_write` for atomicity of check+delete.
    ///
    /// # Errors
    /// WAL I/O.
    pub fn try_release(&self, name: impl AsRef<[u8]>, holder: impl AsRef<[u8]>) -> Result<bool> {
        let key = lease_key(name);
        let holder = holder.as_ref().to_vec();
        self.db.with_write(|db| match db.get(&key) {
            Some(cur) if cur.as_ref() == holder.as_slice() => {
                db.delete(&key)?;
                Ok(true)
            }
            _ => Ok(false),
        })
    }

    /// Point lookup many lease names (RFC-0019 multi_get path).
    #[must_use]
    pub fn holders(&self, names: &[&[u8]]) -> Vec<Option<Bytes>> {
        let keys: Vec<Vec<u8>> = names.iter().map(lease_key).collect();
        let refs: Vec<&[u8]> = keys.iter().map(Vec::as_slice).collect();
        // multi_get takes &[impl AsRef<[u8]>]; use intermediate
        self.db.multi_get(&refs)
    }
}

/// Workload result for limit tests (RFC-0020 W1/W2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadReport {
    /// Workload name (`hotkey-cas` / `lease-crash-reopen`).
    pub name: &'static str,
    /// Operations attempted / contenders.
    pub trials: u64,
    /// Model mismatches (must be 0).
    pub silent_wrong: u64,
    /// Optional note (winner id, path).
    pub detail: String,
}

/// W1: N concurrent `try_acquire` on one lease — exactly one winner, no double-hold.
///
/// # Errors
/// Open/I/O failures.
pub fn workload_hotkey_cas(dir: impl AsRef<Path>, contenders: usize) -> Result<WorkloadReport> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    let store = Arc::new(LeaseStore::open(dir.as_ref())?);
    let wins = Arc::new(AtomicUsize::new(0));
    let mismatches = Arc::new(AtomicUsize::new(0));
    let n = contenders.max(2);
    let barrier = Arc::new(std::sync::Barrier::new(n));
    let mut handles = Vec::new();

    for i in 0..n {
        let store = Arc::clone(&store);
        let wins = Arc::clone(&wins);
        let mismatches = Arc::clone(&mismatches);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            let holder = format!("h{i}");
            match store.try_acquire(b"hot", holder.as_bytes()) {
                Ok(Some(_)) => {
                    wins.fetch_add(1, Ordering::SeqCst);
                }
                Ok(None) => {
                    mismatches.fetch_add(1, Ordering::SeqCst);
                }
                Err(e) => panic!("hotkey-cas unexpected error: {e}"),
            }
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }

    let winners = wins.load(Ordering::SeqCst);
    let lost = mismatches.load(Ordering::SeqCst);
    let live = store.holder(b"hot");
    let mut silent_wrong = 0u64;
    if winners != 1 {
        silent_wrong += 1;
    }
    if winners + lost != n {
        silent_wrong += 1;
    }
    if live.is_none() {
        silent_wrong += 1;
    }
    // Double-hold impossible by CAS; verify single holder bytes.
    if let Some(h) = &live {
        if !h.starts_with(b"h") {
            silent_wrong += 1;
        }
    }

    Ok(WorkloadReport {
        name: "hotkey-cas",
        trials: n as u64,
        silent_wrong,
        detail: format!(
            "winners={winners} lost={lost} holder={:?}",
            live.as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
        ),
    })
}

/// W2: acquire Ok then process kill (close); reopen sees only durable holder.
///
/// # Errors
/// Open/I/O.
pub fn workload_lease_crash_reopen(dir: impl AsRef<Path>) -> Result<WorkloadReport> {
    let dir = dir.as_ref();
    {
        let store = LeaseStore::open(dir)?;
        let seq = store
            .try_acquire(b"svc", b"owner-a")?
            .expect("first acquire must win");
        assert!(seq >= 1);
        // Loser must not steal.
        assert!(store.try_acquire(b"svc", b"owner-b")?.is_none());
        assert_eq!(store.holder(b"svc").as_deref(), Some(b"owner-a".as_ref()));
        // Drop = process exit (files durable via sync puts).
        drop(store);
    }
    let store = LeaseStore::open(dir)?;
    let mut silent_wrong = 0u64;
    match store.holder(b"svc").as_deref() {
        Some(b"owner-a") => {}
        other => {
            silent_wrong += 1;
            return Ok(WorkloadReport {
                name: "lease-crash-reopen",
                trials: 1,
                silent_wrong,
                detail: format!("expected owner-a after reopen, got {other:?}"),
            });
        }
    }
    // Still exclusive.
    if store.try_acquire(b"svc", b"owner-c")?.is_some() {
        silent_wrong += 1;
    }
    Ok(WorkloadReport {
        name: "lease-crash-reopen",
        trials: 1,
        silent_wrong,
        detail: "owner-a durable after reopen".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedra-lease-{n}-{i}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn acquire_exclusive_and_transfer() {
        let dir = temp_dir();
        let store = LeaseStore::open(&dir).unwrap();
        let s1 = store.try_acquire(b"x", b"a").unwrap().unwrap();
        assert!(store.try_acquire(b"x", b"b").unwrap().is_none());
        assert_eq!(store.holder(b"x").as_deref(), Some(b"a".as_ref()));
        let s2 = store.try_transfer(b"x", b"a", b"c").unwrap().unwrap();
        assert!(s2 > s1);
        assert_eq!(store.holder(b"x").as_deref(), Some(b"c".as_ref()));
        assert!(store.try_transfer(b"x", b"a", b"d").unwrap().is_none());
        assert!(store.try_release(b"x", b"c").unwrap());
        assert!(store.holder(b"x").is_none());
        let _ = store.try_acquire(b"x", b"e").unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_cas_exactly_one_winner() {
        let dir = temp_dir();
        let r = workload_hotkey_cas(&dir, 8).unwrap();
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        assert_eq!(r.name, "hotkey-cas");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_reopen_keeps_durable_holder() {
        let dir = temp_dir();
        let r = workload_lease_crash_reopen(&dir).unwrap();
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_get_holders() {
        let dir = temp_dir();
        let store = LeaseStore::open(&dir).unwrap();
        store.try_acquire(b"a", b"1").unwrap().unwrap();
        store.try_acquire(b"b", b"2").unwrap().unwrap();
        let got = store.holders(&[b"a", b"b", b"missing"]);
        assert_eq!(got[0].as_deref(), Some(b"1".as_ref()));
        assert_eq!(got[1].as_deref(), Some(b"2".as_ref()));
        assert!(got[2].is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// W2 under FailingEnv: durable acked holder survives heal + reopen.
    #[test]
    fn lease_crash_reopen_under_failing_env() {
        use pedradb_core::{Db, DetHost};
        use pedradb_sim::FailingEnv;

        let dir = temp_dir();
        let opts = OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        };
        let env = FailingEnv::passing();
        let host = DetHost::with_seed(env.clone(), 0x0020_00A1);
        {
            let db = Db::open_with_host(&dir, opts, &host).unwrap();
            let store = LeaseStore::from_db(db);
            let acked = store.try_acquire(b"svc", b"owner-a").unwrap();
            assert!(acked.is_some(), "healthy acquire must ack");
            env.arm(0, true);
            let _ = store.try_acquire(b"svc", b"thief");
            env.disarm();
            drop(store);
        }
        env.disarm();
        let host2 = DetHost::with_seed(FailingEnv::passing(), 0x0020_00A2);
        let db = Db::open_with_host(&dir, opts, &host2).unwrap();
        let store = LeaseStore::from_db(db);
        assert_eq!(
            store.holder(b"svc").as_deref(),
            Some(b"owner-a".as_ref()),
            "acked holder must survive reopen"
        );
        assert!(store.try_acquire(b"svc", b"other").unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn named_workloads_silent_wrong_zero() {
        let d1 = temp_dir();
        let d2 = temp_dir();
        let w1 = workload_hotkey_cas(&d1, 12).unwrap();
        let w2 = workload_lease_crash_reopen(&d2).unwrap();
        assert_eq!(w1.silent_wrong, 0, "hotkey-cas {w1:?}");
        assert_eq!(w2.silent_wrong, 0, "lease-crash-reopen {w2:?}");
        let _ = fs::remove_dir_all(&d1);
        let _ = fs::remove_dir_all(&d2);
    }

    /// F90: `lease/` || name made `lease/a` a prefix of `lease/ab`.
    #[test]
    fn lease_key_name_is_not_prefix_of_sibling_name() {
        let a = lease_key(b"a");
        let ab = lease_key(b"ab");
        assert!(
            !ab.starts_with(&a),
            "lease_key(a) must not be a byte-prefix of lease_key(ab): {a:?} vs {ab:?}"
        );
        assert_ne!(a, ab);
        let dir = temp_dir();
        let store = LeaseStore::open(&dir).unwrap();
        store.try_acquire(b"a", b"ha").unwrap().unwrap();
        store.try_acquire(b"ab", b"hab").unwrap().unwrap();
        assert_eq!(store.holder(b"a").as_deref(), Some(b"ha".as_ref()));
        assert_eq!(store.holder(b"ab").as_deref(), Some(b"hab".as_ref()));
        let end = pedradb_core::prefix_exclusive_end(&a);
        let hits: Vec<_> = store
            .db()
            .range_limited(
                std::ops::Bound::Included(a.as_slice()),
                match end.as_deref() {
                    Some(e) => std::ops::Bound::Excluded(e),
                    None => std::ops::Bound::Unbounded,
                },
                None,
            )
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(
            hits.iter().any(|k| k.as_ref() == a.as_slice()),
            "own lease missing from prefix scan: {hits:?}"
        );
        assert!(
            !hits.iter().any(|k| k.as_ref() == ab.as_slice()),
            "lease_key(a) prefix scan leaked sibling ab: {hits:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
