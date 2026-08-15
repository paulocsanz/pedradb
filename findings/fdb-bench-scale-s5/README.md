# fdb-bench-scale-s5 — multi-Raft election timeout diversity

**Date:** 2026-08-15  
**Commit:** `30e8c9d` (+ this findings update)

## Root cause (fixed)

`election_timeout = 4 + node_id` → node 1 won **every** range. Multi-client multi-range looked flat because all leaders shared one worker.

## Fix

Per-`(node, range)` timeouts: preferred leader = `members[(r-1) % n]`; ring-stagger others.  
APIs: `leader_nodes()`, `rebalance_range_leaders`.

## Elect proof (this run)

```
S2 r4: r1=1 r2=2 r3=3 r4=1     # 3 distinct leader nodes
S2 r8: r1=1 r2=2 r3=3 r4=1 r5=2 r6=3 r7=1 r8=2  # round-robin
```

Previously common: `r1=1 r2=1 r3=1 r4=1`.

## Throughput (N=16, thr=4, release, this host under load)

| Bench | Ranges | keys/s | ok | notes |
|-------|--------|--------|-----|-------|
| S1 | 1–8 | ~0.8–1.0 | 16 | sequential, flat |
| S2 | 1 | 1.10 | 16/16 | multi-client single range |
| S2 | 4 | 0.38 | 16/16 | leaders **spread**; still slow (HB tax / load) |
| S2 | 8 | 0.08 | 16/16 | more groups → worse wall |
| S3 | 1 | 0.09 | 56/128 | PutBatch partial under load |
| S3 | 4 | 0.13 | 104/128 | slightly better ok-rate than r1 |
| S3 | 8 | — | aborted | hung / multi-minute wall |

**Read:** diversity is **proven** (elect map). Aggregate QPS did **not** recover s2c-era wins on this run — residual is multi-Raft HB + fsync cost per group under laptop load, not “all leaders on node 1”. Prior s2c (~4–8 keys/s multi-range) remains the better-load reference.

S3 r8 was stopped after ~20+ min without completion; `stdout-partial.txt` has the log.

## Reproduce

```bash
MONTANHA_BENCH_SUITE=scale MONTANHA_BENCH_N=16 MONTANHA_BENCH_THREADS=4 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-scale-s5
```
