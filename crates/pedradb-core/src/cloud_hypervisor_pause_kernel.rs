//! kernel: cloud_hypervisor_pause
//! Cloud VM hypervisor pause and vCPU preemption detection kernel.
//!
//! Evaluates physical monotonic clock jumps relative to cooperative scheduler progression,
//! immediately auto-quarantining local leader leases when live migration or vCPU steal
//! events exceed linearizability safety thresholds.

/// Typed errors produced during hypervisor pause evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypervisorPauseError {
    /// Maximum tolerated pause must be strictly greater than zero.
    ZeroTolerance,
    /// Physical monotonic clock regressed backwards in time.
    MonotonicClockRegression { last_ts_ns: u64, attempted_ts_ns: u64 },
}

impl core::fmt::Display for HypervisorPauseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroTolerance => write!(f, "Max tolerated pause threshold cannot be zero"),
            Self::MonotonicClockRegression { last_ts_ns, attempted_ts_ns } => {
                write!(
                    f,
                    "Monotonic clock regressed: last {} ns, attempted {} ns",
                    last_ts_ns, attempted_ts_ns
                )
            }
        }
    }
}

impl std::error::Error for HypervisorPauseError {}

/// Outcome of a scheduler step observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseObservation {
    /// Normal execution progression within expected cloud scheduling bounds.
    NormalProgress { elapsed_ns: u64 },
    /// An unexpected discontinuity in physical time occurred without proportional scheduler steps.
    HypervisorPauseDetected { pause_ns: u64, tolerated_threshold_ns: u64 },
}

/// Autonomic state tracker for detecting cloud hypervisor stalls and vCPU preemption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypervisorPauseDetector {
    /// Last sampled physical monotonic timestamp in nanoseconds.
    last_monotonic_ts_ns: u64,
    /// Maximum physical time gap tolerated without proportional scheduler progress.
    max_tolerated_pause_ns: u64,
    /// Flag indicating whether the node is currently in self-quarantine due to a detected pause.
    is_quarantined: bool,
    /// Cumulative count of pause events detected across process lifetime.
    total_pauses_detected: u64,
}

impl HypervisorPauseDetector {
    /// Creates a new detector initialized at `initial_ts_ns` with `max_tolerated_pause_ns`.
    pub fn new(initial_ts_ns: u64, max_tolerated_pause_ns: u64) -> Result<Self, HypervisorPauseError> {
        if max_tolerated_pause_ns == 0 {
            return Err(HypervisorPauseError::ZeroTolerance);
        }

        Ok(Self {
            last_monotonic_ts_ns: initial_ts_ns,
            max_tolerated_pause_ns,
            is_quarantined: false,
            total_pauses_detected: 0,
        })
    }

    /// Observes scheduler loop progress against physical clock advancement.
    ///
    /// If wall time advanced significantly beyond `max_tolerated_pause_ns` without sufficient
    /// cooperative scheduler ticks, triggers immediate lease quarantine.
    pub fn observe_step(
        &mut self,
        current_ts_ns: u64,
        scheduler_ticks_elapsed: u64,
    ) -> Result<PauseObservation, HypervisorPauseError> {
        if current_ts_ns < self.last_monotonic_ts_ns {
            return Err(HypervisorPauseError::MonotonicClockRegression {
                last_ts_ns: self.last_monotonic_ts_ns,
                attempted_ts_ns: current_ts_ns,
            });
        }

        let elapsed_ns = current_ts_ns.saturating_sub(self.last_monotonic_ts_ns);
        self.last_monotonic_ts_ns = current_ts_ns;

        // If elapsed time exceeded threshold and scheduler progress was starved (ticks == 0 or disproportionately low)
        let is_stalled = elapsed_ns > self.max_tolerated_pause_ns && scheduler_ticks_elapsed <= 1;

        if is_stalled {
            self.is_quarantined = true;
            self.total_pauses_detected = self.total_pauses_detected.saturating_add(1);
            debug_assert!(self.verify_internal_invariants());
            Ok(PauseObservation::HypervisorPauseDetected {
                pause_ns: elapsed_ns,
                tolerated_threshold_ns: self.max_tolerated_pause_ns,
            })
        } else {
            debug_assert!(self.verify_internal_invariants());
            Ok(PauseObservation::NormalProgress { elapsed_ns })
        }
    }

    /// Returns `true` if currently in quarantine.
    #[must_use]
    pub fn is_quarantined(&self) -> bool {
        self.is_quarantined
    }

    /// Clears quarantine status after explicit consensus re-validation (e.g. successful Raft heartbeat round).
    pub fn clear_quarantine_after_sync(&mut self, current_ts_ns: u64) {
        self.is_quarantined = false;
        self.last_monotonic_ts_ns = current_ts_ns;
        debug_assert!(self.verify_internal_invariants());
    }

    /// Total count of pauses detected.
    #[must_use]
    pub fn total_pauses_detected(&self) -> u64 {
        self.total_pauses_detected
    }

    /// Verifies mathematical consistency of the detector state.
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        self.max_tolerated_pause_ns > 0
    }

    /// Autonomically clears quarantine status after a sustained sequence of healthy scheduler ticks.
    pub fn clear_quarantine_after_healthy_ticks(&mut self, healthy_ticks: u64, min_required: u64) -> bool {
        if self.is_quarantined && min_required > 0 && healthy_ticks >= min_required {
            self.is_quarantined = false;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_hypervisor_pause_structural_invariants_red_to_green() {
        // Invariant 1: Zero tolerance threshold rejected fail-closed
        assert_eq!(
            HypervisorPauseDetector::new(100, 0),
            Err(HypervisorPauseError::ZeroTolerance)
        );

        // Invariant 2: Monotonic clock regression rejected
        let mut detector = HypervisorPauseDetector::new(1_000_000, 50_000_000).unwrap();
        assert_eq!(
            detector.observe_step(500_000, 1),
            Err(HypervisorPauseError::MonotonicClockRegression {
                last_ts_ns: 1_000_000,
                attempted_ts_ns: 500_000,
            })
        );

        // Invariant 3: Autonomic quarantine on preemption jump
        let pause = detector.observe_step(100_000_000, 0).unwrap();
        assert!(matches!(pause, PauseObservation::HypervisorPauseDetected { .. }));
        assert!(detector.is_quarantined());

        // Invariant 4: Autonomic self-reconciliation after sufficient healthy ticks
        assert!(!detector.clear_quarantine_after_healthy_ticks(5, 10)); // insufficient
        assert!(detector.is_quarantined());
        assert!(detector.clear_quarantine_after_healthy_ticks(10, 10)); // sufficient
        assert!(!detector.is_quarantined());
    }
}

