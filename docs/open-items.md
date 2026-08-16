# Open items: PedraDB engineering status

> Living document. Updated whenever a design decision is resolved, a research
> item is closed, or a new open question emerges. The authoritative source for
> "what's done, what's next, what's unresolved."

Last updated: 2026-08-14 (RFC-0024 done: Montanha fold for Caixote)

---

## Current state at a glance

```
 pedradb-core    WAL ✅  | MemTable ✅  | Db ✅  | TX ✅  | SST v3+Bloom ✅  | range/limit ✅
                 compact ✅  | apply_batch ✅  | checkpoint ✅  | stats/verify ✅
                 vlog+GC ✅  | group commit / dual-mem ✅  | CAS/feed/seq (0019) ✅
 pedradb-sim     FailingEnv / RecordingEnv ✅
 pedradb-oracle  model oracle + optional live-rocksdb ✅
 pedradb-cli     version + wal + demo

 RFC-0009 done · RFC-0014 done · RFC-0015 done · RFC-0019 done
 RFC-0016 P0 done · RFC-0017 draft · RFC-0020 done (P0–P2 synthetic field maturity)
 Vision: docs/node-primitive-and-unified-platform.md (SoR + projections + multi-leader)
```

---

## 1. Roadmap status

| # | Slice | Status | Key deliverables | Blocked by |
|---|-------|--------|-----------------|------------|
| 0 | WAL | ✅ done | Block format, masked CRC32C, fragmentation, recovery | — |
| 1 | InternalKey + MemTable | ✅ done | `InternalKey` struct + Ord/encode, MemTable get/put/delete/range @ snapshot | — |
| 2 | WAL recover → MemTable + basic engine get/put | ✅ done | WriteRecord v1, `Db::open/put/get/delete`, reopen | Slice 1 |
| 3 | Transaction manager + API | ✅ done | begin/commit multi-key ACID (single-writer) | Slice 2 |
| 4 | SST + flush | ✅ done | SST v3 blocks + bloom + Db::flush | Slice 2–3 |
| 5 | Get + range scan (merged) | ✅ done | `range` / `range_limited` + prune | Slice 4 |
| 6 | Compaction | ✅ done | whole-merge; count **or bytes** auto trigger | Slice 4 |
| 7 | Version GC | ✅ API | `CompactGcOptions` / `compact_with` (auto-compact keeps history) | — |
| 8 | Deterministic simulation | ✅ base / 🔲 sweeps | Env/FailingEnv + campaigns | continuous hunt |
| 9 | Cross-validation harness | ✅ done | pedradb-oracle model (+ live-rocksdb feature) | — |
| 10 | Ops surfaces (RFC-0014 P0) | ✅ done | checkpoint, stats, verify_checksums | — |
| 11 | Streaming range / lazy blocks (RFC-0014 P1) | ✅ done | scan + lazy SST blocks + levels + lz4 | — |
| 12 | Audit correctness fixes (RFC-0015) | ✅ done | fence, sync_dir, Env seams, compact stats, deny CI | — |

**Next action:** Full bindingtester / Java RL only if requested; FDB **field** peer numbers need lab `fdbserver`. Value-store pick C (0029) done including CLI `compact-blob` / `blob-gc` / `maintain`.  
**CI:** `synthetic-field` **montanha-scale-and-compare** — scale_gate (± `MONTANHA_WRITE_BACKPRESSURE=1`) + `montanha_bp_ab_v0` (off vs BP thr/admission delta) + mini_bt_soak (± BP) + fdb-compare template (no FDB required).  
**Shipped (admission):** Pedra L0/mem write stall + soft pressure; Montanha `StoreError::WriteStall*` + `WriteAdmissionSnap`; lab flags on scale-gate / fdb-bench / perf-gate / montanha-tcp / mini_bt_soak; fdb-compare pass-through `write_backpressure`; structured `admission_*` in scale/perf reports.  
**Shipped (YCSB parity workstream):** `montanha-fdb-bench` suite **ycsb** (shapes of the FDB `benchmark` tool: `ycsb_a..f`, 50/50 · 95/5 · 100r · read-latest+insert · short scans · RMW; uniform|zipfian); `scripts/fdb_side_ycsb.sh` (peer FDB: official `benchmark` → python binding → stub); `scripts/montanha_fdb_parity_v0.sh` (end-to-end + `MONTANHA_PARITY_RATIO_FLOOR` gate; CI em template mode com `parity.pass: null`). **Parity numbers require lab fdbserver** — CI cover only the shapes/estrutura.
**Shipped (rocks parity workstream):** `crates/rocksdb-parity-bench` — mesmos shapes YCSB A–F num runner genérico (schedule idêntico por construção; contadores de ops batem entre motores) com dois adaptadores: `compat` (rocksdb-compat sobre pedradb-core, fsync-before-Ok) e `rocksdb` real (crate `rocksdb` 0.22/librocksdb-sys 8.10, feature `real`, `ROCKS_PARITY_SYNC` 1=sync-per-write default); `rocks-parity-compare` com gate `ROCKS_PARITY_RATIO_FLOOR` (exit 2 com peer real abaixo do piso; template mode verde); `scripts/rocks_side_ycsb.sh` + `scripts/rocksdb_parity_v0.sh`; CI em template mode. **Lab (2026-08-15, @e5c6b80):** writes ratio **0.002–0.011** vs RocksDB sync-per-write; reads 0.10–0.71×. **Cliff diagnosticado:** cada write sincronizado regrava o CHANGELOG inteiro (`change_feed::store_on`: encode+tmp+fsync+rename+dir-fsync; quadrático) — RFC-0019 já permite store periódico/batched (cache reconstruído do WAL); até isso pousar o floor fica report-only. Ver [rocksdb-compat.md](rocksdb-compat.md).
**Shipped (rocks parity — suíte de dependentes, shapes TiKV):** suíte **deps** no mesmo runner/peer (CFs `default`+`write`+`lock`+`raftlog`): `deps_apply_batch` (apply do raftstore: prewrite+commit WriteBatch multi-CF, `ROCKS_DEPS_BATCH` txns/commit), `deps_mvcc_latest` (SeekForPrev MVCC + fetch no default), `deps_scan` (coprocessor/GC), `deps_raftlog` (append batched 16 + read trailing), `deps_cache_overwrite` (overwrite unbatched zipf). **Lab (@863df07):** apply 0.016× · mvcc_latest **0.003×** · scan 0.005× · raftlog 0.007× · overwrite 0.006× vs RocksDB sync-per-write. **Segundo mecanismo quantificado:** iterador eager do compat materializa o CF inteiro por chamada (mvcc_latest/scan ~600× abaixo do point-get próprio) = gap #6 da matriz TiKV, agora com número. Leverage para paridade: (1) CHANGELOG batched store, (2) iteradores lazy. Tabela + procedência em [rocksdb-compat.md](rocksdb-compat.md).
**Lab (TiKV-doc YCSB mixes, 2026-08-15):** zipfian + 1 KB + 4096/2000, *not* a 3-node TiKV cluster. vs Rocks fdatasync: writes ~0.02×, point-get 0.03×, MVCC/scan ~0. Writes vs same-class F_FULLFSYNC: **0.67–1.14×**. Hole = eager iterator (deps_mvcc p50 292 ms). [findings/tikv-ycsb-lab-20260815.md](../findings/tikv-ycsb-lab-20260815.md). `scripts/tikv_ycsb_parity_v0.sh`.
**Open (RFC-0031, orçamento 2× same-class):** `F_FULLFSYNC` p50 4.8 ms vs `fdatasync` 50 µs neste Mac — 2× contra o peer fdatasync, mantendo G1, é fisicamente impossível. Peer oficial: `ROCKS_PARITY_FULL_SYNC=1`. **Lab:** writes 0.77–1.23× (várias >1×); falham 2× só `ycsb_c` (0.12) e `deps_mvcc_latest`/`deps_scan` (iterador eager). P1 = iterador janela. Ver [RFC-0031](rfc/0031-rocks-parity-10x-budget.md).
**Open (RFC-0032):** 2× **nesta tabela** TiKV-mix. Gate oficial = vs Rocks **F_FULLFSYNC** ≥ 0.5. Escritas + **C** (405k qps) + ycsb_e já passam (`5cf09a9`). Sem floor 2× de escrita vs fdatasync (G1). Ver [RFC-0032](rfc/0032-tikv-mix-2x-budget.md).
**Open (RFC-0033):** P0 shipped. 2026-08-16 vs FF: MVCC **18×** (9.0k / 160k), scan **27×** (6.1k / 165k) — alvo 2× **não** atingido. Residual = L0 sobrepostos quando a key não está na mem. Ver [RFC-0033](rfc/0033-mvcc-scan-2x.md).
**Open (RFC-0035):** cache 8192 + `count_in_range`. MVCC **105k / 1.2 µs** vs FF 243k — **2.3×**. Scan **54k / 14 µs** vs 262k — **4.8×**. **2× não** (P1.3). Residual = get no fallback SST + 3 streams no scan. Ver [RFC-0035](rfc/0035-mvcc-scan-2x-measure-first.md).
**Open (RFC-0034):** teto **1.1×** (`Pedra / Rocks_F_FULLFSYNC ≥ 0.91`) em **todos** os 11 shapes. Remesura 4096/2000 FF: **0/11 passam** (A 1.12× · F 1.19× · C 8× · E 2.2× · scan 35× · MVCC 69×). Sem 1.1× de escrita vs fdatasync (G1). Sem gate mentiroso no conjunto vazio. Ver [RFC-0034](rfc/0034-rocks-parity-1.1x-all-shapes.md).
**Shipped (rocksdb-compat):** `crates/rocksdb-compat` — rust-rocksdb-shaped API subset on pedradb-core (open_cf / put / get / delete / delete_range_cf / atomic WriteBatch / snapshot / iterator modes / flush·compact; CFs emulated por prefixo com `default` prefixed quando há CFs nomeadas) + suite adversarial FailingEnv (dead-disk ×32, sync-fail ×8, short-write ×8, batch all-or-nothing, iterator positioning) + alias-swap smoke (`rocksdb-compat-alias-smoke`). **Não é drop-in TiKV** — gap matrix em [rocksdb-compat.md](rocksdb-compat.md) (ingest, compaction filters, WriteBatchWithIndex, properties, delete_files_in_range, concorrência multi-thread).  

**DST soak / FDB compare / scale S1–S10:** see prior notes + [montanha-vs-fdb-bench.md](montanha-vs-fdb-bench.md).  
**Shipped Phase 1–3 + A–E continuum:** [montanha-fdb-phases.md](montanha-fdb-phases.md) — fdb-compat 12-step harness (`clear_range`), etcd multiproc + **TCP DCS wire**, `RecordTable` unique/multi-index, platform need faces (CP/OLAP/stream), F47–F49 residuals.  
**Shipped (bench):** `montanha-fdb-bench` — FDB-shaped microbenches + TCP multi-client + mini-bt E1/E2 → `fdb_shaped_bench.json`; method in [montanha-vs-fdb-bench.md](montanha-vs-fdb-bench.md).  
**Shipped (CI mini-bt + scale gate):** mini_bt_soak + `montanha_scale_gate_v0` in synthetic-field CI.  
**Shipped (perf parity RFC-0025 complete):** [0025](rfc/0025-montanha-perf-parity-vs-peers.md) — batch/coalesce/PutBatch/log append; **scale A confirmed** (S6–S10; S3 r8 thr=8 no hang, ~3.5× r1); elect-wait; per-range client; election diversity; multiproc rebalance; scoped tick; scale walls; thr≥min(nr,8); **read capacity**; **fdb-compare**.  
**Shipped (recipes):** `montanha-fdb-recipes` — design recipes + SI/OCC + Record seed.  
**Shipped (0023):** SI, Transaction, watermark, fdb_compat + c-api.  
**Do not** claim FDB field peer, full bindingtester, production Record Layer, or drop-in etcd/Scylla/CH/NATS.  

Shipped (0017 lab): TCP multi-host + caixote mesh + proxy.  
Shipped (0021 **all Status rows**): TX/limits, perf/sim gates, `/v1/cluster`, split, **TCP rewire without SSH**, region-aware dial lab, drills, runbooks.

**Platform north star:** Postgres-class face that replaces **Scylla + ClickHouse + NATS need** in one system  
([`node-primitive-and-unified-platform.md`](node-primitive-and-unified-platform.md) §1).  
Kernel = Pedra; horizontal = Montanha; analytics = OLAP RO; streams = layer; CP scale = scylla-need path.

**Perf ceiling + sled layer:** living plan in
[`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md)
(K1–K3 kernel phases, L0–L3 `pedra-map` / sled-compat; anti-corner checklist F1–X6).  
**RFCs:** [0009](rfc/0009-rocksdb-class-engine.md) · [0010](rfc/0010-dbs-on-top.md) · [0014 maturity](rfc/0014-rocks-pebble-redwood-maturity.md) · [0015 audit](rfc/0015-audit-pedradb-correctness-fixes.md) · [0016 robustness](rfc/0016-pedradb-production-robustness.md) · [0017 Montanha FDB-class](rfc/0017-montanha-fdb-class-substrate.md) · [0018 FDB method](rfc/0018-fdb-method-parity-and-fault-coverage.md) · [0019 L1](rfc/0019-local-primitive-for-platform-and-scylla-need.md) · [0020 synthetic field](rfc/0020-synthetic-field-maturity.md) · [0021 lab gates](rfc/0021-montanha-fdb-tikv-parity-gaps.md) · [**0022 functional FDB + N-writer layers**](rfc/0022-montanha-fdb-functional-parity-and-layer-substrate.md) · [**0024 Montanha fold / Caixote**](rfc/0024-montanha-fold-for-caixote.md) (**done**)

---

## 2. Open design decisions (unresolved)

These are questions where the general direction is known but the specific
approach hasn't been finalized. Each needs a concrete decision before its target
slice can be implemented.

### 2.1 Version GC strategy (Slice 7) — **(b)+(c) shipped**

**Question:** How does PedraDB reclaim old MVCC versions?

**Answer (2026-08-15):**
- **(b) piggyback** via `Db::compact_reclaim` +
  `CompactGcOptions::for_oldest_snapshot` (Rocks-style: drop a superseded version
  when the next newer has `seq <= oldest open pin`). Pins are explicit:
  `pin_snapshot` / `release_snapshot_pin` (also on `ConcurrentDb`). Bare
  `Snapshot` tokens still do **not** block GC (F20: auto-compact stays history-
  preserving).
- **(c) safety valve:** `earliest_readable_sequence` watermark raised on
  history-dropping GC (`latest_only`, `for_oldest_snapshot`, `min_sequence`).
  `get_at` / `multi_get_at` / TX·OCC `get` return
  [`CoreError::SnapshotTooOld`](../crates/pedradb-core/src/error.rs) fail-closed
  (Montanha already maps store-level too-old to FDB `transaction_too_old`).
  **Durable:** MANIFEST format **v4** stores the watermark; reopen restores it.

**Opt-in auto path:** `set_auto_reclaim(true)` makes threshold auto-compact use
snapshot-safe reclaim (pin floor or last seq) and advance the too-old watermark.
**Default remains off** (F20: bare `Snapshot` history preserved across auto-compact).

**Options (historical):**
- **(a) Stop-the-world pause:** scan all versions, remove those older than the
  oldest active snapshot. Simple but causes latency spikes.
- **(b) Incremental GC:** reclaim versions during compaction (piggyback). No
  pause but versions live longer.
- **(c) "Snapshot too old" error:** like FDB — if a snapshot is too old, abort
  the transaction. Safety valve, not primary mechanism.

### 2.2 Value-log GC strategy (Slice 7) — **resolved for P0**

**Answer (RFC-0016 P0.1):** Rewrite-compact via `Db::compact_vlog`: collect live
VLG1 refs from mem/imm/SSTs → write `VALUES.vlog.new` → remap SST/mem pointers →
MANIFEST + adopt marker → promote. Threshold remains **opt-in off by default**.

**Residual (partial):** `set_auto_blob_gc_min_ratio` runs best-effort
`compact_blob_auto` after flush / `latest_only` / ConcurrentDb
`finish_flush_pipeline` (no bg thread in core). **Operator bg substitute:**
`pedra maintain <db> [--every SECS]` (flush + reclaim + blob θ; optional
`--vlog`). Cron/loop lives outside the engine.

**Context (historical):** Values are written append-only. When a key is overwritten or
deleted, the old value becomes garbage. The value log needs periodic
compaction to reclaim space.

**Options:**
- **(a) Online GC:** background thread scans the log, discards orphaned values,
  rewrites live ones. Like BadgerDB's approach.
- **(b) Batch GC:** piggyback on LSM compaction — when a key is compacted,
  discard its old value-log entry.
- **(c) Generational:** split value log into segments, GC the oldest first
  (like generational GC in language runtimes).

**Likely answer:** (a) + (c). BadgerDB and fjall both use online + segment-based.
Needs study of their implementations.

### 2.3 Backpressure strategy (Slice 7) — **(a) partial shipped**

**Question:** When write rate exceeds flush/compaction capacity, what does
PedraDB do?

**Answer (2026-08-15):** **(a)+(b)+(c)** opt-in (no artificial sleep).
- **L0 hard:** `set_write_stall_l0(Some(n))` → [`CoreError::WriteStall`].
- **L0 soft pressure (b):** `set_write_pressure_l0(Some(n))` → one flush+compact
  before admit (no error); counter `write_pressure_count`. Typically n < hard.
- **Mem (c):** `set_write_stall_mem_bytes(Some(b))` → [`CoreError::WriteStallMem`].
- **Hard drain:** `set_write_stall_drain(true)` one drain before hard refuse.
Default for all: **off**. Convenience:
`enable_write_backpressure_defaults()` → pressure @ `L0_COMPACTION_TRIGGER`,
hard stall @ 2×, drain on. Stats: `l0_files` + stall/pressure in `gc_line`.
ConcurrentDb mirrors.

**Options (historical):**
- **(a) Explicit stall:** block new writes until L0 is drained. Honest but harsh.
- **(b) Adaptive admission control:** accept writes at the rate the system can
  sustain, reject excess with backpressure signal.
- **(c) Let the MemTable grow:** unbounded MemTable, flush in background.
  Risk: OOM.

### 2.4 MemTable data structure (Slice 1)

**Question:** Skip list, B-tree, or something else for the sorted in-memory map?

**Options:**
- **(a) Skip list:** classic LSM choice (RocksDB, LevelDB). Lock-free concurrent
  reads + single-writer inserts. O(log n) lookup/insert.
- **(b) B-tree (like ART/adaptive radix tree):** faster point lookups, but
  harder to make concurrent and doesn't naturally produce sorted output for flush.
- **(c) Vector + sort-on-flush:** simple, excellent cache locality, but O(n log n)
  per flush and no concurrent read-during-write.

**Likely answer:** (a) skip list. Proven, well-understood, matches the LSM
pattern. Pebble uses a skip list.

### 2.5 Conflict detection granularity (Slice 2)

**Question:** Per-key tracking or interval/range-based?

**Context:** Per-key tracking has zero false conflicts but O(keys) memory.
Interval-tree tracking has some false conflicts (adjacent keys in the same
range) but O(ranges) memory, which is typically much smaller.

**Options:**
- **(a) Per-key:** exact, but memory-heavy for transactions touching many keys.
- **(b) Interval tree:** track read/write ranges. Memory-efficient. Some false
  conflicts for adjacent-but-unrelated keys.
- **(c) Hybrid:** per-key for small transactions, interval for large ones.

**Likely answer:** (b) interval tree. FDB uses this approach. Matches the
"conflict range" API design.

---

## 3. Research items still pending

| # | Item | Status | Notes |
|---|------|--------|-------|
| 1 | ~~Survey LSM research (Niv Dayan)~~ | ✅ done | Resolved by Dostoevsky [D] + Monkey [M] papers |
| 2 | ~~Dostoevsky / Monkey analysis~~ | ✅ done | Papers persisted in `docs/references/` |
| 3 | ~~Rust LSM engines (fjall, SlateDB)~~ | ✅ done | fjall [F] analyzed; SlateDB noted |
| 4 | ~~Engine landscape (10 engines)~~ | ✅ done | `docs/engine-landscape-and-ideal-path.md` |
| 5 | ~~Distributed systems analysis~~ | ✅ done | `docs/distributed-systems-analysis.md` |
| 6 | ~~FDB limitations analysis~~ | ✅ done | `docs/fdb-limitations-analysis.md` |
| 7 | ~~Why CockroachDB/TiKV didn't use FDB~~ | ✅ done | Analyzed: timing, architecture, feature gaps, language |
| 8 | RocksDB GitHub issues on compaction/amp/memtable | 🔲 open | Low priority — we have the academic analysis |
| 9 | Hardware-consciousness (NVMe, direct I/O, io_uring) | 🔲 open | Relevant for Slice 4 (SST I/O) and performance |
| 10 | Redwood (FDB's new B+tree) internals | 🔲 open | Interesting for comparison, not blocking |
| 11 | Pebble metamorphic testing framework details | 🔲 open | Relevant for Slice 8 (simulation) |
| 12 | ~~Distribution design (how embedded → distributed)~~ | ✅ done | `docs/distribution-design.md` — multi-Raft, CP, strict serializable |
| 13 | ~~Distribution deep research (Percolator, Parallel Commits, PD, TSO, Raft)~~ | ✅ done | `docs/distribution-deep-research.md` + `references/percolator-osdi2010.pdf` |
| 14 | Rust Raft implementations (openraft vs raft-rs) | 🔲 open | Research done; decision deferred to distribution layer (post-Slice 7) |
| 15 | HLC vs TSO for distributed timestamps | 🔲 open | TSO simpler; HLC scalable; both documented in deep research |
| 16 | Optimistic vs pessimistic default (distributed) | 🔲 open | TiDB switched to pessimistic for OLTP; PedraDB may want both |
| 17 | Parallel Commits implementation details | 🔲 open | Target protocol; need design when building pedradb-txn |
| 18 | In-memory vs durable distributed locks | 🔲 open | TiKV lesson: in-memory is fast, fragile under partition |
| 19 | ~~Slipstream + Quicksilver v2 (config fold / edge cache)~~ | ✅ done | `docs/slipstream-and-quicksilver-learnings.md` + `references/{slipstream,quicksilver}/` — not a SoR redesign; P0 is cursor-after-apply + ship resume |

---

## 4. Design decisions already resolved

For reference — these are settled and should not be re-litigated without strong
new evidence.

| # | Decision | Source | Target |
|---|----------|--------|--------|
| 1 | Clean-room rewrite in Rust (not CXX translation) | Pebble [P1], CGO pain | Global |
| 2 | `#![forbid(unsafe_code)]` | fjall [F] | Global |
| 3 | `clippy::pedantic` with zero warnings | Code quality | Global |
| 4 | RocksDB as oracle only (not linked into core) | CXX boundary analysis | Global |
| 5 | FoundationDB layer model (TX in core, nothing else) | FDB layer concept | Architecture |
| 6 | LSM-tree (not B-tree) as storage structure | Write-heavy workload fit | Slice 4+ |
| 7 | WiscKey KV separation (values in separate log) | WiscKey [W], BadgerDB | Slice 4 |
| 8 | Monkey Bloom allocation (FPR ∝ run size) | Monkey [M] | Slice 4 |
| 9 | Lazy Leveling compaction default | Dostoevsky [D] | Slice 6 |
| 10 | `InternalKey` as struct, not encoded string | Pebble [P2] | Slice 1 |
| 11 | Fixed-size MemTable arena | Pebble [P2] | Slice 1 |
| 12 | Batch as LSM level (seqnum high-bit) | Pebble [P2] | Slice 4 |
| 13 | Commit publish-queue lock-free | Pebble [P2] | Slice 2 |
| 14 | Range tombstones integrated in merging iterator | Pebble [P2] | Slice 5 |
| 15 | Invariant-based pacing (not static rate limit) | Pebble [P2] | Slice 6 |
| 16 | No artificial write throttling | Pebble [P2] | Slice 6 |
| 17 | Static dispatch (enum+match) on iterator hot path | Pebble [P2] | Slice 5 |
| 18 | Deterministic simulation testing | FDB | Slice 8 |
| 19 | No built-in distribution (embedded first) | Architecture decision | Global |
| 20 | WAL block format compatible with RocksDB | Compatibility | Slice 0 ✅ |
| 21 | Distribution via multi-Raft (not FDB decoupled model) | Simplicity, latency, Rust Raft libs | Future layer |
| 22 | Strict serializable consistency (not eventual) | FDB model; correctness non-negotiable | Future layer |
| 23 | CP choice (consistency over availability during partition) | FDB CAP analysis | Future layer |
| 24 | Range-based sharding (not hash-based) | Preserves ordered KV semantics | Future layer |
| 25 | Single-leader per Region (not multi-master) | Strict serializability | Future layer |
| 26 | Distribution is a layer on top of embedded core | Architecture decision | Future layer |
| 27 | Cross-Region commit via Parallel Commits (not classic 2PC) | CRDB Parallel Commits | Future layer |
| 28 | Eventual consistency never as default | Foundation correctness | Future layer |
| 29 | Percolator-style primary/secondary locks for cross-Region TX | Percolator OSDI'10 + TiKV | Future layer |
| 30 | PedraDB = local library only (RocksDB role); multi-node = other product | architecture-refined.md | Global |
| 31 | Own LSM (not wrap RocksDB, not port Redwood) | architecture-refined.md | Store |
| 32 | Local TX in PedraDB (outer DB embeds it; no bolt-on from zero) | architecture-refined.md | TX |
| 33 | Distribution docs = research for outer DB, not PedraDB roadmap | architecture-refined.md | Global |
| 34 | Optimize power/surface ratio; public API ≈ open+TX CRUD+range | positioning.md | Global |
| 35 | Research LSM under the hood, not API knobs | positioning.md | Global |

---

## 5. Code-level TODOs

| Crate | Module | Item | Priority |
|-------|--------|------|----------|
| pedradb-core | — | `InternalKey` struct (`user_key`, `seqnum`, `kind`) | Slice 1 |
| pedradb-core | — | `Slice` custom value type (immutable, cheap clone) | Slice 1 |
| pedradb-core | memtable | Skip list implementation (concurrent read, single writer) | Slice 1 |
| pedradb-core | memtable | Fixed-size arena allocator | Slice 1 |
| pedradb-core | memtable | Sequence number counter (monotonic, atomic) | Slice 1 |
| pedradb-core | tx | MVCC version tracking | Slice 2 |
| pedradb-core | tx | Interval tree for conflict ranges | Slice 2 |
| pedradb-core | tx | Commit pipeline (publish-queue) | Slice 2 |
| pedradb-core | sst | Block-based SST writer/reader | Slice 4 |
| pedradb-core | sst | Monkey Bloom filter builder | Slice 4 |
| pedradb-core | vlog | WiscKey value log (append-only, addressable) | Slice 4 |
| pedradb-core | iter | Merged iterator across levels | Slice 5 |
| pedradb-core | iter | Range tombstone integration + block-skip | Slice 5 |
| pedradb-core | compact | Lazy Leveling compaction strategy | Slice 6 |
| pedradb-core | compact | Cost model (Dostoevsky equations) for auto-tuning | Slice 6 |
| pedradb-core | gc | Version GC (piggyback on compaction) | Slice 7 |
| pedradb-core | gc | Value-log GC (online, segment-based) | Slice 7 |
| pedradb-sim | — | Deterministic simulation framework | Slice 8 |
| pedradb-oracle | — | Oracle diff harness implementation | Slice 9 |

---

## 6. Documentation index

| Document | Content |
|----------|---------|
| [`architecture.md`](architecture.md) | Full architecture, anti-features, roadmap, engineering items |
| [`rocksdb-critiques-and-improvements.md`](rocksdb-critiques-and-improvements.md) | 15 design decisions from Pebble, Dostoevsky, Monkey, fjall |
| [`engine-landscape-and-ideal-path.md`](engine-landscape-and-ideal-path.md) | Comparison of 10 engines, the 3 optimizations nobody combined |
| [`distributed-systems-analysis.md`](distributed-systems-analysis.md) | ScyllaDB, Ceph, TiKV, FDB, CockroachDB, ClickHouse layer analysis |
| [`distribution-design.md`](distribution-design.md) | How embedded PedraDB becomes distributed (multi-Raft, CP, strict serializable) |
| [`distribution-deep-research.md`](distribution-deep-research.md) | Protocol-level research: Percolator, Parallel Commits, PD, TSO, Raft libs |
| [`scylladb-architecture.md`](scylladb-architecture.md) | How Scylla operates (AP multi-master, Seastar, tunable CL) vs PedraDB |
| [`scylla-need-replacement.md`](scylla-need-replacement.md) | Replace Scylla *need* (routes, overlay, orchestrator) — not CQL drop-in |
| [`htap-storage-primitives-and-research.md`](htap-storage-primitives-and-research.md) | **Canonical 2026-08-12:** HTAP triangle, storage primitives, LASER/HaSiS/PolarDB/ByteHTAP/PIM; primaries in `references/*htap*` |
| [`tidb-architecture.md`](tidb-architecture.md) | TiDB = MySQL SQL layer on TiKV+PD+TiFlash; validates PedraDB layers |
| [`tidb-vs-postgres-mysql.md`](tidb-vs-postgres-mysql.md) | TiDB vs Postgres vs MySQL monoliths — choice table + PedraDB quadrant |
| [`sql-lessons-for-the-grail.md`](sql-lessons-for-the-grail.md) | Postgres/MySQL + Aurora/Neon (log-is-the-DB), Vitess/Citus (proxy+shard), Spanner (TrueTime) — cross-cutting lessons, Rung 1.5, WAL export Must |
| [`object-storage-as-substrate-possibility.md`](object-storage-as-substrate-possibility.md) | SlateDB/WarpStream/turbopuffer; **scope superseded** — Tigris data-plane claim corrected; SST “not built” stale |
| [`research/object-storage/06-tigris-and-media-tiers.md`](research/object-storage/06-tigris-and-media-tiers.md) | **2026-08-15:** Tigris = FDB brain + block bytes + six jobs; EC is job 5; media ≠ product name |
| [`research/object-storage/07-montanha-as-tigris-control-plane.md`](research/object-storage/07-montanha-as-tigris-control-plane.md) | **2026-08-15:** Montanha can sit in the FDB seat; bytes stay out; Fold not 13 clusters |
| [`references/tigris-architecture-primaries.md`](references/tigris-architecture-primaries.md) | Fetched extracts: architecture, small-object bench, Fly NVMe, snapshot/fork |
| [`sqlite-object-storage-agents-and-pedradb.md`](sqlite-object-storage-agents-and-pedradb.md) | **Canonical 2026-08-12:** SQLite-VFS + object/block-on-object (mercado, não tese de agente); Pedra slots + deep gaps; primaries in `references/sqlite-object-storage-primaries.md` |
| [`conversation-learnings-and-short-term-alignment.md`](conversation-learnings-and-short-term-alignment.md) | All conversation learnings + **conflict matrix vs P0** |
| [`nats-need-replacement.md`](nats-need-replacement.md) | JetStream-class stream on PedraDB+Raft vs Core NATS; Jepsen 2.12.1 findings |
| [`slipstream-and-quicksilver-learnings.md`](slipstream-and-quicksilver-learnings.md) | **2026-08-14:** QS v2 tiered cache + Slipstream fold/cursor-after-apply; what to steal vs not mix into Montanha SoR |
| [`rfc/0024-montanha-fold-for-caixote.md`](rfc/0024-montanha-fold-for-caixote.md) | **Draft:** Slipstream-class fold on Montanha for Caixote (P0 combinator+Pedra fold; P1 federation by seq) |
| [`usage.md`](usage.md) | **P0 user docs**: open/TX, durability, secondary-index sketch |
| [`foundationdb-layers-and-products.md`](foundationdb-layers-and-products.md) | What runs on FDB: Record/Document layers, Snowflake, CloudKit, Astra, … |
| [`etcd-comparison.md`](etcd-comparison.md) | etcd vs PedraDB, FDB, TiKV/TiDB, CRDB, Scylla, RocksDB, … |
| [`competitive-landscape-rust.md`](competitive-landscape-rust.md) | Rust/local engine peers: fjall, SurrealKV, redb, AgateDB, SlateDB… |
| [`positioning.md`](positioning.md) | Focus: tiny API, speed, max layer leverage (power/surface) |
| [`compare-fjall.md`](compare-fjall.md) | PedraDB vs fjall (closest Rust peer) |
| [`rfc/0001-pedradb-high-level-spec.md`](rfc/0001-pedradb-high-level-spec.md) | **Normative** high-level RFC (P0/P1/P2, open decisions) |
| [`rfc/0001-open-decisions-deep-dive.md`](rfc/0001-open-decisions-deep-dive.md) | O1–O10 deep dive: peers, nuances, regrets |
| [`session-synthesis-architecture-and-doubt.md`](session-synthesis-architecture-and-doubt.md) | Full conversation synthesis + “is PedraDB wrong?” |
| [`grail-plan-build-databases-on-pedradb.md`](grail-plan-build-databases-on-pedradb.md) | Plan: kernel → SQLite/etcd/TiKV/TiDB-class products |
| [`pedradb-as-dcs-storage-for-patroni.md`](pedradb-as-dcs-storage-for-patroni.md) | DCS on PedraDB for Patroni elections (not etcd protocol in core) |
| [`doctrine-primitives-and-api-layers.md`](doctrine-primitives-and-api-layers.md) | Doctrine: powerful primitive + API layers only |
| [`plug-map-replace-incumbents.md`](plug-map-replace-incumbents.md) | Where to plug: etcd, Patroni, SQLite, PG, TiKV, TiDB, Scylla |
| [`upsides-only.md`](upsides-only.md) | Upsides only: multi-writer regions, plugs, platform |
| [`plan-limitations-and-failure-modes.md`](plan-limitations-and-failure-modes.md) | Where the grail plan can fail later (perf, security, …) |
| [`performance-ceiling-option-preservation-and-sled-layer.md`](performance-ceiling-option-preservation-and-sled-layer.md) | Perf ceiling vs peers; anti-corner checklist; B-tree reads without dual store; **pedra-map / sled-compat layer** plan |
| [`switch-justification-bar.md`](switch-justification-bar.md) | Guarantees + upsides needed to justify switching to us |
| [`fdb-limitations-analysis.md`](fdb-limitations-analysis.md) | Why PedraDB solves FDB's 4 limitations |
| [`open-items.md`](open-items.md) | This file — living status tracker |
| [`references/`](references/) | All primary sources (papers as PDF+TXT, blog posts, docs) |
