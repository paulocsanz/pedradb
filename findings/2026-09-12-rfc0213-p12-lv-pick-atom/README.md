# RFC-0213 P1.2 1/6 — atom `catalog:leveling_pick` (`pick_l0_to_l1_fate_iff`, Leveling.lean)

Data: 2026-09-12. Par `leveling_pick` promovido de `data_fate` para
`atom`, teorema `pick_l0_to_l1_fate_iff` em
`formal/aeneas/lean/Leveling.lean` sobre o extrato Aeneas de
`leveling.rs` (`pick_l0_to_l1`, entry do catálogo).

## O que o teorema diz (fate forall sobre o corpo extraído)

`pick_l0_to_l1 l0 l1 max_l0 = ok v` ↔ exatamente a rota extraída:

- `is_empty l0 = ok b`, `b = true` ⇒ `v = none`;
- `¬(b = true)` ∧ `max_l0 = 0#usize` ⇒ `v = none` (cap 0 recusa job);
- senão (cap vivo): `index l0 0` ok, clones do hull do primeiro
  arquivo ok, `push` do idx ok, `pick_l0_sel_loop l0
  (if len l0 < max_l0 then len l0 else max_l0) …` ok (o cap
  `min(len, cap)` limita a seleção), `pick_l0_slice_loop l1 …` ok,
  e `v = some (sel, slice)`.

Os dois loops (`@[rust_loop]`) ficam como átomos de loop — a seleção
limitada pelo cap e o corte por overlap são a rota, não sorte do par.

## Por que data_fate não é mais necessário

O corpo inteiro é coberto pelo iff ∃-cadeia; as folhas são os loops
(átomos registrados como extração) e as operações de Vec/clone. O
as-is (`pick_l0_to_l1_as_is_whole_level`) reabsorve o L1 inteiro sem
overlap e é refutado ao vivo:
`leveling::tests::pick_l0_to_l1_on_live_slice_is_not_ok`
(1 passed, `cargo test --lib -p pedradb-core`).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:leveling_pick`
  (`pick_l0_to_l1_fate_iff`, entry `pick_l0_to_l1`).
- `proof_depth.tsv`: floor_atom 116→117, floor_extract 162→161,
  cap_data_fate 8→7.
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN
  (extract=161, close=6, atom=117, count=7, data_fate=7).

## Lembrete de grafia

Neste wrapper o header abre `Aeneas.Std` sem `Aeneas` — os tipos
levam `Usize` (não `Std.Usize`); ifs de igualdade inteira do extrato
(`max_l0 = 0#usize`) são Prop-Decidable diretos (moldes de
`level_target_bytes`), ifs de Bool (`b`) usam `b = true`.
