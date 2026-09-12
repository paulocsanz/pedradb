# RFC-0214 P0.1 3/6 — átomo `catalog:wal_sync`

Data: 2026-09-12. Terceira promoção de P0.1 (espinha de
durabilidade, wal_state ×6). Escada viva: floor_atom 124→125,
floor_extract 154→153 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `wal_sync_fate_iff` em `formal/aeneas/lean/WalState.lean`:
fate ∀ sobre o corpo extraído — o desfecho ok tem exatamente dois
futuros: Honest → `{s with synced := s.written, written := s.written}`;
Lying → `{s with synced := min(synced, written), written := s.written}`.

## Semântica

A barreira honesta promove `synced` até `written`; a mentirosa
deixa as watermarks recortadas pelo min de `CrashModel.of` (nunca
amplia — RFC-0078). Pernas citadas: `wal_sync_honest_closed` e
`wal_sync_lying_closed` (RFC-0198 P1.2) — nada re-provado.

## AS-IS recusado

O twin as-is promove `synced` a `written` independente da
honestidade — um Ok mentiroso é tratado como barreira feita.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`wal_inv_on_live_recording_is_not_ok` — verde (1 passed;
72 filtered). Kernel `wal_state_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py wal_sync`: linha `atom` no `close_proofs.tsv`
(entrada `wal_sync`); `atom_reason` datado no catálogo;
floor_atom 124→125, floor_extract 154→153; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 124→125, floor_extract 154→153. Gate
`check_depth_floor.py` GREEN: extract=153 (floor 153), atom=125
(floor 125), data_fate=0<=0, residuals == live.

## Gates

`lake build WalState` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WalState.lean | grep -c "^+theorem"` = 1).
