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

- **fsync_promote (2/4, entrada `fsync_promotes_pending`)**: lift puro
  — `fsync_promotes_pending_fate_iff`: `(fsync_promotes_pending
  os_honest = ok v) ↔ (os_honest = v)`; pending promove exatamente
  quando o OS/Env é honesto. Build verde. Planta DST
  `fsync_promotes_pending_on_live_sim_is_not_ok` (pedradb-sim,
  exit 0, 9 passed no módulo recording). Gate: floor_atom 179→180,
  floor_extract 99→98.

- **group_fence (3/4, entrada `fence_publish_seq`)**: primeiro átomo de
  LOOP extraído da rodada — molde Form/DecodeFate transplantado:
  `FenceFate` (combustível = membros restantes), `loop.eq_def`,
  progresso estrito `i < i' ≤ len`, `done` só no fim com best = v.
  `fence_publish_seq_fate_iff`: `(fence_publish_seq member_seqs = ok v)
  ↔ FenceFate member_seqs (len) 0 0 v`. O max interno do corpo é
  consumido pelo `bind_ok_inv` sem ramo (ambas as folhas são ok).
  Achado: `Slice` é ambíguo no wrapper (Aeneas.Std.Slice vs Std.Slice) —
  qualificar `Aeneas.Std.Slice` como faz `occ_batch_plan_fate_iff`.
  Build verde (1699 jobs). Planta DST `fence_publish_seq_on_live_group_
  is_not_ok` + `fence_is_max_member_seq` (pedradb-core, exit 0, 5
  passed). Gate: floor_atom 180→181, floor_extract 99→97 (com o 2/4).
