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

## kvrocks long-window same-run (2M ops) — 2026-08-20 16:44

User-ordered at load ~100 (fuzzers niced; bench at normal priority).
`ROCKS_YCSB_OPS=2000000`, `ROCKS_PARITY_ONLY=set,get,pipelined_set`,
async column (`PEDRA_PARITY_ASYNC=1`) vs Rocks `sync=false`:

| shape | Pedra | Rocks | ratio | p50 |
|---|---:|---:|---:|---|
| kvrocks_get | 5.72 M | 1.04 M | **5.50** | 0.0 µs vs 0.5 µs |
| kvrocks_pipelined_set | 114 k | 19 k | **5.86** | 4.3 µs vs 28.5 µs |
| kvrocks_set | 830 k | 173 k | 4.79 | **0.5 µs vs 3.0 µs (6×)** |

Independent confirmation of the P1.3 long-window evidence (20 M ops
4.4–5.5×): **GET ≥ 5 in a second independent long window**. Pipeline
≥ 5 again (P1.1). SET stays just under at 4.79 with a 6× p50 — same
signature: the wall is window/harness, the engine dominates per-op.
Short-window 3-round medians same session (default ops): scan 8.17,
pipeline 4.71, SET 4.46, mc50 3.55 (swings 1.8–7.0), blob 2.03,
GET 1.53. JSONs: `kvrocks-long/{compat,rocks}/` (short-window 3-round
raw in `kvrocks-short/`).

**Not official** (dirty box); the quiet <10 run remains the arbiter
for standing 0041 numbers. But GET/pipeline crossing ≥ 5 in two
independent long windows is strong evidence the 5× is real for these
shapes — the floor just needs a fair window.
