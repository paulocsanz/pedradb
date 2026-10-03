//! [`FailingEnvArc`]: `Send + Sync` fault injection via `Arc` (RFC-0011 P2.4 / RFC-0333).
//!
//! Multi-thread safe fault injection covering the full 42-cell fault alphabet matrix
//! (7 FaultKind x 6 OpClass) with atomic short-write byte capping. Shared state uses
//! atomics so the env can cross threads safely without deadlocks.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use pedradb_core::env::{Env, EnvFile, StdEnv};

use super::{FaultKind, OpClass};

#[derive(Debug)]
struct FailStateArc {
    remaining: AtomicU64,
    fired: AtomicBool,
    once: AtomicBool,
    kind: AtomicU64, // packs FaultKind as discriminant
    sync_only: AtomicBool,
    op_class: AtomicU64, // packs OpClass as discriminant
    short_write_cap: AtomicU64, // u64::MAX = None
    /// RFC-0179: `available_bytes` override is live.
    space_injected: AtomicBool,
    /// Free bytes; `u64::MAX` means unknown (`None`) while injected.
    available: AtomicU64,
    /// RFC-0179: `available_bytes` returns Err (failed probe).
    probe_err: AtomicBool,
}

fn kind_to_u64(k: FaultKind) -> u64 {
    match k {
        FaultKind::IoError => 0,
        FaultKind::StorageFull => 1,
        FaultKind::PermissionDenied => 2,
        FaultKind::Interrupted => 3,
        FaultKind::SyncFail => 4,
        FaultKind::ShortWrite => 5,
        FaultKind::Panic => 6,
    }
}

fn kind_from_u64(v: u64) -> FaultKind {
    match v {
        1 => FaultKind::StorageFull,
        2 => FaultKind::PermissionDenied,
        3 => FaultKind::Interrupted,
        4 => FaultKind::SyncFail,
        5 => FaultKind::ShortWrite,
        6 => FaultKind::Panic,
        _ => FaultKind::IoError,
    }
}

fn op_class_to_u64(op: OpClass) -> u64 {
    match op {
        OpClass::Any => 0,
        OpClass::Write => 1,
        OpClass::Sync => 2,
        OpClass::Rename => 3,
        OpClass::CreateOpen => 4,
        OpClass::Remove => 5,
        OpClass::Meta => 6,
    }
}

fn op_class_from_u64(v: u64) -> OpClass {
    match v {
        1 => OpClass::Write,
        2 => OpClass::Sync,
        3 => OpClass::Rename,
        4 => OpClass::CreateOpen,
        5 => OpClass::Remove,
        6 => OpClass::Meta,
        _ => OpClass::Any,
    }
}

impl FailStateArc {
    fn gate_class(&self, op: OpClass) -> io::Result<()> {
        let op_filter = op_class_from_u64(self.op_class.load(Ordering::Relaxed));
        if !op_filter.matches(op) {
            return Ok(());
        }
        let is_sync = matches!(op, OpClass::Sync);
        let kind = kind_from_u64(self.kind.load(Ordering::Relaxed));
        if (kind.is_sync_only() || self.sync_only.load(Ordering::Relaxed)) && !is_sync {
            return Ok(());
        }
        // Short-write is handled directly in Write::write.
        if kind.is_short_write() && matches!(op, OpClass::Write) {
            return Ok(());
        }
        let left = self.remaining.load(Ordering::Relaxed);
        if pedradb_core::write_admission_kernel::batch_is_empty(left) {
            if self.once.load(Ordering::Relaxed) && self.fired.load(Ordering::Relaxed) {
                return Ok(());
            }
            self.fired.store(true, Ordering::Relaxed);
            if matches!(kind, FaultKind::Panic) {
                panic!("injected panic fault (FailingEnvArc)");
            }
            return Err(kind.to_error());
        }
        if kind.is_sync_only() || self.sync_only.load(Ordering::Relaxed) {
            if is_sync {
                self.remaining.fetch_sub(1, Ordering::Relaxed);
            }
        } else {
            self.remaining.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

/// Thread-safe [`FailingEnv`] (`Send + Sync` when cloned across threads).
/// Generic over the wrapped env since RFC-0051 P2.1 (`FailingEnvArc<IoUringEnv>`
/// Linux trial); `StdEnv` stays the default.
#[derive(Debug, Clone)]
pub struct FailingEnvArc<E: Env = StdEnv> {
    inner: E,
    state: Arc<FailStateArc>,
}

impl FailingEnvArc<StdEnv> {
    /// Permanent fail after `n` ops.
    #[must_use]
    pub fn fail_after(n: u64) -> Self {
        Self::with_state(n, false, FaultKind::IoError)
    }

    /// Passing until armed.
    #[must_use]
    pub fn passing() -> Self {
        Self::with_state(u64::MAX, false, FaultKind::IoError)
    }

    fn with_state(remaining: u64, once: bool, kind: FaultKind) -> Self {
        Self {
            inner: StdEnv,
            state: Arc::new(FailStateArc {
                remaining: AtomicU64::new(remaining),
                fired: AtomicBool::new(false),
                once: AtomicBool::new(once),
                kind: AtomicU64::new(kind_to_u64(kind)),
                sync_only: AtomicBool::new(kind.is_sync_only()),
                op_class: AtomicU64::new(op_class_to_u64(OpClass::Any)),
                short_write_cap: AtomicU64::new(u64::MAX),
                space_injected: AtomicBool::new(false),
                available: AtomicU64::new(u64::MAX),
                probe_err: AtomicBool::new(false),
            }),
        }
    }
}

impl<E: Env> FailingEnvArc<E> {
    /// Passing until armed, over a caller-supplied inner env
    /// (e.g. `IoUringEnv` on Linux).
    #[must_use]
    pub fn with_inner_passing(inner: E) -> Self {
        Self {
            inner,
            state: Arc::new(FailStateArc {
                remaining: AtomicU64::new(u64::MAX),
                fired: AtomicBool::new(false),
                once: AtomicBool::new(false),
                kind: AtomicU64::new(kind_to_u64(FaultKind::IoError)),
                sync_only: AtomicBool::new(false),
                op_class: AtomicU64::new(op_class_to_u64(OpClass::Any)),
                short_write_cap: AtomicU64::new(u64::MAX),
                space_injected: AtomicBool::new(false),
                available: AtomicU64::new(u64::MAX),
                probe_err: AtomicBool::new(false),
            }),
        }
    }

    /// Arm one-shot failure.
    pub fn arm_one_failure(&self) {
        self.arm(0, true);
    }

    /// Runtime arm.
    pub fn arm(&self, after_ops: u64, transient: bool) {
        self.state.remaining.store(after_ops, Ordering::Relaxed);
        self.state.once.store(transient, Ordering::Relaxed);
        self.state.fired.store(false, Ordering::Relaxed);
    }

    /// Runtime arm with an explicit fault kind (e.g. [`FaultKind::Panic`]
    /// to model a mid-commit crash).
    pub fn arm_with_kind(&self, after_ops: u64, transient: bool, kind: FaultKind) {
        self.state.kind.store(kind_to_u64(kind), Ordering::Relaxed);
        self.state
            .sync_only
            .store(kind.is_sync_only(), Ordering::Relaxed);
        self.arm(after_ops, transient);
    }

    /// Arm with explicit FaultKind and OpClass for targeted fault grid testing.
    pub fn arm_with_class(
        &self,
        after_ops: u64,
        transient: bool,
        kind: FaultKind,
        op_class: OpClass,
    ) {
        self.state
            .op_class
            .store(op_class_to_u64(op_class), Ordering::Relaxed);
        self.arm_with_kind(after_ops, transient, kind);
    }

    /// Arm short-write failure with maximum byte cap before failing.
    pub fn arm_short_write(&self, after_ops: u64, cap_bytes: usize) {
        self.state
            .short_write_cap
            .store(cap_bytes as u64, Ordering::Relaxed);
        self.state
            .op_class
            .store(op_class_to_u64(OpClass::Write), Ordering::Relaxed);
        self.arm_with_kind(after_ops, true, FaultKind::ShortWrite);
    }

    /// Whether a fault fired.
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.state.fired.load(Ordering::Relaxed)
    }

    /// RFC-0179: inject `Env::available_bytes` (`None` = unknown).
    pub fn set_available_bytes(&self, n: Option<u64>) {
        self.state.probe_err.store(false, Ordering::Relaxed);
        self.state.space_injected.store(true, Ordering::Relaxed);
        self.state
            .available
            .store(n.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// RFC-0179: inject `available_bytes` Err (failed probe). Glue maps
    /// Err → unknown; must not false-refuse.
    pub fn inject_probe_err(&self) {
        self.state.probe_err.store(true, Ordering::Relaxed);
    }
}

/// File handle for [`FailingEnvArc`].
pub struct FailingFileArc<E: Env = StdEnv> {
    inner: E::File,
    state: Arc<FailStateArc>,
}

impl<E: Env> Read for FailingFileArc<E> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.read(buf)
    }
}

impl<E: Env> Write for FailingFileArc<E> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let op_filter = op_class_from_u64(self.state.op_class.load(Ordering::Relaxed));
        let kind = kind_from_u64(self.state.kind.load(Ordering::Relaxed));
        if kind.is_short_write() && op_filter.matches(OpClass::Write) {
            let left = self.state.remaining.load(Ordering::Relaxed);
            if pedradb_core::write_admission_kernel::batch_is_empty(left) {
                if self.state.once.load(Ordering::Relaxed) && self.state.fired.load(Ordering::Relaxed) {
                    return self.inner.write(buf);
                }
                let cap = self.state.short_write_cap.swap(u64::MAX, Ordering::Relaxed);
                if cap != u64::MAX {
                    self.state.fired.store(true, Ordering::Relaxed);
                    if self.state.once.load(Ordering::Relaxed) {
                        self.state.remaining.store(u64::MAX, Ordering::Relaxed);
                    }
                    let n = (cap as usize).min(buf.len());
                    if pedradb_core::write_admission_kernel::batch_is_empty(n as u64) {
                        return Err(FaultKind::ShortWrite.to_error());
                    }
                    let wrote = self.inner.write(&buf[..n])?;
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        format!("injected short write after {wrote} bytes"),
                    ));
                }
            } else {
                self.state.remaining.fetch_sub(1, Ordering::Relaxed);
            }
        }
        self.state.gate_class(OpClass::Write)?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state.gate_class(OpClass::Write)?;
        self.inner.flush()
    }
}

impl<E: Env> Seek for FailingFileArc<E> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl<E: Env> EnvFile for FailingFileArc<E> {
    fn sync_data(&mut self) -> io::Result<()> {
        self.state.gate_class(OpClass::Sync)?;
        self.inner.sync_data()
    }

    fn sync_all(&mut self) -> io::Result<()> {
        self.state.gate_class(OpClass::Sync)?;
        self.inner.sync_all()
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.state.gate_class(OpClass::Write)?;
        self.inner.set_len(len)
    }

    fn len(&mut self) -> io::Result<u64> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.len()
    }
}

impl<E: Env> Env for FailingEnvArc<E> {
    type File = FailingFileArc<E>;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.state.gate_class(OpClass::CreateOpen)?;
        self.inner.create_dir_all(path)
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate_class(OpClass::CreateOpen)?;
        Ok(FailingFileArc {
            inner: self.inner.create(path)?,
            state: Arc::clone(&self.state),
        })
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate_class(OpClass::CreateOpen)?;
        Ok(FailingFileArc {
            inner: self.inner.open_append(path)?,
            state: Arc::clone(&self.state),
        })
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate_class(OpClass::CreateOpen)?;
        Ok(FailingFileArc {
            inner: self.inner.open_read(path)?,
            state: Arc::clone(&self.state),
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
        self.inner.exists(path)
    }

    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.metadata_len(path)
    }

    /// F5: route through the seam so a wrapped non-Std env decides.
    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        self.state.gate_class(OpClass::Meta)?;
        self.inner.is_dir(path)
    }

    fn available_bytes(&self, path: &Path) -> io::Result<Option<u64>> {
        if self.state.probe_err.load(Ordering::Relaxed) {
            return Err(io::Error::other("injected statvfs failure"));
        }
        if self.state.space_injected.load(Ordering::Relaxed) {
            let n = self.state.available.load(Ordering::Relaxed);
            if n == u64::MAX {
                Ok(None)
            } else {
                Ok(Some(n))
            }
        } else {
            self.inner.available_bytes(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FailingEnvArc>();
    }

    #[test]
    fn fail_after_trips() {
        let env = FailingEnvArc::fail_after(0);
        assert!(env.create_dir_all(Path::new("/tmp/nope-arc-fail")).is_err());
        assert!(env.tripped());
    }

    #[test]
    fn op_class_selective_filtering() {
        let env = FailingEnvArc::passing();
        env.arm_with_class(0, true, FaultKind::SyncFail, OpClass::Sync);
        let file_path = std::env::temp_dir().join(format!("pedra_test_failing_arc_{}.txt", std::process::id()));
        let _ = env.remove_file(&file_path);
        // Create should succeed because filter is OpClass::Sync
        let mut f = env.create(&file_path).unwrap();
        // Write should succeed
        assert!(f.write_all(b"hello").is_ok());
        // Sync should trip
        assert!(f.sync_all().is_err());
        assert!(env.tripped());
        let _ = env.remove_file(&file_path);
    }
}
