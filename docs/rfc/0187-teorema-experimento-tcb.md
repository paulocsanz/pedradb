# RFC: 0187 — Teorema / Experimento / TCB: circuitos de garantia e aprofundamento formal

**Status:** draft
**Updated:** 2026-09-10
**ID:** 0187
**Parents:** [0051](0051-beyond-fdb-sim-holes.md) (runner PCT sobre código real),
[0059](0059-massive-scale-parallel-dst-and-cluster-invariants.md) (campanhas swarm CI),
[0070](0070-pct-depth-not-forall-schedules.md) (PCT não é ∀ escalonamentos),
[0151](0151-three-teeth-as-is-verus-dst.md) (contrato three-teeth),
[0166](0166-prova-de-fato-refinamento-propriedades.md) (corpus machine-checked)
**Peer:** RocksDB default `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).
G1 (fdatasync antes do Ok) não se compara com sync-peer. Sync-peer não é win.

> Toda garantia do Pedra decomponde em três camadas: **teorema** (∀ sobre
> código/modelo), **experimento** (estatística/mecânica) e **TCB** (axioma
> nomeado). Hoje a fronteira existe mas está espalhada; as campanhas são
> estatísticas que nunca viram um ∀; e o lado da prova tem bloqueios
> medidos sem série agendada. Este RFC fecha o circuito: gates exaustivos
> auto-verificantes + ratchets nos CI públicos, ledger de camadas, e a
> série que move classes de experimento para teorema.

## Background

Estado atual (fatos, com dono no repo):

| eixo | hoje | onde |
|---|---|---|
| Concorrência rede | PCT d=3/d=4 + exaustivo N≤3 (66 escalonamentos) + lock interleavings; `synthetic-field` PR 1024/256/256 seeds exit-1; `world-nightly` caça seeds frescas (base YYYYMMDD) | `crates/pedradb-world/src/pct_kernel.rs`, `pct_concurrent.rs`, `coverage.rs` |
| Durabilidade | `fdatasync` antes do Ok é o produto (G1); adversarial de WAL; Env injetor de falhas | `tests/wal_durability_adversarial.rs`, `crates/pedradb-sim/src/failing.rs` |
| Prova | corpus Verus+Kani pinado por sha256 ("proof regression é build regression"); catálogo com pares `l28_*` claim On; kernels three-teeth; extrações Aeneas (61 libs) | `proof-check.yml`, `src/verified.rs`, `scripts/formal/`, RFC-0166 |
| Fronteira honesta | "PCT-ordered World run is not ∀ OS schedules" já registrado | RFC-0070, `pedradb-world/src/lib.rs:149` |

O que a conversa que originou este RFC mediu/estabeleceu:

1. **Campanha ≠ ∀.** PCT d=3/d=4 é amostragem. O exaustivo N≤3 (66
   escalonamentos) é um ∀ completo **no modelo limitado** — e é o único
   ∀ de concorrência que o CI consegue afirmar deterministicamente.
2. **Durabilidade decompõe em três:** (a) *fdatasync-before-Ok* —
   propriedade do script de I/O, **provável** (é o que a extração paga);
   (b) semântica de `fdatasync` — axioma do SO (TCB); (c) "o disco
   persistiu" — **só experimento** (TCG guest power-cut, `F_FULLFSYNC`),
   e mesmo `F_FULLFSYNC` verifica até o *contrato do disco*, nunca a
   física (drive que mente no flush mente no read-back até power-cut
   real). `disk-not-media` / `never_floor` / `∀π` são TCB permanente.
3. **Negativa upstream medida (fire 803):** Aeneas origin/main (89
   commits à frente do pin) **não** alarga o conjunto traduzido —
   bloqueio raiz é `Box<dyn Iterator>` em `StreamingVisibleIter`
   ("Dynamic trait types are not supported yet", recusa no nível do
   tipo, antes de qualquer `-filter-trait-methods`). Sem re-pin.
   Correções de medição: start-from em TYPE emite só o decl; "Code
   failed to compile" do charon = pattern não casada (provado com nome
   inexistente). Detalhes: `findings/2026-09-09-aeneas-iterator-widen.md`.
   A rota viável para os iters é **conversão Isolated-method** (padrão
   `probe_order_covering`), não re-pin.

## Problems This Solves

- **Problem 1 — regressão silenciosa de cobertura:** campanha pode
  ficar verde enquanto o espaço explorado encolhe (menos seeds, menos
  interleavings alcançáveis, enumerador quebrado 66→40). Nada falha.
- **Problem 2 — bug pego pode se perder:** seeds rotativas (nightly
  YYYYMMDD) não garantem que um defeito achado uma vez reprovoque para
  sempre.
- **Problem 3 — durabilidade sem gate exaustivo:** a adversarial de WAL
  não é ∀ sobre pontos de crash; nenhum piso liga "sítio de barreira no
  código" a "ponto injetado no teste".
- **Problem 4 — prova sem série agendada:** dyn-Iterator bloqueia os
  iters; conversão Isolated está mapeada mas não é slice; pares `l28_*`
  são claims On sem série de teoremas (rank H, user-gated).
- **Problem 5 — fronteira dispersa:** a classificação teorema /
  experimento / TCB vive em skills e RFCs espalhadas; não existe ledger
  único que diga o que cada garantia é.

## Proposed Solution

- **A. Circuito campanha→gate (concorrência):** exaustivo N≤3 como gate
  bloqueante **auto-verificante** (oracle por escalonamento + asserção
  de contagem exata + hang=vermelho); **ratchet de seeds** (arquivo
  versionado, PR replaya todas, falha se encolher); hunt fresco continua
  nightly não-bloqueante.
- **B. Circuito durabilidade:** injeção exaustiva de pontos de crash via
  Env que falha (∀ sobre o modelo de injeção) com oracle fail-closed
  (CRC, TX all-or-nothing, sobreviveu-exatamente-o-acked) + asserção de
  contagem; **piso de sítios de barreira** (contagem estática no código
  == conjunto injetado); TCG power-cut e `F_FULLFSYNC` como nightly
  experimental.
- **C. Ledger de camadas:** tabela única classificando cada garantia em
  teorema / experimento / TCB, com o TCB nomeado e congelado
  (`never_floor`, `disk-not-media`, `∀π`, escalonamentos-OS-fora-do-
  conjunto, contrato-do-disco-abaixo-do-fullfsync).
- **D. Série de aprofundamento formal:** conversão Isolated-method do
  heap-sift (`StreamingVisibleIter`) como próximo kernel three-teeth;
  watch upstream com regra re-pin-só-se-alargar-sem-sorry; série L28
  (pares `l28_*` → teoremas), user-gated.

Regra transversal: **job que não é determinístico e reproduzível não é
gate — é nightly.** Toolchains pinadas por sha256, sem rede real, sem
relógio, seeds fixas.

## Delivery slices (mandatory)

### P0 — must ship first (gates determinísticos no CI público)

- [x] **P0.1** Gate exaustivo de concorrência auto-verificante: job
  bloqueante que roda os 66 escalonamentos N≤3 com oracle por
  escalonamento, `assert explorados == 66`, e `timeout-minutes` (hang =
  deadlock = vermelho) — status: `done`
  (`crates/pedradb-world/src/bin/gate_exhaustive_kernel.rs` + job
  `p01-exhaustive` em `.github/workflows/verification-gates.yml`;
  medido: as plantas buggy e clean enumeram o MESMO espaço — 181 nós/66
  folhas — logo o ∀ limpo cobre o espaço completo onde o bug vive;
  `--selftest` 4/4: violação, enumeração truncada, encolhimento de
  cobertura, oracle mentiroso)
- [x] **P0.2** Ratchet de seeds PCT: `pct_seeds.json` versionado; job de
  PR replaya todas as seeds pinadas; CI falha se o arquivo encolher —
  status: `done` (`crates/pedradb-world/src/bin/gate_seed_ratchet_kernel.rs` +
  `scripts/ratchet/pct_seeds.txt`: 15 seeds, floor 15, 8 violadores d=3
  pinados com hash de escalonamento; engine12 replay 2× h1==h2 + reopen
  durável; `--selftest` 4/4: encolhimento, expectativa flipada, oracle
  cego ao bug, hash adulterado)
- [x] **P0.3** Gate exaustivo de crash-injection: enumerar todos os
  pontos de crash no run com Env injetor (`pedradb-sim/failing.rs`),
  oracle CRC fail-closed + TX all-or-nothing + sobreviveu-exatamente-o-
  acked, asserção de contagem de pontos, bloqueante — status: `done`
  (`crates/pedradb-world/src/bin/gate_crash_injection_kernel.rs`; total de ops
  falháveis medido por busca binária sobre `tripped()` no MESMO seam que
  injeta — zero drift por construção; 9/9 pontos injetados, cada um
  `tripped`; morte abrupta via `mem::forget`; fase de corrupção: 7
  flips de byte no WAL → 7 recusas (fail-closed, nunca silent-wrong);
  `--selftest` 6/6)
- [x] **P0.4** Piso de sítios de barreira: script conta `fdatasync`/
  barreiras no código de produção e exige que cada sítio apareça no
  conjunto injetado do P0.3 — status: `done`
  (`scripts/check_barrier_floor.py` +
  `scripts/ratchet/barrier_sites.tsv`: 145 sítios pinados em 59 entradas
  (arquivo, tipo) com igualdade exata — sítio novo não injetado OU
  barreira removida = vermelho; amarração dinâmica `--crash-log` exige
  sync≥1 no stream injetado; `--selftest` 2/2)

### P1 — next wave (piso de cobertura, ledger, primeira conversão)

- [x] **P1.1** Piso de cobertura de interleavings: métricas do
  `coverage.rs` gravadas em floor versionado; CI falha abaixo do piso —
  status: `done` (`crates/pedradb-world/src/bin/gate_coverage_floor_kernel.rs`
  + `scripts/ratchet/coverage_floor.tsv`: união das máscaras de 32 seeds
  pinadas (World real, config do soak) = 11/15 sítios requeridos +
  floor_pop; os 4 sítios não cobertos ficam nomeados no TSV como
  território do soak adaptativo; `--selftest` 3/3)
- [x] **P1.2** Ledger de camadas (teorema/experimento/TCB) como doc
  vivo + gate de consistência contra o catálogo formal — status: `done`
  (`docs/verification-ledger.md` +
  `scripts/check_ledger_consistency.py`: marcador de contagens ==
  catálogo vivo e todo ponteiro `catalog:<id>` resolve; `--selftest`
  2/2 — contagem stale, ponteiro solto)
- [x] **P1.3** Conversão Isolated-method do heap-sift de
  `StreamingVisibleIter`: kernel three-teeth em `merge.rs` (território
  limpo, db.rs intocado), extração Aeneas + gates formais — status:
  `done` (kernel `sift_step`/`sift_step_as_is` + `sift_down` roteado com
  as MESMAS duas comparações; extração via `scripts/aeneas_merge.sh`;
  teoremas `Merge.lean` `merge_sift_step_repairs_iff` /
  `_swap_right_iff` / `_as_is_diverges_on_repair`; dente 3 on-live
  `merge_heap_sift_kernel_three_teeth`; catálogo `merge_sift`
  single_artifact; promovido a primeiro `close` do ladder RFC-0188 P0.2)
- [x] **P1.4** Protocolo de watch upstream Aeneas/Charon com a regra
  re-pin-só-se-alargar-sem-sorry e o procedimento sandbox medido do fire
  803 (`--sysroot default`, shim clone com `#[path]` absolutos) —
  status: `done` (`docs/upstream-watch-protocol.md`: regra permanente,
  pins como TCB, procedimento sandbox em 6 passos, bloqueios medidos do
  fire 803 — dyn-Trait nível de TIPO; charon traduz o crate inteiro)

### P2 — later / polish (experimentos e série L28)

- [ ] **P2.1** Série L28: primeiro par `l28_*` promovido a teorema
  three-teeth (rank H, **user-gated** — não iniciar sem decisão) —
  status: `todo`
- [x] **P2.2** Nightly experimental de durabilidade física: TCG guest
  (sem KVM) power-cut por barreira + run `F_FULLFSYNC` no macOS — hunt,
  não gate — status: `done`
  (absorvido: [RFC-0229](0229-host-tcb-media-sched-pct-stdenv-liveness.md) P2.1;
  `media_durable_admitted` continua `false`;
  `scripts/rfc0229_tcg_fullfsync_hunt.sh`)
- [x] **P2.3** Exaustivo N=4 com poda/simetria se o custo do runner
  permitir (∀ num bound maior) — status: `done`
  (absorvido: [RFC-0229](0229-host-tcb-media-sched-pct-stdenv-liveness.md) P2.3;
  `run_exhaustive_pruned`; não é `∀π` do host)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Gate exaustivo concorrência (66, self-verifying, hang=red) | done | `gate_exhaustive.rs` + `verification-gates.yml` | 2026-09-10 |
| P0.2 | p0 | Ratchet de seeds PCT versionado | done | `gate_seed_ratchet.rs` + `scripts/ratchet/pct_seeds.txt` | 2026-09-10 |
| P0.3 | p0 | Gate exaustivo crash-injection (oracle fail-closed) | done | `gate_crash_injection.rs` | 2026-09-10 |
| P0.4 | p0 | Piso de sítios de barreira | done | `scripts/check_barrier_floor.py` + `scripts/ratchet/barrier_sites.tsv` | 2026-09-10 |
| P1.1 | p1 | Piso de cobertura de interleavings | done | `gate_coverage_floor.rs` + `scripts/ratchet/coverage_floor.tsv` | 2026-09-10 |
| P1.2 | p1 | Ledger teorema/experimento/TCB | done | `docs/verification-ledger.md` + `scripts/check_ledger_consistency.py` | 2026-09-10 |
| P1.3 | p1 | Heap-sift Isolated-method kernel (three-teeth) | done | `merge.rs` `sift_step` + `Merge.lean` + `merge_sift` | 2026-09-10 |
| P1.4 | p1 | Protocolo watch upstream (re-pin só se widen) | done | `docs/upstream-watch-protocol.md` | 2026-09-10 |
| P2.1 | p2 | Série L28 user-gated | todo | — | 2026-09-10 |
| P2.2 | p2 | Nightly TCG power-cut + F_FULLFSYNC | done | RFC-0229 P2.1 | 2026-09-16 |
| P2.3 | p2 | Exaustivo N=4 com poda | done | RFC-0229 P2.3 | 2026-09-16 |

## Acceptance Criteria

- **Tests**
  - P0.1: job nomeado no CI roda 66/66 com oracle por escalonamento;
    mutação em qualquer escalonamento do conjunto derruba o job; job
    com enumerador truncado falha por conta da asserção de contagem.
  - P0.2: remoção de seed do arquivo versionado → CI vermelho;
    replay da seed que pegou um bug histórico reprovoca.
  - P0.3: mutação que perde write ackeado ou sobrevive write não-acked
    → vermelho; CRC corrompido na reabertura → vermelho (fail-closed);
    contagem de pontos < esperado → vermelho.
  - P0.4: adicionar um sítio de barreira sem injetá-lo → vermelho.
- **Telemetry / Analytics** — none — o sinal é binário (CI
  vermelho/verde) e a contagem explorada é asserida dentro do runner;
  métricas de cobertura do P1.1 viram floor versionado, não dashboard.
- **Documentation** — este RFC + ledger de camadas (P1.2) + linhas de
  registro (EXTRACT/finding) quando P1.3/P1.4 mudarem o corpus.
- **Screenshots** — none (backend/CI-only).

## Out of scope

- Provar o contrato do SO ou do disco (TCB nomeado, não objetivo).
- Rigs físicos de power-cut; firmware de drive.
- Comparações com sync-peer ou lead tables com a coluna sync (regras
  Rocks parity do repo permanecem).
- Boards congeladas (cartoon montanha) e demais user-gated sem decisão
  explícita do usuário.
- Re-pin de Aeneas/Charon sem widen medido sem sorry (regra permanente).
