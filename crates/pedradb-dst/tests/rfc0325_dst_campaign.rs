//! RFC-0325: continuous DST campaign — CI gates.
//!
//! 1. Smoke band: dense clamped scenarios over the REAL engine (full oracle
//!    battery) must stay green.
//! 2. Determinism: same seed ⇒ identical final state (O8).
//! 3. Anti-vacuity (RFC-0270 §4 / RFC-0273 §1): M1 (lost WAL), M2 (bit-rot)
//!    and M3 (wrong model) MUST be caught by the battery — an oracle that
//!    passes a broken engine is vacuous.
//! 4. Ratchet replay: every seed pinned by a campaign run must keep passing
//!    (grow-only; new engine code that breaks an old seed fails CI).

#![forbid(unsafe_code)]

use pedradb_dst::campaign::{
    assert_deterministic_replay, campaign_temp, mutant_bitrot_is_detected,
    mutant_comparator_inversion_is_caught, mutant_fabricated_key_is_caught,
    mutant_fdatasync_bypass_is_caught, mutant_lost_wal_is_caught,
    mutant_tombstone_leak_is_caught, mutant_wal_crc_tamper_is_detected,
    mutant_wrong_model_is_caught, replay_seed, CampaignConfig,
};

/// Same policy as `dst_runner`: injected `FaultKind::Panic` unwinds are crash
/// simulations, not test failures — keep the default noise out.
fn silence_expected_panics() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if !msg.contains("injected panic fault") {
            eprintln!("panic: {msg}");
        }
    }));
}

#[test]
fn rfc0325_smoke_band_full_oracles() {
    silence_expected_panics();
    let parent = campaign_temp("rfc0322-smoke");
    for seed in 0..=8u64 {
        let cfg = CampaignConfig::derive(seed).clamped_for_ci();
        let t = pedradb_dst::campaign::run_trial_cfg(&parent, &cfg);
        if let Some(f) = &t.failure {
            panic!("seed {seed} violated [{}]: {}", f.invariant, f.detail);
        }
    }
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn rfc0325_determinism_double_run() {
    silence_expected_panics();
    let parent = campaign_temp("rfc0322-det");
    for seed in [7u64, 9, 15] {
        let t = assert_deterministic_replay(&parent, seed)
            .unwrap_or_else(|f| panic!("O8 determinism broken on seed {seed}: [{}] {}", f.invariant, f.detail));
        assert!(t.failure.is_none());
    }
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn rfc0325_anti_vacuity_mutants_are_caught() {
    silence_expected_panics();
    let parent = campaign_temp("rfc0322-mutants");

    // M1: a durability-lying engine (lost WAL after acked sync puts) must be
    // flagged by the O1 oracle — otherwise the battery is vacuous.
    let m1 = mutant_lost_wal_is_caught(&parent)
        .expect("M1 fixture");
    assert!(m1, "M1 vacuous: lost acked writes were NOT detected");

    // M2: silent bit-rot in an SST must be flagged by verify_checksums /
    // verify_at_rest — otherwise O6 is vacuous and corruption ships silent.
    let m2 = mutant_bitrot_is_detected(&parent).expect("M2 fixture");
    assert!(m2, "M2 vacuous: flipped SST byte was NOT detected");

    // M3: a wrong model must be flagged by scan_mismatch.
    assert!(mutant_wrong_model_is_caught(), "M3 vacuous: wrong model not caught");

    // M4: WAL record/CRC corruption must be flagged by verify_checksums or open_db.
    let m4 = mutant_wal_crc_tamper_is_detected(&parent).expect("M4 fixture");
    assert!(m4, "M4 vacuous: WAL CRC corruption was NOT detected");

    // M5: Tombstone leak / resurrected key must be flagged by O3.
    assert!(mutant_tombstone_leak_is_caught(), "M5 vacuous: tombstone resurrection not caught");

    // M6 (RFC-0329): Durable barrier omission (fdatasync bypass) must be flagged by O1.
    let m6_sync = mutant_fdatasync_bypass_is_caught(&parent).expect("M6 sync fixture");
    assert!(m6_sync, "M6 vacuous: fdatasync bypass write loss was NOT detected");

    // M6: Comparator inversion / unsorted keys must be flagged.
    assert!(mutant_comparator_inversion_is_caught(), "M6 vacuous: comparator inversion not caught");

    // M7: Fabricated unwritten key must be flagged.
    assert!(mutant_fabricated_key_is_caught(), "M7 vacuous: fabricated key not caught");

    let _ = std::fs::remove_dir_all(&parent);
}


/// Ratchet replay: seeds pinned by past campaign runs keep passing. The file
/// grows monotonically; CI replays up to `PEDRA_DST_RATCHET_REPLAY_MAX`
/// (default 32, clamped to CI caps) so regressions on old schedules fail fast.
#[test]
fn rfc0325_ratchet_replay() {
    silence_expected_panics();
    let ratchet = std::env::var("PEDRA_DST_RATCHET").unwrap_or_else(|_| {
        "crates/pedradb-dst/findings/dst/campaign/ratchet/seeds.jsonl".to_owned()
    });
    let Ok(body) = std::fs::read_to_string(&ratchet) else {
        // No campaign run yet on this checkout — nothing pinned to replay.
        return;
    };
    let max: usize = std::env::var("PEDRA_DST_RATCHET_REPLAY_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let mut seeds: Vec<u64> = Vec::new();
    for line in body.lines() {
        if let Some(at) = line.find("\"seed\":") {
            let rest = &line[at + 7..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            if let Ok(s) = rest[..end].parse::<u64>() {
                seeds.push(s);
            }
        }
    }
    seeds.sort_unstable();
    seeds.dedup();
    let parent = campaign_temp("rfc0322-ratchet");
    for seed in seeds.into_iter().take(max) {
        if let Err(f) = replay_seed(&parent, seed) {
            panic!("ratchet seed {seed} regressed: [{}] {}", f.invariant, f.detail);
        }
    }
    let _ = std::fs::remove_dir_all(&parent);
}

/// F-CAMP-1 regression: a WAL append error mid-fault-window used to leave a
/// torn mid-log region with valid records after it; reopen then hit the F171
/// fail-closed refuse (`WAL resync skipped damaged region mid-log`) and the
/// whole DB became unopenable. Seed 424252 reproduces end-to-end and must
/// pass with the fence fix (fails without it).
#[test]
fn rfc0325_regression_fcamp1_wal_torn_tail_reopens() {
    silence_expected_panics();
    let parent = campaign_temp("rfc0322-fcamp1");
    for seed in [424252u64, 424249] {
        if let Err(f) = replay_seed(&parent, seed) {
            panic!("F-CAMP-1 regression seed {seed}: [{}] {}", f.invariant, f.detail);
        }
    }
    let _ = std::fs::remove_dir_all(&parent);
}

/// F-CAMP-2 oracle regression: a window op whose unwind (FaultKind::Panic)
/// lands AFTER the WAL write has UNKNOWN fate — the battery must mark it
/// uncertain, not report the key as lost. Seeds 778004/778259 tripped the
/// blind spot; they must stay green (and the mutants keep dying).
/// F-CAMP-3 (OPEN): a torn archived WAL segment currently fail-stops the
/// reopen ("torn archived WAL segment … orphan Last fragment") and the DB
/// never opens again. Desired end-state: the rotate path heals the archived
/// copy to the last good offset (nothing after the tear was acked — the
/// fence refused it), so both seeds reopen and pass the full battery.
/// Un-#[ignore] when the fix lands.
#[test]
fn rfc0325_regression_fcamp3_torn_archive_reopens() {
    silence_expected_panics();
    let parent = campaign_temp("rfc0325-fcamp3");
    for seed in [778445u64, 778467] {
        if let Err(f) = replay_seed(&parent, seed) {
            panic!("F-CAMP-3 seed {seed}: [{}] {}", f.invariant, f.detail);
        }
    }
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn rfc0325_regression_oracle_inflight_panic_uncertain() {
    silence_expected_panics();
    let parent = campaign_temp("rfc0325-oracle-inflight");
    for seed in [778004u64, 778259] {
        if let Err(f) = replay_seed(&parent, seed) {
            panic!("oracle regression seed {seed}: [{}] {}", f.invariant, f.detail);
        }
    }
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn rfc0325_scenario_derivation_is_stable_and_ci_capped() {
    for seed in [0u64, 1, 0xDEAD_BEEF, u64::MAX] {
        let a = CampaignConfig::derive(seed);
        let b = CampaignConfig::derive(seed);
        assert_eq!(a, b, "derive must be pure");
        let c = a.clone().clamped_for_ci();
        assert!(c.cycles <= 2 && c.ops_per_cycle <= 384 && c.keyspace <= 256);
        assert!(!a.shape().is_empty());
        assert!(a.fault_density_pct <= 100);
    }
}
