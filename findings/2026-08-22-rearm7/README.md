# rearm7 — bateria quieta oficial-candidata (RFC-0044), HEAD d343044

**Status: EXECUTADA 2026-08-23 07:46, caixa quieta (gate load<10 ×2 consecutivos,
todas as pernas load 8–9, watchdog max 9,0).** Binários: compat e rocks-default
(`sync=false`, par oficial) construídos no worktree d343044
(`/tmp/pedradb-rearm7`). 3 rounds × (compat async ycsb,deps / compat kvrocks /
rocks ycsb,deps / rocks kvrocks). `ROCKS_YCSB_OPS=2000`, `PEDRA_PARITY_ASYNC=1`
(coluna same-class RFC-0044; **não** é o cartaz G1 do RFC-0041).

Gate: armada 03:59 (load 12), re-testada 04:43 (18) e 06:17 (20); polling 60 s
a partir de 06:18 → GATE-PASS 07:46:33 (8,0 → 8,0). Rounds r1–r3 em ~7 s.

## Tabela (mediana de 3 rounds; ratio = compat/rocks-default)

| shape | compat_med | rocks_med | ratio | rounds | ≥5×? |
|---|---:|---:|---:|---|:---:|
| kvrocks_scan | 1.749.016 | 49.677 | **35.21** | 1733/1905/1749 vs 52/50/49 | sim |
| kvrocks_get | 15.156.338 | 2.541.700 | **5.96** | 15156/15395/12143 vs 2542/2542/2499 | 2/3 |
| kvrocks_pipelined_set | 243.245 | 42.870 | **5.67** | 245/243/234 vs 48/40/43 | **3/3** |
| ycsb_e | 1.280.991 | 246.111 | **5.21** | 1260/1415/1281 vs 248/246/238 | **3/3** |
| ycsb_d | 9.097.815 | 1.754.194 | **5.19** | 9098/9117/8562 vs 1754/1682/1812 | 2/3 |
| kvrocks_set | 1.860.321 | 362.229 | **5.14** | 1796/1860/1962 vs 427/324/362 | 2/3 |
| ycsb_a | 3.212.851 | 651.360 | 4.93 | 290/3332/3213 vs 671/649/651 | 2/3* |
| ycsb_c | 9.569.378 | 2.210.453 | 4.33 | 9263/10221/9569 vs 2183/2210/2311 | não |
| deps_cache_overwrite | 1.322.387 | 337.916 | 3.91 | 1331/1322/1260 vs 390/301/338 | não |
| ycsb_b | 6.706.728 | 1.716.678 | 3.91 | 5931/7158/6707 vs 1717/1709/1730 | não |
| ycsb_f | 1.968.020 | 533.126 | 3.69 | 2091/1968/1895 vs 533/553/509 | não |
| deps_apply_batch | 20.324 | 9.674 | 2.10 | 22/20/18 vs 10/10/10 | não |
| deps_lock_prewrite | 30.666 | 15.333 | 2.00 | 31/31/27 vs 14/15/16 | não |
| deps_scan | 576.002 | 317.202 | 1.82 | 503/629/576 vs 289/317/324 | não |
| kvrocks_set_mc50 | 265.931 | 156.409 | 1.70 | 248/266/273 vs 154/159/156 | não |
| deps_mvcc_latest | 558.081 | 336.441 | 1.66 | 566/558/542 vs 337/317/336 | não |
| kvrocks_blob_set | 14.133 | 14.273 | 0.99 | 11/14/16 vs 11/19/14 | não |
| deps_raftlog | 43.824 | 124.649 | **0.35** | 50/34/44 vs 134/102/125 | não |

\* ycsb_a r1 = 290 k com `max_ms` 3,13 — a perna r1 do leg async comeu um stall
de flush/extensão de arquivo dentro de ycsb_a (r2/r3: 3,33 M/3,21 M, max
14–16 µs). Ver descoberta abaixo — mesmo mecanismo do raftlog.

## Fechamentos

- **P1.1 `kvrocks_pipelined_set` FECHA: 5,10/6,04/5,45 — 3/3 ≥5 quieta.**
- **P2.2-E re-confirma fechado: 5,08/5,75/5,38 — 3/3 ≥5.**
- P1.3 `kvrocks_get`: med 5,96 mas r3 4,86 (compat 15,4 M→12,1 M, peer
  estável) — 2/3; **não fecha** pelo padrão 3/3-em-toda-condição.
- P1.2 SET 5,14 med (2/3; r1 4,20); blob 0,99 — não fecha.
- P2.2 A 4,93 med (r2 5,13/r3 4,93; r1 artifact) / B 3,91 / C 4,33 / D 5,19
  (2/3) / F 3,69 — slice segue `doing`.

## Descoberta: deps_raftlog 0,35× = steady-state em paridade + stall de
extensão de arquivo APFS no WAL

`deps_raftlog` (p50) **7,5–7,7 µs/batch = paridade com rocks (6,9–8,5)** — o
pubfix (8e3a460/F204) + `apply_batch_vec` (d343044) fecharam o estado
estacionário. O gap inteiro é cauda: p95 ~20 µs, p99 ~63 µs e **um stall de
20–38 ms por perna** (rocks max 25–76 µs). Aritmética: p50×2000 = 15,2 ms;
+ stall ~26 ms ≈ wall 45,6 ms → qps 43,8 k. O stall explica também todo o
ruído histórico dos A/B raftlog (qps acompanha `max_ms` exatamente).

Atribuição por probe per-batch (`raftlog_tail_probe`, worktree d343044, fase
`wal` = 100% do stall, residual ~0): stalls de 7–33 ms em offsets **fixos e
determinísticos** do WAL — batches 3711/7423/11106/14818/18501 ≈ a cada
**7,1 MiB** de crescimento. Microbench C puro (`/tmp/apfs_append.c`, write de
64 KiB em loop): appends *plain* travam **9–54 ms exatamente a cada 8 MiB**
(writes #129/#257/#385/#513/#641, 2 runs); com `fcntl(F_PREALLOCATE,
F_ALLOCATEALL)` os stalls de fronteira caem para ~1,1 ms e **não há nenhum
>5 ms** — é alocação de extent do APFS bloqueando o `write(2)` que estende o
arquivo, dentro do caminho de commit. RocksDB pre-aloca o WAL
(`PosixWritableFile` prealloc) — é por isso que o peer nunca mostra isso.

Fix: pré-alocação de segmento WAL (chunk 8 MiB re-reservado sob demanda) via
`EnvFile::preallocate` — RFC/commit a seguir. Esperado: raftlog ~0,35× →
≥0,9× (wall ≈ p50×2000 ≈ 15 ms → ~130 k ≈ 1,0×) e ycsb_a r1-artifact some.

## Artefatos

- `loads.txt` (gate + rounds), `watchdog.txt` (max 9,0), `r{1,2,3}/{async,kvr,
  rocks,rocks-kvr}/rocks_parity_bench.json` (+ logs `.log`).
- Todos os peers `sync:false` / `peer_policy:rocks-default` (verificado).
