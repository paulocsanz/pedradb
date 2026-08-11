# Distribution deep research: primary-source findings

> Deep dive into how production systems turn an embedded KV into a distributed
> database. Sourced from primary papers and official docs (fetched 2026-08-10).
> Complements `distribution-design.md` with protocol-level detail.

**Primary sources persisted:**
- `docs/references/percolator-osdi2010.pdf` + `.txt` — Google Percolator (OSDI'10)
- Official docs: TiKV/TiDB, CockroachDB, FoundationDB, etcd, openraft, raft-rs

---

## 1. Percolator: the foundational distributed TX protocol

**Paper:** Peng & Dabek, "Large-scale Incremental Processing Using Distributed
Transactions and Notifications", OSDI 2010. Used to build Google's live web
search index over Bigtable.

### What it gives you

- Cross-row, cross-table **ACID transactions** with **snapshot isolation**
- Built as a **client library on top of Bigtable** (no central lock manager)
- Scales to **thousands of machines**, multi-PB repositories
- Designed for **throughput over latency** (lock cleanup can take tens of
  seconds after a crash — acceptable for indexing, not for OLTP)

### Snapshot isolation (not serializable)

Percolator provides SI, **not** full serializability:

- Each transaction reads at a `start_ts` (snapshot) and writes at a later
  `commit_ts`.
- **Write-write conflicts** are detected and abort one side.
- **Write skew** is possible (SI anomaly) — two concurrent transactions can
  each write different cells that together violate an invariant.
- Advantage of SI over serializable: reads need **no locks** — just a lookup
  at the snapshot timestamp.

**Implication for PedraDB:** our embedded core already targets stronger
isolation (OCC serializable / SSI). The distributed layer can still use
Percolator-style 2PC for *coordination*, while local conflict detection
enforces the stronger isolation.

### Metadata columns (how locks live in the KV)

For each user column `c`, Percolator stores sidecar metadata in Bigtable:

| Column | Role |
|--------|------|
| `c:data` | Actual value (at start timestamp) |
| `c:lock` | Uncommitted write lock; secondary locks point at primary |
| `c:write` | Commit record: points to data timestamp when committed |
| `c:notify` | Observer dirty-bit (Percolator-specific, not needed for PedraDB) |

Locks are **persisted in the same store as data**, inside single-row
transactions Bigtable already provides. There is no separate lock service.

### Protocol (client-coordinated 2PC)

```
1. BEGIN
   - Get start_ts from timestamp oracle
   - Buffer writes in client memory

2. PREWRITE (for every key written)
   - Check for write-write conflict: any write after start_ts → ABORT
   - Check for existing lock at any timestamp → ABORT
   - Write lock + data at start_ts
   - One key is designated PRIMARY; others are secondary
     (secondary lock stores pointer to primary)

3. COMMIT
   - Get commit_ts from timestamp oracle (commit_ts > start_ts)
   - Clear primary lock, write commit record at commit_ts
     (pointer: "data lives at start_ts")
   - Asynchronously clear secondary locks / write secondary commit records

4. CRASH RECOVERY (lazy)
   - Any transaction that finds a lock looks up the primary
   - If primary has commit record → secondary is committed, clean up
   - If primary lock expired / missing → abort secondary
   - No global deadlock detector; lock cleanup is opportunistic
```

### Timestamp oracle

Strictly increasing timestamps. Required for SI correctness. In Google's
deployment this is a small replicated service. In TiDB, PD serves TSO.

### Why TiKV/TiDB adopted Percolator

- **Decentralized:** no central transaction manager (locks live in TiKV Regions)
- **Fits multi-Raft:** each prewrite is a Raft write to the Region that owns
  the key
- **Single-row atomicity** from Raft is enough to implement multi-row TX on top
- Scales by adding Regions, not by scaling a central resolver

### Limits called out in the paper (still relevant)

1. **Latency:** 2PC + multi-RPC; lock cleanup after crash can delay commits
2. **No global deadlock detection** (increases latency of conflicting TX)
3. **Snapshot isolation only** (write skew)
4. **Not designed for OLTP latencies** — built for indexing pipelines

TiKV addressed (1)–(3) over years: pipelined locking, in-memory locks,
pessimistic mode default since v3.0.8, and SI+ for practical isolation.

---

## 2. TiKV/TiDB: Percolator adapted for OLTP

### Optimistic transactions (classic Percolator)

From [TiDB Optimistic Transaction docs](https://docs.pingcap.com/tidb/stable/optimistic-transaction/):

```
BEGIN
  → PD allocates start_ts (TSO)
READS
  → route via PD Region map → TiKV at start_ts snapshot
WRITES
  → buffered in TiDB memory (not yet durable)
COMMIT
  → 2PC:
     1. Choose primary key
     2. Group keys by Region
     3. PREWRITE all Regions (lock + check conflicts in TiKV)
     4. Get commit_ts from PD
     5. COMMIT primary Region
     6. Async clean secondary locks
```

**Advantages:** simple, no lock wait on uncontended workloads.
**Disadvantages:** high abort rate under contention; auto-retry was disabled
because it breaks REPEATABLE READ (retry gets a new `start_ts`).

**Since TiDB v8.0.0:** optimistic auto-retry fully removed. Application must
retry. **Default mode since v3.0.8: pessimistic.**

### Pessimistic transactions (default)

From [TiDB Pessimistic Transaction docs](https://docs.pingcap.com/tidb/stable/pessimistic-transaction/):

- On each DML / `SELECT FOR UPDATE`, TiDB sends **Acquire Pessimistic Lock** to
  TiKV **before** 2PC.
- Locks wait up to `innodb_lock_wait_timeout` (default 50s).
- Commit still uses the same 2PC as optimistic.
- Isolation: **Repeatable Read** (default, MySQL-compatible) or **Read Committed**.

**Performance optimizations (critical for production):**

| Feature | What it does | Trade-off |
|---------|--------------|-----------|
| **Pipelined locking** | Return lock success to TiDB before Raft apply finishes; write lock async | Under partition, lock may be lost → correctness risk for apps that rely on lock wait |
| **In-memory pessimistic locks** (v6.0+) | Locks live only in Region leader memory, not Raft-replicated | Same: lock lost on leader fail/transfer unless spilled to disk |
| Memory limits | Spill to disk when Region/node lock memory exceeds threshold | Fallback to durable path |

**Open transactions do not block GC** — but `max-txn-ttl` defaults to **1 hour**.
Longer TX are aborted.

**No gap locking** (unlike InnoDB): concurrent inserts into a scanned range are
not blocked. Important semantic difference for SQL layers on PedraDB.

### TSO (Timestamp Oracle)

From [TiDB TSO docs](https://docs.pingcap.com/tidb/stable/tso/):

```
64-bit TSO:
  [ 46 bits physical (ms since epoch) | 18 bits logical counter ]
```

- PD allocates timestamps in batches (amortize RPC).
- Physical clock + logical counter handles clock regression and multi-timestamp
  within the same millisecond.
- Globally unique and monotonically increasing within the cluster.
- **Central bottleneck risk:** every transaction needs 2 TSO calls
  (start + commit). PD optimizes with batching and local caching.

**Implication for PedraDB:** if we adopt Percolator, we need a TSO service
(Raft-replicated counter, or PD-equivalent). HLC (CockroachDB) is the
decentralized alternative.

### Region scheduling (Placement Driver)

From [TiDB Scheduling docs](https://docs.pingcap.com/tidb/stable/tidb-scheduling/):

**PD collects:**
- Store heartbeats: disk free, Region count, read/write speed, snapshot
  traffic, labels, overload flag
- Region leader heartbeats: peer locations, offline replicas, load stats

**Store states:** Up → Disconnect (20s no heartbeat) → Down (after
`max-store-down-time`, default 30 min) → Offline (manual) → Tombstone.

**Three primitive operators:**
1. `AddReplica`
2. `RemoveReplica`
3. `TransferLeader`

**Goals (hard then soft):**
1. Correct replica count + topology-aware placement (e.g. one per rack/DC)
2. Even leader distribution, capacity balance, hot-spot balance
3. Rate-limited rebalancing so online traffic is not starved

**Learner path for moves:** add Learner → catch up → promote Follower →
optionally TransferLeader → remove old peer. Avoids availability blips.

**Implication for PedraDB:** a Placement Driver is mandatory for multi-Raft.
It is a separate component (like etcd/PD), not part of the storage engine.

---

## 3. CockroachDB: HLC + Parallel Commits

From [Transaction Layer docs](https://www.cockroachlabs.com/docs/stable/architecture/transaction-layer)
and [Parallel Commits blog](https://www.cockroachlabs.com/blog/parallel-commits/).

### Hybrid Logical Clocks (no central TSO)

```
HLC = (physical_wall_time, logical_counter)
```

- Each node advances its HLC on local events and on receiving messages
  (takes max of local and remote).
- Gateway node picks transaction timestamp from local HLC.
- **Max clock offset** enforced (nodes crash if offset exceeds bound —
  typically requiring NTP). Skew beyond bounds can break single-key
  linearizability between dependent transactions.
- Eliminates the TSO as a central bottleneck, at the cost of clock
  synchronization requirements.

### Write intents + transaction records

Similar spirit to Percolator, different vocabulary:

| Concept | CockroachDB | Percolator/TiKV |
|---------|-------------|-----------------|
| Provisional write | Write intent (value + pointer to txn record) | Lock + data at start_ts |
| Transaction state | PENDING / STAGING / COMMITTED / ABORTED | Primary lock + write record |
| Where state lives | Txn record on the range of first write | Primary lock column |
| Cleanup | Async resolve intents | Async clear secondary locks |

### Timestamp cache

Every read records its timestamp in a **timestamp cache** (per-range).
A later write that would land at or below a recently-read timestamp is
**pushed forward** (or aborts under SERIALIZABLE if refresh fails).
This is how CRDB enforces serializable isolation without a central resolver.

### Closed timestamps + follower reads

- Leaseholder continuously **closes** timestamps a few seconds in the past
  ("no writes will ever land below this").
- Closed timestamps piggyback on Raft commands to followers.
- Followers can serve **stale/bounded-staleness reads** at or below closed
  timestamp without contacting the leaseholder.
- Long-running TX may get timestamp-pushed by closed timestamp advancement
  → retry under SERIALIZABLE.

### Parallel Commits (latency breakthrough)

**Problem:** classic 2PC over Raft needs **two sequential consensus RTTs**:
1. Replicate all write intents
2. Replicate COMMITTED txn record

**Parallel Commits solution:**
1. Introduce txn record state **STAGING** = "all write keys known; in-flight"
2. Pipelined: write STAGING record **in parallel with** final intent writes
3. Transaction is considered **committed** when:
   - Record is STAGING, **and**
   - Observer can prove **all listed writes achieved consensus**
4. Client is ACKed after **one** parallel consensus RTT (≈ half the latency)

**Slow path (Transaction Status Recovery):** if an observer finds STAGING,
it checks each listed write. Missing writes → abort (timestamp cache prevents
the missing write from appearing later). Successful writes → treat as committed.

**Implication for PedraDB:** if we use multi-Raft + 2PC, Parallel Commits is
the known optimal for commit latency. Should be the target protocol, not
classic serial 2PC.

### Isolation levels

- **SERIALIZABLE** (default): full serializability via timestamp push + read
  refresh
- **READ COMMITTED** (optional): per-statement snapshots, statement-level
  retries, weaker

---

## 4. FoundationDB recovery model (contrast)

From FDB architecture docs (already in prior research):

- **Generational recovery:** master + GRV proxies + commit proxies + resolvers
  + transaction logs are one generation. Any failure of the write subsystem
  recruits a **full new generation**.
- **5-second MVCC window:** resolvers and storage servers only keep ~5s of
  mutation history → hard transaction timeout.
- **Ratekeeper** slows GRV (read version issuance) under load — backpressure
  at the timestamp allocation layer.
- **Strict serializability** via OCC with a global commit version assigned by
  the master.

**Not the path PedraDB should copy** for distribution (too specialized, hard
to reimplement), but its **simulation testing** and **strict serializability
goal** remain the gold standard.

---

## 5. etcd: single-Raft reference point

From [etcd API guarantees](https://etcd.io/docs/v3.5/learning/api_guarantees/):

- **Strict serializability + durability** for all KV APIs by default
- Linearizability via Raft; optional **serializable** (stale) reads for
  performance
- Single Raft group (or few) — **does not shard data**
- Watch is ordered by revision but **not linearizable**

**Lesson:** single-Raft is correct and simple, but does not scale write
throughput beyond one consensus group. Multi-Raft is required for scale.

---

## 6. Rust Raft libraries (production readiness)

### openraft (databendlabs)

- Production use: **Databend meta-service**
- Async, runtime-agnostic (tokio default; monoio/compio optional)
- Features: extended joint membership, redesigned Vote (fewer election
  conflicts), pluggable storage/network, turmoil-based deterministic fuzzer
- Performance claims (framework microbench, not full app):
  ~33k put/s single client, multi-million with batch
- **API not stable** (pre-1.0; 0.10 still alpha as of research date)
- Chaos testing "not yet completed" (their own status note)
- Unit coverage ~92%

### raft-rs / `raft` crate (tikv)

- Production use: **TiKV** (the gold-standard multi-Raft production system)
- Port of etcd's Raft core (consensus module only)
- **You must provide:** log storage, state machine, network transport
- Stable, battle-tested at TiKV scale
- More "library" than "framework" — more glue code required
- Uses protobuf (or prost) for messages

### Comparison for PedraDB

| Criterion | openraft | raft-rs (TiKV) |
|-----------|----------|----------------|
| Production pedigree | Databend meta | TiKV (massive) |
| Completeness | Higher-level framework | Core consensus only |
| Multi-Raft readiness | Build multi-group yourself | Same; TiKV shows the pattern |
| API stability | Pre-1.0, moving | More stable |
| Deterministic testing | turmoil fuzzer (strong) | External (TiKV's tests) |
| Async | First-class | You wire it |

**Recommendation (provisional):** start with **openraft** for development
speed and deterministic testing; revisit **raft-rs** if we need TiKV-level
proven multi-Raft patterns. Decision is open until distribution layer work
begins (post-Slice 7).

---

## 7. Consistency model spectrum (refined)

| System | Default isolation | Linearizable reads? | Multi-write? | TSO / clock |
|--------|-------------------|---------------------|--------------|-------------|
| PedraDB embedded (target) | Serializable (OCC) | N/A (single node) | N/A | Local seqnum |
| PedraDB distributed (target) | Strict serializable | Yes (leaseholder / ReadIndex) | Multi-Region leaders | TSO or HLC (open) |
| TiKV/TiDB | RR (pessimistic) or SI (optimistic) | Leader reads | Multi-Region leaders | PD TSO |
| CockroachDB | SERIALIZABLE | Leaseholder + closed-ts followers | Multi-range leaseholders | HLC |
| FoundationDB | Strict serializable | Direct storage server at GRV | Via commit proxies (global order) | Master versions |
| etcd | Strict serializable | Yes (Raft) | Single leader | Raft index/revision |
| Cassandra/Scylla | Tunable (often eventual) | Optional | Multi-master | Wall clock / LWW |

**PedraDB is not eventually consistent** in either mode. Eventual consistency
is an anti-feature for a foundation database (layers cannot build correct
indexes on top of eventual bases).

---

## 8. Multi-write refined (from primary sources)

### What production systems actually do

| Pattern | Who | Mechanism |
|---------|-----|-----------|
| Single-leader per shard, multi-shard multi-leader | TiKV, CRDB, etcd-per-range | Raft leader / leaseholder per Region |
| Global write ordering via central sequencer | FDB | Master assigns commit versions |
| Multi-master last-writer-wins | Cassandra, DynamoDB default | Not ACID; conflicts resolved by timestamp |
| Multi-master CRDT | Redis CRDT, some edge DBs | Commutative ops only |

**PedraDB distributed = row 1.** Multiple nodes write at once, but each key
has exactly one leader. Not multi-master.

### Contention modes that matter for design

1. **Uncontended multi-Region TX** — Parallel Commits / pipelined 2PC shines
2. **Hot key single Region** — becomes single-leader bottleneck; need
   application-level sharding or key redesign
3. **High contention same keys** — pessimistic locking (TiKV default) beats
   optimistic; PedraDB should support both modes eventually
4. **Cross-DC TX** — WAN RTT dominates; locality-aware placement is the
   real fix, not protocol tricks

---

## 9. Protocol latency model (concrete)

Assume LAN RTT = 0.5 ms, consensus (majority of 3) ≈ 1 RTT, WAN RTT = 50 ms.

| Protocol | Consensus RTTs to commit | LAN latency order | WAN latency order |
|----------|--------------------------|-------------------|-------------------|
| Local PedraDB (embedded) | 0 | ~0.01–0.1 ms | N/A |
| Single-Region write (Raft only) | 1 | ~0.5–2 ms | ~50–100 ms |
| Classic 2PC over multi-Raft | 2 sequential | ~1–4 ms | ~100–200 ms |
| Parallel Commits | 1 parallel | ~0.5–2 ms | ~50–100 ms |
| FDB full pipeline | ~3+ hops | few ms | high |

**Takeaway:** Parallel Commits is the target. Classic 2PC doubles WAN cost.

---

## 10. What PedraDB's local TX buys us (refined)

When building the distributed layer, PedraDB's embedded ACID changes the work:

| Component | TiKV/CRDB had to build | PedraDB already has |
|-----------|------------------------|---------------------|
| Local MVCC | Yes (on RocksDB/Pebble) | Yes (core design) |
| Local conflict detection | Yes (latches + locks) | Yes (OCC / interval tree) |
| Local atomic apply | Raft apply → RocksDB WriteBatch | Raft apply → PedraDB TX |
| Cross-Region atomicity | Percolator / Parallel Commits | Still need (2PC layer) |
| Timestamp assignment | TSO / HLC | Still need |
| Region split/merge | PD / allocator | Still need |
| Raft replication | Yes | Still need |

**The hard remaining work is consensus + cross-Region coordination + placement,
not reinventing local transactions.** That is PedraDB's structural advantage.

---

## 11. Refined recommendations for PedraDB

### Settled (from this research)

| Decision | Choice | Why |
|----------|--------|-----|
| Distribution architecture | Multi-Raft + range Regions | TiKV/CRDB proven; maps to ordered KV |
| Consistency | Strict serializable, CP | Foundation for layers; FDB/etcd model |
| Multi-write model | Single leader per Region | Required for serializability |
| Sharding | Range-based, split/merge | Preserve range scans |
| Cross-Region commit | Parallel Commits (not classic 2PC) | Half the latency |
| Distribution timing | After embedded slices 0–7 | Core must be solid first |
| Eventual consistency | **Never as default** | Layers need correctness |

### Still open (refined)

| Decision | Options | Notes |
|----------|---------|-------|
| Clock / version source | PD-style TSO vs HLC | TSO simpler, central; HLC scalable, needs NTP discipline |
| TX mode default | Optimistic vs pessimistic | TiDB switched to pessimistic for OLTP; PedraDB layers may want both |
| Raft library | openraft vs raft-rs | openraft for DX + sim; raft-rs for TiKV pedigree |
| Lock storage | In-KV (Percolator) vs in-memory with spill | In-memory is faster, crash-fragile (TiKV lesson) |
| Gap locking | Support or not | Needed for full SQL RR; not needed for pure KV |
| Follower reads | Bounded-staleness via closed timestamps | Optional performance feature |
| max-txn-ttl | What default? | TiDB uses 1h; FDB uses 5s (distributed tax) — PedraDB can be generous locally |

### Explicit non-goals (distribution)

- Multi-master / LWW conflict resolution
- Default eventual consistency
- Hash-only sharding (kills range scans)
- Building distribution before embedded engine is trustworthy

---

## 12. Implementation sketch (future crate layout)

```
pedradb-core/          # embedded TX + LSM (current work)
pedradb-raft/          # multi-Raft glue: Region, log, apply to core
pedradb-pd/            # placement driver + TSO (or HLC helper)
pedradb-txn/           # distributed TX coordinator (Parallel Commits)
pedradb-sim/           # deterministic sim (covers core + later distributed)
```

None of these start until Slices 0–7 of the embedded core are solid.

---

## 13. Sources

| Ref | Source | Persisted / fetched |
|-----|--------|---------------------|
| [P] | Peng & Dabek, Percolator, OSDI 2010 | `references/percolator-osdi2010.pdf` |
| [TiKV-opt] | TiDB Optimistic Transactions | docs.pingcap.com (2026-08-10) |
| [TiKV-pes] | TiDB Pessimistic Transactions | docs.pingcap.com (2026-08-10) |
| [TiKV-sched] | TiDB Scheduling (PD) | docs.pingcap.com (2026-08-10) |
| [TiKV-tso] | TiDB TSO | docs.pingcap.com (2026-08-10) |
| [CRDB-tx] | CockroachDB Transaction Layer | cockroachlabs.com docs (2026-08-10) |
| [CRDB-pc] | Parallel Commits blog | cockroachlabs.com/blog/parallel-commits |
| [FDB-arch] | FoundationDB Architecture | apple.github.io (2026-08-10) |
| [etcd] | etcd API guarantees | etcd.io (2026-08-10) |
| [openraft] | openraft README | github.com/databendlabs/openraft |
| [raft-rs] | raft-rs README | github.com/tikv/raft-rs |
