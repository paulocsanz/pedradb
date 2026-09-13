# RFC-0218 — drenagem da escada seL4 (sel4_coverage 62,33% → 92,47%)

Round 2026-09-13. Métrica: `python3 scripts/sel4_coverage.py` (início
182/292 = 62,33% @ `52afb58c`; capturas `sel4_cov_start.txt`).
Cadência: 1 promoção = 1 commit (teorema iff-∀ no wrapper + TSV atom +
floors `floor_atom +1 / floor_extract −1` + `atom_reason` + residuals,
gates GREEN no commit).

## P0.1 — group_commit ×4 átomo

- **group_commit (1/4, entrada `occ_conflict`)**: o veredito OCC
  first-committer-wins é a janela exata —
  `occ_conflict_fate_iff` em `GroupCommit.lean`: `(occ_conflict snap
  last_seq touched = ok v) ↔ ((last_seq > snap ∧ v = touched) ∨
  (¬(last_seq > snap) ∧ v = false))`, provado sobre o
  `occ_conflict_closed_form` universal já registrado no wrapper.
  Build `lake build GroupCommit` verde (1699 jobs). Planta DST
  `occ_conflict_on_live_group_is_not_ok` (pedradb-core, exit 0,
  1 passed). Gate: floor_atom 178→179, floor_extract 100→99.
