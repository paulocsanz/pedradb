# RFC-0215 — coroa de produto no degrau átomo (rodada 9)

Promoções átomo da rodada (régua: iff-∀ sobre o corpo extraído no
wrapper inscrito, `floor_atom +1 / floor_extract −1` no mesmo commit,
`check_depth_floor.py` GREEN no HEAD de cada promoção, planta DST
verde antes do commit, exatamente 1 teorema público por commit).

## P0.1 spec ×4 (`Properties.lean`)

| # | par | teorema | entry | commit | floor atom/extract | planta DST | quando |
|---|-----|---------|-------|--------|--------------------|------------|--------|
| 1/4 | `catalog:c1_quorum` | `c1_holds_fate_iff` | `c1_holds` | (ver `git log`) | 143/135 | `c1_as_is_does_not_imply_c1` ok | 2026-09-12 |

- **c1_holds (1/4)**: valor servido passa C1 exatamente quando a
  maioria de TODA config ativa replica — joint exige antiga E nova
  (`c1_pass`/`c1_fail` como forma-ramo citando `majority` só onde o
  corpo chama). Mutante AS-IS aceita a maioria antiga sozinha (buraco
  joint-election, RFC-0064); planta `c1_as_is_does_not_imply_c1`
  recusa (exit 0, 1 passed). Fate forall sobre o corpo extraído, sem
  loop; `r1_answer_ok`-style axiomas não usados — só os 3 axiomas
  padrão do Lean (`propext`, `Classical.choice`, `Quot.sound`).
