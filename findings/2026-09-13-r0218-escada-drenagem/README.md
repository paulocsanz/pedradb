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

- **changelog_should_store (2/6)**: portão citado do debounce —
  intervalo 0 nunca armazena na via do commit; intervalo positivo
  armazena quando commits >= intervalo —
  `changelog_should_store_fate_iff` em `Changelog.lean` (split at hval
  no if Prop; reverso rw if_pos/if_neg). Build verde. Planta DST
  `debounce_as_is_ignores_interval` (pedradb-core, exit 0). Gate:
  floor_atom 187→188, floor_extract 91→90.

- **changelog_budget (3/6)**: lift puro da comparação citada —
  materializa sse live_entries ≤ budget_entries —
  `changelog_rebuild_within_budget_fate_iff` em `Changelog.lean`
  (injection direta; as-is devolve true sem freio). Build verde.
  Planta DST `rebuild_budget_bounds_materialization` (pedradb-core,
  exit 0). Gate: floor_atom 188→189, floor_extract 90→89.

- **flush_plan (4/6)**: árvore de dois ifs citada — imm presente
  termina-e-flusha; sem imm, memtable vazio só rotaciona; memtable
  vivo escreve SST antes de rotacionar — `flush_plan_fate_iff` em
  `Flush.lean` (3 ramos; forward split at hval, reverso rw
  if_pos/if_neg com Bool.not_eq_true). Build verde. Planta DST
  `theorem_flush_plan_on_finite_domain` (pedradb-core, exit 0). Gate:
  floor_atom 189→190, floor_extract 89→88.

- **first_install (5/6)**: tabela citada F196 — commitado (com ou sem
  sync) prossegue; falha recusa abrir —
  `first_install_action_fate_iff` em `Manifest.lean` (3 ramos disjuntos,
  reverso refuta por noConfusion). Build verde. Planta DST
  `f196_first_install_tolerance` (pedradb-core, exit 0). Gate:
  floor_atom 190→191, floor_extract 88→87.

- **dictionary_link (6/6)**: política citada do reopen em tabela
  completa de 4 linhas — sem dano serve tudo; dano com point-in-time
  não escalado serve o prefixo reportado; dano escalado ou sem
  point-in-time recusa abrir — `reopen_outcome_flat_fate_iff` em
  `Reopen.lean` (mais fina que a iff agrupada prévia de 2026-09-12,
  que permanece; braço combinado 4 danos com split at hval). Build
  verde. Planta DST `crash_after_sync_recovers_committed`
  (pedradb-sim, exit 0). Gate: floor_atom 191→192, floor_extract
  88→87. **P0.3 fechada: ×6 átomo, 6/6.**

## P0.4 — crc/magic/scan ×9 átomo

- **sst_magic / scan 9/9 (fechamento P0.4)**: admissão de mágica como
  cadeia citada — sem os 8 bytes (len header < len mágica) recusa
  false; com prefixo possível, admite sse o slice-eq do prefixo contra
  PEDRSST\0 casa (`∃ s, lift = ok s ∧ (¬len≥ → false ∨ len≥ → cadeia
  index×2 + eq`)) — `sst_magic_is_pedra_fate_iff` em `Magic.lean`.
  EXTRACT NOVO: `scripts/aeneas_magic.sh` → `MagicKernel.lean` (shim
  `formal/aeneas/magic-kernel` com const SST_MAGIC re-exposta e pin
  no script; Charon/Aeneas 0 sorries). Forward: 4× bind_ok_inv
  (zeta via simp only [] antes do split); reverso: unfold + exact
  bind_intro com show (if len ≥ len). Build verde. Planta DST
  `sst_magic_as_is_admits_cpp_header` (pedradb-core, exit 0 no
  worktree). Gate: floor_atom 200→201, floor_extract 78→77. P0.4
  fechada 9/9 — P0 fechada (205/292 = 70,21%).

- **scan_guard / scan_reads_file (8/9)**: guardião de leitura como a
  cadeia citada — bounds dizem lê (b true ⇒ v true); senão o iter +
  any sobre os túmulos decide (`∃ i, iter = ok i ∧ ∃ b1 c, any =
  ok (b1, c) ∧ v = b1`) — `scan_reads_file_fate_iff` em `Scan.lean`.
  Forward: triplo bind_ok_inv, uncurry do let-tupla via coeção defeq
  (`have hval' : ok b1 = ok v := hval`); reverso: refine bind_intro +
  show (if false …) + if_neg e cadeia bind_intro (subst hv limpava o
  b1 — usado rw). Build verde. Planta DST
  `scan_reads_file_on_live_sst_is_not_ok` (pedradb-core, exit 0 no
  worktree). Gate: floor_atom 199→200, floor_extract 79→78.

- **tombstone_reaches_window (7/9)**: janela de túmulo como os dois
  gates citados half-open — alcança a janela sse t_end > start E
  t_start não passou do fim (`∃ a b, gate = ok a ∧ gate = ok b ∧
  v = a && b`, gates privados DEFEQ aos lets; Included/Excluded do
  start ambos gt) — `tombstone_reaches_window_fate_iff` em
  `Scan.lean`. Moldo da 5/9: duplo bind_ok_inv + split / cadeia
  bind_intro. Build verde. Planta DST `half_open_boundaries`
  (pedradb-core, exit 0 no worktree). Gate: floor_atom 198→199,
  floor_extract 80→79.

- **point_bounds_overlap (6/9)**: gate de arquivo como bounds citados —
  sem smallest/largest lê (true); com ambos, lê sse lo passou no fim E
  hi passou no start (três disjunctos, o terceiro `∃ a b, v = a && b`
  com gates privados DEFEQ aos lets) —
  `point_bounds_overlap_fate_iff` em `Scan.lean`. Forward cases
  smallest/largest + simp only (iota) + duplo bind_ok_inv; reverso
  subst + rfl / cadeia bind_intro. Build verde. Planta DST
  `point_bounds_overlap_on_live_bounds_is_not_ok` (pedradb-core, exit 0
  no worktree). Gate: floor_atom 197→198, floor_extract 81→80.

- **key_in_window (5/9)**: janela booleana como os dois gates citados
  — a chave entra sse passou no start E passou no fim
  (`∃ a b, gate_start = ok a ∧ gate_end = ok b ∧ v = a && b`,
  gates privados DEFEQ aos lets do kernel) —
  `key_in_window_fate_iff` em `Scan.lean`. Forward com duplo
  bind_ok_inv + split; reverso exact bind_intro em cadeia. Build
  verde. Planta DST `key_in_window_on_live_window_is_not_ok`
  (pedradb-core, exit 0 no worktree). Gate: floor_atom 196→197,
  floor_extract 82→81.

- **crc_match (1/9)**: lift puro da igualdade citada — checksum casa
  sse stored = computed (as-is admite sempre) —
  `crc_match_ok_fate_iff` em `Crc.lean`. Build verde. Planta DST
  `crc_match_ok_on_live_wal_is_not_ok` (pedradb-core, exit 0, 1
  passed — no worktree isolado `../pedradb-r0218-p04-plants` ao HEAD
  b92a383a: a árvore principal ficou com pedradb-core não-compilável
  por edição em voo da sessão paralela RFC-0217 em memtable.rs —
  intocada aqui por cânone; captura
  r0218_p04_plants_worktree.txt). Gate: floor_atom 192→193,
  floor_extract 86→85.

- **sst_block_crc (2/9)**: lift da igualdade citada através do
  crc_match_ok — bloco casa sse stored = computed (as-is admite
  sempre) — `sst_block_crc_ok_fate_iff` em `Scan.lean` (helpers
  bind_ok_inv/bind_intro privados copiados do molde WalRecover).
  Build verde. Planta DST `sst_block_crc_uses_crc_match_ok`
  (pedradb-core, exit 0 no worktree). Gate: floor_atom 193→194,
  floor_extract 85→84.

- **zero_glue (3/9)**: constante citada — cola residual zero nunca
  é admitida (false fixo; as-is acha que sumiu) —
  `zero_glue_admitted_fate_iff` em `Scan.lean`. Build verde. Planta
  DST `zero_glue_admitted_on_live_db_is_not_ok` (pedradb-core, exit 0
  no worktree). Gate: floor_atom 194→195, floor_extract 84→83.

- **sst_crc (4/9)**: árvore citada — checksum casa → StripTrailer;
  mismatch em arquivo legado (< SST_LEGACY_NO_CRC_MAX) →
  WholeBuffer; mismatch moderno → Reject (as-is sempre StripTrailer)
  — `sst_crc_fate_flat_fate_iff` em `Scan.lean` (bind_ok_inv no
  crc_match_ok, split at hval duplo, reverso show defeq + rw
  if_pos/if_neg). Build verde. Planta DST
  `sst_crc_fate_on_live_sst_is_not_ok` (pedradb-core, exit 0 no
  worktree). Gate: floor_atom 195→196, floor_extract 83→82.


## P1.1 — compact ×7 + lsm_r1 ×3 átomo

- **lsm_r1 / lsm_reopen (10/10, fecha P1.1)**: reabrir R1 como a
  identidade citada — o estado sai intacto (`r = s`) —
  `lsm_reopen_fate_iff` em `LsmR1.lean`. Forward: unfold + injection +
  hv.symm; reverso: rintro + subst + rfl. Build verde. Planta DST
  `r1_modelo_on_live_delete_shape_is_not_ok` (pedradb-sim, exit 0 no
  worktree). Gate: floor_atom 210→211, floor_extract 68→67. Com este,
  P1.1 fecha 10/10 (floor_atom 201→211).

- **lsm_r1 / lsm_probe (9/10)**: provar R1 como EXATAMENTE um passo
  do loop citado — via `Aeneas.Std.loop.eq_def` (one-step unfold do
  fixpoint): o corpo no nível 0 ou termina (`done o`) ou desce um
  nível (`cont i'`, resto citado) — `lsm_probe_fate_iff` em
  `LsmR1.lean`. Forward: eq_def + cases no corpo (rfl como testemunha
  do gate); reverso: eq_def + rw. Build verde. Planta DST
  `r1_modelo_on_live_delete_shape_is_not_ok` (pedradb-sim, exit 0 no
  worktree). Gate: floor_atom 209→210, floor_extract 69→68.

- **lsm_r1 / lsm_compact (8/10)**: despacho de compactação R1 como
  encaminhamento citado — nível 0 e nível ≥ MAX_LEVELS não compactam
  (ok none); dentro, o loop citado `lsm_compact_src_loop` com
  `drop_all_tombs false` (3 disjunctos; as-is passa true e derruba
  túmulos vivos) — `lsm_compact_fate_iff` em `LsmR1.lean`. Forward:
  duplo split + injection; reverso: rw if_pos/if_neg. Build verde.
  Planta DST `r1_modelo_on_live_delete_shape_is_not_ok` (pedradb-sim,
  exit 0 no worktree). Gate: floor_atom 208→209, floor_extract 70→69.

- **lone_tombstone / lone_tombstone_fate (7/10)**: o túmulo
  solitário cai só no nível mais baixo — Drop exige `bottommost` E
  `lone_newest`; todo o resto Keep (3 disjunctos flat) —
  `lone_tombstone_fate_iff` em `Compact.lean`. Forward: duplo split +
  injection + hv.symm; reverso: rintro + subst + rfl. Build verde.
  Planta DST `theorem_lone_tombstone_on_finite_domain` (pedradb-core,
  exit 0 no worktree). Gate: floor_atom 207→208, floor_extract 71→70.

- **compact_split_at / compact_should_split_at (6/10)**:
  dividir-no-ponto como o lift citado `decide (written_bytes >=
  target)` — mutantes nunca dividem —
  `compact_should_split_at_fate_iff` em `Compact.lean`. Forward:
  unfold + injection + hv.symm; reverso: rintro + subst + rfl.
  Build verde. Planta DST `compact_split_mutants_never_split_is_not_ok`
  (pedradb-core, exit 0 no worktree). Gate: floor_atom 206→207,
  floor_extract 72→71.

- **compact_split / compact_should_split (5/10)**: dividir como o
  bind citado — o gate `COMPACT_TARGET_FILE_BYTES` produz o alvo `i` e
  `compact_should_split_at w i` decide —
  `compact_should_split_fate_iff` em `Compact.lean` (moldes
  `bind_ok_inv`/`bind_intro` locais). Forward: unfold + bind_ok_inv;
  reverso: bind_intro. Build verde. Planta DST
  `compact_should_split_bounds_one_output_file` (pedradb-core, exit 0
  no worktree). Gate: floor_atom 205→206, floor_extract 73→72.

- **compact / compact_ready (4/10)**: pronto-para-compactar como o
  lift citado `decide (min_applied > 0)` — zero aplicado não compacta
  nada — `compact_ready_fate_iff` em `StoreCompact.lean`. Forward:
  unfold + injection + hv.symm; reverso: rintro + subst + rfl. Build
  verde. Planta DST `as_is_ready_at_zero_compacts_nothing`
  (pedradb-store, exit 0 no worktree). Gate: floor_atom 204→205,
  floor_extract 74→73.

- **compact / peer_counts_for_compact (3/10)**: contagem de pares
  como participação booleana — par offline ainda conta, quorum não
  encolhe com queda — `peer_counts_for_compact_fate_iff` em
  `StoreCompact.lean`: `peer_counts_for_compact is_participating = ok v
  ↔ v = true`. Forward: unfold + injection + hv.symm; reverso:
  rintro + subst + rfl. Build verde. Planta DST
  `offline_peer_still_counts` (pedradb-store, exit 0 no worktree).
  Gate: floor_atom 203→204, floor_extract 75→74.

- **compact_floor / compact_index_floor (2/10)**: piso pós-compactação
  como a soma saturada citada through + 1 (u64::MAX satura — nunca
  envolve a zero) — `compact_index_floor_fate_iff` em
  `StoreCompact.lean`. Forward: unfold + injection (fecha sozinho);
  reverso: rw. Build verde. Planta DST
  `as_is_floor_re_requests_compacted_index` (pedradb-store, exit 0 no
  worktree). Gate: floor_atom 202→203, floor_extract 76→75.

- **compact / may_compact_through (1/10)**: permissão de compactar
  como árvore citada de 3 gates — recusa through zero, recusa coberto
  pelo snapshot, recusa term zero; autoriza só com os três abertos
  (4 disjunctos flat) — `may_compact_through_fate_iff` em
  `StoreCompact.lean`. Forward: triplo split + injection; reverso:
  rw if_pos/if_neg encadeado. Build verde. Planta DST
  `may_compact_through_on_live_queued_is_not_ok` (pedradb-store,
  exit 0 no worktree). Gate: floor_atom 201→202, floor_extract
  77→76.
## P1.2 — leveling ×4 + merge ×2 + index_val ×3 + key + prefix ×11 átomo

- **leveling / leveled_enabled (11/11, fecha P1.2)**: modo leveled como a leitura citada de `PEDRA_LEVELED` — gate `std.env.var` cotado; `Err` liga (true); `Ok` deref + trim + `ne "0"` (as-is engole o desligamento) — `leveled_enabled_fate_iff` em `Leveling.lean`. Forward: bind_ok_inv + cases r (Ok/Err do core Result) + 2× bind_ok_inv; reverso: bind_intro ×3. Build verde. Planta DST `leveled_env_switch_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 221→222, floor_extract 57→56. Com este, P1.2 fecha 11/11 (floor_atom 211→222).

- **index_val / len_pref_value (10/11)**: valor com prefixo de comprimento como a cadeia citada — capacidade `len+4`, `try_from`+`expect` do len (u32), `to_be_bytes`, `to_slice`, `extend` do prefixo e do valor (7 binds citados) — `len_pref_value_fate_iff` em `IndexVal.lean`. Forward: 6× bind_ok_inv + hval final; reverso: bind_intro ×6. Build verde. Planta DST `len_pref_value_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 220→221, floor_extract 58→57.

- **leveling / total_bytes (9/11)**: total do nível como a soma citada — `Slice.iter`, `Iterator.map` com a closure que extrai `bytes`, `Iterator.sum` u64 (as-is devolve contagem de arquivos) — `total_bytes_fate_iff` em `Leveling.lean`. Forward: 2× bind_ok_inv; reverso: bind_intro ×2. Build verde. Planta DST `total_bytes_on_live_level_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 219→220, floor_extract 59→58.

- **leveling / overlaps (8/11)**: sobrepor o hull como o par citado — `as_slice` do `lo` + `le hull_hi` abre a porta; `as_slice` do `hi` + `ge hull_lo` confirma — `overlaps_fate_iff` em `Leveling.lean`. Forward: 2× bind_ok_inv + split + bind_ok_inv; reverso: bind_intro ×3 com if defeq. Build verde. Planta DST `overlaps_on_live_slice_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 218→219, floor_extract 60→59.

- **leveling / is_disjoint (7/11)**: disjunção como o despacho citado — `is_disjoint files` É `is_disjoint_outer_loop files 0#usize` — `is_disjoint_fate_iff` em `Leveling.lean`. Forward/reverso: unfold + exact. Build verde. Planta DST `is_disjoint_on_live_stack_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 217→218, floor_extract 61→60.

- **merge / range_tombstone_covers (6/11)**: cobrir por túmulo de range como o par citado — `ge key start` abre a porta e `lt key end` fecha (as-is testa só igualdade com start) — `range_tombstone_covers_fate_iff` em `Merge.lean`. Forward: bind_ok_inv + split; reverso: bind_intro com if defeq. Build verde. Planta DST `range_tombstone_covers_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 216→217, floor_extract 62→61.

- **index_val / exact_value_children (5/11)**: filhos exatos como a cadeia citada — `to_vec` do prefixo, `push 0#u8` (início), `push 1#u8` (fim) — `exact_value_children_fate_iff` em `IndexVal.lean`. Forward: 3× bind_ok_inv + injection; reverso: bind_intro ×3. Build verde. Planta DST `as_is_leaks_nul_sibling` (pedradb-store, exit 0 no worktree). Gate: floor_atom 215→216, floor_extract 63→62.

- **prefix / prefix_exclusive_end (4/11)**: fim exclusivo como o encaminhamento citado — `to_vec` do prefixo e o loop citado decide (incrementa ou some) — `prefix_exclusive_end_fate_iff` em `Prefix.lean`. Forward: unfold + bind_ok_inv; reverso: bind_intro. Build verde. Planta DST `prefix_exclusive_end_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 214→215, floor_extract 64→63.

- **key / pack_sequence_and_type (3/11)**: empacotar ikey como a cadeia citada — teto `MAX_SEQUENCE_NUMBER` lido e afirmado (`massert (seq <= i)`), `seq <<< 8`, `as_u8` + `lift` do tipo, pacote = `i1 ||| i3` (6 componentes ∃) — `pack_sequence_and_type_fate_iff` em `Key.lean` (moldes `bind_ok_inv`/`bind_intro` locais). Forward: 5× bind_ok_inv + injection; reverso: bind_intro ×5. Build verde. Planta DST `pack_sequence_and_type_on_live_db_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 213→214, floor_extract 65→64.

- **merge / write_op_range_end (2/11)**: fim do range como o despacho citado — Deletion e Value sem fim (none); RangeDeletion carrega o valor (some) — `write_op_range_end_fate_iff` em `Merge.lean`. Forward: cases kind + injection; reverso: rintro + subst + rfl. Build verde. Planta DST `write_op_range_end_on_live_stage_unapplied_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 212→213, floor_extract 66→65.

- **index_val / value_len_tag (1/11)**: etiqueta de comprimento como o
  lift citado `len` (identidade — injetiva em len; as-is colapsa a 0) —
  `value_len_tag_fate_iff` em `IndexVal.lean`. Forward: unfold +
  injection + hv.symm; reverso: rintro + subst + rfl. Build verde.
  Planta DST `len_tag_is_injective` (pedradb-store, exit 0 no
  worktree). Gate: floor_atom 211→212, floor_extract 67→66.
## P1.3 — si/snapshot/txn/rpc ×11 átomo

- **txn / tx_recover (11/11, fecha P1.3)**: recuperar é exatamente a decisão citada leftover_fate — tx sobrada (não committed) vira aborto cercado com visible zerado; committed fica como está (as-is deixa o leftover vivo: visibilidade parcial do mid-apply sobrevive) — `tx_recover_fate_iff` em `T1Modelo.lean`. Forward: bind_ok_inv + split + injection; reverso: bind_intro + if_pos/if_neg. Build verde. Planta DST `t1_modelo_on_live_abort_reopen_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 232→233, floor_extract 46→45. Fecho P1.3: floor_atom 222→233, sel4_coverage 81,16%.- **txn / tx_abort (10/11)**: abortar é exatamente a cadeia citada — committed devolve o próprio estado; senão txn_commit_action true é Revert (massert), revert_clears_status true true é false (massert) e o estado sai com visible zerado, aborted e cercado (as-is solta o cerca: commit replay materializa a tx abortada) — `tx_abort_fate_iff` em `T1Modelo.lean`. Forward: split + bind_ok_inv ×5 + injection; reverso: if_neg + bind_intro ×5. Build verde. Planta DST `t1_modelo_on_live_abort_reopen_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 231→232, floor_extract 47→46.- **si / si_reader (9/11)**: eleger o leitor SI é exatamente a cascata citada — liveness (líder+participante) decide; empatada, participação; empatada, self; empatada, o watermark applied (as-is nunca avança o leitor) — `si_reader_beats_fate_iff` em `Si.lean`. Forward: bind_ok_inv ×2 + splits aninhados + injection; reverso: bind_intro ×2 + if_pos/if_neg por nível. Build verde. Planta DST `si_reader_beats_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 230→231, floor_extract 48→47.- **si / si_read (8/11)**: servir ou recusar um snapshot é exatamente comparar contra o piso citado watermark-1 (saturating) — abaixo do piso TooOld fail-closed, no piso ou acima Serve (as-is serve todo mundo: ausência fabricada) — `snapshot_read_plan_fate_iff` em `Si.lean`. Forward: bind_ok_inv + split + injection; reverso: bind_intro + if_pos/if_neg. Build verde. Planta DST `snapshot_read_plan_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 229→230, floor_extract 49→48.- **txn / should_repair_si_hist (7/11)**: repara o hist SI exatamente quando a chave foi restaurada E não é reservada (as-is nunca repara — dente plantado) — `should_repair_si_hist_fate_iff` em `StoreTxn.lean`. Forward: unfold + split + injection; reverso: rintro + subst + rfl. Build verde. Planta DST `repair_hist_only_when_restored_user_key` (pedradb-store, exit 0 no worktree). Gate: floor_atom 228→229, floor_extract 50→49.- **rpc / allow_direct_rpc (4/11)**: RPC direto como o despacho citado — sem pedido direto true; com pedido direto, false se dst_pin é líder, true senão (as-is não olha dst_pin) — `allow_direct_rpc_fate_iff` em `RpcMode.lean`. Forward: duplo split + injection; reverso: rintro + subst + rfl. Build verde. Planta DST `allow_direct_rpc_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 225→226, floor_extract 53→52.

- **txn / recover_si_generation (3/11)**: geração SI sobrevive ao restart como o lift citado `loaded_max` (as-is zera) — `recover_si_generation_fate_iff` em `StoreTxn.lean`. Forward: unfold + injection + hv.symm; reverso: rintro + subst + rfl. Build verde. Planta DST `si_generation_survives` (pedradb-store, exit 0 no worktree). Gate: floor_atom 224→225, floor_extract 54→53.

- **snapshot / snapshot_touches_user_key (6/11)**: o snapshot toca chave de usuário exatamente quando a chave NÃO é reservada — lift citado `decide (¬ (is_reserved = true))` (as-is true toca até reservada) — `snapshot_touches_user_key_fate_iff` em `Snapshot.lean`. Forward: unfold + injection + hv.symm; reverso: rintro + subst + rfl. Build verde. Planta DST `snapshot_touches_user_key_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 227→228, floor_extract 51→50.

- **snapshot / snapshot_needs_txn_meta_clear (2/11)**: restaurar snapshot SEMPRE exige limpar o metadado de txn — constante citada true (as-is false vaza txn meta) — `snapshot_needs_txn_meta_clear_fate_iff` em `Snapshot.lean`. Forward: unfold + injection + hv.symm; reverso: rintro + subst + rfl. Build verde. Planta DST `always_clear_txn_meta` (pedradb-store, exit 0 no worktree). Gate: floor_atom 223→224, floor_extract 55→54.

- **si / point_get_watermark (5/11)**: watermark do point-get como o lift citado `range_applied` — o global_seq não entra (as-is devolve global_seq e lê não-aplicado) — `point_get_watermark_fate_iff` em `Si.lean`. Forward: unfold + injection + hv.symm; reverso: rintro + subst + rfl. Build verde. Planta DST `point_get_uses_range_applied` (pedradb-store, exit 0 no worktree). Gate: floor_atom 226→227, floor_extract 52→51.

- **si / point_get_prefer_applied (1/11)**: point-get prefere o índice
  applied como a constante citada true (as-is false ignora o applied) —
  `point_get_prefer_applied_fate_iff` em `Si.lean`. Forward: unfold +
  injection + hv.symm; reverso: rintro + subst + rfl. Build verde.
  Planta DST `point_get_uses_range_applied` (pedradb-store, exit 0 no
  worktree). Gate: floor_atom 222→223, floor_extract 56→55.

## P2.1 — journal/stream/fold/ship/capi ×12 átomo

- **apply / apply_step (p2.2 16/21)**: o passo de aplicacao e exatamente a arvore citada — na frente do commit Done; atras do commit Apply so com entrada presente, buraco e Stop (o prefixo contiguo e a fronteira; as-is aplica o buraco) — `apply_advance_fate_iff` em `Apply.lean`. Forward: split duplo + injection + Bool.not_eq_true; reverso: rintro 3-folhas + if_pos/if_neg. Build verde. Planta DST `apply_advance_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 260->261, floor_extract 18->17.- **commit / raft_recover_applied (p2.2 15/21)**: o applied recuperado no open e exatamente a constante citada zero — o replay recomeca do zero, o disco nunca dita o applied (as-is reanima o ultimo gravado: salto de aplicacao) — `recover_last_applied_fate_iff` em `Commit.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `recover_last_applied_on_live_queued_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 259->260, floor_extract 19->18.- **l28 / l28_napply_retry (p2.2 14/21)**: retries do harness não são ∀ traços TCP — a admissão é a constante citada false — `l28_tcp_napply_retry_admitted_fate_iff` em `L28.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `l28_tcp_napply_retry_admitted_is_not_forall` (pedradb-store, exit 0 no worktree). Gate: floor_atom 258→259, floor_extract 20→19.- **l28 / l28_durability (p2.2 13/21)**: a impressão digital de durabilidade é exatamente a conjunção citada — get_ok E after_kill_ok E restart_ok (cascata de ifs Bool) — `l28_durability_ok_fate_iff` em `L28.lean`. Forward: split duplo + injection + Bool.not_eq_true; reverso: rintro de rfl tripla. Build verde. Planta DST `l28_durability_ok_on_live_store_is_not_ok` (pedradb-store, exit 0 no worktree). Gate: floor_atom 257→258, floor_extract 21→20.- **scale / scale_forecast (p2.2 12/21)**: a tabela RFC-0176 é exatamente a composição citada — 12 binds, cada campo o átomo do kernel correspondente, hot o gate decide (store <= warm_cap), o registro mk de 13 campos — `scale_forecast_fate_iff` em `Scale.lean` (sem recusa medida: o corpo comporta a iff; a previsão é inteira, sem float). Forward: bind_ok_inv ×12 + split no ite n_files + injection; reverso: bind_intro aninhado + if_pos/if_neg + rw do registro. Build verde. Planta DST `scale_forecast_on_always_hot_walk_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 256→257, floor_extract 22→21.- **scale / scale_predict (p2.2 11/21)**: o relógio previsto é exatamente a cadeia citada — clamps Ord.min duplo, mix saturating em u64, conta em u128 (lift FromU128U64), divisões, try_from com unwrap_or u64::MAX — `predict_get_ns_fate_iff` em `Scale.lean`. Forward: bind_ok_inv ×18 em cadeia; reverso: bind_intro ×18 aninhado + rw. Build verde. Planta DST `predict_get_ns_on_ignoring_noisy_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 255→256, floor_extract 23→22.- **scale / scale_warm (p2.2 10/21)**: o teto do WARM é exatamente a cadeia citada — piso 3 GiB quando ceiling 0; senão max(piso, share 3/4) cortado pelo reservado (ceiling − 1 GiB) — `warm_cap_bytes_fate_iff` em `Scale.lean`. Forward: bind_ok_inv em cadeia (sat_mul, div, WARM_FLOOR, ite-cap, WARM_RESERVE, lift) + split duplo + injection; reverso: bind_intro em cadeia + if_pos/if_neg com subst. Build verde. Planta DST `warm_cap_bytes_on_unbounded_warm_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 254→255, floor_extract 24→23.- **scale / scale_happy_hot (p2.2 9/21)**: a fração quente do caminho feliz é exatamente o gate citado — warm_cap_bytes gate; loja cabendo devolve SCALE_BPS, senão o residual SCALE_HAPPY_COLD_HOT_BPS — `happy_hot_bps_fate_iff` em `Scale.lean`. Forward: bind_ok_inv + split + injection; reverso: bind_intro + if_pos/if_neg. Build verde. Planta DST `happy_hot_bps_on_always_hot_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 253→254, floor_extract 25→24.- **scale / scale_probes_worst (p2.2 8/21)**: o pior caso de produção é exatamente a mesma soma citada — point_get_probes com o trigger cheio do L0 — `probes_worst_fate_iff` em `Scale.lean` (callee direto, Iff.rfl após unfold). Build verde. Planta DST `probes_worst_on_l0_trigger_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 252→253, floor_extract 26→25.- **scale / scale_probes (p2.2 7/21)**: os probes de um point get são exatamente o saturating_add citado — levels + L0 cobrindo, nunca o número de arquivos — `point_get_probes_fate_iff` em `Scale.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `point_get_probes_on_all_files_walk_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 251→252, floor_extract 27→26.- **probe / run_disjoint (p2.2 6/21)**: disjunção de run é exatamente o par citado — n = min dos comprimentos (Ord.min gate), n >= 2 e o all citado sobre 1..n — `run_disjoint_fate_iff` em `ProbeOrder.lean`. Forward: bind_ok_inv (min, Iterator.all) + split + injection; reverso: bind_intro + if_pos/if_neg. Build verde. Planta DST `run_disjoint_on_live_equal_lo_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 250→251, floor_extract 28→27.- **probe / probe_order_covering (p2.2 5/21)**: a lista de candidatos é exatamente o loop citado — capacidade len newest_first, início 0, o porte covering_pos e o gate covering_hi_ge decidindo push/skip — `probe_order_covering_fate_iff` em `ProbeOrder.lean` (loop como callee opaco, canônico). Iff por Iff.rfl após unfold (lets puros). Build verde. Planta DST `probe_order_covering_on_live_gate_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 249→250, floor_extract 29→28.- **probe / probe_order (p2.2 4/21)**: no empate de lo o primeiro probe é exatamente o índice mais novo citado — `probe_order_fate_iff` em `ProbeOrder.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `theorem_first_probe_on_every_distinct_tie` (pedradb-core, exit 0 no worktree, varre os empates distintos). Gate: floor_atom 248→249, floor_extract 30→29.- **bloom / bloom_may_contain (p2.2 3/21)**: a consulta decide exatamente no loop citado — inativo devolve ok true (sem filtro, tudo pode estar presente); ativo calcula hash_pair e a resposta É o loop citado may_contain_loop sobre bits — `may_contain_fate_iff` em `Bloom.lean`. Forward: bind_ok_inv (is_active, hash_pair) + split; reverso: bind_intro em cadeia + if_pos/if_neg + loop exato. Build verde. Planta DST `may_contain_on_live_sst_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 247→248, floor_extract 31→30.- **bloom / bloom_insert (p2.2 2/21)**: o insert escreve exatamente os k probes citados — inativo devolve o próprio filtro; ativo calcula hash_pair e roda o loop citado insert_loop sobre bits (nbits via conversão pura citada) — `insert_fate_iff` em `Bloom.lean`. Forward: bind_ok_inv (is_active, hash_pair, insert_loop) + split + injection; reverso: bind_intro em cadeia + if_pos/if_neg. Build verde. Planta DST `insert_on_live_sst_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 246→247, floor_extract 32→31.- **bloom / bloom_header (p2.2 1/21)**: o cabeçalho admite exatamente a conjunção citada — k dentro de [1, MAX_K] (RangeInclusive.contains gate), nbytes cobrindo div_ceil nbits 8 e nbytes cabendo no residual — `bloom_header_fate_iff` em `Bloom.lean`. Forward: bind_ok_inv em cadeia (contains, lift×2, div_ceil, lift) + split + injection; reverso: bind_intro em cadeia + if_pos/if_neg. Build verde. Planta DST `bloom_header_ok_on_live_decode_is_not_ok` (pedradb-core, exit 0 no worktree). Gate: floor_atom 245→246, floor_extract 33→32.- **capi / c_len (12/12)**: admissao no C ABI e exatamente o teto citado len <= max — acima do teto devolve false e o set/get ao vivo responde LIMIT (as-is aceita qualquer comprimento e copia) — `c_len_fate_iff` em `CapiHandles.lean`. Forward: unfold + injection; reverso: subst + rfl (forma decide para o Bool do teto). Build verde. Planta DST `c_len_admitted_on_live_capi_is_not_ok` (pedradb-capi, exit 0 no worktree). Gate: floor_atom 244→245, floor_extract 34→33.- **ship / pull_plan (11/12)**: o plano de envio e exatamente as guardas citadas — arquivo menor que o cursor devolve Rotated; stamp mudado devolve Rotated; senao Ship com min(len - cursor, max_pull), nunca bytes desalinhados — `pull_plan_fate_iff` em `Ship.lean`. Forward: cases + dsimp + split + bind_ok_inv duplo + injection; reverso: rintro da disjuncao 9-folhas + dsimp + if_pos/if_neg + bind_intro. Build verde. Planta DST `pull_plan_on_live_ship_is_not_ok` (pedradb-replicate, exit 0 no worktree). Gate: floor_atom 243→244, floor_extract 35→34.- **ship / ship_stamp (10/12)**: o stamp mudou exatamente como citado — stamp agora maior já é mudança (true); do mesmo tamanho, mudou é o prefixo citado de mesmo comprimento ser diferente (ne gate no Slice.index) — `stamp_changed_fate_iff` em `Ship.lean`. Forward: dsimp + split + bind_ok_inv + injection; reverso: dsimp + if_pos/if_neg + bind_intro. Build verde. Planta DST `stamp_changed_on_rewrite_is_not_ok` (pedradb-replicate, exit 0 no worktree). Gate: floor_atom 242→243, floor_extract 36→35.- **fold / fold_range (9/12)**: o fold esconde a chave exatamente como citado — evento de range esconde key >= start E key < end (gates em cascata); pontual esconde só a igualdade key = start (as-is mostra tudo: segredo vaza no fold) — `fold_event_hides_key_fate_iff` em `Fold.lean`. Forward: split duplo + bind_ok_inv + injection; reverso: subst + bind_intro por ramo. Build verde. Planta DST `fold_event_hides_key_on_live_fold_is_not_ok` (pedradb-fold, exit 0 no worktree). Gate: floor_atom 241→242, floor_extract 37→36.- **fold / isolated (8/12)**: o id isolado casa exatamente na guarda citada de tamanho — chave menor que o id nunca casa; com tamanho suficiente, a decisão é o loop citado isolated_id_matches_loop (fronteira em ISOLATED_CHILD_SEP; corpo do loop não reaberto neste degrau) — `isolated_id_matches_fate_iff` em `Isolated.lean`. Forward: unfold + dsimp + split + injection; reverso: dsimp + if_pos/if_neg. Build verde. Planta DST `isolated_id_matches_on_live_fold_is_not_ok` (pedradb-fold, exit 0 no worktree). Gate: floor_atom 240→241, floor_extract 38→37.- **fold / isolated_child (7/12)**: depois de um id exato, o byte de continuação é filho exatamente quando é a barra citada ISOLATED_CHILD_SEP (as-is aceita qualquer byte: irmão vira filho) — `isolated_child_byte_fate_iff` em `Isolated.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `theorem_next_byte_domain` (pedradb-fold, exit 0 no worktree, varre o domínio u8 inteiro). Gate: floor_atom 239→240, floor_extract 39→38.- **stream / stream_cursor (6/12)**: ack em ordem é exatamente a cadeia citada — o cursor esperado é next_seq last_acked (gate) e o ack conta só quando a sequência é a esperada E à frente do último (as-is aceita qualquer à frente: pula buracos) — `ack_in_order_fate_iff` em `Cursor.lean`. Forward: bind_ok_inv + split + injection; reverso: bind_intro + if_pos/if_neg. Build verde. Planta DST `ack_in_order_on_live_stream_is_not_ok` (pedradb-stream, exit 0 no worktree). Gate: floor_atom 238→239, floor_extract 40→39.- **stream / stream_next_seq (5/12)**: a próxima sequência é exatamente o lift citado saturating_add last_acked 1 — o ack anda uma casa sem overflow (as-is devolve o próprio last_acked: ack não avança) — `next_seq_fate_iff` em `Cursor.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `ack_in_order_on_live_stream_is_not_ok` (pedradb-stream, exit 0 no worktree). Gate: floor_atom 237→238, floor_extract 41→40.- **journal / journal_next_pin (4/12)**: o próximo pin é exatamente o citado — sem batch, o pin fica; com batch_max, anda para o batch_max somente quando está à frente (as-is deixa o pin regredir) — `next_pin_fate_iff` em `Pin.lean`. Forward: cases Option + dsimp + split + injection; reverso: dsimp + if_pos/if_neg. Build verde. Planta DST `next_pin_on_live_journal_is_not_ok` (pedradb-journal, exit 0 no worktree). Gate: floor_atom 236→237, floor_extract 42→41.- **journal / journal_pin (3/12)**: avançar o pin é exatamente o lift citado applied_through > pin — o journal só solta o que já foi aplicado (as-is nunca segura: pin anda antes do applied) — `may_advance_pin_fate_iff` em `Pin.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `may_advance_pin_on_live_journal_is_not_ok` (pedradb-journal, exit 0 no worktree). Gate: floor_atom 235→236, floor_extract 43→42.- **journal / journal_fold_pin (2/12)**: varrer nunca segura pins — constante citada false (as-is true: o fold congela o journal) — `fold_pins_on_read_fate_iff` em `Pin.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `fold_pins_on_read_on_live_journal_is_not_ok` (pedradb-journal, exit 0 no worktree). Gate: floor_atom 234→235, floor_extract 44→43.- **journal / journal_catch_up_pin (1/12)**: ler com pins atrasados sempre alcança os pins — constante citada true (as-is false: leitura deixa pins para trás) — `catch_up_pins_on_read_fate_iff` em `Pin.lean`. Forward: unfold + injection; reverso: subst + rfl. Build verde. Planta DST `catch_up_pins_on_read_on_live_journal_is_not_ok` (pedradb-journal, exit 0 no worktree). Gate: floor_atom 233→234, floor_extract 45→44.
