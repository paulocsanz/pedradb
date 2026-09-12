# RFC-0213 P0.1 7/9 — átomo `catalog:torn_tail_needs_cut`

Data: 2026-09-12. Sétima promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 104→105, floor_extract 174→173,
cap_data_fate 20→19 (data_fate 20→19).

## O que foi pago

Teorema `torn_tail_needs_cut_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — `(torn_tail_needs_cut len last_good = ok v) ↔
v = decide (len > last_good)`.

## Semântica

Uma cauda tornada (torn tail) precisa ser cortada EXATAMENTE quando
o comprimento do log ultrapassa o último offset bom conhecido.

## AS-IS recusado

O twin as-is devolve `false` incondicional — nunca corta, deixando
bytes tornados no fim do log para a próxima recuperação.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`torn_tail_needs_cut_on_live_overhang_is_not_ok` — verde
(1 passed; 942 filtered).

## Cirurgia de catálogo

`promote_atom.py torn_tail_needs_cut`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 104→105, floor_extract 174→173, cap_data_fate 20→19.
Gate `check_depth_floor.py` GREEN: extract=173 (floor 173),
atom=105 (floor 105), data_fate=19<=19, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).
