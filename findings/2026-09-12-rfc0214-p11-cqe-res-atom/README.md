# RFC-0214 P1.1 1/8 — átomo `catalog:cqe_res`

Data: 2026-09-12. Primeira promoção de P1.1 (cqe ×5 +
write_ack ×3). Escada viva: floor_atom 134→135,
floor_extract 144→143.

## O que foi pago

Teorema `cqe_res_ok_fate_iff` em
`formal/aeneas/lean/Cqe.lean`: fate ∀ sobre o corpo extraído
— `(cqe_res_ok res = ok v) ↔ v = (res >= 0#i32)`.

## Semântica

Um CQE é sucesso se, e somente se, `res >= 0` — resultado
negativo do io_uring é erro e o kernel não promove CQE
com falha a sucesso. É o res-gate que o handler `fsync` do
ring (`crates/pedradb-io-uring/src/ring.rs`) consome.

## AS-IS recusado

O twin as-is `cqe_res_ok_as_is` devolve `true` para todo
`res` — CQE de falha vira sucesso silencioso.

## Planta DST

`crates/pedradb-io-uring/src/lib.rs`:
`cqe_res_ok_on_live_uring_is_not_ok` — verde (1 passed).
Kernel `cqe_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py cqe_res`: linha `atom` no
`close_proofs.tsv` (entrada `cqe_res_ok`);
`atom_reason` datado no catálogo; floor_atom 134→135,
floor_extract 144→143; residuals/proof_depth
re-carimbados no mesmo commit.

## Escada

floor_atom 134→135, floor_extract 144→143. Gate
`check_depth_floor.py` GREEN: extract=143 (floor 143),
atom=135 (floor 135), data_fate=0<=0, residuals == live.

## Gates

`lake build Cqe` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
