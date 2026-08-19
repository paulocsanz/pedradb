//! Engine adapters for the parity bench: `compat` (rocksdb-compat on
//! pedradb-core) and `rocksdb` (real RocksDB via the rocksdb crate, feature
//! `real`). Both implement the same [`Engine`] ops so the runner's schedule
//! cannot drift between engines.

use crate::{CfWrite, Engine, OccEngine, OccTxn, DEPS_CFS};
use std::path::Path;

/// rocksdb-compat on pedradb-core (always available). Single node, single
/// client; WAL `fdatasync` before Ok (RFC-0001 / RFC-0036).
pub struct CompatEngine {
    db: rocksdb_compat::DB,
}

impl CompatEngine {
    pub fn open(path: &Path) -> Self {
        let mut opts = rocksdb_compat::Options::default();
        opts.create_if_missing(true);
        // Bench-only same-class column. Product default remains G1 (fsync).
        if std::env::var("PEDRA_PARITY_ASYNC").as_deref() == Ok("1") {
            opts.set_sync(false);
        }
        // Only register extra CFs when a suite needs them. Named CFs force
        // `default\0` prefix on every ycsb/kvrocks key; Rocks default CF does not.
        let cfs: &[&str] =
            if crate::suites_enabled("deps") || crate::suites_enabled("myrocks") {
                DEPS_CFS
            } else {
                &[]
            };
        let db = rocksdb_compat::DB::open_cf(&opts, path, cfs).expect("compat open_cf");
        Self { db }
    }
}

impl Engine for CompatEngine {
    fn label(&self) -> &'static str {
        "compat"
    }
    fn durability(&self) -> &'static str {
        if self.db.write_sync() {
            "fdatasync-before-ok (pedradb-core WAL; RFC-0001/0036)"
        } else {
            "async-wal (PEDRA_PARITY_ASYNC=1; WAL write, no fdatasync — NOT G1, not official)"
        }
    }
    fn sync(&self) -> bool {
        self.db.write_sync()
    }
    fn set_write_sync(&self, sync: bool) {
        self.db.set_write_sync(sync);
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get(k).map_err(|_| ())
    }
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        // Same visibility as a forward iterator; KeyOnly count (RFC-0033).
        self.db
            .count_named(rocksdb_compat::DEFAULT_CF, start, end, cap)
            .map_err(|_| ())
    }
    fn put_cf(&self, cf: &str, k: &[u8], v: &[u8]) -> bool {
        match self.db.cf_handle(cf) {
            Some(h) => self.db.put_cf(&h, k, v).is_ok(),
            None => false,
        }
    }
    fn get_cf(&self, cf: &str, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get_named(cf, k).map_err(|_| ())
    }
    fn batch(&self, ops: Vec<CfWrite>) -> bool {
        // RFC-0041: move owned keys/values into Bytes (no 1 KiB payload copy).
        let mut puts = Vec::with_capacity(ops.len());
        let mut deletes = Vec::new();
        for op in ops {
            match op {
                CfWrite::Put { cf, k, v } => puts.push((cf, k, v)),
                CfWrite::Delete { cf, k } => deletes.push((cf, k)),
            }
        }
        self.db.write_cf_owned(puts, deletes).is_ok()
    }
    fn latest_cf(&self, cf: &str, prefix: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        // RFC-0033: last_under_prefix — not a prefix walk.
        self.db.last_key_named(cf, prefix).map_err(|_| ())
    }
    fn latest_then_get_cf(
        &self,
        latest_cf: &str,
        prefix: &[u8],
        value_cf: &str,
    ) -> Result<Option<Vec<u8>>, ()> {
        let lh = self.db.cf_handle(latest_cf).ok_or(())?;
        let gh = self.db.cf_handle(value_cf).ok_or(())?;
        self.db
            .last_prefix_then_get(&lh, prefix, &gh)
            .map_err(|_| ())
    }
    fn scan_count_cf(&self, cf: &str, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        self.db.count_named(cf, start, end, cap).map_err(|_| ())
    }
    fn write_group_stats(&self) -> Option<(u64, u64, u64, u64)> {
        Some(self.db.write_group_stats())
    }
    fn flush(&self) -> bool {
        self.db.flush().is_ok()
    }
    fn reset_read_probe(&self) {
        self.db.reset_read_probe();
    }
    fn read_probe_json(&self) -> Option<String> {
        let p = self.db.read_probe();
        Some(format!(
            r#"{{"latest_ops":{lo},"latest_mem_hit":{mh},"latest_sst_fallback":{fb},"latest_sst_probed":{sp},"scan_ops":{so},"scan_sst_probed":{ssp},"sst_count":{sc},"l0_files":{l0},"level1_files":{l1},"mem_entries":{me},"block_cache_hits":{ch},"block_cache_misses":{cm},"blocks_decoded":{bd},"get_mem_hit":{gm},"get_sst_fallback":{gs},"get_inline":{gi},"get_vlog":{gv},"mvcc_split_ops":{so2},"mvcc_ns_encode":{ne},"mvcc_ns_last":{nl},"mvcc_ns_get":{ng},"mvcc_ns_copy":{nc}}}"#,
            lo = p.latest_ops,
            mh = p.latest_mem_hit,
            fb = p.latest_sst_fallback,
            sp = p.latest_sst_probed,
            so = p.scan_ops,
            ssp = p.scan_sst_probed,
            sc = p.sst_count,
            l0 = p.l0_files,
            l1 = p.level1_files,
            me = p.mem_entries,
            ch = p.block_cache_hits,
            cm = p.block_cache_misses,
            bd = p.blocks_decoded,
            gm = p.get_mem_hit,
            gs = p.get_sst_fallback,
            gi = p.get_inline,
            gv = p.get_vlog,
            so2 = p.mvcc_split_ops,
            ne = p.mvcc_ns_encode,
            nl = p.mvcc_ns_last,
            ng = p.mvcc_ns_get,
            nc = p.mvcc_ns_copy,
        ))
    }
}

struct CompatOccTxn<'a> {
    inner: Option<rocksdb_compat::Transaction<'a>>,
}

impl OccTxn for CompatOccTxn<'_> {
    fn get(&mut self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.inner.as_ref().ok_or(())?.get(k).map_err(|_| ())
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.inner
            .as_ref()
            .map(|t| t.put(k, v).is_ok())
            .unwrap_or(false)
    }
    fn delete(&mut self, k: &[u8]) -> bool {
        self.inner
            .as_ref()
            .map(|t| t.delete(k).is_ok())
            .unwrap_or(false)
    }
    fn scan_count(&mut self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        self.inner
            .as_ref()
            .ok_or(())?
            .scan_count(start, end, cap)
            .map_err(|_| ())
    }
    fn commit(&mut self) -> bool {
        match self.inner.take() {
            Some(tx) => tx.commit().is_ok(),
            None => false,
        }
    }
}

impl OccEngine for CompatEngine {
    fn with_txn<R>(&self, f: impl FnOnce(&mut dyn OccTxn) -> R) -> R {
        let mut wrap = CompatOccTxn {
            inner: Some(self.db.transaction()),
        };
        f(&mut wrap)
    }
}

/// pedradb-core `ConcurrentDb` with Rocks-style group commit on the write
/// path (RFC-0037 P2.2). YCSB-subset adapter: the multi-client harness only
/// uses default-CF put/get/rmw — CF methods are unimplemented and return
/// errors rather than silently degrading.
pub struct ConcurrentEngine {
    db: pedradb_core::concurrent::ConcurrentDb<pedradb_core::StdEnv>,
}

impl ConcurrentEngine {
    pub fn open(path: &Path) -> Self {
        let db = pedradb_core::concurrent::ConcurrentDb::open(path)
            .expect("concurrent open (sync WAL default)");
        Self { db }
    }
}

impl Engine for ConcurrentEngine {
    fn label(&self) -> &'static str {
        "concurrent"
    }
    fn durability(&self) -> &'static str {
        "fdatasync-before-ok via ConcurrentDb write group (leader fsyncs once per group)"
    }
    fn sync(&self) -> bool {
        true
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        Ok(self.db.get(k).map(|b| b.to_vec()))
    }
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        use std::ops::Bound;
        let got = self
            .db
            .scan_collect(Bound::Included(start), Bound::Excluded(end));
        Ok(got.len().min(cap))
    }
    fn put_cf(&self, _cf: &str, _k: &[u8], _v: &[u8]) -> bool {
        false
    }
    fn get_cf(&self, _cf: &str, _k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        Err(())
    }
    fn batch(&self, _ops: Vec<CfWrite>) -> bool {
        false
    }
    fn latest_cf(&self, _cf: &str, _prefix: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        Err(())
    }
    fn scan_count_cf(
        &self,
        _cf: &str,
        _start: &[u8],
        _end: &[u8],
        _cap: usize,
    ) -> Result<usize, ()> {
        Err(())
    }
}

/// Real RocksDB via the rocksdb crate (feature `real`). Durability is labeled:
/// `sync` per write when `sync=true` (matched to Pedra's contract), else
/// RocksDB's async-WAL default (reference run).
#[cfg(feature = "real")]
pub struct RocksEngine {
    db: rocksdb::DB,
    wopts_async: rocksdb::WriteOptions,
    wopts_sync: rocksdb::WriteOptions,
    cur_sync: std::sync::atomic::AtomicBool,
    /// After each durable write, `sync_all` every `*.log` so the peer pays
    /// `F_FULLFSYNC` (macOS) — same syscall class as Pedra `File::sync_all`.
    /// librocksdb-sys is built *without* `HAVE_FULLFSYNC`, so default Rocks
    /// `WriteOptions.sync` is `fdatasync` (~50µs here), not `F_FULLFSYNC` (~5ms).
    full_sync: bool,
    dir: std::path::PathBuf,
}

#[cfg(feature = "real")]
impl RocksEngine {
    pub fn open(path: &Path, sync: bool) -> Self {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = rocksdb::DB::open_cf(&opts, path, DEPS_CFS).expect("rocksdb open_cf");
        let wopts_async = rocksdb::WriteOptions::default();
        let mut wopts_sync = rocksdb::WriteOptions::default();
        wopts_sync.set_sync(true);
        let full_sync = crate::env_usize("ROCKS_PARITY_FULL_SYNC", 0) != 0;
        Self {
            db,
            wopts_async,
            wopts_sync,
            cur_sync: std::sync::atomic::AtomicBool::new(sync),
            full_sync,
            dir: path.to_path_buf(),
        }
    }

    fn wopts(&self) -> &rocksdb::WriteOptions {
        if self.cur_sync.load(std::sync::atomic::Ordering::Relaxed) {
            &self.wopts_sync
        } else {
            &self.wopts_async
        }
    }

    fn full_sync_wal(&self) {
        if !self.full_sync {
            return;
        }
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().ends_with(".log") {
                continue;
            }
            if let Ok(f) = std::fs::File::open(e.path()) {
                let _ = f.sync_all();
            }
        }
    }
}

#[cfg(feature = "real")]
impl Engine for RocksEngine {
    fn label(&self) -> &'static str {
        "rocksdb"
    }
    fn durability(&self) -> &'static str {
        match (crate::peer_reports_sync(), self.full_sync) {
            (true, true) => {
                "host-default / sync-on-commit + F_FULLFSYNC (not official Rocks default)"
            }
            (true, false) => {
                "host-default (MyRocks commit-sync / Surreal sync=every; not official Rocks async)"
            }
            (false, _) => "async-wal (WriteOptions.sync=false, rocksdb default)",
        }
    }
    fn sync(&self) -> bool {
        crate::peer_reports_sync()
    }
    fn set_write_sync(&self, sync: bool) {
        self.cur_sync
            .store(sync, std::sync::atomic::Ordering::Relaxed);
    }
    fn flush(&self) -> bool {
        self.db.flush().is_ok()
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        let ok = self.db.put_opt(k, v, self.wopts()).is_ok();
        if ok {
            self.full_sync_wal();
        }
        ok
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get(k).map_err(|_| ())
    }
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        Ok(self
            .db
            .iterator(rocksdb::IteratorMode::From(
                start,
                rocksdb::Direction::Forward,
            ))
            .map_while(|r| r.ok())
            .take(cap)
            .take_while(|(k, _)| k.as_ref() < end)
            .count())
    }
    fn put_cf(&self, cf: &str, k: &[u8], v: &[u8]) -> bool {
        let ok = match self.db.cf_handle(cf) {
            Some(h) => self.db.put_cf_opt(h, k, v, self.wopts()).is_ok(),
            None => false,
        };
        if ok {
            self.full_sync_wal();
        }
        ok
    }
    fn get_cf(&self, cf: &str, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        self.db.get_cf(h, k).map_err(|_| ())
    }
    fn batch(&self, ops: Vec<CfWrite>) -> bool {
        let mut wb = rocksdb::WriteBatch::default();
        for op in ops {
            let staged = match op {
                CfWrite::Put { cf, k, v } => match self.db.cf_handle(cf) {
                    Some(h) => {
                        wb.put_cf(h, k, v);
                        true
                    }
                    None => false,
                },
                CfWrite::Delete { cf, k } => match self.db.cf_handle(cf) {
                    Some(h) => {
                        wb.delete_cf(h, k);
                        true
                    }
                    None => false,
                },
            };
            if !staged {
                return false;
            }
        }
        let ok = self.db.write_opt(wb, self.wopts()).is_ok();
        if ok {
            self.full_sync_wal();
        }
        ok
    }
    fn latest_cf(&self, cf: &str, prefix: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        let mut seek = prefix.to_vec();
        seek.extend_from_slice(&u64::MAX.to_be_bytes());
        Ok(self
            .db
            .iterator_cf(
                h,
                rocksdb::IteratorMode::From(&seek, rocksdb::Direction::Reverse),
            )
            .map_while(|r| r.ok())
            .take(1)
            .find(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.to_vec()))
    }
    fn scan_count_cf(&self, cf: &str, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        Ok(self
            .db
            .iterator_cf(
                h,
                rocksdb::IteratorMode::From(start, rocksdb::Direction::Forward),
            )
            .map_while(|r| r.ok())
            .take(cap)
            .take_while(|(k, _)| k.as_ref() < end)
            .count())
    }
}

/// RocksDB `OptimisticTransactionDB` peer for the SurrealDB suite.
/// Regular `RocksEngine` stays the official 16-shape peer (plain `DB`).
#[cfg(feature = "real")]
pub struct RocksOccEngine {
    db: rocksdb::OptimisticTransactionDB,
    wopts_async: rocksdb::WriteOptions,
    wopts_sync: rocksdb::WriteOptions,
    cur_sync: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "real")]
impl RocksOccEngine {
    pub fn open(path: &Path, sync: bool) -> Self {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = rocksdb::OptimisticTransactionDB::open_cf(&opts, path, DEPS_CFS)
            .expect("rocks OptimisticTransactionDB open_cf");
        let wopts_async = rocksdb::WriteOptions::default();
        let mut wopts_sync = rocksdb::WriteOptions::default();
        wopts_sync.set_sync(true);
        Self {
            db,
            wopts_async,
            wopts_sync,
            cur_sync: std::sync::atomic::AtomicBool::new(sync),
        }
    }

    fn wopts(&self) -> &rocksdb::WriteOptions {
        if self.cur_sync.load(std::sync::atomic::Ordering::Relaxed) {
            &self.wopts_sync
        } else {
            &self.wopts_async
        }
    }
}

#[cfg(feature = "real")]
impl Engine for RocksOccEngine {
    fn label(&self) -> &'static str {
        "rocksdb"
    }
    fn durability(&self) -> &'static str {
        if crate::peer_reports_sync() {
            "host-default (MyRocks commit-sync / Surreal sync=every; OptimisticTransactionDB)"
        } else {
            "async-wal (OptimisticTransactionDB; WriteOptions.sync=false, rocksdb default)"
        }
    }
    fn sync(&self) -> bool {
        crate::peer_reports_sync()
    }
    fn set_write_sync(&self, sync: bool) {
        self.cur_sync
            .store(sync, std::sync::atomic::Ordering::Relaxed);
    }
    fn flush(&self) -> bool {
        self.db.flush().is_ok()
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        self.db.put_opt(k, v, self.wopts()).is_ok()
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get(k).map_err(|_| ())
    }
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        Ok(self
            .db
            .iterator(rocksdb::IteratorMode::From(
                start,
                rocksdb::Direction::Forward,
            ))
            .map_while(|r| r.ok())
            .take(cap)
            .take_while(|(k, _)| k.as_ref() < end)
            .count())
    }
    fn put_cf(&self, cf: &str, k: &[u8], v: &[u8]) -> bool {
        match self.db.cf_handle(cf) {
            Some(h) => self.db.put_cf_opt(h, k, v, self.wopts()).is_ok(),
            None => false,
        }
    }
    fn get_cf(&self, cf: &str, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        self.db.get_cf(h, k).map_err(|_| ())
    }
    fn batch(&self, ops: Vec<CfWrite>) -> bool {
        let mut to = rocksdb::OptimisticTransactionOptions::default();
        to.set_snapshot(true);
        let tx = self.db.transaction_opt(self.wopts(), &to);
        for op in ops {
            let ok = match op {
                CfWrite::Put { cf, k, v } => match self.db.cf_handle(cf) {
                    Some(h) => tx.put_cf(h, k, v).is_ok(),
                    None => false,
                },
                CfWrite::Delete { cf, k } => match self.db.cf_handle(cf) {
                    Some(h) => tx.delete_cf(h, k).is_ok(),
                    None => false,
                },
            };
            if !ok {
                return false;
            }
        }
        tx.commit().is_ok()
    }
    fn latest_cf(&self, cf: &str, prefix: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        let mut seek = prefix.to_vec();
        seek.extend_from_slice(&u64::MAX.to_be_bytes());
        Ok(self
            .db
            .iterator_cf(
                h,
                rocksdb::IteratorMode::From(&seek, rocksdb::Direction::Reverse),
            )
            .map_while(|r| r.ok())
            .take(1)
            .find(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.to_vec()))
    }
    fn scan_count_cf(&self, cf: &str, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        Ok(self
            .db
            .iterator_cf(
                h,
                rocksdb::IteratorMode::From(start, rocksdb::Direction::Forward),
            )
            .map_while(|r| r.ok())
            .take(cap)
            .take_while(|(k, _)| k.as_ref() < end)
            .count())
    }
}

#[cfg(feature = "real")]
struct RocksOccTxn<'a> {
    inner: Option<rocksdb::Transaction<'a, rocksdb::OptimisticTransactionDB>>,
}

#[cfg(feature = "real")]
impl OccTxn for RocksOccTxn<'_> {
    fn get(&mut self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.inner.as_ref().ok_or(())?.get(k).map_err(|_| ())
    }
    fn put(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.inner
            .as_ref()
            .map(|t| t.put(k, v).is_ok())
            .unwrap_or(false)
    }
    fn delete(&mut self, k: &[u8]) -> bool {
        self.inner
            .as_ref()
            .map(|t| t.delete(k).is_ok())
            .unwrap_or(false)
    }
    fn scan_count(&mut self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        let tx = self.inner.as_ref().ok_or(())?;
        Ok(tx
            .iterator(rocksdb::IteratorMode::From(
                start,
                rocksdb::Direction::Forward,
            ))
            .map_while(|r| r.ok())
            .take(cap)
            .take_while(|(k, _)| k.as_ref() < end)
            .count())
    }
    fn commit(&mut self) -> bool {
        match self.inner.take() {
            Some(tx) => tx.commit().is_ok(),
            None => false,
        }
    }
}

#[cfg(feature = "real")]
impl OccEngine for RocksOccEngine {
    fn with_txn<R>(&self, f: impl FnOnce(&mut dyn OccTxn) -> R) -> R {
        // Same begin as SurrealDB kv-rocksdb: snapshot=true, sync=peer flag.
        let mut to = rocksdb::OptimisticTransactionOptions::default();
        to.set_snapshot(true);
        let tx = self.db.transaction_opt(self.wopts(), &to);
        let mut wrap = RocksOccTxn { inner: Some(tx) };
        f(&mut wrap)
    }
}
