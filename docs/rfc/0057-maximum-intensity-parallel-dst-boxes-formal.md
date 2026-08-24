# RFC-0057: Intensidade máxima — DST paralelo em escala, caixas no CI e formalização do que falta

**Status:** draft (P0 done; P1.1–P1.3 done — P1.4 é 0052 P2)  
**Updated:** 2026-08-24

## Em uma frase

Depois de 0050 (World in-tree) e 0051 (π sobre o `ConcurrentDb` real), este RFC fecha o
**espaço mecânico verificável**: explorar seeds **em paralelo** até a taxa que o hardware
der, rodar as **caixas** (Miri / TSan / ASan — regras de empilhamento do 0052) como jobs
irmãos no CI, e **formalizar o que falta** (kernel do group-commit + validação OCC,
fechando o P2.1 do 0056) — com oráculo independente em cada camada e residual publicado.

## Sobre "100% de certeza que não tem bugs"

Essa frase não é um estado alcançável por método finito — nem FDB, nem TLA+, nem nós
(Dijkstra: teste mostra presença de bugs, não ausência). O que este RFC entrega é o que
o repo já contrata no 0056 dito com precisão: **100% do espaço mecanicamente verificável
relativo ao TCB declarado** — todo schedule explorável por máquina roda, todo caminho
formalizável tem kernel provado, e **o residual é publicado, não escondido**:

- Verificável por execução: DST determinístico (0050), π sobre threads reais (0051),
  swarm paralelo (aqui), UB/race via caixas (aqui, regras do 0052).
- Verificável por prova: kernels Verus/Aeneas→Lean (35 existentes + group-commit/OCC aqui).
- **Não** verificável hoje (residual honesto): kernel do OS/fsync real (det_io/TCG ficam
  como no 0052), hardware, campo. `DST-VS-FDB-SIM.md` continua normativo.

## Background

- 0051 P1.3 documentou a semântica que este RFC formaliza: membros do **mesmo grupo
  atômico** commitam no mesmo instante; seq per-member ali não é ordem de serialização.
  O oráculo group-aware (`RunReport::group_ranges`) codifica isso empiricamente; falta o
  **teorema**.
- As forensices PCT viviam num `static` global (`pct_hooks::GROUP_RANGES`) — dois trials
  paralelos no mesmo processo corromperiam um ao outro. Corrigido neste slice: o estado
  é por-run (`Turnstile`), exposto no `RunReport`. Paralelização total começa aqui.
- 0052 (caixas) tem os slices prontos e `todo` (Miri smoke P0.2/P0.3, ASan P1.1, TSan
  P1.2, TCG P2.x) — este RFC **agenda** esses slices no CI, não os re-inventa; as
  recusas (Miri-in-TCG, TCG-bench, ASan+Miri no mesmo processo) ficam intocadas.
- 0056 deixou P2.1 (formalização do `ConcurrentDb`) gated; os dentes do 0051 deram a
  semântica exata que o kernel precisa provar.

## Problems This Solves

- **Problem:** 1 trial PCT por vez ≈ minutos para 256 seeds — escala linear de
  exploração limitada a um núcleo.
- **Problem:** UB e data races não são observados por PCT (π lógico ≠ scheduler do SO);
  Miri/TSan existem como scripts mas não como jobs de rotina.
- **Problem:** a correção do commit path em grupo (o argumento que fechou o diagnóstico
  do 0051 P1.3) vive em comentário de código e oráculo de teste — nenhum teorema.

## Proposed Solution

1. **Paralelizar**: executor de swarm multi-núcleo (trials independentes, dirs
   independentes, forenses por-run); CI matrix paralela; telemetria de throughput
   (seeds/s total) — sem claim de CPU-hours.
2. **Caixas como jobs irmãos**: executar os slices do 0052 no `synthetic-field`
   (Miri smoke, TSan, ASan), cada um com oráculo e skip explícito quando a caixa não
   existir.
3. **Formalizar o grupo**: kernel Verus do group-commit (assign/validate/apply/fence) +
   lema de atomicidade de grupo; extrato Aeneas→Lean; freeze no `pedra_formal`.

## Delivery slices (mandatory)

### P0 — paralelização total (in-process)

- [x] **P0.1** Forenses por-run: `group_ranges` deixa de ser static global e passa a
  estado do `Turnstile`, drenado para o `RunReport` — trials paralelos nunca
  interferem — status: `done` (removido `pct_hooks::GROUP_RANGES`; `record_group_range`
  via thread-local do worker; `occ_three_teeth` lê `report.group_ranges`)  
- [x] **P0.2** (absorvido no P0.1) trial paralelo não-interferente como teste de
  regressão — status: `done` (`pct_parallel_trials_no_interference`: 4 seeds × 2
  trials concorrentes vs serial ⇒ mesmo `schedule_hash` **e** mesmas `group_ranges`)  
- [x] **P0.3** Executor `world_swarm`: bin/teste que particiona uma faixa de seeds por
  núcleos (`std::thread::scope`), cada seed num `World::run` isolado (dir próprio),
  oráculo por seed (`silent_wrong=0`, `row_half_indexed=0`, `trace_hash` igual ao run
  serial do mesmo seed); telemetria `seeds/s` total no relatório — status: `done`
  (RFC-0059 P0.1: `run_swarm` work-stealing + backend mem + gate
  `world_swarm_parallel_matches_serial`; ver 0059)
- [x] **P0.4** CI: job `world-parallel` (ubuntu, `--test-threads` natural + swarm)
  com o executor do P0.3 e gate de determinismo serial-vs-paralelo — status: `done`
  (synthetic-field `world-parallel`: swarm tests + campanhas 3n/7n/9n-4ranges +
  determinismo por `cmp` do JSONL; nota de preempção real do OS documentada no job)
  - **Achado de carga (2026-08-23, bateria do 0058 P0):** sob carga extrema
    (múltiplas suítes fsync-heavy encadeadas na mesma máquina), o teste
    `pct_disk_fence_three_teeth` do 0051 flakou 1× (43/43 verde no re-run
    limpo). Mecanismo plausível: a premissa "run-to-completion não forma
    grupo multi-membro" (`seq_hits == 0`) assume que **só o π** estaciona o
    líder mid-commit — uma preempção real do SO sob carga quebra a mesma
    janela. Implicação para P0.3/P0.4: o executor paralelo multiplica essa
    carga; o dente seq precisa de janela insensível a preempção real do OS
    (ou o assert vira estatístico com tolerância) antes do CI paralelo.

### P1 — caixas no CI (executa os slices do RFC-0052; recusas herdadas)

- [x] **P1.1** Miri: rodar 0052 P0.2+P0.3 (`miri_dst_smoke.sh`, 2 testes de recovery,
  `MIRI_REQUIRED=1` no Ubuntu; skip residual documentado fora dele) — status: `done`
  (supply-chain `miri-dst-smoke`: nightly+miri, `MIRI_REQUIRED=1`)  
- [x] **P1.2** TSan: rodar 0052 P1.2 (`PEDRA_RUN_TSAN=1`, Ubuntu) **e** um alvo novo:
  `pct_concurrent` build TSan — o runner é o cenário de corrida por excelência; fail
  se TSan acusar; **nunca** TSan e PCT no mesmo processo (regra 0052) — status:
  `done` (`race_job.sh` com dois alvos em processos separados: `concurrent_race_stress`
  + `pct_runner_without_pct_replays_and_covers_engine`, o runner sobre um
  `ConcurrentDb` real com políticas Sequential/RoundRobin — π lógico PCT fora do
  processo; `TSAN_REQUIRED=1` no job `tsan-box`. Nota: TSan não roda em Apple Silicon —
  o job Ubuntu é a autoridade; nativamente o alvo é o teste de replay/cobertura verde)  
- [x] **P1.3** ASan: rodar 0052 P1.1 (capi + posix, Ubuntu, fail-closed) — status:
  `done` (supply-chain `capi-asan-harness` pré-existente, `ASAN_REQUIRED=1`)  
- [ ] **P1.4** TCG: fica como 0052 P2 (guest com imagem, `trace_hash` nativo vs guest,
  **proibido** wall-clock) — este RFC só referencia; não antecipa — status: `todo`

### P2 — formalização do que falta (fecha 0056 P2.1)

- [ ] **P2.1** Kernel Verus `group_commit`: estados assign→validate→apply→fence com o
  **lema de atomicidade de grupo**: membros do mesmo grupo são simultâneos (nenhuma
  ordem de serialização entre eles) e commit cross-group com `w ∈ (snap, seq)` ⇒
  validação detecta (first-committer-wins) — a semântica do 0051 P1.3 como teorema —
  status: `todo`  
- [ ] **P2.2** Extrato Aeneas→Lean do kernel (`GroupCommit.lean`) + theorem-link no
  CI (`lean_wal_apply_reopen` ganha o novo alvo) — status: `todo`  
- [ ] **P2.3** Inventário final formalizável-vs-residual: tabela kernel×glue extendida
  (group-commit/OCC dentro; io_uring ring e OS scheduler fora, com motivo) — status: `todo`  
- [ ] **P2.4** Freeze: novos kernels no `pedra_formal --ci` (fail se drift) — status: `todo`

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Forenses por-run (fim do static global) | done | `Turnstile::take_group_ranges` + `RunReport::group_ranges` | 2026-08-23 |
| P0.2 | p0 | Trials paralelos não-interferentes | done | `pct_parallel_trials_no_interference` (4 seeds, hash+forenses estáveis) | 2026-08-23 |
| P0.3 | p0 | Executor `world_swarm` multi-núcleo | done | `run_swarm` work-stealing + gate serial-vs-paralelo (`trace_hash`, cmp JSONL) | 2026-08-24 |
| P0.4 | p0 | CI `world-parallel` | done | job `world-parallel` (synthetic-field: swarm tests + campanhas 3n/7n/9n-4ranges + determinismo) | 2026-08-24 |
| P1.1 | p1 | Miri smoke no CI (0052 P0.2) | done | supply-chain `miri-dst-smoke` (`scripts/miri_dst_smoke.sh`, `MIRI_REQUIRED=1`) | 2026-08-24 |
| P1.2 | p1 | TSan job + alvo pct_concurrent (0052 P1.2) | done | `race_job.sh` 2 alvos: `concurrent_race_stress` + `pct_runner_without_pct_replays_and_covers_engine` (sem PCT no processo; XOR 0052) | 2026-08-24 |
| P1.3 | p1 | ASan job (0052 P1.1) | done | `capi-asan-harness` (job pré-existente, agora slice requerido) | 2026-08-24 |
| P1.4 | p1 | TCG = 0052 P2 (referência) | todo | — | 2026-08-23 |
| P2.1 | p2 | Kernel Verus group-commit + lema | todo | — | 2026-08-23 |
| P2.2 | p2 | Lean `GroupCommit` theorem-link | todo | — | 2026-08-23 |
| P2.3 | p2 | Inventário formalizável-vs-residual final | todo | — | 2026-08-23 |
| P2.4 | p2 | Freeze dos novos kernels | todo | — | 2026-08-23 |

---

## Acceptance Criteria

- **Tests** (unit + e2e scenarios named)
  - P0: `pct_parallel_trials_no_interference` (já verde); `world_swarm` determinismo
    serial-vs-paralelo ×8 seeds; oráculos por seed no relatório.
  - P1: `miri_dst_smoke.sh` verde no nightly (ou skip documentado com comando); TSan
    job sem reports; ASan idem.
  - P2: `GroupCommit.lean` build ok; lema de atomicidade sem `sorry`; freeze verde.
- **Telemetry / Analytics**
  - `seeds/s` total do swarm (local e CI) — medida própria, **sem** comparação com
    FDB CPU-hours.
- **Documentation**
  - Status table na mesma mudança que o código; LEDGER L44/L45 apontando o programa;
    `DST-VS-FDB-SIM.md` continua normativo para claims.
- **Screenshots**
  - backend-only — não se aplica.

## Claims (o que se pode / não se pode dizer)

| Quando | Pode dizer | **Não** pode dizer |
|--------|-----------|-------------------|
| P0 done | "swarm paralelo in-tree; determinismo por seed preservado sob concorrência" | "exploramos mais schedules que o FDB em CPU-hours" |
| P1 done | "UB/races checados por Miri/TSan/ASan nos alvos nomeados, em jobs irmãos" | "sem UB em todo o binário"; "simulamos como o FDB e mais" |
| P2 done | "commit path em grupo tem kernel provado (atomicidade + first-committer-wins)" | "Pedra provadamente sem bugs"; "100% de certeza" |
| Sempre | "100% do espaço verificável relativo ao TCB declarado, residual publicado" | "mais confiável que FDB"; "garantia total" |

## Out of scope (non-goals)

- Provar ausência total de bugs (impossível; o contrato é o TCB do 0056).
- Reescrever nada em Flow / actor runtime novo.
- Vendorar envelope `dst_core` multi-alvo.
- TCG-bench, Miri-in-TCG, ASan+Miri no mesmo processo (recusas vivas do 0052).
- Fuzz de gramática nova (fica no 0020); papel HTTP (a escolha do 0050 foi fold).
- Paridade RocksDB/FDB de throughput como meta deste RFC (regras do Agents.md intactas).

## Relação com o mapa existente

| Doc | Relação |
|-----|---------|
| RFC-0050 | Base (World in-tree); P0.3 reusa `World::run` por seed |
| RFC-0051 | Base (π); P0 conserta o hazard de paralelismo que o 0051 introduziu |
| RFC-0052 | P1 daqui **executa** os slices de lá; teses de empilhamento intocadas |
| RFC-0053 | Método formal (extratos, lemmas) aplicado ao group-commit |
| RFC-0056 | P2.1 daqui **fecha** o gated P2.1 de lá (formalização do ConcurrentDb) |
| L44/L45 | LEDGER: programa apontado aqui; gates originais mantidos |
