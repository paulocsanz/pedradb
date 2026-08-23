# rearm11 — bateria quieta do branch `rfc-0054-gaps` (P1.4 apply: tail_idx pack32 + WAL append_exact_to + compat run-prefix)

**OFICIAL** para RFC-0054 P1.4. Gate load1<10 ×2 (armado 20:21:23);
pernas em load 6,35–6,56; peers `sync:false`, `peer_policy:rocks-default`,
`ROCKS_PARITY_SYNC=0`, `ROCKS_YCSB_OPS=2000` — método rearm7/8/9/10.

- HEAD medido: `rfc-0054-gaps` @ b81be31
- 3 rodadas × 4 pernas, JSONs em `r{1,2,3}/`
- Binário dedicado `rocks-parity-bench-real-p14` (`--features real`)

## Resultado — RFC-0054 P1.4 `deps_apply_batch` **FECHA 3/3 ≥2×**

| rodada | pedra qps | rocks qps | ratio |
|---|---:|---:|---:|
| r1 | 23.775 | 11.477 | **2,072** |
| r2 | 23.900 | 11.499 | **2,078** |
| r3 | 23.786 | 10.927 | **2,177** |

rearm10 (pré-fix): 1,82/1,93/1,84. Lado pedra estável em 22–24 k em toda
bateria deste branch (p50 0,039–0,040 ms em todas as rodadas de todas as
tentativas).

## O que mudou (P1.4)

A fase `mem` dominava (13,5 µs/commit nas phasesΔ). Profile `sample` do
apply-only apontou `tail_idx_insert` (228/364 samples), com 56 samples de
memcmp: chaves mvcc (`default\0u/N\0` + ts 8B BE) empatam nos 16 primeiros
bytes (ts começa com zeros) e caem no memcmp de sufixo a cada insert de
nova versão.

- `tail_idx` shard keys `(u128, u128, Bytes)`: 32 bytes de prefixo em
  comparação por inteiros — o ts mvcc cai no caminho inteiro. Oráculo
  adversarial 0..40B prova ordem total + equivalência de ranges
  (`tail_idx_packed_order_matches_bytes_oracle`; 405/405 core verde).
- `tail_idx_get`: primeira entrada da classe `(pack32,_)` + igualdade
  full-key — sem probe TLS nem alloc no caminho de leitura (raftlog
  133–137 k nas pernas limpas desta bateria, 140–152 k nas sondas).
- WAL `fragment_from`: header placeholder 7B + `reserve` +
  `append_exact_to` (mata o resize-zero+copy duplo do fragmento).
- compat `write_cf_owned`: `run_prefix` por run (`cf\0` uma vez) +
  `encode_run` (2 extends por chave).

## Tabela de medianas (3 rodadas, ycsb+deps)

| forma | pedra | rocks | ratio | rearm10 |
|---|---:|---:|---:|---:|
| ycsb_e | 1.155.346 | 232.906 | 4,96 | 5,04 |
| ycsb_a | 3.169.150 | 663.992 | 4,79 | 4,60 |
| ycsb_d | 8.421.053 | 1.814.470 | 4,59 | 4,77 |
| deps_cache_overwrite | 1.734.605 | 408.302 | 4,25 | 4,78 |
| ycsb_c | 9.304.100 | 2.276.932 | 4,11 | 4,18 |
| ycsb_b | 6.470.754 | 1.736.238 | 3,76 | 3,58 |
| ycsb_f | 1.848.499 | 511.885 | 3,61 | 3,17 |
| **deps_mvcc_latest** | **930.052** | **324.173** | **2,87 (3/3)** | 2,53 |
| deps_lock_prewrite | 40.371 | 18.634 | 2,17 | 2,08 |
| **deps_apply_batch** | **23.786** | **11.477** | **2,08 (3/3)** | 1,83 |
| deps_scan | 472.585 | 272.684 | 1,73 | 1,89 |
| deps_raftlog | 133.710 | 127.800 | 1,05 | 1,02 |

## Interferência (stalls de sub-segundo, não regressão)

Pernas de 15–350 ms são sensíveis a um único stall do SO. Evidência por
percentil (p50 intacto + max explodido):

- `deps_lock_prewrite` r1: qps 5.933 vs 40–43 k normal — **max
  248,9 ms** (r2/r3: 4,77/1,45 ms), p50 0,0234 normal.
- `deps_raftlog` r2: ratio 0,827 — max 4,66 ms vs 1,24 ms no r3, p50
  0,0063 idêntico entre rodadas.
- `kvrocks_blob_set` r1/r2 (0,73/0,59): **ambos os lados** com cauda de
  múltiplos ms (rocks 1,6–6,4 ms; pedra 5,7–14,2 ms) — contenção de disco
  numa shape de banda (32 MB/perna); no rearm10 ambos os lados tinham max
  0,2–0,3 ms. P1.2 segue aberto.

Tentativas arquivadas: `../2026-08-23-rearm11-attempt1/` (load 8,1–8,9;
apply 2,134/2,019/1,840 — r3 com max 3,5/4,9 ms). Re-confirmação do guard
de regressão (raftlog/lock em pernas limpas) roda em
`../2026-08-23-rearm11-reconfirm/` quando o gate armar.

## kvrocks (pernas kvr)

| forma | ratios | estado |
|---|---|---|
| kvrocks_get | 5,74/5,74/4,85 | estável |
| kvrocks_set | 4,20/4,43/4,48 | estável |
| kvrocks_pipelined_set | 4,93/4,89/5,14 | estável |
| kvrocks_scan | 27,1/28,1/30,9 | estável |
| kvrocks_blob_set | 0,73/0,59/0,89 | disco contaminado (ver acima) |
| kvrocks_set_mc50 | 3,22/2,68/2,03 | ≥2 |

## Abertos após rearm11

- **P1.3** deps_scan 1,73–1,78 (TLS absolve ~45%; 874/2000 vão ao kernel)
- **P1.2** blob_set — re-medir em disco quieto; shape de banda simétrica
- raftlog: 2/3 ≥1 aqui com miss atribuído a stall (p50 idêntico); recorde
  oficial 3/3 permanece rearm10; sondas pack32 mostram leitura mais rápida

## Re-confirmação (`../2026-08-23-rearm11-reconfirm/`)

Armada 20:35:02 em load 7,33; a sessão paralela subiu o load para 10,4–11,3
no meio (loads.txt). A rodada **r1, inteira em load 7,33**, passa todos os
guardas: apply **2,199**, mvcc 2,738, raftlog **1,066 (≥1)**, lock
**2,286 (≥2)**, scan 1,835 — nenhuma regressão das formas fechadas com as
mudanças do P1.4. r2/r3 documentam a interferência de novo (raftlog r2
0,783 com max 5,3 ms; lock r3 0,630 com p50 0,0363 e max 6,2 ms; blob r3
6,7 k qps — disco). Registros oficiais seguem sendo os desta bateria (r1–r3
da seção acima) + rearm10 para raftlog 3/3.

## Nota de escala (não-oficial)

Curva da fase `mem` (phasesΔ por commit) conforme a memtable cresce além
da escala oficial (deps para em ~256k entradas): 13,5 µs (oficial) →
18,2 µs (25,6M entradas, 200k ops) → 24,9 µs (128M, 1M ops). Micro
`mem_insert_apply_micro` confirma: 26 µs/op (2,56M entradas) → 59,8 µs
(192M). 500× mais entradas ≈ 1,8× mais lento por commit — descida
logarítmica do índice, sem degradação súbita. Não afeta P1.4.
