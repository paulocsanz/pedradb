# fdb-bench-scale-s10 — thr ≥ min(nr, 8) closes thr≪ranges residual

**Date:** 2026-08-15  
**Env:** N=12, MONTANHA_BENCH_THREADS=4 → thr = max(4, min(nr,8))

## Code

- S2/S3: `thr = max(threads, min(nr, 8))` so r8 runs **8** writers  
- 500ms settle after elect for `rebalance_local_leaders`  
- S3 r8 restored (walls + rebalance; was hang residual)

## Numbers

| Bench | Ranges | thr | keys/s | ok |
|-------|--------|-----|--------|-----|
| S2 | 1 | 4 | 5.81 | 12 |
| S2 | 4 | 4 | 2.47 | 12 |
| S2 | 8 | **8** | 1.30 | 16 |
| S3 PutBatch | 1 | 4 | 0.77 | 48 |
| S3 | 4 | 4 | **2.44** | 96 |
| S3 | 8 | **8** | **2.73** | **128** |

## Read

- **S3 multi-range PutBatch scales with ranges+writers:** r1→r4→r8 ≈ 0.8→2.4→2.7 keys/s (~3.5× r1).  
- S2 single-put still pays per-key fsync; hot r1 can beat multi-range on a lucky run.  
- r8 **no hang** — thr=nr + walls + local rebalance.

Option A remains default; batch+multi-range is the honest capacity path.
