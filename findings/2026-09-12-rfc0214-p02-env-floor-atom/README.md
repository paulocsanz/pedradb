# RFC-0214 P0.2 4/6 — átomo `catalog:env_barrier_floor`

Data: 2026-09-12. Quarta promoção de P0.2 (costura Env,
env_crash ×6). Escada viva: floor_atom 131→132,
floor_extract 147→146 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `barrier_floor_fate_iff` em
`formal/aeneas/lean/EnvCrash.lean`: fate ∀ sobre o corpo extraído
— `(barrier_floor_holds m cut = ok v) ↔ v = true`.

## Semântica

A corolária do piso vale sempre: ou o corte é ilegal (nada a
perder), ou é legal e então `cut ≥ synced` — o piso da janela do
`crash_legal_fate_iff` (átomo 1/6 desta fatia, citado como perna).
Um crash legal nunca perde byte que a barreira honesta tornou
durável.

## AS-IS recusado

O twin as-is é o `crash_legal` sem piso (`crash_legal_as_is`) — a
legalidade floor-less admite cortes abaixo da barreira e a
corolária deixa de valer.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`env_crash_on_live_recording_is_not_ok` — verde (1 passed).
Kernel `env_crash_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py env_barrier_floor`: linha `atom` no
`close_proofs.tsv` (entrada `barrier_floor_holds`); `atom_reason`
datado no catálogo; floor_atom 131→132, floor_extract 147→146;
residuals/proof_depth re-carimbados no mesmo commit.

## Escada

floor_atom 131→132, floor_extract 147→146. Gate
`check_depth_floor.py` GREEN: extract=146 (floor 146), atom=132
(floor 132), data_fate=0<=0, residuals == live.

## Gates

`lake build EnvCrash` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …EnvCrash.lean | grep -c "^+theorem"` = 1).
