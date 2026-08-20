# RFC-0044 P2 — async column remeasure

## Hot-box baseline (user-ordered) — 2026-08-20 15:16–15:19

3 paired rounds via `scripts/tikv_ycsb_parity_v0.sh`
(`PEDRA_PARITY_ASYNC=1`, Rocks `ROCKS_PARITY_SYNC=0`), recorded
load **149–154 / 12 CPUs** (caixote-api + 8 NSS fuzzers). NOT quiet —
this is the hot-box baseline, not the P2.1 arbiter.

Median ratio (3 rounds), all shapes with `qps`:

| shape | med | runs |
|---|---:|---|
| ycsb_e | **15.06** | 15.06 / 5.10 / 17.64 |
| ycsb_c | 3.41 | 3.66 / 3.41 / 1.56 |
| ycsb_a | 3.12 | 3.12 / 0.28 / 5.77 |
| ycsb_d | 2.98 | 2.85 / 2.98 / 10.09 |
| ycsb_b | 2.74 | 2.74 / 2.58 / 7.46 |
| deps_cache_overwrite | 2.73 | 2.73 / 3.89 / 0.80 |
| deps_apply_batch | 2.27 | 2.27 / 1.08 / 5.98 |
| deps_lock_prewrite | 1.88 | 1.88 / 0.93 / 3.45 |
| ycsb_f | 1.85 | 1.85 / 2.05 / 1.22 |
| deps_scan | 1.79 | 1.67 / 1.79 / 16.52 |
| deps_mvcc_latest | 1.62 | 1.62 / 1.63 / 0.76 |
| deps_raftlog | 0.74 | 0.74 / 0.38 / 1.41 |

Reading (honest):

- **E ≥ 5 in all 3 rounds even at load 150** — the CountCache fix
  (`3a722e7`) holds under the worst box. E is consistently the top
  shape of the async column.
- F is stable (1.2–2.1) but does not close in 2000-op walls at hot
  box; per-op p50 1.5× / p99 22× (see `rfc0044-p1` per-op note).
- Everything else swings too much at load 150 to call (A 0.28→5.77,
  scan 1.67→16.52): short-window walls are single-outcome lotteries
  under this load. Medians here are NOT standing numbers.
- kvrocks shapes (SET/mc50/GET/blob/pipeline) are not in this suite;
  their crossings stay in `rfc0044-p1` (l14/merge dirs).

Quiet 3× remains the arbiter for standing numbers (P2.1 definition:
load < 10). JSONs: `run{1,2,3}/{compat,rocks,compare}/`.
