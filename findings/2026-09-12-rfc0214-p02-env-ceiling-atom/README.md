# RFC-0214 P0.2 5/6 — átomo `catalog:env_no_invented`

Data: 2026-09-12. Quinta promoção de P0.2 (costura Env,
env_crash ×6). Escada viva: floor_atom 132→133,
floor_extract 146→145 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `no_invented_bytes_fate_iff` em
`formal/aeneas/lean/EnvCrash.lean`: fate ∀ sobre o corpo extraído
— `(no_invented_bytes_holds m cut = ok v) ↔ v = true`.

## Semântica

A corolária do teto vale sempre: ou o corte é ilegal, ou é legal e
então `cut ≤ written` — o teto da janela do `crash_legal_fate_iff`
(átomo 1/6 desta fatia, citado como perna). A recuperação nunca
observa um byte que o writer não escreveu.

## AS-IS recusado

O twin as-is é o `crash_legal` sem piso (`crash_legal_as_is`).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`env_crash_on_live_recording_is_not_ok` — verde (1 passed).
Kernel `env_crash_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py env_no_invented`: linha `atom` no
`close_proofs.tsv` (entrada `no_invented_bytes_holds`);
`atom_reason` datado no catálogo; floor_atom 132→133,
floor_extract 146→145; residuals/proof_depth re-carimbados no
mesmo commit.

## Escada

floor_atom 132→133, floor_extract 146→145. Gate
`check_depth_floor.py` GREEN: extract=145 (floor 145), atom=133
(floor 133), data_fate=0<=0, residuals == live.

## Gates

`lake build EnvCrash` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …EnvCrash.lean | grep -c "^+theorem"` = 1).
