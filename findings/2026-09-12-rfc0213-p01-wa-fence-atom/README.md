# RFC-0213 P0.1 4/9 — átomo `catalog:fence_on_sync_fail`

Data: 2026-09-12. Quarta promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 101→102, floor_extract 177→176,
cap_data_fate 23→22 (data_fate 23→22).

## O que foi pago

Teorema `fence_on_sync_fail_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — `(fence_on_sync_fail sync_required sync_failed = ok v) ↔
v = (sync_required && sync_failed)`.

## Semântica

A cerca de recuperação dispara EXATAMENTE quando um fdatasync era
exigido para o registro corrente e esse sync falhou — a conjunção
Booleana dos dois fatos, sem terceiro estado.

## AS-IS recusado

O twin as-is devolve `false` incondicional — nunca cerca, mesmo com
sync exigido + falhado (deixaria seguir como se durável).

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`fence_on_sync_fail_on_live_required_fail_is_not_ok` — verde
(1 passed; 942 filtered).

## Cirurgia de catálogo

`promote_atom.py fence_on_sync_fail`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 101→102, floor_extract 177→176, cap_data_fate 23→22.
Gate `check_depth_floor.py` GREEN: extract=176 (floor 176),
atom=102 (floor 102), data_fate=22<=22, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).
