# RFC-0214 P0.1 4/6 — átomo `catalog:wal_ack`

Data: 2026-09-12. Quarta promoção de P0.1 (espinha de
durabilidade, wal_state ×6). Escada viva: floor_atom 125→126,
floor_extract 153→152 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `wal_ack_fate_iff` em `formal/aeneas/lean/WalState.lean`:
fate ∀ sobre o corpo extraído — dois futuros ok exatos: dentro da
barreira `∃ a, saturating_add(acked,n) ≤ synced ∧ acked+n = ok a ∧
s' = {s with acked := a}`; fora dela `s' = s` (recusa fail-closed).

## Semântica

O Ok do cliente nunca avança `acked` além do que a barreira tornou
durável — a contenção checada no corpo é a do valor SATURADO contra
`synced` (o caso de borda `synced = max` com estouro do add cai na
recusa: o add falha, o passo nem é ok). Fail-closed é o produto
(fdatasync antes de Ok).

## AS-IS recusado

O twin as-is avança `acked` incondicionalmente — `acked` passa da
barreira (`wal_ack_as_is`).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`wal_inv_on_live_recording_is_not_ok` — verde (1 passed;
72 filtered). Kernel `wal_state_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py wal_ack`: linha `atom` no `close_proofs.tsv`
(entrada `wal_ack`); `atom_reason` datado no catálogo;
floor_atom 125→126, floor_extract 153→152; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 125→126, floor_extract 153→152. Gate
`check_depth_floor.py` GREEN: extract=152 (floor 152), atom=126
(floor 126), data_fate=0<=0, residuals == live.

## Gates

`lake build WalState` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WalState.lean | grep -c "^+theorem"` = 1).
