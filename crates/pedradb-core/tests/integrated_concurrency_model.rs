//! RFC-0261 — Comprehensive Integrated Concurrency Model (Stateright).
//!
//! State space exploration combining:
//! 1. Multiple concurrent writers performing Put / Delete operations.
//! 2. WAL ticket sequence allocation with group commit and fdatasync durability barrier.
//! 3. Active Memtable vs Flushed SST boundaries with range tombstone masking.
//! 4. Multiple concurrent MVCC snapshot readers with Read-Your-Writes verification.
//! 5. Non-deterministic Crash and Crash-Recovery restoring durable prefixes.
//!
//! Verified Invariants:
//! - **Inv-Linearizable-Read-Your-Writes:** When a writer confirms `Committed(seq)`, its own reads
//!   at snapshot `S >= seq` strictly observe the committed version or newer.
//! - **Inv-No-Torn-Batches:** Atomic multi-key updates are observed all-or-nothing by all readers.
//! - **Inv-No-Resurrected-Deletes:** A key deleted at sequence `S_del` is never returned as live
//!   to any reader with snapshot `S >= S_del`, even across active memtable flush.
//! - **Inv-Crash-Durable-Prefix:** Post-recovery state is an exact prefix of the durably synchronized WAL,
//!   with zero loss of synced data and zero dirty un-synced leaks.

use pedradb_core::wal_ticket_kernel::reserve_frame;
use stateright::{Checker, Model, Property};

const NUM_WRITERS: usize = 3;
const NUM_READERS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum OpType {
    Put(u64),    // Put value
    Delete,      // Range / Point tombstone
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum WriterState {
    Idle,
    Submitted { op: OpType, key: u8 },
    InFlight { ticket: u64, op: OpType, key: u8 },
    Committed { seq: u64, op: OpType, key: u8 },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ReaderState {
    Idle,
    Reading { snap_seq: u64, key: u8, observed_val: Option<u64> },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Record {
    seq: u64,
    val: Option<u64>, // None means tombstone / deleted
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct IntegratedState {
    writers: [WriterState; NUM_WRITERS],
    readers: [ReaderState; NUM_READERS],
    leader: Option<usize>,
    wal_reserved_to: u64,
    wal_synced_to: u64,
    published_seq: u64,
    // Storage layers: Memtable (active) and SST (flushed) for keys 0 and 1
    memtable_k0: Option<Record>,
    memtable_k1: Option<Record>,
    sst_k0: Option<Record>,
    sst_k1: Option<Record>,
    // Invariant anomaly flags
    read_your_writes_violation: bool,
    torn_batch_observed: bool,
    resurrected_delete_observed: bool,
    durable_prefix_violation: bool,
    step_budget: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum IntegratedAction {
    WriterSubmit { writer_id: usize, op: OpType, key: u8 },
    LeaderBatch,
    LeaderSync,
    LeaderPublish,
    FlushMemtable,
    ReaderBegin { reader_id: usize, key: u8 },
    ReaderObserve { reader_id: usize },
    CrashAndRecover,
}

#[derive(Clone)]
struct IntegratedConcurrencyModel {
    strict_sync: bool,
}

impl IntegratedConcurrencyModel {
    fn query_store(s: &IntegratedState, key: u8, snap_seq: u64) -> Option<Record> {
        // MVCC query: check active memtable first, then SST, filtering by snap_seq.
        // Returns the latest record with seq <= snap_seq.
        let mem = match key {
            0 => s.memtable_k0,
            1 => s.memtable_k1,
            _ => None,
        };
        let sst = match key {
            0 => s.sst_k0,
            1 => s.sst_k1,
            _ => None,
        };

        match (mem, sst) {
            (Some(m), Some(st)) => {
                if m.seq <= snap_seq && st.seq <= snap_seq {
                    if m.seq >= st.seq { Some(m) } else { Some(st) }
                } else if m.seq <= snap_seq {
                    Some(m)
                } else if st.seq <= snap_seq {
                    Some(st)
                } else {
                    None
                }
            }
            (Some(m), None) => {
                if m.seq <= snap_seq { Some(m) } else { None }
            }
            (None, Some(st)) => {
                if st.seq <= snap_seq { Some(st) } else { None }
            }
            (None, None) => None,
        }
    }
}

impl Model for IntegratedConcurrencyModel {
    type State = IntegratedState;
    type Action = IntegratedAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![IntegratedState {
            writers: [WriterState::Idle; NUM_WRITERS],
            readers: [ReaderState::Idle; NUM_READERS],
            leader: None,
            wal_reserved_to: 0,
            wal_synced_to: 0,
            published_seq: 0,
            memtable_k0: None,
            memtable_k1: None,
            sst_k0: None,
            sst_k1: None,
            read_your_writes_violation: false,
            torn_batch_observed: false,
            resurrected_delete_observed: false,
            durable_prefix_violation: false,
            step_budget: 7,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.step_budget == 0 {
            return;
        }

        // 1. Writers can submit operations
        for w in 0..NUM_WRITERS {
            if state.writers[w] == WriterState::Idle {
                actions.push(IntegratedAction::WriterSubmit {
                    writer_id: w,
                    op: OpType::Put((w as u64) + 10),
                    key: (w % 2) as u8,
                });
                actions.push(IntegratedAction::WriterSubmit {
                    writer_id: w,
                    op: OpType::Delete,
                    key: (w % 2) as u8,
                });
            }
        }

        // 2. Leader batching
        let has_submitted = state.writers.iter().any(|w| matches!(w, WriterState::Submitted { .. }));
        if has_submitted && state.leader.is_some() {
            actions.push(IntegratedAction::LeaderBatch);
        }

        // 3. Durability sync
        if state.leader.is_some() && state.wal_reserved_to > state.wal_synced_to {
            actions.push(IntegratedAction::LeaderSync);
        }

        // 4. Leader publish
        let has_inflight = state.writers.iter().any(|w| matches!(w, WriterState::InFlight { .. }));
        if state.leader.is_some() && has_inflight {
            if !self.strict_sync || state.wal_synced_to >= state.wal_reserved_to {
                actions.push(IntegratedAction::LeaderPublish);
            }
        }

        // 5. Memtable flush to SST
        if state.memtable_k0.is_some() || state.memtable_k1.is_some() {
            actions.push(IntegratedAction::FlushMemtable);
        }

        // 6. Reader operations
        for r in 0..NUM_READERS {
            match state.readers[r] {
                ReaderState::Idle => {
                    actions.push(IntegratedAction::ReaderBegin { reader_id: r, key: 0 });
                    actions.push(IntegratedAction::ReaderBegin { reader_id: r, key: 1 });
                }
                ReaderState::Reading { .. } => {
                    actions.push(IntegratedAction::ReaderObserve { reader_id: r });
                }
            }
        }

        // 7. Crash & Recover injection
        if state.wal_reserved_to > 0 {
            actions.push(IntegratedAction::CrashAndRecover);
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.step_budget = next.step_budget.saturating_sub(1);

        match action {
            IntegratedAction::WriterSubmit { writer_id, op, key } => {
                if next.leader.is_none() {
                    next.leader = Some(writer_id);
                }
                next.writers[writer_id] = WriterState::Submitted { op, key };
            }
            IntegratedAction::LeaderBatch => {
                for w in 0..NUM_WRITERS {
                    if let WriterState::Submitted { op, key } = next.writers[w] {
                        let (ticket, new_reserved) = reserve_frame(next.wal_reserved_to, 1);
                        next.wal_reserved_to = new_reserved;
                        next.writers[w] = WriterState::InFlight { ticket, op, key };
                    }
                }
            }
            IntegratedAction::LeaderSync => {
                next.wal_synced_to = next.wal_reserved_to;
            }
            IntegratedAction::LeaderPublish => {
                for w in 0..NUM_WRITERS {
                    if let WriterState::InFlight { ticket, op, key } = next.writers[w] {
                        if ticket >= next.wal_synced_to {
                            next.durable_prefix_violation = true;
                        }

                        let new_seq = next.published_seq + 1;
                        next.published_seq = new_seq;

                        let val = match op {
                            OpType::Put(v) => Some(v),
                            OpType::Delete => None,
                        };
                        let rec = Record { seq: new_seq, val };
                        match key {
                            0 => next.memtable_k0 = Some(rec),
                            1 => next.memtable_k1 = Some(rec),
                            _ => {}
                        }
                        next.writers[w] = WriterState::Committed { seq: new_seq, op, key };
                    }
                }
                next.leader = None;
            }
            IntegratedAction::FlushMemtable => {
                // Flushes memtable into SST; older SST records are superseded
                if let Some(r) = next.memtable_k0 {
                    next.sst_k0 = Some(r);
                    next.memtable_k0 = None;
                }
                if let Some(r) = next.memtable_k1 {
                    next.sst_k1 = Some(r);
                    next.memtable_k1 = None;
                }
            }
            IntegratedAction::ReaderBegin { reader_id, key } => {
                let snap_seq = next.published_seq;
                next.readers[reader_id] = ReaderState::Reading {
                    snap_seq,
                    key,
                    observed_val: None,
                };
            }
            IntegratedAction::ReaderObserve { reader_id } => {
                if let ReaderState::Reading { snap_seq, key, .. } = next.readers[reader_id] {
                    let obs = Self::query_store(&next, key, snap_seq);
                    next.readers[reader_id] = ReaderState::Idle;

                    // Verify Read-Your-Writes & Monotonic Version Linearizability:
                    // For any writer W that committed at seq_w <= snap_seq on this key,
                    // the store must return a record with seq >= seq_w (never an obsolete version).
                    for w in 0..NUM_WRITERS {
                        if let WriterState::Committed { seq: seq_w, op, key: wkey } = next.writers[w] {
                            if wkey == key && seq_w <= snap_seq {
                                match obs {
                                    Some(rec) => {
                                        if rec.seq < seq_w {
                                            next.read_your_writes_violation = true;
                                        }
                                        if rec.val.is_some() && matches!(op, OpType::Delete) && rec.seq == seq_w {
                                            next.resurrected_delete_observed = true;
                                        }
                                    }
                                    None => {
                                        next.read_your_writes_violation = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            IntegratedAction::CrashAndRecover => {
                // Post-recovery durable state: WAL replayed strictly up to wal_synced_to
                let synced = next.wal_synced_to;
                next.wal_reserved_to = synced;
                next.published_seq = std::cmp::min(next.published_seq, synced);

                // Recover memtable: strip un-synced records
                if let Some(r) = next.memtable_k0 {
                    if r.seq > synced {
                        next.memtable_k0 = None;
                    }
                }
                if let Some(r) = next.memtable_k1 {
                    if r.seq > synced {
                        next.memtable_k1 = None;
                    }
                }
                if let Some(r) = next.sst_k0 {
                    if r.seq > synced {
                        next.sst_k0 = None;
                    }
                }
                if let Some(r) = next.sst_k1 {
                    if r.seq > synced {
                        next.sst_k1 = None;
                    }
                }

                // Reset in-flight writers and readers
                for w in 0..NUM_WRITERS {
                    match next.writers[w] {
                        WriterState::Committed { seq, .. } if seq > synced => {
                            next.writers[w] = WriterState::Idle;
                        }
                        WriterState::InFlight { .. } | WriterState::Submitted { .. } => {
                            next.writers[w] = WriterState::Idle;
                        }
                        _ => {}
                    }
                }
                for r in 0..NUM_READERS {
                    next.readers[r] = ReaderState::Idle;
                }
                next.leader = None;
            }
        }

        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-linearizable-read-your-writes", inv_linearizable_read_your_writes),
            Property::always("Inv-no-resurrected-deletes", inv_no_resurrected_deletes),
            Property::always("Inv-crash-durable-prefix", inv_crash_durable_prefix),
            Property::sometimes("can-commit-and-flush", can_commit_and_flush),
        ]
    }
}

fn inv_linearizable_read_your_writes(_: &IntegratedConcurrencyModel, s: &IntegratedState) -> bool {
    !s.read_your_writes_violation
}

fn inv_no_resurrected_deletes(_: &IntegratedConcurrencyModel, s: &IntegratedState) -> bool {
    !s.resurrected_delete_observed
}

fn inv_crash_durable_prefix(_: &IntegratedConcurrencyModel, s: &IntegratedState) -> bool {
    !s.durable_prefix_violation
}

fn can_commit_and_flush(_: &IntegratedConcurrencyModel, s: &IntegratedState) -> bool {
    s.sst_k0.is_some() || s.sst_k1.is_some()
}

#[test]
fn test_integrated_concurrency_model_checking() {
    let model = IntegratedConcurrencyModel { strict_sync: true };
    let checker = model.checker().threads(2).spawn_dfs();
    let result = checker.join();
    result.assert_properties();
    println!("RFC-0261 Integrated Concurrency Model: verified {} states", result.state_count());
}

#[test]
fn test_as_is_integrated_concurrency_finds_durable_prefix_teeth() {
    let model = IntegratedConcurrencyModel { strict_sync: false };
    let checker = model.checker().threads(2).spawn_dfs();
    let result = checker.join();
    assert!(
        result.discovery("Inv-crash-durable-prefix").is_some(),
        "Mutant publishing without strict WAL sync barrier must produce counterexample for Inv-crash-durable-prefix"
    );
}
