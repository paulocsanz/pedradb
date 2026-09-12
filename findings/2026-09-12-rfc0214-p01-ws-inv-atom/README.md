# RFC-0214 P0.1 1/6 — átomo `catalog:wal_state`

Data: 2026-09-12. Primeira promoção de P0.1 (espinha de
durabilidade, wal_state ×6). Escada viva: floor_atom 122→123,
floor_extract 156→155 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `inv_wal_fate_iff` em `formal/aeneas/lean/WalState.lean`:
fate ∀ sobre o corpo extraído —
`(inv_wal s = ok v) ↔ v = ((acked ⊆ synced) && (synced ⊆ written))`.

## Semântica

O invariante Inv-WAL decide exatamente a conjunção Booleana das
duas contenções (o teto de um crash legal é `written`, logo
`synced ≤ written` é "synced está no prefixo-recuperável").
Perna citada: `wal_inv_closed` (RFC-0191 P2.1) — nada re-provado.

## AS-IS recusado

O twin as-is esquece o braço `acked ⊆ synced` — "acked" sem
barreira é admitido (dente `inv_wal_as_is_dente`, já no wrapper).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`wal_inv_on_live_recording_is_not_ok` — verde (1 passed;
72 filtered). Kernel `wal_state_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py wal_state`: linha `atom` no
`close_proofs.tsv` (entrada `inv_wal`); `atom_reason` datado no
catálogo; floor_atom 122→123, floor_extract 156→155;
residuals/proof_depth re-carimbados no mesmo commit.

## Escada

floor_atom 122→123, floor_extract 156→155. Gate
`check_depth_floor.py` GREEN: extract=155 (floor 155), atom=123
(floor 123), data_fate=0<=0, residuals == live.

## Gates

`lake build WalState` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WalState.lean | grep -c "^+theorem"` = 1).
