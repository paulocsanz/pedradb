//! Exclusive directory lock (RFC-0009 P2.2).
//!
//! Advisory single-process ownership via a `LOCK` file containing the holder's PID.
//!
//! - Same-PID re-open (e.g. crash tests that `mem::forget` a `Db`) steals the lock.
//! - Different live PID → [`CoreError::AlreadyOpen`].
//! - Dead / corrupt PID → treat as stale and take over.
//!
//! Not a kernel `flock`; two racing openers can still collide. Good enough for
//! “don't accidentally open twice” and crash-sim reuse of the same process.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::env::{Env, EnvFile};
use crate::error::{CoreError, Result};

/// Lock file name inside the DB directory.
pub const LOCK_FILE: &str = "LOCK";

/// Held exclusive ownership of a DB directory.
///
/// Prefer [`DirLock::release`] via [`Env`] on the primary shutdown path
/// ([`crate::db::Db::close`] / `Drop`) so unlock is fault-injectable.
/// [`Drop`] remains OS best-effort (`std::fs`) when `release` was not called.
#[derive(Debug)]
pub struct DirLock {
    path: PathBuf,
    pid: u32,
    /// Set after a successful [`Self::release`] so Drop is a no-op.
    released: bool,
}

impl DirLock {
    /// Acquire `dir/LOCK`, stealing if the holder is us or a dead process.
    ///
    /// # Errors
    /// [`CoreError::AlreadyOpen`] if another live process holds the lock; I/O otherwise.
    pub fn acquire<E: Env>(env: &E, dir: &Path) -> Result<Self> {
        let path = dir.join(LOCK_FILE);
        let pid = std::process::id();

        if env.exists(&path) {
            let holder = read_lock_pid(env, &path)?;
            match holder {
                Some(h) if h == pid => {
                    // Same process re-open after leak / forget — steal.
                }
                Some(h) if process_alive(h) => {
                    return Err(CoreError::AlreadyOpen {
                        path: dir.to_path_buf(),
                        holder_pid: Some(h),
                    });
                }
                _ => {
                    // Stale or unreadable — take over.
                }
            }
        }

        write_lock_pid(env, &path, pid)?;
        Ok(Self {
            path,
            pid,
            released: false,
        })
    }

    /// Path of the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether this lock still owns the file (not yet [`Self::release`]d).
    #[must_use]
    pub fn is_held(&self) -> bool {
        !self.released
    }

    /// Release `LOCK` through [`Env`] (primary unlock path; DST-visible).
    ///
    /// Removes the file only if it still contains our PID. Idempotent after success.
    ///
    /// # Errors
    /// I/O from `Env::remove_file` / read.
    pub fn release<E: Env>(&mut self, env: &E) -> Result<()> {
        if self.released {
            return Ok(());
        }
        if env.exists(&self.path) {
            let holder = read_lock_pid(env, &self.path)?;
            if holder.is_none_or(|h| h == self.pid) {
                env.remove_file(&self.path)?;
            }
        }
        self.released = true;
        Ok(())
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        // Best-effort only when primary Env release was not used (RFC-0015 H3).
        if self.released {
            return;
        }
        if let Ok(bytes) = std::fs::read(&self.path) {
            let text = String::from_utf8_lossy(&bytes);
            if text.trim().parse::<u32>().ok() == Some(self.pid) {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

fn read_lock_pid<E: Env>(env: &E, path: &Path) -> Result<Option<u32>> {
    let mut f = env.open_read(path)?;
    let mut buf = String::new();
    match f.read_to_string(&mut buf) {
        Ok(_) => Ok(buf.trim().parse::<u32>().ok()),
        Err(_) => Ok(None),
    }
}

fn write_lock_pid<E: Env>(env: &E, path: &Path, pid: u32) -> Result<()> {
    let mut f = env.create(path)?;
    writeln!(f, "{pid}")?;
    f.sync_all()?;
    Ok(())
}

/// Best-effort liveness probe (`kill -0` on Unix).
///
/// On macOS, probing PID 1 (or other privileged PIDs) as non-root often fails
/// with EPERM — that still means the process **exists**. Dead PIDs yield ESRCH
/// ("No such process"). We inspect stderr so EPERM counts as live.
fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        let output = Command::new("kill").args(["-0", &pid.to_string()]).output();
        match output {
            Ok(o) if o.status.success() => true,
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).to_ascii_lowercase();
                // EPERM / operation not permitted → alive; ESRCH / no such → dead.
                if err.contains("not permitted") || err.contains("operation not permitted") {
                    true
                } else if err.contains("no such process") {
                    false
                } else {
                    // Unknown failure: refuse steal.
                    true
                }
            }
            // If we can't probe, assume live (refuse steal).
            Err(_) => true,
        }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;
    use std::fs;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pedradb-lock-{n}-{i}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn acquire_release_and_reacquire() {
        let dir = temp_dir();
        let env = StdEnv;
        {
            let lock = DirLock::acquire(&env, &dir).unwrap();
            assert!(dir.join(LOCK_FILE).exists());
            assert_eq!(lock.pid, std::process::id());
        }
        assert!(!dir.join(LOCK_FILE).exists());
        let _lock = DirLock::acquire(&env, &dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_pid_steals_after_forget() {
        let dir = temp_dir();
        let env = StdEnv;
        let lock = DirLock::acquire(&env, &dir).unwrap();
        std::mem::forget(lock);
        assert!(dir.join(LOCK_FILE).exists());
        // Re-open in same process must succeed (steal).
        let lock2 = DirLock::acquire(&env, &dir).unwrap();
        drop(lock2);
        assert!(!dir.join(LOCK_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_via_env_clears_lock_and_drop_is_noop() {
        let dir = temp_dir();
        let env = StdEnv;
        let mut lock = DirLock::acquire(&env, &dir).unwrap();
        assert!(dir.join(LOCK_FILE).exists());
        lock.release(&env).unwrap();
        assert!(!dir.join(LOCK_FILE).exists());
        assert!(!lock.is_held());
        // Second release is idempotent; Drop must not error.
        lock.release(&env).unwrap();
        drop(lock);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_live_pid_refuses() {
        let dir = temp_dir();
        let env = StdEnv;
        // Hold the lock with a child process so kill -0 sees a live foreign PID.
        #[cfg(unix)]
        {
            let mut child = Command::new("sleep")
                .arg("30")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn sleep");
            let child_pid = child.id();
            fs::write(dir.join(LOCK_FILE), format!("{child_pid}\n")).unwrap();
            let err = DirLock::acquire(&env, &dir).unwrap_err();
            match err {
                CoreError::AlreadyOpen { holder_pid, .. } => {
                    assert_eq!(holder_pid, Some(child_pid));
                }
                other => {
                    let _ = child.kill();
                    panic!("expected AlreadyOpen, got {other:?}");
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
