# SQL lessons for the grail: Postgres, MySQL, TiDB/CRDB, and the newer split-storage / proxy-shard architectures

> Two already-covered families (Percolator-on-RocksDB and Parallel Commits/HLC)
> are **not** repeated here in depth — see `tidb-architecture.md` and
> `distribution-deep-research.md`. This doc adds the two monoliths PedraDB
> ultimately wants to enable (Postgres, MySQL) and the architectures that
> emerged **after** TiDB/CockroachDB solved "SQL layer + bolt-on TX KV":
> log-is-the-database storage/compute split (Aurora, Neon), proxy-and-shard
> unmodified engines (Vitess, Citus), and TrueTime (Spanner). The payoff is
> §7 — cross-cutting lessons for the grail ladder in
> `grail-plan-build-databases-on-pedradb.md`.

Primary sources (fetched 2026-08-11 unless noted):
- [Neon / Databricks Lakebase architecture overview](https://neon.com/docs/introduction/architecture-overview) (Neon was acquired by Databricks in 2025; this doc is the current, maintained descendant of Neon's own architecture docs)
- [Google Cloud Spanner — TrueTime and external consistency](https://docs.cloud.google.com/spanner/docs/true-time-external-consistency)
- [Vitess docs — VTGate concept](https://vitess.io/docs/22.0/concepts/vtgate/)
- [Citus docs — What is Citus](https://docs.citusdata.com/en/stable/get_started/what_is_citus.html)
- Amazon Aurora: **secondary** sources only this session (AWS Database Blog deep-dive, paper-notes summaries of the 2017 SIGMOD paper *"Amazon Aurora: Design Considerations for High Throughput Cloud-Native Relational Databases"*) — the primary paper itself was not fetched; treat Aurora claims below as corroborated-by-multiple-summaries, not primary-source-verified, until the paper is read directly
- Prior in-repo: `distribution-deep-research.md` (Percolator, Parallel Commits, PD, TSO, HLC), `tidb-architecture.md` (TiDB/TiKV key encoding, SQL-as-layer)

---

## TL;DR

| System | What's split from what | Distribution primitive | TX story | One-line lesson |
|---|---|---|---|---|
| **Postgres** | Nothing (monolith) | WAL (physical + logical) | Native SSI, single writer | The WAL is *already* the interface everything else reads — design it as a first-class artifact, not an implementation detail |
| **MySQL/InnoDB** | Nothing (monolith) | Binlog | RR + gap locks, single writer (Group Replication mostly abandoned) | Clustered index (secondary index stores PK) is the single-node ancestor of TiDB's key encoding |
| **TiDB/TiKV** | SQL vs KV+TX | Multi-Raft log + Percolator 2PC | Bolt-on TX on RocksDB | *(see tidb-architecture.md)* |
| **CockroachDB** | SQL+KV fused, storage swappable (Pebble) | Multi-Raft + Parallel Commits + HLC | Native, but built after the fact on Pebble | *(see distribution-deep-research.md)* |
| **Amazon Aurora** | Compute (engine) vs storage (redo-log fleet) | **Redo log records only** — "the log is the database" | Engine-native (unmodified-ish MySQL/Postgres); storage just replays | Ship the *minimal* WAL delta, not whole KV mutations; let storage nodes materialize independently |
| **Neon / Lakebase** | Compute (stateless PG) vs Safekeepers (WAL durability) vs Pageserver (versioned pages) vs object storage (cold) | Paxos-quorum WAL + LSM-based versioned page store | Unmodified Postgres engine | The "Pageserver" role — LSM store answering "page X as of LSN V" — **is** what PedraDB already wants to be |
| **Vitess** | Query routing (VTGate) vs storage (VTTablet→MySQL) | Sharding key + app-level 2PC | Unmodified MySQL per shard | Horizontal scale without touching the engine at all — costs you live resharding |
| **Citus** | Coordinator vs worker Postgres | Fixed shard count + 2PC | Unmodified Postgres per worker | Same trade as Vitess, for Postgres; explicitly *not* a drop-in for all workloads (their own docs say so) |
| **Spanner** | Nothing new vs TiDB/CRDB structurally — different **clock** | TrueTime (bounded uncertainty, GPS/atomic clocks) + commit-wait | Native 2PC/Paxos | A third way to get global order, requiring hardware nobody but hyperscalers has |
| **Turso/libSQL** | Primary (SQLite) vs edge read replicas | Async WAL frame shipping | Single writer | Cheap HA/read-scale rung that needs **no consensus at all** |

---

## 1. Monoliths still teach real lessons

### Postgres

- **The WAL is the actual product interface.** Streaming replication, logical
  decoding (the primitive behind CDC/Debezium), and PITR are all just WAL
  readers — Postgres never had to build a second "replication protocol";
  it exposed the log it already had. **Lesson:** PedraDB's WAL must be
  designed as a versioned, self-describing, independently-replayable
  artifact from day 0. Every rung above the kernel (Raft apply, follower
  replicas, CDC, a future Neon-style page service) will want to read it —
  retrofitting that later means changing the on-disk format under existing
  users.
- **Real SSI (serializable snapshot isolation)** is reachable on a single
  node with predicate locking over MVCC snapshots — no global clock
  service required. Confirms PedraDB's own target (strict serializable,
  achieved locally via OCC/locks) doesn't need Spanner-grade infrastructure
  at Rung 0–2.
- **Vacuum is the unpaid tax of MVCC.** Thirty years in, vacuum tuning is
  still a top operational pain point in production Postgres. **Lesson:**
  bounded-history GC (grail plan §6, "Should: snapshot/seq export"; Horizon
  B3, "GC of old versions") needs to be a first-class kernel job designed
  in from the start, not an afterthought bolted on when users complain.
- **Extensions (Citus, TimescaleDB, pg_partman, FDWs)** prove a good
  hook API lets outsiders build huge functionality without forking core —
  the same idea as PedraDB's "layers," but the Postgres-specific lesson is
  that the hook surface (custom scan nodes, FDW, logical-decoding output
  plugins) has to be *decided deliberately*, or every layer reinvents its
  own hooks. PedraDB's "tiny API" goal (grail §6) is the right instinct;
  it just needs to include the *few* hooks layers actually reuse (apply,
  snapshot export, prefix/subspace) rather than none.

### MySQL / InnoDB

- **Clustered index**: secondary indexes store the **primary key**, not a
  physical row pointer — so reorganizing/splitting a table never
  invalidates a secondary index's target. This is the single-node ancestor
  of the TiDB-style `t{table}_r{PK}` / `t{table}_i{idx}_{val}_{PK}`
  encoding the grail plan already commits to (§9; `tidb-architecture.md`).
  Confirms it, doesn't change it.
- **Binlog + (semi-)sync replication** gave the entire industry ~15 years
  of "good enough HA" before Raft-based systems existed — Facebook, GitHub,
  and later Vitess/PlanetScale scaled MySQL for years on nothing more
  exotic than binlog shipping plus a sharding proxy. **Lesson:** a
  WAL-shipping, single-leader mode is a legitimate, cheap intermediate step
  — not a toy — worth its own rung (see §6, proposed Rung 1.5) before
  paying for multi-Raft.
- **Group Replication** (MySQL's multi-primary mode) is rarely used in
  production for the same reason the grail plan already excludes
  Scylla-style multi-master: row-level conflict resolution under real
  contention is a tax nobody wants to keep paying twice. This is an
  *independent* data point for the grail's existing single-writer-per-key-range
  stance — it's not just "Scylla is a different animal," even MySQL's own
  multi-primary experiment lost to single-writer-per-shard in practice.

---

## 2. Storage/compute split: Aurora and Neon — the log *is* the distribution primitive

This is the freshest lesson relative to what's already in the repo (TiDB/CRDB
are "SQL layer over a distributed transactional KV"; Aurora/Neon are a
different axis entirely: "keep the *SQL engine* single-node, distribute only
the *storage*").

### Amazon Aurora ("the log is the database")

- Compute nodes run an (adapted) MySQL/PostgreSQL engine but write **only
  redo log records** to storage — not whole pages, not KV batches.
- Storage is a separate multi-tenant fleet: each storage node receives log
  records for the segments it owns, applies them to materialize pages
  **asynchronously and independently**, and does its own crash recovery by
  replaying in parallel, on demand.
- No compute-side checkpointing is needed — checkpoint-equivalent work moves
  to the continuous, distributed storage layer.
- Contrast with TiKV/CockroachDB: those replicate whole KV mutations via
  Raft and apply them **identically** on every replica. Aurora ships a
  *smaller* unit (the log delta) and lets each storage node materialize
  independently — less network traffic, more storage-side CPU, and a
  fundamentally different failure/repair story (storage self-heals segments,
  not whole replicas).

### Neon (architecture now maintained under Databricks "Lakebase" post-acquisition)

Fetched directly this session — the split is cleaner than Aurora's:

- **Safekeepers**: durability only. Paxos-quorum-acknowledge WAL records;
  a transaction commits once a quorum of Safekeepers has the record — no
  local fsync dependency, no page logic at all.
- **Pageserver**: turns WAL into queryable pages. Given a page + an LSN, it
  either serves a cached materialized version or reconstructs it by
  replaying WAL up to that LSN, from its own **LSM-based, versioned
  page store**.
- **Object storage**: cold historical snapshots, never on the read path —
  only consulted when the Pageserver needs to reconstruct something not in
  its local cache.
- **Compute**: unmodified, disposable, stateless Postgres. It exists to
  execute queries, not to preserve data; it can be killed and restarted
  with zero data loss because it owns nothing durable.
- **Branching**: copy-on-write over the Pageserver's versioned LSM — a full
  database "clone" is a metadata pointer, not a data copy.

**Why this matters for PedraDB specifically:** the Pageserver's actual job —
*"an LSM-based, versioned store that answers 'give me key/page X as of
version V', fed by a replayable log"* — is close to a direct description of
what PedraDB already wants to be at Rung 0–1. This exposes a **third**
strategy for "Postgres-compatible, horizontally scalable," distinct from
grail's Recipe P (§9 of the grail plan: new SQL layer + new distributed KV,
TiDB/CRDB-shaped):

| Strategy | Example | What changes | What stays | Cost |
|---|---|---|---|---|
| **(a) New SQL layer + new distributed KV** | TiDB, CockroachDB — grail's Recipe P | Everything (parser, planner, storage, distribution) | Wire compatibility only, maybe | Years; full control |
| **(b) Swap the storage engine under an *unmodified* SQL engine** | Aurora, Neon | Storage/durability layer only, via the engine's existing WAL/smgr hook | Postgres's parser, planner, executor, extension ecosystem, license | Much smaller surface than (a); couples you permanently to that engine's execution model and licensing |
| **(c) Proxy + shard unmodified single-node engines** | Vitess, Citus (§3) | Nothing engine-internal; add a routing + 2PC layer in front | Whole engine, unmodified, per shard | Fastest to ship; weakest live-rebalance story |

Grail already, correctly, commits to (a) for full control over the whole
stack (own SQL surface, own storage, no upstream license/roadmap
dependency). But (b) is a **real, much cheaper detour** if the near-term
goal ever becomes "Postgres-wire-compatible, horizontally readable/scalable,
fast" rather than "own the entire stack" — worth naming explicitly as a
considered-and-not-taken alternative rather than leaving it as a blind spot
(see proposed decision-log addition, §8 below).

---

## 3. Proxy-and-shard: Vitess and Citus — distribution without touching the engine at all

### Vitess (VTGate / VTTablet)

Confirmed from Vitess's own docs:

- **VTGate**: a stateless proxy that speaks both the MySQL wire protocol
  and Vitess gRPC. It "routes traffic to the correct VTTablet servers and
  returns consolidated results back to the client," choosing targets by
  sharding strategy, latency, table availability, and tablet health.
  Applications connect to VTGate as if it *were* a single MySQL server.
- **VTTablet**: one per actual, unmodified MySQL instance. Manages that
  instance's lifecycle, connection pooling, and query rewriting/validation,
  and participates in Vitess's own cross-shard transaction coordination
  when a query spans more than one shard.
- **Topology Service**: the PD-equivalent — cluster metadata (keyspace →
  shard → tablet mapping).

**Lesson:** this is the cheapest possible version of "Rung 5" — no new
storage engine, no new SQL engine, just a routing + 2PC layer in front of
N independently-running single-node engines. It buys real horizontal write
throughput (many independent MySQLs, each with its own single writer) at
the cost of losing the online-Region-split story: resharding in Vitess is
an explicit, operator-driven workflow (`Reshard`/`MoveTables` actions), not
a live Raft rebalance the way TiKV/CockroachDB Regions split and move
automatically under load. That's the real price paid instead of "invent
Percolator/Parallel Commits."

### Citus

- Postgres **extension**, not a rewrite: a coordinator node plus N worker
  Postgres nodes. Tables are distributed by a shard key into a **fixed**
  number of shards, each a normal Postgres table on a worker. The
  coordinator plans and parallelizes queries across workers and runs 2PC
  for cross-shard writes.
- Citus's own docs are explicit about the trade-off: it "extends
  PostgreSQL with distributed functionality, but it is not a drop-in
  replacement that scales out all workloads" — certain query shapes (e.g.
  data-heavy ETL producing large result sets rather than summaries) degrade
  badly.

**Lesson:** same family as Vitess, for Postgres instead of MySQL — a second
independent confirmation that "proxy + shard the existing single-node
engine" is a real, shippable strategy with the same structural trade-off:
a shard count fixed at distribution time, and a much weaker live-rebalance
story than a Region/Raft model, in exchange for reusing 100% of an
existing, mature SQL engine.

---

## 4. Spanner: TrueTime — a third way to answer "who assigns transaction order"

TrueTime is a bounded clock-uncertainty API — every call returns an
*interval* `[earliest, latest]`, not a point in time — backed by GPS and
atomic clock hardware Google runs in its own datacenters. Spanner assigns
commit timestamps from TrueTime and does **commit-wait**: it delays
acknowledging a commit until real time has definitely advanced past the
upper bound of that timestamp's uncertainty interval. The result is
external consistency (linearizability across the whole system) without a
single centralized sequencer.

This is a genuinely different axis from what `distribution-deep-research.md`
already covers (PD's TSO, CockroachDB's HLC) — it's worth stating the three
options side by side, since grail's Rung 3 will need to pick one:

| Approach | Who does it | Mechanism | Cost |
|---|---|---|---|
| **Centralized TSO** | TiDB/PD | One service hands out the next timestamp on request | Simplest to reason about; a scaling bottleneck / SPOF you must engineer around separately |
| **HLC (hybrid logical clock)** | CockroachDB | Each node keeps a clock nudged by message timestamps; no dedicated service | No extra infra; weaker bound → needs uncertainty-interval **retries** on read/write conflicts near the bound |
| **TrueTime** | Spanner | Bounded-uncertainty interval from real atomic-clock/GPS hardware in every datacenter | Strongest guarantee, **zero retries** for the uncertainty window — but requires hardware infrastructure nobody outside a hyperscaler has |

**Lesson for the grail:** HLC is the only realistic starting point for
Rung 3 (matches `distribution-deep-research.md`'s existing recommendation);
this doc adds the explicit *reason* to rule out TrueTime — it's not that
it's architecturally wrong, it's that it needs hardware PedraDB will never
have access to. A PD-style centralized TSO is a reasonable *upgrade* once
there's already a control-plane service for other reasons (grail's
`pedra-pd`, `tidb-architecture.md`'s mapping) — but it shouldn't be the
Rung 3 starting point given the "small, correct, embeddable" ethos.

---

## 5. Edge/embedded-replica: Turso / libSQL

A SQLite fork adding **embedded read replicas**: writes go to a single
primary, WAL frames ship asynchronously to replicas (often at the edge),
which serve local reads with no consensus round-trip at all.

**Lesson:** this is "Rung 1 (pure local embed) + async WAL shipping to
followers," with **no Raft, no SQL-layer rewrite, no quorum**. It's a
legitimate, cheap rung sitting *between* grail's Rung 1 and Rung 3 — see
the proposed Rung 1.5 below.

---

## 6. Cross-cutting lessons for the grail ladder

| Pattern seen across (almost) every system above | Who does it | What it implies for PedraDB |
|---|---|---|
| The storage engine stays dumb & stable; all product innovation happens in the layer above it | RocksDB/TiKV, Pebble/CockroachDB, storage-nodes/Aurora, Pageserver/Neon, unmodified InnoDB+Postgres/Vitess+Citus | Confirms grail §2's own framing — PedraDB's job is to be the boring, correct, fast kernel; never chase the layer's feature race in-core |
| The WAL/log is the actual distribution primitive, not an implementation detail | Aurora ("the log is the database"), Neon (Safekeepers = pure WAL durability), any Raft system (log entries ARE the replication unit), Postgres (physical + logical replication both just read the WAL) | Promote "snapshot / seq-number export" from grail §6's **Should** to a **Must** — every rung above the kernel (Raft apply, follower replicas, CDC, a future Pageserver-style product) needs to treat PedraDB's WAL as a first-class, addressable, replayable artifact from day 1 |
| Ordered-key-per-row + ordered-key-per-index-entry, updated in one transaction | TiDB/TiKV, CockroachDB, Spanner, (InnoDB's clustered index is the single-node ancestor) | Zero controversy across every serious system studied — grail §9's key-encoding plan for Rung 5 is aimed correctly; nothing to change |
| "Make SQL horizontally scalable" has (at least) three independently-viable strategies, not one | (a) new SQL + new distributed KV (TiDB/CRDB) (b) swap storage under an unmodified SQL engine (Aurora/Neon) (c) proxy + shard unmodified engines (Vitess/Citus) | Grail correctly commits to (a) for full-stack control (Recipe P) — but (b) and (c) are real, much cheaper, and should be named as *considered and declined* rather than left as blind spots |
| Multi-primary/multi-master writes on the same row keep losing to single-writer-per-shard in production | MySQL Group Replication (rarely used) — independently of Scylla/Cassandra AP-LWW, which grail already excludes for pillar reasons | A second, unrelated data point for grail's existing single-writer-per-key-range stance — not just "Scylla is different," even MySQL's own multi-primary mode lost |
| A missing rung: single-writer + WAL-shipped read replicas, no consensus at all | Turso/libSQL, MySQL binlog replication, Postgres streaming replication | Add explicit **Rung 1.5** to the ladder: cheap HA/read-scale win, reuses the same WAL-export primitive the whole rest of this table depends on, ships **before** Rung 3's multi-Raft investment |
| Someone always needs an order/placement authority once you distribute | PD/TSO (TiDB), HLC (CockroachDB), TrueTime (Spanner), Topology Service (Vitess) | Grail's Rung 3 needs one too. Start with HLC — cheapest, no dedicated service, matches PedraDB's ethos; TrueTime is ruled out on infrastructure grounds, not architecture grounds |

---

## 7. Changes applied to the grail plan

These findings were **folded into** `grail-plan-build-databases-on-pedradb.md`
(2026-08-11). Status:

| # | Change | Applied? |
|---|--------|----------|
| 1 | Promote snapshot/seq export to **Must** (§6) | Yes |
| 2 | Insert **Rung 1.5** WAL-shipped replicas (§5) | Yes |
| 3 | Decision log: strategies (b)/(c) named+declined; HLC first | Yes |
| 4 | Object storage for Rung 1.5 export | Open row in §12 + `object-storage-as-substrate-possibility.md` |
| 5 | Short-term conflict matrix | `conversation-learnings-and-short-term-alignment.md` |
| 6 | TiDB vs PG/MySQL table | `tidb-vs-postgres-mysql.md` |
| 7 | RFC-0001 **P1.6** WAL seek/export | Yes (not a P0 blocker) |

---

## Sources

| Ref | Source | Kind |
|---|---|---|
| [Neon/Lakebase] | neon.com/docs/introduction/architecture-overview | Primary (fetched 2026-08-11) |
| [Spanner TrueTime] | docs.cloud.google.com/spanner/docs/true-time-external-consistency | Primary (fetched 2026-08-11) |
| [Vitess VTGate] | vitess.io/docs/22.0/concepts/vtgate/ | Primary (fetched 2026-08-11) |
| [Citus] | docs.citusdata.com/en/stable/get_started/what_is_citus.html | Primary (fetched 2026-08-11) |
| [Aurora] | AWS Database Blog deep-dive; paper-notes summaries of the 2017 SIGMOD paper | **Secondary** — primary paper not fetched this session |
| [TiDB/TiKV] | `tidb-architecture.md` (already primary-sourced from PingCAP docs) | In-repo |
| [CockroachDB/Percolator/PD/TSO] | `distribution-deep-research.md` | In-repo |
