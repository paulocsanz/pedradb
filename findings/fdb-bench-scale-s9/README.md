# fdb-bench-scale-s9 — multiproc local leader rebalance

**Date:** 2026-08-15  
**Env:** N=12 thr=4 release

## Code

`StoreCluster::rebalance_local_leaders` — if this multiproc node holds more than
`ceil(n_ranges/n_members)` leaders, step down excess (prefer non-preferred ranges).
`montanha-tcp` runs it every ~2s after ticks.

## Numbers

| Bench | Ranges | keys/s | ok | notes |
|-------|--------|--------|-----|-------|
| S2 | 1 | 6.46 | 16/16 | hot single-range spike |
| S2 | 4 | 2.61 | 16/16 | leaders multi-node |
| S2 | 8 | 1.06 | 16/16 | thr≪ranges |
| S3 | 1 | 1.39 | 80/96 | wall capped |
| S3 | 4 | **7.23** | **96/96** | **~5.2× S3 r1** |

## Read

S3 multi-range PutBatch **wins hard** when leaders are not single-node (matches S6 peak ~7.76). Local rebalance + diversity keep multiproc from staying fully colocated.
