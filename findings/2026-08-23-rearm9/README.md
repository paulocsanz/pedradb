# rearm9 — bateria quieta do branch `rfc-0054-gaps` (CountCache envelope + F219)

**OFICIAL** para RFC-0054 P0.2. Gate load1<10 ×2 consecutivos (15:48:00,
últimas 9,11/8,58); pernas em load 8,45–8,58; peers `sync:false`,
`peer_policy:rocks-default`, `ROCKS_PARITY_SYNC=0`, `ROCKS_YCSB_OPS=2000`
— método rearm7/8. Box em compile-storm de outros projetos antes do gate
(load 20–28); o gate esperou ~25 min até assentar.

- HEAD medido: `rfc-0054-gaps` @ 520538e (fix publish: CountCache envelope
  + watermark no insert; F219 tail_idx_range multi-shard)
- Binário: `/tmp/pedradb-rearm9/rocks-parity-bench` (build `--features real`)
- 3 rodadas × 4 pernas (compat async ycsb,deps / compat kvrocks / rocks
  ycsb,deps / rocks kvrocks), JSONs em `r{1,2,3}/`

## Resultado — RFC-0054 P0.2 `deps_raftlog` **FECHA 3/3 >1×**

| rodada | pedra qps | rocks qps | ratio |
|---|---:|---:|---:|
| r1 | 136.250 | 131.577 | **1,036** |
| r2 | 139.234 | 132.129 | **1,054** |
| r3 | 139.452 | 133.665 | **1,043** |

rearm8 (pré-fix): 0,59× (76,5k vs 130,6k). O gap era o phase `publish`
(`CountCache::record_dirty`: 2 `Box`/key/publish depois que um scan enche
o cache de janelas) — ver `findings/rfc0054-p02/`. Peer estável
(131–134 k, mesma faixa do rearm8).

## Tabela de medianas (3/3 rodadas)

| forma | pedra qps | rocks qps | ratio | rearm8 |
|---|---:|---:|---:|---:|
| kvrocks_scan | 1.698.334 | 50.114 | **33,89** | 36,42 |
| kvrocks_get | 14.285.714 | 2.520.876 | **5,67** | 5,58 |
| ycsb_e | 1.218.707 | 236.970 | **5,14** | 5,44 |
| ycsb_d | 8.441.775 | 1.806.957 | 4,67 | 4,59 |
| kvrocks_pipelined_set | 222.320 | 46.504 | 4,78 | 5,20 |
| ycsb_a | 2.982.479 | 671.338 | 4,44 | 4,62 |
| deps_cache_overwrite | 1.802.005 | 410.681 | 4,39 | 3,35 |
| ycsb_c | 9.322.184 | 2.287.238 | 4,08 | 4,23 |
| ycsb_b | 6.171.259 | 1.760.757 | 3,50 | 3,33 |
| ycsb_f | 1.687.586 | 548.471 | 3,08 | 3,80 |
| kvrocks_set | 1.877.273 | 423.849 | 4,43 | 4,58 |
| deps_lock_prewrite | 39.996 | 20.389 | 1,96 | 2,31 |
| deps_apply_batch | 22.641 | 11.705 | 1,93 | 2,08 |
| deps_scan | 520.760 | 289.853 | 1,80 | 1,68 |
| deps_mvcc_latest | 498.282 | 352.713 | 1,41 | 1,79 |
| kvrocks_set_mc50 | 321.138 | 170.146 | 1,89 | 2,12 |
| kvrocks_blob_set | 112.775 | 90.209 | 1,25 | 1,70 |
| **deps_raftlog** | **139.234** | **132.129** | **1,05 (3/3)** | **0,59** |

Notas: `blob_set` caiu porque o **peer** acelerou (31,6k → 90,2k; máx dele
era 11–14 ms no rearm8) — P1.2 segue aberto contra o peer novo. `mvcc_latest`
1,41 e `apply` 1,93 estão abaixo do piso 2× (P1.1/P1.4 abertos; não são
alvo do P0.2). Nenhuma forma que era ≥2× caiu abaixo de 2× exceto
`deps_lock_prewrite` (1,96, na borda; 2,31 no rearm8) e `apply` (1,93) —
ambas já estavam listadas como pendências P1 no RFC-0054.
