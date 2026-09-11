# RFC: 0204 — espelhos aposentados: cotas derivadas de ponta a ponta + âncoras re-marcadas

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0203](0203-escada-viva-cotas-maquina-ancoras-classe.md) (escada
viva: contratos de twin, inventário terminal, âncoras por classe — 5/5),
[0199](0199-escada-de-contagem-complexidade-verificada.md) (a escada que
declarou a dívida dos espelhos), [0187](0187-teorema-experimento-tcb.md)
(ns nunca é teorema; re-âncora é medição datada, não promove claim)

Nota de régua: fatias de prova e âncoras medidas. Nenhum claim de perf ou
cartaz; a régua RocksDB default `sync=false` (`ROCKS_PARITY_SYNC=0`)
segue intocada. Âncora DIAG não vira quiet por desejo — só por loadavg.

> **Tese:** o 0203 deixou a escada MANTIDA POR MÁQUINA nos contratos
> (twin ↔ fn de produção ↔ anotação), mas dois registros de dívida
> continuam abertos no próprio repo: (1) o header do
> `LsmCompactCount.lean` ainda declara "count twins are hand-written Nat
> mirrors … declared debt until the P2.1 cost tool derives them
> mechanically" — a ferramenta deriva `step_work` (constante por
> expansão) e emite os contratos, mas os ESPELHOS Nat (as funções de
> contagem de iteração) e os teoremas de cota registrados continuam
> hand-escritos em 5 arquivos `*Count.lean`; (2) as âncoras da tabela
> `host_anchors.tsv` são honestas mas congeladas — darwin DIAG (loadavg
> 10–16) sem rito de re-marcação quando a caixa ficar quieta, linux
> sem ns de barreira isolado (a âncora é um pino de FASES por op), e
> nenhuma semântica de supersessão (âncora velha nunca é substituída,
> só acumula). Este RFC paga os dois: a ferramenta passa a EMITIR os
> espelhos Nat e as cotas dos pares inscritos (o hand-mirror se aposenta
> arquivo a arquivo, contrato e registro andando no mesmo commit), e a
> tabela de âncoras ganha rito de re-marcação datada com supersessão
> explícita.

## Background (fatos)

- `scripts/ratchet/derive_count_annotations.py` (0199 P2.1 + 0203 P0.1):
  conta textualmente o trabalho de UMA expansão de cada fn inscrita (6
  fns / 5 pares), emite `CountDerived.lean` (defs + teoremas rfl/decide)
  e `twin_contracts.tsv` (7 linhas `count`, gate
  `check_twin_contracts.py` 6/6).
- Espelhos hand: `LsmCompactCount.lean`, `ProbeLadderCount.lean` (2
  pares), `FlushAmortCount.lean`, `ScanDecisionCount.lean`,
  `BloomCount.lean` — funções Nat de contagem + teorema registrado +
  pontes semânticas (`cont ⇒ índice +1 exato`, …), todos sem sorry,
  compilados no mesmo lake build.
- Sem anotação derivada (contrato registra teorema de mão): `wal_commit_plan`
  (WorkIo.lean — contagem de CONSTRUTORES na álgebra `Work.io`, modelagem,
  não espelho de loop Rust) e `bloom_may_contain` (BloomCount.lean — sem
  inscrição na ferramenta; inscrever era non-goal do 0203).
- Âncoras (`scripts/ratchet/host_anchors.tsv`, consumo por teste
  `host_anchor_table.rs` 2/2): `linux_fdatasync` = pino de fases quiet
  2026-09-10 (labeled-stale pré-0193 na fonte); `darwin_fdatasync`
  17667 ns e `darwin_fullfsync` 4002000 ns = DIAG 2026-09-11 (loadavg
  10–16, `findings/2026-09-11-rfc0203-p11-darwin-fullfsync-anchor.md`).
- Gates vivos: twin-contracts 6/6, inventory-terminal 6/6 (caso
  new-pair-without-row), depth-floor 5/5 (floor_count 7), lean
  `--required` com `--check` da ferramenta.

## Problems This Solves

1. **Dívida declarada dos espelhos** — o header promete aposentadoria
   "when the P2.1 cost tool derives them mechanically"; hoje a ferramenta
   deriva CONSTANTES, não ESPELHOS. Um refactor no kernel muda o shape do
   loop e o espelho hand envelhece em silêncio (só o `--check` das
   constantes pega; a função Nat de iteração pode divergir do loop real).
2. **Âncora congelada** — DIAG para sempre se ninguém re-medir; sem rito,
   uma re-medição quiet não sabe onde morar (linha nova? sobrescreve?
   o teste de consumo aceita os dois?).
3. **Cobertura de anotação** — 2 dos 7 pares sem nada derivado; o do
   bloom é inscrevível (extract `Bloom` existe no pin Aeneas), o do
   wal é álgebra por design (fica hand, registrado como tal — sem dívida).

## Proposed Solution

A ferramenta cresce um passo: de anotadora a EMISSORA de espelhos. Para
cada par inscrito ela emite, num arquivo derivado por kernel, a função
Nat de contagem de iterações (do MESMO parse de corpo de loop que hoje
conta `step_work`) e a cota `iterações × step_work` como teorema
rfl/decide; as PONTES semânticas (o que um `cont` prova sobre o extract)
permanecem humanas num `*Bridges.lean` declarado — ponte é sobre a
semântica do extract Aeneas, não sobre contagem. O teorema REGISTRADO
mantém o nome (o registro é byte-stável; só a coluna `lean_file` anda),
o twin test Rust não muda, e o contrato no `twin_contracts.tsv` troca a
anotação de "teorema de mão" para def derivada — tudo no mesmo commit,
gates verdes. Aposentadoria = arquivo `*Count.lean` hand vira
`*Bridges.lean` (só pontes) ou morre; o gate twin-contracts é a prova de
continuidade (nenhum contrato pode ficar sem anotação existente).

Na tabela de âncoras: linhas novas nunca sobrescrevem — a antiga ganha
`superseded-by` datado, a nova entra com fonte própria; o teste de
consumo exige que toda classe tenha UMA linha viva (não-superseded)
datada e fontada. O exemplo `fullfsync_anchor` ganha modo `--gate-quiet`
(exit 1 se loadavg ≥ limiar): re-marcar darwin quiet vira procedimento
gateável, não desejo.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** Primeiro espelho aposentado (`lsm_compact`, o nome da
  dívida): a ferramenta emite a função Nat de iteração dos dois loops
  (`lsm_compact_src_loop`/`lsm_compact_inner_loop`) + cota
  `iterações × step_work` em arquivo derivado; teorema registrado
  `lsm_compact_work_bound` aponta para o derivado (nome byte-stável);
  `LsmCompactCount.lean` vira `LsmCompactBridges.lean` (só as pontes
  semânticas, declaradas como humanas); contrato twin-contracts troca
  anotação mão→derivada no MESMO commit; gates + lean `--required` +
  twin test `lsm_compact_count.rs` (intocado) verdes — status: `done`
  (2026-09-11; `LsmCompactDerived.lean` AUTO-GENERATED (template
  `drain_over_levels` validado contra o parse: fns inscritas, drain com
  exatamente 1 nested call, teorema emitido == registrado); lake build
  verde 61 libs + 13 compose; registro anda só na coluna lean_file;
  twin test 7/7 byte-intocado; contrato anota `lsm_compact_work_bound`
  @ arquivo gerado)

### P1 — next wave (depends on P0 or clearly deferrable)

- [ ] **P1.1** Espelhos restantes dos inscritos: `probe_order_covering` +
  `scale_predict` (ProbeLadder), `scan_guard`, `auto_flush_due` — mesma
  receita da P0.1, um kernel por commit, registro byte-stável —
  status: `todo`
- [ ] **P1.2** Bloom inscrito: `bloom_may_contain` entra na ferramenta
  (nova inscrição Aeneas sobre o extract `Bloom` do pin — non-goal do
  0203, agora é o passo); anotação derivada + espelho emitido;
  `BloomCount.lean` aposenta o espelho, pontes ficam; contrato anda no
  mesmo commit — status: `todo`

### P2 — later / polish

- [ ] **P2.1** Rito de re-âncora: `fullfsync_anchor --gate-quiet`
  (exit 1 se loadavg ≥ limiar; imprime linha pronta para a tabela);
  `host_anchors.tsv` ganha semântica `superseded-by` datada (linha velha
  nunca apagada); teste de consumo exige uma linha VIVA por classe;
  re-medição darwin executada quando houver janela quiet real (senão a
  linha DIAG continua, honesta) — status: `todo`
- [ ] **P2.2** Âncora linux de barreira ISOLADA (ns de `fdatasync(2)`
  sozinho, não o pino de fases): requer caixa linux quiet; se a janela
  não existir, linha `deferido` datada no inventário da âncora via
  vocabulário do 0203 — status: `todo`

## Status (living — update with every PR)

| Slice | Band | Outcome | Status | Start | End |
|---|---|---|---|---|---|
| P0.1 | p0 | Espelho lsm_compact emitido pela ferramenta, hand-mirror aposentado | done | 2026-09-11 | 2026-09-11 |
| P1.1 | p1 | Espelhos restantes dos inscritos aposentados | todo | — | — |
| P1.2 | p1 | Bloom inscrito na ferramenta (anotação derivada) | todo | — | — |
| P2.1 | p2 | Rito de re-âncora com supersessão datada | todo | — | — |
| P2.2 | p2 | Âncora linux de barreira isolada (ou deferido datado) | todo | — | — |

## Acceptance Criteria

- **Tests:** em cada fatia de aposentadoria, o MESMO commit carrega: código
  da ferramenta + arquivo derivado + registro (`lean_file` andando) +
  contrato atualizado + RFC flip; `check_twin_contracts.py` (6/6),
  `check_inventory_terminal.py` (6/6), `check_depth_floor.py` (5/5) e
  lean `--required` verdes; twin tests Rust dos pares tocados verdes SEM
  edição (prova de continuidade — a cota registrada não mudou de nome nem
  de forma). Na P2.1, `host_anchor_table.rs` estendido exige linha viva
  por classe + supersessão datada coerente.
- **Telemetry / Analytics:** none — por quê: espelhos são provas e âncoras
  são TSVs versionados com findings datados; nada vive em métrica runtime.
- **Documentation:** flips deste RFC no commit de cada fatia; ledger
  §"Escada de contagem" atualizado quando o último espelho hand cair;
  runbook `docs/runbooks/verification-gates.md` §Movimento de linha ganha
  o passo de aposentadoria de espelho na P1.1.
- **Screenshots:** backend-only (gates Lean/Rust + TSVs).

## Out of scope

- Teorema de ns ou durabilidade física (0187): re-âncora é medição datada;
  supersessão não promove DIAG a quiet sem loadavg.
- `WorkIo.lean` como dívida: contagem de construtores na álgebra `Work.io`
  é MODELAGEM (o par `wal_commit_plan` fica com anotação hand registrada
  por design — o contrato 0203 já registra isso).
- Pontes semânticas automáticas: `cont ⇒ índice +1 exato` é sobre a
  semântica do extract — permanece humana, declarada em `*Bridges.lean`.
- Perf/cartaz/Rocks: cotas e âncoras não são wins; régua
  `ROCKS_PARITY_SYNC=0` intocada.
- Arquivos in-flight da sessão paralela (0192–0196, 0201, 0202, kernels):
  consumo só por teste e tabela, nunca por edição.
