//! RFC-0059 P0.1: storage backend switch for World simulation nodes.
//!
//! `WorldEnv` lets the same `World::run` schedule execute against either
//! the real filesystem (`Disk` — production-shaped I/O: extent
//! preallocation, page cache, real `fdatasync` barriers) or an
//! in-memory virtual FS (`Mem` — `RecordingEnv` wrapped by the same
//! `FailingEnv` fault layer). Fault seams are identical in both: the
//! `OpClass` gating, arm/short-write/trip state and sync op counting
//! live in the `FailingEnv` wrapper, not the backing store, so
//! buggify arms fire at the same points. `Mem` exists because the
//! physical barrier and file-metadata churn serialize on one volume
//! and cap swarm parallelism (measured: ~1.3× at 8 workers on APFS
//! with `F_FULLFSYNC` off; the CPU is >85% idle waiting on syscalls).

use std::cell::Cell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use pedradb_core::{AdviseKind, Env, EnvFile};
use pedradb_sim::{FailingEnv, FaultKind, OpClass, RecordingEnv};

thread_local! {
    /// Bound for the duration of [`crate::World::run`]. `None` = Env default
    /// (wall clock). World is single-threaded per run; swarm workers each
    /// bind their own cell. This is the fold of `Env::unix_millis` into the
    /// discrete scheduler (FDB-class: no wall-clock in the seed function).
    static LOGICAL_UNIX_MS: Cell<Option<u64>> = Cell::new(None);
}

/// RAII bind of [`WorldEnv::unix_millis`] to a seed-derived logical clock.
pub(crate) struct LogicalClockGuard;

/// Discrete Unix-ms epoch for a World seed (not wall clock).
pub(crate) fn epoch_ms(seed: u64) -> u64 {
    1_700_000_000_000u64.wrapping_add(seed)
}

impl LogicalClockGuard {
    /// Bind `Env::unix_millis` to the seed epoch (now_ms = 0).
    pub(crate) fn bind_seed(seed: u64) -> Self {
        LOGICAL_UNIX_MS.with(|c| c.set(Some(epoch_ms(seed))));
        Self
    }

    /// Keep Env time locked to cluster `now_ms` (one clock, seed-derived).
    pub(crate) fn sync_now_ms(seed: u64, now_ms: u64) {
        LOGICAL_UNIX_MS.with(|c| {
            if c.get().is_some() {
                c.set(Some(epoch_ms(seed).wrapping_add(now_ms)));
            }
        });
    }
}

impl Drop for LogicalClockGuard {
    fn drop(&mut self) {
        LOGICAL_UNIX_MS.with(|c| c.set(None));
    }
}

/// Real-FS node backend (`FailingEnv<StdEnv>`).
pub type DiskEnv = FailingEnv;
/// In-memory node backend (`FailingEnv<RecordingEnv>`).
pub type MemEnv = FailingEnv<RecordingEnv>;

/// Storage backend for one simulated node.
#[derive(Debug, Clone)]
pub enum WorldEnv {
    /// Real filesystem under the run parent dir.
    Disk(DiskEnv),
    /// In-memory virtual FS (no host I/O).
    Mem(MemEnv),
}

impl WorldEnv {
    /// Healthy backend: `mem` picks the in-memory virtual FS.
    #[must_use]
    pub fn passing(mem: bool) -> Self {
        if mem {
            Self::Mem(FailingEnv::wrap(RecordingEnv::new()))
        } else {
            Self::Disk(FailingEnv::passing())
        }
    }

    /// Arm a one-shot fault on the `after_ops`-th op of `class`
    /// (delegates to the [`FailingEnv`] wrapper in either variant).
    pub fn arm_op_class(&self, class: OpClass, after_ops: u64, transient: bool, kind: FaultKind) {
        match self {
            Self::Disk(e) => e.arm_op_class(class, after_ops, transient, kind),
            Self::Mem(e) => e.arm_op_class(class, after_ops, transient, kind),
        }
    }

    /// Arm a short-write fault every `n` bytes.
    pub fn arm_short_write(&self, n: usize) {
        match self {
            Self::Disk(e) => e.arm_short_write(n),
            Self::Mem(e) => e.arm_short_write(n),
        }
    }

    /// Arm the legacy dead-disk fault (`IoError`) after `after_ops`.
    pub fn arm(&self, after_ops: u64, transient: bool) {
        match self {
            Self::Disk(e) => e.arm(after_ops, transient),
            Self::Mem(e) => e.arm(after_ops, transient),
        }
    }

    /// Delay every op by `ticks` (logical busy fault).
    pub fn set_delay_per_op(&self, ticks: u64) {
        match self {
            Self::Disk(e) => e.set_delay_per_op(ticks),
            Self::Mem(e) => e.set_delay_per_op(ticks),
        }
    }

    /// Clear any armed fault.
    pub fn disarm(&self) {
        match self {
            Self::Disk(e) => e.disarm(),
            Self::Mem(e) => e.disarm(),
        }
    }

    /// Whether any fault has tripped on this node's env.
    #[must_use]
    pub fn tripped(&self) -> bool {
        match self {
            Self::Disk(e) => e.tripped(),
            Self::Mem(e) => e.tripped(),
        }
    }

    /// Process-crash the backing image: drop unsynced (`pending`) bytes.
    /// Disk is host-durable after Ok+sync; Mem is [`RecordingEnv::crash`].
    pub fn crash_unsynced(&self) {
        match self {
            Self::Mem(e) => e.inner().crash(),
            Self::Disk(_) => {}
        }
    }
}

/// File handle for [`WorldEnv`] (variant of the wrapped env's file).
pub enum WorldFile {
    /// Real-FS handle.
    Disk(<DiskEnv as Env>::File),
    /// In-memory handle.
    Mem(<MemEnv as Env>::File),
}

impl Read for WorldFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Disk(f) => f.read(buf),
            Self::Mem(f) => f.read(buf),
        }
    }
}

impl Write for WorldFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Disk(f) => f.write(buf),
            Self::Mem(f) => f.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Disk(f) => f.flush(),
            Self::Mem(f) => f.flush(),
        }
    }
}

impl Seek for WorldFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Disk(f) => f.seek(pos),
            Self::Mem(f) => f.seek(pos),
        }
    }
}

impl EnvFile for WorldFile {
    fn sync_data(&mut self) -> io::Result<()> {
        match self {
            Self::Disk(f) => f.sync_data(),
            Self::Mem(f) => f.sync_data(),
        }
    }
    /// Delegated (not defaulted): the strong/weak class distinction is
    /// the product behavior `wal_full_fsync` selects.
    fn sync_data_strong(&mut self) -> io::Result<()> {
        match self {
            Self::Disk(f) => f.sync_data_strong(),
            Self::Mem(f) => f.sync_data_strong(),
        }
    }
    fn sync_all(&mut self) -> io::Result<()> {
        match self {
            Self::Disk(f) => f.sync_all(),
            Self::Mem(f) => f.sync_all(),
        }
    }
    fn set_len(&mut self, len: u64) -> io::Result<()> {
        match self {
            Self::Disk(f) => f.set_len(len),
            Self::Mem(f) => f.set_len(len),
        }
    }
    fn preallocate(&mut self, len: u64) -> io::Result<()> {
        match self {
            Self::Disk(f) => f.preallocate(len),
            Self::Mem(f) => f.preallocate(len),
        }
    }
    fn len(&mut self) -> io::Result<u64> {
        match self {
            Self::Disk(f) => f.len(),
            Self::Mem(f) => f.len(),
        }
    }
}

impl Env for WorldEnv {
    type File = WorldFile;

    fn unix_millis(&self) -> u64 {
        if let Some(t) = LOGICAL_UNIX_MS.with(|c| c.get()) {
            return t;
        }
        match self {
            Self::Disk(e) => e.unix_millis(),
            Self::Mem(e) => e.unix_millis(),
        }
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        match self {
            Self::Disk(e) => e.create_dir_all(path),
            Self::Mem(e) => e.create_dir_all(path),
        }
    }
    fn create(&self, path: &Path) -> io::Result<Self::File> {
        match self {
            Self::Disk(e) => Ok(WorldFile::Disk(e.create(path)?)),
            Self::Mem(e) => Ok(WorldFile::Mem(e.create(path)?)),
        }
    }
    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        match self {
            Self::Disk(e) => Ok(WorldFile::Disk(e.open_append(path)?)),
            Self::Mem(e) => Ok(WorldFile::Mem(e.open_append(path)?)),
        }
    }
    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        match self {
            Self::Disk(e) => Ok(WorldFile::Disk(e.open_read(path)?)),
            Self::Mem(e) => Ok(WorldFile::Mem(e.open_read(path)?)),
        }
    }
    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        match self {
            Self::Disk(e) => e.sync_dir(path),
            Self::Mem(e) => e.sync_dir(path),
        }
    }
    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
        // Fold OS / HashMap listing order into the seed function (FDB-class).
        let mut names = match self {
            Self::Disk(e) => e.read_dir_names(path)?,
            Self::Mem(e) => e.read_dir_names(path)?,
        };
        names.sort_unstable();
        Ok(names)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        match self {
            Self::Disk(e) => e.remove_file(path),
            Self::Mem(e) => e.remove_file(path),
        }
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self {
            Self::Disk(e) => e.rename(from, to),
            Self::Mem(e) => e.rename(from, to),
        }
    }
    fn exists(&self, path: &Path) -> bool {
        match self {
            Self::Disk(e) => e.exists(path),
            Self::Mem(e) => e.exists(path),
        }
    }
    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        match self {
            Self::Disk(e) => e.metadata_len(path),
            Self::Mem(e) => e.metadata_len(path),
        }
    }
    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        match self {
            Self::Disk(e) => e.is_dir(path),
            Self::Mem(e) => e.is_dir(path),
        }
    }
    fn copy_file(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self {
            Self::Disk(e) => e.copy_file(from, to),
            Self::Mem(e) => e.copy_file(from, to),
        }
    }
    fn advise(&self, path: &Path, offset: u64, len: u64, kind: AdviseKind) -> io::Result<()> {
        match self {
            Self::Disk(e) => e.advise(path, offset, len, kind),
            Self::Mem(e) => e.advise(path, offset, len, kind),
        }
    }
}
