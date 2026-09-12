# RFC-0214 P2.1 — composição ∀ da espinha de durabilidade (env → wal → ack)

Data: 2026-09-12. Nova compose lib `ComposeDurabilitySpine.lean`
(23ª da fila COMPOSE), twin kernel Rust
`crates/pedradb-core/src/durability_spine_kernel.rs` e planta DST
`durability_spine_compose_on_live_profile_is_not_ok` (1 passed).

## O que foi pago

A frase seL4 completa da espinha de durabilidade, composta sobre os
três átomos write_ack registrados no P1.1:

- `on_append_ok_step` / `on_barrier_ok_step` / `on_ack_ok_step`:
  cada passo da espinha É o futuro Ok do seu átomo — pontes
  derivadas dos iff (`on_append_fate_iff`, `on_barrier_fate_iff`,
  `on_ack_fate_iff`); os corpos extraídos não são abertos nos
  futuros ok. O passo barrier já carrega a costura env→wal dentro
  do átomo (`CrashModel.of` + sync honesto via
  `fsync_promotes_pending`).
- `spine_step` / `spine_reach`: qualquer caminho
  append/barrier/ack a partir do ledger frio.
- `spine_inv_every_reach`: Inv-WAL (`acked ⊆ synced ⊆ written`) é
  invariante de TODO caminho — indução sobre a cadeia, cada perna
  do átomo correspondente.
- `spine_d1_every_reach` (coroa): em todo estado alcançável, D1
  vale para o prefixo acked sobre TODO corte torn — a costura env
  (`CrashModel.of` + `crash_legal`) composta através do
  `d1_modelo`: nenhum corte legal perde byte acked. A coroa abre
  `inv_wal` + `d1_modelo` (+ `crash_legal`, total) uma vez —
  montagem da composição, não re-prova de perna.

## Twin kernel Rust

`durability_spine_kernel.rs`: `SpineStep` (Append/Barrier/Ack),
`spine_replay` dobra o ledger REAL (`write_ack_kernel`) sobre
qualquer sequência com `assert_inv` por passo (invariante composto
fail-closed), `spine_replay_as_is` pula a barreira
(`write_ack_ledger_as_is` por append) — 2 testes verdes no kernel.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`durability_spine_compose_on_live_profile_is_not_ok` — verde
(1 passed). Lado modelo: sequência entrelaçada (2 grupos + cauda
não sincada) mantém Inv-WAL por passo e D1 em todo corte; twin
as-is quebra o invariante no primeiro grupo ((64, 0, 64)). Lado
live: perfil verificado pinado, 2 puts (caminho lone G1 corrigido
no 6/8), Inv-WAL live, crash, os dois puts acked sobrevivem.

## Por que SEM linha TSV (razão datada)

Mesma regra das demais compose libs (precedente
`ComposeStorageWrite`, RFC-0213 P2.2): uma linha do ratchet pede
um PAR/entry único do catálogo; esta composição atravessa TRÊS
átomos (`catalog:write_ack_append`, `catalog:write_ack_barrier`,
`catalog:write_ack_ack`) — não é par único. A contagem autoritativa
da escada segue o gate `check_depth_floor.py` (imutável neste
slice: atom=142, extract=136, close=6, data_fate=0).

## Inscrição

`lakefile.toml` `[[lean_lib]] ComposeDurabilitySpine`; array
COMPOSE em `scripts/lean_extracts.sh` (64 libs + 23 compose,
build 1939 jobs verde); `pub mod durability_spine_kernel` no
`crates/pedradb-core/src/lib.rs`.

## Gates

`lake build ComposeDurabilitySpine` verde; `lean_extracts.sh
--required` ok (23 compose); planta DST verde (1 passed, exit
checado); twin kernel verde (2 passed); zero `sorry` na compose
lib; gate depth-floor imutável GREEN.
