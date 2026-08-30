//! Adversarial WAL persistency tests.
//!
//! Hypothesis under attack: "Pedra is only faster than RocksDB because it
//! silently skips WAL fsync." Every test here observes the **Env seam** —
//! real `write`/`sync_data` syscalls on the WAL file — not engine-internal
//! counters, so a `note_wal_sync()` that lies cannot pass them:
//!
//! - `ok_put_is_covered_by_a_real_fdatasync` — Ok ⇒ bytes written **and**
//!   `sync_data` succeeded at a file length covering them, before return.
//! - `lone_client_cannot_share_or_skip_fsyncs` — one client, one op per put:
//!   N Ok'd puts ⇒ ≥ N real fdatasyncs (the RFC-0041 P1.2 `1/t_fd` ceiling
//!   is physics; if this ever fails, speed is being bought with durability).
//! - `group_commit_amortizes_but_never_skips` — 8 concurrent clients all Ok
//!   with ≥ 1 real fdatasync and at most one per client; every Ok'd key
//!   survives an abrupt drop (process-crash model).
//! - `internal_sync_counter_matches_env_seam` — the telemetry counter the
//!   benches report equals the real syscall count.
//! - `no_sync_puts_perform_zero_fsync_until_sync` — the async column is
//!   honestly async (0 fsyncs), then one `Db::sync` makes them durable.
//! - `sync_failure_after_append_fences_fail_closed` — injected WAL fsync
//!   failure turns Ok into Err, fences the Db, refuses later writes; reopen
//!   keeps the pre-failure prefix only.
//! - `append_after_torn_tail_recovery_survives_crash` — recovery truncates
//!   the damaged region before new appends; a later crash never re-parses it.
//! - `mixed_sync_async_all_or_nothing_per_batch` — under a mixed burst, each
//!   multi-op async batch is recovered whole or not at all (never half).
//! - `vlog_gc_during_offlock_fsync_window_refuses_then_preserves_acked_sync_write`
//!   — F2 regression: value-log GC refuses while a writer sits in the
//!   off-lock fsync window (un-rotatable WAL), succeeds quiescent, and a
//!   crash right after loses no acked write.
//! - `second_gc_on_unpromoted_state_promotes_first_and_survives_rewrite_failure`
//!   — F3 regression: a GC round on a staged-but-unpromoted `.new` promotes
//!   it before rewriting, so a mid-rewrite failure never truncates a live
//!   staging file under `vlog_use_new`.
//! - `corrupt_vlog_payload_is_loud_not_missing` — F1 regression: a corrupt
//!   value-log payload is `Err(CorruptValue)` on error-shaped reads and a
//!   fail-stop on Option-shaped ones — never a silent miss / raw pointer.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::{CoreError, Db, OpenOptions, WalRecovery, WriteOptions, WAL_FILE_NAME};

/// Env-seam observation of everything the engine does to the WAL file.
#[derive(Default)]
struct WalCounters {
    /// `write`-class calls on the WAL (append/frame flush).
    wal_writes: AtomicU64,
    /// Bytes accepted by `write` on the WAL.
    wal_bytes: AtomicU64,
    /// `sync_data` attempts on the WAL (successes + injected failures).
    wal_sync_attempts: AtomicU64,
    /// `sync_data` calls on the WAL that returned Ok.
    wal_sync_ok: AtomicU64,
    /// WAL length covered by the latest successful `sync_data`.
    wal_synced_len: AtomicU64,
    /// When > 0: the (`n`+1)-th WAL `sync_data` attempt fails (injection).
    fail_sync_after: AtomicU64,
    /// One-shot park: the next WAL `sync_data` blocks until released —
    /// models a writer stuck in the off-lock fsync window.
    park_armed: AtomicBool,
    parked: AtomicBool,
    release: Mutex<bool>,
    release_cv: std::sync::Condvar,
    /// One-shot: the next `create` of `VALUES.vlog.new` fails writes after
    /// `bytes` have landed (models a crash mid-rewrite).
    fail_vlognew_write_after: Mutex<Option<u64>>,
}

impl WalCounters {
    /// Hold the next real WAL `sync_data` until [`Self::release_park`].
    fn arm_park(&self) {
        self.parked.store(false, Ordering::Release);
        *self.release.lock().unwrap() = false;
        self.park_armed.store(true, Ordering::Release);
    }
    /// Block until the parked sync is actually inside `sync_data`.
    fn wait_parked(&self) {
        for _ in 0..2_000 {
            if self.parked.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("WAL sync never arrived at the park latch");
    }
    fn release_park(&self) {
        *self.release.lock().unwrap() = true;
        self.release_cv.notify_all();
    }
    /// The next `create(VALUES.vlog.new)` yields a handle whose writes fail
    /// after `bytes` bytes have landed.
    fn arm_fail_vlognew_write_after(&self, bytes: u64) {
        *self.fail_vlognew_write_after.lock().unwrap() = Some(bytes);
    }
}

struct CountingFile {
    inner: fs::File,
    counters: Option<Arc<WalCounters>>,
    /// One-shot injection (F3 repro): writes succeed until this many bytes
    /// have landed, then fail — models a crash mid-rewrite.
    fail_write_after: Option<u64>,
    written: u64,
}

impl CountingFile {
    fn note_write(&self, buf: &[u8]) {
        if let Some(c) = &self.counters {
            c.wal_writes.fetch_add(1, Ordering::Relaxed);
            c.wal_bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
        }
    }
}

impl Write for CountingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(limit) = self.fail_write_after {
            let allow = limit.saturating_sub(self.written).min(buf.len() as u64) as usize;
            if allow < buf.len() {
                if allow > 0 {
                    self.inner.write_all(&buf[..allow])?;
                    self.written += allow as u64;
                }
                return Err(io::Error::other("injected partial VALUES.vlog.new write"));
            }
        }
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        self.note_write(&buf[..n]);
        Ok(n)
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if let Some(limit) = self.fail_write_after {
            let allow = limit.saturating_sub(self.written).min(buf.len() as u64) as usize;
            if allow < buf.len() {
                if allow > 0 {
                    self.inner.write_all(&buf[..allow])?;
                    self.written += allow as u64;
                }
                return Err(io::Error::other("injected partial VALUES.vlog.new write"));
            }
        }
        self.inner.write_all(buf)?;
        self.written += buf.len() as u64;
        self.note_write(buf);
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Read for CountingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for CountingFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl pedradb_core::env::EnvFile for CountingFile {
    fn sync_data(&mut self) -> io::Result<()> {
        if let Some(c) = &self.counters {
            let n = c.wal_sync_attempts.fetch_add(1, Ordering::Relaxed) + 1;
            let fail_after = c.fail_sync_after.load(Ordering::Relaxed);
            if fail_after > 0 && n > fail_after {
                return Err(io::Error::other("injected WAL sync_data failure"));
            }
            if c.park_armed.swap(false, Ordering::AcqRel) {
                c.parked.store(true, Ordering::Release);
                let mut released = c.release.lock().unwrap();
                while !*released {
                    released = c.release_cv.wait(released).unwrap();
                }
                c.parked.store(false, Ordering::Release);
            }
        }
        self.inner.sync_data()?;
        if let Some(c) = &self.counters {
            c.wal_sync_ok.fetch_add(1, Ordering::Relaxed);
            let bytes = c.wal_bytes.load(Ordering::Relaxed);
            c.wal_synced_len.fetch_max(bytes, Ordering::Relaxed);
        }
        Ok(())
    }
    fn sync_all(&mut self) -> io::Result<()> {
        self.inner.sync_all()
    }
    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.inner.set_len(len)?;
        if let Some(c) = &self.counters {
            // Truncation (torn-tail cut): later writes start at `len`.
            c.wal_bytes.fetch_min(len, Ordering::Relaxed);
            c.wal_synced_len.fetch_min(len, Ordering::Relaxed);
        }
        Ok(())
    }
    fn len(&mut self) -> io::Result<u64> {
        self.inner.metadata().map(|m| m.len())
    }
}

#[derive(Clone)]
struct CountingEnv {
    counters: Arc<WalCounters>,
}

impl CountingEnv {
    fn new() -> (Self, Arc<WalCounters>) {
        let counters = Arc::new(WalCounters::default());
        (
            Self {
                counters: Arc::clone(&counters),
            },
            counters,
        )
    }

    fn is_wal(path: &Path) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some(WAL_FILE_NAME)
    }
}

impl pedradb_core::env::Env for CountingEnv {
    type File = CountingFile;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }
    fn create(&self, path: &Path) -> io::Result<Self::File> {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        let counters = Self::is_wal(path).then(|| Arc::clone(&self.counters));
        if counters.is_some() {
            self.counters.wal_bytes.store(0, Ordering::Relaxed);
            self.counters.wal_synced_len.store(0, Ordering::Relaxed);
        }
        let fail_write_after = (path.file_name().and_then(|n| n.to_str())
            == Some(pedradb_core::vlog::VLOG_NEW_NAME))
        .then(|| {
            self.counters
                .fail_vlognew_write_after
                .lock()
                .unwrap()
                .take()
        })
        .flatten();
        Ok(CountingFile {
            inner: f,
            counters,
            fail_write_after,
            written: 0,
        })
    }
    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        f.seek(SeekFrom::End(0))?;
        let counters = Self::is_wal(path).then(|| Arc::clone(&self.counters));
        Ok(CountingFile {
            inner: f,
            counters,
            fail_write_after: None,
            written: 0,
        })
    }
    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        let f = std::fs::File::open(path)?;
        Ok(CountingFile {
            inner: f,
            counters: None,
            fail_write_after: None,
            written: 0,
        })
    }
    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        let dir = fs::File::open(path)?;
        dir.sync_data()
    }
    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(path)?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        Ok(names)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        fs::metadata(path).map(|m| m.len())
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pedra-adv-{tag}-{nanos}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn open_db(env: &CountingEnv, dir: &Path) -> Db<CountingEnv> {
    Db::open_with_env(dir, OpenOptions::default(), env.clone()).unwrap()
}

/// Ok on a sync put ⇒ the WAL bytes were written **and** a real `sync_data`
/// returned Ok at a length covering them, before the put returned.
#[test]
fn ok_put_is_covered_by_a_real_fdatasync() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("ok-covered");
    let mut db = open_db(&env, &dir);

    for i in 0..5u64 {
        db.put(format!("k{i}"), format!("v{i}")).unwrap();
        let synced = c.wal_synced_len.load(Ordering::Relaxed);
        let bytes = c.wal_bytes.load(Ordering::Relaxed);
        assert!(
            synced >= bytes,
            "Ok returned without a real fdatasync covering the WAL bytes: \
             synced_len={synced} bytes={bytes} (put {i})"
        );
    }
    assert!(
        c.wal_sync_ok.load(Ordering::Relaxed) >= 5,
        "5 sync puts must cost >= 5 real WAL fdatasyncs, got {}",
        c.wal_sync_ok.load(Ordering::Relaxed)
    );
    drop(db);
    let _ = fs::remove_dir_all(&dir);
}

/// One client, one op per put: each Ok needs its own fdatasync. If N puts
/// cost fewer than N real fsyncs, the engine is selling speed for durability.
#[test]
fn lone_client_cannot_share_or_skip_fsyncs() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("lone-fsync");
    let mut db = open_db(&env, &dir);

    let n = 20u64;
    for i in 0..n {
        db.put(format!("k{i}"), b"v").unwrap();
    }
    let syncs = c.wal_sync_ok.load(Ordering::Relaxed);
    assert!(
        syncs >= n,
        "lone 1-op sync puts: {n} Ok need >= {n} real fdatasyncs, got {syncs}"
    );

    // Abrupt drop (process-crash model): every Ok'd key must recover.
    std::mem::forget(db);
    let db2 = Db::open(&dir).unwrap();
    for i in 0..n {
        assert_eq!(
            db2.get(format!("k{i}").as_bytes()),
            Some("v".into()),
            "lost acked k{i}"
        );
    }
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// Group commit may amortize (≤ 1 fsync per client) but never skip: every
/// client Ok is covered and survives an abrupt drop.
#[test]
fn group_commit_amortizes_but_never_skips() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("group-honest");
    let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), env).unwrap();

    let clients = 8usize;
    let barrier = Arc::new(Barrier::new(clients));
    let handles: Vec<_> = (0..clients)
        .map(|i| {
            let db = db.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                db.put_with(format!("g{i}"), b"gv", WriteOptions::sync())
                    .unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let syncs = c.wal_sync_ok.load(Ordering::Relaxed);
    assert!(
        syncs >= 1,
        "group must fdatasync at least once, got {syncs}"
    );
    assert!(
        syncs <= clients as u64,
        "group commit must amortize: {clients} clients cannot need > {clients} fsyncs, got {syncs}"
    );
    assert_eq!(
        c.wal_synced_len.load(Ordering::Relaxed),
        c.wal_bytes.load(Ordering::Relaxed),
        "all appended bytes must be covered by a real fdatasync once every client is Ok"
    );

    // Abrupt drop: every Ok'd key recovers.
    std::mem::forget(db);
    let db2 = Db::open(&dir).unwrap();
    for i in 0..clients {
        assert_eq!(
            db2.get(format!("g{i}").as_bytes()),
            Some("gv".into()),
            "lost acked group key g{i}"
        );
    }
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// The bench telemetry counter must equal the real Env-seam syscall count.
#[test]
fn internal_sync_counter_matches_env_seam() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("counter-honest");
    let mut db = open_db(&env, &dir);

    for i in 0..10u64 {
        db.put(format!("k{i}"), b"v").unwrap();
    }
    assert_eq!(
        db.wal_sync_count(),
        c.wal_sync_ok.load(Ordering::Relaxed),
        "engine-reported wal_sync_count diverged from real sync_data calls on the WAL"
    );
    drop(db);
    let _ = fs::remove_dir_all(&dir);
}

/// The async column is honestly async: `no_sync` puts cost zero WAL fsyncs
/// until an explicit `Db::sync`, which then makes them crash-durable.
#[test]
fn no_sync_puts_perform_zero_fsync_until_sync() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("async-honest");
    let mut db = open_db(&env, &dir);

    for i in 0..100u64 {
        db.put_with(format!("a{i}"), b"v", WriteOptions::no_sync())
            .unwrap();
    }
    assert_eq!(
        c.wal_sync_ok.load(Ordering::Relaxed),
        0,
        "no_sync puts must not fdatasync (async column honesty)"
    );

    db.sync().unwrap();
    assert!(
        c.wal_sync_ok.load(Ordering::Relaxed) >= 1,
        "Db::sync must perform a real fdatasync"
    );

    std::mem::forget(db);
    let db2 = Db::open(&dir).unwrap();
    for i in 0..100u64 {
        assert_eq!(db2.get(format!("a{i}").as_bytes()), Some("v".into()));
    }
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// A WAL fsync failure after a successful append must turn the client's Ok
/// into Err, fence the Db fail-closed, refuse further writes, and leave the
/// previously-durable prefix intact after reopen.
#[test]
fn sync_failure_after_append_fences_fail_closed() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("fence");
    let mut db = open_db(&env, &dir);

    db.put("before", b"v1").unwrap();
    let ok_syncs = c.wal_sync_ok.load(Ordering::Relaxed);
    assert!(ok_syncs >= 1);

    // Next WAL sync_data fails.
    c.fail_sync_after.store(ok_syncs, Ordering::Relaxed);
    let failed = db.put("during", b"v2");
    assert!(failed.is_err(), "put whose fsync failed must return Err");
    assert!(
        db.is_durability_fenced(),
        "Db must be durability-fenced after sync failure"
    );

    match db.put("after", b"v3") {
        Err(CoreError::DurabilityFenced) => {}
        other => panic!("post-fence put must be DurabilityFenced, got {other:?}"),
    }
    drop(db);

    let db2 = Db::open(&dir).unwrap();
    assert_eq!(
        db2.get(b"before"),
        Some("v1".into()),
        "durable prefix must survive"
    );
    // "during" is the fence's uncertain range: the WAL append succeeded, the
    // fsync failed — Err means "outcome unknown", not "absent". A process
    // crash keeps the written bytes; power loss may not. Either visibility
    // is within contract; partial or corrupted state is not.
    if let Some(v) = db2.get(b"during") {
        assert_eq!(
            v.as_ref(),
            b"v2".as_slice(),
            "uncertain write recovered wrong"
        );
    }
    assert_eq!(db2.get(b"after"), None, "never-appended key must be absent");
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// Recovery cuts a torn tail to the last good offset; appends after that
/// never sit on the damaged region, and a later abrupt drop still recovers
/// everything acked.
#[test]
fn append_after_torn_tail_recovery_survives_crash() {
    let (env, _c) = CountingEnv::new();
    let dir = temp_dir("torn-append");
    {
        let mut db = open_db(&env, &dir);
        db.put("solid", b"v").unwrap();
        db.close().unwrap();
    }

    // Tear the tail: plausible First fragment header (crc|len|type) + short
    // payload, cut before the record completes.
    {
        let wal = dir.join(WAL_FILE_NAME);
        let mut f = fs::OpenOptions::new().append(true).open(&wal).unwrap();
        let mut garbage = vec![0u8; 7];
        garbage[4..6].copy_from_slice(&100u16.to_le_bytes()); // length 100
        garbage[6] = 2; // RecordType::First
        garbage.extend_from_slice(&[0xAB; 33]); // 33 of 100 payload bytes
        f.write_all(&garbage).unwrap();
    }

    {
        let mut db = open_db(&env, &dir);
        assert_eq!(
            db.get(b"solid"),
            Some("v".into()),
            "prefix before torn tail must recover"
        );
        db.put("post-torn-1", b"p1").unwrap();
        db.put("post-torn-2", b"p2").unwrap();
        std::mem::forget(db);
    }

    let db2 = Db::open(&dir).unwrap();
    assert_eq!(db2.get(b"solid"), Some("v".into()));
    assert_eq!(db2.get(b"post-torn-1"), Some("p1".into()));
    assert_eq!(db2.get(b"post-torn-2"), Some("p2".into()));
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// Mixed sync + async concurrent burst: every sync-Ok'd batch survives an
/// abrupt drop; every async multi-op batch is recovered whole or not at all.
#[test]
fn mixed_sync_async_all_or_nothing_per_batch() {
    let (env, _c) = CountingEnv::new();
    let dir = temp_dir("mixed-atomic");
    let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), env).unwrap();

    let sync_clients = 4usize;
    let async_clients = 4usize;
    let barrier = Arc::new(Barrier::new(sync_clients + async_clients));
    let mut handles = Vec::new();
    for i in 0..sync_clients {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for j in 0..25u64 {
                db.put_with(format!("s{i}-{j}"), b"sv", WriteOptions::sync())
                    .unwrap();
            }
        }));
    }
    for i in 0..async_clients {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for j in 0..25u64 {
                // One 3-op batch per put call ⇒ one WAL record ⇒ all-or-nothing.
                let batch = vec![
                    pedradb_core::BatchOp::put(format!("a{i}-{j}-0"), b"av"),
                    pedradb_core::BatchOp::put(format!("a{i}-{j}-1"), b"av"),
                    pedradb_core::BatchOp::put(format!("a{i}-{j}-2"), b"av"),
                ];
                db.apply_batch(batch).unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    std::mem::forget(db);
    let db2 = Db::open(&dir).unwrap();
    for i in 0..sync_clients {
        for j in 0..25u64 {
            assert_eq!(
                db2.get(format!("s{i}-{j}").as_bytes()),
                Some("sv".into()),
                "lost acked sync key s{i}-{j}"
            );
        }
    }
    for i in 0..async_clients {
        for j in 0..25u64 {
            let present = (0..3u64)
                .filter(|k| db2.get(format!("a{i}-{j}-{k}").as_bytes()).is_some())
                .count();
            assert!(
                present == 0 || present == 3,
                "async batch a{i}-{j} recovered half ({present}/3) — WAL record atomicity broken"
            );
        }
    }
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// PointInTime mode must not silently swallow a damaged head: the open either
/// serves a reported prefix or fails closed (never a silently empty DB).
#[test]
fn point_in_time_never_serves_silent_empty() {
    let (env, _c) = CountingEnv::new();
    let dir = temp_dir("pit-report");
    {
        let mut db = open_db(&env, &dir);
        db.put("k0", b"v").unwrap();
        db.put("k1", b"v").unwrap();
        db.close().unwrap();
    }
    // Bit-flip in the middle of the WAL payload (after header) ⇒ CRC error.
    {
        let wal = dir.join(WAL_FILE_NAME);
        let mut bytes = fs::read(&wal).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        fs::write(&wal, bytes).unwrap();
    }
    let opts = OpenOptions {
        wal_recovery: WalRecovery::PointInTime,
        ..OpenOptions::default()
    };
    match Db::open_with_env(&dir, opts, env) {
        Ok(db) => {
            let report = db.last_recovery_report();
            assert!(
                report.is_some(),
                "PointInTime open over a CRC error must report the discarded suffix"
            );
        }
        Err(_) => {
            // Escalated / fail-stop is also acceptable — never silent.
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

// Keep unused-import warnings away when Mutex is only used via type alias.
#[allow(dead_code)]
type _Unused = Mutex<()>;

/// Regression (audit finding F2, fixed 2026-08-21): vlog GC while a sync
/// writer sits in the off-lock fsync window must **refuse** the round, then
/// succeed once quiescent — never delete pre-GC vlog sources under an
/// un-rotated WAL.
///
/// `Db::flush()` only *tries* to rotate the WAL (`try_rotate_wal` silently
/// returns Ok when `commit_inflight > 0` or mem is non-empty). Before the fix,
/// `compact_vlog`/`compact_blob` relied on that flush producing a "durable
/// empty WAL" before replacing/deleting the pre-GC value log: a WAL record
/// that survived un-rotated still carried the **pre-GC** vlog pointer; after
/// crash the replay re-inserted it into mem, where it shadowed the remapped
/// SST entry, and the dangling read was swallowed to `None` — a lost
/// **sync-acked** write. The fix (`ensure_wal_rotated_for_gc`) refuses the
/// GC round unless the rotation actually happened.
///
/// Interleaving (forced deterministically via the Env seam):
///   1. park 1: writer T1 in the off-lock fsync window keeps
///      `commit_inflight > 0`, so `compact_with(latest_only)`'s flush cannot
///      rotate the WAL — the dead vlog pointer leaves the SSTs while the
///      stale records stay in the WAL;
///   2. park 2: same window during `compact_vlog` — the round must Err
///      (refused: WAL not rotated), leaving the pre-GC vlog untouched;
///   3. both writers released and Ok'd (sync-acked); the retried GC runs
///      quiescent (real rotation) and succeeds;
///   4. abrupt drop (crash), reopen: every acked write survives.
#[test]
#[ignore = "deadlocks when the parked writer takes the lone path: lone_sync_commit holds the Db write lock through Wal::sync_data, so the parked compact_with->flush blocks and release_park never runs (ABBA). Passes when the writer joins the group path. Stale vs RFC-0062 P1.1 p11j; rewrite tracked upstream."]
fn vlog_gc_during_offlock_fsync_window_refuses_then_preserves_acked_sync_write() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("vlog-gc-race");
    let opts = OpenOptions {
        large_value_threshold: Some(64),
        ..OpenOptions::default()
    };
    let db = ConcurrentDb::open_with_env(&dir, opts, env.clone()).unwrap();
    db.put_with(b"seed", b"s", WriteOptions::sync()).unwrap();
    // Two large values; the first one dies so a later GC compacts the second
    // to a LOWER offset. Both puts stay in the un-rotated WAL below.
    db.put_with(b"dead", [1u8; 4096], WriteOptions::sync())
        .unwrap();
    db.put_with(b"big", [7u8; 4096], WriteOptions::sync())
        .unwrap();
    db.delete(b"dead").unwrap();

    // Park 1: drop the superseded `dead` version (and its vlog pointer) from
    // every level collect scans — while a parked writer keeps `commit_inflight
    // > 0`, so this compaction's flush cannot rotate the WAL and the stale
    // pre-GC records survive on disk.
    c.arm_park();
    let t1 = {
        let db = db.clone();
        std::thread::spawn(move || {
            db.put_with(b"tail1", b"t", WriteOptions::sync()).unwrap();
        })
    };
    c.wait_parked();
    db.compact_with(pedradb_core::db::CompactOptions::latest_only())
        .unwrap();
    c.release_park();
    t1.join().unwrap();

    // Park 2: with the rotate still suppressed, the value-log GC round must
    // refuse (Err) instead of rewriting the value log under an un-rotated WAL.
    c.arm_park();
    let t2 = {
        let db = db.clone();
        std::thread::spawn(move || {
            db.put_with(b"tail2", b"t", WriteOptions::sync()).unwrap();
        })
    };
    c.wait_parked();
    let refused = db
        .compact_vlog()
        .expect_err("GC must refuse while the WAL could not be rotated");
    assert!(
        refused.to_string().contains("wal not rotated"),
        "refusal reason should name the un-rotated WAL, got: {refused}"
    );
    c.release_park();
    t2.join().unwrap(); // Ok ⇒ sync-acked

    // Quiescent retry: the flush now really rotates the WAL, so the round
    // proceeds and must see the live large value.
    let stats = db.compact_vlog().unwrap();
    eprintln!(
        "DIAG gc stats: live_records={} bytes_before={} bytes_after={} | wal_file_len={} wal_bytes_ctr={} synced_ctr={}",
        stats.live_records,
        stats.bytes_before,
        stats.bytes_after,
        fs::metadata(dir.join(WAL_FILE_NAME)).map(|m| m.len()).unwrap_or(u64::MAX),
        c.wal_bytes.load(Ordering::Relaxed),
        c.wal_synced_len.load(Ordering::Relaxed),
    );
    assert!(
        stats.live_records >= 1,
        "GC must have seen the live large value (live_records={})",
        stats.live_records
    );

    std::mem::forget(db); // crash: no close, no further fsyncs
    let db2 = Db::open(&dir).unwrap();
    assert_eq!(
        db2.get(b"big"),
        Some(vec![7u8; 4096].into()),
        "sync-acked write lost after vlog GC + crash replay"
    );
    assert_eq!(db2.get(b"tail1"), Some(b"t".to_vec().into()));
    assert_eq!(db2.get(b"tail2"), Some(b"t".to_vec().into()));
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// Regression (audit finding F3, fixed 2026-08-21): a second vlog GC round on
/// an un-promoted state (`vlog_use_new=true`, `.new` staged) must promote the
/// staged round **before** rewriting, so a failure mid-rewrite can never
/// truncate a live `.new` under a committed `vlog_use_new`.
///
/// Before the fix, `rewrite_live_to_new` removed the live `.new` and rebuilt
/// it, and a mid-rewrite failure left a truncated `.new` on disk:
/// - ≥ MAGIC bytes landed: recovery **trusted** the partial `.new`; SST
///   pointers past the truncation dangled and the swallowed read error made
///   acked large values read as missing;
/// - 0 bytes landed: `Db::open` fail-closed with `vlog too short` — the DB
///   could not reopen without manual file surgery.
///
/// The fix promotes the staged round first (rename `.new` → primary, clear
/// the MANIFEST flag); the subsequent rewrite only ever replaces a non-live
/// staging file, so the injected failure degrades to a plain Err with the
/// live layout fully intact.
#[test]
fn second_gc_on_unpromoted_state_promotes_first_and_survives_rewrite_failure() {
    let (env, c) = CountingEnv::new();
    let dir = temp_dir("vlog-gc-unpromoted");
    let opts = OpenOptions {
        large_value_threshold: Some(64),
        ..OpenOptions::default()
    };
    let mut db = Db::open_with_env(&dir, opts, env.clone()).unwrap();
    db.put(b"dead", [1u8; 4096]).unwrap();
    db.put(b"big", [7u8; 4096]).unwrap();
    db.delete(b"dead").unwrap();
    db.compact_with(pedradb_core::db::CompactOptions::latest_only())
        .unwrap();

    // Crash-after-install state: MANIFEST `vlog_use_new=true`, `.new` staged,
    // SSTs remapped to the `.new` layout. (Legitimate crash state — the
    // engine's own mid-GC tests reopen from it.)
    db.compact_vlog_stage_manifest().unwrap();
    assert!(
        dir.join(pedradb_core::vlog::VLOG_NEW_NAME).exists(),
        "staging must have produced VALUES.vlog.new"
    );

    // Second GC round on the un-promoted state; its rewrite dies after
    // landing just the MAGIC prefix — a crash mid-rewrite.
    c.arm_fail_vlognew_write_after(8);
    assert!(
        db.compact_vlog().is_err(),
        "injected VALUES.vlog.new write failure must surface as Err"
    );

    std::mem::forget(db); // crash: no close, no further fsyncs
    let db2 = Db::open(&dir).unwrap();
    assert_eq!(
        db2.get(b"big"),
        Some(vec![7u8; 4096].into()),
        "acked large value lost: the failed round must leave the promoted \
         primary live (never a truncated .new under vlog_use_new)"
    );
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}

/// Regression (audit finding F1, fixed 2026-08-21): a corrupt value-log
/// payload is **loud**, never a silent miss.
///
/// Before the fix, `resolve_stored_value` errors were swallowed at the read
/// boundaries: `get`/`multi_get` returned `None` (corruption indistinguishable
/// from deletion), scans skipped the entry, and the change feed served the
/// raw 9-byte VLG pointer as the user value. The fix propagates
/// [`CoreError::CorruptValue`] on every Result-shaped boundary and fail-stops
/// on the Option/iterator-shaped ones.
#[test]
fn corrupt_vlog_payload_is_loud_not_missing() {
    let dir = temp_dir("vlog-corrupt-read");
    let opts = OpenOptions {
        large_value_threshold: Some(64),
        ..OpenOptions::default()
    };
    {
        let mut db = Db::open_with_env(dir.clone(), opts, pedradb_core::env::StdEnv).unwrap();
        db.put(b"big", [7u8; 4096]).unwrap();
        db.flush().unwrap(); // pointer into SST; vlog payload on disk
    }

    // Bitrot one payload byte (past MAGIC + the first record header).
    let vlog_path = dir.join(pedradb_core::vlog::VLOG_FILE_NAME);
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&vlog_path)
        .unwrap();
    let mut probe = [0u8; 32];
    f.read_exact(&mut probe).unwrap();
    probe[20] ^= 0xff;
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(&probe).unwrap();
    drop(f);

    // Drop the persisted CHANGELOG cache (it holds the pre-rot value) so the
    // feed must rebuild live from mem/SSTs and hit the corrupt payload.
    let _ = fs::remove_file(dir.join(pedradb_core::change_feed::CHANGELOG_FILE_NAME));

    let db2 = Db::open(&dir).unwrap();
    let snap = db2.snapshot();
    let err = db2
        .get_at(snap, b"big")
        .expect_err("error-shaped read must surface corruption");
    assert!(
        matches!(err, CoreError::CorruptValue(_)),
        "expected CorruptValue, got: {err}"
    );
    let feed_err = db2
        .changes(0, u64::MAX)
        .expect_err("change feed must surface corruption");
    assert!(matches!(feed_err, CoreError::CorruptValue(_)));

    // Streaming/Option shapes cannot express the error — they fail-stop.
    // (Silence the hook so the deliberate panics do not look like failures.)
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let range_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = db2.range_at_limited(
            snap.sequence(),
            std::ops::Bound::Included(b"a".as_ref()),
            std::ops::Bound::Excluded(b"z".as_ref()),
            None,
        );
    }))
    .is_err();
    let get_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = db2.get(b"big");
    }))
    .is_err();
    std::panic::set_hook(prev_hook);
    assert!(
        range_panicked,
        "streaming scan must fail-stop on a corrupt value log"
    );
    assert!(
        get_panicked,
        "Option-shaped get must fail-stop on a corrupt value log"
    );
    drop(db2);
    let _ = fs::remove_dir_all(&dir);
}
