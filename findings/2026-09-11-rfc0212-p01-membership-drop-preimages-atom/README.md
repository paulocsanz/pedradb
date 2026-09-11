# RFC-0212 P0.1 — cadência membership 3/4: atom `catalog:drop_preimages`

Data: 2026-09-11
Par: `drop_preimages` (kernel
`crates/pedradb-raft/src/membership_kernel.rs`, RFC-0139; entry
`drop_preimages_node_counts`)
Teorema: `drop_preimages_node_counts_fate_iff` em
`formal/aeneas/lean/Membership.lean` (cadência membership 1/6).

## Semântica

Preimages de prepare-time caem em TODA réplica local exatamente
quando o nó é local — pertencer a `ids` NÃO é o portão. O mutante
AS-IS `is_local && in_ids` deixa a réplica removida com preimages
(leftover 0138).

## Escada

- `cap_data_fate` 53→52, `floor_atom` 71→72, `floor_extract`
  207→206.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate`
  removido, `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract Aeneas do corpo de produção.
- Planta DST verde ANTES do commit:
  `cargo test --lib --
  drop_preimages_node_counts_on_live_queued_is_not_ok` (módulo three_teeth_queued, roda via --lib;
  captura no scratch do round 6).
- Gates 3× GREEN após a cirurgia (depth 206/72/6, data_fate
  52<=52).
