# RFC-0214 P0.1 5/6 — átomo `catalog:wal_rotate`

Data: 2026-09-12. Quinta promoção de P0.1 (espinha de
durabilidade, wal_state ×6). Escada viva: floor_atom 126→127,
floor_extract 152→151 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `wal_rotate_fate_iff` em `formal/aeneas/lean/WalState.lean`:
fate ∀ sobre o corpo extraído — dois futuros ok exatos: log todo
durável e acked (`acked = synced = written`) → estado zerado
(`{acked := 0, synced := 0, written := 0}`); cauda não-durável →
recusa fail-closed, o estado volta inteiro.

## Semântica

Rotate (drop do log) só quando nada é perdido: `wal_state_of 0 0 0`
desdobra os mins a zero — o corpo extraído derruba o log apenas no
único ponto em que `acked ⊆ synced ⊆ written` colapsa num ponto.

## AS-IS recusado

O twin as-is derruba o log sempre — mesmo com cauda não-durável,
os bytes acked somem do log (`wal_rotate_as_is`).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`wal_inv_on_live_recording_is_not_ok` — verde (1 passed;
72 filtered). Kernel `wal_state_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py wal_rotate`: linha `atom` no `close_proofs.tsv`
(entrada `wal_rotate`); `atom_reason` datado no catálogo;
floor_atom 126→127, floor_extract 152→151; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 126→127, floor_extract 152→151. Gate
`check_depth_floor.py` GREEN: extract=151 (floor 151), atom=127
(floor 127), data_fate=0<=0, residuals == live.

## Gates

`lake build WalState` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WalState.lean | grep -c "^+theorem"` = 1).
