# rearm8 — bateria quieta do branch `wal-prealloc` (F_PREALLOCATE no WAL)

**OFICIAL** (gate load<10 ×2 consecutivos 09:50/09:51; watchdog 17 polls, pico
load 8, nenhum cruzamento ≥10 durante as pernas; todas as pernas load 7–8;
peers `sync:false`, `peer_policy:rocks-default`, `ROCKS_PARITY_SYNC=0`,
`ROCKS_YCSB_OPS=2000` — método idêntico ao rearm7).

- HEAD medido: `wal-prealloc` @ 6c49252 (base d343044 + pré-alocação WAL)
- Binários: compat reconstruído no branch; peer rocks **inalterado** desde rearm7
  (`/tmp/pedradb-rearm7/.../rocks-parity-bench-real`)
- 3 rodadas × 4 pernas (compat async ycsb,deps / compat kvr / rocks ycsb,deps /
  rocks kvr), JSONs em `r{1,2,3}/`.

## Tabela de medianas (3/3 rodadas, todas as formas)

| forma | pedra qps | rocks qps | razão | máx c/r (ms, por rodada) |
|---|---:|---:|---:|---|
| kvrocks_scan | 1.761.533 | 48.374 | **36,42** | 0,058/0,714 · 0,060/0,070 · 0,045/0,059 |
| **kvrocks_get** | 13.517.350 | 2.422.774 | **5,58** | 0,0012/0,0012 · 0,0005/0,0123 · 0,0009/0,0015 |
| kvrocks_pipelined_set | 233.447 | 44.886 | **5,20** | 0,562/0,108 · 0,447/0,302 · 0,423/0,075 |
| ycsb_e | 1.284.212 | 236.215 | **5,44** | 0,069/0,043 · 0,046/0,023 · 0,044/0,021 |
| ycsb_d | 8.210.754 | 1.787.310 | 4,59 | 0,014/0,017 · 0,010/0,007 · 0,009/0,007 |
| ycsb_a | 3.052.074 | 660.157 | 4,62 | 0,078/0,015 · 0,010/0,008 · 0,020/0,028 |
| kvrocks_set | 1.915.250 | 418.035 | 4,58 | 0,012/0,025 · 0,012/0,031 · 0,023/0,019 |
| ycsb_c | 9.268.184 | 2.192.682 | 4,23 | — |
| ycsb_f | 1.903.855 | 501.054 | 3,80 | — |
| ycsb_b | 6.024.096 | 1.806.956 | 3,33 | — |
| deps_cache_overwrite | 1.308.579 | 390.975 | 3,35 | — |
| deps_lock_prewrite | 38.602 | 16.709 | 2,31 | 0,116/12,29 · 0,127/12,67 · 0,124/14,44 |
| kvrocks_set_mc50 | 330.271 | 156.107 | 2,12 | 8,77/12,30 · 7,78/14,13 · 8,43/30,93 |
| deps_apply_batch | 21.940 | 10.549 | 2,08 | — |
| deps_mvcc_latest | 556.993 | 311.355 | 1,79 | — |
| kvrocks_blob_set | 53.617 | 31.628 | 1,70 | 0,014/14,09 · 0,014/11,78 · 0,012/12,47 |
| deps_scan | 481.242 | 287.111 | 1,68 | — |
| deps_raftlog | 76.551 | 130.606 | 0,59 | 0,067/0,050 · 0,098/0,047 · 0,063/0,068 |

## Fincas (veredictos por slice)

### RFC-0044 P1.3 `kvrocks_get` **FECHA 3/3**
5,465 / 6,074 / 5,710 (med 5,58). rearm7 tinha med 5,96 mas 2/3 (r3 4,86);
a rodada que faltava chegou com folga. Peer estável (2,37–2,44 M em todas
as rodadas). Compat 13,3–14,7 M.

### RFC-0044 P1.2 `kvrocks_set` — não fecha
4,400 / 4,681 / 4,477 (3/3 <5, med 4,58). Antes: 5,14 med mas 2/3. Segue
consistentemente na faixa 4,4–4,7. `blob_set` saiu de 0,99 para
1,64/1,79/1,70 — longe do alvo ainda (p50 compat vs rocks 10,2 µs; máx
do peer 11–14 ms é comportamento do próprio rocks).

### RFC-0044 P2.2 — `ycsb_e` re-confirma (2ª bateria quieta 3/3)
5,175/5,379/5,493 (med 5,44). `ycsb_d` 4,69/4,56/4,84 (3/3 <5). `ycsb_a`
3,47/4,48/4,64 — o artifact r1 do rearm7 (max 3,1 ms = stall de extent do
WAL) **sumiu** (máx r1 agora 0,078 ms), mas r1 segue ~30% abaixo de r2/r3
(warmup, não stall). B 3,33 / C 4,23 / F 3,80.

### RFC-0041 P1.3 `deps_raftlog` — 0,35× → 0,59×, cauda resolvida, gap agora é caminho
- rearm7 (sem fix): 50/34/44 k vs rocks 134/102/125 k; máx compat 19,7–38,5 ms
  vs rocks 0,025–0,076 — um stall de extent APFS por perna comia ~2/3 do tput.
- rearm8 (com F_PREALLOCATE): 79,6/66,0/76,6 k vs 131,2/130,6/124,8 k; **máx
  compat 0,063–0,098 ms ≈ máx rocks 0,047–0,068 ms** — mesma classe de cauda,
  stall eliminado. Confirma o fix em bateria quieta.
- Gap restante é throughput de caminho: 13–15 µs/batch vs 7,6–8,0 µs/batch
  (wall_s/n). O probe no branch (20k×16, fora da bateria) mede o write core em
  **4,0–4,1 µs p50** → ~9 µs/batch vivem na construção do lote + submit path
  do bench (format!/clone por chave, conversão compat). p50_ms do compat no
  JSON não discrimina (amostras 0 — o bench registra latência sub-µs demais);
  wall/qps é a métrica honesta aqui.
- Próxima alavanca: cortar overhead de submit/construção de lote (não é mais
  cauda, não é fold — `fold_parked_once_off_lock` só roda com memtable staged
  ≥256 MiB, inatingível nesta forma).

## Deltas vs rearm7 (mesma metodologia, mesma caixa)
raftlog 0,35→0,59 (fix pré-alocação); blob 0,99→1,70; kvr_get 5,96(2/3)→5,58(3/3);
cache_overwrite 3,91→3,35; scan_kvr 35,2→36,4; demais dentro de ±10%.
