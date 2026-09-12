# RFC-0214 P1.1 2/8 — átomo `catalog:cqe_tags`

Data: 2026-09-12. Segunda promoção de P1.1 (cqe ×5 + write_ack
×3). Escada viva: floor_atom 135→136, floor_extract 143→142.

## O que foi pago

Teorema `next_user_data_fate_iff` em
`formal/aeneas/lean/Cqe.lean`: fate ∀ sobre o corpo extraído
— de `c ≠ 0` devolve `(c, c+1)` (wrapping); de `c = 0` pula o
zero reservado e devolve `(1, 2)`. O loop real do kernel
(`next_user_data_loop`) é provado com `loop.spec_decr_nat`
(precedente `Isolated.lean`), sem `simp [loop]`.

## Semântica

Cada SQE recebe uma tag única: o contador avança e nunca
reusa `user_data` — o `harvest_ready_cqe` do ring
(`crates/pedradb-io-uring/src/ring.rs`) casa CQE com o SQE
certo pela tag. O zero é reservado (pulo no primeiro passo).

## AS-IS recusado

O twin as-is `next_user_data_as_is` devolve tag constante por
opcode — CQE leftover de outra operação do mesmo opcode é
adotado como resposta própria.

## Planta DST

`crates/pedradb-io-uring/src/cqe_kernel.rs`:
`unique_tags_discard_leftover_same_opcode` — verde
(1 passed). Kernel `cqe_kernel.rs` intocado nesta promoção
(nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py cqe_tags`: linha `atom` no
`close_proofs.tsv` (entrada `next_user_data`);
`atom_reason` datado no catálogo; floor_atom 135→136,
floor_extract 143→142; residuals/proof_depth re-carimbados
no mesmo commit.

## Escada

floor_atom 135→136, floor_extract 143→142. Gate
`check_depth_floor.py` GREEN: extract=142 (floor 142),
atom=136 (floor 136), data_fate=0<=0, residuals == live.

## Gates

`lake build Cqe` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
