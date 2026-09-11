# RFC-0212 P0.2 — cadência membership 1/4: atom `catalog:persist_meta`

Data: 2026-09-11
Par: `persist_meta` (kernel
`crates/pedradb-raft/src/membership_kernel.rs`, RFC-0135; entry
`persist_meta_node_counts`)
Teorema: `persist_meta_node_counts_fate_iff` em
`formal/aeneas/lean/Membership.lean` (cadência membership 2/6).

## Semântica

Meta SI persiste em TODA réplica local exatamente quando o nó é
local — pertencer a `ids` NÃO é o portão. O mutante AS-IS
`is_local && in_ids` deixa o clock da réplica removida sem
durabilidade (leftover 0134).

## Escada

- `cap_data_fate` 51→50, `floor_atom` 73→74, `floor_extract`
  205→204.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate`
  removido, `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract Aeneas do corpo de produção.
- Planta DST verde ANTES do commit (módulo three_teeth_queued, via
  `cargo test --lib -- persist_meta_node_counts_on_live_queued_is_not_ok`
  em pedradb-store — 1 passed, 0.57s; captura no scratch do
  round 6).
- Gates 3× GREEN após a cirurgia (depth 204/74/6, data_fate
  50<=50).
