# RFC-0010: Databases and products on PedraDB

**Status:** done (P0–P2 charter shipped; HTTP/network delivery in [RFC-0012](0012-next-significant-steps.md))  
**Updated:** 2026-08-11  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md), [grail plan](../grail-plan-build-databases-on-pedradb.md)  
**Sibling:** [RFC-0009](0009-rocksdb-class-engine.md) — make the kernel RocksDB-class  
**Track:** C (outer products) — runs **in parallel** with 0009 A/B  

---

## Background

PedraDB is the **local** substrate (RocksDB’s role + multi-key TX). Outer products embed it per process — same pattern as TiKV→RocksDB, FDB→Redwood.

Hooks that already exist for outer products:

- `apply_batch` — ordered multi-op apply, no OCC (Raft apply).  
- `snapshot` / `get_at` / `range_at` — seq export.  
- `Wal::recover_from_offset` / `stream_position` — log shipping (Rung 1.5).  

Missing: any multi-node runtime, wire protocols, SQL, or packaging of those hooks.

## Problems this solves

- **Problem:** Without a clear outer ladder, work either stalls on engine perfection or jumps to “full TiDB” too early.  
- **Problem:** Raft/etcd/SQL teams need a **stable apply + snapshot contract** documented and exercised outside core.  
- **Problem:** Scylla/NATS/etcd *needs* (control plane, durable log, DCS) must not pollute the kernel.

## Proposed solution

Separate crates / products that **only** use public PedraDB APIs:

```
Rung 1    examples / index layers          (embed)
Rung 1.5  WAL ship / read replicas         (pedradb-replicate*)
Rung 3    multi-Raft + apply_batch         (pedradb-raft*)
Rung 4    distributed KV API               (pedradb-kv*)
Rung 5    SQL wire                         (pedradb-sql*)
Rung 6    etcd-class DCS                   (pedradb-coord*)
Rung X    Scylla need (routes/watch)       (control-plane product)
```

\* names illustrative.

**P0 of this RFC:** a minimal **ordered apply** crate + docs so Raft (or a fake log) can drive PedraDB without forking core. Full multi-Raft is P1+ of *this* product line, not of pedradb-core.

## Delivery slices

### P0 — apply substrate (start now)

- [x] **P0.1** Crate `pedradb-apply`: `LogApplier` that calls `Db::apply_batch` — status: `done`  
- [x] **P0.2** In-process “fake log” demo: append entries → apply → reopen checks — status: `done`  
- [x] **P0.3** Document Raft integration sketch (leader apply only; snapshot = PedraDB snapshot seq) — status: `done`  

### P1 — distribution MVP

- [x] **P1.1** Single-region Raft + PedraDB per node — status: `done`  
  — `pedradb-raft`: real Raft (terms, RequestVote, AppendEntries, commit/apply) in-process; `InProcessCluster` remains the simpler FakeLog path
- [x] **P1.2** WAL-shipped read replica (Rung 1.5) — status: `done`  
  — crate `pedradb-replicate`: physical `CURRENT.log` ship + reopen recover; detects WAL rotate after flush
- [x] **P1.3** KV API thin layer (get/put/TX) — status: `done` (in-process)  
  — `KvService` façade; gRPC/HTTP wire deferred to product packaging

### P2 — product surfaces

- [x] **P2.1** etcd-class DCS plugin path (Patroni) — status: `done` (state machine)  
  — `pedradb-dcs`: CAS/create, leases/TTL, watch, `try_acquire_leader` / `renew_leader`. etcd gRPC / Patroni plugin wire → RFC-0012.
- [x] **P2.2** SQL subset — status: `done`  
  — `pedradb-sql`: CREATE/INSERT/SELECT/DELETE minimal dialect (not PG wire).
- [x] **P2.3** Control-plane KV + watch (Scylla *need*) — status: `done` (local)  
  — same `pedradb-dcs` watch/CAS surface; multi-node network CP → RFC-0012.
- [x] **P2.4** JetStream-class durable stream product — status: `done` (library)  
  — `pedradb-stream`: publish / consumer cursor / durable reopen (not NATS protocol).

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | pedradb-apply crate | done | crates/pedradb-apply | 2026-08-11 |
| P0.2 | p0 | Fake log demo + tests | done | FakeLog + tests | 2026-08-11 |
| P0.3 | p0 | Raft integration doc | done | docs/apply-and-raft.md | 2026-08-11 |
| P1.1 | p1 | Raft + PedraDB | done | pedradb-raft (+ InProcessCluster) | 2026-08-11 |
| P1.2 | p1 | WAL-shipped replica | done | crates/pedradb-replicate | 2026-08-11 |
| P1.3 | p1 | Thin KV API | done | KvService (in-process) | 2026-08-11 |
| P2.1 | p2 | etcd/DCS SM | done | crates/pedradb-dcs | 2026-08-11 |
| P2.2 | p2 | SQL subset | done | crates/pedradb-sql | 2026-08-11 |
| P2.3 | p2 | CP routes/watch | done | pedradb-dcs watch/CAS | 2026-08-11 |
| P2.4 | p2 | Durable stream | done | crates/pedradb-stream | 2026-08-11 |

## Acceptance criteria

### Tests
- Fake log applies multi-entry batches; process reopen shows all applied keys.  
- Partial log (truncated) does not leave half-applied batches (depends on core atomic `apply_batch`).  
- (P1+) Raft smoke: 3 nodes, put via leader, follower read after commit.

### Telemetry
- None in P0.

### Documentation
- This RFC + `docs/apply-and-raft.md` (P0.3).  
- Cross-link RFC-0009: outer products should pin minimum engine maturity (e.g. auto-flush recommended before production Raft).

### Screenshots
- backend-only.

## Out of scope

- Growing pedradb-core into a server.  
- Full TiDB/Postgres compatibility.  
- CQL/NATS drop-in (need replacement only).

## Dependency on RFC-0009

| Outer feature | Soft dependency on 0009 |
|---------------|-------------------------|
| Embed index example | None (works today) |
| Fake log apply | None (`apply_batch` exists) |
| Production Raft | **0009 P0** (write path) strongly recommended; **0009 P1** before large state |
| SQL horizontal | 0009 P1+ and 0010 P1+ |

**Parallel rule:** implement 0010 P0 **now** against current core; track engine debt in 0009 without blocking the apply crate.
