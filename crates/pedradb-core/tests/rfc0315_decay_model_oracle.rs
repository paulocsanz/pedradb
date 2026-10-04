//! RFC-0315: Deterministic Decay Model Oracle Test Suite
//!
//! Verifies:
//! - Least squares regression for alpha parameter fit.
//! - Contamination detection on measurement noise > 15%.
//! - Alpha ceiling enforcement per phase kind.
//! - Allocs-per-operation ceiling enforcement.

use pedradb_core::decay_model_kernel::{
    DecayMeasurement, DecayModelError, DecayModelEvaluator, DecayVerdict, PhaseKind,
};

#[test]
fn test_least_squares_alpha_fitting() {
    // True relationship: cost = 10.0 * n^0.10
    // Scales: 200_000, 2_000_000, 20_000_000
    let n1 = 200_000u64;
    let n2 = 2_000_000u64;
    let n3 = 20_000_000u64;

    let c1 = 10.0 * (n1 as f64).powf(0.10);
    let c2 = 10.0 * (n2 as f64).powf(0.10);
    let c3 = 10.0 * (n3 as f64).powf(0.10);

    let measurements = vec![
        DecayMeasurement::new(n1, vec![c1 * 0.99, c1 * 1.01], 0.0),
        DecayMeasurement::new(n2, vec![c2 * 0.99, c2 * 1.01], 0.0),
        DecayMeasurement::new(n3, vec![c3 * 0.99, c3 * 1.01], 0.0),
    ];

    let alpha = DecayModelEvaluator::fit_alpha(&measurements).unwrap();
    assert!((alpha - 0.10).abs() < 0.01, "Fitted alpha was {}", alpha);
}

#[test]
fn test_contamination_rejection() {
    let n1 = 200_000u64;
    let n2 = 2_000_000u64;

    // Measurement at n1 has high spread: min 10.0, max 15.0 -> spread 5.0 / 12.5 = 40% > 15%
    let measurements = vec![
        DecayMeasurement::new(n1, vec![10.0, 15.0], 0.0),
        DecayMeasurement::new(n2, vec![20.0, 20.5], 0.0),
    ];

    let verdict = DecayModelEvaluator::evaluate_phase(PhaseKind::MissInNs, &measurements, 0.15);
    match verdict {
        DecayVerdict::Contaminated { scale_n, spread } => {
            assert_eq!(scale_n, n1);
            assert!(spread > 0.15);
        }
        other => panic!("Expected Contaminated verdict, got {:?}", other),
    }
}

#[test]
fn test_alpha_ceiling_enforcement() {
    let n1 = 1000u64;
    let n2 = 10000u64;

    // Steep increase: alpha = 0.50 (violates MissOutNs ceiling of 0.20)
    let c1 = 10.0 * (n1 as f64).powf(0.50);
    let c2 = 10.0 * (n2 as f64).powf(0.50);

    let measurements = vec![
        DecayMeasurement::new(n1, vec![c1], 0.0),
        DecayMeasurement::new(n2, vec![c2], 0.0),
    ];

    let verdict = DecayModelEvaluator::evaluate_phase(PhaseKind::MissOutNs, &measurements, 0.15);
    match verdict {
        DecayVerdict::FailAlpha { alpha, ceiling } => {
            assert!((alpha - 0.50).abs() < 0.05);
            assert_eq!(ceiling, 0.20);
        }
        other => panic!("Expected FailAlpha verdict, got {:?}", other),
    }
}

#[test]
fn test_alloc_budget_enforcement() {
    let n1 = 1000u64;
    let n2 = 10000u64;

    // MissOutNs requires max_allocs == 0.0, but measurement reports 0.8 allocs/op
    let measurements = vec![
        DecayMeasurement::new(n1, vec![100.0], 0.8),
        DecayMeasurement::new(n2, vec![105.0], 0.8),
    ];

    let verdict = DecayModelEvaluator::evaluate_phase(PhaseKind::MissOutNs, &measurements, 0.15);
    match verdict {
        DecayVerdict::FailAllocs { allocs, ceiling } => {
            assert_eq!(allocs, 0.8);
            assert_eq!(ceiling, 0.0);
        }
        other => panic!("Expected FailAllocs verdict, got {:?}", other),
    }
}

#[test]
fn test_decay_model_validation_and_prediction_green() {
    // 1. DecayMeasurement::try_new validation
    assert_eq!(
        DecayMeasurement::try_new(0, vec![10.0], 0.0).err(),
        Some(DecayModelError::ZeroScaleN)
    );
    assert_eq!(
        DecayMeasurement::try_new(100, vec![], 0.0).err(),
        Some(DecayModelError::EmptySamples)
    );
    assert_eq!(
        DecayMeasurement::try_new(100, vec![f64::NAN], 0.0).err(),
        Some(DecayModelError::InvalidSample)
    );
    assert_eq!(
        DecayMeasurement::try_new(100, vec![-1.0], 0.0).err(),
        Some(DecayModelError::InvalidSample)
    );
    assert_eq!(
        DecayMeasurement::try_new(100, vec![10.0], -0.5).err(),
        Some(DecayModelError::InvalidAllocs)
    );
    assert_eq!(
        DecayMeasurement::try_new(100, vec![10.0], f64::NAN).err(),
        Some(DecayModelError::InvalidAllocs)
    );

    // 2. fit_model computes scaling constant c, alpha, and R^2
    let n1 = 200_000u64;
    let n2 = 2_000_000u64;
    let n3 = 20_000_000u64;
    let c_true = 10.0f64;
    let alpha_true = 0.10f64;

    let measurements = vec![
        DecayMeasurement::try_new(n1, vec![c_true * (n1 as f64).powf(alpha_true)], 0.0).unwrap(),
        DecayMeasurement::try_new(n2, vec![c_true * (n2 as f64).powf(alpha_true)], 0.0).unwrap(),
        DecayMeasurement::try_new(n3, vec![c_true * (n3 as f64).powf(alpha_true)], 0.0).unwrap(),
    ];

    let model = DecayModelEvaluator::fit_model(&measurements).expect("model fit");
    assert!((model.alpha - 0.10).abs() < 0.001);
    assert!((model.c - 10.0).abs() < 0.05);
    assert!(model.r_squared > 0.999);

    // Extrapolate to 200M
    let n_target = 200_000_000u64;
    let predicted = model.predict_cost(n_target);
    let expected = c_true * (n_target as f64).powf(alpha_true);
    assert!((predicted - expected).abs() < 0.1);
}

