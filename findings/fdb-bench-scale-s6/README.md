# fdb-bench-scale-s6 — scoped put-path ticks + leader diversity

**Date:** 2026-08-15  
**Env:** `MONTANHA_BENCH_N=12 MONTANHA_BENCH_THREADS=4` release

## Code

1. Leader diversity (`30e8c9d`) — per-range election timeouts  
2. **This:** `tick_range_id` + TCP finish/pump only ticks the **active** range while waiting for majority (not all multi-Raft groups)

## Numbers

| Bench | Ranges | keys/s | vs r1 | leaders |
|-------|--------|--------|-------|---------|
| S2 | 1 | 0.77 | 1× | r1=1 |
| S2 | 4 | **1.36** | **~1.8×** | r1=1 r2=2 r3=3 r4=1 |
| S2 | 8 | 0.99 | ~1.3× | round-robin (thr=4 uses 4 ranges) |
| S3 PutBatch b=8 | 1 | 3.03 | 1× | |
| S3 | 4 | **7.76** | **~2.6×** | spread leaders |
| S3 | 8 | — | aborted after long wall | node-2 heavy skew |

## Read

- **Option A reconfirmed** with honest multi-client + spread leaders + scoped ticks.  
- S3 r4 is the clean win: batch amortize **and** multi-leader parallelism.  
- S3 r8 still fragile when leaders re-skew / thr≪ranges.

Compare S5 (diversity only, full tick on wait): S2 r4 was **0.38** keys/s; S6 **1.36**. S3 r4 was ~0.13; S6 **7.76**.
