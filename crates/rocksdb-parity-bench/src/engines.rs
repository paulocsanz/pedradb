//! Engine adapters for the parity bench: `compat` (rocksdb-compat on
//! pedradb-core) and `rocksdb` (real RocksDB via the rocksdb crate, feature
//! `real`). Both implement the same [`Engine`] ops so the runner's schedule
//! cannot drift between engines.

use crate::{CfWrite, Engine, OccEngine, OccTxn, DEPS_CFS};
use std::path::Path;
use std::sync::atomic::AtomicU64;

use pedradb_core::Env;

static INGEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// rocksdb-compat on pedradb-core (always available). Single node.
/// Drop-in default is Rocks-shaped async WAL (RFC-0054); `set_sync(true)`
/// is G1 (`F_FULLFSYNC` on Darwin).
pub struct CompatEngine<E: Env = pedradb_io_uring::IoUringEnv> {
    db: rocksdb_compat::DB<E>,
    /// Column label — "compat" (default) or "compatv" (RFC-0058 verified
    /// profile: StdEnv + lone-commit-only; `set_write_sync(false)` is
    /// refused there).
    label: &'static str,
}

impl CompatEngine {
    pub fn open(path: &Path) -> Self {
        let opts = compat_bench_opts();
        // Only register extra CFs when a suite needs them. Named CFs force
        // `default\0` prefix on every ycsb/kvrocks key; Rocks default CF does not.
        let cfs: &[&str] = if crate::suites_enabled("deps") || crate::suites_enabled("myrocks") {
            DEPS_CFS
        } else {
            &[]
        };
        let db = rocksdb_compat::DB::open_cf(&opts, path, cfs).expect("compat open_cf");
        Self {
            db,
            label: "compat",
        }
    }
}

impl CompatEngine<pedradb_core::StdEnv> {
    /// RFC-0058 P1.2: the **verified profile** column — `StdEnv` (no
    /// io_uring ring), `OpenOptions::verified()` forced and the
    /// lone-commit-only pin (via [`rocksdb_compat::DB::open_verified`]).
    /// Same bench opts (memtable / blob / retention / CFs) as `compat` so
    /// the only delta vs the official column is the profile itself.
    pub fn open_verified(path: &Path) -> Self {
        let opts = compat_bench_opts();
        let cfs: &[&str] = if crate::suites_enabled("deps") || crate::suites_enabled("myrocks") {
            DEPS_CFS
        } else {
            &[]
        };
        let db = rocksdb_compat::DB::open_verified(&opts, path, cfs).expect("compat open_verified");
        Self {
            db,
            label: "compatv",
        }
    }
}

/// Shared bench-side compat options (memtable, blob profile, retention) —
/// one place so `compat` and `compatv` differ only by the profile.
fn compat_bench_opts() -> rocksdb_compat::Options {
    let mut opts = rocksdb_compat::Options::default();
    opts.create_if_missing(true);
    // Match RocksDB default memtable (64 MiB). 4 MiB was for apply_mc4;
    // kvrocks_set_mc50 at 4 MiB flushed ~25× per timed run.
    // Larger than any timed suite's write volume (mc50 100k×1 KiB) so
    // auto-flush does not run in the measured window. Rocks default is
    // 64 MiB — 4 MiB was flushing ~25× during set_mc50.
    // `ROCKS_PARITY_COMPAT_MEMTABLE` (bytes) overrides for long-window
    // experiments that want the same flush pressure as Rocks default.
    opts.write_buffer_size = std::env::var("ROCKS_PARITY_COMPAT_MEMTABLE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(256 * 1024 * 1024) as usize;
    // Drop-in default is already Rocks-shaped async (RFC-0054). This
    // env remains the explicit same-class column (no-op if already false).
    // (Ignored by the verified column — the profile forces sync.)
    if std::env::var("PEDRA_PARITY_ASYNC").as_deref() == Ok("1") {
        opts.set_sync(false);
    }
    // The G1 product column: Pedra fdatasyncs before Ok (the Agents.md
    // peer rule — more durability AND faster than the Rocks people
    // actually run). RFC-0054 made async the drop-in default and left no
    // way back to the durable column; this env restores it for the
    // official parity battery. The reported `durability` label flips to
    // "strongest-data-barrier-before-ok" when it is on.
    if std::env::var("PEDRA_PARITY_G1").as_deref() == Ok("1") {
        opts.set_sync(true);
    }
    // WiscKey / BlobDB: values ≥ threshold go to VALUES.vlog so the WAL
    // holds a pointer (16 KiB blob is not copied into every WAL record).
    // Default 4096 — 1 KiB SET/GET stay inline. `ROCKS_PARITY_MIN_BLOB=0`
    // restores always-inline (A/B). Rocks default is blob files off;
    // this is the Pedra large-value profile on the parity harness only.
    match std::env::var("ROCKS_PARITY_MIN_BLOB") {
        Ok(s) if s == "0" || s.eq_ignore_ascii_case("off") => {}
        Ok(s) => {
            let n = s.parse::<u64>().unwrap_or(4096);
            opts.set_enable_blob_files(true);
            opts.set_min_blob_size(n);
        }
        Err(_) => {
            opts.set_enable_blob_files(true);
            opts.set_min_blob_size(4096);
        }
    }
    // `ROCKS_PARITY_RETENTION` (RFC-0047 P0.3): pin the retention the
    // column measures, so the compat default flip (auto_reclaim=true)
    // never silently changes official numbers. `product` (default) =
    // kernel default retention, whatever it currently is — since
    // RFC-0046 that is `HistoryHorizon::Window(24h)` + archive tier
    // (before 2026-08-21 it was F20 keep-all). `rocks` = RocksDB
    // storage profile (drop-in default): auto-compact GCs unpinned
    // obsolete versions, no archive. Legacy `ROCKS_PARITY_AUTO_RECLAIM=1`
    // == `rocks`.
    let retention = std::env::var("ROCKS_PARITY_RETENTION").unwrap_or_else(|_| "product".into());
    let mut reclaim = match retention.as_str() {
        "product" => false,
        "rocks" => true,
        other => {
            eprintln!(
                "ROCKS_PARITY_RETENTION={other}: invalid (want product|rocks) — refusing to bench an ambiguous retention"
            );
            std::process::exit(2);
        }
    };
    if crate::env_usize("ROCKS_PARITY_AUTO_RECLAIM", 0) != 0 {
        if retention == "product" {
            eprintln!(
                "ROCKS_PARITY_RETENTION=product conflicts with ROCKS_PARITY_AUTO_RECLAIM=1 — refusing ambiguous retention"
            );
            std::process::exit(2);
        }
        reclaim = true;
    }
    opts.auto_reclaim = reclaim;
    opts
}

impl<E: Env> Engine for CompatEngine<E> {
    fn label(&self) -> &'static str {
        self.label
    }
    fn durability(&self) -> &'static str {
        if self.label == "compatv" {
            "verified profile: strongest-data-barrier-before-ok + lone-commit-only (RFC-0058)"
        } else if self.db.write_sync() {
            "strongest-data-barrier-before-ok (F_FULLFSYNC on Darwin, fdatasync on Linux; RFC-0001/0036 v2)"
        } else {
            "async-wal (PEDRA_PARITY_ASYNC=1; WAL write, no fdatasync — NOT G1, not official)"
        }
    }
    fn sync(&self) -> bool {
        self.db.write_sync()
    }
    fn set_write_sync(&self, sync: bool) {
        if self.label == "compatv" && !sync {
            // RFC-0058: the verified column never leaves the profile.
            return;
        }
        self.db.set_write_sync(sync);
    }
    fn put(&self, k: &[u8], v: &[u8]) -> bool {
        self.db.put(k, v).is_ok()
    }
    fn get(&self, k: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        self.db.get(k).map_err(|_| ())
    }
    fn get_probe(&self, k: &[u8]) -> Result<bool, ()> {
        self.db.contains(k).map_err(|_| ())
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
    fn batch_put_same(&self, cf: &'static str, keys: &[Vec<u8>], v: &[u8]) -> bool {
        self.db.put_batch_same(cf, keys, v).is_ok()
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
    fn mem_entries(&self) -> Option<u64> {
        Some(self.db.stats().mem_entries as u64)
    }
    fn fold_mem_tail(&self) -> usize {
        self.db.fold_mem_tail()
    }
    /// Raw phase counters for per-shape deltas (cumulative line mixes
    /// shapes): `[commits, prepare, wal, mem, publish, flush, lock_wait]`.
    fn write_phase_snapshot(&self) -> Option<[u64; 7]> {
        let st = self.db.write_phase_stats()?;
        let r = std::sync::atomic::Ordering::Relaxed;
        Some([
            st.commits.load(r),
            st.prepare_ns.load(r),
            st.wal_ns.load(r),
            st.mem_ns.load(r),
            st.publish_ns.load(r),
            st.flush_check_ns.load(r),
            st.lock_wait_ns.load(r),
        ])
    }
    fn write_phase_line(&self) -> Option<String> {
        let st = self.db.write_phase_stats()?;
        let n = st.commits.load(std::sync::atomic::Ordering::Relaxed).max(1);
        let us = |a: &std::sync::atomic::AtomicU64| {
            a.load(std::sync::atomic::Ordering::Relaxed) as f64 / n as f64 / 1000.0
        };
        Some(format!(
            "prepare={:.2}µs wal={:.2}µs mem={:.2}µs publish={:.2}µs flsh={:.2}µs lock_wait={:.2}µs n={n}",
            us(&st.prepare_ns),
            us(&st.wal_ns),
            us(&st.mem_ns),
            us(&st.publish_ns),
            us(&st.flush_check_ns),
            us(&st.lock_wait_ns),
        ))
    }
    fn flush(&self) -> bool {
        self.db.flush().is_ok()
    }
    fn wbwi_overlay_get(&self, puts: &[(&[u8], &[u8])], key: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        let mut b = rocksdb_compat::WriteBatchWithIndex::new();
        for (k, v) in puts {
            b.put(k, v);
        }
        b.get_from_batch_and_db(&self.db, key).map_err(|_| ())
    }
    fn ingest_kvs(&self, kvs: &[(&[u8], &[u8])]) -> bool {
        let n = INGEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sst = std::env::temp_dir().join(format!(
            "pedra-parity-ingest-{}-{n}.sst",
            std::process::id()
        ));
        let mut pairs: Vec<(&[u8], &[u8])> = kvs.to_vec();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let mut w = rocksdb_compat::SstFileWriter::create(&rocksdb_compat::Options::new());
        if w.open(&sst).is_err() {
            return false;
        }
        for (k, v) in &pairs {
            if w.put(k, v).is_err() {
                let _ = std::fs::remove_file(&sst);
                return false;
            }
        }
        if w.finish().is_err() {
            let _ = std::fs::remove_file(&sst);
            return false;
        }
        let ok = self.db.ingest_external_file(vec![&sst]).is_ok();
        let _ = std::fs::remove_file(&sst);
        ok
    }
    fn compact_drop_prefix(&self, prefix: &[u8]) -> bool {
        self.db
            .compact_with_filter(|_lvl, key, _val| {
                if key.starts_with(prefix) {
                    rocksdb_compat::CompactionDecision::Remove
                } else {
                    rocksdb_compat::CompactionDecision::Keep
                }
            })
            .is_ok()
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

struct CompatOccTxn<'a, E: Env> {
    inner: Option<rocksdb_compat::Transaction<'a, E>>,
}

impl<E: Env> OccTxn for CompatOccTxn<'_, E> {
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

impl<E: Env> OccEngine for CompatEngine<E> {
    fn with_txn<R>(&self, f: impl FnOnce(&mut dyn OccTxn) -> R) -> R {
        let mut wrap = CompatOccTxn::<E> {
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
    /// After each durable write, `sync_all` every `*.log`. Reconstruction of
    /// CMake `HAVE_FULLFSYNC` when the linked `librocksdb-sys` omitted it.
    /// Not equivalent: extra inner `fdatasync`, and every `*.log` not just
    /// the live WAL fd. Prefer `CXXFLAGS=-DHAVE_FULLFSYNC` on the sys-crate
    /// compile and leave this off (`ROCKS_PARITY_FULL_SYNC=0`).
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
        if !crate::rocks_full_sync_after_write(
            self.full_sync,
            self.cur_sync.load(std::sync::atomic::Ordering::Relaxed),
        ) {
            return;
        }
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let ents: Vec<_> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let Some(name) = crate::live_wal_log_name(ents.iter().map(String::as_str)) else {
            return;
        };
        if let Ok(f) = std::fs::File::open(self.dir.join(name)) {
            let _ = f.sync_all();
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
    fn wbwi_overlay_get(&self, puts: &[(&[u8], &[u8])], key: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        // rust-rocksdb 0.22 has no WriteBatchWithIndex type; last-write-wins
        // overlay then DB is the same observable as compat WBWI for this shape.
        for (k, v) in puts.iter().rev() {
            if *k == key {
                return Ok(Some(v.to_vec()));
            }
        }
        self.db.get(key).map_err(|_| ())
    }
    fn ingest_kvs(&self, kvs: &[(&[u8], &[u8])]) -> bool {
        let n = INGEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sst = std::env::temp_dir().join(format!(
            "pedra-parity-rocks-ingest-{}-{n}.sst",
            std::process::id()
        ));
        let mut pairs: Vec<(&[u8], &[u8])> = kvs.to_vec();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let opts = rocksdb::Options::default();
        let mut w = rocksdb::SstFileWriter::create(&opts);
        if w.open(&sst).is_err() {
            return false;
        }
        for (k, v) in &pairs {
            if w.put(k, v).is_err() {
                let _ = std::fs::remove_file(&sst);
                return false;
            }
        }
        if w.finish().is_err() {
            let _ = std::fs::remove_file(&sst);
            return false;
        }
        let ok = self.db.ingest_external_file(vec![&sst]).is_ok();
        let _ = std::fs::remove_file(&sst);
        ok
    }
    fn compact_drop_prefix(&self, prefix: &[u8]) -> bool {
        let mut drop = Vec::new();
        {
            let it = self.db.iterator(rocksdb::IteratorMode::From(
                prefix,
                rocksdb::Direction::Forward,
            ));
            for r in it {
                let Ok((k, _)) = r else {
                    break;
                };
                if !k.starts_with(prefix) {
                    break;
                }
                drop.push(k.to_vec());
            }
        }
        for k in drop {
            if self.db.delete_opt(&k, self.wopts()).is_err() {
                return false;
            }
        }
        self.db.compact_range(None::<&[u8]>, None::<&[u8]>);
        true
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
    fn get_probe(&self, k: &[u8]) -> Result<bool, ()> {
        match self.db.get_pinned(k) {
            Ok(v) => Ok(v.is_some()),
            Err(_) => Err(()),
        }
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
    fn batch_put_same(&self, cf: &'static str, keys: &[Vec<u8>], v: &[u8]) -> bool {
        let Some(h) = self.db.cf_handle(cf) else {
            return false;
        };
        let mut wb = rocksdb::WriteBatch::default();
        for k in keys {
            wb.put_cf(h, k, v);
        }
        let ok = self.db.write_opt(wb, self.wopts()).is_ok();
        if ok {
            self.full_sync_wal();
        }
        ok
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
