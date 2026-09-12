# RFC-0213 P0.1 8/9 — átomo `catalog:seq_after_feed`

Data: 2026-09-12. Oitava promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 105→106, floor_extract 173→172,
cap_data_fate 19→18 (data_fate 19→18).

## O que foi pago

Teorema `seq_after_feed_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — `(seq_after_feed seq feed_max = ok v) ↔
v = decide (seq > feed_max)`.

## Semântica

Uma sequência está "depois do feed" EXATAMENTE quando ultrapassa o
teto do feed — comparação decidida, sem terceiro estado.

## AS-IS recusado

O twin as-is devolve `false` incondicional — nunca vê além do feed,
ignorando sequências mais novas que chegaram.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`seq_after_feed_on_live_newer_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py seq_after_feed`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 105→106, floor_extract 173→172, cap_data_fate 19→18.
Gate `check_depth_floor.py` GREEN: extract=172 (floor 172),
atom=106 (floor 106), data_fate=18<=18, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).
