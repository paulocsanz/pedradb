# RFC-0044 P2 — async column remeasure

## QUIET ARBITER (P2.1) — 2026-08-20 16:53–16:55, load 9.4/12

User killed the fuzzers; battery auto-fired when 1-min load < 10
(`loads.txt`: reached 9.4 at 16:53:22; user's `caixote-api` + system
daemons still resident ~4 CPUs — quiet by the P2.1 bar, recorded as
such). Full battery: 3× v0 (ycsb+deps), 3× kvrocks default, 1×
kvrocks 2M long-window, all paired `PEDRA_PARITY_ASYNC=1` vs Rocks
`sync=false`.

### v0 (ycsb+deps), median of 3 rounds

| shape | med | runs | ≥5? |
|---|---:|---|:---:|
| ycsb_e | **10.55** | 10.55 / 12.76 / 10.47 | **sim (3/3)** |
| ycsb_c | 3.84 | 3.87 / 3.84 / 3.76 | não |
| deps_cache_overwrite | 3.43 | 3.48 / 3.12 / 3.43 | não |
| ycsb_d | 3.02 | 3.02 / 3.59 / 3.02 | não |
| ycsb_b | 2.93 | 2.93 / 3.35 / 2.93 | não |
| ycsb_a | 2.88 | 2.59 / 5.80 / 2.88 | não (1 outlier) |
| deps_scan | 1.96 | 1.96 / 1.96 / 1.93 | não |
| deps_mvcc_latest | 1.67 | 1.62 / 1.69 / 1.67 | não |
| ycsb_f | 1.66 | 1.66 / 2.85 / 1.45 | não |
| deps_raftlog | 1.51 | 1.24 / 1.51 / 2.31 | não |
| deps_apply_batch | 1.41 | 1.62 / 1.41 / 1.35 | não |
| deps_lock_prewrite | 0.94 | 1.40 / 0.94 / 0.77 | **abaixo de 1** |

### kvrocks default window, median of 3 rounds

| shape | Pedra | Rocks | ratio | runs |
|---|---:|---:|---:|---|
| scan | 809 k | 109 k | **7.39** | 7.62 / 7.26 / 7.45 |
| set | 1.84 M | 399 k | 4.61 | 4.59 / 4.47 / 4.99 |
| blob_set | 194 k | 96 k | 2.02 | 2.47 / 1.80 / 2.02 |
| pipelined_set | 116 k | 56 k | 2.05 | 4.35 / 2.05 / 1.74 |
| get | 3.82 M | 2.38 M | 1.61 | 1.73 / 1.51 / 1.64 |
| set_mc50 | 307 k | 152 k | **2.02** | 2.13 / 1.92 / 1.88 |

### kvrocks 2M long window, same-run

| shape | Pedra | Rocks | ratio | p50 |
|---|---:|---:|---:|---|
| set | 1.83 M | 338 k | **5.41** | 0.4 µs vs 2.5 µs |
| get | 11.6 M | 2.52 M | 4.61 | 0.0 µs vs 0.4 µs |
| pipelined_set | 166 k | 39 k | 4.22 | 4.0 µs vs 23.2 µs |

### Verdicts (the arbiter's word)

- **Fecham ≥5 consistentes: `ycsb_e` (10.5–12.8, 3/3 em toda condição
  testada) e `kvrocks_scan` (7.4–8.5).**
- **Cruzam na janela longa quieta mas ficam ~4.2–4.6 na curta/curta:
  SET (5.41 longo / 4.61 curto), e straddle entre janelas: GET
  (4.61 quieta / 5.50 a load 100), pipeline (4.22 / 5.86).** p50
  sempre 5–6× melhor (0.4 vs 2.5 µs; 4.0 vs 23 µs) — o wall é cauda.
- **Não fecham: mc50 2.02, blob 2.02, F 1.66, A 2.88, B 2.93, C 3.84,
  D 3.02, deps 0.94–3.43.**
- **P0.5 (mc50) NÃO fecha vs peer são**: os 3.38–9.24 anteriores eram
  Rocks doente sob carga (55 k); quieto e saudável o Rocks faz 152 k e
  o ratio real é ~2.0. O bypass/grupo já é o formato certo (merge
  rejeitado 0.19×); o gap é outro mecanismo.
- `deps_lock_prewrite` abaixo de 1 (0.94 med) — único shape perdendo.

**Standing P2.1 verdict: só E e scan fecham ≥5 amplamente; SET cruza
em janela longa quieta; GET/pipeline straddle 4.2–5.9; o resto está
1.0–3.8.** O piso ≥5 de RFC-0044 para *todos* os shapes não está
alcançado na caixa quieta — registrado sem maquiagem. JSONs:
`quiet/{run1..3,kvrocks-r1..3,kvlong-2m}/`, `loads.txt`.

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
