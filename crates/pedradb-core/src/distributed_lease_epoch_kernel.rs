//! kernel: distributed_lease_epoch
//! Distributed leader lease epoch barrier with bounded clock drift guards.
//!
//! Provides mathematically verified lease validity checks preventing split-brain
//! local read admissions under asymmetric network partitions and NTP clock skew.

/// Typed errors produced during distributed leader lease evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseError {
    /// Leader ID must be non-zero.
    ZeroLeaderId,
    /// Term must be non-zero.
    ZeroTerm,
    /// Lease duration must be greater than zero.
    ZeroDuration,
    /// HLC grant timestamp and duration caused integer overflow.
    HlcOverflow,
    /// Renewal attempted with an HLC timestamp strictly lower than current grant HLC.
    HlcRegressed { current: u64, attempted: u64 },
    /// Maximum clock drift cannot exceed or equal total lease duration.
    DriftExceedsDuration { drift: u64, duration: u64 },
    /// Renewal attempted with a term strictly lower than current lease term.
    TermRegressed { current: u64, attempted: u64 },
    /// Lease has already expired at the specified timestamp.
    ExpiredLease,
}

impl core::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroLeaderId => write!(f, "Leader ID must be non-zero"),
            Self::ZeroTerm => write!(f, "Consensus term must be non-zero"),
            Self::ZeroDuration => write!(f, "Lease duration must be greater than zero"),
            Self::HlcOverflow => write!(f, "HLC timestamp + duration caused integer overflow"),
            Self::HlcRegressed { current, attempted } => {
                write!(f, "Grant HLC regressed backwards: current {}, attempted {}", current, attempted)
            }
            Self::DriftExceedsDuration { drift, duration } => {
                write!(f, "Clock drift ({}) exceeds or equals lease duration ({})", drift, duration)
            }
            Self::TermRegressed { current, attempted } => {
                write!(f, "Lease term regressed: current {}, attempted {}", current, attempted)
            }
            Self::ExpiredLease => write!(f, "Lease has expired"),
        }
    }
}

impl std::error::Error for LeaseError {}

/// Represents an active, drift-bounded leader lease epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaderLeaseGuard {
    /// Identifier of the leader holding the lease.
    pub leader_id: u64,
    /// Consensus term for which the lease was granted.
    pub term: u64,
    /// Logical clock timestamp (HLC ticks) at which the lease was granted.
    pub grant_hlc: u64,
    /// Nominal lease duration in logical clock ticks.
    pub duration_ticks: u64,
    /// Maximum upper-bound network/clock drift ticks deducted for safety.
    pub max_drift_ticks: u64,
}

impl LeaderLeaseGuard {
    /// Attempts to establish a new drift-bounded leader lease.
    pub fn try_grant(
        leader_id: u64,
        term: u64,
        grant_hlc: u64,
        duration_ticks: u64,
        max_drift_ticks: u64,
    ) -> Result<Self, LeaseError> {
        if leader_id == 0 {
            return Err(LeaseError::ZeroLeaderId);
        }
        if term == 0 {
            return Err(LeaseError::ZeroTerm);
        }
        if duration_ticks == 0 {
            return Err(LeaseError::ZeroDuration);
        }
        if max_drift_ticks >= duration_ticks {
            return Err(LeaseError::DriftExceedsDuration {
                drift: max_drift_ticks,
                duration: duration_ticks,
            });
        }
        grant_hlc
            .checked_add(duration_ticks)
            .ok_or(LeaseError::HlcOverflow)?;

        let guard = Self {
            leader_id,
            term,
            grant_hlc,
            duration_ticks,
            max_drift_ticks,
        };

        debug_assert!(guard.verify_internal_invariants());
        Ok(guard)
    }

    /// Computes the conservative physical expiration timestamp accounting for maximum drift.
    #[must_use]
    pub fn effective_expiry_hlc(&self) -> u64 {
        let nominal_expiry = self.grant_hlc.saturating_add(self.duration_ticks);
        nominal_expiry.saturating_sub(self.max_drift_ticks)
    }

    /// Returns remaining valid ticks before conservative expiration.
    #[must_use]
    pub fn remaining_ticks(&self, current_hlc: u64) -> u64 {
        let expiry = self.effective_expiry_hlc();
        if current_hlc >= expiry {
            0
        } else {
            expiry - current_hlc
        }
    }

    /// Returns `true` if the lease is currently valid at `current_hlc`.
    #[must_use]
    pub fn is_valid_at(&self, current_hlc: u64) -> bool {
        current_hlc >= self.grant_hlc && current_hlc < self.effective_expiry_hlc()
    }

    /// Evaluates whether a local read operation can be safely served without quorum roundtrip.
    #[must_use]
    pub fn can_admit_local_read(&self, query_term: u64, current_hlc: u64) -> bool {
        self.term == query_term && self.is_valid_at(current_hlc)
    }

    /// Renews the lease for an equal or higher term starting at `new_grant_hlc`.
    /// Rejects regression in term or grant HLC timestamp.
    pub fn renew(&mut self, new_term: u64, new_grant_hlc: u64) -> Result<(), LeaseError> {
        if new_term < self.term {
            return Err(LeaseError::TermRegressed {
                current: self.term,
                attempted: new_term,
            });
        }
        if new_grant_hlc < self.grant_hlc {
            return Err(LeaseError::HlcRegressed {
                current: self.grant_hlc,
                attempted: new_grant_hlc,
            });
        }
        new_grant_hlc
            .checked_add(self.duration_ticks)
            .ok_or(LeaseError::HlcOverflow)?;

        self.term = new_term;
        self.grant_hlc = new_grant_hlc;
        debug_assert!(self.verify_internal_invariants());
        Ok(())
    }

    /// Verifies all mathematical invariants of the lease guard.
    #[must_use]
    pub fn verify_internal_invariants(&self) -> bool {
        if self.leader_id == 0 || self.term == 0 {
            return false;
        }
        if self.duration_ticks == 0 {
            return false;
        }
        if self.max_drift_ticks >= self.duration_ticks {
            return false;
        }
        if self.effective_expiry_hlc() <= self.grant_hlc {
            return false;
        }
        true
    }
}
