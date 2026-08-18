# RFC-0042 P0 — baseline instrumentada + split do custo lone-writer

2026-08-18 05:44. Baseline pré-P1.1 (janela catch-up fixa de 50 µs, sem
bound `fd/2`) sob a forma oficial: `ROCKS_PARITY_SYNC=0`, `CLIENTS=4`,
records=4096 / ops=2000 / payload=1000 / zipfian, suite `ycsb,deps`,
intercalado por run (compat → rocks → compare), mediana de 3 runs.
Binários congelados em `implementer/bin-head` (HEAD + instrumentação
P0.2, **sem** o bound do P1.1). Peer RocksDB default async-WAL
(`sync:false` em `run*/compare/rocks_shaped_peer.template.json`).

## Ambiente — LEIA ANTES DE COMPARAR NÚMEROS

A caixa **não estava silenciosa**: 12 CPUs lógicas (8P+4E) com load
average ~42 durante toda a medição (campanha de fuzzing do usuário —
`fuzz_mapdet`/`fuzz_ftdiff`/`fuzz_merge`/`fuzz_opts`, ~600% CPU, sem
`-max_total_time` — mais Spotlight/FSEvents indexando os diretórios de
dados dos benches). Ambos os lados caíram vs. o head3 do RFC-0041
(2026-08-17, caixa silenciosa, mesmo protocolo): Rocks `ycsb_b`
785 k → ~410 k; compat caiu 2–4× com scatter alto. **Razões deste
baseline não são comparáveis a head3**; servem como referência
"antes" apenas para medições feitas no mesmo dia, sob a mesma carga.

## Medianas (3 runs, `compat_over_rocksdb`)

| shape | r1 | r2 | r3 | mediana | head3 |
|---|---:|---:|---:|---:|---:|
| deps_apply_batch | 0.965 | 1.627 | 1.221 | 1.221 | 1.297 |
| deps_apply_batch_mc4 | 2.488 | 1.541 | 2.080 | 2.080 | 2.788 |
| deps_cache_overwrite | 0.095 | 0.136 | 0.589 | 0.136 | 0.298 |
| deps_cache_overwrite_mc4 | 0.218 | 0.111 | 0.143 | 0.143 | 0.569 |
| deps_mvcc_latest | 2.189 | 2.522 | 3.307 | 2.522 | 2.342 |
| deps_raftlog | 0.589 | 0.700 | 0.353 | 0.589 | 0.994 |
| deps_raftlog_mc4 | 0.388 | 0.775 | 0.702 | 0.702 | 1.792 |
| deps_scan | 1.985 | 1.560 | 1.055 | 1.560 | 1.790 |
| ycsb_a | 0.126 | 0.157 | 0.156 | 0.156 | 0.233 |
| ycsb_a_mc4 | 0.118 | 0.369 | 0.108 | 0.118 | 0.522 |
| ycsb_b | 0.117 | 0.190 | 0.757 | 0.190 | 0.496 |
| ycsb_c | 3.808 | 1.503 | 1.471 | 1.503 | 1.796 |
| ycsb_d | 0.360 | 0.476 | 0.410 | 0.410 | 0.487 |
| ycsb_e | 4.651 | 0.202 | 0.767 | 0.767 | 2.121 |
| ycsb_f | 0.137 | 0.312 | 0.186 | 0.186 | 0.265 |
| ycsb_f_mc4 | 0.308 | 0.136 | 0.427 | 0.308 | 0.479 |

## Evidência do agrupamento (motivação do P1.1)

`write_group` por run (linha de diagnóstico do compat):
`avg_group=1.54 / 1.42 / 1.50` com 4 clientes. A janela fixa de 50 µs
não coalesce: ~46% dos submits passam pela fila e ainda assim o grupo
médio carrega só ~1.5 escritores — cada grupo paga um `fdatasync`
inteiro. Se o bound `min(window, fd_ema/2)` + sweep elevar o grupo
médio para ~3, o custo por op cai ~2× nas `_mc4`.

## Split do lone-writer (1c) — `lone-split-baseline.txt`

Example `lone_split` (passo append 1c e overwrite quente, µs/op):
append: start 0.50 / apply 0.39 / **io 58.59** / publish 0.12 (total
59.6); overwrite: start 1.11 / apply 0.64 / **io 88.32** / publish
0.37 (total 90.4). Non-fd é 1.0–2.1 µs/op (~2%): **o 1c é 98%
`fdatasync`** — confirma o teto físico registrado no RFC-0042.
`wal_fd_ema` 29.8 µs; microbench de arquivo cru p50 32.2 µs / mean
62.1 / p99 206.7 µs (caixa ruidosa; p99 já chegou a 2.6 ms).

## Conclusão do P0

O ganho possível das `_mc4` vem de agrupamento (P1.1 + sweep), não de
cortes de CPU (non-fd é ~2%). O 1c segue no teto `1/t_fd`. Sob a carga
de hoje, ratios absolutos ficam abaixo de head3; o efeito do P1.1 é
medido contra este baseline do mesmo dia.
