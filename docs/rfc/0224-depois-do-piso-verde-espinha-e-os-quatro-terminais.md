# RFC-0224: Depois do Piso Verde — a espinha do writer, os 7 átomos de recovery que faltam, e os quatro terminais que o seL4 pagou em anos

**Status:** active (2026-09-14: P0.4 + P0.1 done — leftover I/O + writer-spine sync)
**Updated:** 2026-09-14

## Background

- O [RFC-0222](0222-escada-ate-o-par-sel4-gap-por-eixo-com-denominador.md) fechou o Piso Verde de engenharia que não depende de fan-out: métrica por eixo, gates baratos 8/8, enrollment 69/69, ComposeWriter (m2 11,15%→13,18%), ComposeRecovery (A4 0/11→4/11). P2.3–P2.5 saíram por **recusa medida** (A3=0, A6=0, TV ausente). P0.8 (CI GitHub verde) continua portão de push.
- Board vivo (`python3 scripts/sel4_gap.py`, 2026-09-14, pós-0222 P2.2):

```
A1 refinamento topo:           0
A2a superfície kernel LOC:     28158/164738 = 17,09%
A2b superfície kernel fns:     868/925 = 93,84%
A3 confinamento:               0
A4 espinha de recovery:        4/11
A6 ∀-concorrência:             0
A8 TCB escrito:                1
A9a gates baratos:             8/8
A9b CI GitHub:                 0
A10 cadeia nomeada:            1
escada 0188:                   300/322 = 93,17%
composição m2:                 39/296 = 13,18%
DEFINING:                      24,55%
CLAIM+EVIDÊNCIA:               75,0%
```

- O [RFC-0220](0220-escada-de-composicao-encadeia-os-atomos-dual-unfold.md) P0.2 (workerless) foi pago pelo 0222 P2.1; P0.3–P0.5 da espinha do writer (sync × fence × publish) ainda são `todo`.
- `leftover_page_kernel` / `scan_readahead_kernel`: rustc liga **e** o I/O em `db.rs` chama (`90bf4b64`: leftover DontNeed + scan WillNeed).

## Problems This Solves

- **Problem:** DEFINING parou em 24,55% — recovery 4/11, writer-spine só o primeiro elo, topo/confinamento/∀-concorrência/TV = 0. Sem o próximo RFC o board congela no Piso Verde e o destino seL4 vira slogan.
- **Problem:** CLAIM está a 75% com um buraco só (A9b CI GitHub) que o 0222 nomeou portão do usuário — este RFC não finge fechá-lo.
- **Problem:** 7 átomos de recovery e 3 elos do writer já têm iff-∀ isolados e não estão encadeados.

## Proposed Solution

Três frentes, nesta ordem, todas medidas por `sel4_gap.py`:

1. **Espinha do writer (RFC-0220 P0.3–P0.5)** — dual-unfold das cadeias sync/fence/publish. Move m2 e não inventa par novo.
2. **Os 7 recovery que faltam** — o mesmo rito de `ComposeRecovery.lean` para os átomos ainda fora da composição. Move A4 4/11 → 11/11.
3. **Os quatro terminais de pesquisa** — topo (`pedra_refines`), confinamento, ConcurrentDb-∀, TV de um objeto pinado — cada um com terminal honesto (teorema ou recusa datada). Nunca "par do seL4".

Alvos datados no bloco DEFINING (mesmo script, mesmo denominador):

- P0 (2026-10-31): ≥ **32%** (writer-spine P0.3–P0.5 + wiring leftover/scan se o I/O existir).
- P1 (2026-12-15): ≥ **40%** (A4 = 11/11).
- P2 (2027-03-31): ≥ **55%** teto de engenharia deste RFC; o resto (topo/confinamento/∀/TV) continua pesquisa nomeada.

m1 (`sel4_coverage`) não sobe por composição. Piso: **300/322 = 93,17%**.

## Delivery slices (mandatory)

### P0 — espinha do writer + wiring (menor fatia vertical útil)

- [x] **P0.1** RFC-0220 P0.3: `changelog_durable_commit_fate` × `wal_commit_plan` (Count+sync ⇒ AppendSync; Skip async ⇒ AppendApplyOk) — dual-unfold, `lake build`, m2 sobe no mesmo commit — status: done (`ComposeWriter.lean`: `writer_sync_chain_iff` / `count_with_sync_requires_append_sync` / `skip_async_is_append_apply_ok`; lake 2× verde; m2 39→40)
- [ ] **P0.2** RFC-0220 P0.4: `wal_commit_plan::AppendSyncFence` × `fence_admission_plan` — status: `todo`
- [ ] **P0.3** RFC-0220 P0.5: `manifest_publish_plan` × `changelog_durable_commit_fate` — status: `todo`
- [x] **P0.4** wiring: `db.rs` chama `leftover_page_advice` / `scan_readahead_window` no I/O que o kernel nomeia, ou recusa medida de que o I/O é outro (drop_page_cache ≠ leftover compaction) — status: done (`90bf4b64`; `leftover_drop_pages` → DontNeed; scan load → WillNeed; teste `scan_at_raw_calls_scan_readahead_window` 2× verde)

### P1 — recovery 4/11 → 11/11

- [ ] **P1.1** encadear os átomos de recovery ainda fora de `Compose*.lean` (7 restantes; lista viva = `sel4_gap.py` A4) — status: `todo`
- [ ] **P1.2** piso `recovery_atoms_chained` = 11 no mesmo commit do último elo — status: `todo`

### P2 — quatro terminais de pesquisa (nomeados)

- [ ] **P2.1** teorema topo `pedra_refines` (A1) ou recusa datada do que falta para um refinamento único — status: `todo`
- [ ] **P2.2** confinamento (A3) sobre o path rustc-linked de lookup/scan, ou recusa que reafirma `findings/2026-09-14-rfc0222-p23-confinement-recusa.md` com número novo — status: `todo`
- [ ] **P2.3** ∀ do `ConcurrentDb` (A6) dual-unfold group-commit × caller, ou recusa que reafirma o P2.4 do 0222 — status: `todo`
- [ ] **P2.4** TV de um objeto pinado (`write_admission_kernel.rs`, RFC-0172) sem promover `R-rustc` — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | writer-spine sync (fate × wal_commit) | done | ComposeWriter `writer_sync_chain_iff`; m2 39→40 | 2026-09-14 |
| P0.2 | p0 | writer-spine fence | todo | — | 2026-09-14 |
| P0.3 | p0 | writer-spine publish | todo | — | 2026-09-14 |
| P0.4 | p0 | wiring leftover_page / scan_readahead | done | `90bf4b64` | 2026-09-14 |
| P1.1 | p1 | recovery atoms 4→11 chained | todo | — | 2026-09-14 |
| P1.2 | p1 | floor recovery_atoms_chained=11 | todo | — | 2026-09-14 |
| P2.1 | p2 | pedra_refines (A1) | todo | — | 2026-09-14 |
| P2.2 | p2 | confinement (A3) | todo | — | 2026-09-14 |
| P2.3 | p2 | ConcurrentDb ∀ (A6) | todo | — | 2026-09-14 |
| P2.4 | p2 | TV one object (R-rustc stays never) | todo | — | 2026-09-14 |

## Acceptance Criteria

- **Tests:** `lake build` of each new `Compose*.lean`; `python3 scripts/sel4_gap.py --gate` GREEN; floors move only in the same commit as the proof.
- **Telemetry / Analytics:** none — verification RFC.
- **Documentation:** EXTRACT.md dated note per land; living table updated in the same commit.
- **Screenshots:** none — backend-only.

## Out of scope

- Declarar paridade seL4 ou "sem bugs" (RFC-0061).
- Fechar RFC-0222 P0.8 (push GitHub) — portão do usuário.
- Dump de `db.rs` / `concurrent.rs` no prover.
- Montanha cartoons; `∀π` / media-durable.
- Promover `R-rustc` para fora de `never_floor`.
