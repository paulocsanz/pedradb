//! FDB-shaped client face for plug/test (RFC-0023 P1.1) — **not** product identity.
//!
//! Maps a small FoundationDB transaction mental model onto Montanha's native
//! [`Transaction`](crate::client::Transaction) physics. No fdbcli / full C ABI.

use crate::client::{ClientClass, Transaction};
use crate::{classify, Result, StoreCluster, StoreError};

/// FDB-like error class for harness mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdbError {
    /// `not_committed` / conflict.
    NotCommitted,
    /// `transaction_too_old`.
    TransactionTooOld,
    /// Size / limit.
    Limit,
    /// Unavailable / not leader.
    Unavailable(String),
    /// Other.
    Other(String),
}

impl FdbError {
    /// Map a store error into FDB-shaped classes.
    #[must_use]
    pub fn from_store(err: &StoreError) -> Self {
        match err {
            StoreError::TransactionTooOld { .. } => FdbError::TransactionTooOld,
            StoreError::Conflict | StoreError::TxnAborted(_) => FdbError::NotCommitted,
            StoreError::ValueTooLarge { .. } | StoreError::TransactionTooLarge { .. } => {
                FdbError::Limit
            }
            other => match classify(other) {
                ClientClass::Conflict => FdbError::NotCommitted,
                ClientClass::LimitRejected { .. } => FdbError::Limit,
                ClientClass::NotLeader { .. }
                | ClientClass::StaleLeader { .. }
                | ClientClass::NotCommitted { .. }
                | ClientClass::Unavailable(_) => FdbError::Unavailable(other.to_string()),
                ClientClass::Other(s) => FdbError::Other(s),
            },
        }
    }
}

/// Database handle (cluster reference for in-process lab).
pub struct FdbDatabase<'a> {
    cluster: &'a mut StoreCluster,
}

impl<'a> FdbDatabase<'a> {
    /// Wrap a store cluster (leadership-invisible API surface).
    pub fn open(cluster: &'a mut StoreCluster) -> Self {
        Self { cluster }
    }

    /// `create_transaction` — snapshot TX at current read version.
    #[must_use]
    pub fn create_transaction(&self) -> FdbTransaction {
        FdbTransaction {
            inner: Transaction::at_version(self.cluster.read_version()),
        }
    }

    /// Commit a transaction against this database.
    ///
    /// # Errors
    /// Store / FDB-mapped failures.
    pub fn commit(&mut self, tx: FdbTransaction) -> std::result::Result<u64, FdbError> {
        tx.inner
            .commit(self.cluster)
            .map_err(|e| FdbError::from_store(&e))
    }

    /// Raw cluster (tests).
    pub fn cluster(&mut self) -> &mut StoreCluster {
        self.cluster
    }
}

/// FDB-shaped transaction (get/set/clear/commit).
pub struct FdbTransaction {
    inner: Transaction,
}

impl FdbTransaction {
    /// Snapshot version at begin.
    #[must_use]
    pub fn read_version(&self) -> u64 {
        self.inner.snapshot_version()
    }

    /// Snapshot get.
    ///
    /// # Errors
    /// Store errors.
    pub fn get(
        &mut self,
        db: &FdbDatabase<'_>,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>> {
        self.inner.get(db.cluster, key)
    }

    /// Stage set.
    ///
    /// # Errors
    /// Limits.
    pub fn set(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.inner.set(key, value)
    }

    /// Stage clear.
    ///
    /// # Errors
    /// Limits.
    pub fn clear(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.inner.clear(key)
    }

    /// Range get + conflict range registration.
    ///
    /// # Errors
    /// Store errors.
    pub fn get_range(
        &mut self,
        db: &FdbDatabase<'_>,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.inner.get_range(db.cluster, start, end)
    }

    /// Clear all keys in half-open `[start, end)` visible at snapshot (seed).
    ///
    /// # Errors
    /// Store / limits.
    pub fn clear_range(
        &mut self,
        db: &FdbDatabase<'_>,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.inner.clear_range(db.cluster, start, end)
    }
}

/// Result of the Phase-1 bindingtester-subset harness (human-readable steps).
#[derive(Debug, Clone)]
pub struct Phase1HarnessReport {
    /// Steps that succeeded (labels).
    pub steps_ok: Vec<&'static str>,
}

/// Phase 1 harness: FDB-client-shaped sequence on a **real** [`StoreCluster`].
///
/// Covers: create_transaction → set/get/clear/commit, WW → `NotCommitted`,
/// snapshot isolation (concurrent put invisible to in-TX get + read-set Conflict),
/// too-old / limit mapping, **plus** Phase 1b: multi-key atomic commit,
/// `get_range` contents + staging overlay, range OCC conflict, out-of-range
/// write does not conflict. Not full FoundationDB bindingtester.
///
/// # Errors
/// First failed assertion as `FdbError::Other` or mapped store error.
pub fn run_phase1_bindingtester_subset(
    cluster: &mut StoreCluster,
) -> std::result::Result<Phase1HarnessReport, FdbError> {
    let mut steps_ok = Vec::new();
    let mut db = FdbDatabase::open(cluster);

    // 1) set + commit + get
    {
        let mut tr = db.create_transaction();
        tr.set(b"p1/k", b"v1")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr)?;
        let mut tr2 = db.create_transaction();
        let got = tr2
            .get(&db, b"p1/k")
            .map_err(|e| FdbError::from_store(&e))?;
        if got.as_deref() != Some(b"v1".as_ref()) {
            return Err(FdbError::Other(format!("get after set got {got:?}")));
        }
        steps_ok.push("set_get_commit");
    }

    // 2) clear + commit → absent
    {
        let mut tr = db.create_transaction();
        tr.clear(b"p1/k")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr)?;
        let mut tr2 = db.create_transaction();
        let got = tr2
            .get(&db, b"p1/k")
            .map_err(|e| FdbError::from_store(&e))?;
        if got.is_some() {
            return Err(FdbError::Other(format!(
                "clear must remove key, got {got:?}"
            )));
        }
        steps_ok.push("clear_commit");
    }

    // 3) WW conflict → NotCommitted
    {
        let mut t1 = db.create_transaction();
        let mut t2 = db.create_transaction();
        t1.set(b"p1/ww", b"a")
            .map_err(|e| FdbError::from_store(&e))?;
        t2.set(b"p1/ww", b"b")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(t1)?;
        let err = db.commit(t2).expect_err("second WW commit must fail");
        if err != FdbError::NotCommitted {
            return Err(FdbError::Other(format!(
                "expected NotCommitted, got {err:?}"
            )));
        }
        steps_ok.push("ww_not_committed");
    }

    // 4) Snapshot isolation: concurrent writer invisible; read-set Conflict on commit
    {
        db.cluster().put(b"p1/si", b"v0").map_err(|e| FdbError::from_store(&e))?;
        let mut tr = db.create_transaction();
        let snap = tr
            .get(&db, b"p1/si")
            .map_err(|e| FdbError::from_store(&e))?;
        if snap.as_deref() != Some(b"v0".as_ref()) {
            return Err(FdbError::Other(format!("si begin get {snap:?}")));
        }
        // Concurrent commit after begin (bypasses face — real cluster writer).
        db.cluster()
            .put(b"p1/si", b"v1")
            .map_err(|e| FdbError::from_store(&e))?;
        let mid = tr
            .get(&db, b"p1/si")
            .map_err(|e| FdbError::from_store(&e))?;
        if mid.as_deref() != Some(b"v0".as_ref()) {
            return Err(FdbError::Other(format!(
                "SI violation: in-TX get saw {mid:?} after concurrent put"
            )));
        }
        tr.set(b"p1/other", b"x")
            .map_err(|e| FdbError::from_store(&e))?;
        let err = db.commit(tr).expect_err("read-set must Conflict");
        if err != FdbError::NotCommitted {
            return Err(FdbError::Other(format!(
                "expected read-set NotCommitted, got {err:?}"
            )));
        }
        steps_ok.push("si_read_set_conflict");
    }

    // ── Phase 1b: deeper bindingtester subset (range + multi-key) ──────────
    // Run *before* too-old: force_safe_watermark would poison later commits.

    // 5) multi-key atomic commit via face
    {
        let mut tr = db.create_transaction();
        tr.set(b"p1b/a", b"1")
            .map_err(|e| FdbError::from_store(&e))?;
        tr.set(b"p1b/b", b"2")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr)?;
        let mut tr2 = db.create_transaction();
        let a = tr2
            .get(&db, b"p1b/a")
            .map_err(|e| FdbError::from_store(&e))?;
        let b = tr2
            .get(&db, b"p1b/b")
            .map_err(|e| FdbError::from_store(&e))?;
        if a.as_deref() != Some(b"1".as_ref()) || b.as_deref() != Some(b"2".as_ref()) {
            return Err(FdbError::Other(format!(
                "multi-key after commit a={a:?} b={b:?}"
            )));
        }
        steps_ok.push("multi_key_atomic");
    }

    // 6) get_range sees committed keys; staging overlay visible in same TX
    {
        let mut tr = db.create_transaction();
        let pairs = tr
            .get_range(&db, b"p1b/", b"p1b0")
            .map_err(|e| FdbError::from_store(&e))?;
        let keys: Vec<&[u8]> = pairs.iter().map(|(k, _)| k.as_slice()).collect();
        if !keys.contains(&b"p1b/a".as_slice()) || !keys.contains(&b"p1b/b".as_slice()) {
            return Err(FdbError::Other(format!(
                "get_range missing committed keys: {keys:?}"
            )));
        }
        // Stage new key in range — must appear in subsequent get_range on same TX.
        tr.set(b"p1b/c", b"3")
            .map_err(|e| FdbError::from_store(&e))?;
        let pairs2 = tr
            .get_range(&db, b"p1b/", b"p1b0")
            .map_err(|e| FdbError::from_store(&e))?;
        let has_c = pairs2.iter().any(|(k, v)| k.as_slice() == b"p1b/c" && v == b"3");
        if !has_c {
            return Err(FdbError::Other(format!(
                "staging overlay missing p1b/c in get_range: {pairs2:?}"
            )));
        }
        db.commit(tr)?;
        steps_ok.push("get_range_contents_and_overlay");
    }

    // 7) range OCC: get_range then concurrent write **in** range → NotCommitted
    {
        let mut tr = db.create_transaction();
        let _ = tr
            .get_range(&db, b"p1b/", b"p1b0")
            .map_err(|e| FdbError::from_store(&e))?;
        db.cluster()
            .put(b"p1b/a", b"mut")
            .map_err(|e| FdbError::from_store(&e))?;
        tr.set(b"p1b/d", b"x")
            .map_err(|e| FdbError::from_store(&e))?;
        let err = db.commit(tr).expect_err("range OCC must Conflict");
        if err != FdbError::NotCommitted {
            return Err(FdbError::Other(format!(
                "expected range NotCommitted, got {err:?}"
            )));
        }
        steps_ok.push("range_conflict");
    }

    // 8) write **outside** conflict range must commit (no false Conflict)
    {
        let mut tr = db.create_transaction();
        let _ = tr
            .get_range(&db, b"p1b/", b"p1b0")
            .map_err(|e| FdbError::from_store(&e))?;
        // Concurrent mutation outside [p1b/, p1b0)
        db.cluster()
            .put(b"p1b_out/z", b"ok")
            .map_err(|e| FdbError::from_store(&e))?;
        tr.set(b"p1b/e", b"y")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr)?;
        let mut tr2 = db.create_transaction();
        let got = tr2
            .get(&db, b"p1b/e")
            .map_err(|e| FdbError::from_store(&e))?;
        if got.as_deref() != Some(b"y".as_ref()) {
            return Err(FdbError::Other(format!(
                "out-of-range concurrent write must not block commit, got {got:?}"
            )));
        }
        steps_ok.push("range_no_false_conflict");
    }

    // 9) clear then set same key in one TX (staging)
    {
        db.cluster()
            .put(b"p1b/flip", b"old")
            .map_err(|e| FdbError::from_store(&e))?;
        let mut tr = db.create_transaction();
        tr.clear(b"p1b/flip")
            .map_err(|e| FdbError::from_store(&e))?;
        let mid = tr
            .get(&db, b"p1b/flip")
            .map_err(|e| FdbError::from_store(&e))?;
        if mid.is_some() {
            return Err(FdbError::Other(format!(
                "clear must hide key in-TX, got {mid:?}"
            )));
        }
        tr.set(b"p1b/flip", b"new")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr)?;
        let mut tr2 = db.create_transaction();
        let got = tr2
            .get(&db, b"p1b/flip")
            .map_err(|e| FdbError::from_store(&e))?;
        if got.as_deref() != Some(b"new".as_ref()) {
            return Err(FdbError::Other(format!(
                "clear+set same key commit got {got:?}"
            )));
        }
        steps_ok.push("clear_then_set_same_key");
    }

    // 10) clear_range removes all keys in half-open range
    {
        let mut tr = db.create_transaction();
        tr.set(b"p1c/a", b"1")
            .map_err(|e| FdbError::from_store(&e))?;
        tr.set(b"p1c/b", b"2")
            .map_err(|e| FdbError::from_store(&e))?;
        tr.set(b"p1c_out", b"keep")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr)?;
        let mut tr2 = db.create_transaction();
        tr2.clear_range(&db, b"p1c/", b"p1c0")
            .map_err(|e| FdbError::from_store(&e))?;
        db.commit(tr2)?;
        let mut tr3 = db.create_transaction();
        let left = tr3
            .get_range(&db, b"p1c/", b"p1c0")
            .map_err(|e| FdbError::from_store(&e))?;
        if !left.is_empty() {
            return Err(FdbError::Other(format!(
                "clear_range left keys {left:?}"
            )));
        }
        let keep = tr3
            .get(&db, b"p1c_out")
            .map_err(|e| FdbError::from_store(&e))?;
        if keep.as_deref() != Some(b"keep".as_ref()) {
            return Err(FdbError::Other(format!(
                "clear_range must not touch outside key, got {keep:?}"
            )));
        }
        steps_ok.push("clear_range");
    }

    // 11) limit maps (oversized value) — no watermark side effects
    {
        let mut tr = db.create_transaction();
        let big = vec![0u8; crate::MAX_VALUE_BYTES + 1];
        let err = tr
            .set(b"p1/big", &big)
            .expect_err("oversized set must fail");
        let mapped = FdbError::from_store(&err);
        if mapped != FdbError::Limit {
            return Err(FdbError::Other(format!(
                "expected Limit, got {mapped:?} from {err}"
            )));
        }
        steps_ok.push("limit");
    }

    // 12) too-old maps — last: advances safe_watermark and would poison later commits
    {
        let mut tr = db.create_transaction();
        tr.set(b"p1/old", b"1")
            .map_err(|e| FdbError::from_store(&e))?;
        let snap = tr.read_version();
        db.cluster()
            .force_safe_watermark_for_test(snap.saturating_add(1));
        let err = db.commit(tr).expect_err("too old");
        if err != FdbError::TransactionTooOld {
            return Err(FdbError::Other(format!(
                "expected TransactionTooOld, got {err:?}"
            )));
        }
        steps_ok.push("too_old");
    }

    Ok(Phase1HarnessReport { steps_ok })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreCluster;

    fn temp() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("fdb-compat-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fdb_compat_get_set_commit_roundtrip() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let mut db = FdbDatabase::open(&mut c);
        let mut tr = db.create_transaction();
        tr.set(b"fdb/k", b"v1").unwrap();
        let ver = db.commit(tr).unwrap();
        assert!(ver >= 1);
        let mut tr2 = db.create_transaction();
        assert_eq!(tr2.get(&db, b"fdb/k").unwrap().as_deref(), Some(b"v1".as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fdb_compat_conflict_maps_not_committed() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let mut db = FdbDatabase::open(&mut c);
        let mut t1 = db.create_transaction();
        let mut t2 = db.create_transaction();
        t1.set(b"x", b"1").unwrap();
        t2.set(b"x", b"2").unwrap();
        db.commit(t1).unwrap();
        let err = db.commit(t2).unwrap_err();
        assert_eq!(err, FdbError::NotCommitted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fdb_compat_too_old_maps() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        let mut db = FdbDatabase::open(&mut c);
        let mut tr = db.create_transaction();
        tr.set(b"late", b"1").unwrap();
        db.cluster().force_safe_watermark_for_test(1);
        let err = db.commit(tr).unwrap_err();
        assert_eq!(err, FdbError::TransactionTooOld);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 1 + 1b: FDB-shaped harness on real 3-node store (incl. range OCC).
    #[test]
    fn phase1_bindingtester_subset_harness() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(100).unwrap();
        let report = run_phase1_bindingtester_subset(&mut c).expect("phase1 harness");
        assert!(
            report.steps_ok.contains(&"si_read_set_conflict"),
            "isolation step missing: {:?}",
            report.steps_ok
        );
        assert!(
            report.steps_ok.contains(&"ww_not_committed"),
            "ww step missing: {:?}",
            report.steps_ok
        );
        assert!(
            report.steps_ok.contains(&"range_conflict"),
            "range OCC missing: {:?}",
            report.steps_ok
        );
        assert!(
            report.steps_ok.contains(&"range_no_false_conflict"),
            "false-conflict guard missing: {:?}",
            report.steps_ok
        );
        assert!(
            report.steps_ok.contains(&"clear_range"),
            "clear_range missing: {:?}",
            report.steps_ok
        );
        assert_eq!(report.steps_ok.len(), 12, "steps={:?}", report.steps_ok);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
