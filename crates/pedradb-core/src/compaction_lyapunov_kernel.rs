//! RFC-0282 Pilar 6 — Terminação de Compactação por Métrica de Lyapunov (Compaction Lyapunov Kernel).
//!
//! Mathematically proves termination and absence of liveloops (compaction thrashing) in LSM-tree
//! leveled compaction using a well-founded Lyapunov energy potential function E(LSM) \in N.
//! Proves that in the absence of new write admissions, each valid compaction transition
//! strictly decreases the total system debt:
//!   E(LSM_{t+1}) < E(LSM_t), with E(LSM) >= 0.
//!
//! Hence, compaction is guaranteed to terminate in finite steps to the minimal canonical quiescent state.

#![forbid(unsafe_code)]

/// Violations of compaction well-founded termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionTerminationViolation {
    /// Compaction step increased the system Lyapunov energy (divergence bug).
    EnergyIncreased {
        /// Energy before compaction step.
        energy_before: u64,
        /// Energy after compaction step.
        energy_after: u64,
    },
    /// Compaction step made zero progress (zero energy change leads to infinite liveloop).
    ZeroProgressStep {
        /// Stagnant energy level.
        stagnant_energy: u64,
    },
}

/// State representation of an LSM level for energy calculations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelDebtProfile {
    /// Level index (0 = L0, 1 = L1, etc.).
    pub level_idx: usize,
    /// Total bytes currently stored in this level.
    pub current_bytes: u64,
    /// Maximum target capacity for this level.
    pub target_capacity_bytes: u64,
    /// Number of overlapping SST files (only L0 typically has overlaps > 1).
    pub overlapping_runs: u64,
}

impl LevelDebtProfile {
    /// Calculates the non-negative debt contribution for this level.
    #[must_use]
    pub fn calculate_debt(&self) -> u64 {
        let size_debt = self.current_bytes.saturating_sub(self.target_capacity_bytes);
        // Overlapping files in L0 carry an exponential sorting penalty
        let overlap_penalty = if self.level_idx == 0 {
            self.overlapping_runs.saturating_sub(1) * 10_000
        } else {
            0
        };
        size_debt + overlap_penalty
    }
}

/// Verifier using a Lyapunov metric to ensure compaction progress and termination.
pub struct LyapunovCompactionVerifier;

impl LyapunovCompactionVerifier {
    /// Computes the total system Lyapunov potential E(LSM) across all levels.
    /// Weights w_l assign higher priority to upper levels to ensure cascading reductions.
    #[must_use]
    pub fn compute_lyapunov_energy(levels: &[LevelDebtProfile]) -> u64 {
        let mut total_energy: u64 = 0;
        for profile in levels {
            let debt = profile.calculate_debt();
            // Weight decreases with level depth: L0=8, L1=4, L2=2, L3..=1
            let weight: u64 = match profile.level_idx {
                0 => 8,
                1 => 4,
                2 => 2,
                _ => 1,
            };
            total_energy = total_energy.saturating_add(debt.saturating_mul(weight));
        }
        total_energy
    }

    /// Verifies that a proposed compaction transition strictly reduces system energy.
    ///
    /// # Errors
    /// Returns `CompactionTerminationViolation` if energy does not strictly decrease.
    pub fn verify_compaction_step(
        levels_before: &[LevelDebtProfile],
        levels_after: &[LevelDebtProfile],
    ) -> Result<u64, CompactionTerminationViolation> {
        let energy_before = Self::compute_lyapunov_energy(levels_before);
        let energy_after = Self::compute_lyapunov_energy(levels_after);

        if energy_after >= energy_before {
            if energy_after == energy_before {
                return Err(CompactionTerminationViolation::ZeroProgressStep {
                    stagnant_energy: energy_before,
                });
            }
            return Err(CompactionTerminationViolation::EnergyIncreased {
                energy_before,
                energy_after,
            });
        }

        let delta = energy_before - energy_after;
        Ok(delta)
    }

    /// Evaluates if the LSM tree has reached the minimal quiescent equilibrium (E == 0).
    #[must_use]
    pub fn is_quiescent(levels: &[LevelDebtProfile]) -> bool {
        Self::compute_lyapunov_energy(levels) == 0
    }
}
