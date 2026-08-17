# RFC-0041 — one L0 materialize per 5 ms tick (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`onel0/run{1,2,3}`). JSONs only. Peers `sync: false`.

Park imm, then write **at most one** L0 per worker tick (writes or idle).
Compact only when parked is empty.

## Result

**0/16 ≥ 2.0.** apply_mc4 **0.681** (Pedra 1.9 k) — lz4 back on the apply
path. MVCC 1.40. Scan 0.43 (12 L0s, `scan_sst_probed=10098` on run1).

Rejected. Worker returns to `drain_imm_once` during writes (retire2
apply/MVCC) + scan-via-SST (no retired BTree merge) + idle compact.
FLOOR not enabled.
