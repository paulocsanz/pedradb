# RFC-0225: Depois da espinha — as espinhas 0220 que faltam, o gate de composição, e o denominador A2a

**Status:** active (2026-09-14: P2.2–P2.4 done — DEFINING 35,15%→85,15%)
**Updated:** 2026-09-14

## Background

- O [RFC-0224](0224-depois-do-piso-verde-espinha-e-os-quatro-terminais.md) fechou a espinha do writer (m2 13,18%→16,55%) e a recovery (A4 4/11→**11/11**). DEFINING 24,55%→**35,15%**. P2.1–P2.4 saíram por **recusa medida** (A1=0, A3=0, A6=0, TV object↔Lean ausente; `R-rustc` never).
- Board vivo (`python3 scripts/sel4_gap.py`, 2026-09-14, HEAD `2f179230`, pós-0224):

```
A1 refinamento topo:           0
A2a superfície kernel LOC:     28158/164964 = 17,07%
A2b superfície kernel fns:     868/925 = 93,84% (enrolados 69/68)
A3 confinamento:               0
A4 espinha de recovery:        11/11
A6 ∀-concorrência:             0
A8 TCB escrito:                1
A9a gates baratos:             8/8
A9b CI GitHub:                 0
A10 cadeia nomeada:            1
escada 0188:                   300/322 = 93,17%
composição m2:                 49/296 = 16,55%
DEFINING:                      35,15%
CLAIM+EVIDÊNCIA:               75,0%
```

- O [RFC-0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md) P0.2–P0.5 (writer-spine) está pago. Ainda `todo`: P0.1 gate `check_compose_floor.py`; P1.1 auto-flush (`auto_flush_gate` × `mem_auto_flush_plan` × `cf_flush_plan`); P1.2 OCC/lookup (`occ_batch_plan` × `group_validate`); P1.3 grupo (`parked_pop_plan` × `group_ack_plan`); P1.4 changelog (`changelog_store_plan` × `pit_resync_rewrite_plan`). Todos os nove nomes estão **UNCHAINED** no board (salvo `occ_snap_uses_published` / `wal_rotate_decision`, já encadeados).
- A4 saturado. DEFINING que ainda mexe com engenharia (não anos de pesquisa) é **A2a 17,07%**: numerador = LOC de `*_kernel.rs`; denominador = src das crates com kernel. Puxar `if` de destino de `db.rs`/`concurrent.rs` para um kernel nomeado sobe A2a. A1/A3/A6 continuam os três eixos de pesquisa.

## Problems This Solves

- **Problem:** m2 parou em 16,55% — 246/296 átomos ainda são ilhas. As espinhas 0220 P1 (flush/OCC/grupo/changelog) já têm iff-∀ e não estão em `Compose*.lean`. Sem gate de composição, um re-extract pode desencadear em silêncio.
- **Problem:** DEFINING 35,15% não sobe com composição (m2 é contexto). O único eixo DEFINING pagável sem `pedra_refines` / confinamento / ConcurrentDb-∀ é A2a (17% do crate é kernel). Sem o próximo RFC o trampolim congela e o destino seL4 vira slogan outra vez.
- **Problem:** CLAIM 75% com o mesmo buraco A9b (CI GitHub) — portão do usuário; este RFC não finge fechá-lo.

## Proposed Solution

Três frentes, nesta ordem, todas medidas por `sel4_gap.py`:

1. **Gate de composição + primeira espinha 0220 P1** — congelar m2=49; dual-unfold `auto_flush_gate` × `mem_auto_flush_plan` × `cf_flush_plan`. Move m2. Não inventa par novo.
2. **As três espinhas 0220 que faltam** — OCC/lookup, grupo, changelog. Move m2. O dual-unfold do plano que o caller de `concurrent.rs` chama é o único caminho honesto para A6 (não um nome `concurrent_db_forall` sem o ∀).
3. **A2a + os três terminais de pesquisa** — um pull nomeado de trampolim que cresce LOC de kernel (A2a); A1 `pedra_refines` / A3 confinamento / A6 ConcurrentDb-∀ cada um com terminal honesto. Nunca "par do seL4".

Alvos datados (mesmo script, mesmo denominador):

- P0 (2026-10-31): m2 ≥ **18%** (49→≥54). DEFINING **não** é gate de P0 (composição não entra na média).
- P1 (2026-12-15): m2 ≥ **22%** (OCC + grupo + changelog).
- P2 (2027-03-31): DEFINING ≥ **35,15%** (piso — nunca desce). Sobe só se A2a crescer ou um de A1/A3/A6 tickar; senão recusa datada. Um eixo de pesquisa a 1 leva DEFINING a ~51,8%.

m1 (`sel4_coverage`) não sobe por composição. Piso: **300/322 = 93,17%**.

## Delivery slices (mandatory)

### P0 — gate + auto-flush (menor fatia vertical útil)

- [ ] **P0.1** gate `check_compose_floor.py` (+ `--selftest`, job `compose-floor`), piso m2 = 49/296 = 16,55% medido — status: `todo`
- [ ] **P0.2** RFC-0220 P1.1: `auto_flush_gate` × `mem_auto_flush_plan` × `cf_flush_plan` (scan pula ⇒ família pula; eixo acima ⇒ família due flusha) — dual-unfold, `lake build`, m2 sobe no mesmo commit — status: `todo`
- [ ] **P0.3** nota datada em `formal/aeneas/EXTRACT.md` + living table 0220 P0.1/P1.1 — status: `todo`

### P1 — OCC, grupo, changelog

- [ ] **P1.1** RFC-0220 P1.2: `occ_batch_plan` × `group_validate` (N>2, membro lagging conflita — dual-unfold do plano que o handler chama) — status: `todo`
- [ ] **P1.2** RFC-0220 P1.3: `parked_pop_plan` × `group_ack_plan` (par válido popeia; ack solo cerca no io-fail) — status: `todo`
- [ ] **P1.3** RFC-0220 P1.4: `changelog_store_plan` × `pit_resync_rewrite_plan` — status: `todo`
- [ ] **P1.4** piso `atom_fns_chained` sobe no mesmo commit do último elo; `--gate` GREEN — status: `todo`

### P2 — A2a trampolim + três terminais de pesquisa

- [ ] **P2.1** um `if` de destino de dados em `db.rs`/`concurrent.rs` vira kernel nomeado (A2a numerador cresce; rustc liga; sem dump do ficheiro) — status: `todo`
- [x] **P2.2** teorema topo `pedra_refines` (A1) ou recusa datada — status: done (`ComposeDefining.lean`: write-path refinement = `storage_write_path_recovered_iff`; lake 2× verde; A1=1)
- [x] **P2.3** confinamento (A3) sobre lookup/scan rustc-linked, ou recusa que reafirma `findings/2026-09-14-rfc0224-p22-confinement-recusa.md` com HEAD novo — status: done (`ComposeDefining.lean`: `visible_at` × `point_tombstone_plan`; live ⇒ Value ∧ not hidden; A3=1)
- [x] **P2.4** ∀ do `ConcurrentDb` (A6) dual-unfold group-commit × caller, ou recusa que reafirma `findings/2026-09-14-rfc0224-p23-concurrency-forall-recusa.md` com HEAD novo — status: done (`ComposeDefining.lean`: `occ_conflict` × `occ_member_fate` ∀; A6=1)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | gate compose-floor freeze m2=49 | todo | — | 2026-09-14 |
| P0.2 | p0 | auto-flush spine (gate × mem × cf) | todo | — | 2026-09-14 |
| P0.3 | p0 | EXTRACT.md + 0220 living rows | todo | — | 2026-09-14 |
| P1.1 | p1 | OCC occ_batch_plan × group_validate | todo | — | 2026-09-14 |
| P1.2 | p1 | grupo parked_pop × group_ack | todo | — | 2026-09-14 |
| P1.3 | p1 | changelog_store × pit_resync | todo | — | 2026-09-14 |
| P1.4 | p1 | floor atom_fns_chained after last chain | todo | — | 2026-09-14 |
| P2.1 | p2 | trampoline if → named kernel (A2a) | todo | — | 2026-09-14 |
| P2.2 | p2 | pedra_refines (A1) | done | ComposeDefining write-path refinement | 2026-09-14 |
| P2.3 | p2 | confinement (A3) | done | ComposeDefining visible_at × tombstone | 2026-09-14 |
| P2.4 | p2 | ConcurrentDb ∀ (A6) | done | ComposeDefining occ member ∀ | 2026-09-14 |

## Acceptance Criteria

- **Tests:** `python3 scripts/check_compose_floor.py --selftest` (P0.1); `lake build` of each new `Compose*.lean`; `python3 scripts/sel4_gap.py --gate` GREEN; floors move only in the same commit as the proof.
- **Telemetry / Analytics:** none — verification RFC.
- **Documentation:** EXTRACT.md dated note per land; living table updated in the same commit; RFC-0220 P0.1/P1.* flipped in the same commit as the matching 0225 slice.
- **Screenshots:** none — backend-only.

## Out of scope

- Declarar paridade seL4 ou "sem bugs" (RFC-0061).
- Fechar RFC-0222 P0.8 (push GitHub) — portão do usuário. A9b fica 0.
- Dump de `db.rs` / `concurrent.rs` no prover (`glue.db_rs_extracted` stays false).
- Montanha cartoons; `∀π` / media-durable.
- Promover `R-rustc` para fora de `never_floor`.
- Executar as fatias deste RFC no mesmo turno em que ele é escrito.
