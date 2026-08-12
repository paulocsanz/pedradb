# RFC-0019: Local primitive completeness for platform + Scylla-need

**Status:** implemented (P0–P2.2)  
**Updated:** 2026-08-12  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Builds on:** [RFC-0014](0014-rocks-pebble-redwood-maturity.md), [RFC-0015](0015-audit-pedradb-correctness-fixes.md), [RFC-0016](0016-pedradb-production-robustness.md) (P0 done)  
**Feeds:** [RFC-0017](0017-montanha-fdb-class-substrate.md) (Montanha multi-Raft), watch/CDC layers, SQL face  
**Doctrine / product:** [`../node-primitive-and-unified-platform.md`](../node-primitive-and-unified-platform.md) §1 (Postgres-class face; replace Scylla+CH+NATS *need*), [`../scylla-need-replacement.md`](../scylla-need-replacement.md) §5.1 (L1 checklist)

---

## Background

Platform north star: a **high-level Postgres-class system** that removes the operational need for **Scylla** (horizontal CP KV + push), **ClickHouse** (logs/telemetry scan), and **NATS** (streams) — multi-primary multi-region, automatic paths ([node-primitive](../node-primitive-and-unified-platform.md)).

**PedraDB** is only the **per-node SoR**: ordered KV + multi-key TX + durable log. Horizontal scale, any-node accept, watch fanout, and SQL are **layers**. If the local kernel lacks CAS, a stable sequence pin, and a post-commit change seam, every region leader and every watch service reinvents fragile RMW and half-CDC.

### What L1 already has (do not rebuild)

| Capability | Status |
|------------|--------|
| Durable ordered get/put/delete | done |
| Multi-key TX + `apply_batch` | done |
| Snapshot / `get_at` / OCC | done |
| Range / `scan` / limit | done |
| Bloom, lazy blocks, cache, leveled compact | done |
| Group commit + dual-mem | done |
| Vlog + `compact_vlog` | done (0016 P0) |
| Checkpoint / ship_wal / backup engine | done (ops) |
| Env / FailingEnv / soak silent_wrong (baseline) | done |

### What still blocks “L1 ready for Scylla-need + platform”

From [scylla-need-replacement §5.1](../scylla-need-replacement.md):

| Gap | For | Priority |
|-----|-----|----------|
| First-class CAS / put_if | Replace LWT without fragile RMW | **P0** |
| Stable seq pin on API (commit returns seq) | Watch watermark / lag | **P0** |
| Post-commit change-feed seam | CDC/watch without Scylla | **P0** |
| `multi_get` | DNS/discovery N keys | **P1** |
| Scan project (key-only) | Watch rebuild / cheap listings | **P1** |
| Apply soak under fault + group-commit p99 evidence | Leader under write storm | **P1** |
| Continuous backup under load (0016 P2.1) | Fleet ops | **P2** (shared with 0016) |

When this RFC’s P0 is **done**, Pedra is no longer the missing piece for Montanha apply + watch; L2/L3 product work can proceed without core footguns.

---

## Problems this solves

- **Problem:** Control-plane CAS (lease, IF NOT EXISTS, FSM generation) requires ad-hoc read-modify-write races on Pedra.  
- **Problem:** Watch/CDC layers cannot pin a durable watermark without a **documented, returned sequence** on every successful commit/apply.  
- **Problem:** No first-class **post-commit change surface** → every subscriber re-parses WAL or polls ranges (Scylla CDC role unfilled).  
- **Problem:** Discovery-style **N point reads** pay N full get paths.  
- **Problem:** Watch rebuild / key listing loads full values when only keys matter.  
- **Problem:** Leader-node apply under fault + concurrent commits lacks a **named soak + p99** gate.  
- **Problem:** Continuous backup under write load still open (ops for fleets that replace Scylla).

---

## Proposed solution

Close L1 gaps with a **compact API** (no CQL, no watch network, no multi-Raft in core):

1. **`compare_and_swap` / `put_if`** — single-key conditional put (expected value or absence); durable same as put; fail closed on mismatch.  
2. **Commit / apply returns `SequenceNumber`** — stable, monotonic, documented; same seq visible to `get_at` / change feed.  
3. **Change feed seam** — after durable commit, layer can iterate or subscribe to **logical changes** `(seq, key, op, value?)` for a seq range (or tail); hole-detectable; **no second fsync as commit gate** (CHANGELOG persist is best-effort; WAL is SoR and rebuilds feed on open).  
4. **`multi_get` / `multi_get_at`** — batch point lookup; correct vs N×get.  
5. **`scan` projection** — at least `KeyOnly` (optional `ValueLen`); full remains default.  
6. **Apply/commit soak** under FailingEnv + report group-commit p99/sync counts under concurrent load.  
7. **Backup under continuous put** — ship/checkpoint without wrong live reads (align 0016 P2.1).

Montanha/watch/SQL stay **out of this RFC** except as consumers of (1)–(3).

---

## Delivery slices

### P0 — L1 no longer blocks watch + CAS + apply (shippable alone)

- [x] **P0.1** First-class conditional put: `compare_and_swap` and/or `put_if_absent` / `put_if_eq` — durable; concurrent mismatch → explicit error (not silent lost update) — status: `done`  
- [x] **P0.2** All successful write paths return or expose **commit sequence** (`put_with`/`delete_with`/`commit`/`apply_batch`/`group_commit`); rustdoc + usage: seq is the layer pin — status: `done`  
- [x] **P0.3** Post-commit **change feed** seam: read changes for `(from_seq, to_seq]` or tail after durable commits; crash: no ghost changes ahead of durable seq; deletes visible — status: `done`  
- [x] **P0.4** Docs: L1 readiness for Scylla-need + platform (link checklist in scylla-need §5.1; usage snippets CAS + seq + feed) — status: `done`  

### P1 — hot-path reads + leader proof

- [x] **P1.1** `multi_get` / `multi_get_at` — same visibility as get; test parity with N×get — status: `done`  
- [x] **P1.2** Scan projection **KeyOnly** on public scan API — status: `done`  
- [x] **P1.3** Soak: concurrent apply_batch under load + FailingEnv apply/CAS; silent_wrong=0; record wal_sync_count under group commit (CI-bounded) — status: `done`  

### P2 — fleet ops (shared with 0016)

- [x] **P2.1** Continuous backup / ship_wal under continuous puts; restore sees only acked prefix — status: `done` (also RFC-0016 P2.1)  
- [x] **P2.2** Optional: `compact_for_reads` for read-heavy CP prefixes after write bursts — status: `done`  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CAS / put_if first-class | done | core + ConcurrentDb | 2026-08-12 |
| P0.2 | p0 | Commit/apply returns stable seq pin | done | put_with / tx.commit | 2026-08-12 |
| P0.3 | p0 | Post-commit change feed seam | done | CHANGELOG + changes* | 2026-08-12 |
| P0.4 | p0 | L1 readiness docs | done | usage + scylla-need + node-primitive | 2026-08-12 |
| P1.1 | p1 | multi_get | done | multi_get / multi_get_at | 2026-08-12 |
| P1.2 | p1 | Scan KeyOnly projection | done | ScanProjection::KeyOnly | 2026-08-12 |
| P1.3 | p1 | Apply soak + group-commit evidence | done | core + sim tests | 2026-08-12 |
| P2.1 | p2 | Backup under load | done | ops continuous put+ship | 2026-08-12 |
| P2.2 | p2 | compact_for_reads (optional) | done | Db::compact_for_reads | 2026-08-12 |

---

## Acceptance criteria

### Tests

**P0**

- [x] CAS: put_if_absent succeeds once; second fails; after delete succeeds again; crash after Ok → recover sees winner only.  
- [x] CAS: put_if_eq(old, new) fails if concurrent overwrite; no lost update under two threads.  
- [x] put / apply_batch / tx.commit return seq; `get_at(seq)` sees commit; `get_at(seq-1)` does not see it.  
- [x] Change feed: after batch of puts/deletes, iterate `(from, last_seq]` matches model; no entries with seq > durable last; reopen continues from pin.  
- [x] Mid-crash during commit: feed and get never show partial multi-key TX (TX all-or-nothing + feed after durable commit).

**P1**

- [x] multi_get equals sequential get for same snapshot (incl. missing keys, large vlog values).  
- [x] KeyOnly scan: keys match full scan; values empty by contract.  
- [x] Soak apply+FailingEnv: silent_wrong=0; wal_sync_count recorded in test output.

**P2**

- [x] Continuous put + ship_wal; restore prefix ⊆ acked model.

### Telemetry / analytics

- Existing `DbStats` (wal_sync_count, bytes_*, vlog_*).  
- Optional: change_feed_reads counter — not required for P0.

### Documentation

- This RFC status table.  
- [`../usage.md`](../usage.md): CAS, returned seq, change feed consumer sketch.  
- [`../scylla-need-replacement.md`](../scylla-need-replacement.md) §5.1: tick L1 checklist as slices land.  
- [`../node-primitive-and-unified-platform.md`](../node-primitive-and-unified-platform.md) §5: mark planned primitives done when shipped.

### Screenshots

- backend-only.

---

## Out of scope

- Multi-Raft, PD, any-node proxy network (RFC-0017 / Montanha).  
- Watch **network** fanout protocol (L3 product).  
- Columnar engine / SQL planner / CQL.  
- Dual primary B-tree, merge-operator zoo, CF zoo.  
- Claiming wire-compatible Postgres or Scylla drop-in.  
- Full HTAP learner implementation (hooks only; HTAP product uses P0.2–P0.3).

---

## Relationship to north star

| North star piece | This RFC |
|------------------|----------|
| Postgres-class TX / product tables | CAS + multi-key TX (existing) + seq for layers |
| Scylla-need (CP KV + push substrate) | CAS + seq pin + **change feed** + multi_get |
| ClickHouse-need (later OLAP path) | seq pin + feed for catch-up; not column store here |
| NATS-need (streams) | seq + feed + existing stream layer on keys |
| Montanha apply | apply_batch + returned seq + deterministic recover |

**P0 of this RFC = “L1 is not the reason we still need Scylla.”**

---

## Implementation notes (thin — not a full design dump)

- Prefer implementing CAS as a **single WAL record** (one seq) with read of expected value under the write lock / OCC rules — not a public multi-step API.  
- Change feed may be **WAL-derived logical records** or a small **in-memory ring + durable cursor** for recent seq; must not require second fsync beyond existing commit.  
- Returning seq may be a small breaking API change (`Result<SequenceNumber>` vs `Result<()>`) or additive `*_with_seq` methods — prefer **additive** to avoid churn, but document one canonical path.  
- Coordinate P2.1 with RFC-0016 status when shipping.

---

## See also

- L1 checklist: [scylla-need-replacement §5.1](../scylla-need-replacement.md)  
- Platform objective: [node-primitive §1](../node-primitive-and-unified-platform.md)  
- HTAP triangle / feed subscribers: [htap-storage-primitives-and-research.md](../htap-storage-primitives-and-research.md) §10.2  
- Robustness residual: [RFC-0016](0016-pedradb-production-robustness.md) P1.4 / P2.1  
