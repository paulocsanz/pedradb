# RFC-0191 P2.4 — registro terminal dos herdados do 0187 + recount do marker do ledger

P2.4 é um slice de REGISTRO, não de código: os três gates herdados do
RFC-0187 ficam registrados em estado terminal, sem re-fatiar e sem
descartar em silêncio.

## O que ficou registrado (`docs/verification-ledger.md`, nova seção
"Herdados do 0187 — estado terminal")

1. **Série L28** (33 pares `l28_*`, tier campaign): **user-gated** —
   promoção a teorema three-teeth não inicia sem decisão registrada
   (rank H). Nenhum par foi promovido por esta campanha.
2. **Nightly experimental de durabilidade física** (TCG guest power-cut
   por barreira + `F_FULLFSYNC` no macOS): permanece nightly,
   **sempre experimento**. "Persistiu no disco" além da barreira de SO
   não vira claim de teorema; a barreira de SO é TCB (linha já existente
   na tabela TCB).
3. **Exaustivo N=4 com poda/simetria**: aberto no 0187 por custo do
   runner; o exaustivo registrado segue N≤3 (gate P0.1).

Fronteira de crash-injection permanece NOMEADA: max T=12, max S=4, com a
coluna "Fora" explícita (T>12, S>4, setor partido/torn write, ∀π, timing
de grupo). Alargar é movimento de ledger com gate verde no mesmo commit.

## Dívida achada e paga no mesmo commit: marker do ledger defasado

`check_ledger_consistency.py` estava RED: marker `total=295` vs catálogo
298. Rastreio por `git log -S`: o marker congelou em 3afd7491 (294 pares)
e QUATRO pares desta própria campanha entraram sem mover o marker —
`merge_sift` (59738caa), `si_hist_repair` (2d60af11),
`apply_put_plan` (658951c3), `hist_load_fate` (f42d17ed) — 294+4=298.
Marker recontado: total 298, proof 265, single_artifact 291,
aeneas_scripts 231. Gate: **GREEN** (12 ponteiros, counts batem).

## Verificações

- `python3 scripts/check_ledger_consistency.py` → GREEN.
- RFC-0191: checkbox P2.4 `[x]`, status row `done` com o registro.
- status.md: linha RFC-0191 agora inclui P2.4 no prefixo.

## Estado do pacote RFC-0191 após este slice

P0.1–P0.3, P1.1–P1.5, P2.1–P2.4 done; falta P1.6 (sweep final de gates).
