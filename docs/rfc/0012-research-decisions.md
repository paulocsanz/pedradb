# RFC-0012 companion: measured research decisions (P2.1–P2.2)

**Status:** partially superseded (Bloom shipped under RFC-0014)  
**Updated:** 2026-08-12  

## Decision

| Item | Decision | Evidence gate | Status 2026-08-12 |
|------|----------|---------------|-------------------|
| Bloom filters | **Shipped** (SST v3 + in-memory rebuild for v2) | Correctness/shape for multi-SST get; not thruput-only | **done** — [RFC-0014](0014-rocks-pebble-redwood-maturity.md) P0.1 |
| Value log (WiscKey) | **Shipped minimal (RFC-0014 P2.2)** | `large_value_threshold` + `VALUES.vlog`; GC deferred | **done** threshold spill; GC still open |
| Lazy Leveling | **Do not ship** | Compact remains whole-merge + count/bytes policy | still non-ship; [ficha R007 D4](../../research/fichamentos/ficha_R007_Dayan_Dostoevsky.md) — exige Monkey FPR (L2), piora short range; reabrir só com o gate da ficha |
| MemTable skiplist/arena | **Keep Vec tail + live idx** | Revisit if write path is CPU-bound in MemTable | **keep** — 2026-08-30: apply insert 15 µs of 65.7 µs p50 (23%). RFC-0154 P2.2 REFUSED. P2.3 WAL/prepare 2×64 REFUSED CHV 9/17. |

## How further research re-opens

1. Run `cargo bench -p pedradb-core --bench baseline` and attach numbers.  
2. Open / extend **RFC-0014** P1–P2 (or a new RFC); do not silently revive 0009.

## Delivered

- 2026-08-11: explicit non-ship for Bloom/value-log/LL/skiplist.  
- 2026-08-12: **Bloom reopened and shipped** because negative filters are a
  *correctness-of-shape* requirement for Rocks/Pebble-class engines (skip tables
  that cannot hold a key), not only a bench optimization. Value-log and Lazy
  Leveling remain non-ship until measured.
