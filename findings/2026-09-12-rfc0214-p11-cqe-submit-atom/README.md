# RFC-0214 P1.1 4/8 — átomo `catalog:cqe_submit`

Data: 2026-09-12. Quarta promoção de P1.1 (cqe ×5 +
write_ack ×3). Escada viva: floor_atom 137→138,
floor_extract 141→140.

## O que foi pago

Teorema `submit_complete_act_fate_iff` em
`formal/aeneas/lean/Cqe.lean`: fate ∀ sobre o corpo
extraído — `(submit_complete_act submit_ok harvested = ok
act) ↔ (harvested ∧ act = UseHarvested) ∨ (¬harvested ∧
act = WaitMore)`. `submit_ok` não influencia a decisão.

## Semântica

Depois de `submit_sqe` (ring.rs), o CQE colhido é usado
mesmo se o submit voltou Err (EINTR): o SQE já está no
anel. Sem CQE, espera (`WaitMore`) — voltar Err no submit
solta o buffer do caller enquanto o kernel ainda pode fazer
DMA (F208: UAF potencial).

## AS-IS recusado

O twin as-is `submit_complete_act_as_is` decide só pelo
`submit_ok`: Err no submit com CQE pendente vira
`ReturnSubmitErr` — o buffer cai com DMA em curso.

## Planta DST

`crates/pedradb-io-uring/src/cqe_kernel.rs`:
`harvest_on_submit_err_uses_cqe` — verde (1 passed).
Kernel `cqe_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py cqe_submit`: linha `atom` no
`close_proofs.tsv` (entrada `submit_complete_act`);
`atom_reason` datado no catálogo; floor_atom 137→138,
floor_extract 141→140; residuals/proof_depth re-carimbados
no mesmo commit.

## Escada

floor_atom 137→138, floor_extract 141→140. Gate
`check_depth_floor.py` GREEN: extract=140 (floor 140),
atom=138 (floor 138), data_fate=0<=0, residuals == live.

## Gates

`lake build Cqe` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
