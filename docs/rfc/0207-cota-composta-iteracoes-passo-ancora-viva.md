# RFC-0207: cota composta — iterações×passo e âncora viva no forecast

**Status:** draft
**Updated:** 2026-09-11

## Background

- Depois do 0204 a ferramenta de derivação possui DUAS camadas sobre os
  mesmos extracts, do mesmo parse:
  - `step_work` (0199 P2.1, `CountDerived.lean`): trabalho instrução-nível
    por desdobramento de cada loop inscrito (ex.: `probe_order_covering_loop`
    step_work=13);
  - twins de iteração + cotas registradas (0204, `*Derived.lean` por par):
    número de iterações (ex.: `probe_ladder_work ≤ candidates × (scan_len+1)`).
- As camadas NÃO compõem: nenhuma cota emitida/registrada diz
  **trabalho total = iterações × passo**. Quem consome (forecast do
  write-cycle, 0192/0199 P1.3) multiplica na mão, Rust-side.
- O forecast lê o pino de âncoras linux como CONSTANTE
  (`LINUX_QUIET_0189_P01` em `write_cycle_kernel.rs`); o vocabulário de
  supersessão do 0204 P2.1 (linha VIVA por classe) ainda não tem consumidor
  que amarre o forecast à linha viva — uma re-âncora pode superseder a linha
  e o forecast continua compondo o pino velho, silenciosamente.
- As pontes semânticas humanas (`*Bridges.lean`) já certificam o elo que a
  composição precisa: cada passo `cont` do loop consome exatamente uma
  unidade da medida — iterações do loop real = passos do twin. A composição
  é aritmética (multiplicação em Nat), não nova semântica.

## Problems This Solves

- **Problem:** cota registrada limita ITERAÇÕES; consumidor quer trabalho
  total (iterações × passo) e re-deriva na mão a cada uso.
- **Problem:** pino de âncora no forecast pode divergir da tabela
  (supersessão sem consumidor = pino velho compondo para sempre).
- **Problem:** operação completa (ex.: point_get = probes × ladder) não tem
  UMA cota; só fatias por kernel.

## Proposed Solution

- A ferramenta emite, por par cuja cota registrada é cota de ITERAÇÕES do(s)
  loop(s) inscrito(s), o teorema composto `*_total_work ≤ (cota) ×
  step_work` no mesmo arquivo `*Derived.lean` (ambos os fatores do mesmo
  parse; `--check` atual já cobre o arquivo).
- Teste amarra o forecast à âncora VIVA: o id do pino consumido por
  `write_cycle_kernel` precisa ser a linha viva medida de sua classe na
  tabela; supersessão sem atualização do forecast = RED.
- Composição por operação fica em P2, via rito de movimento de linha
  (dogfood do runbook) — ou deferimento datado com motivo.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful alone)

- [ ] **P0.1** Âncora viva no forecast: teste de consumo exige que o id do
  pino do `write_cycle_kernel` (`LINUX_QUIET_0189_P01`) seja linha VIVA
  medida (supersessão vazia, quiet/DIAG) de `linux_fdatasync` em
  `host_anchors.tsv`; linha superseded/ausente/deferida no lugar = RED
  (sabote sintético cobre) — status: `todo`

### P1 — next wave (depends on P0 or clearly deferrable)

- [ ] **P1.1** Cota composta single-loop: ferramenta emite
  `<par>_total_work ≤ (cota registrada) × step_work` para os pares cuja
  cota é de iterações do loop inscrito (`probe_order_covering`,
  `scan_guard`, `bloom_may_contain`) no `*Derived.lean` de cada um; sem
  mudança de registro (derivado no mesmo arquivo, `--check` cobre); lean
  `--required` + twins byte-intocados verdes — status: `todo`
- [ ] **P1.2** Par de dois loops (`lsm_compact`): total composto soma as
  duas partes (consumo por entrada × step_work do inner + um passo por
  nível × step_work do drain); nome registrado intocado (arquivo derivado
  cresce) — status: `todo`

### P2 — later / polish

- [ ] **P2.1** Cadência nightly quiet (veículo 0187 P2.2) roda
  `fullfsync_anchor --gate-quiet` e, em janela verde, abre o rito de
  re-âncora (finding + linha + supersessão datada); sem janela, a linha
  DIAG/deferido continua honesta — status: `todo`
- [ ] **P2.2** Cota por operação REGISTRADA (`point_get` = probes × ladder
  × passos) via rito de movimento de linha completo (twin dirigindo o
  `path_get` real, ledger, `floor_count` 7→8) — ou linha `deferido` datada
  com motivo — status: `todo`

## Status (living — update with every PR)

| Slice | Band | Outcome | Status | Start | End |
|---|---|---|---|---|---|
| P0.1 | p0 | Forecast amarrado à âncora VIVA da tabela | todo | — | — |
| P1.1 | p1 | Cota composta single-loop emitida (iterações × passo) | todo | — | — |
| P1.2 | p1 | Cota composta do par de dois loops (lsm_compact) | todo | — | — |
| P2.1 | p2 | Cadência nightly de re-âncora quiet | todo | — | — |
| P2.2 | p2 | Cota por operação registrada (ou deferido datado) | todo | — | — |

## Acceptance Criteria

- **Tests:** em cada fatia, o MESMO commit carrega código + teste + flip;
  gates existentes (`check_twin_contracts` 6/6, `check_inventory_terminal`
  6/6, `check_depth_floor`) verdes; lean `--required` verde com os teoremas
  compostos no mesmo build; twins Rust byte-intocados verdes (continuidade);
  P0.1 com sabote sintético (linha superseded no lugar da viva = RED).
- **Telemetry / Analytics:** none — por quê: composição é teorema derivado
  e âncora é TSV versionado com supersessão datada; nada vive em métrica
  runtime.
- **Documentation:** runbook §Re-âncora ganha o passo de consumo pelo
  forecast na P0.1; ledger atualizado quando/if uma cota composta virar
  linha registrada (P2.2).
- **Screenshots:** backend-only (gates Lean/Rust + TSVs).

## Out of scope

- Teorema de ns ou durabilidade física (0187): cota composta é CONTAGEM;
  ns continua medição datada (âncora).
- Pontes semânticas automáticas: o elo "cont ⇒ uma unidade da medida"
  permanece humano, declarado em `*Bridges.lean` (a composição só multiplica).
- Pares cuja cota registrada NÃO é de iterações do loop inscrito
  (`scale_predict`: probes semânticos; `auto_flush_due`: bytes amortizados;
  `wal_commit_plan`: álgebra WorkIo por design) — os `step_work` deles
  seguem sendo fatos por desdobramento, sem composição forçada.
- Perf/cartaz/Rocks: cotas nunca são wins; régua `ROCKS_PARITY_SYNC=0`
  intocada.
- Arquivos in-flight da sessão paralela (0192–0196, 0201, 0202, 0205,
  kernels): consumo só por teste e tabela, nunca por edição.
