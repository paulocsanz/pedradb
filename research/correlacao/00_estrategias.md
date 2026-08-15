# Strategies — what the literature actually supports

**Updated:** 2026-08-14
**Epistemic status:** síntese **pré-ficha D3** — hipótese de correlação.
Não citar como prova de leitura dos 100. Eleva-se à medida que
`fichamentos/ficha_R*.md` existirem.

Rows marked **settled-here** were read for this repo. Rows marked **survey-claim** come from Lv et al. 2025 / Zhang et al. 2024 / our own engine landscape and are **not** independently re-measured. Rows marked **incumbent-eng** are production engineering writeups, not theorems.

The point of this file is to stop re-deriving the same tradeoffs every session. When a ficha confirms or kills a row, edit the row in the same change.

## 0. The stack Pedra is walking

| Layer | Job | Incumbents | Pedra name |
|-------|-----|------------|------------|
| Local ordered KV + multi-key ACID | persist, recover, compact, snapshot | RocksDB, Pebble, fjall, Redwood | `pedradb-core` |
| Distributed TX / roles | majority, 2PC or OCC+resolvers, shards | FDB, TiKV, Cockroach | Montanha / `pedradb-store` |
| Products on top | SQL, DCS, stream, fold, HTAP projection | Record Layer, TiDB, Patroni+etcd, Slipstream | `pedradb-sql`, `-dcs`, `-stream`, `-fold` |

**Grail sentence (already in-tree):** Pedra never replaces TiKV/FDB/Postgres *alone*. Products built on Pedra do. Same as RocksDB vs TiKV.

## 1. Local engine (layer 1)

### 1.1 LSM vs B+tree

| Claim | Status | Consequence for Pedra |
|-------|--------|------------------------|
| LSM wins write-heavy ingest by turning random writes into sequential flushes + deferred merge | **settled-here** (O'Neil 1996; WiscKey; RocksDB experience) | Pedra is LSM. Do not dual-primary a B+tree in the same MANIFEST. |
| B+tree (Redwood, WiredTiger, LMDB) remains competitive for read-heavy / long-lived snapshots | **incumbent-eng** + FDB docs | Montanha can *face* FDB without Pedra becoming Redwood. |
| Hybrid (Magma, WiredTiger dual mode, TurtleKV 2026) exists because neither shape dominates all points of RUM | **survey-claim** | Only reopen if `benches/baseline` shows a workload Pedra loses by >2× to a B+tree peer *and* we name the workload. |

### 1.2 Compaction is the design space

Sarkar et al. VLDB'21 factor compaction into four knobs: **when**, **which data**, **how much**, **layout** (leveling vs tiering vs hybrid).

| Strategy | What it buys | What it costs | Pedra now |
|----------|--------------|---------------|-----------|
| Leveled (LevelDB/Rocks default) | low space amp, predictable reads | write amp ~O(T·L), write stalls | **this is us** (whole-merge + count/bytes) |
| Tiered / universal | lower write amp | space amp, worse reads | not shipped |
| Lazy Leveling (Dostoevsky) | merge last level only; drop superfluous merges | more runs above last level; implementation + tuning | **explicit non-ship** (RFC-0012) |
| Partial / granulated (Spooky) | cut WA of global compact and SA of naive partial | must partition lower levels by upper file boundaries | candidate after we have file-granular compact |
| Fluid / per-level T (Moose, RusKey, How-to-grow 2025) | independent knobs per level | huge config surface; needs a model | do not add knobs we cannot DST |
| Workload-adaptive trigger (TRIAD, DOPA-DB, Endure) | fewer stalls when mix shifts | needs a workload estimator that is not a lie | measure first; Endure-style *robust* tuning > point-optimal |

**Working rule:** one compaction policy we can simulate beats five policies we cannot explain under `FailingEnv`.

### 1.3 Filters and indexes (read amp)

| Strategy | Paper | Pedra |
|----------|-------|-------|
| Bloom per SST, skip tables that cannot hold the key | Rocks / Pebble practice | **shipped** as shape correctness (RFC-0014), not as a bench trick |
| Non-uniform FPR (more bits on small/upper levels) | Monkey SIGMOD'17 | not shipped; the math is the next cheap win after we have per-level stats |
| Succinct / ribbon / cuckoo | Ribbon 2021, Chucky SIGMOD'21 | only if Bloom CPU or memory shows up in `baseline` |
| Range filters | SuRF, Rosetta, REMIX, GRF, Disco SIGMOD'25 | range path is still young; Disco/REMIX are the first to fichar when we care about `scan` amp |
| Learned indexes inside SST | Bourbon OSDI'20 | high risk, DST-hostile; backlog |

### 1.4 Key-value separation

WiscKey FAST'16: keys stay in the LSM, values go to an append log. WA drops when values are large; range scans of large values become random I/O; GC becomes a second engine.

| Incumbent | Shape |
|-----------|-------|
| Badger | native vlog |
| Titan (TiKV) | Rocks plugin, threshold |
| BlobDB (Rocks) | optional, not default |
| HashKV | hashed vlog to make GC local |
| Pedra | **threshold spill** to `VALUES.vlog`; **GC open** |

Do not copy Badger GC by folklore. Fichar HashKV + Titan notes before designing GC.

### 1.5 Write stalls and pacing

SILK / SILK+, ADOC, Vigil-KV, Rocks `slowdown`/`stop` thresholds: stalls are **resource contention**, not a missing compact algorithm.

Pedra should grow: (1) named stall reasons in metrics, (2) a rate limiter we can turn off in sim, (3) flush vs compact priority. Not an LLM tuner (ELMo-Tune: ~100s per suggestion in the survey's own trial — unusable on the commit path).

### 1.6 WAL / recovery

ARIES (Mohan 1992) is still the recovery vocabulary: log before page, steal/no-force variants, analysis + redo + undo. Pedra is closer to **LevelDB/Rocks**: WAL + immutable SSTs + manifest, no ARIES undo of in-page updates. That is fine **if** torn writes fail closed (CRC) and `Ok` is not returned before the durability the API promised.

Crash papers that bind us: Pillai OSDI'14 (rename/fsync folklore is false), Alagappan OSDI'18 (protocol-aware recovery for consensus logs). These belong to `pedradb-sim` / DST before they belong to compact.

### 1.7 What we refuse at layer 1 (until measured)

- Lazy Leveling
- Skiplist/arena MemTable (keep `BTreeMap`)
- Learned index inside SST
- Dual row+column in one MANIFEST (HTAP triangle — see §3)
- FPGA / DPU compaction offload as a product dependency

## 2. Distributed substrate (layer 2)

| Strategy | Who | Pedra/Montanha reading |
|----------|-----|------------------------|
| Unbundled roles (proxy, resolver, log, storage) | FDB SIGMOD'21 | Montanha already aims at this face; do not collapse roles back into "one Raft does everything" without a named product |
| Multi-Raft + range shards | TiKV, Cockroach | `pedradb-raft` is single-group today; Multi-Raft is a product RFC, not a core change |
| Shared log + deterministic apply (Calvin) | Calvin SIGMOD'12 | alternative to OCC+2PC; only interesting if we want a *replayable* distributed apply |
| Percolator-style OCC + timestamp oracle + 2PC on Bigtable | Percolator OSDI'10 | closest classic to "TX on a dumb KV" — fichar against Montanha 2PC |
| Strict serializability by default | FDB, Spanner (with clocks) | do not silently weaken isolation to win a bench |
| Follower / learner reads | TiFlash learner, HotCloud'17, CRDB leases | fold/HTAP replica is a **learner**, never a voter (RFC-0024) |
| DST before disk | FDB; MODIST NSDI'09 | this is already Pedra doctrine (`Env`/`Host`/`Clock`/`Rng`) |
| Serverless / multi-tenant virtualization | CRDB Serverless SIGMOD'25 | not P0; read when we sell a cloud face |

**Working rule:** distribution is a layer that *applies* to Pedra (`apply_batch`, snapshot seq, WAL ship). It does not grow a second local engine.

## 3. Databases on top (layer 3)

### 3.1 The FDB lesson (Record Layer, Snowflake-on-FDB, CouchDB-on-FDB)

FDB ships the **lower half**: ordered KV, strict serializable TX, no query language. Layers are stateless. This is the same bet as RFC-0010.

What Record Layer actually adds (to be confirmed in the ficha): record types, indexes as KV projections, multi-tenant directory prefixes, index maintenance inside the same TX.

**Implication:** indexes, SQL, DCS, streams are **projections + protocols**, not new durability.

### 3.2 Fold / change feed (Slipstream, RFC-0024)

Not a paper family with a single SIGMOD name. Discipline, already encoded:

1. Cursor advances **after apply**, never on receipt.
2. Torn apply may leave data ahead of cursor (replay-safe), never a cursor naming a hole.
3. After the log forgets your cursor, **folds are the only full replicas** — resync must reconstruct deletes.
4. Fold `get` is LocalApplied, not linearizable.

Naiad / differential dataflow (SOSP'13) is the closest academic lineage for incremental fold. Do not import the Naiad runtime. Steal the *vocabulary* (versioned frontier, incremental operator) if a ficha says it maps.

### 3.3 HTAP (triangle, not a motor)

From Zhang et al. 2024 + papers already in `docs/references/` (read for the HTAP note):

| Vertex | Meaning |
|--------|---------|
| Layout | row for point, column for scan |
| Freshness | analytics sees recent commits (named SLA) |
| Isolation | mixed load does not wreck OLTP p99 |

No pure-software system sits on all three. Pedra's honest shapes:

1. **Composed:** CDC / change-feed → DuckDB/CH/Iceberg
2. **TiFlash-shaped:** Raft learner → columnar projection
3. **LASER-in-kernel** (Real-Time LSM): only if proven; lifecycle layout inside LSM, not a second primary

Generic "HTAP engine" = one SoR + planners + projections. Not a flag on SST.

## 4. Testing strategy (binds all layers)

| Practice | Source | Pedra |
|----------|--------|-------|
| Deterministic single-thread cluster sim | FDB; Will Wilson 2014 | `pedradb-sim`, `pedradb-dst` |
| Fault injection at the Env seam | FDB AsyncFile; Pillai | `FailingEnv` |
| Protocol-aware recovery | Alagappan OSDI'18 | Raft/Montanha logs |
| Metamorphic / bidirectional format tests | Pebble | SST/WAL version tests |
| Jepsen-style linearizability | Kyle; etcd/TiKV | product layers, not a substitute for DST |
| Buggify | FDB | `buggify_hooks` |

DST does not prove performance. Perf gates stay separate (`montanha-perf-gate`). Never trade a silent-wrong for a faster compact.

## 5. How we will refine "what works"

After each wave of fichas, update this table. A strategy **works for Pedra** only if:

1. The paper's number was reproduced *or* the mechanism is simple enough that a unit/DST test is the proof (e.g. Bloom skip is a shape test).
2. It does not require hardware we will not ship.
3. It does not violate fail-closed durability / CRC / TX all-or-nothing.

Until then it is a candidate, even if it is famous.
