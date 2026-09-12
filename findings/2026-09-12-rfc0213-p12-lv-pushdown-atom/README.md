# RFC-0213 P1.2 2/6 — atom `catalog:leveling_pushdown` (`pick_pushdown_fate_iff`, Leveling.lean)

Data: 2026-09-12. Par `leveling_pushdown` promovido de `data_fate`
para `atom`, teorema `pick_pushdown_fate_iff` em
`formal/aeneas/lean/Leveling.lean` sobre o extrato Aeneas de
`leveling.rs` (`pick_pushdown`, entry do catálogo).

## O que o teorema diz (fate forall sobre o corpo extraído)

`pick_pushdown src dst = ok v` ↔ exatamente a rota extraída:

- `is_empty src = ok b`, `b = true` ⇒ `v = none`;
- senão `is_disjoint dst = ok d`:
  - `d = true`: `index src 0` ok (fonte mais antiga),
    `pick_l0_slice_loop dst source.lo source.hi …` ok, e
    `v = some (source.idx, slice)`;
  - `¬(d = true)`: `v = none` — o gate de disjunção RECUSA o job
    sobre nível empilhado (a cascata deslimitada que o gate existe
    para recusar).

## Por que data_fate não é mais necessário

O corpo inteiro é coberto pelo iff ∃-cadeia; as folhas são o loop de
slice (átomo `@[rust_loop]`) e `is_disjoint`. O as-is
(`pick_pushdown_as_is_blind`) pula o gate e é refutado ao vivo:
`leveling::tests::pick_pushdown_on_live_pushdown_gate_is_not_ok`
(1 passed, `cargo test --lib -p pedradb-core`).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:leveling_pushdown`
  (`pick_pushdown_fate_iff`, entry `pick_pushdown`).
- `proof_depth.tsv`: floor_atom 117→118, floor_extract 161→160,
  cap_data_fate 7→6.
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN
  (extract=160, close=6, atom=118, count=7, data_fate=6).
