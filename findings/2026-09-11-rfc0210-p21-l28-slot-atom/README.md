# RFC-0210 P2.1 — cadência l28 final 4/6: atom `catalog:l28_tcp_slot`

Data: 2026-09-11
Par: `l28_tcp_slot` (kernel `crates/pedradb-store/src/l28.rs`, RFC-0147 P1.2)
Teorema: `l28_tcp_slot_ok_fate_iff` em `formal/aeneas/lean/L28.lean`
(cadência l28 5/6; extração pure-lift do corpo `ok b`).

## Semântica

Após planta REAL TCP + morte do processo, o ctor TCP de um eleitor
remanescente esquece next/match/sent_through da réplica removida
exatamente quando o slot caiu (install derruba os repl-slots do
removido). O mutante AS-IS `ok true` pula o drop do slot (leftover
0146: manter next/match/sent_through).

## Escada

- `cap_data_fate` 58→57, `floor_atom` 66→67, `floor_extract` 212→211.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate` removido,
  `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract do corpo de produção.
- Planta TCP REAL verde ANTES do commit:
  `cargo test --test l28_real_tcp -- l28_real_tcp_drop_repl` —
  1 passed, 349.23s (captura no scratch do round 5).
- Gates 3× GREEN após a cirurgia (depth 211/67/6, data_fate 57<=57).
