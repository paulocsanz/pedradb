# RFC-0213 P0.2 3/4 — átomo `catalog:mem_point_decides`

Data: 2026-09-12. Terceira promoção de P0.2 (cadência lookup ×4).
Escada viva: floor_atom 109→110, floor_extract 169→168,
cap_data_fate 15→14 (data_fate 15→14).

## O que foi pago

Teorema `mem_point_decides_fate_iff` em
`formal/aeneas/lean/Lookup.lean`: fate ∀ sobre o corpo extraído —
`(mem_point_decides has_point = ok v) ↔ v = has_point`.

## Semântica

O veredito de ponto na memtable é a própria flag de hit —
identidade, sem transformação.

## AS-IS recusado

O twin as-is devolve `false` incondicional — sempre reporta miss,
escondendo hits da memtable.

## Planta DST

`crates/pedradb-core/src/lookup_kernel.rs` (mod tests):
`mem_point_decides_on_live_hit_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py mem_point_decides`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 109→110, floor_extract 169→168, cap_data_fate 15→14.
Gate `check_depth_floor.py` GREEN: extract=168 (floor 168),
atom=110 (floor 110), data_fate=14<=14, residuals == live.

## Gates

`lake build Lookup` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Lookup.lean | grep -c "^+theorem"` = 1).
