# RFC-0213 P0.1 6/9 — átomo `catalog:torn_head_empty_log`

Data: 2026-09-12. Sexta promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 103→104, floor_extract 175→174,
cap_data_fate 21→20 (data_fate 21→20).

## O que foi pago

Teorema `torn_head_empty_log_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — `(torn_head_is_empty_log len tiny_max = ok v) ↔
v = decide (len < tiny_max)`.

## Semântica

Uma cabeça tornada (torn head) conta como log vazio EXATAMENTE
quando o comprimento do log está abaixo do limite tiny. Sem zona
cinzenta: o veredito é o próprio `decide` da comparação.

## AS-IS recusado

O twin as-is devolve `true` incondicional — chama toda cabeça de
log vazio, descartando WAL grande como se fosse nascente.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`torn_head_is_empty_log_on_live_large_wal_is_not_ok` — verde
(1 passed; 942 filtered).

## Cirurgia de catálogo

`promote_atom.py torn_head_empty_log` (entry
`torn_head_is_empty_log`): linha `atom` no `close_proofs.tsv`;
`atom_reason` datado no catálogo; `data_fate` removido;
residuals/proof_depth re-carimbados.

## Escada

floor_atom 103→104, floor_extract 175→174, cap_data_fate 21→20.
Gate `check_depth_floor.py` GREEN: extract=174 (floor 174),
atom=104 (floor 104), data_fate=20<=20, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).
