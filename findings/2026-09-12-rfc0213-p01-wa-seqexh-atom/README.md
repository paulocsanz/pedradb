# RFC-0213 P0.1 3/9 — átomo `catalog:seq_exhausted`

Data: 2026-09-12. Terceira promoção de P0.1 (cadência write_admission
×9). Escada viva: floor_atom 100→101, floor_extract 178→177,
cap_data_fate 24→23 (data_fate 24→23).

## O que foi pago

Teorema `seq_exhausted_fate_iff` em
`formal/aeneas/lean/WriteAdmission.lean`: fate ∀ sobre o corpo
extraído — `(seq_exhausted seq max = ok v) ↔ v = decide (seq > max)`.

## Semântica

O contador de sequência está esgotado EXATAMENTE quando já queimou
além do teto (`seq > max`). Sem intervalo cinzento: o veredito é o
próprio `decide` da comparação.

## AS-IS recusado

O twin as-is nunca reporta esgotamento (comentário do kernel:
"wrap / burn past the ceiling") — aceitaria reutilizar sequências
exaustas.

## Planta DST

`crates/pedradb-core/src/write_admission_kernel.rs` (mod tests):
`seq_exhausted_on_live_ceiling_is_not_ok` — verde (1 passed;
942 filtered).

## Cirurgia de catálogo

`promote_atom.py seq_exhausted`: linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo; `data_fate`
removido; residuals/proof_depth re-carimbados.

## Escada

floor_atom 100→101, floor_extract 178→177, cap_data_fate 24→23.
Gate `check_depth_floor.py` GREEN: extract=177 (floor 177),
atom=101 (floor 101), data_fate=23<=23, residuals == live.

## Gates

`lake build WriteAdmission` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WriteAdmission.lean | grep -c "^+theorem"` = 1).

## Nota de prova

O corpo extraído emite `decide (seq > max)` (coerção Prop→Bool); o
enunciado usa `v = decide (seq > max)` para casar sem coerção
implícita. `simp` reduz a `a = v ↔ v = a`, fechado por `eq_comm`.
