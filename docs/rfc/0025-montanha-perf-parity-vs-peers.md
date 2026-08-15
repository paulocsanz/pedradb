# RFC-0025: Montanha performance, ergonomics, efficiency & capacity vs peers

**Status:** done (P0–P2 slices landed; continuous re-measure)  
**Updated:** 2026-08-14  
**Parents:** [0016](0016-pedradb-production-robustness.md), [0021](0021-montanha-fdb-tikv-parity-gaps.md), [0021-commit-path](0021-commit-path-scale-decision.md), [perf doctrine](../performance-ceiling-option-preservation-and-sled-layer.md), [bench method](../montanha-vs-fdb-bench.md)

## Background

- Functional FDB-layer seeds (phases 1–3, A–E) and mini-bt CI gates are green.
- Lab benches show **write cliffs**: single-key durable put ~0.5–1s p50 / ~1–2 qps on laptop in-process 3-node majority; cross-range 2PC multi-second; get/range cheap.
- Peers (FDB, TiKV, etcd, Rocks-class embeds) win on different axes; we need **honest targets** and a **wave plan**, not a claim of overnight field parity.

## Problems This Solves

- **Problem:** No written map of perf/ergonomics/capacity gaps vs peers with shippable slices.
- **Problem:** Layers and benches default to single-key `put`, which multiplies Raft+WAL fsyncs; **batch same-range** path exists but is under-used.
- **Problem:** No first-class “capacity / bulk load” open mode (Pedra `sync=false`) for Montanha lab without forking open paths.
- **Problem:** Cross-range TX cost is unguided (layers co-locate hot keys? how many ranges?).

## Proposed Solution

1. **Measure** continuously (`montanha-fdb-bench`, mini-bt CI) — already started.  
2. **Ergonomics:** make the fast path the easy path (`put_many` range-grouped → `put_batch`; docs).  
3. **Efficiency:** amortize fsync/Raft entry cost (batch, group commit, log compact, later pipeline).  
4. **Capacity:** more ranges (N writers), open modes for bulk, TCP multi-client scale-out story.  
5. **Honesty:** never claim FDB field QPS; report **ratios** (batch/single, cross/same, TCP/inproc).

## Peer axes (what “parity” means)

| Axis | FDB | TiKV | etcd | Montanha target (mature) |
|------|-----|------|------|---------------------------|
| Point durable write | proxy+disk | Raft+Rocks | Raft+bolt/WAL | Majority Raft+Pedra; **batch amortize** |
| Multi-key TX same shard | OCC cheap | region TX | single key mostly | Snapshot TX / put_batch |
| Multi-shard TX | GRV+resolve | 2PC / latches | n/a | 2PC; **co-locate first** |
| Concurrent clients | multi-proxy | multi-region | multi-client | multi-range + TCP clients |
| Embed local ACID | n/a | n/a | n/a | **Pedra win** |
| Ops maturity | Simulation | PD/tools | simple | DST + mini-bt + runbooks |

## Delivery slices (mandatory)

### P0 — must ship first (useful alone)

- [x] **P0.1** Range-grouped `put_many` + lab `StoreOpenOptions { pedra_sync }` — status: `done`  
- [x] **P0.2** Bench A1b/A1c: batch vs single put + open_items/phases note — status: `done`  
- [x] **P0.3** RFC + living Status (this doc) — status: `done`  

### P1 — next wave

- [x] **P1.1** Write coalesce: `put_buffered`/`flush_writes`/`put_coalesce` + TCP worker Put batching — status: `done`  
- [x] **P1.2** Raft log persist: contiguous append → segment keys + one `apply_batch` (not full blob rewrite) — status: `done`  
- [x] **P1.3** TCP `PutBatch` wire + `client_put_batch` / `TcpClusterClient::put_batch` — status: `done`  
- [x] **P1.4** Layer defaults: `table_put` via put_batch; secondary index already TX; docs point to put_many — status: `done`  

### P2 — later / polish

- [x] **P2.1** Optional FDB comparator (`montanha-fdb-compare`, template if no fdbcli) — status: `done`  
- [x] **P2.2** Scale probe S1 ranges 1/2/4/8 + reconfirm option A — status: `done`  
- [x] **P2.3** Read capacity doc + `get_strong`/`get_fast_replica` tests/bench B4 — status: `done`  

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | put_many + StoreOpenOptions | done | 0025 | 2026-08-14 |
| P0.2 | p0 | batch vs single bench | done | 0025 | 2026-08-14 |
| P0.3 | p0 | RFC living status | done | 0025 | 2026-08-14 |
| P1.1 | p1 | write coalesce / TCP batch Puts | done | put_buffered + tcp coalesce | 2026-08-14 |
| P1.2 | p1 | raft log append cost | done | incremental log_e + log_hi | 2026-08-14 |
| P1.3 | p1 | TCP PutBatch wire | done | tag 17 + client | 2026-08-14 |
| P1.4 | p1 | layer batch defaults | done | table_put/docs | 2026-08-14 |
| P2.1 | p2 | FDB side comparator | done | montanha-fdb-compare | 2026-08-14 |
| P2.2 | p2 | scale A vs B decision | done | suite scale + 0021 update | 2026-08-14 |
| P2.3 | p2 | strong RO capacity | done | montanha-read-capacity.md | 2026-08-14 |

## Acceptance Criteria

- **Tests:** unit for `put_many` same-range + multi-range split; open with `pedra_sync=false` still elects/puts (lab only).  
- **Telemetry:** bench JSON fields `A1_raw_put`, `A1b_put_batch_*`, `A1c_put_many_*` with qps/p50.  
- **Documentation:** this RFC + open-items link + montanha-vs-fdb-bench ratio note.  
- **Screenshots:** backend-only.

## Out of scope

- Claiming production FDB/TiKV field QPS.  
- Full unbundled FDB roles (proxy/resolver/storage) without P2.2 numbers.  
- Dropping majority durability as default.  
