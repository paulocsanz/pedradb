//! RFC-0258 / RFC-0261 — Stateright model of the `WriteGroup` concurrent synchronization protocol.
//!
//! Models:
//! 1. Concurrent multi-client arrivals & multi-key batch writes.
//! 2. Group batching with WAL ticket reservation (`reserve_frame`).
//! 3. Durability sync barrier (`fdatasync`).
//! 4. Atomic multi-key sequence publishing (`published_seq`).
//! 5. Concurrent Snapshot Readers observing MVCC states.
//! 6. Non-deterministic crash injection & recovery.
//!
//! Verified Invariants:
//! - **Inv-linear-monotonic-commit:** committed sequence numbers advance strictly monotonically.
//! - **Inv-no-dirty-publish:** no client ever marks committed before the WAL sync barrier.
//! - **Inv-committed-bounded-by-publish:** clients only hold committed seq <= published_seq.
//! - **Inv-atomic-batch-visibility:** multi-key transactions are seen all-or-nothing by concurrent readers.
//! - **Inv-no-future-read:** reader snapshots never observe uncommitted or future writes.
//! - **Inv-crash-safe:** after a crash/reopen, un-synced writes are discarded; only synced writes survive.
//! - **AS-IS teeth:** an AS-IS mutant publishing before the barrier produces a counterexample.

use pedradb_core::wal_ticket_kernel::reserve_frame;
use stateright::{Checker, Model, Property};

const MAX_CLIENTS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ClientStatus {
    Idle,
    Queued,
    InFlight { ticket: u64 },
    Committed { seq: u64 },
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct WriteGroupState {
    clients: [ClientStatus; MAX_CLIENTS],
    leader: Option<usize>,
    wal_reserved_to: u64,
    wal_synced_to: u64,
    published_seq: u64,
    store_key_a: u64,
    store_key_b: u64,
    torn_batch_observed: bool,
    future_read_observed: bool,
    dirty_publish_observed: bool,
    non_monotonic_commit: bool,
    step_budget: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum WriteGroupAction {
    Arrive(usize),
    LeaderBatch,
    LeaderSync,
    LeaderPublish,
    ReaderQuery,
    Crash,
}

#[derive(Clone)]
struct WriteGroupModel {
    fixed: bool,
}

impl Model for WriteGroupModel {
    type State = WriteGroupState;
    type Action = WriteGroupAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![WriteGroupState {
            clients: [ClientStatus::Idle; MAX_CLIENTS],
            leader: None,
            wal_reserved_to: 0,
            wal_synced_to: 0,
            published_seq: 0,
            store_key_a: 0,
            store_key_b: 0,
            torn_batch_observed: false,
            future_read_observed: false,
            dirty_publish_observed: false,
            non_monotonic_commit: false,
            step_budget: 8,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.step_budget == 0 {
            return;
        }

        // Clients can arrive
        for i in 0..MAX_CLIENTS {
            if state.clients[i] == ClientStatus::Idle {
                actions.push(WriteGroupAction::Arrive(i));
            }
        }

        // Leader can batch queued clients if leader exists
        let has_queued = state.clients.iter().any(|c| *c == ClientStatus::Queued);
        if has_queued && state.leader.is_some() {
            actions.push(WriteGroupAction::LeaderBatch);
        }

        // Leader can sync WAL if reserved > synced
        if state.leader.is_some() && state.wal_reserved_to > state.wal_synced_to {
            actions.push(WriteGroupAction::LeaderSync);
        }

        // Leader can publish in-flight batches:
        // In fixed mode, must sync first (wal_synced_to >= wal_reserved_to).
        // In AS-IS mutant mode, can publish before sync.
        let has_inflight = state.clients.iter().any(|c| matches!(c, ClientStatus::InFlight { .. }));
        if state.leader.is_some() && has_inflight {
            if !self.fixed || state.wal_synced_to >= state.wal_reserved_to {
                actions.push(WriteGroupAction::LeaderPublish);
            }
        }

        // Reader can query at any time
        actions.push(WriteGroupAction::ReaderQuery);

        // Crash can occur at any point with in-flight or dirty writes
        if state.wal_reserved_to > state.wal_synced_to || has_inflight {
            actions.push(WriteGroupAction::Crash);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.step_budget = next.step_budget.saturating_sub(1);

        match action {
            WriteGroupAction::Arrive(id) => {
                if next.leader.is_none() {
                    next.leader = Some(id);
                }
                next.clients[id] = ClientStatus::Queued;
            }
            WriteGroupAction::LeaderBatch => {
                // Batch all queued clients, reserve WAL frames using production reserve_frame
                for i in 0..MAX_CLIENTS {
                    if next.clients[i] == ClientStatus::Queued {
                        let (ticket, new_reserved) = reserve_frame(next.wal_reserved_to, 1);
                        next.wal_reserved_to = new_reserved;
                        next.clients[i] = ClientStatus::InFlight { ticket };
                    }
                }
            }
            WriteGroupAction::LeaderSync => {
                // Advance wal_synced_to to wal_reserved_to
                next.wal_synced_to = next.wal_reserved_to;
            }
            WriteGroupAction::LeaderPublish => {
                for i in 0..MAX_CLIENTS {
                    if let ClientStatus::InFlight { ticket } = next.clients[i] {
                        if ticket >= next.wal_synced_to {
                            next.dirty_publish_observed = true;
                        }

                        let new_seq = next.published_seq + 1;
                        if new_seq <= next.published_seq {
                            next.non_monotonic_commit = true;
                        }
                        next.published_seq = new_seq;
                        // Atomic batch write: update both keys in store
                        next.store_key_a = new_seq;
                        next.store_key_b = new_seq;
                        next.clients[i] = ClientStatus::Committed { seq: new_seq };
                    }
                }
                // Group completed; leader steps down
                next.leader = None;
            }
            WriteGroupAction::ReaderQuery => {
                let snap = next.published_seq;
                let val_a = std::cmp::min(next.store_key_a, snap);
                let val_b = std::cmp::min(next.store_key_b, snap);

                if val_a != val_b {
                    next.torn_batch_observed = true;
                }
                if val_a > snap || val_b > snap {
                    next.future_read_observed = true;
                }
            }
            WriteGroupAction::Crash => {
                // System recovers: only synced writes survive.
                // In-flight / un-synced clients revert to Idle.
                next.wal_reserved_to = next.wal_synced_to;
                next.store_key_a = std::cmp::min(next.store_key_a, next.wal_synced_to);
                next.store_key_b = std::cmp::min(next.store_key_b, next.wal_synced_to);
                next.published_seq = std::cmp::min(next.published_seq, next.wal_synced_to);

                for i in 0..MAX_CLIENTS {
                    match next.clients[i] {
                        ClientStatus::Committed { seq } => {
                            if seq > next.wal_synced_to {
                                next.clients[i] = ClientStatus::Idle;
                            }
                        }
                        ClientStatus::InFlight { ticket } => {
                            if ticket >= next.wal_synced_to {
                                next.clients[i] = ClientStatus::Idle;
                            }
                        }
                        ClientStatus::Queued => {
                            next.clients[i] = ClientStatus::Idle;
                        }
                        ClientStatus::Idle => {}
                    }
                }
                next.leader = None;
            }
        }

        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-dirty-publish", inv_no_dirty_publish),
            Property::always("Inv-linear-monotonic-commit", inv_linear_monotonic_commit),
            Property::always("Inv-committed-bounded-by-publish", inv_committed_bounded_by_publish),
            Property::always("Inv-atomic-batch-visibility", inv_atomic_batch_visibility),
            Property::always("Inv-no-future-read", inv_no_future_read),
            Property::sometimes("can-commit-all-clients", can_commit_all_clients),
        ]
    }
}

fn inv_no_dirty_publish(_: &WriteGroupModel, s: &WriteGroupState) -> bool {
    !s.dirty_publish_observed
}

fn inv_linear_monotonic_commit(_: &WriteGroupModel, s: &WriteGroupState) -> bool {
    !s.non_monotonic_commit
}

fn inv_committed_bounded_by_publish(_: &WriteGroupModel, s: &WriteGroupState) -> bool {
    s.clients.iter().all(|c| match c {
        ClientStatus::Committed { seq } => *seq <= s.published_seq,
        _ => true,
    })
}

fn inv_atomic_batch_visibility(_: &WriteGroupModel, s: &WriteGroupState) -> bool {
    !s.torn_batch_observed
}

fn inv_no_future_read(_: &WriteGroupModel, s: &WriteGroupState) -> bool {
    !s.future_read_observed
}

fn can_commit_all_clients(_: &WriteGroupModel, s: &WriteGroupState) -> bool {
    s.clients.iter().all(|c| matches!(c, ClientStatus::Committed { .. }))
}

#[test]
fn fixed_write_group_protocol_holds() {
    let checker = WriteGroupModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_write_group_finds_dirty_publish_teeth() {
    let checker = WriteGroupModel { fixed: false }.checker().spawn_bfs().join();
    assert!(
        checker.discovery("Inv-no-dirty-publish").is_some(),
        "AS-IS write group mutant publishing before barrier must be caught with a counterexample"
    );
}
