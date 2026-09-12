# RFC-0214 P1.1 3/8 — átomo `catalog:cqe_leftover`

Data: 2026-09-12. Terceira promoção de P1.1 (cqe ×5 +
write_ack ×3). Escada viva: floor_atom 136→137,
floor_extract 142→141.

## O que foi pago

Teorema `cqe_act_fate_iff` em `formal/aeneas/lean/Cqe.lean`:
fate ∀ sobre o corpo extraído — `(cqe_act user_data want =
ok act) ↔ (user_data = want ∧ act = Take) ∨ (user_data ≠
want ∧ act = Discard)`.

## Semântica

O `harvest_ready_cqe` do ring (`crates/pedradb-io-uring/src/
ring.rs`) só toma o CQE cuja tag casa com a esperada; CQE
leftover de outra operação é descartado — a resposta de um
op nunca vem do SQE de outro.

## AS-IS recusado

O twin as-is `cqe_act_as_is` devolve `Take` para qualquer
`user_data` — adota CQE leftover como resposta própria.

## Planta DST

`crates/pedradb-io-uring/src/cqe_kernel.rs`:
`cqe_act_as_is_adopts_leftover` — verde (1 passed).
Kernel `cqe_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py cqe_leftover`: linha `atom` no
`close_proofs.tsv` (entrada `cqe_act`); `atom_reason`
datado no catálogo; floor_atom 136→137, floor_extract
142→141; residuals/proof_depth re-carimbados no mesmo
commit.

## Escada

floor_atom 136→137, floor_extract 142→141. Gate
`check_depth_floor.py` GREEN: extract=141 (floor 141),
atom=137 (floor 137), data_fate=0<=0, residuals == live.

## Gates

`lake build Cqe` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
