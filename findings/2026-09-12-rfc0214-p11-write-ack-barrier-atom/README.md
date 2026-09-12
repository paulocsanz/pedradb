# RFC-0214 P1.1 7/8 — átomo `catalog:write_ack_barrier`

Data: 2026-09-12. Sétima promoção de P1.1 (cqe ×5 + write_ack
×3). Escada viva: floor_atom 140→141, floor_extract 138→137.

## O que foi pago

Teorema `on_barrier_fate_iff` em
`formal/aeneas/lean/WriteAck.lean`: fate ∀ sobre o corpo
extraído — o Ok da barreira Honest é EXATAMENTE o estado
promovido `synced := written` (e `written := written`); não
existe Ok que deixe `synced` atrás de `written`. O `min` de
`CrashModel.of` é sobrescrito pelo ramo Honest — o discard do
min se reduz nos dois ramos do split (ambos `ok`), fechando o
fate simbólico sem concretizar os campos.

## Semântica

`WriteAckLedger.on_barrier` promove TODO o pendente à
durabilidade: a barreira Honest não escolhe quais bytes
sobrevivem — `fsync_promotes_pending` devolve o próprio
honesty flag e o modelo promove `synced` até `written`. É o
degrau que separa o ack do as-is: ack sem essa promoção
promete durabilidade que a classe não tem.

## AS-IS recusado

O twin as-is `write_ack_ledger_as_is` pula a barreira — o
grupo é ackado com `synced` atrás de `written`: o Ok promete
durabilidade que a classe não tem (snapshot (64, 0, 64)).

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`verified_write_ack_on_live_profile_is_not_ok` — verde
(1 passed, testada isolada com exit checado ANTES da
promoção). Kernel `write_ack_kernel.rs` intocado nesta
promoção (nenhum re-extract necessário).

## Cirurgia de catálogo

`promote_atom.py write_ack_barrier` (entrada `on_barrier`;
teorema `on_barrier_fate_iff`): linha `atom` no
`close_proofs.tsv`; `atom_reason` datado no catálogo;
floor_atom 140→141, floor_extract 138→137;
residuals/proof_depth re-carimbados no mesmo commit.

## Escada

floor_atom 140→141, floor_extract 138→137. Gate
`check_depth_floor.py` GREEN: extract=137 (floor 137),
close=6 (floor 6), atom=141 (floor 141), count=7.

## Gates

`lake build WriteAck` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.
