# P209 — meter do buffer de WAL em user-space (RFC-0209 P0.3): buf vs nobuf, same-boot

**Data:** 2026-09-11 (p209a 17:59–18:07Z; p209b 19:0xZ) | **Caixa:** caixote
`linux-gate-p149b` (CHV, 4 vCPU, guest Alpine, nproc=4)
**Imagens:** `p209a` digest `sha256:51b96399…`, `p209b` digest
`sha256:449b4b68…` (base `p04a`, src = árvore `c064d621` = commit do P0.1+P0.2)
**Protocolo:** mesmo boot, 3 rounds quiet (load1<2 ×2), ordem de suíte
rotativa por round, `ROCKS_PARITY_CLIENTS=4` nos DOIS engines,
`PEDRA_PARITY_ASYNC=1` (coluna same-class), peer RocksDB default
`ROCKS_PARITY_SYNC=0` do MESMO round, `ROCKS_YCSB_OPS=2000`, braços
`PEDRA_WAL_BUFFER=1` (cap default 64 KiB) vs env-limpo. p209b:
`ROCKS_YCSB_DIST` **unset = uniform** (idem âncoras p201o/p201r2).

## Lições de protocolo (onda p209a, DIAG)

1. **Célula 10k vazia (0/3):** `deps_cache_overwrite_mc4` é emitido pelo
   harness multi-cliente da suíte **ycsb** (`run_clients` — shapes
   ycsb_a/ycsb_f/deps_cache_overwrite → `{name}_mc{clients}`); o grupo 10k
   da p209a rodou com `SUITE=deps` → harness nunca executou (rc=0, JSON sem
   o shape). Corrigido na p209b (`ycsb,deps`).
2. **dist=zipfian herdada do script base p04:** as âncoras não setam
   `ROCKS_YCSB_DIST` (uniform). p209a é DIAG interno; p209b roda uniform.

## Re-ancoragem (aviso de honestidade)

O braço **nobuf** (= default enviado, código idêntico ao medido em p201r2
para o caminho async) NÃO reproduz os absolutos das âncoras p201o/p201r2
neste boot: overwrite single 2,78× (âncora 0,512), ycsb_a single 2,60×
(âncora 0,605), ycsb_f_mc4 0,532 (âncora 0,295), overwrite_mc4 1,035
(âncora 0,370). Causa: boot/host diferente do serviço (cross-boot absoluto
não é comparável — o protocolo só compara same-boot). **Esta onda
re-ancora tudo o que mede**; os cartazes pagos (apply_mc4 1,0859; mc50
1,678) permanecem pagos nas suas ondas — aqui apareceram como guardiãs em
níveis ≥ (1,466 / 3,284), sem contradição.

## Resultado p209b (uniform) — ratio vs rocks do MESMO round

| forma | nobuf r1/r2/r3 (min) | buf r1/r2/r3 (min) | buf/nobuf med |
|---|---|---|---|
| ycsb_a | 2,602/3,892/2,765 (**2,602**) | 2,671/3,833/2,538 (**2,538**) | 0,985 |
| ycsb_a_mc4 | 1,914/1,857/1,537 (**1,537**) | 1,115/1,998/1,383 (**1,115**) | 0,900 |
| ycsb_f | 2,005/2,455/1,881 (**1,881**) | 1,949/2,418/1,094 (**1,094**) | 0,972 |
| ycsb_f_mc4 | 1,882/0,791/0,532 (**0,532**) | 0,963/1,055/0,780 (**0,780**) | **1,334** |
| deps_cache_overwrite | 2,826/3,036/2,783 (**2,783**) | 2,860/3,047/2,952 (**2,860**) | 1,012 |
| deps_cache_overwrite_mc4 | 1,877/1,594/1,035 (**1,035**) | 1,026/1,602/1,106 (**1,026**) | 1,005 |
| deps_apply_batch_mc4 (guarda) | 1,466/1,869/1,691 (**1,466**) | 1,473/1,485/1,724 (**1,473**) | 1,005 |
| kvrocks_set | 2,158/2,430/2,588 (**2,158**) | 2,613/2,799/2,448 (**2,448**) | **1,152** |
| kvrocks_set_mc50 (guarda) | 3,524/3,755/3,284 (**3,284**) | 3,214/3,828/4,479 (**3,214**) | 1,020 |
| **deps_cache_overwrite_mc4@10k** | 1,113/1,573/1,078 (**1,078**) | 1,268/1,846/1,144 (**1,268**) | **1,173** |

Efeito de variância observado: o braço buf **aperta a dispersão** nas
células mc4 (ycsb_f_mc4 pedra 388/408/483k vs nobuf 758/306/329k;
overwrite_mc4 335/389/366k vs 614/386/343k) e **levanta o piso** da pior
célula (ycsb_f_mc4 min 0,532 → 0,780).

## Adjudicação

1. **Célula 10k do gate 0185 P0.3: PAGA 3/3 ≥ 1,0 nos dois braços** —
   nobuf (default enviado) 1,113/1,573/1,078 (min **1,078**), buf
   1,268/1,846/1,144 (min **1,268**); peer saudável (rocks 200k–307k,
   acima da linha de colapso 157k). Medição same-boot 3 rounds quiet no
   gate; cross-boot vs trim7/8 não é comparável (nota de re-ancoragem).
2. **P0.3 do RFC-0209: medido.** O staging é **positivo na mediana** nas
   células rmw/overwrite quentes (ycsb_f_mc4 +33% med; 10k +17% med;
   kvrocks_set single +15% med), **levanta o piso** da pior célula e é
   neutro nas guardiãs (apply 1,005; mc50 1,020 med). É **negativo na
   mediana** em ycsb_a_mc4 (−10%) e ycsb_f single (−2,8%).
3. **P1.1 (flip do default): NÃO.** A regra do RFC exige min-of-3 ≥ alvo
   sem regredir guardiãs — o min caiu em ycsb_f single (1,881→1,094) e
   ycsb_a_mc4 (1,537→1,115). Default permanece env-limpo; staging fica
   opt-in (`PEDRA_WAL_BUFFER=1`), revertível, com este finding datado.
4. **P1.2 (hat ycsb_a write-side): fechado com número.** Neste boot
   ycsb_a single nobuf = 2,602 min — a perda de 0,605 do sweep p201o é
   boot-specific, não uma perda estrutural do caminho de escrita; o
   buraco estrutural vivo (same-boot, min) é **ycsb_f_mc4 0,532 nobuf /
   0,780 buf** — dono: escalonamento do grupo rmw (herança 0201), não o
   syscall write() (o buf não o fecha).

## Nota de semântica (async same-class)

Com `PEDRA_WAL_BUFFER=1`, o Ok async pode retornar com bytes ainda em
user-space (nenhum `write()` até o cap/drain) — a mesma classe do
WritableFile bufferizado do Rocks `sync=false` (a tese do RFC-0209). A
coluna G1 (fdatasync antes de Ok) é intocada: o drain antes do fd é
forçado pelas regras (c)/(d) e pinado por teste em arquivo real
(`rfc0209_sync_after_staged_flushes_before_fd`).

Serial bruto: `caixote logs linux-gate-p149b` (linhas P209B_*; capturas
`p209a-meter.log` / `p209b-meter.log` no scratch da sessão). Entrypoints:
`p209a_entrypoint.sh` / `p209b_entrypoint.sh` no scratch da sessão
(reconstruídos do base `p04_entrypoint.sh` — o da p201r2 foi perdido com o
scratch anterior).
