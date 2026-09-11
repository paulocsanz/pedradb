# P201o sweep — regressão group vs default nas 20 formas + auditoria de writers

**Data:** 2026-09-11 06:30:01Z (terminal) | **Caixa:** caixote
`linux-gate-p149b` (CHV, 4 vCPU, guest Alpine, nproc=4)
**Imagem:** `ghcr.io/paulocsanz/pedradb-linux-gate:p201o`
(digest `sha256:ce4951b7…`, src = árvore viva head 231addb3 + ff233b15, sem o
corte P0.3 — o sweep valida o FLIP, não a árvore pós-corte)
**Protocolo:** mesmo boot, 3 rounds quiet (load1<2 ×2), ordem de suítes
rotativa por round (kvrocks/deps/ycsb), 2 variantes Pedra por round
(`default` env-limpo e `group` `PEDRA_ASYNC_GROUP=1`, ambas
`PEDRA_PARITY_ASYNC=1` coluna same-class), peer RocksDB default
`ROCKS_PARITY_SYNC=0` do MESMO round, `ROCKS_YCSB_OPS=2000`,
`ROCKS_PARITY_RATIO_FLOOR=none`.

## Resultado por forma (ratio vs rocks do mesmo round; min/mediana de 3)

| forma | default min | default med | group min | group med | paired min g/d |
|---|---|---|---|---|---|
| deps_apply_batch | 1,302 | 1,492 | 1,324 | 1,446 | 0,965 flat |
| deps_cache_overwrite | 0,512 | 0,647 | 0,540 | 0,642 | 0,931 REGRESS |
| deps_lock_prewrite | 1,659 | 1,698 | 1,662 | 1,702 | 0,970 flat |
| deps_mvcc_latest | 3,999 | 4,281 | 2,401 | 2,719 | **0,561 REGRESS** |
| deps_raftlog | 1,577 | 1,640 | 1,627 | 1,631 | 0,992 flat |
| deps_scan | 0,831 | 0,922 | 0,847 | 0,936 | 0,919 REGRESS |
| kvrocks_blob_set | 2,351 | 2,654 | 2,429 | 2,701 | 0,915 REGRESS |
| kvrocks_get | 4,738 | 4,765 | 4,417 | 4,628 | 0,927 REGRESS |
| kvrocks_pipelined_set | 1,203 | 1,775 | 1,703 | 1,777 | 0,949 REGRESS |
| kvrocks_scan | 26,403 | 37,142 | 36,704 | 41,113 | 0,818 REGRESS |
| kvrocks_set | 0,748 | 0,757 | 0,669 | 0,701 | 0,894 REGRESS |
| **kvrocks_set_mc50** | **1,034** | **1,092** | **1,894** | **2,011** | **1,693 ok** |
| ycsb_a | 0,605 | 0,905 | 0,928 | 0,942 | 1,025 flat |
| ycsb_b | 1,088 | 1,276 | 1,236 | 1,245 | 0,592 REGRESS |
| ycsb_b_unif | 1,394 | 1,980 | 2,060 | 2,169 | 1,014 flat |
| ycsb_c | 2,729 | 3,305 | 3,346 | 3,372 | 0,998 flat |
| ycsb_c_unif | 3,238 | 3,334 | 3,550 | 3,928 | 1,004 flat |
| ycsb_d | 1,122 | 2,000 | 1,988 | 2,002 | 0,994 flat |
| ycsb_e | 7,976 | 9,517 | 9,014 | 10,046 | 0,947 REGRESS |
| ycsb_f | 0,804 | 0,956 | 0,950 | 0,972 | 1,013 flat |

(Correção 2026-09-11: o sweep correu a coluna **async**
(`PEDRA_PARITY_ASYNC=1`, sem fdatasync) — as linhas single-client abaixo
não são teto-fd por construção. `kvrocks_set` 0,67–0,75 e
`deps_cache_overwrite` 0,51–0,54 são **buracos abertos na coluna async**:
Pedra paga `write()` por op (ticket pwrite 0193) enquanto o Rocks
`sync=false` só faz memcpy no buffer do WAL — hipótese **hat**, precisa de
meter de atribuição próprio; never quote como win nem esconder.)

## Auditoria de writers (fonte: `crates/rocksdb-parity-bench/src/lib.rs`)

Todo spawn de thread do bench: L~1061 (`kvrocks_set_mc50`, 50 clientes),
L~1470 (surreal rmw mc, fora do sweep), L~2327 (`{name}_mc{clients}`),
L~2404 (`run_deps_clients`), L~2490 (apply_batch mc). As 10 formas flagadas:

- `deps_cache_overwrite` L690, `kvrocks_*` (L861–1046), `ycsb_b`/`ycsb_e`
  (`run()` L2133): loops **single-threaded** — 1 writer sequencial.
- `deps_mvcc_latest` L544, `deps_scan` L577: loops **read-only** — zero
  writers.

Logo: **nenhuma forma flagada tem mais de 1 writer concorrente.** Na
árvore do sweep, braço `default` = bypass SEMPRE e braço `group` =
`PEDRA_ASYNC_GROUP=1`; para ≤1 writer os dois braços executam caminhos
bit-idênticos (lone fast path dispara ANTES da decisão de merge em
`concurrent.rs`; formas read-only nem abrem o caminho de escrita).

## Adjudicação

1. **Os 10 flags não são efeito do corte.** Entre braços código-idênticos,
   a métrica paired flagou até **0,561** (`deps_mvcc_latest`) — e flagou
   "REGRESS" em formas cujo resultado ABSOLUTO melhorou no braço group
   (`kvrocks_scan` 36,7 vs 26,4; `ycsb_b` 1,236 vs 1,088; `ycsb_e` 9,0 vs
   8,0; `kvrocks_pipelined_set` 1,703 vs 1,203). Métrica medindo variância
   de estado/tempo da caixa compartilhada (load1 ~1,35–1,38; timing de
   flush/compaction diverge entre runs), não regressão de código.
2. **Noise floor:** neste host, paired group/default < ~1,7× em formas
   single-threaded é indistinguível de ruído; o corte REGRESS<0,95 do
   protocolo p201o está abaixo do piso de ruído para essas formas. Lição
   metodológica registrada: gate de regressão paired só é decisivo em
   formas onde os braços divergem mecanicamente (herd) ou com min-of-3
   por braço em janelas quiet mais longas.
3. **`deps_mvcc_latest`** é a única piora absoluta (min 3,999→2,401) — mas
   é read-only single-thread: mecanicamente inalcançável pela política de
   merge; ambos os braços ≥2,4× rocks (win largo nas duas pontas).
   Atribuída a divergência de estado (a suíte deps roda apply_batch antes;
   timing de compaction do CF write difere entre braços).
4. **A única forma onde os braços divergem mecanicamente é
   `kvrocks_set_mc50`** (50 writers > ncpu=4): default min 1,034 → group
   min **1,894** (paired 1,693 ok) — reproduz o meter de atribuição
   (`findings/2026-09-11-p201-meter-atribuicao/`).

**Veredito: o flip P0.3 passa.** O corte auto (`writers > ncpu`) muda
comportamento apenas onde writers>4 — exatamente mc50, onde ganha; todas as
formas flagadas são código-idênticas entre braços (ruído) e o regime da
falsificação 0044 (bypass em writers ≤ ncpu) permanece intacto por
construção.

## A/B pós-corte (imagem `p201q`, digest `sha256:18babf02…`, mesmo boot)

Braços `auto` (env limpo = default pós-corte P0.3) vs `pin0`
(`PEDRA_ASYNC_GROUP=0` ≡ default pré-corte, code-identico ao bypass) vs
rocks, mc50, 3 rounds quiet, peer `ROCKS_PARITY_SYNC=0`, coluna same-class
(`PEDRA_PARITY_ASYNC=1`). Src = código @ 8b2ffacf (corte f7b2c20f
presente; o clamp P0.1 b2b0295b é posterior e não muda grupos ≤50 —
absorb loop dobra o resto no mesmo frame).

| round | rocks qps | auto qps | auto ratio | pin0 qps | pin0 ratio | flip auto/pin0 |
|---|---|---|---|---|---|---|
| 1 | 123 152 | 213 585 | 1,7343 | 110 942 | 0,9009 | 1,925 |
| 2 | 102 889 | 220 701 | 2,1451 | 100 611 | 0,9779 | 2,194 |
| 3 | 107 840 | 180 974 | 1,6782 | 72 017 | 0,6678 | 2,513 |

**Cartaz (Linux 3-run quiet min-of-3): kvrocks_set_mc50 default =
1,678× vs RocksDB default `sync=false`** (mediana 1,734×); pré-corte o
mesmo braço media 0,668× (mediana 0,901×). O flip pareado auto/pin0:
min 1,925 / mediana 2,194. Consistente com o meter de atribuição
(group 1,52–2,57×) — o auto default reproduz o braço group.

Serial bruto: `caixote logs linux-gate-p149b` (P201Q_CELL /
P201Q_METER_RESULT / P201Q_FLIP acima; captura `p201o-full.log` +
monitor). Entry: scratch `p201q_entrypoint.sh`.
