# RFC-0213 P0.1 2/9 — átomo `catalog:write_admit`

Data: 2026-09-12. Segunda promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 99→100, floor_extract 179→178,
cap_data_fate 25→24 (data_fate 25→24).

## O que foi pago

Teorema `write_admit_fate_iff` em `formal/aeneas/lean/WriteAdmission.lean`:
fate ∀ sobre o corpo extraído — `(write_admit mem_bytes mem_armed
mem_limit l0 l0_armed l0_limit = ok r) ↔ (StallMem ∧ mem armado estourado)
∨ (StallL0 ∧ L0 armado estourado ∧ mem não estourado) ∨ (Ok ∧ nenhum
eixo armado estourado)`.

## Semântica

Veredito do hard-admit: StallMem exatamente quando o eixo mem ESTÁ
ARMADO e `mem_bytes >= mem_limit`; StallL0 exatamente quando o eixo L0
está armado e `l0 >= l0_limit` com o eixo mem já passado; Ok exatamente
quando nenhum eixo armado estoura seu limite.

## AS-IS recusado

O twin as-is sempre admitia (Ok incondicional) — mentira de
admissibilidade sob pressão de mem/L0.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`write_admit_on_live_mem_over_is_not_ok` — verde (1 passed;
941 filtered).

## Cirurgia de catálogo

`promote_atom.py write_admit`: TSV `close_proofs.tsv` linha
`atom catalog:write_admit write_admit_fate_iff …`; catálogo ganha
`atom_reason` datado e perde o `data_fate`; residuals/proof_depth
re-carimbados.

## Escada

floor_atom 99→100, floor_extract 179→178, cap_data_fate 25→24.
Gate `check_depth_floor.py` GREEN: extract=178 (floor 178),
atom=100 (floor 100), data_fate=24<=24, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).
