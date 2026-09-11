# RFC-0210 P2.1 — cadência l28 final 3/6: atom `catalog:l28_tcp_hnt`

Data: 2026-09-11
Par: `l28_tcp_hnt` (kernel `crates/pedradb-store/src/l28.rs`, RFC-0146 P1.2)
Teorema: `l28_tcp_hnt_ok_fate_iff` em `formal/aeneas/lean/L28.lean`
(cadência l28 5/6; extração pure-lift do corpo `ok b`).

## Semântica

Após planta REAL TCP + morte do processo, o ctor TCP de um eleitor
remanescente não roteia `leader_hint` à réplica removida exatamente
quando o hint foi filtrado. O mutante AS-IS `ok true` pula o filtro
do hint (leftover 0145: qualquer `leader_id`).

## Escada

- `cap_data_fate` 59→58, `floor_atom` 65→66, `floor_extract` 213→212.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate` removido,
  `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract do corpo de produção.
- Planta TCP REAL verde ANTES do commit:
  `cargo test --test l28_real_tcp -- l28_real_tcp_hint` —
  1 passed, 366.61s (captura no scratch do round 5).
- Gates 3× GREEN após a cirurgia (depth 212/66/6, data_fate 58<=58).
