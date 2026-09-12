# RFC-0214 P1.1 5/8 — átomo `catalog:cqe_ring_refusal`

Data: 2026-09-12. Quinta promoção de P1.1 (cqe ×5 +
write_ack ×3) — FECHAMENTO do bloco cqe. Escada viva:
floor_atom 138→139, floor_extract 140→139.

## O que foi pago

Teorema `cqe_ring_model_admitted_fate_iff` em
`formal/aeneas/lean/Cqe.lean`: fate ∀ sobre o corpo
extraído — `(cqe_ring_model_admitted = ok v) ↔ v = false`.

## Semântica

A porta do modelo de anel Verus fica fechada: o res-gate
cqe não é prova de anel e a admissão é recusada por
construção (RFC-0074 P2.2 — a recusa é o comportamento
produção do kernel).

## AS-IS recusado

O twin as-is `cqe_ring_model_admitted_as_is` devolve
`true` — um twin res-gate passaria a parecer prova de anel.

## Planta DST

`crates/pedradb-io-uring/src/cqe_kernel.rs`:
`cqe_ring_model_is_not_admitted` — verde (1 passed).
Kernel `cqe_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py cqe_ring_refusal`: linha `atom` no
`close_proofs.tsv` (entrada `cqe_ring_model_admitted`);
`atom_reason` datado no catálogo; floor_atom 138→139,
floor_extract 140→139; residuals/proof_depth re-carimbados
no mesmo commit.

## Escada

floor_atom 138→139, floor_extract 140→139. Gate
`check_depth_floor.py` GREEN: extract=139 (floor 139),
atom=139 (floor 139), data_fate=0<=0, residuals == live.

## Gates

`lake build Cqe` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
