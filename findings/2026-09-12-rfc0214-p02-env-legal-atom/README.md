# RFC-0214 P0.2 1/6 — átomo `catalog:env_crash`

Data: 2026-09-12. Primeira promoção de P0.2 (costura Env,
env_crash ×6). Escada viva: floor_atom 128→129,
floor_extract 150→149 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `crash_legal_fate_iff` em `formal/aeneas/lean/EnvCrash.lean`:
fate ∀ sobre o corpo extraído —
`(crash_legal m cut = ok v) ↔ v = (synced ≤ cut && cut ≤ written)`.

## Semântica

Um corte é legal exatamente quando sobrevive entre o piso da
barreira e o teto escrito: caudas tornadas podem manter um prefixo,
bytes synced nunca somem, nenhum byte é inventado. É o mesmo termo
que `acked_survives_fate_iff` (P0.1) consome pela costura — agora
pinado no dono.

## AS-IS recusado

O twin as-is ignora o piso da barreira — um corte abaixo de
`synced` é chamado de legal e come bytes que a barreira prometeu
(dente `crash_legal_as_is_dente`, já no wrapper).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`env_crash_on_live_recording_is_not_ok` — verde (1 passed).
Kernel `env_crash_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py env_crash`: linha `atom` no `close_proofs.tsv`
(entrada `crash_legal`); `atom_reason` datado no catálogo;
floor_atom 128→129, floor_extract 150→149; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 128→129, floor_extract 150→149. Gate
`check_depth_floor.py` GREEN: extract=149 (floor 149), atom=129
(floor 129), data_fate=0<=0, residuals == live.

## Gates

`lake build EnvCrash` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …EnvCrash.lean | grep -c "^+theorem"` = 1).
