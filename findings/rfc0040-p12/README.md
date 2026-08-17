# RFC-0040 P1.2 — sticky group between apply pre+com

2026-08-17. Same harness as P1.1 (`scripts/tikv_ycsb_parity_mc_async.sh`, N=4).

## What changed

Lone-writer fast path was firing **between** `batch(pre)` and `batch(com)` (`active` drops to 1). Each of 4 clients forced its own `fdatasync` on the second write.

Fix: after `active > 1`, keep the fast path off for 250 µs (`MULTI_HOLD`). Leader still only waits the 50 µs catch-up. `group_commit` now **moves** `BatchOp`s instead of cloning them.

## Diagnostics

| run | avg_group | apply MC vs async | raftlog MC vs async |
|---|---:|---:|---:|
| p11 median (before) | ~1.1 (fast path) | 0.93 | 0.61 |
| p12 run1 | 1.55 | 0.54 | 0.75 |
| p12 run2 | 1.63 | **0.97** | 0.66 |
| p12 catch200 (`PEDRA_CATCHUP_US=200`) | 1.70 | 0.69 | 0.74 |

Catch-up 200 µs grows the group a little and **hurts** qps (leader waits more than one fd). Default stays 50 µs.

## Honest close

P1.2 **does not** stably put apply MC ≥ 1.0× async. Grouping is real (1.55 vs ~1.1). The leftover is CPU under the write lock + compact tails (max 40–120 ms), not “we forgot to group.” Next: P2.2 pipeline encode∥fsync no host, or P2.1 scan L0.

Raw: `run{1,2}/{compat,rocks-sync,rocks-async}/rocks_parity_bench.json`.
