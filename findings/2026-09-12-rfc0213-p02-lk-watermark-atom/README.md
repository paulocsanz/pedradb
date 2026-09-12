# RFC-0213 P0.2 2/4 — átomo `catalog:snap_below_watermark`

Data: 2026-09-12. Segunda promoção de P0.2 (cadência lookup ×4).
Escada viva: floor_atom 108→109, floor_extract 170→169,
cap_data_fate 16→15 (data_fate 16→15).

## O que foi pago

Teorema `snap_below_watermark_fate_iff` em
`formal/aeneas/lean/Lookup.lean`: fate ∀ sobre o corpo extraído —
`(snap_below_watermark seq earliest = ok v) ↔
v = decide (seq < earliest)`.

## Semântica

Um snapshot está abaixo da marca d'água EXATAMENTE quando sua
sequência é mais velha que a mínima visível — comparação decidida.

## AS-IS recusado

O twin as-is devolve `false` incondicional — nunca considera o
snapshot abaixo da marca d'água, expondo leituras velhas demais.

## Planta DST

`crates/pedradb-core/src/lookup_kernel.rs` (mod tests):
`snap_below_watermark_on_live_below_is_not_ok` — verde
(1 passed; 942 filtered).

## Cirurgia de catálogo

`promote_atom.py snap_below_watermark`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 108→109, floor_extract 170→169, cap_data_fate 16→15.
Gate `check_depth_floor.py` GREEN: extract=169 (floor 169),
atom=109 (floor 109), data_fate=15<=15, residuals == live.

## Gates

`lake build Lookup` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Lookup.lean | grep -c "^+theorem"` = 1).
