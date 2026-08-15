# Strategies — what the literature actually supports

**Updated:** 2026-08-15
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

Sarkar et al. VLDB'21 (**ficha R012 D4**) factor compaction into four primitives: **trigger**, **data layout**, **granularity**, **data movement** (p. 1–4). Catalog shorthand was when / which / how much / layout — *layout* and *which file* are distinct axes.

| Strategy | What it buys | What it costs | Pedra now |
|----------|--------------|---------------|-----------|
| Full two-level merge | simple; fewer jobs | WA 63× ingest no paper; picos | **this is us** (`compact_levels`: \(N\cup N+1\) → 1 SST; trigger = count/bytes) |
| Partial + LO+1 | −34–56% vs Full; −10–23% vs outros partial; tail write ~1.3 ms | mais jobs (\(4\times\)); precisa pick | **L35 MEASURE** — próximo knob, antes de Spooky |
| 1-leveling (Rocks default) | mais estável no mix (p. 12) | L0 tiered cresce | L0 já acumula 4 ficheiros; não copiar às cegas |
| Tiered / universal | melhor em update-heavy (TA VI) | tail ~25 ms; escala mal >8 GB; point 1.1–2.2× | **L37 REFUSE** default |
| Lazy Leveling (Dostoevsky) | merge last level only | precisa Monkey FPR; short range piora | **REFUSE** (ficha R007; L3) |
| TSD/TSA / FADE (Lethe, ficha R031 D4) | tombstones dentro de Dth; space −48% (Dth=50%) | WA inicial 1.4×, fim +0.7%; R012 +18–35% é outro bench | **L38 MEASURE** depois de L35, só com SLA Dth. **L43 REFUSE** KiWi |
| Menu de 10 / auto-switch | “no perfect strategy” (p. 2) | DST não explica 10 | **L36 REFUSE** |
| Partial / granulated (Spooky, ficha R013 D4) | >2× SA vs Full e >2× WA vs Partial *no NVMe cheio*; alinha fronteiras ao \(L\) | precisa ficheiros + \(L\) grande; \(X=L-1/L-4\) crasham | **L11 MEASURE** depois de L35; Pedra Full-do-par \(L\le 3\) **não** é o Full ≤50% |
| Fluid / per-level T | knobs por nível | config surface | do not add knobs we cannot DST |
| Endure-style robust (ficha R014 D4) | pior caso na bola KL; 5× modelo / 2.4× Rocks | precisa T + bits + L\|T; retune online inviável | **L12 MEASURE** se ≥2 knobs. Hoje 0. Tuner **não**. Table 3 = sempre leveling (L3/L37) |

**Working rule:** one compaction policy we can simulate beats five policies we cannot explain under `FailingEnv`.

### 1.3 Filters and indexes (read amp)

| Strategy | Paper | Pedra |
|----------|-------|-------|
| Bloom per SST, skip tables that cannot hold the key | Rocks / Pebble practice | **shipped** as shape correctness (RFC-0014), not as a bench trick |
| Non-uniform FPR (more bits on small/upper levels) | Monkey SIGMOD'17 (ficha R006 D4) | **not shipped.** Math holds (\(R=\sum p_i\)); 50–80% é HDD + cache-off + \(L\) grande. Pedra: 10 bits/key, `MAX_LSM_LEVEL=3`. L2 `MEASURE` |
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
| HashKV (ficha R018 D4) | hash(key) → grupo; GC sem get LSM; Fig. 2 tail = 19.7×. L19 MEASURE — 0028 não P0 |
| Pedra | **threshold spill** to `VALUES.vlog`; **GC open** |

Do not copy Badger GC by folklore. HashKV fichado (R018): não SHIP grupos até um soak em que blobs/rewrite percam. Titan ainda listed.

### 1.5 Write stalls and pacing

SILK (ficha R017 D4) / ADOC (ficha R016 D4) / Rocks `slowdown`/`stop`: stalls are **data overflow** (MMO/L0O/RDO = MT/L0/PS), not a missing compact algorithm. Rate-limit cego e “adiar compact” adiam o spike (SILK lições 2–3). CPU/BW/L0-L1/fundo, cada um sozinho, não generaliza (ADOC Table 2). Teste curto mente.

**L41 MEASURE:** named stall reasons (taxonomia ADOC) + flush > L0-compact > L≥1. Limiter só se `Env` o desligar no DST. Não o scheduler SILK completo.
**L42 REFUSE:** tuner ADOC (AIMD threads+batch). Pedra tem 0 knobs vivos; retune live é DST-hostil. SILK e ADOC são eixos diferentes (p99 vs caudal).

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
| Multi-Raft + range shards | TiKV, Cockroach (ficha R045 D4) | `pedradb-raft` is 1 group. L39 MEASURE split/lease when store has >1 group. Not a core change today |
| Shared log + deterministic apply (Calvin) | Calvin SIGMOD'12 | alternative to OCC+2PC; only interesting if we want a *replayable* distributed apply |
| Percolator-style SI + timestamp oracle + 2PC on Bigtable | Percolator OSDI'10 (ficha R041 D4) | clássico de “TX on a dumb KV”. **Não** é o 2PC do store (intents FDB). L22/L23 `REFUSE` |
| Strict serializability by default | FDB, Spanner (with clocks) | do not silently weaken isolation to win a bench |
| Follower / learner reads | TiFlash learner (ficha R048 D4); CRDB closed-ts ~2 s (ficha R045 D4) | fold **não** é voter (L8). Read-index e follower-read CRDB **não** são fold (L24) |
| DST before disk | FDB; MODIST NSDI'09 | this is already Pedra doctrine (`Env`/`Host`/`Clock`/`Rng`) |
| Serverless / multi-tenant virtualization | CRDB Serverless SIGMOD'25 | not P0; read when we sell a cloud face |

**Working rule:** distribution is a layer that *applies* to Pedra (`apply_batch`, snapshot seq, WAL ship). It does not grow a second local engine.

## 3. Databases on top (layer 3)

### 3.1 The FDB lesson (Record Layer, Snowflake-on-FDB, CouchDB-on-FDB)

FDB ships the **lower half**: ordered KV, strict serializable TX, no query language. Layers are stateless. This is the same bet as RFC-0010.

What Record Layer actually adds (**ficha R044 D4**): protobuf records, record store = subspace (records+indexes+header), key expressions, index maintenance **inside the same TX** (p. 6), atomic-mutation indexes that need FDB conflict-free atomics (p. 7), VERSION+incarnation for sync (p. 8–9), continuations because FDB kills TX at 5 s. SQL is a *further* layer on top of RL (p. 12). L31 SHIP / L32 REFUSE product in kernel / L33 MEASURE VERSION / L34 REFUSE atomics.

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
2. **TiFlash-shaped:** Raft learner → columnar projection (ficha R048: o *papel* learner sim; read-index no fold **não**)
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
