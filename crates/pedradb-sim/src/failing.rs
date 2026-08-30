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
    /// Partial write then error (RFC-0018 short-write class).
    ShortWrite,
    /// `panic!` instead of an Err when the fault fires — models a leader
    /// crash mid-commit (unwind handling contracts, e.g. write-group
    /// leadership release).
    Panic,
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
            Self::ShortWrite => io::Error::new(io::ErrorKind::WriteZero, "injected short write"),
            // Unreachable: `gate_class` panics before calling `to_error`.
            Self::Panic => io::Error::other("injected panic"),
        }
    }

    pub(crate) fn is_sync_only(self) -> bool {
        matches!(self, Self::SyncFail)
    }

    pub(crate) fn is_short_write(self) -> bool {
        matches!(self, Self::ShortWrite)
    }
}

/// Which Env operation class participates in the fail budget (RFC-0018 P0.2).
///
/// Default [`OpClass::Any`] preserves historical fail_after-N (every fallible op).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpClass {
    /// Count / fault any fallible op (legacy behaviour).
    #[default]
    Any,
    /// `EnvFile::write` / `flush` / `set_len` only.
    Write,
    /// `sync_data` / `sync_all` / `sync_dir`.
    Sync,
    /// `Env::rename`.
    Rename,
    /// `create` / `open_append` / `open_read` / `create_dir_all`.
    CreateOpen,
    /// `remove_file`.
    Remove,
    /// `metadata_len` / `read_dir_names`.
    Meta,
}

impl OpClass {
    fn matches(self, op: OpClass) -> bool {
        matches!(self, OpClass::Any) || self == op
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
    /// Only count/fault ops of this class (default Any).
    op_class: Cell<OpClass>,
    /// Bytes allowed on next short-write injection (`None` = full fail, not short).
    short_write_cap: Cell<Option<usize>>,
    /// Logical stall ticks accumulated (tests/World observe; no wall sleep).
    delay_ticks: Cell<u64>,
    /// Stall ticks to add each time a counted op passes while armed.
    delay_per_op: Cell<u64>,
}

impl FailState {
    fn gate_class(&self, op: OpClass) -> io::Result<()> {
        if !self.op_class.get().matches(op) {
            return Ok(());
        }
        let is_sync = matches!(op, OpClass::Sync);
        let kind = self.kind.get();
        if (kind.is_sync_only() || self.sync_only.get()) && !is_sync {
            return Ok(());
        }
        // Short-write is handled in Write::write (partial success then Err).
        if kind.is_short_write() && matches!(op, OpClass::Write) {
            return Ok(());
        }
        let left = self.remaining.get();
        if left == 0 {
            if self.once.get() && self.fired.get() {
                return Ok(());
            }
            self.fired.set(true);
            let d = self.delay_per_op.get();
            if d > 0 {
                self.delay_ticks
                    .set(self.delay_ticks.get().saturating_add(d));
            }
            if matches!(kind, FaultKind::Panic) {
                panic!("injected panic fault (FailingEnv)");
            }
            return Err(kind.to_error());
        }
        // Non-sync-only: every matching op counts. Sync-only: only sync ops count.
        if kind.is_sync_only() || self.sync_only.get() {
            if is_sync {
                self.remaining.set(left - 1);
            }
        } else {
            self.remaining.set(left - 1);
        }
        let d = self.delay_per_op.get();
        if d > 0 {
            self.delay_ticks
                .set(self.delay_ticks.get().saturating_add(d));
        }
        Ok(())
    }
}

/// Test [`Env`]: wraps an inner [`Env`] and injects faults.
///
/// Default inner is [`StdEnv`]. For Linux io_uring stress, wrap an
/// `IoUringEnv` from `pedradb-io-uring` via [`FailingEnv::wrap`].
///
/// Models:
/// - **dead disk**: `fail_after(n)` — N ops succeed, then permanent failure
/// - **one-shot**: `arm_one_failure()` / `arm(n, true)` — single blip then heal
/// - **sync-only**: `FaultKind::SyncFail` — writes land, fsync fails (durability lie edge)
#[derive(Debug, Clone)]
pub struct FailingEnv<E: Env = StdEnv> {
    inner: E,
    state: Rc<FailState>,
}

impl FailingEnv<StdEnv> {
    /// Let `n` fallible ops succeed; fail the next and all after (dead disk).
    #[must_use]
    pub fn fail_after(n: u64) -> Self {
        Self::with_inner(StdEnv, n, false, FaultKind::IoError)
    }

    /// Like [`fail_after`](Self::fail_after) but with an explicit error kind.
    #[must_use]
    pub fn fail_after_kind(n: u64, kind: FaultKind) -> Self {
        Self::with_inner(StdEnv, n, false, kind)
    }

    /// Never injects until [`arm_one_failure`](Self::arm_one_failure) / [`arm`](Self::arm).
    #[must_use]
    pub fn passing() -> Self {
        Self::with_inner(StdEnv, u64::MAX, false, FaultKind::IoError)
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

impl<E: Env> FailingEnv<E> {
    /// Wrap any production/test [`Env`] with a healthy (passing) fault layer.
    #[must_use]
    pub fn wrap(inner: E) -> Self {
        Self::with_inner(inner, u64::MAX, false, FaultKind::IoError)
    }

    /// Wrap `inner` and arm dead-disk after `n` successful ops.
    #[must_use]
    pub fn wrap_fail_after(inner: E, n: u64) -> Self {
        Self::with_inner(inner, n, false, FaultKind::IoError)
    }

    /// Wrap `inner` with an explicit fail-after kind.
    #[must_use]
    pub fn wrap_fail_after_kind(inner: E, n: u64, kind: FaultKind) -> Self {
        Self::with_inner(inner, n, false, kind)
    }

    fn with_inner(inner: E, remaining: u64, once: bool, kind: FaultKind) -> Self {
        let state = FailState {
            remaining: Cell::new(remaining),
            fired: Cell::new(false),
            once: Cell::new(once),
            kind: Cell::new(kind),
            sync_only: Cell::new(kind.is_sync_only()),
            op_class: Cell::new(OpClass::Any),
            short_write_cap: Cell::new(None),
            delay_ticks: Cell::new(0),
            delay_per_op: Cell::new(0),
        };
        Self {
            inner,
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
        if kind.is_short_write() && self.state.short_write_cap.get().is_none() {
            self.state.short_write_cap.set(Some(1));
        }
        self.arm(after_ops, transient);
    }

    /// Restrict budget to one op class (RFC-0018). Does not change remaining.
    pub fn set_op_class(&self, class: OpClass) {
        self.state.op_class.set(class);
        if matches!(class, OpClass::Sync) {
            self.state.sync_only.set(true);
        }
    }

    /// Arm fault on a specific op class after `after_ops` matching successes.
    pub fn arm_op_class(&self, class: OpClass, after_ops: u64, transient: bool, kind: FaultKind) {
        self.set_op_class(class);
        self.arm_with_kind(after_ops, transient, kind);
    }

    /// Next matching write returns at most `n` bytes then errors (ShortWrite).
    pub fn arm_short_write(&self, n: usize) {
        self.state.kind.set(FaultKind::ShortWrite);
        self.state.sync_only.set(false);
        self.state.op_class.set(OpClass::Write);
        self.state.short_write_cap.set(Some(n));
        self.state.remaining.set(0);
        self.state.once.set(true);
        self.state.fired.set(false);
    }

    /// Add `ticks` logical stall on each counted op (no wall sleep).
    pub fn set_delay_per_op(&self, ticks: u64) {
        self.state.delay_per_op.set(ticks);
    }

    /// Accumulated logical stall ticks since construct/disarm.
    #[must_use]
    pub fn delay_ticks(&self) -> u64 {
        self.state.delay_ticks.get()
    }

    /// Heal even a permanent fault.
    pub fn disarm(&self) {
        self.state.remaining.set(u64::MAX);
        self.state.once.set(false);
        self.state.fired.set(false);
        self.state.op_class.set(OpClass::Any);
        self.state.sync_only.set(false);
        self.state.short_write_cap.set(None);
        self.state.delay_per_op.set(0);
        self.state.delay_ticks.set(0);
    }

    /// Inner env (crash image, production FS, …).
    #[must_use]
    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Whether an injection actually refused an op (or short-wrote).
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.state.fired.get()
    }

    /// Current fault kind.
    #[must_use]
    pub fn kind(&self) -> FaultKind {
        self.state.kind.get()
    }

    /// Current op-class filter.
    #[must_use]
    pub fn op_class(&self) -> OpClass {
        self.state.op_class.get()
    }
}

/// File handle of [`FailingEnv`].
pub struct FailingFile<F: EnvFile> {
    inner: F,
    state: Rc<FailState>,
}

impl<F: EnvFile> FailingFile<F> {
    fn gate(&self, class: OpClass) -> io::Result<()> {
        self.state.gate_class(class)
    }
}

impl<F: EnvFile> Read for FailingFile<F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Reads count as Meta-class only when filter is Any or Meta.
        self.gate(OpClass::Meta)?;
        self.inner.read(buf)
    }
}

impl<F: EnvFile> Write for FailingFile<F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Short-write injection: partial success then error once.
        if self.state.kind.get().is_short_write()
            && self.state.op_class.get().matches(OpClass::Write)
        {
            let left = self.state.remaining.get();
            if left == 0 {
                if self.state.once.get() && self.state.fired.get() {
                    return self.inner.write(buf);
                }
                if let Some(cap) = self.state.short_write_cap.get() {
                    self.state.fired.set(true);
                    self.state.short_write_cap.set(None);
                    if self.state.once.get() {
                        self.state.remaining.set(u64::MAX);
                    }
                    let n = cap.min(buf.len());
                    if n == 0 {
                        return Err(FaultKind::ShortWrite.to_error());
                    }
                    let wrote = self.inner.write(&buf[..n])?;
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        format!("injected short write after {wrote} bytes"),
                    ));
                }
            } else {
                self.state.remaining.set(left - 1);
            }
        }
        self.gate(OpClass::Write)?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.gate(OpClass::Write)?;
        self.inner.flush()
    }
}

impl<F: EnvFile> Seek for FailingFile<F> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // Seek is not a durability barrier; do not count against fail budget.
        self.inner.seek(pos)
    }
}

impl<F: EnvFile> EnvFile for FailingFile<F> {
    fn sync_data(&mut self) -> io::Result<()> {
        self.gate(OpClass::Sync)?;
        self.inner.sync_data()
    }

    /// Same `OpClass::Sync` fault seam as [`Self::sync_data`] — the strong
    /// class is a different syscall, not a different failure policy.
    fn sync_data_strong(&mut self) -> io::Result<()> {
        self.gate(OpClass::Sync)?;
        self.inner.sync_data_strong()
    }

    fn sync_all(&mut self) -> io::Result<()> {
        self.gate(OpClass::Sync)?;
        self.inner.sync_all()
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.gate(OpClass::Write)?;
        self.inner.set_len(len)
    }

    fn len(&mut self) -> io::Result<u64> {
        self.gate(OpClass::Meta)?;
        self.inner.len()
    }
}

impl<E: Env> Env for FailingEnv<E> {
    type File = FailingFile<E::File>;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.state.gate_class(OpClass::CreateOpen)?;
        self.inner.create_dir_all(path)
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate_class(OpClass::CreateOpen)?;
        Ok(FailingFile {
            inner: self.inner.create(path)?,
            state: Rc::clone(&self.state),
        })
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate_class(OpClass::CreateOpen)?;
        Ok(FailingFile {
            inner: self.inner.open_append(path)?,
            state: Rc::clone(&self.state),
        })
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate_class(OpClass::CreateOpen)?;
        Ok(FailingFile {
            inner: self.inner.open_read(path)?,
            state: Rc::clone(&self.state),
        })
    }

    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        self.state.gate_class(OpClass::Sync)?;
        self.inner.sync_dir(path)
    }

    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<String>> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.read_dir_names(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.state.gate_class(OpClass::Remove)?;
        self.inner.remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.state.gate_class(OpClass::Rename)?;
        self.inner.rename(from, to)
    }

    fn exists(&self, path: &Path) -> bool {
        // Pure metadata; do not inject (matches "list may succeed, write fails").
        self.inner.exists(path)
    }

    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.metadata_len(path)
    }

    /// F5: route through the seam so a wrapped non-Std env decides, and the
    /// Meta fault class can inject here like any other metadata op.
    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.is_dir(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "failing-rfc18-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn failing_op_class_write_and_short_write() {
        let dir = tmp();
        let p = dir.join("f");
        let env = FailingEnv::passing();
        {
            let mut f = env.create(&p).unwrap();
            f.write_all(b"hello").unwrap();
            f.sync_all().unwrap();
        }
        env.arm_short_write(2);
        let mut f = env.open_append(&p).unwrap();
        let r = f.write(b"abcdef");
        assert!(r.is_err(), "short write must error");
        assert!(env.tripped());
        env.disarm();
        env.set_delay_per_op(3);
        env.arm_op_class(OpClass::Write, 0, true, FaultKind::IoError);
        let mut f = env.open_append(&p).unwrap();
        assert!(f.write_all(b"x").is_err());
        assert!(env.delay_ticks() >= 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_op_class_rename() {
        let dir = tmp();
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::write(&a, b"1").unwrap();
        let env = FailingEnv::passing();
        env.arm_op_class(OpClass::Rename, 0, true, FaultKind::IoError);
        assert!(env.rename(&a, &b).is_err());
        assert!(env.tripped());
        // create/open still works under Rename filter
        env.disarm();
        env.arm_op_class(OpClass::Rename, 0, true, FaultKind::IoError);
        assert!(env.create(&dir.join("c")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_op_class_create_open() {
        let dir = tmp();
        let env = FailingEnv::passing();
        env.arm_op_class(OpClass::CreateOpen, 0, true, FaultKind::PermissionDenied);
        assert!(env.create(&dir.join("x")).is_err());
        assert!(env.tripped());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_op_class_remove() {
        let dir = tmp();
        let p = dir.join("r");
        std::fs::write(&p, b"z").unwrap();
        let env = FailingEnv::passing();
        env.arm_op_class(OpClass::Remove, 0, true, FaultKind::IoError);
        assert!(env.remove_file(&p).is_err());
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_op_class_meta() {
        let dir = tmp();
        let p = dir.join("m");
        std::fs::write(&p, b"zz").unwrap();
        let env = FailingEnv::passing();
        env.arm_op_class(OpClass::Meta, 0, true, FaultKind::IoError);
        assert!(env.metadata_len(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fail_after_legacy_still_trips() {
        let dir = tmp();
        let env = FailingEnv::fail_after(1);
        let p = dir.join("l");
        // first create succeeds, second fails (remaining 1 -> 0 after first)
        assert!(env.create(&p).is_ok());
        assert!(env.create(&dir.join("l2")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
