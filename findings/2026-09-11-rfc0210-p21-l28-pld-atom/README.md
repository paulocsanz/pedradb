# RFC-0210 P2.1 — cadência l28 final 1/6: atom `catalog:l28_tcp_pld`

Data: 2026-09-11
Par: `l28_tcp_pld` (kernel `crates/pedradb-store/src/l28.rs`, RFC-0144 P1.2)
Teorema: `l28_tcp_pld_ok_fate_iff` em `formal/aeneas/lean/L28.lean`
(cadência l28 5/6; extração pure-lift do corpo `ok b`).

## Semântica

Após planta REAL TCP + morte do processo, o persist-leader do abort
sem líder é LOCAL (o `next_index` repair roda) exatamente quando o
persist aconteceu. O mutante AS-IS `ok true` pula a localidade do
persist-leader (leftover 0143: `ids.first`).

## Escada

- `cap_data_fate` 61→60, `floor_atom` 63→64, `floor_extract` 215→214
  (base pós-aposentadoria dos 7 fantasmas, fe236f91).
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate` removido,
  `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract do corpo de produção (o que o rustc
  liga é o termo da prova).
- Planta TCP REAL verde ANTES do commit:
  `cargo test --test l28_real_tcp -- l28_real_tcp_removed_pld` —
  1 passed, 231.81s (captura no scratch do round 5).
- Gates 3× GREEN após a cirurgia (depth 214/64/6, data_fate 60<=60).
