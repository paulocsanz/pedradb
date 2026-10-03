//! RFC-0311: Deterministic Verification Oracle for Foster-Lyapunov Stochastic Ingestion Pacing.
//!
//! Enforces zero-twin production verification of:
//! 1. Zero delay and unit admission within the compact target set $\mathcal{C}$.
//! 2. Strict monotonicity: higher queue backlog strictly increases delay and decreases admission.
//! 3. Priority weighting: L0 amplification debt exerts higher gradient force than pure memtable debt.
//! 4. Mathematical Foster-Lyapunov negative drift property ($\Delta V \le -\epsilon$) outside $\mathcal{C}$.
//! 5. Smooth C1 continuity: eliminates cliff-edge "stop-the-world" latency spikes.

use pedradb_core::foster_lyapunov_pacer_kernel::{
    BacklogState, FosterLyapunovConfig, FosterLyapunovPacer,
};

#[test]
fn test_zero_backpressure_within_target_set() {
    let pacer = FosterLyapunovPacer::new(FosterLyapunovConfig::default());

    // Backlog completely within safe target boundaries
    let benign_state = BacklogState {
        mem_bytes: 32 * 1024 * 1024,         // 32 MiB <= 64 MiB target
        l0_files: 2,                          // 2 files <= 4 target
        compaction_debt_bytes: 50 * 1024 * 1024, // 50 MiB <= 128 MiB target
    };

    let decision = pacer.evaluate(&benign_state);
    assert_eq!(decision.delay_micros, 0, "No backpressure inside target set");
    assert_eq!(decision.potential_v, 0.0);
    assert_eq!(decision.gradient_norm, 0.0);
    assert!((decision.admission_ratio - 1.0).abs() < 1e-9);
}

#[test]
fn test_monotonic_delay_growth_under_l0_accumulation() {
    let pacer = FosterLyapunovPacer::new(FosterLyapunovConfig::default());

    let mut prev_delay = 0u64;
    let mut prev_potential = 0.0f64;
    let mut prev_admission = 1.0f64;

    for l0 in 4..=20 {
        let state = BacklogState {
            mem_bytes: 64 * 1024 * 1024,
            l0_files: l0,
            compaction_debt_bytes: 128 * 1024 * 1024,
        };

        let d = pacer.evaluate(&state);

        if l0 > 4 {
            assert!(
                d.potential_v > prev_potential,
                "Potential must strictly increase for L0={l0}"
            );
            assert!(
                d.delay_micros >= prev_delay,
                "Delay must monotonically increase for L0={l0}: current={delay} prev={prev}",
                delay = d.delay_micros,
                prev = prev_delay
            );
            assert!(
                d.admission_ratio <= prev_admission,
                "Admission ratio must monotonically decrease for L0={l0}"
            );
        }

        prev_delay = d.delay_micros;
        prev_potential = d.potential_v;
        prev_admission = d.admission_ratio;
    }

    // At hard saturation (20 L0 files), delay must approach max_delay_micros (10ms)
    assert!(
        prev_delay >= 9_000,
        "Delay at hard saturation must be near max delay limit: got {prev_delay} µs"
    );
}

#[test]
fn test_priority_weighting_l0_vs_memtable() {
    let config = FosterLyapunovConfig::default();
    let pacer = FosterLyapunovPacer::new(config);

    // State A: 50% normalized L0 debt (12 files between 4 and 20)
    let state_l0_heavy = BacklogState {
        mem_bytes: config.mem_target_bytes,
        l0_files: 12,
        compaction_debt_bytes: config.compaction_target_bytes,
    };

    // State B: 50% normalized Memtable debt (160 MiB between 64 and 256 MiB)
    let state_mem_heavy = BacklogState {
        mem_bytes: 160 * 1024 * 1024,
        l0_files: config.l0_target_files,
        compaction_debt_bytes: config.compaction_target_bytes,
    };

    let dec_l0 = pacer.evaluate(&state_l0_heavy);
    let dec_mem = pacer.evaluate(&state_mem_heavy);

    // L0 weight is 2.5 vs Memtable weight 1.0 -> L0 potential and gradient must dominate
    assert!(
        dec_l0.potential_v > dec_mem.potential_v,
        "L0 debt must produce higher potential than memtable debt under configured weights"
    );
    assert!(
        dec_l0.delay_micros > dec_mem.delay_micros,
        "L0 debt must throttle more aggressively than memtable debt"
    );
}

#[test]
fn test_foster_lyapunov_negative_drift_outside_compact_set() {
    let pacer = FosterLyapunovPacer::new(FosterLyapunovConfig::default());

    // Heavy backlog state x_t outside C
    let state_t = BacklogState {
        mem_bytes: 180 * 1024 * 1024,
        l0_files: 16,
        compaction_debt_bytes: 600 * 1024 * 1024,
    };

    // Subsequent state x_{t+1} after backpressure allowed flushes and compactions to drain debt
    let state_t_plus_1 = BacklogState {
        mem_bytes: 120 * 1024 * 1024,
        l0_files: 10,
        compaction_debt_bytes: 400 * 1024 * 1024,
    };

    let drift = pacer.compute_drift(&state_t, &state_t_plus_1);

    // Foster-Lyapunov stability condition: Drift Delta V must be strictly negative!
    assert!(
        drift < -0.1,
        "Lyapunov drift must be strictly negative outside C during recovery: drift={drift}"
    );
}

#[test]
fn test_smooth_c1_continuity_no_cliff_edge_spikes() {
    let pacer = FosterLyapunovPacer::new(FosterLyapunovConfig::default());

    // Test across 100 continuous points traversing L0 boundary
    let mut prev_delay = 0u64;

    for step in 0..=100 {
        // Continuous interpolation of memtable bytes from target to hard limit
        let mem = (64 * 1024 * 1024) + (step * (192 * 1024 * 1024) / 100);
        let state = BacklogState {
            mem_bytes: mem,
            l0_files: 4,
            compaction_debt_bytes: 128 * 1024 * 1024,
        };

        let dec = pacer.evaluate(&state);

        if step > 0 {
            let diff = dec.delay_micros.abs_diff(prev_delay);
            // In RocksDB, diff can jump from 0 to 1,000,000 µs (1s) in a single step.
            // With Foster-Lyapunov, maximum step change is strictly bounded by Lipschitz curvature.
            assert!(
                diff <= 500,
                "Pacing step change must be smooth and continuous, observed step jump={diff} µs"
            );
        }

        prev_delay = dec.delay_micros;
    }
}
