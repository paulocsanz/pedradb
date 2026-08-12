//! Seedable deterministic fault schedule (RFC-0012 P1.3 + Host seams).
//!
//! Plugs the full DST bundle:
//! - disk: [`FailingEnv::from_seed`]
//! - time / entropy: [`DetHost`] (`ManualClock` + `SeedRng`)
//! - open path: [`Db::open_with_host`]
//!
//! External harnesses in `determinismo/` should prefer the same mapping so a
//! seed is portable across in-tree smoke and out-of-tree campaigns.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use pedradb_core::{Db, DetHost, Host, OpenOptions, Rng};
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
    /// How many host-driven put ops completed Ok under the faulted host.
    pub puts_ok: u32,
    /// Last RNG state observed (for repro logs).
    pub rng_state: u64,
}

fn durable_opts() -> OpenOptions {
    OpenOptions {
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        auto_compact_sst_bytes: None,
        exclusive: true,
                large_value_threshold: None,
    }
}

/// Build the same [`DetHost`] a trial uses for seed `seed`.
#[must_use]
pub fn host_for_seed(seed: u64) -> DetHost<FailingEnv> {
    DetHost::with_seed(FailingEnv::from_seed(seed), seed)
}

/// Run seed `seed` against a fresh dir via [`Db::open_with_host`].
///
/// # Errors
/// Only I/O outside the intentional fault path (reopen after disarm).
pub fn run_seed_trial(parent: impl AsRef<Path>, seed: u64) -> std::io::Result<SeedTrial> {
    let fail_after = FailingEnv::seed_to_fail_after(seed);
    let dir = parent.as_ref().join(format!("seed-{seed}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;

    let opts = durable_opts();

    // Durable seed with healthy host first (DetHost + passing env).
    {
        let healthy = DetHost::with_seed(FailingEnv::passing(), seed);
        let mut db = Db::open_with_host(&dir, opts, &healthy).map_err(std::io::Error::other)?;
        db.put(b"seed", b"ok").map_err(std::io::Error::other)?;
        db.close().map_err(std::io::Error::other)?;
    }

    let host = host_for_seed(seed);
    // Advance logical time once so clock seam is exercised (kernel ignores it;
    // layers share the same host shape).
    host.clock()
        .advance(std::time::Duration::from_millis(1 + host.rng().gen_range(49)));

    let open_ok = Db::open_with_host(&dir, opts, &host).is_ok();
    let mut puts_ok = 0u32;
    if open_ok {
        if let Ok(mut db) = Db::open_with_host(&dir, opts, &host) {
            // Seed-driven workload: number and keys from the host RNG stream.
            let n = 1 + (host.rng().gen_range(4) as u32);
            for i in 0..n {
                let k = format!("k-{}", host.rng().next_u64());
                let v = [i as u8, (seed & 0xff) as u8];
                if db.put(k.as_bytes(), v).is_ok() {
                    puts_ok += 1;
                } else {
                    break;
                }
            }
            drop(db);
        }
    }
    host.env().disarm();
    let rng_state = host.seed_rng().state();

    let healthy = DetHost::with_seed(FailingEnv::passing(), seed ^ 1);
    let db = Db::open_with_host(&dir, opts, &healthy).map_err(std::io::Error::other)?;
    let seed_key_ok = db.get(b"seed").as_deref() == Some(b"ok".as_ref());
    let _ = db.close();
    let _ = std::fs::remove_dir_all(&dir);

    Ok(SeedTrial {
        seed,
        fail_after,
        open_ok,
        seed_key_ok,
        puts_ok,
        rng_state,
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

/// Same seed ⇒ identical trial fields (open_ok / puts_ok / rng_state).
///
/// # Errors
/// I/O.
pub fn assert_seed_replayable(parent: impl AsRef<Path>, seed: u64) -> std::io::Result<()> {
    let a = run_seed_trial(parent.as_ref().join("a"), seed)?;
    let b = run_seed_trial(parent.as_ref().join("b"), seed)?;
    assert_eq!(a.fail_after, b.fail_after);
    assert_eq!(a.open_ok, b.open_ok);
    assert_eq!(a.puts_ok, b.puts_ok);
    assert_eq!(a.rng_state, b.rng_state);
    assert!(a.seed_key_ok && b.seed_key_ok);
    Ok(())
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
    fn host_for_seed_is_deterministic() {
        let h1 = host_for_seed(7);
        let h2 = host_for_seed(7);
        assert_eq!(h1.rng().next_u64(), h2.rng().next_u64());
        // Host is built from FailingEnv::from_seed(seed); mapping is stable.
        assert_eq!(
            FailingEnv::seed_to_fail_after(7),
            FailingEnv::seed_to_fail_after(7)
        );
        assert_eq!(h1.seed_rng().state(), h2.seed_rng().state());
    }

    #[test]
    fn sweep_no_silent_loss() {
        let parent = temp_parent("sweep");
        let trials = sweep_seeds(&parent, 16).unwrap();
        assert_eq!(trials.len(), 16);
        assert!(trials.iter().all(|t| t.seed_key_ok));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn seed_trial_replayable() {
        let parent = temp_parent("replay");
        assert_seed_replayable(&parent, 42).unwrap();
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Denser multi-seed: durable seed key never silently lost under FailingEnv host.
    #[test]
    fn denser_sweep_silent_wrong_zero() {
        let parent = temp_parent("dense");
        let trials = sweep_seeds(&parent, 48).unwrap();
        assert_eq!(trials.len(), 48);
        let silent = trials.iter().filter(|t| !t.seed_key_ok).count();
        assert_eq!(silent, 0, "silent_wrong must be 0 across denser seed sweep");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
