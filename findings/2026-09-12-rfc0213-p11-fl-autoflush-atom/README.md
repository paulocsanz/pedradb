# RFC-0213 P1.1 2/5 — átomo `catalog:auto_flush_due`

Data: 2026-09-12. Segunda promoção de P1.1 (cadência flush ×3 +
cf ×2). Escada viva: floor_atom 112→113, floor_extract 166→165,
cap_data_fate 12→11 (data_fate 12→11).

## O que foi pago

Teorema `auto_flush_due_fate_iff` em
`formal/aeneas/lean/Flush.lean`: fate ∀ sobre o corpo extraído —
o resultado é `decide (mem_bytes >= limit)` exatamente no ramo
armado, e `false` exatamente no ramo desarmado.

## Semântica

O auto-flush está devido EXATAMENTE quando o eixo está armado e os
bytes da memtable chegaram ao limite.

## AS-IS recusado

O twin as-is devolve `false` incondicional — nunca auto-flusha,
deixando a memtable crescer além do limite.

## Planta DST

`crates/pedradb-core/src/flush_kernel.rs` (mod tests):
`auto_flush_due_on_live_over_limit_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py auto_flush_due`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 112→113, floor_extract 166→165, cap_data_fate 12→11.
Gate `check_depth_floor.py` GREEN: extract=165 (floor 165),
atom=113 (floor 113), data_fate=11<=11, residuals == live.

## Gates

`lake build Flush` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Flush.lean | grep -c "^+theorem"` = 1).
