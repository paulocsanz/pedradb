# RFC-0213 P0.2 4/4 — átomo `catalog:prefer_newer_seq` (FECHAMENTO P0.2)

Data: 2026-09-12. Quarta e última promoção de P0.2 (cadência lookup
×4). Escada viva: floor_atom 110→111, floor_extract 168→167,
cap_data_fate 14→13 (data_fate 14→13).

## O que foi pago

Teorema `prefer_newer_seq_fate_iff` em
`formal/aeneas/lean/Lookup.lean`: fate ∀ sobre o corpo extraído —
o resultado é `decide (new_seq > best_seq)` exatamente no ramo com
incumbente, e `true` exatamente no ramo sem incumbente.

## Semântica

Um candidato vence o desempate EXATAMENTE quando não há melhor
incumbente, ou quando há e a sequência do candidato é mais nova.

## AS-IS recusado

O twin as-is devolve `true` incondicional — prefere tudo, inclusive
mais velho (escolheria versão antiga sobre a nova).

## Planta DST

`crates/pedradb-core/src/lookup_kernel.rs` (mod tests):
`prefer_newer_seq_on_live_older_first_is_not_ok` — verde
(1 passed; 942 filtered).

## Cirurgia de catálogo

`promote_atom.py prefer_newer_seq`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 110→111, floor_extract 168→167, cap_data_fate 14→13.
Gate `check_depth_floor.py` GREEN: extract=167 (floor 167),
atom=111 (floor 111), data_fate=13<=13, residuals == live.

## Gates

`lake build Lookup` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Lookup.lean | grep -c "^+theorem"` = 1).

## Totais do P0.2 (fechamento)

- 4/4 átomos: snap_empty, snap_below_watermark, mem_point_decides,
  prefer_newer_seq.
- cap_data_fate 17→13, floor_atom 107→111, floor_extract 171→167.
- 4 commits, 1 teorema `_fate_iff` por commit; todas as plantas
  DST do lookup_kernel verdes no fechamento.
