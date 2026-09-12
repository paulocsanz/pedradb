# RFC-0214 P0.2 2/6 — átomo `catalog:env_append`

Data: 2026-09-12. Segunda promoção de P0.2 (costura Env,
env_crash ×6). Escada viva: floor_atom 129→130,
floor_extract 149→148 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `append_fate_iff` em `formal/aeneas/lean/EnvCrash.lean`:
fate ∀ sobre o corpo extraído —
`(append m n = ok {m with written := w}) ↔ (m.written + n = ok w)`.

## Semântica

O append do Env (costura do byte-log) tem desfecho ok exatamente
quando a soma não estoura; a barreira (`synced`) não se move — o
comprimento lógico só cresce.

## AS-IS recusado

O par compartilha o mutante da costura (`crash_legal_as_is` —
legalidade sem piso); o corpo do append não tem mutant próprio.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`env_crash_on_live_recording_is_not_ok` — verde (1 passed).
Kernel `env_crash_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py env_append`: linha `atom` no `close_proofs.tsv`
(entrada `append`); `atom_reason` datado no catálogo;
floor_atom 129→130, floor_extract 149→148; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 129→130, floor_extract 149→148. Gate
`check_depth_floor.py` GREEN: extract=148 (floor 148), atom=130
(floor 130), data_fate=0<=0, residuals == live.

## Gates

`lake build EnvCrash` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …EnvCrash.lean | grep -c "^+theorem"` = 1).
