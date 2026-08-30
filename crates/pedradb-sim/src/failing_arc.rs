//! [`FailingEnvArc`]: `Send + Sync` fault injection via `Arc` (RFC-0011 P2.4).
//!
//! Same fail-after semantics as [`super::FailingEnv`], but shared state uses
//! atomics so the env can cross threads. Prefer the `Rc` variant for single-threaded
//! tests (lighter).

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use pedradb_core::env::{Env, EnvFile, StdEnv};

use super::FaultKind;

#[derive(Debug)]
struct FailStateArc {
    remaining: AtomicU64,
    fired: AtomicBool,
    once: AtomicBool,
    kind: AtomicU64, // packs FaultKind as discriminant
    sync_only: AtomicBool,
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

impl FailStateArc {
    fn gate(&self, is_sync: bool) -> io::Result<()> {
        let kind = kind_from_u64(self.kind.load(Ordering::Relaxed));
        if (kind.is_sync_only() || self.sync_only.load(Ordering::Relaxed)) && !is_sync {
            return Ok(());
        }
        let left = self.remaining.load(Ordering::Relaxed);
        if left == 0 {
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

    /// Whether a fault fired.
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.state.fired.load(Ordering::Relaxed)
    }
}

/// File handle for [`FailingEnvArc`].
pub struct FailingFileArc<E: Env = StdEnv> {
    inner: E::File,
    state: Arc<FailStateArc>,
}

impl<E: Env> Read for FailingFileArc<E> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.state.gate(false)?;
        self.inner.read(buf)
    }
}

impl<E: Env> Write for FailingFileArc<E> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.state.gate(false)?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state.gate(false)?;
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
        self.state.gate(true)?;
        self.inner.sync_data()
    }

    fn sync_all(&mut self) -> io::Result<()> {
        self.state.gate(true)?;
        self.inner.sync_all()
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.state.gate(false)?;
        self.inner.set_len(len)
    }

    fn len(&mut self) -> io::Result<u64> {
        self.state.gate(false)?;
        self.inner.len()
    }
}

impl<E: Env> Env for FailingEnvArc<E> {
    type File = FailingFileArc<E>;

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.state.gate(false)?;
        self.inner.create_dir_all(path)
    }

    fn create(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate(false)?;
        Ok(FailingFileArc {
            inner: self.inner.create(path)?,
            state: Arc::clone(&self.state),
        })
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate(false)?;
        Ok(FailingFileArc {
            inner: self.inner.open_append(path)?,
            state: Arc::clone(&self.state),
        })
    }

    fn open_read(&self, path: &Path) -> io::Result<Self::File> {
        self.state.gate(false)?;
        Ok(FailingFileArc {
            inner: self.inner.open_read(path)?,
            state: Arc::clone(&self.state),
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
        self.inner.exists(path)
    }

    fn metadata_len(&self, path: &Path) -> io::Result<u64> {
        self.state.gate(false)?;
        self.inner.metadata_len(path)
    }

    /// F5: route through the seam so a wrapped non-Std env decides.
    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        self.state.gate(false)?;
        self.inner.is_dir(path)
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
}
