//! Fsync amortization bench: where legitimate write speed comes from.
//!
//! Runs each shape with an Env-seam counter on the WAL file so every number
//! comes with the **real** `fdatasync` count that produced it:
//!
//! - `lone_sync_1c`   — one client, one 1-op put per Ok: exactly one real
//!   fdatasync per op. Throughput is bounded by `1/t_fd` (RFC-0041 P1.2);
//!   any "5× Rocks async" claim on this shape would have to be a durability
//!   bug, not an optimization.
//! - `group_sync_{2,4,8}c` — ConcurrentDb write group: C clients share the
//!   leader's fdatasync. ops/fsync ≈ C is the honest amortization ceiling
//!   (never more ops per fsync than concurrent clients).
//! - `lone_async_1c`  — `WriteOptions::no_sync` (Rocks `sync=false` class):
//!   zero fdatasyncs; the CPU-only column (RFC-0044).
//!
//! Run: `cargo bench -p pedradb-core --bench fsync_amortization`

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::env::{Env, EnvFile};
use pedradb_core::{Db, OpenOptions, WriteOptions, WAL_FILE_NAME};

/// Counts `write` / `sync_data` syscalls on the WAL file (Env seam, not
/// engine telemetry).
#[derive(Default)]
struct WalIo {
    bytes: AtomicU64,
    syncs: AtomicU64,
}

struct CountingFile {
    inner: fs::File,
    io: Option<Arc<WalIo>>,
}

impl Write for CountingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        if let Some(c) = &self.io {
            c.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
        Ok(n)
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.inner.write_all(buf)?;
        if let Some(c) = &self.io {
            c.bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
        }
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

impl EnvFile for CountingFile {
    fn sync_data(&mut self) -> io::Result<()> {
        self.inner.sync_data()?;
        if let Some(c) = &self.io {
            c.syncs.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    fn sync_all(&mut self) -> io::Result<()> {
        self.inner.sync_all()
    }
    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.inner.set_len(len)
    }
    fn len(&mut self) -> io::Result<u64> {
        self.inner.metadata().map(|m| m.len())
    }
}

#[derive(Clone)]
struct CountingEnv {
    io: Arc<WalIo>,
}

impl Env for CountingEnv {
    type File = CountingFile;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }
    fn create(&self, path: &Path) -> io::Result<Self::File> {
        let f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(CountingFile {
            inner: f,
            io: is_wal(path).then(|| Arc::clone(&self.io)),
        })
    }
    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        f.seek(SeekFrom::End(0))?;
        Ok(CountingFile {
            inner: f,
            io: is_wal(path).then(|| Arc::clone(&self.io)),
        })
    }
    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        Ok(CountingFile {
            inner: fs::File::open(path)?,
            io: None,
        })
    }
    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        fs::File::open(path)?.sync_data()
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

fn is_wal(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some(WAL_FILE_NAME)
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pedra-fsync-bench-{tag}-{n}"));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn report(name: &str, n: u64, elapsed: Duration, syncs: u64) {
    let secs = elapsed.as_secs_f64().max(1e-12);
    let ops = n as f64 / secs;
    let per_sync = if syncs == 0 {
        f64::INFINITY
    } else {
        n as f64 / syncs as f64
    };
    println!(
        "{name}: {ops:>10.0} ops/s  {secs:>7.3}s  real_wal_fdatasyncs={syncs:>6}  ops/fsync={per_sync:.2}"
    );
}

fn lone_sync(n: u64) {
    let env = CountingEnv {
        io: Arc::new(WalIo::default()),
    };
    let dir = temp_dir("lone-sync");
    let mut db = Db::open_with_env(&dir, OpenOptions::default(), env.clone()).expect("open");
    let t0 = Instant::now();
    for i in 0..n {
        db.put(format!("k{i:08}"), b"v").expect("put");
    }
    let el = t0.elapsed();
    let syncs = env.io.syncs.load(Ordering::Relaxed);
    report("lone_sync_1c   ", n, el, syncs);
    db.close().ok();
    let _ = fs::remove_dir_all(&dir);
}

fn group_sync(clients: usize, puts_per: u64) {
    let env = CountingEnv {
        io: Arc::new(WalIo::default()),
    };
    let dir = temp_dir("group-sync");
    let db = ConcurrentDb::open_with_env(&dir, OpenOptions::default(), env.clone()).expect("open");
    let barrier = Arc::new(Barrier::new(clients));
    let t0 = Instant::now();
    let handles: Vec<_> = (0..clients)
        .map(|c| {
            let db = db.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                for i in 0..puts_per {
                    db.put_with(format!("k{c:02}-{i:06}"), b"v", WriteOptions::sync())
                        .expect("put");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("join");
    }
    let el = t0.elapsed();
    let n = clients as u64 * puts_per;
    let syncs = env.io.syncs.load(Ordering::Relaxed);
    report(&format!("group_sync_{clients:2}c "), n, el, syncs);
    drop(db);
    let _ = fs::remove_dir_all(&dir);
}

fn lone_async(n: u64) {
    let env = CountingEnv {
        io: Arc::new(WalIo::default()),
    };
    let dir = temp_dir("lone-async");
    let mut db = Db::open_with_env(&dir, OpenOptions::default(), env.clone()).expect("open");
    let t0 = Instant::now();
    for i in 0..n {
        db.put_with(format!("k{i:08}"), b"v", WriteOptions::no_sync())
            .expect("put");
    }
    let el = t0.elapsed();
    db.sync().expect("sync");
    let syncs = env.io.syncs.load(Ordering::Relaxed);
    report("lone_async_1c  ", n, el, syncs);
    db.close().ok();
    let _ = fs::remove_dir_all(&dir);
}

fn main() {
    let n_lone = 2_000u64;
    let n_group = 2_000u64;
    println!("shape            :      ops/s    wall   real WAL fdatasyncs  amortization");
    lone_sync(n_lone);
    for clients in [2usize, 4, 8] {
        group_sync(clients, n_group / clients as u64);
    }
    lone_async(n_lone);
}
