# RFC-0001: PedraDB high-level specification

**Status:** done (P0–P2 main-line complete; multi-node/wire in [RFC-0012](0012-next-significant-steps.md) also done)  
**Updated:** 2026-08-11  
**Authors:** PedraDB session notes (consolidated)  
**Readers:** product + engineering — challenge open decisions before more code

---

## Background

### What exists today (facts)

- Workspace `pedradb` with crates: `pedradb-core`, `pedradb-sim` (fault injection), `pedradb-oracle` (model + optional RocksDB), `pedradb-cli`.
- **P0–P2 main line shipped** on the local library (see status table).
- Extensive research docs (engines, FDB, TiKV, fjall, distribution) — **research is not product surface**.

### Why now

- Need a single **high-level spec** so decisions stop drifting (local vs multi-node, TX vs store-only, “like fjall” vs “like FDB kernel”).
- Market for Rust embed KV is **crowded**; only a **tiny, justified kernel** is worth building.
- User feedback: almost there, but some decisions may be wrong — this RFC makes them **explicit and reviewable**.

### Related docs (non-normative unless cited here)

| Doc | Role |
|-----|------|
| `docs/positioning.md` | Justify use first; power/surface |
| `docs/architecture-refined.md` | Local substrate role (not multi-node product) |
| `docs/compare-fjall.md` | Closest peer |
| `docs/competitive-landscape-rust.md` | Ecosystem |
| Distribution docs | Research for a **future outer product**, not PedraDB scope |

**This RFC is normative for PedraDB product scope.** Other docs must not contradict it without updating this RFC.

---

## Problems this solves

1. **Problem:** Apps and database builders need **multi-key atomic updates** on ordered keys (e.g. row + secondary index) **in-process**, without running Postgres/FDB/TiKV.  
   RocksDB-class engines force bolting on TX; SQL engines are huge.

2. **Problem:** A future multi-node DB (TiKV-class) needs a **local library** to own disk on each process — like RocksDB under TiKV — ideally with **local ACID** so the outer system is not inventing MVCC from put/get only.

3. **Problem:** Existing Rust embeds (fjall, redb, SurrealKV…) either optimize for **general kit surface**, **product-specific** needs, or **B-tree** trade-offs. There is room for a **ruthlessly small TX-first kernel** — but only if we don’t become “worse fjall.”

---

## Proposed solution (product definition)

### One sentence

**PedraDB is a pure-Rust embedded library that provides ordered key-value storage with multi-key ACID transactions on a single process — and nothing else.**

### What users get

```text
Db::open(path) -> Db
db.begin() -> Transaction
tx.get(key) -> Option<Value>
tx.put(key, value)
tx.delete(key)
tx.range(start..end) -> Iterator  // read snapshot of this TX
tx.commit() -> Result<(), Conflict | Io | ...>
tx.abort()
```

Optional later (still small, only if needed for justify-use or outer embed):

- `Db::snapshot()` / read-only TX  
- explicit durability on commit (`sync` vs `os_buffer`) — **one** clear default  
- `WriteBatch` as non-interactive atomic apply (for outer Raft apply) — **if** TX alone is awkward for apply path  

### What users do **not** get (this product)

| Out | Rationale |
|-----|-----------|
| Network server / multi-node / Raft / PD | Other product may embed PedraDB per process |
| Multi-process open of same `path` | Wrong model for LSM; use one process (see concurrency) |
| SQL, documents, secondary indexes in core | Layers using TX |
| Redis-like types, pub/sub | Different product |
| RocksDB on-disk compatibility | Clean-room; oracle optional for tests |
| Large knob surface / compaction filter zoo | Power/surface ratio |

### Role in a larger stack

```
┌─────────────────────────────────────┐
│  App  OR  future multi-node DB      │  ← not PedraDB
│  (your protocol, SQL layer, …)      │
└──────────────┬──────────────────────┘
               │ link library
┌──────────────▼──────────────────────┐
│  PedraDB (this RFC)                 │
│  ordered KV + multi-key ACID        │
│  one process · one directory        │
└──────────────┬──────────────────────┘
               │
             disk
```

**PedraDB ≈ RocksDB’s role + local multi-key TX.**  
**Not ≈ TiKV/FDB as a product.**

---

## High-level architecture

### Logical layers **inside** the library (single process)

```
┌──────────────────────────────────────────┐
│  Transactional API (public)              │  begin/commit, OCC, MVCC filter
├──────────────────────────────────────────┤
│  Versioned store                         │  seqnums, batches, recovery
├──────────────────────────────────────────┤
│  Storage engine                          │  WAL · MemTable · SST · compact
│  (LSM; research opts under the hood)     │  value log when justified
└──────────────────────────────────────────┘
```

Public crates today may keep code in `pedradb-core`; **logical** split is store vs tx even if one crate.

### Data model

- Keys: arbitrary bytes (reasonable max size TBD — see open decisions).  
- Values: arbitrary bytes.  
- Total order: lexicographic byte order (user-key order for ranges).  
- Multiple logical namespaces: **key prefixes chosen by the application/layer**, not first-class “keyspace” objects in v1 API.  
  - *Open:* this rejects fjall-style physical keyspaces — **review carefully** (§ Open decisions).

### Concurrency model

| Allowed | Not allowed |
|---------|-------------|
| Many threads in **one** process sharing one `Db` | Two OS processes opening the same data directory for R/W |
| Concurrent read TXs | Assuming multi-process LMDB semantics |
| Concurrent write TXs with **OCC** (conflicts → abort + retry) | Silent lost updates on the default API |

**Single-writer TX mode** (serialize all write TXs) is a possible **implementation option** or config for simplicity — not a second product API. Prefer one mental model: optimistic multi-writer if we can make it correct; else start single-writer for P0 justify-use.

### Durability model (high level)

- Committed TX must be recoverable after process crash **according to the documented commit durability policy**.  
- Default must be **one sentence** users can trust (proposal below — **open for challenge**).  
- **Proposal (draft):**  
  - `commit()` durability = **WAL synced to disk** (`fdatasync` of WAL) before success returns (**stronger default than fjall/RocksDB “OS buffers”**), accepting latency cost for “justify correctness first.”  
  - Optional later: `commit_relaxed()` or option for group commit / no-sync for benchmarks.  
- **Open:** strong default vs RocksDB-like default — product call (§ Open decisions).

### Isolation / consistency (single process)

- **Target:** serializable transactions via **optimistic concurrency control** (read/write sets or ranges; abort on conflict).  
- Reads in a TX see a **snapshot** as of TX start (or as of first read — **pin down in P1**).  
- After successful `commit()`, effects are visible to new TXs.  
- Not distributed consistency. No cluster. No linearizability across machines.

### Storage engine (implementation, not user surface)

- **LSM-tree** (not B-tree): write-heavy substrate path; peer to RocksDB/fjall, not redb/LMDB.  
- Components: WAL → MemTable → flush → SSTables → compaction.  
- **Versioning:** internal keys carry sequence + kind (put/delete) for MVCC.  
- Research (WiscKey value log, Monkey Bloom, Lazy Leveling): **under the hood when benches justify**; **not** public knobs; **not** required for P0 justify-use.  
- **Open:** shipping classic leveling first vs designing for Lazy Leveling from day one (§ Open decisions).

---

## Guarantees (normative goals)

| ID | Guarantee | Notes |
|----|-----------|--------|
| G1 | **Atomicity** | TX commits all or nothing (crash during commit does not leave partial TX visible after recovery) |
| G2 | **Durability** | Successful `commit()` under default policy survives process crash |
| G3 | **Isolation** | Concurrent TXs serializable under OCC (or documented single-writer serialization) |
| G4 | **Ordered range** | `range` respects byte order of user keys at the TX snapshot |
| G5 | **Single-process** | Concurrent use only within one process; multi-process open is undefined / error |
| G6 | **No silent upgrade of scope** | No network, SQL, or multi-node without a new RFC |

Non-guarantees: cross-process sharing, multi-node consistency, SQL semantics, performance SLOs until measured.

---

## Justify use (success for P0)

Someone can, with the public API only:

1. Open a DB, run a multi-key TX (`put` A and B, or row + index key), `commit`.  
2. Kill process, reopen, observe both keys.  
3. Sketch a secondary-index layer in a small amount of code (doc example).  

**Until that works, simulation frameworks and paper lists do not justify PedraDB.**

---

## Delivery slices

### P0 — must ship first (justify use: multi-key TX + crash)

Smallest vertical that proves the product sentence.

- [x] **P0.1** Crash-safe WAL append + recovery — status: `done`  
- [x] **P0.2** Versioned in-memory map (InternalKey + MemTable) + seqnums — status: `done` ([RFC-0002](0002-internal-key-memtable.md))  
- [x] **P0.3** Recover MemTable from WAL; basic `get`/`put`/`delete` auto-commit — status: `done` ([RFC-0003](0003-wal-recover-basic-engine.md))
- [x] **P0.4** Public `Transaction` (begin, get/put/delete, commit/abort) multi-key atomicity (single-writer) — status: `done` ([RFC-0004](0004-transaction-api.md))  
- [x] **P0.5** Documented durability on `commit` + crash test (kill after commit → reopen) — status: `done`  
- [x] **P0.6** Minimal user-facing docs: open/TX example + index-layer sketch — status: `done` ([usage.md](../usage.md))  

P0 **does not require** SST, full compaction, Monkey/WiscKey, or simulation framework.

### P1 — real store (survive growth)

- [x] **P1.1** Flush MemTable → SST; reopen loads SST set — status: `done` ([RFC-0006](0006-sst-flush.md))  
- [x] **P1.2** `get`/`range` merge MemTable ∪ SSTs with MVCC visibility — status: `done`  
- [x] **P1.3** Compaction (correctness first; simple strategy OK) — status: `done`  
- [x] **P1.4** Conflict detection solid (interval tree or equivalent) if OCC — status: `n/a` (single-writer retained; OCC deferred)  
- [x] **P1.5** Benches for get/put/commit (baseline numbers) — status: `done` (`benches/baseline.rs`)  
- [x] **P1.6** WAL addressable read from offset / sequence (export primitive) — status: `done` (`Wal::recover_from_offset`)

### P2 — trust, speed, substrate polish

- [x] **P2.1** Deterministic simulation / fault injection (disk, crash) — status: `done` (`pedradb-sim`)  
- [x] **P2.2** Research opts when benches show gain — status: `n/a` / deferred until measured win (baseline captured; no Lazy Leveling/WiscKey/Bloom shipped unmeasured)  
- [x] **P2.3** Apply-batch (ordered external apply, no OCC) + snapshot hooks — status: `done` (`apply_batch`, `Snapshot`)  
- [x] **P2.4** Oracle harness vs RocksDB where meaningful — status: `done` (model default; `live-rocksdb` optional)  

### Explicit non-slices (other RFCs / other repos)

- Multi-node cluster product  
- SQL / query layer  
- Network server  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | WAL append + recovery | done | initial WAL | 2026-08-10 |
| P0.2 | p0 | InternalKey + MemTable + seqnums | done | RFC-0002 / key+memtable | 2026-08-11 |
| P0.3 | p0 | WAL → MemTable recover; basic get/put | done | RFC-0003 / batch+db | 2026-08-11 |
| P0.4 | p0 | Public multi-key Transaction | done | RFC-0004 / tx.rs | 2026-08-11 |
| P0.5 | p0 | Commit durability + crash test | done | db rustdoc + crash tests | 2026-08-11 |
| P0.6 | p0 | Minimal usage + index-layer docs | done | docs/usage.md + pedra demo | 2026-08-11 |
| P1.1 | p1 | SST flush + reopen | done | RFC-0006 / sst + Db::flush | 2026-08-11 |
| P1.2 | p1 | Merged get/range + MVCC | done | Db::range + merge | 2026-08-11 |
| P1.3 | p1 | Compaction correctness | done | Db::compact | 2026-08-11 |
| P1.4 | p1 | OCC conflict structure | n/a | single-writer retained (O2) | 2026-08-11 |
| P1.5 | p1 | Baseline benches | done | benches/baseline.rs | 2026-08-11 |
| P1.6 | p1 | WAL seek/export by offset/seq | done | recover_from_offset | 2026-08-11 |
| P2.1 | p2 | Deterministic simulation | done | pedradb-sim FaultEnv | 2026-08-11 |
| P2.2 | p2 | Research LSM opts (measured) | n/a | deferred; baseline benches only | 2026-08-11 |
| P2.3 | p2 | Apply-batch / substrate hooks | done | apply_batch + Snapshot | 2026-08-11 |
| P2.4 | p2 | RocksDB oracle harness | done | model default; live-rocksdb optional | 2026-08-11 |

---

## Acceptance criteria

### Tests

- **P0:** unit tests for WAL; crash-recovery integration (commit then reopen); concurrent TX conflict or single-writer serialization tests as applicable.  
- **P1:** reopen after flush; range order; compaction doesn’t drop live keys.  
- **P2:** sim/fault scenarios named when framework exists.  

### Telemetry / analytics

- **None in P0** — library; optional counters later for benches.  
- No product analytics.

### Documentation

- This RFC + short README usage for P0.6.  
- positioning.md remains “why”; this RFC remains “what.”

### Screenshots

- **backend-only** — N/A.

---

## Out of scope (this RFC / this product)

- Multi-node, Raft, PD, gRPC server  
- Multi-process shared directory  
- SQL, secondary indexes in core, document models  
- On-disk compatibility with RocksDB/fjall  
- Matching fjall feature checklist  
- Claiming production readiness before P0–P1 acceptance  

---

## Decisions: locked vs challenge

### Locked (for now — change only with RFC amendment)

| ID | Decision |
|----|----------|
| L1 | **Local library only** — no multi-node in PedraDB |
| L2 | **No multi-process** open of same path |
| L3 | **Public identity = multi-key ACID + ordered KV** |
| L4 | **Tiny surface** — open/TX get/put/delete/range/commit |
| L5 | **LSM** storage family (not B-tree as primary design) |
| L6 | **Justify use before** sim/papers as product story |
| L7 | **`forbid(unsafe_code)`** in core |
| L8 | Outer multi-node DB is a **separate product** that may embed PedraDB |

### Open / possibly wrong (need explicit call)

Deep research (peers, nuances, regrets):  
**[`0001-open-decisions-deep-dive.md`](0001-open-decisions-deep-dive.md)**

| ID | Topic | Options | Notes / recommendation |
|----|--------|---------|------------------------|
| **O1** | **Commit durability default** | (a) fsync WAL on every commit (safer, slower) (b) OS buffer default like fjall/RocksDB (faster, easy to misuse) | **Locked for P0: (a)** — WAL `fdatasync` (or equiv.) before `commit` returns Ok. Group commit / relaxed later. |
| **O2** | **Write concurrency** | (a) OCC multi-writer from day one (b) single-writer TX only for P0 | **Locked for P0: (b)** single-writer TX; concurrent readers OK. OCC optional in P1. |
| **O3** | **Physical keyspaces** | (a) none in v1 — prefixes only (b) fjall-like keyspaces | **Recommend (a)** FDB-style; RocksDB CF zoo is an ops regret; TiKV keeps tiny fixed CF set. |
| **O4** | **Interactive TX vs apply-batch first** | (a) interactive TX is P0 (b) atomic batch apply first | **Recommend (a)** for justify-use; implement commit as internal batch; expose `apply_batch` P2 for substrate. |
| **O5** | **Snapshot epoch** | (a) at `begin` (b) at first read | **Recommend (a)**; FDB-like; simpler tests. |
| **O6** | **LSM strategy day one** | (a) simple correct first (b) Lazy Leveling now | **Recommend (a)**; don’t sled-research-block P0. |
| **O7** | **Value log (WiscKey)** | (a) later P2 (b) early | **Recommend (a)**; Badger/Titan GC is the regret. |
| **O8** | **Max key/value size** | hard limits vs soft | Proposal: key 64KiB hard; value soft large; TX buffer cap for memory — see deep dive. |
| **O9** | **Language bindings** | Rust only vs C ABI later | **Rust only** until P2+. |
| **O10** | **Name “PedraDB”** | keep vs rename | Keep; message “library kernel” in README. |

**Author bias to challenge:** O1 (sync default may be too slow for “fast” pitch), O2 (single-writer may feel “not serious”), O3 (no keyspaces may lose fjall users who want CF isolation).

---

## Comparison snapshot (non-normative)

| | PedraDB (this RFC) | fjall | redb | RocksDB |
|--|-------------------|-------|------|---------|
| Multi-node | No | No | No | No |
| Multi-process same path | No | No | No | No (normal use) |
| TX multi-key | **Core** | Optional | Yes (B-tree) | No (core) |
| Structure | LSM | LSM | B-tree | LSM |
| Surface | Minimal TX | Richer embed kit | Small ACID | Huge |
| Maturity | Early | Shipped 3.x | Stable | Industry |

If PedraDB grows to fjall’s surface without better TX/substrate story → **use fjall instead**.

---

## Risks

| Risk | Mitigation |
|------|------------|
| Becomes worse fjall | Enforce surface filter; RFC non-goals |
| Sled-like eternal rewrite | P0 ships useful TX; freeze small API |
| Strong fsync default kills “fast” | Measure; optional relaxed commit after P0 |
| OCC too hard for P0 | Start single-writer TX |
| Research distraction | P2 only with benches |

---

## Amendment process

- Changing **Locked** decisions or **Out of scope** requires updating this RFC status notes and Status table.  
- Shipping code for a slice updates checkbox + Status table **in the same commit**.

---

## Appendix A — Glossary

| Term | Meaning |
|------|---------|
| **Local** | One OS process, one data directory |
| **Layer** | Code using only public TX API to implement indexes/models |
| **Substrate** | Library role under an app or multi-node product |
| **OCC** | Optimistic concurrency control — commit may abort on conflict |

## Appendix B — Minimal API sketch (informative, not final signatures)

```rust
// Informative only — exact types evolve in implementation RFCs / code.

pub struct Db { /* ... */ }
pub struct Transaction<'db> { /* ... */ }

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;
    pub fn begin(&self) -> Transaction<'_>;
}

impl Transaction<'_> {
    pub fn get(&self, key: &[u8]) -> Result<Option<bytes::Bytes>>;
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()>;
    pub fn delete(&mut self, key: &[u8]) -> Result<()>;
    // range: exact iterator API TBD
    pub fn commit(self) -> Result<(), CommitError>; // includes Conflict
    pub fn abort(self);
}
```
