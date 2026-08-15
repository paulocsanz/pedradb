# fdb-bench-scale-s8 — wall deadlines + full clean run

**Date:** 2026-08-15  
**Env:** N=12 thr=4 release; ~2 min wall for full suite

## Code

- S2 worker: **45s wall**, `max_attempts=8`  
- S3 worker: **60s wall**, `max_attempts=8`  
- Prior: diversity, `tick_range_id`, preferred-node thr map, S3 r1+r4 only

## Numbers

| Bench | Ranges | keys/s | ok | notes |
|-------|--------|--------|-----|-------|
| S1 | 1–8 | 1.5–2.0 | 12 | sequential |
| S2 | 1 | 1.80 | 16/16 | multi-client |
| S2 | 4 | **1.94** | 16/16 | ~1.08× r1; leaders spread |
| S2 | 8 | 0.72 | 16/16 | thr≪ranges residual |
| S3 | 1 | **5.57** | 96/96 | PutBatch amortize |
| S3 | 4 | 5.10 | 96/96 | leaders skewed → ~r1 |

## Read

- **No hang** — walls cap put/batch wait (S3 finished clean).  
- S2 r4 still shows option-A multi-client win when leaders spread.  
- S3 r4 = r1 when leaders re-skew to one node; batch amortize still wins vs S2 (~3× keys/s).  
- S6 remains peak S3 multi-range (~7.76) when leaders stayed spread.

```bash
MONTANHA_BENCH_SUITE=scale MONTANHA_BENCH_N=12 MONTANHA_BENCH_THREADS=4 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-scale-s8
```
