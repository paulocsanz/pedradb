# RFC-0212 P0.1 — cadência membership 1/6: atom `catalog:discard_uncommitted`

Data: 2026-09-11
Par: `discard_uncommitted` (kernel
`crates/pedradb-raft/src/membership_kernel.rs`, RFC-0143; entry
`discard_node_counts`)
Teorema: `discard_node_counts_fate_iff` em
`formal/aeneas/lean/Membership.lean` (cadência membership 1/6;
extração do corpo real `is_local`).

## Semântica

O discard vivo do log não-commitado roda em TODA réplica local
exatamente quando o nó é local — pertencer a `ids` NÃO é o portão:
réplica removida de `ids` ainda descarta seu sufixo. O mutante
AS-IS `is_local && in_ids` deixa a réplica removida com o sufixo
(leftover 0142).

## Escada

- `cap_data_fate` 55→54, `floor_atom` 69→70, `floor_extract`
  209→208.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate`
  removido, `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract Aeneas do corpo de produção.
- Planta DST verde ANTES do commit:
  `cargo test --lib --
  discard_node_counts_on_live_queued_is_not_ok` (módulo three_teeth_queued, roda via --lib;
  captura no scratch do round 6).
- Gates 3× GREEN após a cirurgia (depth 208/70/6, data_fate
  54<=54).
