# RFC-0062 P0.3 — Linux coluna A `pwrite`, 4 vCPU caixote AMD

**Veredito:** `RESULT=P03_FAIL` `min_ratio=0.745` `fail=ycsb_f,deps_raftlog`
**Quando:** 2026-08-25 `P03_START` 05:48:03Z → `P03_DONE` 05:54:42Z
**VM:** `linux-gate-f208` (4 vCPU / 4 GB / 8 GB disk, brasil, Threadripper PRO 3975WX)
**Imagem:** `ghcr.io/paulocsanz/pedradb-linux-gate:p03a` = bench19diag (`pwrite`, fonte `17d10c4`) + `scripts/rfc0062_p03_entrypoint.sh`
**Coluna:** A — `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`. `ROCKS_PARITY_BIG=0`. ops=2000, zipfian. `rm` do DB depois de cada processo.

Gate pedido: `min(3 rounds) > 1.0` em toda shape oficial, raftlog e blob inclusos.

## Scoreboard oficial (3 rounds, suite completa, sem `ycsb_c_big`)

| shape | min | mediana | rounds | vs 1.0 |
|---|---:|---:|---|---|
| ycsb_a | 2.156 | 2.666 | 2.156 / 2.797 / 2.666 | PASS |
| ycsb_b | 1.434 | 2.603 | 1.434 / 3.289 / 2.603 | PASS |
| ycsb_c | 2.162 | 3.374 | 2.162 / 4.308 / 3.374 | PASS |
| ycsb_d | 1.680 | 3.051 | 1.680 / 3.244 / 3.051 | PASS |
| ycsb_e | 9.053 | 13.967 | 9.053 / 14.198 / 13.967 | PASS |
| **ycsb_f** | **0.859** | 1.539 | 1.539 / **0.859** / 2.460 | **FAIL** |
| deps_cache_overwrite | 2.088 | 3.130 | 2.088 / 3.818 / 3.130 | PASS |
| deps_lock_prewrite | 1.836 | 2.248 | 1.836 / 2.248 / 2.254 | PASS |
| deps_mvcc_latest | 2.237 | 2.788 | 3.545 / 2.788 / 2.237 | PASS |
| deps_apply_batch | 1.590 | 1.936 | 2.165 / 1.936 / 1.590 | PASS |
| **deps_raftlog** | **0.745** | **0.785** | 1.576 / **0.745** / 0.785 | **FAIL** |
| deps_scan | 2.250 | 2.461 | 3.037 / 2.461 / 2.250 | PASS |
| kvrocks_get | 4.585 | 4.720 | 4.720 / 5.781 / 4.585 | PASS |
| kvrocks_set | 2.525 | 2.582 | 2.525 / 2.582 / 2.803 | PASS |
| kvrocks_scan | 41.022 | 41.854 | 41.022 / 41.854 / 43.940 | PASS |
| kvrocks_pipelined_set | 3.124 | 3.409 | 3.124 / 3.409 / 3.748 | PASS |
| **kvrocks_blob_set** | **2.166** | 2.579 | 2.166 / 2.619 / 2.579 | **PASS** |

Blob sai do cartaz de defeito nesta caixa: 3/3 >2× na suite e 3/3 isolado.

## Isolado (DB vazio, `ROCKS_PARITY_ONLY`)

`deps_raftlog`:

| r | ratio | p50 µs P/R | p99 µs P/R |
|---|---:|---|---|
| 1 | 0.958 | 10.0 / 10.3 | 34.3 / 19.0 |
| 2 | 0.976 | 9.6 / 10.1 | 31.6 / 18.0 |
| 3 | 1.111 | 9.9 / 12.3 | 35.3 / 30.4 |

**min 0.958, mediana ~0.976.** p50 empatado. p99 nosso ~32–35 vs Rocks quieto ~18–19. Mesmo desenho do diag-6 (mediana 0.979, 2/5 >1×). `rm` + `BIG=0` não fecha o min.

`kvrocks_blob_set` isolado: min **2.277** (r2 2.593, r3 2.277; r1 no recorte do serial).

## Leitura

- P0.3 **medido**. Gate **não** verde. Hipótese “a mesma bateria com pwrite passa o min” **falhou** no raftlog isolado (0.958) e no raftlog da suite (0.745).
- 2× recusado: p50 continua empatado (~10 µs).
- `ycsb_f` min 0.859 é round sujo (r2); mediana 1.539. Não era o defeito nomeado; o gate min>1.0 pegou. Tratar como ruído de round até repetir, ou como segundo FAIL oficial.
- Próximo slice: RFC-0062 **P0.4** — um corte no p99 (encode WAL **ou** 16 inserts), A/B na mesma VM. Sem skiplist, sem journal-CF.

Fonte serial: `caixote logs linux-gate-f208` 2026-08-25T05:54:42Z.
