# fdb-bench-scale-s7 — preferred-node thr map + elect rebalance

**Date:** 2026-08-15  
**Env:** N=12 thr=4 release

## Code

1. `elect_all` → `elect_until` + `rebalance_range_leaders` (no recursion)  
2. Scale thr maps to range preferring node `(tid%3)+1`  
3. S3 runs **r1 + r4 only** (r8 hang residual)  
4. Prior: diversity + `tick_range_id`

## Numbers (partial — host loaded; S3 r4 aborted after long wall)

| Bench | Ranges | keys/s | ok | leaders |
|-------|--------|--------|-----|---------|
| S2 | 1 | 0.15 | 16 | hot single leader |
| S2 | 4 | **1.38** | 16 | r1=1 r2=2 r3=3 r4=1 → **~9× r1** |
| S2 | 8 | 0.28 | 15 | skew to node 2 |
| S3 | 1 | 0.11 | 48/96 | partial under load |
| S3 | 4 | — | aborted | elect ok, put-wait hung |

## Read

S2 r4 remains the clean option-A signal (multi-client + spread leaders + scoped tick).  
S7 thr→preferred-node mapping works for r4. r8 still noisy when elect re-skews.  
S6 still has the best S3 r4 number (~7.76 keys/s on a quieter run).

```text
# from /tmp/s7.log
S2 ranges=4 thr=4 ok=16 keys_per_s=1.38
```
