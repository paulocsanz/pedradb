# fdb-bench-scale-s5 — multi-Raft election timeout diversity

**Date:** 2026-08-15

## Root cause

`RangePeer` used `election_timeout = 4 + node_id` → **node 1 always timed out first on every range**, so multi-range elect often put all leaders on one process (single worker queue; option-A false negative).

## Fix

Per-`(node, range)` timeouts: preferred leader for range `r` is `members[(r-1) % n]` (shortest timeout); others stagger by ring distance.

- `election_timeout_for`
- `leader_nodes()` / `rebalance_range_leaders`
- unit: `multi_range_election_timeouts_diversify_leaders`

## TCP smoke (this host)

```
elected all ranges (r1=1 r2=2 r3=3 r4=1)
```

Three distinct leader nodes for 4 ranges (was often `r*=1` only).

## Bench note

Full `suite=scale` re-run hit **disk full** mid-S1 on this machine; cluster data dirs under prior `fdb-bench-scale*` were cleaned. Re-run when free space allows; code + unit/TCP elect proof stand alone.
