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

- **group_validate (4/4, entrada `group_validate`)**: mesmo molde —
  `ValidateFate` com combustível = membros restantes; o passo `cont`
  cita index→occ_conflict→push (todos ok-ramos consumidos pelo
  bind_ok_inv). `group_validate_fate_iff`: `(group_validate reads
  last_seq = ok v) ↔ ValidateFate reads last_seq (len) (with_capacity)
  0 v`. A entrada com dois lets puros fecha por `unfold group_validate;
  rfl`. Planta DST: módulo group_commit_kernel inteiro (pedradb-core,
  exit 0, 16 passed — inclui `group_members_are_simultaneous` e o
  property sweep rfc0157). Gate: floor_atom 181→182, floor_extract
  97→96. **P0.1 fechada: group_commit ×4 átomo, 4/4.**

## P0.2 — wal/recover ×4 átomo

- **from_record_type (1/4)**: bijeção total citada RecordType→FragKind —
  `from_record_type_fate_iff` em `WalRecover.lean` (5 ramos disjuntos,
  reverso refuta por noConfusion). Build verde. Planta DST
  `from_record_type_on_wire_type_is_not_ok` (pedradb-core, exit 0).
  Gate: floor_atom 182→183, floor_extract 96→95.

- **is_length_resyncable (2/4)**: classificador citado — trio de dano de
  comprimento (Truncated/LengthCorrupt/UnknownType) → true, demais seis
  tipos → false — `is_length_resyncable_fate_iff` em `WalRecover.lean`
  (9 ramos disjuntos planos, reverso refuta por noConfusion). Build
  verde. Planta DST `resync_only_length_class` (pedradb-core, exit 0).
  Gate: floor_atom 183→184, floor_extract 95→94.

- **physical_payload_act (3/4)**: guarda físico do payload — árvore de
  três ifs citada (oversize → FailStop; payload além do bloco →
  FailStop no fim físico, Truncated no meio; dentro → Continue) —
  `physical_payload_act_fate_iff` em `WalRecover.lean` (forward split
  at hval; reverso rw if_pos/if_neg). Build verde. Planta DST
  `physical_oversize_and_torn` (pedradb-core, exit 0). Gate:
  floor_atom 184→185, floor_extract 94→93.

- **fragment_act (4/4)**: tabela completa FragKind × scratch_empty citada —
  Full produz, First começa, órfão (Middle/Last com scratch vazio)
  fail-stopa, Middle cheio acumula, Last cheio produz, Zero pula —
  `fragment_act_fate_iff` em `WalRecover.lean` (7 ramos disjuntos;
  forward simp only + split at hval no if, reverso subst+rfl). Build
  verde. Planta DST `orphan_middle_last_fail_stop` (pedradb-core,
  exit 0). Gate: floor_atom 185→186, floor_extract 93→92.
  **P0.2 fechada: wal/recover ×4 átomo, 4/4.**

## P0.3 — changelog+flush+manifest+reopen ×6 átomo

- **changelog (1/6)**: janela citada do rebuild — feed vazio com
  seq>0 precisa, feed vivo nunca — `changelog_needs_sst_rebuild_fate_iff`
  em `Changelog.lean` (2 ramos por cases do Bool; valor decide citado).
  Build verde. Planta DST: `--test changelog_model` (pedradb-core, 2
  passed, exit 0 — modelo Stateright sobre o fn real F53; a planta
  queued viva `changelog_needs_sst_rebuild_on_live_queued_is_not_ok`
  falha PREEXISTENTE nesta caixa macOS/PosixFallback — fora do escopo
  r0218, nada de Rust tocado). Gate: floor_atom 186→187,
  floor_extract 92→91.
