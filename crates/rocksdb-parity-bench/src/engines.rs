//! Engine adapters for the parity bench: `compat` (rocksdb-compat on
//! pedradb-core) and `rocksdb` (real RocksDB via the rocksdb crate, feature
//! `real`). Both implement the same [`Engine`] ops so the runner's schedule
//! cannot drift between engines.

use crate::{CfWrite, Engine, DEPS_CFS};
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
        let db = rocksdb_compat::DB::open_cf(&opts, path, DEPS_CFS).expect("compat open_cf");
        Self { db }
    }
}

impl Engine for CompatEngine {
    fn label(&self) -> &'static str {
        "compat"
    }
    fn durability(&self) -> &'static str {
        "fdatasync-before-ok (pedradb-core WAL; RFC-0001/0036)"
    }
    fn sync(&self) -> bool {
        true
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get(k).map_err(|_| ())
    }
    fn scan_count(&self, start: &[u8], end: &[u8], cap: usize) -> Result<usize, ()> {
        // Same visibility as a forward iterator; KeyOnly count (RFC-0033).
        let h = self.db.cf_handle(rocksdb_compat::DEFAULT_CF).ok_or(())?;
        self.db.count_cf(&h, start, end, cap).map_err(|_| ())
    }
    fn put_cf(&self, cf: &str, k: &[u8], v: &[u8]) -> bool {
        match self.db.cf_handle(cf) {
            Some(h) => self.db.put_cf(&h, k, v).is_ok(),
            None => false,
        }
    }
    fn get_cf(&self, cf: &str, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let h = self.db.cf_handle(cf).ok_or(())?;
        self.db.get_cf(&h, k).map_err(|_| ())
    }
    fn batch(&self, ops: Vec<CfWrite>) -> bool {
        let mut wb = rocksdb_compat::WriteBatch::new();
        for op in ops {
            let staged = match op {
                CfWrite::Put { cf, k, v } => match self.db.cf_handle(cf) {
                    Some(h) => {
                        wb.put_cf(&h, k, v);
                        true
                    }
                    None => false,
                },
                CfWrite::Delete { cf, k } => match self.db.cf_handle(cf) {
                    Some(h) => {
                        wb.delete_cf(&h, k);
                        true
                    }
                    None => false,
                },
            };
            if !staged {
                return false;
            }
        }
        self.db.write(&wb).is_ok()
    }
    fn latest_cf(&self, cf: &str, prefix: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        // RFC-0033: last_under_prefix — not a prefix walk.
        let h = self.db.cf_handle(cf).ok_or(())?;
        self.db.last_key_with_prefix(&h, prefix).map_err(|_| ())
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
        let h = self.db.cf_handle(cf).ok_or(())?;
        self.db.count_cf(&h, start, end, cap).map_err(|_| ())
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
    fn scan_count_cf(&self, _cf: &str, _start: &[u8], _end: &[u8], _cap: usize) -> Result<usize, ()> {
        Err(())
    }
}

/// Real RocksDB via the rocksdb crate (feature `real`). Durability is labeled:
/// `sync` per write when `sync=true` (matched to Pedra's contract), else
/// RocksDB's async-WAL default (reference run).
#[cfg(feature = "real")]
pub struct RocksEngine {
    db: rocksdb::DB,
    wopts: rocksdb::WriteOptions,
    sync: bool,
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
        let mut wopts = rocksdb::WriteOptions::default();
        wopts.set_sync(sync);
        let full_sync = crate::env_usize("ROCKS_PARITY_FULL_SYNC", 0) != 0;
        Self {
            db,
            wopts,
            sync,
            full_sync,
            dir: path.to_path_buf(),
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
        match (self.sync, self.full_sync) {
            (true, true) => {
                "sync-per-write + F_FULLFSYNC on WAL (same class as Pedra File::sync_all)"
            }
            (true, false) => "sync-per-write (WriteOptions.sync=true; fdatasync on this build)",
            (false, _) => "async-wal (WriteOptions.sync=false, rocksdb default)",
        }
    }
    fn sync(&self) -> bool {
        self.sync
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        let ok = self.db.put_opt(k, v, &self.wopts).is_ok();
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
            Some(h) => self.db.put_cf_opt(h, k, v, &self.wopts).is_ok(),
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
        let ok = self.db.write_opt(wb, &self.wopts).is_ok();
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
