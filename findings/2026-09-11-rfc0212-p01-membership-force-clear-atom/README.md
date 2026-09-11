# RFC-0212 P0.1 — cadência membership 4/4 (FECHAMENTO): atom `catalog:force_clear`

Data: 2026-09-11
Par: `force_clear` (kernel
`crates/pedradb-raft/src/membership_kernel.rs`, RFC-0138; entry
`force_clear_node_counts`)
Teorema: `force_clear_node_counts_fate_iff` em
`formal/aeneas/lean/Membership.lean` (cadência membership 1/6).

## Semântica

O clear TX force-local roda em TODA réplica local exatamente quando
o nó é local — pertencer a `ids` NÃO é o portão. O mutante AS-IS
`is_local && in_ids` deixa a réplica removida com intents presos
(leftover 0137).

## Escada (fechamento do P0.1)

- `cap_data_fate` 52→51, `floor_atom` 72→73, `floor_extract`
  206→205.
- Números exatos do P0.1: cap 55→51, floor_atom 69→73,
  floor_extract 209→205 (4 commits: 365e7c65, e5c4c346, 322b2241,
  este).

## O que foi pago

- Teorema ∀ fate sobre o extract Aeneas do corpo de produção.
- Planta DST verde ANTES do commit:
  `cargo test --lib --
  force_clear_node_counts_on_live_queued_is_not_ok` (módulo three_teeth_queued, roda via --lib;
  captura no scratch do round 6).
- Gates 3× GREEN após a cirurgia (depth 205/73/6, data_fate
  51<=51, ledger 292/266/26).
