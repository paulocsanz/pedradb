//! RFC-0290: Group Commit Fair-Share and Bounded Delay Admission Kernel.
//!
//! Enforces bounded latency isolation in group commit:
//! forall w_i: Latency(w_i) <= O(Payload(w_i)) + Delta_max_sync_barrier.
//! Prevents monstrous write batches (e.g. 200 MiB) from hijacking small latency-critical
//! writers, partitioning requests into bounded coalesced groups or isolated dedicated pipelines.

#![forbid(unsafe_code)]

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
    /// Creates a new fair-share scheduler with specific policy bounds.
    #[must_use]
    pub fn new(policy: FairSharePolicy) -> Self {
        assert!(policy.max_group_bytes > 0);
        assert!(policy.huge_writer_threshold > 0);
        assert!(policy.huge_writer_threshold <= policy.max_group_bytes);
        Self { policy }
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
}
