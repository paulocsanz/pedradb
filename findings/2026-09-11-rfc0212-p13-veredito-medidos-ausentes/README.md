# RFC-0212 P1.3 — veredito datado dos medidos ausentes

**Data:** 2026-09-11
**Veredito:** ZERO pares medidos ausentes. Nenhuma reescrita, nenhuma
aposentadoria.

## O que foi medido

A fatia era condicional: "SE alguma promoção acima medir
fn/planta/handler inexistente". Nas **22 promoções** do bloco
membership (P0.1 ×4, P0.2 ×4, P1.1 ×8, P1.2 ×6), tudo mediu PRESENTE:

- **22 fns** do kernel `crates/pedradb-raft/src/membership_kernel.rs`
  existiam com corpo real (Bool decisions, incluindo 2 corpos
  invertidos `!in_ids` e 1 constante `true`);
- **22 mutantes as_is** existiam e seus dentes assert no kernel
  continuam verdes (bloco de testes do próprio kernel);
- **22 plantas DST** existiam em
  `crates/pedradb-store/src/three_teeth_queued.rs` e rodaram verdes via
  `--lib` (lotes paralelos: 8 em 1.93s, 6 em 1.51s, e as 8 do P0
  individualmente);
- **22 defs Lean** extraídos (`MembershipKernel.lean`, `Result Bool`)
  sustentaram os teoremas iff no wrapper `Membership.lean` —
  `lake build Membership` verde a cada promoção (1699 jobs).

## Âncoras 0203/0204 — verdes antes/depois

- **Antes** (worktree destacado no início da rodada, commit
  `5db39a2c`):
  - `check_inventory_terminal.py`: GREEN — 7 rows terminal, 0 deferido,
    0 todo
  - `check_twin_contracts.py`: GREEN — 7/7 count rows bound
- **Depois** (HEAD `7f733f96`):
  - `check_inventory_terminal.py`: GREEN — idem
  - `check_twin_contracts.py`: GREEN — idem

O worktree do "antes" foi removido após a captura.

## Consequência

Nenhum gate de identidade foi inventado; nenhum par foi forçado. O
dreno do bloco membership fechou limpo: 22/22 pares promovidos a
`atom` com teorema sobre corpo real, membership ZERO `data_fate`
medido ao vivo no catálogo.
