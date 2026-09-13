# RFC: 0217 — Fechar o board async: grupo em baixa concorrência, U-cells nativas, escala pesada, encode off-path, read-side e produto

**Status:** in-progress
**Updated:** 2026-09-13

## Background

O board async same-class (Pedra `PEDRA_PARITY_ASYNC=1` vs RocksDB default
`sync=false`, peer `ROCKS_PARITY_SYNC=0`, mesmo round/boot) tem hoje:

- **15/15 formas smoke ≥ 1,254** (gate oficial, `rocks-parity-floor1x/`) e
  20/20 no sweep p201o single-client — single-client e leitura pagos.
- **Fronteira de concorrência**: writers 2–4 perdem (`ycsb_f_mc4` cartaz
  Linux **0,491** default / **0,836** braço rmw p211m; DIAG Darwin mc2/mc3
  0,25–0,39), writers ≥ 6 o drenar-grupo amortiza (DIAG mc6/mc8 rmw
  1,97×/2,23× sobre o braço clean), mc50 paga (`kvrocks_set_mc50` 1,678×).
  Mecanismo medido (PHASE/wg p211p/p211q): `avg_grp` 1,00 em TODAS as
  contagens no default (nunca forma grupo); rmw 1,04→1,14→2,1→3,1
  (mc2/3/6/8) com `lwait` 0,2–1,8µs. Entre mc8 e mc50 não há ponto medido.
- **6 perdas U-cells nomeadas** (Linux 3-run, `p209-ucells-gate/`):
  kafka_changelog_flush **0,036**; ingest_sst **0,069**;
  compaction_filter_drop **0,081**; linkbench_mix **0,236**; wbwi **0,494**;
  myrocks_write_tx **0,751**.
- **Escala pesada sem meter**: prefix 100M @4GiB **0,70×**; overwrite_mc4
  25M @4GiB **0,557×** (3/3); 15M sem número — mecanismos 0193/0194/0195
  aterrizados, meter blocked (orçamento de onda + custo, datado).
- **Encode de memtable por commit**: `mem=3,13–14,2µs` (2º dono
  quantificado; −0,180 do déficit de kvrocks_set_mc50 é mem amortizado).
- **Read-side −18%** (F8): `deps_scan` single 0,831 (p201o).
- **Escada de admissão** (produto): sob banda Reclaim (FS pequeno, free <
  `DISK_SOFT_FREE_BYTES`=256MiB) o admit custa dezenas de ms por commit
  (14–18ms macOS/F_PREALLOCATE; 6–7ms Linux tmpfs) + parks 230–310ms/op
  (20–60× no tempo de célula).
- **Linha G1 single-client write-per-op** (0,001–0,056×): fd-ceiling por
  construção (p50 F_FULLFSYNC 3,8ms ⇒ ~1/p50; Linux fdatasync p50 25,7µs ⇒
  0,056×) — prova viva na família
  `rfc0041_one_fdatasync_cannot_hit_2x_rocks_default_ycsb_a`.

**Errata que re-ancora este RFC (commit `426272f4`, 2026-09-13):** o
veredito P1.2 nomeou o dono do residual 0,836 como "fdatasync per-commit da
coluna de paridade" — **falso no código**: `PEDRA_PARITY_ASYNC=1` faz
`opts.set_sync(false)` (`rocksdb-parity-bench/src/engines.rs:83-85`) e o
commit async só sincroniza sob plano G1 (`WalCommitPlan::AppendSync*`;
`concurrent.rs:1228-1243`: "Async: write() per group, no fdatasync").
Telemetria Darwin (`flsh≈0,03µs`, `wal≈1µs`) com células mc2–mc8 SUB-1
(0,25–0,84 DIAG) prova que o dono existe **sem barreira nenhuma**: é a
**seção serial por commit** — write() do WAL sob mutex + encode de memtable
+ publish sob write-lock — contra o memcpy userspace por writer do Rocks.
O fd-ceiling é mecanismo exclusivo da G1. A absolvição do escalonador
(P1.2, dois extremos) permanece de pé.

Pain/why now: RFC-0211 fechou sem caixa aberta e adjudicou o mecanismo
drenar-grupo; a direção do usuário é **resolver todos os pontos fracos do
board**, estendendo este RFC a cada etapa nova descoberta.

## Problems This Solves

- **Problem:** writers 2–4 pagam a seção serial inteira por commit (ratio
  0,49–0,84) porque o default nunca forma grupo (`async_merge_policy` só
  mergeia quando writers > ncpu; `avg_grp=1,00` sempre).
- **Problem:** seis formas de workload real (flush-por-op, ingestão SST,
  filtro de compaction, mix delete/scan, batch indexado, tx N-updates)
  perdem por rodar em emulação do compat, não em caminho nativo.
- **Problem:** as duas células de 4 GiB (0,70/0,557) estão sem meter desde
  que os mecanismos aterrizzaram — nem claim, nem fechamento.
- **Problem:** encode de memtable (3,13–14,2µs) mora no caminho serial do
  líder/membro — quantificado como 2º dono.
- **Problem:** scan/cursor −18% no lado de leitura.
- **Problem:** sob pressão de disco o admit por commit custa dezenas de ms
  (produto: qualquer deploy em FS pequeno).
- **Problem:** eixos inteiros sem número (delete-heavy, mc9–49, 1 GiB,
  p99) — cobertura para poder publicar o board com perdas nomeadas.

## Proposed Solution

- **P0:** formar grupo em baixa concorrência — janela de coleta bounded no
  líder (`PEDRA_GROUP_WINDOW_US`, default off) + attach de chegadas ao
  grupo in-flight; merge elegível a partir de 2 writers quando a janela
  está on. Amortiza write()+encode (async) e a barreira real (G1 mc≥2).
- **P1:** tirar as 6 U-cells da emulação — caminhos nativos (flush
  amortizado, SST write direto + hook de filtro no compactor, batch
  indexado nativo, tx single-writer real) ou veredito datado de teto com
  número.
- **P2:** fechar escala pesada (meter 4 GiB), encode off-path, cursor de
  scan, histerese do admit (produto) e os eixos de cobertura sem número.
- **Adjudicação formal (non-goal):** G1 single-client write-per-op é
  teto por construção — documentado, nunca win, nunca escondido.

## Delivery slices (mandatory)

### P0 — Grupo em baixa concorrência (ranking nº 1; residual 0,491/0,836)

- [x] **P0.1** Kernel da janela de coleta: `group_window_kernel` puro
  (decisão bounded: espera W µs no líder quando ≥2 writers ativos e merge
  elegível; zeros para lone/single; twin AS-IS) + wiring no caminho real
  (`submit_after_begin`/WriteGroup: `PEDRA_GROUP_WINDOW_US=N` torna o
  merge elegível em writers ≥2 e o líder espera W antes de drenar;
  caminho lone `active==1 ∧ ¬recently_concurrent` intocado) + testes
  `rfc0217_group_window_*` no caminho real. — status: `done`
- [ ] **P0.2** Attach in-flight: chegada durante o dreno/write do líder
  entra no mesmo voo (fold/stage na janela off-lock; na G1, attach também
  durante a barreira do grupo) + testes `rfc0217_inflight_attach_*`. —
  status: `todo`
- [ ] **P0.3** Meter DIAG Darwin (driver host p211p/p211q): frontier
  mc2/3/4/6/8, braços window on/off, PHASE/wg; alvo `avg_grp` mc2–mc4
  ≥2,0 e ratio DIAG mc2/mc3 saindo de 0,25–0,39 → ≥0,9; guardas
  deps_apply_batch/mc50. — status: `todo`
- [ ] **P0.4** Meter Linux gate 3-run quiet min-of-3 (âncora p149):
  `ycsb_f_mc4` default 0,491 → **≥1,0** com janela on; guardas ≥ nível
  p211m (ycsb_a_mc4, overwrite_mc4, apply_mc4, mc50); cartazes pagos em
  guarda ≥; veredito datado; **flip do default só com este meter
  válido**; se o gate seguir blocked, veredito gate-blocked datado. —
  status: `todo`
- [ ] **P0.5** Re-adjudicação do dono no Linux (errata `426272f4`): onda
  admission-clean com PHASE distribuindo a seção serial no âncora ext4
  (wal/mem/publish/lwait por commit); finding datado. — status: `todo`

### P1 — U-cells nativas (ranking nº 2; cada fatia: ≥1,0 OU teto datado com número)

- [ ] **P1.1** kafka_changelog_flush 0,036: flush amortizado no compat
  (pipeline completo por chamada — snapshot do hang: 1331/1793 amostras em
  `compact_gate` — vira batch/staged com gate) sem mudar a semântica de
  durabilidade do flush explícito. — status: `todo`
- [ ] **P1.2** ingest_sst 0,069 + compaction_filter_drop 0,081: sair da
  emulação — ingest = escrita SST direta + install; filter = hook real no
  compactor. — status: `todo`
- [ ] **P1.3** wbwi 0,494 + myrocks_write_tx 0,751: batch indexado nativo
  + tx single-writer real (`begin_occ`/commit já existem) no lugar do
  WriteBatch emulado. — status: `todo`
- [ ] **P1.4** linkbench_mix 0,236: decompor primeiro (mix scan+delete;
  telemetria read_probe/scan), atacar o dono nomeado. — status: `todo`

### P2 — Escala pesada, encode, read-side, produto, cobertura

- [ ] **P2.1** Escala pesada: meter e fechar prefix 100M @4GiB
  (0,70→≥1,0), overwrite_mc4 25M (0,557→≥1,0) e 15M (sem número→≥1,0);
  blocked atual: orçamento de onda + custo, datado. — status: `todo`
- [ ] **P2.2** Encode de memtable off-path: encode no membro antes do
  grupo ou batch-encode no apply (alvo: `mem=` saindo de 3,13–14,2µs do
  caminho do líder; recuperar o −0,180 do déficit kvrocks_set_mc50). —
  status: `todo`
- [ ] **P2.3** Read-side −18%: decompor cursor do scan (`deps_scan`
  single 0,831 DIAG p201o → ≥1,0 no cartaz Linux). — status: `todo`
- [ ] **P2.4** Escada de admissão (produto): probe com histerese/cache
  curto em vez de por commit (alvo ≤1ms/commit sob Reclaim; parks
  230–310ms eliminados do caminho quente; guarda: semântica Refuse abaixo
  do hard intacta, `disk_pressure` 11/11). — status: `todo`
- [ ] **P2.5** Cobertura dos eixos sem número: sweep delete-heavy (hat:
  perda — única família tocando deletes hoje é perda), concorrência 9–49
  (fronteira mc8→mc50 sem ponto), célula 1 GiB (faixa smoke↔4GiB vazia),
  captura p99/p999 (todas as comparações atuais são throughput). —
  status: `todo`

### Regra de extensão (board aberto)

Este RFC **permanece aberto** até toda célula nomeada do board estar ≥1,0
ou carregar veredito datado de teto-por-construção. Qualquer onda deste
RFC que descubra célula nova <1,0 abre fatia datada nova aqui (P2.6,
P2.7, …) no mesmo commit do finding — nada é descoberto e deixado sem
dono.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Kernel janela de coleta + wiring real + testes `rfc0217_group_window_*` | done | este commit | 2026-09-13 |
| P0.2 | p0 | Attach in-flight (fold na janela off-lock; G1 attach na barreira) | doing | — | 2026-09-13 |
| P0.3 | p0 | Meter DIAG Darwin frontier (avg_grp ≥2 mc2–mc4; ratio ≥0,9) | todo | — | 2026-09-13 |
| P0.4 | p0 | Meter Linux 3-run quiet (0,491→≥1,0) + veredito + flip default | todo | — | 2026-09-13 |
| P0.5 | p0 | Re-adjudicação do dono no Linux (distribuição da seção serial) | todo | — | 2026-09-13 |
| P1.1 | p1 | kafka_changelog_flush: flush amortizado | todo | — | 2026-09-13 |
| P1.2 | p1 | ingest_sst + compaction_filter: caminhos nativos | todo | — | 2026-09-13 |
| P1.3 | p1 | wbwi + write_tx: batch indexado + tx nativos | todo | — | 2026-09-13 |
| P1.4 | p1 | linkbench_mix: decompor + atacar dono | todo | — | 2026-09-13 |
| P2.1 | p2 | Escala pesada 4GiB: meter + fechar (0,70/0,557) | todo | — | 2026-09-13 |
| P2.2 | p2 | Encode memtable off-path | todo | — | 2026-09-13 |
| P2.3 | p2 | Read-side: cursor de scan | todo | — | 2026-09-13 |
| P2.4 | p2 | Escada de admissão: histerese (produto) | todo | — | 2026-09-13 |
| P2.5 | p2 | Cobertura: delete-heavy, mc9–49, 1GiB, p99 | todo | — | 2026-09-13 |

## Acceptance Criteria

- **Tests:** `rfc0217_group_window_*` e `rfc0217_inflight_attach_*` no
  caminho real (`ConcurrentDb` + env real, env pin on/off); guarda do
  caminho lone (single-client p50 sem janela — sem regressão); twin AS-IS
  com env off idêntico ao comportamento atual; `disk_pressure` 11/11
  intactas (P2.4); serial A/B ⊆ baseline conhecido + musl exit 0 a cada
  fatia de código.
- **Telemetry:** `write_group_stats` (`avg_grp` alvo ≥2 em mc2–mc4),
  `write_phase_stats` (decomposição da seção serial no Linux), contadores
  do flush amortizado (P1.1), read_probe/scan (P1.4), parks do admit
  (P2.4) — sempre na linha existente das ondas p211p/p211q.
- **Documentation:** finding datado por onda; linha viva no
  `docs/status.md`; inventário rev. 4 ao fechar o board; errata
  `426272f4` referenciada (dono corrigido).
- **Screenshots:** none — backend-only.

## Out of scope

- Ratios contra peer `sync=true` (nunca win; `rocks-parity-compare`
  exits 2) e qualquer re-medida de cartaz pago (apply_mc4 1,0859;
  kvrocks_set_mc50 1,678; G1 apply_mc4 2,788 head3) — guardas ≥ apenas.
- Mudar o contrato G1 (fdatasync antes de Ok é o produto); a linha G1
  single-client write-per-op fica no teto por construção, documentada,
  nunca citada como win nem escondida.
- Mass-fix das 23 falhas formais shared-lane (sessão separada); RFCs da
  sessão paralela (0216); multi-node/Montanha; Darwin como claim (DIAG).
