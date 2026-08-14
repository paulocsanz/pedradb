# RFC-0023: FDB functional TX parity + dual face (native power + FDB plug-in)

**Status:** done (P0–P2 thin-but-real proofs in-tree; not FDB field peer)  
**Updated:** 2026-08-14  
**Parents:** [RFC-0022](0022-montanha-fdb-functional-parity-and-layer-substrate.md) · [RFC-0017](0017-montanha-fdb-class-substrate.md)  
**Related:** [`../fdb-limitations-analysis.md`](../fdb-limitations-analysis.md) · [`../montanha-vs-foundationdb.md`](../montanha-vs-foundationdb.md)

---

## 0. One sentence

Close gaps where Montanha TX is **worse** than FDB (real snapshot reads + OCC + too-old/GC).  
**Native TX** is the powerful default. **fdb-compat** is a plug/test face only.  
Do **not** regress Pedra embed strengths (no forced FDB 5s/100KB on local-only).

---

## 1. Delivery slices

### P0 — physics

- [x] **P0.1** Isolation contract + anti-read-skew tests — status: `done`
- [x] **P0.2** Real snapshot reads (`get_at_version`) — status: `done`
- [x] **P0.3** Unified OCC on majority commit — status: `done`
- [x] **P0.4** Real TransactionTooOld via version watermark GC — status: `done`
- [x] **P0.5** Single native `Transaction` API default — status: `done`

### P1 — plug + range + watch

- [x] **P1.1** fdb-compat face subset — status: `done`
- [x] **P1.2** Leadership-invisible paths (no node_id in faces) — status: `done`
- [x] **P1.3** Range-read conflict subset — status: `done`
- [x] **P1.4** Versioned watch after majority — status: `done`

### P2 — honesty / stubs

- [x] **P2.1** Cluster vs embed limits documented — status: `done`
- [x] **P2.2** Single-language (Rust) fdb-compat surface (no multi-lang) — status: `done`
- [x] **P2.3** Parallel-commit residual documented (not field path) — status: `done`

---

## 2. Status (living)

| ID | Band | Title | Status | Evidence | Updated |
|----|------|-------|--------|----------|---------|
| P0.1 | p0 | Isolation contract + anti-read-skew | done | `tx_snapshot_hides_concurrent_commit` | 2026-08-14 |
| P0.2 | p0 | Real snapshot reads | done | `get_at_version` + notes flushed on Direct **and** Queued majority (`finish_queued_propose`); `queued_put_advances_versions_for_tx_occ` | 2026-08-14 |
| P0.3 | p0 | Unified OCC majority commit | done | `commit_transaction` + version notes on majority (not coordinator-Ok-only); Queued isolation test | 2026-08-14 |
| P0.4 | p0 | Too-old via watermark GC | done | `tx_too_old_after_gc_watermark` | 2026-08-14 |
| P0.5 | p0 | Native Transaction default | done | `cluster.begin()`; layers use Transaction | 2026-08-14 |
| P1.1 | p1 | fdb-compat face | done | `fdb_compat` + harness tests | 2026-08-14 |
| P1.2 | p1 | Leadership-invisible | done | faces without node_id | 2026-08-14 |
| P1.3 | p1 | Range conflict subset | done | `tx_range_read_conflict` + `keys_in_range_at_after_reopen_sees_pedra` (Pedra scan) | 2026-08-14 |
| P1.4 | p1 | Versioned watch | done | WatchEvent.version + tests | 2026-08-14 |
| P2.1 | p2 | Cluster vs embed limits | done | §Limits below + `docs` pointer | 2026-08-14 |
| P2.2 | p2 | fdb-compat + thin C ABI | done | `fdb_compat` + feature `c-api` (`fdb_c`, `include/montanha_fdb.h`) | 2026-08-14 |
| P2.3 | p2 | Parallel commit residual | done | §Out of scope residual | 2026-08-14 |

**Honesty:** not FDB field peer; not full fdbcli/C API; not Apple Simulation.

---

## 3. Isolation contract (normative)

1. **Begin** captures `read_version` R (= commit generation).  
2. **Get** returns the value as of R (not concurrent commits after begin), plus own write-set.  
3. **Commit** aborts with **Conflict** if any key in read set, write set, or a conflict range was committed at version > R.  
4. **TransactionTooOld** if R < `safe_watermark` (GC).  
5. **Read skew is a bug:** if get sees a post-begin commit and commit still succeeds, the system is wrong.

---

## 4. Limits (cluster vs embed) — P2.1

| Limit | Cluster (Montanha) | Pedra embed-only |
|-------|--------------------|------------------|
| Value size | `MAX_VALUE_BYTES` (100KiB soft, FDB-order) | Large values via vlog; **no** forced 100KB |
| TX size | `MAX_TX_BYTES` / `MAX_TX_KEYS` | Local TX policy in core |
| TX lifetime | Watermark GC / retention generations | No artificial 5s wall clock |
| Snapshot lag | `VERSION_RETENTION` watermark | N/A |

**Do not** impose FDB 5s/10MB/100KB on Pedra-only paths to “look like FDB.”

---

## 5. Out of scope / residual

- Full FDB wire, fdbcli, multi-language C ABI (P2.2 = Rust face only).  
- Parallel commit / unbundled proxies (P2.3 residual — future).  
- Zero-downtime range-leader HA (orthogonal).  
- Field pedigree / Simulation Apple-scale.

---

## 6. Relationship

| RFC | Role |
|-----|------|
| **0022** | N-writer layers/faces; points here for **TX physics parity** |
| **0023** | Real FDB-class TX + native API + fdb-compat plug face |
