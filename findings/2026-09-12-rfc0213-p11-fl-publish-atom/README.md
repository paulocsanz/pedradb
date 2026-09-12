# RFC-0213 P1.1 1/5 — átomo `catalog:flush_publish`

Data: 2026-09-12. Primeira promoção de P1.1 (cadência flush ×3 +
cf ×2). Escada viva: floor_atom 111→112, floor_extract 167→166,
cap_data_fate 13→12 (data_fate 13→12).

## O que foi pago

Teorema `flush_publish_fate_iff` em `formal/aeneas/lean/Flush.lean`:
fate ∀ sobre o corpo extraído — `(may_publish_manifest sst_durable
= ok v) ↔ v = sst_durable`.

## Semântica

O manifesto pode ser publicado EXATAMENTE quando o SST acabou de
ficar durável — o veredito é a própria flag de durabilidade.

## AS-IS recusado

O twin as-is devolve `true` incondicional — publica SST sem sync,
expondo no manifesto um arquivo que uma queda pode apagar.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`may_publish_manifest_on_live_unsynced_sst_is_not_ok` — verde
(1 passed; 72 filtered; crate pedradb-sim).

## Cirurgia de catálogo

`promote_atom.py flush_publish` (entry `may_publish_manifest`):
linha `atom` no `close_proofs.tsv`; `atom_reason` datado no
catálogo; `data_fate` removido; residuals/proof_depth
re-carimbados.

## Escada

floor_atom 111→112, floor_extract 167→166, cap_data_fate 13→12.
Gate `check_depth_floor.py` GREEN: extract=166 (floor 166),
atom=112 (floor 112), data_fate=12<=12, residuals == live.

## Gates

`lake build Flush` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …Flush.lean | grep -c "^+theorem"` = 1).
