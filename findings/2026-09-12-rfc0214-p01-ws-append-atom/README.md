# RFC-0214 P0.1 2/6 — átomo `catalog:wal_append`

Data: 2026-09-12. Segunda promoção de P0.1 (espinha de
durabilidade, wal_state ×6). Escada viva: floor_atom 123→124,
floor_extract 155→154 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `wal_append_fate_iff` em
`formal/aeneas/lean/WalState.lean`: fate ∀ sobre o corpo extraído —
`(wal_append s n = ok {s with written := w}) ↔ (s.written + n = ok w)`.

## Semântica

O append tem desfecho ok exatamente quando a soma dos bytes não
estoura; e nesse caso o único futuro possível é `{s with written :=
w}` — a barreira (`synced`) e o prefixo `acked` não se movem, o log
só cresce. A rota ← CITA o fechado ∀ `wal_append_closed`
(RFC-0191 P2.1).

## AS-IS recusado

O twin as-is acka os mesmos bytes junto com o write — antes de
qualquer barreira (`acked` cresce com `written`).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`wal_inv_on_live_recording_is_not_ok` — verde (1 passed;
72 filtered). Kernel `wal_state_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py wal_append`: linha `atom` no
`close_proofs.tsv` (entrada `wal_append`); `atom_reason` datado no
catálogo; floor_atom 123→124, floor_extract 155→154;
residuals/proof_depth re-carimbados no mesmo commit.

## Escada

floor_atom 123→124, floor_extract 155→154. Gate
`check_depth_floor.py` GREEN: extract=154 (floor 154), atom=124
(floor 124), data_fate=0<=0, residuals == live.

## Gates

`lake build WalState` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WalState.lean | grep -c "^+theorem"` = 1).
