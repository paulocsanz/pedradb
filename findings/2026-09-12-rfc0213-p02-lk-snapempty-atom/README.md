# RFC-0213 P0.2 1/4 — átomo `catalog:snap_empty`

Data: 2026-09-12. Primeira promoção de P0.2 (cadência lookup ×4).
Escada viva: floor_atom 107→108, floor_extract 171→170,
cap_data_fate 17→16 (data_fate 17→16).

## O que foi pago

Teorema `snap_empty_fate_iff` em `formal/aeneas/lean/Lookup.lean`:
fate ∀ sobre o corpo extraído — `(snap_is_empty seq = ok v) ↔
v = decide (seq = 0#u64)`.

## Semântica

Um snapshot está vazio EXATAMENTE quando sua sequência é zero — a
comparação decidida, sem terceiro estado.

## AS-IS recusado

O twin as-is devolve `false` incondicional — nunca vê snapshot
vazio, tratando snapshot nascente como povoado.

## Planta DST

`crates/pedradb-core/src/lookup_kernel.rs` (mod tests):
`snap_is_empty_on_live_zero_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py snap_empty` (entry `snap_is_empty`): linha `atom`
no `close_proofs.tsv`; `atom_reason` datado no catálogo;
`data_fate` removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 107→108, floor_extract 171→170, cap_data_fate 17→16.
Gate `check_depth_floor.py` GREEN: extract=170 (floor 170),
atom=108 (floor 108), data_fate=16<=16, residuals == live.

## Gates

`lake build Lookup` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Lookup.lean | grep -c "^+theorem"` = 1).

## Nota de prova

Mesma coerção Prop→Bool dos átomos de comparação: o corpo extraído
emite `decide (seq = 0#u64)`; `simp` + `eq_comm` fecha.
