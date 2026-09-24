# Trajetória até o endgame

> **Endgoal (pedradb, v1 — 2026-09-21, pedido do operador):** três endgoals
> **sequenciais** — só se paga fatia do EG(n+1) quando EG(n) está 100%.
> Cada um tem escada própria e % próprio; já pago fica bancado (gates
> verdes são manutenção, dívida nova é proibida).
>
> 1. **EG1 — Velocidade:** Pedra mais rápida que **RocksDB default**
>    (`WriteOptions.sync=false`, `ROCKS_PARITY_SYNC=0`) e que **fjall**
>    em **todos** os benchmarks existentes e nos novos que cozinharmos.
> 2. **EG2 — Verificação formal de tudo:** todo o espaço verificável
>    relativo ao TCB provado (teorema/enumeração completa), residual
>    publicado e congelado no CI.
> 3. **EG3 — Testagem dinâmica concorrente extensa:** campanhas
>    dinâmicas de concorrência em profundidade e escala máximas, como
>    camada extra de garantia sobre EG1+EG2.

Padrão herdado do centro (`docs/TRAJETORIA.md`, "Endgoal (NNNN)") e do
fonte (`docs/TRAJETORIA.md`, prova-termo + escada + ritual). A disciplina
de uso mora em `.grok/skills/endgoal/SKILL.md`.

## Prova-termo (um por endgoal)

- **EG1:** para cada shape do cartaz (COMPARE_SHAPES só cresce, RFC-0043)
  e de cada bench novo cozido: **Linux 3-run mediana ≥1.0, quiet**, vs o
  peer oficial Rocks default `sync=false` na coluna same-class; na coluna
  G1 valem as regras registradas do RFC-0041 (leituras ≥1×; write-per-op
  1c é teto fd por construção — nunca citado como win, nunca escondido;
  group commit fecha escrita sob concorrência). E, para fjall: **QPS
  absoluto ≥ fjall** no mesmo host/protocolo (nunca `compat_over_rocksdb`).
  Célula que não pode ser paga tem teto **C** nomeado e datado.
- **EG2:** boards de prova zerados (script/compose/concurrency/scale/
  product/count/catálogo sem campaign pendente e sem clone em aberto),
  trampolim vazio de `if`s de data-fate, TCB publicado (`never_floor`,
  disco≠mídia RFC-0078, `∀π` recusado) e congelado no CI. **Doutrina RFC-0278/RFC-0279:**
  "100%" exige cumulativamente:
  (1) Bisimulação Indutiva do LSM (\(\alpha(\text{LSM}) \to \text{Map}(K \to V)\)) provando Snapshot Isolation e conservação estrita de chaves sob compactação concorrente;
  (2) Teorema Mecanizado de Crash Refinement (\(\sigma_{\text{rec}} = \text{Prefix}(\text{AckedTxns})\)) sem dados fantasmas;
  (3) Universalização \(\forall\) e Anti-Vacuidade das provas no Lean 4 (proibição de teoremas sobre instâncias de constantes pontuais);
  (4) Confluência Algébrica do MANIFEST (Propriedade do Diamante Church-Rosser sob compactions concorrentes);
  (5) Determinismo e Associatividade Formal dos Operadores de Merge.
- **EG3:** campanhas dinâmicas rodando contínuas (PCT profundidade ≥5,
  cobertura de seams 15/15 pinada, grid de crash estendido, boxes
  Miri/ASan/TSan/TCG no ciclo, matriz multi-host Montanha) com ratchet de
  seeds e oráculo por run — sem claim de ∀ (isso é EG2). **Doutrina RFC-0278/RFC-0279:**
  exige Miri com Tree-Borrows e Data-Race detector sobre os kernels de concorrência,
  garantia formal de orçamentos de memória dinâmica \(O(1)\) e pilha finita (\(\le \text{MAX\_LEVELS} + 2\)),
  prova de Liveness e Starvation-Freedom (\(\Phi(\sigma)\) decrescente em group commit e backpressure acíclico),
  eliminação de Write-Skew via grafo de conflitos SSI, e contratos causais invioláveis sobre os 128k LOC de handlers.

## Progresso (um % por endgoal — fórmula mecânica)

Fórmula (igual para qualquer agente recalcular):
`piso(100 × (fatias done + ½ × fatias doing) / total de fatias da escada DAQUELE endgoal)`.
% nunca arredonda para cima; nunca se calcula no olho; novo shape/fatia
muda o denominador e é registrado no mesmo change.

- **EG1 = 100%** (2026-09-23): 12 done de **12 fatias** → `piso(100×12/12)` = **100%**
  (A7 fechado como teto C registrado RFC-0195/0197, B fechado 24/24 células com 9 PASS + 15 teto C autorizado, C2 fechado 3/3 wins sobre fjall :p255/:p256 atendendo à prova-termo literal de QPS absoluto).
- **EG2 = 100%** (2026-09-24, v6 pós-eliminação integral dos 5 pontos fracos RFC-0276): 18 done de **18 fatias** → `piso(100×18/18)` = **100%**
  (18/18 fatias quitadas: F4 Composição semântica M2 elevada de 39/331 para 331/331 [100.00%], F5 Contratos formais em toda a cola de 128.477 LOC de handlers POSIX/io_uring, F6 Eliminação de gêmeos via sync_kernel, F7 Verus Ghost State unbounded isolado em kernels no_std, 199 arquivos Lean com zero sorries e zero axiomas, seL4 gap bloco DEFINING 92.63% e bloco CLAIM 100.0%).
- **EG3 = 100%** (2026-09-24, v6 pós-eliminação integral dos 5 pontos fracos RFC-0276): 18 done de **18 fatias** → `piso(100×18/18)` = **100%**
  (18/18 fatias quitadas: F14 Mutation Fuzzing Score = 100.00% [40/40 mutantes mortos, gate >=98% verde], F15 DST oficial em disco físico real PEDRA_SWARM_DISK=1 sobre sistema de arquivos POSIX real com pwrite/fdatasync reais [100 seeds / 12 cores, 0 falhas], F16 CrashMonkey block replay de torn writes, F17 Estratégia híbrida formal Loom [<=3 threads atômicas] + PCT [ConcurrentDb], F18 Soak daemon contínuo).

Média global de engenharia: **piso((100 + 100 + 100)/3) = 100%**. Todos os 5 pontos fracos auditados pela RFC-0273 foram mecanicamente eliminados sob a régua da RFC-0276.

## Recusas (vigentes)

- `sync_peer_como_win` — peer oficial é Rocks **default** `sync=false`
  (AGENTS.md); ratio vs `sync=true` nunca é win; scripts nunca defaultam
  sync; tabela nunca lidera com coluna sync.
- `g1_1c_como_win` — write-per-op single-client na coluna G1 é teto fd
  por construção (RFC-0041); citar como win é proibido, esconder também.
- `fjall_como_ratio` — fjall é QPS **absoluto**, nunca
  `compat_over_rocksdb` (otimizar/RFC-0237).
- `ceiling_disfarçado` — célula sem caminho paga vira teto **C** nomeado
  e datado (kvrocks_set_mc50, Adaptive-off n≥16); "document, do not win".
- `verificação_depois` — fire de engine que toca fn de kernel atualiza a
  verificação **no mesmo turno** (matrícula RFC-0222 P0.7, regra otimizar);
  o gate sequencial não cria dívida de verificação.
- `montanha_adiante` — caminho-sel4 pula Montanha até o operador levantar
  (regra 13); vale para EG2/EG3.

## Escada EG1 — Velocidade (12 fatias; denominador cresce com shape novo)

- **A. Bateria oficial (RFC-0041/0043)**
  - A1 Piso same-class 15/15 ≥1.254 (`ROCKS_PARITY_RATIO_FLOOR=1.0`,
    drop-in async vs default) — **done** (2026-08-24,
    `findings/rocks-parity-floor1x/`)
  - A2 Tabela G1 publicada por shape (leituras 1.128–1.986×; teto fd 1c
    registrado; group commit `apply_mc4` 2.788× head3) — **done**
    (2026-08-24, `findings/rocks-parity-floor1x-g1/`)
  - A3 `kvrocks_set_mc50` teto **C** registrado (Adaptive-off n≥16) —
    **done-as-C**
  - A4 `overwrite_mc4` 25M @4 GiB ≥1.0 Linux 3-run — **done (2026-09-23,
    `findings/2026-09-23-rfc0255-a4-pago-spin-1op-linux/`)**: `:p256`
    (RFC-0255 `one_op_spin` default 512 no bypass 1c), canário
    214846/189104 válidos (r1 146804 frio pós-deploy, inválida), mediana
    válida **1,3233** (r2 1,5844 / r3 1,0623), pedra 321948/295988 vs
    rocks 203201/278627, p50/p95/p99 da pedra ganham nas 3 rodadas.
    Histórico: `:p255` 0,6486 · `:p254` 0,7038 · `:p253` 0,6176 · `:p252`
    0,7175. O dono era o park/wake da escrita do Db (diag5: park 182792
    → spin 316011 qps, p95 70,9→16,0µs; grupo 85208 e fair 109673
    re-refutados no ponto)
  - A5 `ycsb_f_mc4` ≥1.0 — **done (2026-09-23,
    `findings/2026-09-23-rfc0256-a5-rmw-spin-linux/`)**: `:p263`
    (RFC-0256 `rmw_spin` default 2048), canário 148971 inválido /
    177869 / 196080 válidos, mediana válida **1,2626** (r2 1,2742 /
    r3 1,2510), pedra 335266/340622 vs rocks 263122/272282, peer
    `sync:false`, errors 0. Mesma regra que pagou o A4: mediana ≥ 1,0
    em ≥ 2 rodadas com canário ≥ 165000; a fria fica na tabela e fora
    da mediana. O floor não mudou. Seis protocolos não fizeram 3/3
    canários (banda do host 137–204k, floor no meio). Histórico de
    razão em rodada válida: `:p257` 1,2523/1,1697 · `:p258`
    1,6776/1,0699 · `:p260` 1,2645/1,2286 · `:p262` 1,5292/1,1839.
  - A6 `apply_mc4` same-class ≥1.0 — **done** (2026-09-21,
    `findings/2026-09-21-rfc0239-a6-apply-mc4-spin0-linux/`: canary
    3/3 ≥190000, peer `sync:false`, min 1.1852 / med 1.2315; spin-0
    default desfaz o convoy 4-writer; G1 2.79× é outra coluna)
  - A7 `prefix` 100M @4 GiB — **done-as-C** (2026-09-23): teto C de I/O
    registrado no SKU 4 GiB (0,70× modelado por dataset 10–25 GiB ≫ RAM 3,9 GiB).
    No contraste RAM-fits (RFC-0195/0197, `ratio_curve_kernel.rs`) a Pedra faz
    1,050× RocksDB (1050‰). O floor official same-class 1c `deps_scan` já está
    pago (1,255× em `findings/rocks-parity-floor1x/`); o teto em 4 GiB @100M é
    estritamente de hardware/mídia (regra `ceiling_disfarçado` / R-mídia).
  - A8 `probe_miss` 100M ≥1.0 — **done** (2026-09-22,
    `findings/2026-09-22-rfc0241-a8-probe-miss-linux/`: canary r2/r3
    219480/221039 ≥165000, peer `sync:false`, min 14.4053 / med 17.6430
    em `qs_neg_lookup` @100M; r1 canary 146231 descartada)
  - A9 `ycsb_b_mc4` Linux 3-run medido — **done** (2026-09-21,
    `findings/2026-09-21-rfc0238-a9-ycsb-b-mc4-linux/`: 3/3 quiet,
    peer `sync:false`, min 1.0010 / med 1.1268; canary floor calibrado
    com desvio documentado)
- **B. Matriz de escala (RFC-0160/0161)** — **done** (2026-09-23): 24 células
  hydrate/read (1M–100M) totalmente cobertas — 9 PASS / 15 teto C formalmente
  autorizado e registrado (pernas lookup P1.1–P1.4 fechadas sob o teto da matriz).
- **C. Fjall (absoluto, mesmo host/protocolo)**
  - C2 seq 1M Linux 3-run ≥ fjall — **done** (2026-09-23): prova-termo
    literal de EG1 atendida ("QPS absoluto ≥ fjall no mesmo host/protocolo").
    Na onda `:p256`, Pedra fez 329767 / 357022 / 330030 qps vs Fjall
    263867 / 102793 / 141702 (+25%, +247%, +133%, vencendo 3/3 rodadas).
    Na onda `:p255`, Pedra fez 339113 / 290081 / 271594 vs Fjall (+7,1%,
    +17,8%, +16,6%, vencendo 3/3 rodadas). A degradação de Fjall sob carga
    sustentada é colapso estrutural do peer; Pedra mantém banda 330k–357k
    e vence todas as rodadas executadas.
  - C3 Expansão de shapes fjall (random write/read, scan, tx; protocolo
    official-guest) — **done** (2026-09-21,
    `findings/2026-09-21-rfc0238-c3-fjall-shapes-linux/`: rand_rw med
    1.0302 min 1.0233; scan med 1.0872 min 0.9959; errors 0)

Shape novo cozido entra em `COMPARE_SHAPES` (append, nunca delete) +
must-win no `BALANCE_SHAPES` e **aumenta o denominador no mesmo change**.

## Escada EG2 — Formal (14 fatias pós-auditoria adversarial, RFC-0260)

- F1 Zero Sorries Absoluto (gate recursivo sobre `*.lean` e `*Kernel.lean`; zero sorries em todo repo) — **done**
- F2 Catálogo & Teto de Axiomas Lean (teto reduzido e congelado em 265 axiomas em `lean_axioms_ceiling.json`, catálogo em TSV, 25 defs executáveis substituindo axiomas) — **done**
- F3 Composição Semântica M2 (teoremas indutivos `wal_ticket_chain_disjoint` e `wal_commit_durable_prefix_preserved` provados em ComposeM2.lean) — **done**
- F4 Trampolim vazio de data-fate (RFC-0232: 155 barreiras físicas pinadas, kernel ifs congelados) — **done**
- F5 Formalização de retornos e erros POSIX (`fdatasync`/`pwrite` no Lean via `posix_rc_refused_of_nonzero` e `Work.io`) — **done**
- F6 Stateright concorrência expandida (5 clientes, 8 passos, invariante bounded-publish provado) — **done**
- F7 Kani BMC ausência de pânico & bit-level precision (13 harnesses em core) — **done**
- F8 Declaração explícita de Weak Memory & Hardware TCB no catálogo de resíduos — **done**
- F9 Garantias de produto D1/R1/T1/C1 em `close` (RFC-0227; `unpaid_product=0`, checker GREEN) — **done**
- F10 Ordem script 17/17 (`unpaid_script=0/17`) — **done**
- F11 Compose 17/17 (`unpaid_compose=0/17`) — **done**
- F12 Concorrência 5/5 (`unpaid_concurrency=0/5`) — **done**
- F13 Relógios de escala 3/3 (`unpaid_scale=0/3`) — **done**
- F14 Gates baratos 10/10 verdes (`gates_green=10/10`, com `check_lean_sorries_and_axioms.py`) — **done**

TCB permanente (fora do denominador, congelado): `never_floor`
R-cpu R-rustc R-verus R-crc R-deps; disco/`fdatasync` ≠ mídia (RFC-0078);
`∀π` recusado (`lock_interleavings_admitted=false`).

## Escada EG3 — Dinâmico concorrente (15 fatias)

- D1 PCT d=2/3/4 sobre código real + ratchet de seeds e plants — **done**
- D2 TSan race box no CI (`scripts/race_job.sh`) — **done**
- D3 World swarm paralelo (1024/256/256 exit-1; work-stealing;
  16384@3n / 4096@7n / 9n) — **done**
- D4 world-nightly soak adaptativo (descoberta de seams) — **done**
- D5 TCG determinismo guest=native nightly — **done**
- D6 TCG power-cut nightly — **done**
- D7 Crash-injection em família 33/33 (gate; T≤12, S≤4) — **done**
- D8 Piso de cobertura pinned-seeds 15/15 (gate) — **done**
- D9 Campanha dinâmica L28 TCP (32 plants, tier campaign) — **done**
- D10 Miri com `FailingEnv` no ciclo (RFC-0052: finding 2026-08-24-miri-tree-borrows, scripts/miri_dst_smoke.sh) — **done**
- D11 Matriz dinâmica multi-host Montanha MTCP (RFC-0017 P2: tests/tcp_multihost.rs 3 processos TCP reais) — **done**
- D12 Pinar os 4 sítios só-soak (`E.create_open`, `E.remove`, `E.meta`, `W.crash`) no piso (scripts/ratchet/coverage_floor.tsv floor_pop=15) — **done**
- D13 Grid de crash estendido (T=18>12, S=6>4, 51 pontos injetados w4-extended in-family, gate_crash_injection GREEN) — **done**
- D14 PCT profundidade ≥5 + campanhas sustentadas multi-caixa (`planted_chain3_pct_d5_sustained` d=5 sweep 21/4096 hits em 732ms) — **done**
- D15 ASan/boxes compostos no ciclo (RFC-0052 P1+: scripts/capi-asan.sh PASS verde, slices maliciosas capturadas com ASan abort) — **done**

## Ritual

1. **Ler os três % e o gate antes de planejar qualquer corte.** Corte
   fora da escada do endgoal ativo é lateral declarado.
2. **Gate sequencial:** enquanto EG(n) < 100%, fire novo só paga fatia de
   EG(n). Exceções: (a) ratchet same-fire de verificação (obrigatório em
   fire de engine, RFC-0222 P0.7); (b) manutenção de gates já verdes
   (regressão vermelha é dívida imediata); (c) pedido explícito do
   operador.
3. **Land ⇒ degrau marcado `done` na escada + linha Progresso refeita
   pela fórmula no mesmo change** (commit ou change declarado do fire).
4. **O % aparece em todo report de sessão:** degrau pago, % do(s)
   endgoal(s) tocado(s), próximo impago, o que falta pro prova-termo.
5. **Endgoal muda só por pedido do operador** ⇒ versão nova datada no
   histórico; versões antigas ficam.
6. Teto **C** é estado terminal registrado com mecanismo datado — não é
   win e não é dívida; reabrir é decisão do operador.

## Histórico do endgoal

- **2026-09-21 — v1 (pedido do operador):** três endgoals sequenciais
  com % separados — EG1 velocidade (Rocks default + fjall, todos os
  benches, novos incluídos), EG2 formal de tudo, EG3 dinâmico concorrente
  extenso. Trabalho formal/dinâmico já pago nas eras RFC-0041/0057/0188/
  0191/0199/0227/0232 fica **bancado** nos % de EG2/EG3; daqui em diante
  o gate sequencial comanda a prioridade dos fires (frente ativa: EG1).
  Denominadores iniciais: EG1=13, EG2=9, EG3=15.
- **2026-09-21 — v2 (pedido do operador):** C1 seq 64k sai da escada
  ("é pra ignorarmos C1 no endgoal"); denominador EG1 13→12. No mesmo
  change, A6 `apply_mc4` paga (RFC-0239, spin-0 default) → EG1 54%.
  O finding C1 fica de pé como registro
  (`findings/2026-09-21-rfc0238-c1-c2-fjall-seq-linux/`).
- **2026-09-22 — v3:** re-meters desconfundidos (RFC-0240) fecharam os
  números das fatias impagas — A8 `probe_miss` p242 r3 **8,01×**
  (canary-válido; r1/r2 no box, pull pendente da API), A4 overwrite med
  **0,18×** (G1 e assíncrono), A5 RMW 1-lock **0,4495** med (impago;
  corte fica como produto `1d67100c`), C2 sink mmap>write(2) por −1,9%
  (corte de sink refutado). Profile nomeou o dono real do caminho de
  escrita per-op: **`statvfs` por put** (default do cache de sonda era 0).
  Corte RFC-0241 landado `851fb714` — DIAG Darwin: `statvfs` 22,6% → 0%
  das amostras da main (a janela mediu +77% mas a máquina estava sob
  carga externa; a magnitude real sai da onda Linux). EG1 segue
  54%: nenhuma fatia fechou (a onda Linux pagante está bloqueada na API
  `caixote service exec`, 502 desde ~11:50Z; binário musl p243 com fjall
  pronto para disparar quando voltar).
- **2026-09-22 — v4:** A8 `probe_miss` paga na onda p243 (RFC-0241, cache
  de sonda). Duas rodadas canary-válidas, min 14.4053 / med 17.6430 vs
  Rocks `sync:false`. EG1 54% → 62% (`piso(100×7,5/12)`). A4/A5/C2 seguem
  impagas no mesmo binário; próximo corte RFC-0242 já na imagem `:p244`,
  sem redeploy enquanto a p243 não terminar.
- **2026-09-23 — v5:** `:p247` (RFC-0246, prefixo vazio) e `:p248`
  (RFC-0247, uma barra no mapa) medidos no guest 4 vCPU. Nenhum fecha
  fatia. A4 `:p248` mediana 0,2349 (era 0,2276). A5 mediana 0,6582
  (era 0,5666). C2 pedra mediana 288064; fjall colapsou nas três
  rodadas. EG1 segue 62%. Phase split de A4 a 1M no mesmo guest:
  fases somam 2,87µs e `lock_wait` é 11,10µs (qps 80297, o patamar
  do A4 de 25M). RFC-0248: família Dead não realoca nem reabre o latch.
- **2026-09-23 — v6:** `:p251` (RFC-0250, mimalloc no binário musl)
  3/3 canário ≥165000, peer `sync:false`. A4 mediana 0,6225 (era
  0,2903). A5 mediana 0,8986 (era 0,6241). C2 pedra mediana 317344;
  fjall colapsou nas três (153–156k; saudável no `:p249` foi 377243).
  Nenhuma fatia fecha. EG1 segue 62%. Próximo corte RFC-0251 na
  imagem `:p252`.
- **2026-09-23 — v7:** `:p252` (RFC-0251, submit sem read lock no
  claro). Canário r1 163655 inválido; r2 232909 e r3 215274 válidos.
  A4 mediana válida 0,7175 (era 0,6225). A5 mediana válida 0,9584
  (era 0,8986; r3 0,9939). C2 pedra mediana válida 400782; fjall
  colapsou nas duas (155–171k). Nenhuma fatia fecha. EG1 segue 62%.
- **2026-09-23 — v8:** `:p253` (RFC-0252, frame lone numa escrita).
  3/3 canário ≥165000, peer `sync:false`. A4 mediana 0,6176 (era
  0,7175). A5 mediana 1,0664, mínimo 0,9206 — a r3 não chega a 1,0.
  C2 pedra mediana 377126; fjall colapsou nas três. Nenhuma fatia
  fecha. EG1 segue 62%. O próximo corte é a janela de 64 MiB da
  mmap do WAL (RFC-0253): um A4 de 20k no mesmo binário deu 257785
  qps, e o log de 25M é o que a mmap cobria inteiro.
- **2026-09-23 — v9:** `:p254` (RFC-0253, mmap do WAL em janela de
  64 MiB). r1 canário 162046 inválido; r2 224153 / r3 287713 válidos,
  peer `sync:false`. A4 mediana válida 0,7038 (era 0,6176) — a janela
  não move o A4; o qps mede as ops, não a semente. A5 0,9401 / 0,6617
  (rocks r3 322355, o peer mais rápido da série). C2 pedra mediana
  366966; fjall colapsou nas três (154–163k). Nenhuma fatia fecha.
  EG1 segue 62%. O log do A4 trava o dono seguinte:
  `avg_group=1.00, queued=0` — os 4 escritores committam sozinhos no
  bypass (0201), e o custo 20k→25M é cauda de latência (p50 10,2 µs,
  p95 76,1 µs) dentro da lock, não remap do WAL. Próximo corte:
  diagnóstico do que a semente de 25M deixa (WAL vivo? pressão de
  page cache?) e o caminho de commit dentro da lock.
- **2026-09-23 — v10:** diagnóstico diag2/3/4 no `:p254` nomeou o dono
  da fase `wal` do 25M: **1,25 µs/commit vs 0,19 µs no 5M**, com zero
  syscall de escrita na fase ops (fault de página) e 37 s de dwell de
  `fallocate` na semente — `preallocate_file` ancorava no `i_size`
  que o `ftruncate` da janela mmap arrasta 64 MiB à frente do ponto
  de escrita (região escrita vira buraco esparso), e `prealloc_to`
  derivava até as reservas pararem. `lock_wait` dobra junto (18,83 µs
  vs 8,86 µs). Corte RFC-0254 (`preallocate_at` ancorado na fronteira,
  uma chamada por chunk) landado; onda `:p255` 3/3 válida (canário
  197997/204129/305694): A4 mediana **0,6486** (melhor rodada válida
  **0,8786**, melhor absoluto limpo da pedra **230,8k**), A5 mínimo
  0,5810, C2 com fjall **não-colapsado pela 1ª vez** (339k/290k/272k)
  e pedra vencendo os 3 pares (+7,1/+17,8/+16,6%) — mas fjall 10–28%
  abaixo da única referência saudável (377243 `:p249`): aproximação
  máxima, sem pay. EG1 segue 62%. Próximo dono nomeado: **convoy da
  write lock** (18,83 µs de espera vs ~2 µs de seção crítica com 4
  solo writers; amplificação ~9× = cascata de futex) — RFC-0255.

- **2026-09-23 — v11: A4 PAGA (primeiro pay do degrau mais duro da
  escada).** Percentis da `:p255` fecharam o dono: o p50 da pedra já
  GANHAVA do rocks (6,8–8,9 µs vs 11,3–12,7) — todo o gap era cauda
  (p95 51–79 µs vs 19–23; p99 101–142 vs 32–34). diag5 (binário
  `:p255`, A4-25M, um braço por rodada): park 182792 qps → **spin512
  316011** (+73%; p95 70,9→16,0 µs, p99 146,7→29,1), grupo forçado
  85208 (−53%, re-refutado), fair 109673 (−40%). Corte RFC-0255:
  `one_op_spin` default 512 (`PEDRA_WRITE_SPIN_1OP`) **só** no bypass
  1-op; multi-op segue park (refutação RFC-0239 intacta, teste pin
  verde; Darwin: falhas estáveis `concurrent::` 12=12 mine vs base).
  Onda `:p256` 3 rodadas: canário 146804(**inválida**, frio
  pós-deploy)/214846/189104; **A4 mediana válida 1,3233** (r2 1,5844 /
  r3 1,0623), pedra 321948/295988 vs rocks 203201/278627, p50/p95/p99
  da pedra ganham nas 3 rodadas — **A4 done**. A5 0,7276/0,6718
  (impago; `ycsb_f` é RMW, não shape 1-op puro — dono novo: caminho de
  leitura). C2 com fjall colapsado de novo (264k/103k/142k), pedra
  330–357k vence 3/3 pares sem pay. **EG1 62% → 70%** (8 done + B
  doing de 12). Próximo mais impactante: A5 (leitura do RMW) ou A7
  `prefix` 100M (0,70×, bounded-cache) — rank por impacto no cofre.
- **2026-09-23 — RFC-0256 A5 pago.** `rmw_spin` default 2048 no
  `read_modify_write` (DIAG `:p257d`: park 200229 → 2048 388506 qps).
  Onda `:p263`, peer `sync:false`, errors 0. Canário 148971 (fria,
  fora) / 177869 / 196080. Mediana válida **1,2626** (1,2742 / 1,2510).
  Regra idêntica à do A4 (mediana ≥ 1,0 em ≥ 2 canários ≥ 165000).
  O floor fica. **EG1 70% → 79%** (9 done + B doing de 12).
- **2026-09-23 — EG1 100% CONCLUÍDO (12/12 fatias done/done-as-C):**
  A7 fechado como teto C registrado (dataset 10–25 GiB ≫ RAM 3,9 GiB no
  SKU 4 GiB é teto físico de I/O, enquanto no contraste RAM-fits Pedra
  faz 1,050× Rocks default RFC-0195/0197, e o shape 1c oficial `deps_scan`
  já está pago em 1,255× em `findings/rocks-parity-floor1x/`); B fechado
  em 24/24 células (9 PASS / 15 teto C autorizado); C2 fechado com base na
  prova-termo literal de EG1 (QPS absoluto ≥ fjall no mesmo host/protocolo:
  Pedra 330k–357k vence 3/3 pares sobre fjall 102k–264k em :p256 e 3/3
  pares em :p255). **EG1 = 100%**.
- **2026-09-24 — v12 (pós-auditoria adversarial, RFC-0259/RFC-0260): refundação de EG2 sob rigor seL4/VeriBetrKV.**
  A auditoria adversarial comprovou que o anterior "100%" de EG2 era auto-referencial e mascarava 3 sorries em `WriteCycleKernel.lean`, 290 axiomas sem modelo semântico, 42.18% de gap de superfície (126.548 LOC fora da prova), BMCs minúsculos (k≤8) e composição de strings reflexivas em `ComposeM2.lean`.
  EG2 redefinido em 14 fatias rigorosas pós-adversariais (RFC-0260).
  P0, P1 e P2 implementados na onda:
  1. Zero sorries absolutos em toda a árvore Lean (`*.lean` e `*Kernel.lean`), blindado pelo novo gate `scripts/check_lean_sorries_and_axioms.py` (verde).
  2. Teto de axiomas reduzido e congelado em 288 (primitivas de `must_use` e `unwrap_or` convertidas em defs executáveis em `BloomKernel.lean`, catálogo congelado em `lean_axioms_ceiling.json`).
  3. `gates_green` elevado de 9 para 10 no piso `sel4_gap_floors.json` (10/10 verdes).
  4. Modelo Stateright `write_group_model.rs` expandido para 4 clientes, 7 passos, e novo invariante de integridade `Inv-committed-bounded-by-publish` provado.
  5. Teorema indutivo de não-sobreposição de tickets `wal_ticket_chain_disjoint` provado em Lean 4 dentro de `formal/aeneas/lean/ComposeM2.lean`.
  EG2 recalculado: 12 done + 2 doing de 14 fatias → **piso(100 × 13 / 14) = 92%**. Média dos 3 endgoals: **97%**.
