# RFC-0214 P1.1 8/8 (FECHAMENTO) — átomo `catalog:write_ack_ack`

Data: 2026-09-12. Oitava e última promoção de P1.1 (cqe ×5 +
write_ack ×3). Escada viva: floor_atom 141→142, floor_extract
137→136. FECHAMENTO P1.1: floor_atom 134→142, floor_extract
144→136.

## O que foi pago

Teorema `on_ack_fate_iff` em
`formal/aeneas/lean/WriteAck.lean`: fate ∀ profundo sobre o
corpo extraído — o Ok do ack existe iff `acked ≤ synced` (o
passo exige a invariante) e é EXATAMENTE o estado com
`acked := synced`. A prova atravessa a subtração checada do
gap (`UScalar.sub_equiv`), o `saturating_add` do corpo (val =
`Nat.min max (acked + pending)` com `U64.lt_succ_max` +
`native_decide` nos limites) e o add checado
(`UScalar.add_equiv`): o saturado promove só até a barreira,
o add checado fecha `acked + pending = synced`, e o ramo else
do corpo é absurdo dentro da invariante.

## Semântica

`WriteAckLedger.on_ack` transforma o gap `synced − acked` em
acknowledged — e NADA além: cada byte que o Ok promete ao
cliente passou pela barreira antes. Fora de Inv-WAL
(`acked > synced`) a subtração checada do gap falha: sem Ok,
o ack nunca fabrica durabilidade.

## AS-IS recusado

O twin as-is `write_ack_ledger_as_is` acka sem barreira — o
grupo é ackado com `synced` atrás de `written`: o Ok promete
durabilidade que a classe não tem.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`verified_write_ack_on_live_profile_is_not_ok` — verde
(1 passed, testada isolada com exit checado ANTES da
promoção). Kernel `write_ack_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py write_ack_ack` (entrada `on_ack`; teorema
`on_ack_fate_iff`): linha `atom` no `close_proofs.tsv`;
`atom_reason` datado no catálogo; floor_atom 141→142,
floor_extract 137→136; residuals/proof_depth re-carimbados
no mesmo commit.

## Escada

floor_atom 141→142, floor_extract 137→136 — alvo final de
P1.1 atingido. Gate `check_depth_floor.py` GREEN:
extract=136 (floor 136), close=6 (floor 6), atom=142
(floor 142), count=7.

## Gates

`lake build WriteAck` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
