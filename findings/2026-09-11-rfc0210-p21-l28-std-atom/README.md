# RFC-0210 P2.1 — cadência l28 final 2/6: atom `catalog:l28_tcp_std`

Data: 2026-09-11
Par: `l28_tcp_std` (kernel `crates/pedradb-store/src/l28.rs`, RFC-0145 P1.2)
Teorema: `l28_tcp_std_ok_fate_iff` em `formal/aeneas/lean/L28.lean`
(cadência l28 5/6; extração pure-lift do corpo `ok b`).

## Semântica

Após planta REAL TCP + morte do processo, a re-instalação do C-new
derruba (step-down) um Leader plantado em réplica removida de `ids`
exatamente quando o step-down aconteceu. O mutante AS-IS `ok true`
pula o step-down (leftover 0144: manter `Role::Leader`).

## Escada

- `cap_data_fate` 60→59, `floor_atom` 64→65, `floor_extract` 214→213.
- `close_proofs.tsv` +1 linha atom; catálogo: `data_fate` removido,
  `atom_reason` datado.

## O que foi pago

- Teorema ∀ fate sobre o extract do corpo de produção.
- Planta TCP REAL verde ANTES do commit:
  `cargo test --test l28_real_tcp -- l28_real_tcp_removed_std` —
  1 passed, 368.96s (captura no scratch do round 5).
- Gates 3× GREEN após a cirurgia (depth 213/65/6, data_fate 59<=59).
