# RFC-0213 P2.1 2/2 — cap-only `catalog:dictionary_link` (`reopen_outcome_fate_iff`, Reopen.lean)

Data: 2026-09-12. Par `dictionary_link` drenado CAP-ONLY: já era atom
desde 2026-09-10 (RFC-0191 P2.3, `b26c0ebf`,
`reopen_outcome_serve_all_iff_damage_none`) com a flag `data_fate`
esquecida — mesma classe da nota do visible_at (P1.2 3/6).

## O que o teorema novo diz

`reopen_outcome damage pt esc = ok v` ↔
((damage = None ∧ v = ServeAll) ∨
 (damage ≠ None ∧ pt ∧ ¬esc ∧ v = ServePrefixReport) ∨
 (damage ≠ None ∧ (¬pt ∨ esc) ∧ v = RefuseOpen)) — o fate para TODO
valor de saída, sobre o match extraído de 5 construtores (os 4
danos compartilham o if-chain `pt`/`esc`). O átomo de 2026-09-10
pinava só ServeAll; este iff fecha os três fates. Prova:
`cases damage <;> cases pt <;> cases esc <;> simp [reopen_outcome,
eq_comm]` (20 casos, todos por simp com o unfold da def).

## Por que cap-only (sem linha TSV nova)

O par já pagou seu degrau atom; a cirurgia desta data: só cap
1→0 (`proof_depth.tsv`), `glue.data_fate` 1→0 (`residuals.json`) e
`catalog.json` sem `data_fate` com `atom_reason` datado citando o
teorema fortalecido. floor_atom/floor_extract/close inalterados.

## LIBS + re-carimbo

`Reopen` inscrito no LIBS do `scripts/lean_extracts.sh` nesta
fatia (mesmo buraco do WalRecover — o script oficial não exigia o
wrapper). Extrato re-carimbado: `bash scripts/aeneas_reopen.sh`
regenerou `ReopenKernel.lean` **byte-idêntico** (diff vazio).

## Planta

`crash_after_sync_recovers_committed` (1 passed,
`cargo test --lib --manifest-path crates/pedradb-sim/Cargo.toml`)
— o crash-dictionary put→get que o as-is silencioso enganaria.

## Gate

`check_depth_floor.py`: GREEN — extract=156, ladder close=6 linhas,
atom=122, residuals close=5/atom=122 == live 5/122, count=7,
**data_fate=0**.

## FECHAMENTO P2.1 — CATÁLOGO ZERO

Sweep vivo do catálogo: 292 pares, `data_fate` restantes ZERO
(python sobre `catalog.json`, nesta data). A campanha de drenagem
do RFC-0213 cumpriu a promessa: nenhum par do catálogo fica sem
veredito medido.
