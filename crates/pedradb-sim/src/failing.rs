//! [`FailingEnv`]: depot-store `FailingMedia` pattern for PedraDB.
//!
//! Injects `io::Error` on the Nth fallible Env operation (and optionally every
//! op after — dead disk). Shared trip state across clones via `Rc`.

use std::cell::Cell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::rc::Rc;

use pedradb_core::env::{Env, EnvFile, StdEnv};

/// Which `io::ErrorKind` (and message) to inject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaultKind {
    /// Generic dead disk (`ErrorKind::Other`).
    #[default]
    IoError,
    /// Disk full (`ErrorKind::StorageFull` / ENOSPC).
    StorageFull,
    /// Permission denied.
    PermissionDenied,
    /// Interrupted system call (often retried by callers — useful stress).
    Interrupted,
    /// Failure only on `sync_data` / `sync_all` / `sync_dir` (write already happened).
    SyncFail,
}

impl FaultKind {
    pub(crate) fn to_error(self) -> io::Error {
        match self {
            Self::IoError => io::Error::other("injected fault"),
            Self::StorageFull => io::Error::new(io::ErrorKind::StorageFull, "injected ENOSPC"),
            Self::PermissionDenied => {
                io::Error::new(io::ErrorKind::PermissionDenied, "injected EACCES")
            }
            Self::Interrupted => io::Error::new(io::ErrorKind::Interrupted, "injected EINTR"),
            Self::SyncFail => io::Error::other("injected sync failure"),
        }
    }

    pub(crate) fn is_sync_only(self) -> bool {
        matches!(self, Self::SyncFail)
    }
}

/// Shared arm/trip state (clone-shared like RBS `FailState`).
#[derive(Debug, Default)]
struct FailState {
    remaining: Cell<u64>,
    fired: Cell<bool>,
    once: Cell<bool>,
    kind: Cell<FaultKind>,
    /// When true, only gate sync_* ops (writes/opens pass).
    sync_only: Cell<bool>,
}

impl FailState {
    fn gate(&self, is_sync: bool) -> io::Result<()> {
        let kind = self.kind.get();
        if (kind.is_sync_only() || self.sync_only.get()) && !is_sync {
            return Ok(());
        }
        let left = self.remaining.get();
        if left == 0 {
            if self.once.get() && self.fired.get() {
                return Ok(());
            }
            self.fired.set(true);
            return Err(kind.to_error());
        }
        // Non-sync-only: every fallible op counts. Sync-only: only sync ops count.
        if kind.is_sync_only() || self.sync_only.get() {
            if is_sync {
                self.remaining.set(left - 1);
            }
        } else {
            self.remaining.set(left - 1);
        }
        Ok(())
    }
}

/// Test [`Env`]: passthrough to [`StdEnv`] that injects faults.
///
/// Models:
/// - **dead disk**: `fail_after(n)` — N ops succeed, then permanent failure
/// - **one-shot**: `arm_one_failure()` / `arm(n, true)` — single blip then heal
/// - **sync-only**: `FaultKind::SyncFail` — writes land, fsync fails (durability lie edge)
#[derive(Debug, Clone)]
pub struct FailingEnv {
    inner: StdEnv,
    state: Rc<FailState>,
}

impl FailingEnv {
    /// Let `n` fallible ops succeed; fail the next and all after (dead disk).
    #[must_use]
    pub fn fail_after(n: u64) -> Self {
        Self::with_state(n, false, FaultKind::IoError)
    }

    /// Like [`fail_after`](Self::fail_after) but with an explicit error kind.
    #[must_use]
    pub fn fail_after_kind(n: u64, kind: FaultKind) -> Self {
        Self::with_state(n, false, kind)
    }

    /// Never injects until [`arm_one_failure`](Self::arm_one_failure) / [`arm`](Self::arm).
    #[must_use]
    pub fn passing() -> Self {
        Self::with_state(u64::MAX, false, FaultKind::IoError)
    }

    fn with_state(remaining: u64, once: bool, kind: FaultKind) -> Self {
        let state = FailState {
            remaining: Cell::new(remaining),
            fired: Cell::new(false),
            once: Cell::new(once),
            kind: Cell::new(kind),
            sync_only: Cell::new(kind.is_sync_only()),
        };
        Self {
            inner: StdEnv,
            state: Rc::new(state),
        }
    }

    /// Arm a single recoverable failure on the next counted op.
    pub fn arm_one_failure(&self) {
        self.arm(0, true);
    }

    /// Runtime arm: `after_ops` further successes, then fail.
    /// `transient` = one-shot (heal after one failure); else permanent dead disk.
    pub fn arm(&self, after_ops: u64, transient: bool) {
        self.state.remaining.set(after_ops);
        self.state.once.set(transient);
        self.state.fired.set(false);
    }

    /// Arm with kind (and sync-only mode if `SyncFail`).
    pub fn arm_with_kind(&self, after_ops: u64, transient: bool, kind: FaultKind) {
        self.state.kind.set(kind);
        self.state.sync_only.set(kind.is_sync_only());
        self.arm(after_ops, transient);
    }

    /// Heal even a permanent fault.
    pub fn disarm(&self) {
        self.state.remaining.set(u64::MAX);
        self.state.once.set(false);
        self.state.fired.set(false);
    }

    /// Whether an injection actually refused an op.
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.state.fired.get()
    }

    /// Current fault kind.
    #[must_use]
    pub fn kind(&self) -> FaultKind {
        self.state.kind.get()
    }

    /// Seedable arm: derive `fail_after(n)` from a `u64` seed (RFC-0011 P1.2).
    ///
    /// Deterministic: same seed → same `n` in `1..=32` (avoids fail_after(0) open
    /// always failing so put/flush paths get exercised in sweeps).
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let n = (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48) % 32 + 1;
        Self::fail_after(n)
    }

    /// Seed + kind.
    #[must_use]
    pub fn from_seed_kind(seed: u64, kind: FaultKind) -> Self {
        let n = (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48) % 32 + 1;
        Self::fail_after_kind(n, kind)
    }

    /// The `n` that [`from_seed`] would use (for harness logs).
    #[must_use]
    pub fn seed_to_fail_after(seed: u64) -> u64 {
        (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48) % 32 + 1
    }
}

/// File handle of [`FailingEnv`].
pub struct FailingFile {
    inner: <StdEnv as Env>::File,
    state: Rc<FailState>,
}

impl FailingFile {
    fn gate(&self, is_sync: bool) -> io::Result<()> {
        self.state.gate(is_sync)
    }
}

impl Read for FailingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.gate(false)?;
        self.inner.read(buf)
    }
}

impl Write for FailingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.gate(false)?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.gate(false)?;
        self.inner.flush()
    }
}

impl Seek for FailingFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // Seek is not a durability barrier; do not count against fail budget.
        self.inner.seek(pos)
    }
}

impl EnvFile for FailingFile {
    fn sync_data(&mut self) -> io::Result<()> {
        self.gate(true)?;
        self.inner.sync_data()
    }

    fn sync_all(&mut self) -> io::Result<()> {
        self.gate(true)?;
        self.inner.sync_all()
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.gate(false)?;
        self.inner.set_len(len)
    }

    fn len(&mut self) -> io::Result<u64> {
        self.gate(false)?;
        self.inner.len()
    }
}

impl Env for FailingEnv {
    type File = FailingFile;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.state.gate(false)?;
        self.inner.create_dir_all(path)
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate(false)?;
        Ok(FailingFile {
            inner: self.inner.create(path)?,
            state: Rc::clone(&self.state),
        })
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate(false)?;
        Ok(FailingFile {
            inner: self.inner.open_append(path)?,
            state: Rc::clone(&self.state),
        })
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate(false)?;
        Ok(FailingFile {
            inner: self.inner.open_read(path)?,
            state: Rc::clone(&self.state),
        })
    }

    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        self.state.gate(true)?;
        self.inner.sync_dir(path)
    }

    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
        self.state.gate(false)?;
        self.inner.read_dir_names(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.state.gate(false)?;
        self.inner.remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.state.gate(false)?;
        self.inner.rename(from, to)
    }

    fn exists(&self, path: &Path) -> bool {
        // Pure metadata; do not inject (matches "list may succeed, write fails").
        self.inner.exists(path)
    }

    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        self.state.gate(false)?;
        self.inner.metadata_len(path)
    }
}
