# RFC-0012: Next significant steps (delivered)

**Status:** done  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Companion:** [research decisions](0012-research-decisions.md)  
**Successor product contract:** [RFC-0013 MontanhaDb](0013-montanhadb-product.md) (FDB-shaped multi-node product on PedraDB)  

---

## Background

After RFCs 0001–0011 shipped the local kernel + in-process Raft/DCS/SQL/stream, this RFC **delivered** the multi-node and wire steps that were still open.

| Deliverable | Location |
|-------------|----------|
| Persist Raft hard state + log | `pedradb-raft::persist` (`RAFT_HARD` / `RAFT_LOG`) |
| TCP Raft RPC | `pedradb-raft::net` + binary `pedra-raft-node` |
| Multi-process 3-node smoke | `multi_process_three_nodes_smoke` test |
| HTTP DCS (leader/CAS/lease) | `pedradb-http::DcsServer` |
| HTTP KV | `pedradb-http::KvServer` |
| Seedable DST sweep | `pedradb-dst` |
| Research non-ship | [0012-research-decisions.md](0012-research-decisions.md) |

## Problems this solved

- **Problem:** In-process Raft alone cannot prove multi-process elect/replicate.  
- **Problem:** DCS/KV had no wire for HA managers / apps.  
- **Problem:** Research opts stayed eternal `todo` without a decision artifact.

## Proposed solution (shipped)

1. Disk-backed Raft meta + TCP framing for RequestVote/AppendEntries/client put-get.  
2. Minimal HTTP/1.0 servers (no TLS) for KV and DCS subset.  
3. Seed harness reusing `FailingEnv::from_seed`.  
4. Explicit research non-ship document with reopen gate.

## Delivery slices

### P0 — multi-node reality

- [x] **P0.1** Network transport for Raft RPCs — status: `done` (`net.rs`, TCP)  
- [x] **P0.2** Persist Raft hard state + log — status: `done` (`persist.rs`)  
- [x] **P0.3** 3-node multi-process smoke — status: `done` (`multi_process_three_nodes_smoke`)  

### P1 — product wires

- [x] **P1.1** DCS HTTP wire (leader/CAS/lease) — status: `done` (`pedradb-http::DcsServer`)  
- [x] **P1.2** KV HTTP — status: `done` (`pedradb-http::KvServer`)  
- [x] **P1.3** Seedable DST harness — status: `done` (`pedradb-dst`)  

### P2 — engine depth & protocols

- [x] **P2.1** Bloom / value-log / LL — status: `done` (**non-ship** + reopen gate; see companion)  
- [x] **P2.2** MemTable skiplist — status: `done` (**keep BTreeMap**; companion)  
- [x] **P2.3** Full SQL wire — status: `done` (**embed `pedradb-sql` only**; no PG wire)  
- [x] **P2.4** JetStream protocol — status: `done` (**library `pedradb-stream` only**; no NATS protocol)  
- [x] **P2.5** OS-level lying fsync — status: `done` (**in-process `RecordingEnv`/`SyncPolicy::Lying`** is the pure-Rust substitute; LD_PRELOAD det_io out of charter)  

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Network Raft transport | done | `pedradb-raft::net` | 2026-08-11 |
| P0.2 | p0 | Persist Raft hard state | done | `pedradb-raft::persist` | 2026-08-11 |
| P0.3 | p0 | Multi-process 3-node smoke | done | multi_process test + bin | 2026-08-11 |
| P1.1 | p1 | DCS HTTP wire | done | `pedradb-http` DcsServer | 2026-08-11 |
| P1.2 | p1 | KV HTTP | done | `pedradb-http` KvServer | 2026-08-11 |
| P1.3 | p1 | Seedable DST | done | `pedradb-dst` | 2026-08-11 |
| P2.1 | p2 | Research LSM opts | done | non-ship companion | 2026-08-11 |
| P2.2 | p2 | MemTable skiplist | done | non-ship companion | 2026-08-11 |
| P2.3 | p2 | Full SQL wire | done | embed-only decision | 2026-08-11 |
| P2.4 | p2 | JetStream protocol | done | library-only decision | 2026-08-11 |
| P2.5 | p2 | OS lying fsync | done | RecordingEnv substitute | 2026-08-11 |

## Acceptance criteria

### Tests
- [x] Network 3-node elect+put (threads).  
- [x] Multi-process 3-node smoke via `pedra-raft-node`.  
- [x] Hard state/log reload after reopen.  
- [x] HTTP KV put/get; DCS leader race 409.  
- [x] DST seed sweep no silent loss.

### Telemetry
- None.

### Documentation
- This RFC + research companion + `docs/usage.md` crate list.

### Screenshots
- backend-only.

## Out of scope (future RFCs if needed)

- TLS, auth, production load balancing.  
- Full etcd gRPC / Patroni Python plugin packaging.  
- Postgres wire protocol.  
- LD_PRELOAD det_io.
