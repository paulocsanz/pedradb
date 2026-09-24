# Conversation learnings → durable docs, and short-term alignment

**Status:** living synthesis of research/session conclusions  
**Updated:** 2026-08-11  
**Normative product spec:** `rfc/0001-pedradb-high-level-spec.md`  
**Purpose:** One place that (1) indexes what this conversation produced, (2) states what was locked vs left open, and (3) checks **conflicts with short-term PedraDB (P0)**.

---

## 1. Doc map (what landed where)

| Learning | Durable home |
|----------|----------------|
| Product = local ordered KV + multi-key ACID; no multi-node in core | RFC-0001, `positioning.md`, `architecture-refined.md` |
| P0.1 WAL done; P0.2 InternalKey+MemTable done | RFC-0001 status, RFC-0002, code under `crates/pedradb-core` |
| O1 durability default = sync; O2 single-writer TX for P0 | RFC-0001 open table (locked for P0) |
| Scylla **product** off main line; Scylla **need** (routes/orch CP) on main line as multi-Raft+watch | `scylla-need-replacement.md`, grail §10, plug-map |
| TiDB vs Postgres vs MySQL monoliths | **`tidb-vs-postgres-mysql.md`** (this session) |
| Lessons from PG/MySQL + Aurora/Neon/Vitess/Citus/Spanner/Turso | `sql-lessons-for-the-grail.md` |
| Rung 1.5 WAL-shipped replicas; WAL export promoted to Must | grail plan §5–§6, §12 |
| PedraDB can be the “pageserver materializer” (seq + snapshot get); network is outer | sql-lessons + this doc §3 |
| Object storage as kernel medium = no; as Rung 1.5 export / speculative cold tier = open | `object-storage-as-substrate-possibility.md`, positioning non-goal footnote |
| NATS: JetStream *need* (durable log) replaceable; Core pub/sub is not a DB job; Jepsen 2.12.1 | `nats-need-replacement.md` |
| Plug points for etcd/Patroni/SQLite/PG/TiKV/TiDB | `plug-map-replace-incumbents.md`, `pedradb-as-dcs-storage-for-patroni.md` |
| Switch bar / upsides / limitations | `switch-justification-bar.md`, `upsides-only.md`, `plan-limitations-and-failure-modes.md` |
| Doctrine: primitive + API layers only | `doctrine-primitives-and-api-layers.md` |

---

## 2. Locked for the grail (not reopened lightly)

| ID | Decision |
|----|----------|
| L1 | PedraDB = **local library only** |
| L2 | Identity = **multi-key ACID + ordered KV**, tiny API |
| L3 | LSM family; TX in core (not bolt-on later) |
| L4 | P0 durability default **sync** (DataSync); optional async later |
| L5 | P0 write concurrency = **single-writer TX** |
| L6 | Scylla **AP multi-master product** not kernel goal |
| L7 | Scylla-shaped **control-plane jobs** solvable with CP multi-Raft + watches |
| L8 | Horizontal SQL Recipe **P** (own stack) committed; (b) Aurora/Neon-under-PG and (c) Vitess/Citus named and declined as primary path |
| L9 | Rung 3 clock: **HLC first**; TrueTime out (hardware) |
| L10 | Object-store-**first kernel** stays non-goal |

---

## 3. Open (explicitly not decided)

| Topic | Status |
|-------|--------|
| Object storage as **WAL export medium** for Rung 1.5 | Open possibility — see object-storage doc |
| PedraDB as local cache in front of object truth | Speculative; needs SST first |
| First upper product after P0 (embed vs etcd-class vs …) | Not picked |
| OCC in P1 vs stay single-writer longer | Open after P0 |
| Full PG wire vs MySQL wire for Rung 5 | Product choice later |

---

## 4. Short-term plan (P0) — current

From RFC-0001 (authoritative for delivery):

| ID | Slice | Status |
|----|-------|--------|
| P0.1 | WAL append + recovery | **done** |
| P0.2 | InternalKey + MemTable + seqnums | **done** |
| P0.3 | WAL → MemTable recover; basic get/put | **next** |
| P0.4 | Public multi-key `Transaction` (single-writer) | todo |
| P0.5 | Commit durability + crash test | todo |
| P0.6 | Minimal usage + index-layer sketch docs | todo |

P0 **excludes:** SST, compaction, network, multi-Raft, SQL, object storage, Scylla gateway, full WAL streaming API.

---

## 5. Conflict check: research vs short-term P0

### 5.1 Summary

| Research push | Conflicts with P0? | Resolution |
|---------------|--------------------|------------|
| WAL must be first-class / addressable (Aurora/Neon) | **No hard conflict** | Elevates **design quality** of WAL records in P0.3 and a **P1/P2 API** (seek by offset/seq). Does **not** block finishing interactive TX. |
| Rung 1.5 WAL-shipped replicas | **No** | Outer product after kernel works. Same primitive as P0.3 replay. |
| `apply_batch` for Raft/log apply | **No** | Already RFC P2 / grail Should; implement after P0 TX. |
| Scylla need replacement architecture | **No** | Multi-Raft + watch — years above P0. |
| Object storage substrate | **No** | Kernel exclusion confirmed; export medium open and **post-P0**. |
| NATS / JetStream replacement | **No** | JetStream-class stream is outer multi-Raft product; Core NATS is not a DB. Strengthens O1 sync default. |
| Horizontal SQL / Recipe P | **No** | Rung 5; not P0. |
| TiDB vs PG/MySQL lessons | **No** | Confirms TX-in-core and “don’t put SQL wire in kernel.” |
| Promote snapshot/seq export to Must | **Soft tension only** | Must for **grail completeness**, not for “P0 justify-use demo.” Ship minimal seq in-process for P0; **public export API** in P1+ without rewriting P0. |
| Pageserver role (materialize log → versioned get) | **Aligns** | P0.2 already has seq + snapshot get; P0.3 wires WAL→MemTable — **exactly** the next step. |

### 5.2 What would be a real conflict (we are **not** doing these)

| Bad pivot | Why wrong now |
|-----------|----------------|
| Stop P0 TX to build object-store LSM | Latency niche ≠ PedraDB pitch; SlateDB exists |
| Put multi-Raft or gRPC in pedradb-core for P0 | Contradicts L1; sled-scale surface |
| CQL / multi-master for “Scylla parity” | Wrong physics |
| Unmodified Postgres storage swap as P0 | Different product (strategy b), not kernel |
| Expand public API to full CDC/WAL stream before `commit` works | Violates justify-use-first |

### 5.3 Ordered backlog that **absorbs** research without derailing P0

```
NOW   P0.3  WAL record payload + recover → MemTable + basic get/put
      P0.4  Transaction API (single-writer)
      P0.5  Durable commit + crash test
      P0.6  Tiny docs

P1    SST flush; merged get/range
      WAL addressable read from offset/seq (export primitive — Must for grail)
      Benches

P2    apply_batch (ordered apply, no OCC)
      Group commit; optional async commit
      Sim / oracle

LATER Rung 1.5 product (WAL ship to followers; optional object export)
      multi-Raft + watches (etcd / Scylla-need / TiKV-class)
      SQL layers
```

**Key insight already in code (not only docs):**  
`SequenceNumber` ≈ LSN; `MemTable::get(key, snapshot)` ≈ point-in-time materialization; WAL is opaque bytes. Closing P0.3 is both “justify use” **and** the Aurora/Neon primitive path — no roadmap rewrite required.

---

## 6. Near-term plan updates (grail §13 refined)

1. **Finish P0** as in RFC-0001 — no new P0 slices from this research.  
2. When designing **P0.3 WAL payloads**, prefer self-describing records (seq, type, key, value) so P1 export doesn’t force a format break.  
3. Schedule **WAL seek/export** as first-class **P1** (not buried only in “P2 substrate polish”).  
4. Keep **apply_batch** as P2 unless a real outer consumer appears sooner.  
5. Leave object storage / Scylla CP / SQL horizontal as **documented future**, not active implementation tracks until P0 ships.  
6. First product above kernel still **not chosen** — embed vs etcd-class remains a post-P0 product RFC.

---

## 7. One-page “are we still right?”

| Claim | Still true after this conversation? |
|-------|-------------------------------------|
| Local TX kernel first | **Yes** — PG/MySQL and TiDB both reinforce it from opposite sides |
| Multi-node not in this repo | **Yes** — Aurora/Neon/Vitess are outer products |
| Justify use before sim/papers | **Yes** |
| WAL quality matters more than we thought | **Yes** — promoted, but still **after** basic TX works if forced to sequence; in practice P0.3 **is** WAL quality |
| Object store as default disk for PedraDB | **No** — still wrong for kernel latency |
| Need Scylla for CP networking | **Replaceable** by multi-Raft+watch architecture (doc’d), not by CQL |

---

## 8. Related reading order

1. RFC-0001 + RFC-0002 (what we ship)  
2. This doc (alignment)  
3. `tidb-vs-postgres-mysql.md` + `sql-lessons-for-the-grail.md` (SQL world)  
4. `scylla-need-replacement.md` (control plane scale-out)  
5. `object-storage-as-substrate-possibility.md` (open S3 path)  
6. grail plan (ladder + decision log)
