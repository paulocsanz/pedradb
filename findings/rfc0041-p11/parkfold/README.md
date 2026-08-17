# RFC-0041 — park + pairwise fold, clone under read lock (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`parkfold/run{1,2,3}`). JSONs only. Peers `sync: false`.

Park imm during writes (no lz4). Fold two parked BTrees per tick.
Materialize+compact only after 200 ms idle. Scan merges mem, not L0.

## Result

**2/16 ≥ 2.0** (`ycsb_e` **2.218**, `deps_scan` **1.827** — scan Pedra
368–401 k, L0=0, `scan_sst_probed=0`).

MVCC median **0.422** (run1/3 max 16–18 ms): fold deep-cloned the BTree
**under the read lock**. apply 1c 1.85 k (max 206 ms), same stall.
apply_mc4 1.324 (Pedra 3.3 k).

**Rejected** as the fold implementation. Mechanism kept; clone moved
off-lock via `Arc<MemTable>` in [`parkfold2/`](../parkfold2/README.md).
FLOOR not enabled.
