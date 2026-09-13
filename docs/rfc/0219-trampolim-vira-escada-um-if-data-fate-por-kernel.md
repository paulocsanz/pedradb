# RFC-0219 — O trampolim vira escada: um `if` data-fate de `db.rs`/`concurrent.rs` por kernel nomeado, a escada classe-seL4 além dos 92,47%

**Status:** draft
**Data:** 2026-09-13
**Autoria:** agente grind (round 11), sucessora direta do RFC-0218
(degrau extrato drenado, `**Status:** done` no HEAD `95755148`,
270/292 = 92,47%)

## Contexto

O RFC-0218 fechou o degrau extrato do sistema inteiro: 88 pares
pagáveis promovidos ao degrau átomo (1 iff-∀ = 1 commit),
floor_atom 178→266, floor_extract 120→12, sweep final verde em
worktree dentro de `software/`. A métrica de máquina subiu de
182/292 = 62,33% para 270/292 = 92,47% — monotônica em cada HEAD.

O board vivo pós-drenagem (2026-09-13, `candidates.py`) mede o resto:
todos os ranks formais estão pagos (script 0/17, compose 0/17,
concurrency 0/5, scale 0/3, product 0). O resíduo do catálogo é o
registrado: **10 pares cartoon Montanha** (`children`×4, `fields`×5,
`pack`×1 — portão do usuário) e **12 cânone-excluídos**
(`*_admitted`/campanha — nunca flipados). O único próximo passo
nomeado pelo board é o `leftover_next`: os `if`s data-fate do
**trampolim** — `crates/pedradb-core/src/db.rs` (22.526 linhas) e
`crates/pedradb-core/src/concurrent.rs` (9.179 linhas),
`db_rs_extracted=false` desde o RFC-0191.

Precedente registrado (RFC-0191 P1.5, status done): um `if` data-fate
vivo do trampolim é puxado a um **kernel nomeado** que o rustc linka
(`si_hist_repair_plan` em `txn_kernel.rs`; o trampolim passa a fazer
`match` no kernel), o corpo DO KERNEL (não do arquivo inteiro) é
extraído para Aeneas e o par `catalog:*` NASCE átomo no mesmo commit.
Este RFC generaliza aquele rito para a fila inteira — sem nunca
despejar `db.rs`/`concurrent.rs` no provador.

## Fila medida (2026-09-13, `grep -n -E "if .*(sync|flush|durable|visible|publish|fence|fsync)"` sobre os corpos)

- `db.rs`: **53 sítios** de decisão data-fate (gates de sync/flush/
  fence/publish/visibilidade dos caminhos de escrita e leitura);
- `concurrent.rs`: **22 sítios** (gates do caminho concorrente);
- total da fila: **75 `if`s pulláveis**, −1 por pull (contador medido
  no findings a cada commit, captura do grep datada).

## Meta mensurável

`python3 scripts/sel4_coverage.py` verde e monotônico no HEAD de cada
pull (cross-check floors). Cada pull adiciona 1 par (denominador +1)
JÁ coberto (nasce átomo: numerador +1), então a % sobe em cada commit:
início 270/292 = **92,47%**. Alvos DATADOS:
- P0 fechado: **≥ 92,54%** (273/295 — 3 pulls; 2026-09-14);
- P1 fechado: **≥ 92,83%** (285/307 — 15 pulls; 2026-09-28);
- P2 fechado: **≥ 94,01%** (345/367 — a fila medida inteira, 75 pulls;
  2026-11-30; recusa medida ajusta o alvo datado, nunca gate
  inventado);
- contador trampolim: 75 → ≤72 (P0) → ≤60 (P1) → 0 medido ou recusa
  nomeada por sítio (P2);
- portão Montanha (decisão DO USUÁRIO, registrada em ledger, não é
  fatia executável): erguido em qualquer ponto soma +10 cobertos no
  denominador atual — 283/295 = 95,93% pós-P0; 355/367 = 96,73%
  pós-P2;
- cadência: 1 pull = 1 commit (kernel + trampolim + extrato + teorema
  iff-∀ + TSV/floors + planta DST quando aplicável), mesma regra dos
  RFC-0214/0215/0216/0218.

## Fatias

- [ ] **P0.1** Rito do pull registrado e primeiro `if` do caminho de
  escrita (`db.rs`; corpo lido, kernel nomeado pelo corpo — o board
  aponta `wal_commit_plan` como plano dominante com 14 handlers) —
  par nasce átomo, contador 75→74 — status: `todo`
- [ ] **P0.2** +1 `if` do caminho de escrita (`db.rs`) — 272/294 —
  status: `todo`
- [ ] **P0.3** +1 `if` do caminho de escrita (`db.rs`) — 273/295 =
  92,54% — status: `todo`
- [ ] **P1.1**–**P1.4** +12 `if`s de `db.rs` (escrita e leitura;
  4 commits por fatia, 3 por commit) — 285/307 = 92,83% — status:
  `todo`
- [ ] **P2.1** fila `db.rs` restante medida (53−15) — status: `todo`
- [ ] **P2.2** fila `concurrent.rs` (22) — pausa em sítio tocado pela
  sessão paralela (RFC-0217) até ela commitar; nunca construir sobre
  arquivo não-commitado — status: `todo`
- [ ] **P2.3** sweep final em worktree DENTRO de `software/` (gates
  verdes, sorry 0, `lean_extracts.sh --required` exit 0) + nota datada
  em `formal/aeneas/EXTRACT.md` + `**Status:** done` — status: `todo`

## Vereditos / riscos

- `if` de trampolim não-isolável (estado espalhado por locks/epochs):
  recusa MEDIDA e nomeada (finding datado + captura), o sítio sai da
  fila com o número publicado e o alvo datado ajusta — nunca wrap
  `is_empty` para virar número, nunca spray `compact_refuse` (ambos
  nomeados pelo board como anti-padrões).
- Denominador cresce 1 por pull: a % sobe devagar por commit
  (diluição honesta). O avanço "drástico" é a FRONTEIRA — o trampolim
  medido 75→0, família por família no degrau átomo — e o teto que
  sobe: com o portão Montanha, 96,73% pós-P2.
- `promote_atom.py`/`promote.py` (rito round 8–11) segue válido: TSV
  com tabs reais, `floor_atom +1`, `atom_reason` datado,
  `residuals.json`, 1 teorema/commit — e o par NOVO entra no catálogo
  com `single_artifact`/`twin_kind` corretos (nasce átomo, nunca
  reabre `data_fate` congelado).
- A métrica NÃO é declaração de paridade com seL4 (seL4 ≈ 20:1
  prova:impl por anos, RFC-0061): mede a COBERTURA DA ESCADA (degraus
  ∀ registrados sobre a superfície declarada, agora crescendo para
  dentro do trampolim).

## Não-metas

- NÃO despejar `db.rs`/`concurrent.rs` no provador: `db_rs_extracted`
  fica false PARA SEMPRE neste RFC — só kernel nomeado por `if`
  (corpo do kernel extraído, trampolim faz `match`), nunca o arquivo
  inteiro.
- `never_floor` imutável (R-cpu R-rustc R-verus R-crc R-deps);
  `cap_data_fate ≤ 0` imutável (par novo nasce átomo, sem novo
  data_fate); campanha ≠ ∀π segue TCB.
- NÃO flipar os 12 cânone-excluídos (`forall_schedules`,
  `media_durable`, `lock_interleavings`, `stacked_liars`,
  `default_pct_depth_raised`, `pct_default_depth`, `fsync_lie_tcg`,
  `tcg_guest`, `world_trajectory`, `world_trajectory_fold`,
  `liveness_claim`, `fdatasync_rc`) — só por decisão registrada de
  cânone.
- NÃO tocar `crates/montanha-fdb-recipes/**`: os 10 pares cartoon
  só sobem se o USUÁRIO erguer o portão (decisão de ledger); ficam no
  denominador como dívida visível.
- NÃO é meta deste RFC: benchmark/paridade Rocks (RFC-0217, sessão
  paralela — seus arquivos não-commitados nunca commitados nem
  construídos), executar as fatias é das próximas sessões de grind.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | rito do pull + 1º if db.rs (75→74) | todo | — | 2026-09-13 |
| P0.2 | p0 | +1 if db.rs (272/294) | todo | — | 2026-09-13 |
| P0.3 | p0 | +1 if db.rs (273/295 = 92,54%) | todo | — | 2026-09-13 |
| P1.1 | p1 | +3 ifs db.rs | todo | — | 2026-09-13 |
| P1.2 | p1 | +3 ifs db.rs | todo | — | 2026-09-13 |
| P1.3 | p1 | +3 ifs db.rs | todo | — | 2026-09-13 |
| P1.4 | p1 | +3 ifs db.rs (285/307 = 92,83%) | todo | — | 2026-09-13 |
| P2.1 | p2 | fila db.rs restante medida (53−15) | todo | — | 2026-09-13 |
| P2.2 | p2 | fila concurrent.rs (22; pausa na paralela) | todo | — | 2026-09-13 |
| P2.3 | p2 | sweep final + EXTRACT.md + done | todo | — | 2026-09-13 |

## Critérios de aceite

- **Cada pull**: kernel nomeado no rustc que o trampolim linka
  (`match` no kernel, corpo do `if` fora do trampolim) + extrato
  Aeneas do corpo do kernel (`lean_extracts.sh --required` exit 0) +
  teorema iff-∀ no wrapper com `lake build` verde + par
  `catalog:*` NASCE átomo no mesmo commit (TSV com tabs reais,
  `floor_atom +1`, `atom_reason` datado, `residuals.json`) +
  `check_depth_floor.py` GREEN + `sel4_coverage.py` monotônico +
  contador trampolim −1 (captura datada) + planta DST quando o kernel
  tocar caminho vivo + 1 teorema/commit (`git show HEAD -- <lean> |
  grep -c "^+theorem"` == 1).
- **A métrica**: `sel4_coverage.py` verde no HEAD de cada pull,
  capturas datadas início/fim em findings; alvos datados acima; teto
  com portão Montanha publicado, nunca alegado.
- **P2.3**: worktree destacado DENTRO de `software/`: depth-floor,
  inventory-terminal, twin-contracts, campaign, extracts, sorry 0 —
  tudo verde; nota datada em `formal/aeneas/EXTRACT.md`; flip
  `**Status:** done` no mesmo commit.
