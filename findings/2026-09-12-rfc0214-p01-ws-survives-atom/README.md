# RFC-0214 P0.1 6/6 (FECHAMENTO) — átomo `catalog:wal_acked_survives`

Data: 2026-09-12. Sexta e última promoção de P0.1 (espinha de
durabilidade, wal_state ×6). Escada viva: floor_atom 127→128,
floor_extract 151→150 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `acked_survives_fate_iff` em
`formal/aeneas/lean/WalState.lean`: fate ∀ sobre o corpo extraído —
dois futuros ok exatos, decididos pela costura Env: corte legal
(`∃ cm b, CrashModel.of s.written s.synced = ok cm ∧ crash_legal cm
cut = ok b ∧ b = true → v = (cut ≥ acked)`); corte ilegal
(`b = false → v = true`).

## Semântica

O prefixo acked só é julgado PERDÍVEL por cortes que a costura Env
chama legais — a legalidade (piso da barreira, teto `written`) é a
do `env_crash_kernel`, não re-provada aqui: o P0.2 do RFC-0214
pinará o `crash_legal` no próprio EnvCrash.lean.

## AS-IS recusado

O twin as-is usa a legalidade floor-less — chama sobrevivável um
corte abaixo do piso da barreira e perde bytes acked
(`acked_survives_as_is_dente`, já no wrapper).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`wal_inv_on_live_recording_is_not_ok` — verde (1 passed;
72 filtered). Kernel `wal_state_kernel.rs` intocado em toda a
fatia (nenhum re-extract necessário em P0.1).

## Cirurgia de catálogo

`promote_atom.py wal_acked_survives`: linha `atom` no
`close_proofs.tsv` (entrada `acked_survives_every_legal_crash`);
`atom_reason` datado no catálogo; floor_atom 127→128,
floor_extract 151→150; residuals/proof_depth re-carimbados no
mesmo commit.

## Escada (FECHAMENTO P0.1)

floor_atom 122→128, floor_extract 156→150 — meta da fatia batida
exata. Gate `check_depth_floor.py` GREEN: extract=150 (floor 150),
atom=128 (floor 128), close=6, data_fate=0<=0, residuals == live.

## Gates

`lake build WalState` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …WalState.lean | grep -c "^+theorem"` = 1).
6 promoções × 1 teorema = 6 commits (d6179165, bb0f8e08,
eb5c65b4, 8f370758, 2a23f62a, este).
