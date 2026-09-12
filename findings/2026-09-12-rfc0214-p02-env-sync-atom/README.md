# RFC-0214 P0.2 3/6 — átomo `catalog:env_sync`

Data: 2026-09-12. Terceira promoção de P0.2 (costura Env,
env_crash ×6). Escada viva: floor_atom 130→131,
floor_extract 148→147 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `sync_fate_iff` em `formal/aeneas/lean/EnvCrash.lean`:
fate ∀ sobre o corpo extraído — dois futuros ok exatos: Honest →
`{m with synced := m.written}`; Lying → `m` (Ok devolvido, nada
promovido).

## Semântica

A honestidade do Env decide: o sync honesto promove a barreira ao
comprimento todo; o sync mentiroso (RFC-0078 — `SyncPolicy::Lying`)
devolve Ok sem mover a barreira. Formas fechadas provadas inline
(`have` sobre o corpo extraído, desdobrando `fsync_promotes_pending`).

## AS-IS recusado

O twin as-is promove a barreira sempre (`sync_lying_promotes_as_is`)
— um Ok mentiroso é tratado como barreira feita.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`env_crash_on_live_recording_is_not_ok` — verde (1 passed).
Kernel `env_crash_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py env_sync`: linha `atom` no `close_proofs.tsv`
(entrada `sync`); `atom_reason` datado no catálogo;
floor_atom 130→131, floor_extract 148→147; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 130→131, floor_extract 148→147. Gate
`check_depth_floor.py` GREEN: extract=147 (floor 147), atom=131
(floor 131), data_fate=0<=0, residuals == live.

## Gates

`lake build EnvCrash` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …EnvCrash.lean | grep -c "^+theorem"` = 1).
