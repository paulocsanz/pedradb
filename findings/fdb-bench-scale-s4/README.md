# fdb-bench-scale-s4 — per-range leader routing

**Date:** 2026-08-15  
**Change:** `TcpClusterClient` multi-Raft routing

## Shipped

- `leaders_from_status` — parse **all** `r*:leader=N`
- Per-range leader map on the client (`range_leaders` / NotLeader updates)
- `active_range` + `with_active_range` for partitioned writers
- `warm_leaders()` after elect
- Bench S2/S3 use `with_active_range(range_id)` + warm

## Lab note (this host under load)

Full suite re-runs were noisy/slow (S1 ~0.5–1 keys/s; machine load).  
Stable prior signal remains **s2c/s3** (r4 multi-client ~2–4× r1 when leaders spread).

Observed residual reconfirmed: when elect places **all leaders on one node**, multi-range multi-client does not scale (single worker queue).

## Reproduce

```bash
MONTANHA_BENCH_SUITE=scale MONTANHA_BENCH_N=16 MONTANHA_BENCH_THREADS=4 \
  cargo run -p pedradb-store --release --bin montanha-fdb-bench -- findings/fdb-bench-scale-s4
```
