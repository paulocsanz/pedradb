//! Seedable deterministic fault schedule (RFC-0012 P1.3).
//!
//! Uses [`pedradb_sim::FailingEnv::from_seed`] so external harnesses (and this
//! crate) share one mapping seed → fail_after(n).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use pedradb_core::{Db, OpenOptions};
use pedradb_sim::FailingEnv;

/// One seed trial outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedTrial {
    /// Seed used.
    pub seed: u64,
    /// Derived fail_after budget.
    pub fail_after: u64,
    /// Whether open under fault succeeded.
    pub open_ok: bool,
    /// Whether seed key survived reopen after faulted put path.
    pub seed_key_ok: bool,
}

/// Run seed `seed` against a fresh dir: open under `from_seed`, put, reopen healthy.
///
/// # Errors
/// Only I/O outside the intentional fault path (reopen after disarm).
pub fn run_seed_trial(parent: impl AsRef<Path>, seed: u64) -> std::io::Result<SeedTrial> {
    let fail_after = FailingEnv::seed_to_fail_after(seed);
    let dir = parent.as_ref().join(format!("seed-{seed}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;

    let opts = OpenOptions {
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        exclusive: true,
    };

    // Durable seed with healthy env first.
    {
        let mut db = Db::open_with(&dir, opts).map_err(std::io::Error::other)?;
        db.put(b"seed", b"ok").map_err(std::io::Error::other)?;
        db.close().map_err(std::io::Error::other)?;
    }

    let env = FailingEnv::from_seed(seed);
    let open_ok = Db::open_with_env(&dir, opts, env.clone()).is_ok();
    if open_ok {
        if let Ok(mut db) = Db::open_with_env(&dir, opts, env.clone()) {
            let _ = db.put(b"extra", b"x");
            drop(db);
        }
    }
    env.disarm();

    let db = Db::open_with_env(&dir, opts, FailingEnv::passing()).map_err(std::io::Error::other)?;
    let seed_key_ok = db.get(b"seed").as_deref() == Some(b"ok".as_ref());
    let _ = db.close();
    let _ = std::fs::remove_dir_all(&dir);

    Ok(SeedTrial {
        seed,
        fail_after,
        open_ok,
        seed_key_ok,
    })
}

/// Sweep seeds `0..count`; every trial must keep the durable seed key.
///
/// # Errors
/// I/O.
pub fn sweep_seeds(parent: impl AsRef<Path>, count: u64) -> std::io::Result<Vec<SeedTrial>> {
    let mut out = Vec::with_capacity(count as usize);
    for seed in 0..count {
        let t = run_seed_trial(parent.as_ref(), seed)?;
        assert!(
            t.seed_key_ok,
            "silent loss on seed {} fail_after={}",
            t.seed, t.fail_after
        );
        out.push(t);
    }
    Ok(out)
}

/// Parent dir helper.
#[must_use]
pub fn temp_parent(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pedradb-dst-{tag}-{n}-{i}"));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_map_stable() {
        assert_eq!(
            FailingEnv::seed_to_fail_after(1),
            FailingEnv::seed_to_fail_after(1)
        );
    }

    #[test]
    fn sweep_no_silent_loss() {
        let parent = temp_parent("sweep");
        let trials = sweep_seeds(&parent, 16).unwrap();
        assert_eq!(trials.len(), 16);
        assert!(trials.iter().all(|t| t.seed_key_ok));
        let _ = std::fs::remove_dir_all(&parent);
    }
}
