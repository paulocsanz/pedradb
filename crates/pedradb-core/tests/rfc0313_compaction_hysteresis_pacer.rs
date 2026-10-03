//! RFC-0313: Compaction Anti-Thrashing Hysteresis Pacer Test Suite.
//!
//! Validates mathematical properties:
//! - Strict deadband invariance (no premature state flips).
//! - Finite work transition bound (anti-flapping under noisy write load).
//! - Multilevel energy aggregation and L0 penalty monotonicity.

use pedradb_core::compaction_hysteresis_pacer_kernel::{
    CompactionHysteresisController, CompactionLevelMetrics, CompactionState, HysteresisConfig,
    HysteresisConfigError,
};

#[test]
fn test_hysteresis_configuration_validation() {
    // 1. Zero lower threshold is rejected
    let bad_config1 = HysteresisConfig {
        theta_low: 0,
        theta_high: 1000,
        ..Default::default()
    };
    assert_eq!(
        CompactionHysteresisController::new(bad_config1).unwrap_err(),
        HysteresisConfigError::ZeroThreshold
    );

    // 2. theta_low >= theta_high is rejected
    let bad_config2 = HysteresisConfig {
        theta_low: 1500,
        theta_high: 1500,
        ..Default::default()
    };
    assert_eq!(
        CompactionHysteresisController::new(bad_config2).unwrap_err(),
        HysteresisConfigError::InvalidThresholds {
            theta_low: 1500,
            theta_high: 1500,
        }
    );

    // 3. Valid default config
    let controller = CompactionHysteresisController::new(HysteresisConfig::default()).unwrap();
    assert_eq!(controller.state(), CompactionState::Idle);
    assert_eq!(controller.deadband_width(), 500); // 1500 - 1000
    assert_eq!(controller.total_transitions(), 0);
}

#[test]
fn test_hysteresis_deadband_invariance() {
    let config = HysteresisConfig {
        theta_low: 1000,
        theta_high: 2000,
        l0_target_files: 4,
        l0_file_weight: 100,
        level_weight: 1000,
    };
    let mut controller = CompactionHysteresisController::new(config).unwrap();

    // Deadband is (1000, 2000)
    assert!(controller.is_in_deadband(1500));
    assert!(!controller.is_in_deadband(1000));
    assert!(!controller.is_in_deadband(2000));

    // Case 1: Starting at Idle, stay inside deadband (e.g. energy = 1500)
    // 4 target files + 15 excess files * 100 = 1500 energy
    let (state1, changed1) = controller.update(19, &[]);
    assert_eq!(state1, CompactionState::Idle);
    assert!(!changed1);
    assert_eq!(controller.current_energy(), 1500);

    // Case 2: Exceed upper threshold (energy = 2100) -> Transitions to Active
    // 4 + 21 = 25 files -> 21 * 100 = 2100
    let (state2, changed2) = controller.update(25, &[]);
    assert_eq!(state2, CompactionState::Active);
    assert!(changed2);
    assert_eq!(controller.total_transitions(), 1);

    // Case 3: Drop back into deadband (energy = 1500) -> MUST STAY ACTIVE!
    let (state3, changed3) = controller.update(19, &[]);
    assert_eq!(state3, CompactionState::Active);
    assert!(!changed3, "In deadband, state must be preserved as Active");

    // Case 4: Drop below lower threshold (energy = 900) -> Transitions back to Idle
    // 4 + 9 = 13 files -> 9 * 100 = 900
    let (state4, changed4) = controller.update(13, &[]);
    assert_eq!(state4, CompactionState::Idle);
    assert!(changed4);
    assert_eq!(controller.total_transitions(), 2);
}

#[test]
fn test_hysteresis_anti_flapping_noisy_walk() {
    let config = HysteresisConfig {
        theta_low: 1000,
        theta_high: 1600,
        l0_target_files: 0,
        l0_file_weight: 10,
        level_weight: 1000,
    };
    let mut controller = CompactionHysteresisController::new(config).unwrap();

    // Simulate noisy oscillating L0 file counts between 110 and 150 (energy 1100 to 1500).
    // All values reside entirely within the deadband (1000, 1600).
    // A naive trigger oscillating at 1300 would flip hundreds of times.
    // The Schmitt trigger MUST experience EXACTLY ZERO transitions!
    for step in 0..500 {
        let count = if step % 2 == 0 { 110 } else { 150 };
        let (state, changed) = controller.update(count, &[]);
        assert_eq!(state, CompactionState::Idle);
        assert!(!changed, "Should never flip inside the deadband");
    }
    assert_eq!(controller.total_transitions(), 0);

    // Now push it above 1600 (count = 170 -> 1700 energy)
    let (state_act, changed_act) = controller.update(170, &[]);
    assert_eq!(state_act, CompactionState::Active);
    assert!(changed_act);
    assert_eq!(controller.total_transitions(), 1);

    // Now oscillate again in the deadband: MUST STAY ACTIVE with ZERO transitions!
    for step in 0..500 {
        let count = if step % 2 == 0 { 110 } else { 150 };
        let (state, changed) = controller.update(count, &[]);
        assert_eq!(state, CompactionState::Active);
        assert!(!changed, "Should remain Active inside deadband");
    }
    assert_eq!(controller.total_transitions(), 1);
}

#[test]
fn test_multilevel_energy_calculation() {
    let controller = CompactionHysteresisController::new(HysteresisConfig::default()).unwrap();

    let levels = vec![
        CompactionLevelMetrics {
            level: 1,
            file_count: 10,
            total_bytes: 10_000_000,
            target_bytes: 10_000_000, // 1.0x capacity = 1000 permille
        },
        CompactionLevelMetrics {
            level: 2,
            file_count: 20,
            total_bytes: 140_000_000,
            target_bytes: 100_000_000, // 1.4x capacity = 1400 permille (maximum)
        },
    ];

    // L0 file count = 6 (target = 4, excess = 2 * 250 = 500 permille)
    // Max level debt = 1400 permille
    // Total energy = 500 + 1400 = 1900 permille
    let energy = controller.calculate_energy(6, &levels);
    assert_eq!(energy, 1900);
}
