# RFC-0062 P0.4 — Linux 4 vCPU coluna A, LAST_CF write-through

**Veredito:** `RESULT=P04_PASS` `min_ratio=1.014`
**Quando:** deploy 16:21Z → `P04_DONE` 16:33:51Z
**VM:** `linux-gate-f208` (4 vCPU / 4 GB, brasil, Threadripper PRO 3975WX)
**Imagem:** `ghcr.io/paulocsanz/pedradb-linux-gate:p04a`
**Coluna:** A — `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.
ops=2000 zipfian, `ROCKS_PARITY_BIG=0`, `rm` após cada processo.

Corte vs P0.3 (min 0.745 suite / 0.958 isolado): write-through `LAST_CF` no
`write_cf_owned` (get `idx-1` sem encode+lock). `pwrite(2)` já estava no
P0.3.

## Scoreboard oficial (3 rounds)

| shape | min | mediana | rounds | vs 1.0 |
|---|---:|---:|---|---|
| ycsb_a | 1.447 | 2.238 | 2.238 / 1.447 / 2.778 | PASS |
| ycsb_b | 1.460 | 2.259 | 2.985 / 1.460 / 2.259 | PASS |
| ycsb_c | 2.212 | 3.400 | 3.580 / 2.212 / 3.400 | PASS |
| ycsb_d | 1.706 | 3.028 | 3.231 / 1.706 / 3.028 | PASS |
| ycsb_e | 7.179 | 12.599 | 12.599 / 7.179 / 15.746 | PASS |
| ycsb_f | 1.065 | 2.298 | 2.310 / 1.065 / 2.298 | PASS |
| deps_cache_overwrite | 2.608 | 2.773 | 2.608 / 3.697 / 2.773 | PASS |
| deps_lock_prewrite | 1.385 | 1.864 | 1.385 / 1.945 / 1.864 | PASS |
| deps_mvcc_latest | 3.181 | 3.292 | 3.707 / 3.181 / 3.292 | PASS |
| deps_apply_batch | 1.655 | 1.680 | 1.655 / 1.720 / 1.680 | PASS |
| **deps_raftlog** | **1.014** | **1.244** | 1.503 / 1.244 / **1.014** | **PASS** |
| deps_scan | 2.955 | 3.094 | 3.094 / 3.125 / 2.955 | PASS |
| kvrocks_get | 4.050 | 4.753 | 5.855 / 4.050 / 4.753 | PASS |
| kvrocks_set | 1.981 | 2.578 | 3.379 / 1.981 / 2.578 | PASS |
| kvrocks_scan | 39.070 | 43.655 | 43.655 / 39.070 / 47.651 | PASS |
| kvrocks_pipelined_set | 3.305 | 3.639 | 4.083 / 3.305 / 3.639 | PASS |
| kvrocks_blob_set | 2.342 | 2.395 | 2.342 / 2.395 / 2.486 | PASS |

**17/17 min > 1.0.** O piso é o raftlog 1.014 (r3). Não é 2×; o p50
Linux isolado P0.3 já empatava. Fonte: serial `caixote logs linux-gate-f208`
2026-08-25T16:33:51Z.
