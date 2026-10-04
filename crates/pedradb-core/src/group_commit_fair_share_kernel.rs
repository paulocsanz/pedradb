//! RFC-0290: Group Commit Fair-Share and Bounded Delay Admission Kernel.
//!
//! Enforces bounded latency isolation in group commit:
//! forall w_i: Latency(w_i) <= O(Payload(w_i)) + Delta_max_sync_barrier.
//! Prevents monstrous write batches (e.g. 200 MiB) from hijacking small latency-critical
//! writers, partitioning requests into bounded coalesced groups or isolated dedicated pipelines.

#![forbid(unsafe_code)]

/// Errors resulting from invalid group commit fair-share policies or requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupCommitPolicyError {
    /// Maximum group bytes cannot be zero.
    ZeroMaxGroupBytes,
    /// Huge writer threshold cannot be zero.
    ZeroHugeWriterThreshold,
    /// Huge writer threshold cannot exceed maximum group bytes.
    HugeWriterThresholdExceedsMaxGroup {
        /// Configured huge writer threshold.
        threshold: usize,
        /// Configured maximum group bytes.
        max_group: usize,
    },
    /// Request payload cannot be zero bytes.
    ZeroPayloadBytes(u64),
    /// Writer ID cannot be zero.
    ZeroWriterId,
}

impl std::fmt::Display for GroupCommitPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroMaxGroupBytes => write!(f, "max_group_bytes must be > 0"),
            Self::ZeroHugeWriterThreshold => write!(f, "huge_writer_threshold must be > 0"),
            Self::HugeWriterThresholdExceedsMaxGroup { threshold, max_group } => {
                write!(f, "huge_writer_threshold ({threshold}) cannot exceed max_group_bytes ({max_group})")
            }
            Self::ZeroPayloadBytes(w) => write!(f, "Writer {w} has zero payload bytes"),
            Self::ZeroWriterId => write!(f, "Writer ID cannot be zero"),
        }
    }
}

impl std::error::Error for GroupCommitPolicyError {}

/// An incoming write request to the write admission queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAdmissionRequest {
    /// Identifier of the writing thread or client.
    pub writer_id: u64,
    /// Size of the write payload in bytes.
    pub payload_bytes: usize,
    /// Whether fsync / durability barrier is requested before acknowledgment.
    pub is_sync: bool,
}

impl WriteAdmissionRequest {
    /// Validates and constructs a write admission request.
    pub fn try_new(writer_id: u64, payload_bytes: usize, is_sync: bool) -> Result<Self, GroupCommitPolicyError> {
        if writer_id == 0 {
            return Err(GroupCommitPolicyError::ZeroWriterId);
        }
        if payload_bytes == 0 {
            return Err(GroupCommitPolicyError::ZeroPayloadBytes(writer_id));
        }
        Ok(Self {
            writer_id,
            payload_bytes,
            is_sync,
        })
    }
}

/// Execution classification of an admitted write batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupAdmissionPlan {
    /// Coalesced group of bounded small/medium writers synchronized together.
    CoalescedGroup {
        /// IDs of writers included in this group commit.
        writers: Vec<u64>,
        /// Aggregated byte payload of the group.
        total_bytes: usize,
    },
    /// An isolated write pipeline assigned to a huge write batch to preserve p99 latency.
    IsolatedHugeWriter {
        /// ID of the huge writer.
        writer_id: u64,
        /// Size of the huge payload.
        payload_bytes: usize,
    },
}

/// Fair-share admission policy parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairSharePolicy {
    /// Maximum aggregate bytes allowed in a single coalesced commit group.
    pub max_group_bytes: usize,
    /// Individual request size threshold above which a writer is isolated.
    pub huge_writer_threshold: usize,
}

impl FairSharePolicy {
    /// Validates and constructs a fair-share policy.
    pub fn try_new(max_group_bytes: usize, huge_writer_threshold: usize) -> Result<Self, GroupCommitPolicyError> {
        if max_group_bytes == 0 {
            return Err(GroupCommitPolicyError::ZeroMaxGroupBytes);
        }
        if huge_writer_threshold == 0 {
            return Err(GroupCommitPolicyError::ZeroHugeWriterThreshold);
        }
        if huge_writer_threshold > max_group_bytes {
            return Err(GroupCommitPolicyError::HugeWriterThresholdExceedsMaxGroup {
                threshold: huge_writer_threshold,
                max_group: max_group_bytes,
            });
        }
        Ok(Self {
            max_group_bytes,
            huge_writer_threshold,
        })
    }
}

impl Default for FairSharePolicy {
    fn default() -> Self {
        Self {
            max_group_bytes: 1024 * 1024,      // 1 MiB max per coalesced sync group
            huge_writer_threshold: 256 * 1024, // 256 KiB triggers isolated processing
        }
    }
}

/// Fair-share group commit admission scheduler.
pub struct GroupCommitFairShareScheduler {
    policy: FairSharePolicy,
}

impl GroupCommitFairShareScheduler {
    /// Safely creates a new fair-share scheduler, returning an error on invalid bounds.
    pub fn try_new(policy: FairSharePolicy) -> Result<Self, GroupCommitPolicyError> {
        FairSharePolicy::try_new(policy.max_group_bytes, policy.huge_writer_threshold)?;
        Ok(Self { policy })
    }

    /// Creates a new fair-share scheduler with specific policy bounds.
    #[must_use]
    pub fn new(policy: FairSharePolicy) -> Self {
        Self::try_new(policy).expect("valid fair share policy bounds")
    }

    /// Schedules a sequence of incoming write requests into fair-share execution plans.
    #[must_use]
    pub fn schedule_admissions(&self, requests: &[WriteAdmissionRequest]) -> Vec<GroupAdmissionPlan> {
        let mut plans = Vec::new();
        let mut cur_writers = Vec::new();
        let mut cur_bytes = 0usize;

        for req in requests {
            // Invariant: Huge writers (> huge_writer_threshold) are never packed with small writers
            if req.payload_bytes >= self.policy.huge_writer_threshold {
                // Flush existing pending coalesced group first
                if !cur_writers.is_empty() {
                    plans.push(GroupAdmissionPlan::CoalescedGroup {
                        writers: std::mem::take(&mut cur_writers),
                        total_bytes: cur_bytes,
                    });
                    cur_bytes = 0;
                }

                // Isolate the huge writer
                plans.push(GroupAdmissionPlan::IsolatedHugeWriter {
                    writer_id: req.writer_id,
                    payload_bytes: req.payload_bytes,
                });
                continue;
            }

            // Normal small/medium writer: check if adding it exceeds max_group_bytes
            if cur_bytes + req.payload_bytes > self.policy.max_group_bytes && !cur_writers.is_empty() {
                plans.push(GroupAdmissionPlan::CoalescedGroup {
                    writers: std::mem::take(&mut cur_writers),
                    total_bytes: cur_bytes,
                });
                cur_bytes = 0;
            }

            cur_writers.push(req.writer_id);
            cur_bytes += req.payload_bytes;
        }

        // Flush remaining small writers
        if !cur_writers.is_empty() {
            plans.push(GroupAdmissionPlan::CoalescedGroup {
                writers: cur_writers,
                total_bytes: cur_bytes,
            });
        }

        plans
    }

    /// Safely schedules write admission requests, rejecting invalid writer IDs or zero payloads.
    pub fn try_schedule_admissions(
        &self,
        requests: &[WriteAdmissionRequest],
    ) -> Result<Vec<GroupAdmissionPlan>, GroupCommitPolicyError> {
        for req in requests {
            if req.writer_id == 0 {
                return Err(GroupCommitPolicyError::ZeroWriterId);
            }
            if req.payload_bytes == 0 {
                return Err(GroupCommitPolicyError::ZeroPayloadBytes(req.writer_id));
            }
        }
        Ok(self.schedule_admissions(requests))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_max_group_bytes_rejected() {
        let err = FairSharePolicy::try_new(0, 100).unwrap_err();
        assert_eq!(err, GroupCommitPolicyError::ZeroMaxGroupBytes);
    }

    #[test]
    fn test_threshold_exceeds_max_group_rejected() {
        let err = FairSharePolicy::try_new(100, 200).unwrap_err();
        assert_eq!(
            err,
            GroupCommitPolicyError::HugeWriterThresholdExceedsMaxGroup {
                threshold: 200,
                max_group: 100,
            }
        );
    }

    #[test]
    fn test_write_request_validation() {
        let err_writer = WriteAdmissionRequest::try_new(0, 100, false).unwrap_err();
        assert_eq!(err_writer, GroupCommitPolicyError::ZeroWriterId);

        let err_payload = WriteAdmissionRequest::try_new(1, 0, false).unwrap_err();
        assert_eq!(err_payload, GroupCommitPolicyError::ZeroPayloadBytes(1));
    }

    #[test]
    fn test_try_schedule_admissions_rejects_empty_payload() {
        let scheduler = GroupCommitFairShareScheduler::new(FairSharePolicy::default());
        let requests = [
            WriteAdmissionRequest {
                writer_id: 1,
                payload_bytes: 0,
                is_sync: false,
            },
        ];
        let err = scheduler.try_schedule_admissions(&requests).unwrap_err();
        assert_eq!(err, GroupCommitPolicyError::ZeroPayloadBytes(1));
    }
}

