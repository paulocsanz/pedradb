# RFC-0215 — coroa de produto no degrau átomo (rodada 9)

Promoções átomo da rodada (régua: iff-∀ sobre o corpo extraído no
wrapper inscrito, `floor_atom +1 / floor_extract −1` no mesmo commit,
`check_depth_floor.py` GREEN no HEAD de cada promoção, planta DST
verde antes do commit, exatamente 1 teorema público por commit).

## P0.1 spec ×4 (`Properties.lean`)

| # | par | teorema | entry | commit | floor atom/extract | planta DST | quando |
|---|-----|---------|-------|--------|--------------------|------------|--------|
| 1/4 | `catalog:c1_quorum` | `c1_holds_fate_iff` | `c1_holds` | `1ae394dc` | 143/135 | `c1_as_is_does_not_imply_c1` ok | 2026-09-12 |
| 2/4 | `catalog:d1_durability` | `d1_holds_fate_iff` | `d1_holds` | `491ff82e` | 144/134 | `d1_as_is_does_not_imply_d1` ok | 2026-09-12 |
| 3/4 | `catalog:t1_atomicity` | `t1_holds_fate_iff` | `t1_holds` | (este commit) | 145/133 | `t1_as_is_does_not_imply_t1` ok | 2026-09-12 |

- **c1_holds (1/4)**: valor servido passa C1 exatamente quando a
  maioria de TODA config ativa replica — joint exige antiga E nova
  (`c1_pass`/`c1_fail` como forma-ramo citando `majority` só onde o
  corpo chama). Mutante AS-IS aceita a maioria antiga sozinha (buraco
  joint-election, RFC-0064); planta `c1_as_is_does_not_imply_c1`
  recusa (exit 0, 1 passed). Fate forall sobre o corpo extraído, sem
  loop; `r1_answer_ok`-style axiomas não usados — só os 3 axiomas
  padrão do Lean (`propext`, `Classical.choice`, `Quot.sound`).

- **d1_holds (2/4)**: D1 aceita `(acked, survives)` exatamente quando
  todo índice ackado fica dentro do prefixo sobrevivente (`d1_ok` como
  ∀-semântica first-order sobre `Slice.val`). Loop real provado por
  `loop.spec_decr_nat` (medida `len − j`, invariante prefixo limpo);
  mutante AS-IS (barreira só para synced) recusado por
  `d1_as_is_does_not_imply_d1` (exit 0, 1 passed). Axiomas: os 3
  padrão do Lean.

- **t1_holds (3/4)**: T1 aceita `(committed, aborted, staged_n,
  visible)` exatamente quando a tx é all-or-nothing — nunca ambos os
  flags, índices visíveis nomeiam writes staged, committed ⇒ todos
  visíveis, senão nenhum (`t1_ok` 4-conjuntiva). Dois loops reais
  (loop0 dos visíveis, loop1 do is_empty) via `spec_decr_nat`; mutante
  AS-IS (só integridade de bytes; tx abortada com efeito parcial
  passa) recusado por `t1_as_is_does_not_imply_t1` (exit 0, 1
  passed). Axiomas: os 3 padrão do Lean.
