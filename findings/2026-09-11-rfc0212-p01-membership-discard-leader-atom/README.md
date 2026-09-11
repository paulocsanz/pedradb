# RFC-0212 P0.1 — cadência membership 2/4: atom `catalog:discard_leader`

Data: 2026-09-11
Par: `discard_leader` (kernel
`crates/pedradb-raft/src/membership_kernel.rs`, RFC-0144; entry
`discard_leader_local`)
Teorema: `discard_leader_local_fate_iff` em
`formal/aeneas/lean/Membership.lean` (cadência membership 1/6).

## Semântica

O persist-leader do discard sem líder é nó LOCAL exatamente quando
o nó escolhido é local — o repair de `next_index` roda onde o
persist aterra. O mutante AS-IS `ok true` aceita `ids.first()`
mesmo remoto (leftover 0143).

## Escada

- `cap_data_fate` 54→53, `floor_atom` 70→71, `floor_extract`
  208→207.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate`
  removido, `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract Aeneas do corpo de produção.
- Planta DST verde ANTES do commit:
  `cargo test --lib --
  discard_leader_local_on_live_queued_is_not_ok` (módulo three_teeth_queued, roda via --lib;
  captura no scratch do round 6).
- Gates 3× GREEN após a cirurgia (depth 207/71/6, data_fate
  53<=53).
