# 2026-08-22 — rearm6: bateria P0.4-método no HEAD `8a36666` (5ª suja; raftlog diagnosticado)

**VERDICT: NON-OFFICIAL** — gate passou a 9,32 às 18:16:59 e o load pulou
para 11,05 às 18:17:01, um segundo depois; watchdog flagou 10,25 na perna
longa. Rounds rodaram sob 10,3–11,1 — contaminados. 5ª bateria suja
consecutiva na caixa (rearm1/3/4/5/6).

## Rounds mediana-de-3 (n=2000, peer = RocksDB default `sync=false`; SUJOS)

### async
| shape | runs | med |
|---|---|---|
| ycsb_a | 19,40/8,20/4,65 | 8,20 |
| ycsb_e | 5,50/5,39/5,17 | 5,39 |
| ycsb_d | 4,99/5,47/4,48 | 4,99 |
| ycsb_f | 4,97/5,41/3,60 | 4,97 |
| deps_cache_overwrite | 4,69/5,45/3,18 | 4,69 |
| ycsb_b | 7,27/4,23/3,60 | 4,23 |
| ycsb_c | 4,05/4,11/3,02 | 4,05 |
| deps_apply_batch | 2,32/2,61/1,99 | 2,32 |
| deps_lock_prewrite | 2,41/2,10/2,14 | 2,14 |
| deps_scan | 2,06/1,71/1,86 | 1,86 |
| deps_mvcc_latest | 1,66/1,61/1,51 | 1,61 |
| deps_raftlog | 1,02/1,07/0,72 | 1,02 |

O 19,40 do A r1 é perna rocks deprimida (rocks 131k vs 376–683k nas outras
— compat estável 2,5–3,2M). F encostou na linha (4,97 med).

### kvr
get 6,47 / pipeline 7,81 / set 7,50 med (r3 mais deprimido: 6,19/5,35/4,47);
scan 32,77; mc50 2,12; blob 2,29 (um run 0,51 — perna rocks doente).

## O achado desta bateria: `deps_raftlog` limpo = **0,72×**

A rodada r3 (mais limpa: compat 93 426 / rocks 129 246) reproduz
exatamente a bateria limpa de manhã (`rfc0046-p04/clean`: 95–101k vs
129–139k = 0,71–0,75×). O ~1,0 dos rearm5/6 rounds sujos era o **rocks
deprimido** até o nível do compat — não melhora nossa.

- Forma da shape: 16 puts sequenciais por batch no CF `raftlog` + 1 get_cf
  a cada 8 ops; coluna async (sem fdatasync em nenhum dos lados).
- Per-write: compat ~1,5M writes/s (0,66 µs) vs rocks ~2,06M (0,5 µs) —
  buraco de ~37% por write, coerente com 0,72×.
- Todas as outras ycsb da coluna async estão 3–19×: raftlog é o outlier
  >10× dentro do nosso próprio engine. Próximo arco de engenharia
  (perfil → causa-raiz → fix → A/B).
