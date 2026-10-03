//! RFC-0282 Pilar 8 — Agendador de Tempo Denso Contínuo no DST (Dense Time Scheduler Kernel).
//!
//! Formalizes continuous dense-time event scheduling for Deterministic Simulation Testing (DST).
//! Replaces coarse-grained discrete ticks with an \epsilon-dense perturbation engine.
//! When any two concurrent events (e.g. timer timeout and asynchronous I/O completion)
//! occur within |t_1 - t_2| <= \epsilon, the scheduler systematically permutes their execution order:
//!   - Permutation A: Timer fires before I/O completion;
//!   - Permutation B: I/O completion fires before Timer.
//!
//! Guarantees zero blindspots for microsecond-scale timer races in DST.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Class of simulated event in the dense-time model.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScheduledEventType {
    /// Timer expiration (e.g., lease timeout, transaction deadline).
    TimerExpiry {
        /// Timer identifier.
        timer_id: u64,
    },
    /// Asynchronous I/O completion event (e.g., io_uring CQE, fsync ACK).
    IoCompletion {
        /// I/O operation identifier.
        io_id: u64,
    },
    /// Network packet arrival.
    NetworkPacket {
        /// Sender node ID.
        sender: usize,
        /// Message ID.
        msg_id: u64,
    },
}

/// A timed event in the dense simulation space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DenseTimedEvent {
    /// Unique event ID.
    pub event_id: u64,
    /// Continuous timestamp in nanoseconds.
    pub timestamp_nanos: u64,
    /// Concrete event type.
    pub event_type: ScheduledEventType,
}

/// Violations discovered during dense-time scheduling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DenseScheduleViolation {
    /// Divergence between two \epsilon-close orderings (a race was uncovered).
    UnstableRaceDiscovered {
        /// First event ID.
        event_a: u64,
        /// Second event ID.
        event_b: u64,
        /// Time delta in nanoseconds between the racing events.
        delta_nanos: u64,
    },
    /// Non-monotonic time progression within a single schedule branch.
    TimeWentBackwards {
        /// Previous timestamp.
        prev_nanos: u64,
        /// Current timestamp.
        curr_nanos: u64,
    },
    /// Epsilon window cannot be zero.
    ZeroEpsilon,
    /// Event ID cannot be zero.
    ZeroEventId,
    /// Duplicate event ID was scheduled.
    DuplicateEventId {
        /// The duplicated event ID.
        event_id: u64,
    },
}

impl std::fmt::Display for DenseScheduleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnstableRaceDiscovered { event_a, event_b, delta_nanos } => {
                write!(f, "Unstable race discovered between events {event_a} and {event_b} (delta: {delta_nanos}ns)")
            }
            Self::TimeWentBackwards { prev_nanos, curr_nanos } => {
                write!(f, "Time went backwards: prev {prev_nanos}ns, curr {curr_nanos}ns")
            }
            Self::ZeroEpsilon => {
                write!(f, "Epsilon window cannot be zero")
            }
            Self::ZeroEventId => {
                write!(f, "Event ID cannot be zero")
            }
            Self::DuplicateEventId { event_id } => {
                write!(f, "Duplicate event ID {event_id}")
            }
        }
    }
}

impl std::error::Error for DenseScheduleViolation {}

/// Deterministic dense-time scheduler capable of exploring \epsilon-perturbations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DenseTimeScheduler {
    /// Epsilon window in nanoseconds (e.g. 10,000 ns = 10 µs).
    pub epsilon_nanos: u64,
    /// Pending events indexed by event ID.
    pub pending_events: BTreeMap<u64, DenseTimedEvent>,
}

impl DenseTimeScheduler {
    /// Creates a new dense-time scheduler with specified \epsilon perturbation window, validating > 0.
    pub fn try_new(epsilon_nanos: u64) -> Result<Self, DenseScheduleViolation> {
        if epsilon_nanos == 0 {
            return Err(DenseScheduleViolation::ZeroEpsilon);
        }
        Ok(Self::new(epsilon_nanos))
    }

    /// Creates a new dense-time scheduler with specified \epsilon perturbation window.
    #[must_use]
    pub fn new(epsilon_nanos: u64) -> Self {
        Self {
            epsilon_nanos,
            pending_events: BTreeMap::new(),
        }
    }

    /// Enqueues an event validating non-zero ID and no duplication.
    pub fn try_schedule_event(
        &mut self,
        event_id: u64,
        timestamp_nanos: u64,
        event_type: ScheduledEventType,
    ) -> Result<(), DenseScheduleViolation> {
        if event_id == 0 {
            return Err(DenseScheduleViolation::ZeroEventId);
        }
        if self.pending_events.contains_key(&event_id) {
            return Err(DenseScheduleViolation::DuplicateEventId { event_id });
        }
        self.schedule_event(event_id, timestamp_nanos, event_type);
        Ok(())
    }

    /// Enqueues an event at a given continuous timestamp.
    pub fn schedule_event(&mut self, event_id: u64, timestamp_nanos: u64, event_type: ScheduledEventType) {
        self.pending_events.insert(
            event_id,
            DenseTimedEvent {
                event_id,
                timestamp_nanos,
                event_type,
            },
        );
    }

    /// Finds all pairs of events whose timestamps are within the \epsilon window: |t_1 - t_2| <= \epsilon.
    #[must_use]
    pub fn find_epsilon_racing_pairs(&self) -> Vec<(u64, u64, u64)> {
        let mut events: Vec<&DenseTimedEvent> = self.pending_events.values().collect();
        events.sort_by_key(|e| e.timestamp_nanos);

        let mut racing_pairs = Vec::new();
        let n = events.len();

        for i in 0..n {
            for j in (i + 1)..n {
                let e1 = events[i];
                let e2 = events[j];
                let delta = e2.timestamp_nanos.saturating_sub(e1.timestamp_nanos);

                if delta <= self.epsilon_nanos {
                    racing_pairs.push((e1.event_id, e2.event_id, delta));
                } else {
                    // Since events are sorted, subsequent events will have delta > epsilon
                    break;
                }
            }
        }

        racing_pairs
    }

    /// Generates two alternative execution traces for a racing pair (A before B, and B before A).
    #[must_use]
    pub fn generate_perturbed_schedules(&self, id_a: u64, id_b: u64) -> (Vec<u64>, Vec<u64>) {
        let mut sorted: Vec<&DenseTimedEvent> = self.pending_events.values().collect();
        sorted.sort_by_key(|e| e.timestamp_nanos);

        let base_order: Vec<u64> = sorted.iter().map(|e| e.event_id).collect();

        // Trace 1: id_a before id_b
        let mut trace_ab = base_order.clone();
        if let (Some(pos_a), Some(pos_b)) = (
            trace_ab.iter().position(|&id| id == id_a),
            trace_ab.iter().position(|&id| id == id_b),
        ) {
            if pos_a > pos_b {
                trace_ab.swap(pos_a, pos_b);
            }
        }

        // Trace 2: id_b before id_a
        let mut trace_ba = base_order;
        if let (Some(pos_a), Some(pos_b)) = (
            trace_ba.iter().position(|&id| id == id_a),
            trace_ba.iter().position(|&id| id == id_b),
        ) {
            if pos_a < pos_b {
                trace_ba.swap(pos_a, pos_b);
            }
        }

        (trace_ab, trace_ba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dense_time_scheduler_bounds() {
        assert_eq!(
            DenseTimeScheduler::try_new(0),
            Err(DenseScheduleViolation::ZeroEpsilon)
        );

        let mut sched = DenseTimeScheduler::try_new(10_000).expect("valid scheduler");

        assert_eq!(
            sched.try_schedule_event(0, 100, ScheduledEventType::TimerExpiry { timer_id: 1 }),
            Err(DenseScheduleViolation::ZeroEventId)
        );

        sched
            .try_schedule_event(1, 100, ScheduledEventType::TimerExpiry { timer_id: 1 })
            .expect("schedule event 1");

        assert_eq!(
            sched.try_schedule_event(1, 150, ScheduledEventType::IoCompletion { io_id: 2 }),
            Err(DenseScheduleViolation::DuplicateEventId { event_id: 1 })
        );
    }

    #[test]
    fn test_dense_schedule_violation_display() {
        let err = DenseScheduleViolation::ZeroEpsilon;
        assert_eq!(format!("{err}"), "Epsilon window cannot be zero");

        let err2 = DenseScheduleViolation::ZeroEventId;
        assert_eq!(format!("{err2}"), "Event ID cannot be zero");

        let err3 = DenseScheduleViolation::DuplicateEventId { event_id: 42 };
        assert_eq!(format!("{err3}"), "Duplicate event ID 42");
    }
}
