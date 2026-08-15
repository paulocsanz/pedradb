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
    host.clock().advance(std::time::Duration::from_millis(
        1 + host.rng().gen_range(49),
    ));

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

/// Machine-readable soak / gate report (RFC-0020 P0.1–P0.2 / P1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeReport {
    /// Gate id (`C0.1` silent_wrong / `C2.1` volume soak).
    pub gate: &'static str,
    /// Number of trials executed.
    pub trials: u64,
    /// Model mismatches / silent losses (must be 0).
    pub silent_wrong: u64,
    /// Successful puts under fault schedules (informational).
    pub puts_ok: u64,
    /// Seed trials that kept durable seed key.
    pub seed_key_ok: u64,
    /// Explore offset applied (`PEDRA_EXPLORE_OFFSET`); 0 when unset.
    pub explore_offset: u64,
    /// Effective RNG / seed base used for this run (proves rounds diverge).
    pub seed_base: u64,
}

impl VolumeReport {
    /// Serialize as compact JSON (no extra deps).
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"gate\": \"{}\",\n  \"trials\": {},\n  \"silent_wrong\": {},\n  \"puts_ok\": {},\n  \"seed_key_ok\": {},\n  \"explore_offset\": {},\n  \"seed_base\": {}\n}}\n",
            self.gate,
            self.trials,
            self.silent_wrong,
            self.puts_ok,
            self.seed_key_ok,
            self.explore_offset,
            self.seed_base
        )
    }

    /// Write JSON to `path`.
    ///
    /// # Errors
    /// I/O.
    pub fn write_to(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, self.to_json())
    }
}

/// Read `PEDRA_EXPLORE_OFFSET` (RFC-0020 P1.3). Default 0.
#[must_use]
pub fn explore_offset_from_env() -> u64 {
    std::env::var("PEDRA_EXPLORE_OFFSET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Sweep seeds `start..start+count` (explore offset support).
///
/// # Errors
/// I/O.
pub fn sweep_seeds_range(
    parent: impl AsRef<Path>,
    start: u64,
    count: u64,
) -> std::io::Result<Vec<SeedTrial>> {
    let mut out = Vec::with_capacity(count as usize);
    for seed in start..start.saturating_add(count) {
        let t = run_seed_trial(parent.as_ref().join(format!("seed-{seed}")), seed)?;
        assert!(
            t.seed_key_ok,
            "silent loss on seed {} fail_after={}",
            t.seed, t.fail_after
        );
        out.push(t);
    }
    Ok(out)
}

/// RFC-0020 P0.1: silent_wrong gate (in-tree).
///
/// Runs a dense seed sweep starting at [`explore_offset_from_env`]; fails closed
/// if any durable seed key is lost.
///
/// # Errors
/// I/O.
pub fn run_silent_wrong_gate(parent: impl AsRef<Path>) -> std::io::Result<VolumeReport> {
    let parent = parent.as_ref();
    let offset = explore_offset_from_env();
    let count = 32u64;
    let trials = sweep_seeds_range(parent.join("gate"), offset, count)?;
    let silent_wrong = trials.iter().filter(|t| !t.seed_key_ok).count() as u64;
    let puts_ok = u64::from(trials.iter().map(|t| t.puts_ok).sum::<u32>());
    let seed_key_ok = trials.iter().filter(|t| t.seed_key_ok).count() as u64;
    let report = VolumeReport {
        gate: "C0.1",
        trials: count,
        silent_wrong,
        puts_ok,
        seed_key_ok,
        explore_offset: offset,
        seed_base: offset,
    };
    if silent_wrong != 0 {
        return Err(std::io::Error::other(format!(
            "silent_wrong={silent_wrong} on gate matrix offset={offset}"
        )));
    }
    Ok(report)
}

/// RFC-0020 P0.2 / P1.6: volume soak with `trials` steps and optional report path.
///
/// - **seed mode** (default for `trials ≤ 256`): each trial is `run_seed_trial`.
/// - **ops mode** (default for larger trials, or `PEDRA_SOAK_MODE=ops`): one DB,
///   `trials` put/get ops vs model — scales to 10k/100k without N directory opens.
///
/// # Errors
/// I/O.
pub fn run_volume_soak(
    parent: impl AsRef<Path>,
    trials: u64,
    report_path: Option<&Path>,
) -> std::io::Result<VolumeReport> {
    let mode = std::env::var("PEDRA_SOAK_MODE").unwrap_or_default();
    let use_ops = mode.eq_ignore_ascii_case("ops")
        || mode.eq_ignore_ascii_case("op")
        || (mode.is_empty() && trials > 256);
    if use_ops {
        run_volume_soak_ops(parent, trials, report_path)
    } else {
        run_volume_soak_seeds(parent, trials, report_path)
    }
}

fn run_volume_soak_seeds(
    parent: impl AsRef<Path>,
    trials: u64,
    report_path: Option<&Path>,
) -> std::io::Result<VolumeReport> {
    let parent = parent.as_ref();
    let offset = explore_offset_from_env();
    let trials = trials.max(1);
    let mut silent_wrong = 0u64;
    let mut puts_ok = 0u64;
    let mut seed_key_ok = 0u64;
    for i in 0..trials {
        let seed = offset.wrapping_add(i).wrapping_mul(0x9E37_79B9);
        let t = run_seed_trial(parent.join(format!("t-{i}")), seed)?;
        if !t.seed_key_ok {
            silent_wrong += 1;
        } else {
            seed_key_ok += 1;
        }
        puts_ok += u64::from(t.puts_ok);
    }
    finish_volume_report(
        "C2.1",
        trials,
        silent_wrong,
        puts_ok,
        seed_key_ok,
        offset,
        offset.wrapping_mul(0x9E37_79B9),
        report_path,
    )
}

/// High-volume ops soak: model vs Db under occasional FailingEnv arms.
///
/// # Errors
/// I/O.
pub fn run_volume_soak_ops(
    parent: impl AsRef<Path>,
    trials: u64,
    report_path: Option<&Path>,
) -> std::io::Result<VolumeReport> {
    use pedradb_core::{Db, DetHost, Rng, SeedRng};
    use std::collections::HashMap;

    let parent = parent.as_ref();
    let dir = parent.join("ops-soak");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let trials = trials.max(1);
    // Ops soak measures logical silent_wrong vs model. Use group-friendly
    // no_sync puts + periodic sync so 10k/100k finish in CI time; crash durability
    // remains covered by seed-mode soaks and lease/index canaries.
    let opts = OpenOptions {
        sync: false,
        auto_flush_bytes: Some(256 * 1024),
        auto_compact_sst_count: Some(8),
        auto_compact_sst_bytes: None,
        exclusive: true,
        large_value_threshold: None,
    };
    let offset = explore_offset_from_env();
    // Mix offset into host + workload RNG so explore rounds produce distinct stats.
    let seed_base = 0x0020_50A1u64
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(offset.wrapping_mul(0x85EB_CA6B));
    let env = FailingEnv::passing();
    let host = DetHost::with_seed(env.clone(), seed_base ^ 0x0020_50AC);
    let mut db = Db::open_with_host(&dir, opts, &host).map_err(std::io::Error::other)?;
    let rng = SeedRng::new(seed_base);
    let mut model: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let mut silent_wrong = 0u64;
    let mut puts_ok = 0u64;
    use pedradb_core::WriteOptions;

    for step in 0..trials {
        if step > 0 && step % 2000 == 0 {
            env.arm(0, true);
        }
        let k = format!("k{:04}", rng.next_u64() % 128);
        let key = k.as_bytes().to_vec();
        let op = rng.next_u64() % 10;
        if op < 7 {
            let val = format!("v{step}").into_bytes();
            match db.put_with(&key, &val, WriteOptions::no_sync()) {
                Ok(_) => {
                    model.insert(key, val);
                    puts_ok += 1;
                }
                Err(_) => env.disarm(),
            }
        } else if op < 9 {
            match db.delete_with(&key, WriteOptions::no_sync()) {
                Ok(_) => {
                    model.remove(&key);
                }
                Err(_) => env.disarm(),
            }
        } else {
            let got = db.get(&key);
            let expect = model.get(&key).map(Vec::as_slice);
            if got.as_deref() != expect {
                silent_wrong += 1;
            }
        }
        if step % 2000 == 1999 {
            env.disarm();
            let _ = db.sync();
        }
    }
    env.disarm();
    let _ = db.sync();
    for (k, v) in &model {
        if db.get(k).as_deref() != Some(v.as_slice()) {
            silent_wrong += 1;
        }
    }
    let seed_key_ok = trials.saturating_sub(silent_wrong);
    let _ = db.close();
    finish_volume_report(
        "C2.1-ops",
        trials,
        silent_wrong,
        puts_ok,
        seed_key_ok,
        offset,
        seed_base,
        report_path,
    )
}

fn finish_volume_report(
    gate: &'static str,
    trials: u64,
    silent_wrong: u64,
    puts_ok: u64,
    seed_key_ok: u64,
    explore_offset: u64,
    seed_base: u64,
    report_path: Option<&Path>,
) -> std::io::Result<VolumeReport> {
    let report = VolumeReport {
        gate,
        trials,
        silent_wrong,
        puts_ok,
        seed_key_ok,
        explore_offset,
        seed_base,
    };
    if let Some(p) = report_path {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        report.write_to(p)?;
    }
    if silent_wrong != 0 {
        return Err(std::io::Error::other(format!(
            "volume soak silent_wrong={silent_wrong} trials={trials}"
        )));
    }
    Ok(report)
}

/// Read `PEDRA_SOAK_TRIALS` (default 1000 for overnight entry).
#[must_use]
pub fn soak_trials_from_env() -> u64 {
    std::env::var("PEDRA_SOAK_TRIALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
        .max(1)
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

    /// RFC-0020 P0.1: fixed matrix gate — silent_wrong=0.
    #[test]
    fn rfc20_silent_wrong_gate_matrix() {
        let parent = temp_parent("rfc20-gate");
        std::env::remove_var("PEDRA_EXPLORE_OFFSET");
        let r = run_silent_wrong_gate(&parent).unwrap();
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        assert!(r.trials >= 16);
        assert_eq!(r.explore_offset, 0);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0020 P1.3: PEDRA_EXPLORE_OFFSET changes seed_base / puts_ok across runs.
    #[test]
    fn rfc20_explore_offset_diverges_volume_reports() {
        let parent = temp_parent("rfc20-explore-div");
        std::env::set_var("PEDRA_SOAK_MODE", "ops");
        std::env::set_var("PEDRA_EXPLORE_OFFSET", "0");
        let a = run_volume_soak(parent.join("a"), 64, None).unwrap();
        std::env::set_var("PEDRA_EXPLORE_OFFSET", "17");
        let b = run_volume_soak(parent.join("b"), 64, None).unwrap();
        std::env::set_var("PEDRA_EXPLORE_OFFSET", "41");
        let c = run_volume_soak(parent.join("c"), 64, None).unwrap();
        assert_eq!(a.explore_offset, 0);
        assert_eq!(b.explore_offset, 17);
        assert_eq!(c.explore_offset, 41);
        assert_ne!(a.seed_base, b.seed_base, "offset must change seed_base");
        assert_ne!(b.seed_base, c.seed_base);
        // Workload stats must not be identical for all three offsets.
        let puts = [a.puts_ok, b.puts_ok, c.puts_ok];
        assert!(
            puts.iter().collect::<std::collections::HashSet<_>>().len() >= 2,
            "puts_ok should diverge across offsets, got {puts:?}"
        );
        assert_eq!(a.silent_wrong + b.silent_wrong + c.silent_wrong, 0);
        std::env::remove_var("PEDRA_EXPLORE_OFFSET");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0020 P0.2: volume soak ≥1000 trials, silent_wrong=0, report file.
    #[test]
    fn rfc20_volume_soak_1k_silent_wrong_zero() {
        let parent = temp_parent("rfc20-soak");
        let report_path = parent.join("volume_report.json");
        // Force ops mode for speed at 1k.
        std::env::set_var("PEDRA_SOAK_MODE", "ops");
        let r = run_volume_soak(&parent, 1000, Some(&report_path)).unwrap();
        assert_eq!(r.trials, 1000);
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        assert!(report_path.is_file());
        let body = std::fs::read_to_string(&report_path).unwrap();
        assert!(body.contains("\"silent_wrong\": 0") || body.contains("\"silent_wrong\":0"));
        assert!(body.contains("1000"));
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// RFC-0020 P1.6: 10k ops soak.
    #[test]
    fn rfc20_volume_soak_10k_ops() {
        let parent = temp_parent("rfc20-10k");
        let report = parent.join("volume_report_10k.json");
        std::env::set_var("PEDRA_SOAK_MODE", "ops");
        let r = run_volume_soak(&parent, 10_000, Some(&report)).unwrap();
        assert_eq!(r.trials, 10_000);
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
