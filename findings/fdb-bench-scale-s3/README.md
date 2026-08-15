# fdb-bench-scale-s3 — multi-client scale + PutBatch

**Date:** 2026-08-14  
**Harness:** `montanha-fdb-bench` suite `scale`  
**Env:** `MONTANHA_BENCH_N=16 MONTANHA_BENCH_THREADS=4` release, localhost 3-node TCP

## What changed vs s2c

1. **`elect-wait` multi-range-safe** — waits until **every** `r*:leader` is set (not first only).
2. **S3** — multi-client multi-range TCP `PutBatch` (batch_sz=8).

## Numbers (this host)

| ID | Ranges | keys/s | notes |
|----|--------|--------|-------|
| S1 | 1–8 | ~2.0–2.4 | sequential, flat |
| S2 | 1 | 2.26 | 4 thr single-range |
| S2 | 4 | **4.02** | option A signal |
| S2 | 8 | 0.12 | cliff (noisy) |
| S3 | 1 | **3.15** | PutBatch amortize keys |
| S3 | 4 | 0.84 | concurrent batch residual |
| S3 | 8 | 0.45 | same |

See `fdb_shaped_bench.json` and [docs/rfc/0021-commit-path-scale-decision.md](../../docs/rfc/0021-commit-path-scale-decision.md).

## Reproduce

```bash
MONTANHA_BENCH_SUITE=scale MONTANHA_BENCH_N=16 MONTANHA_BENCH_THREADS=4 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-scale-s3
```
