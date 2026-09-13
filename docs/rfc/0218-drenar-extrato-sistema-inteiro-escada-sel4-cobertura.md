# RFC-0218 — Drenar o degrau extrato do sistema inteiro: a escada classe-seL4 medida por `sel4_coverage`

**Status:** draft
**Data:** 2026-09-13
**Autoria:** agente grind (round 10→11), sucessora direta do RFC-0216
(superfície de parse HTTP fechada, `**Status:** done` no HEAD `f45f5b14`)

## Contexto

O RFC-0216 fechou a família http 27/27 no degrau átomo — nenhum gate
do plano de request decidido por teste em vez de teorema — e a escada
medida ao vivo no HEAD `f45f5b14` é extract=100, close=6, atom=178,
count=7, `cap_data_fate 0<=0`. A medição ao vivo (`candidates.py` no
HEAD) zerou TODOS os boards de máquina outra vez (`unpaid_script 0/17`,
`unpaid_compose 0/17`, `unpaid_concurrency 0/5`, `unpaid_scale 0/3`,
`unpaid_product 0`, `unpaid_no_extract none`, buracos A/B/D/C vazios,
`atom_to_close none`); o cartoon Montanha segue em skip até o usuário
levantar (regra 13 do caminho), e o trampolim db.rs/concurrent.rs segue
congelado por non-goal herdado (RFC-0191 P1.5).

O que resta pagável de máquina é a ESCADA INTEIRA, não uma família: 110
pares do catálogo sem teorema ∀ registrado, espalhados por 39 kernels
em 9 crates — core (49), store (20), dcs (4), journal (4), fold (3),
raft (3), stream (2), replicate (2), capi (1). Este RFC é o plano de
drenagem gradual desse pool com UMA métrica de máquina repetível.

**A métrica (fixada aqui; comando único read-only):**
`python3 scripts/sel4_coverage.py` — cobertura seL4-escada =
pares do catálogo com degrau ∀ registrado (linha close OU atom em
`close_proofs.tsv`) ÷ total de pares do catálogo (292). Denominador é a
superfície declarada INTEIRA (pares nascidos de campanha `l28_*`
incluídos: 24 já provados contam; a EXECUÇÃO de campanha nunca conta —
campanha ≠ ∀π segue TCB). Crédito `count` (RFC-0199) não conta
(ortogonal à escada). O comando cross-checka os floors ao vivo
(`floor_atom`/`floor_close`) e falha se a métrica divergir da escada.

Medida datada:
- **início (pré-promoção do 8/8, HEAD `a193ed1d`): 181/292 = 61,99%**
  (captura `{SCRATCH}/sel4_cov_start.txt`);
- pós-RFC-0216 (HEAD `f45f5b14`): 182/292 = 62,33%.

## Pool pagável medido (2026-09-13, board vivo + corpo dos kernels lido)

Dos 110 pendentes:
- **88 pagáveis** (73 close-tier + 15 model-tier; os model-tier sobem
  pela mesma escada iff — o twin modelo é executável, o teorema é sobre
  o corpo extraído do kernel);
- **10 Montanha** (`children`×4, `fields`×5, `pack`×1) — cartoon em
  skip até o usuário levar; ficam no denominador, nunca no numerador
  deste RFC;
- **12 cânone-excluídos** (corpo lido, medidos): stand-ins `*_admitted`
  e constantes de campanha — `forall_schedules`, `media_durable`,
  `lock_interleavings` (admitted, cânone do 0216), `stacked_liars`,
  `default_pct_depth_raised`, `pct_default_depth`,
  `fsync_lie_tcg`/`fsync_lie_closes_tcg_guest`, `tcg_guest`,
  `world_trajectory`, `world_trajectory_fold`, `liveness_claim`,
  `fdatasync_rc` — campanha/TCG, não viram ∀ por construção.

Teto honesto SEM mudar cânone: 270/292 = **92,47%** (drenando os 88
pagáveis); com os 10 Montanha (se o usuário levantar): 280/292 =
95,89%. Os 12 excluídos só sobem por decisão registrada de cânone,
nunca por promoção silenciosa.

## Meta mensurável

`python3 scripts/sel4_coverage.py` verde no HEAD de cada promoção
(cross-check floors), alvos DATADOS:
- P0 fechado: **≥ 70%** (205/292 = 70,21%);
- P1 fechado: **≥ 81%** (237/292 = 81,16%);
- P2 fechado: **≥ 92%** (270/292 = 92,47%);
- cadência: 1 promoção = 1 commit (teorema iff no wrapper, planta DST
  verde ANTES do commit, gate GREEN no commit), mesma regra dos
  RFC-0214/0215/0216.

## Fatias

1. **P0.1** group_commit ×4 (`GroupCommit.lean`): `group_validate`
   (loop de validação OCC — molde de indução dos loops Form/DecodeFate),
   `group_commit`, `group_fence`, `fsync_promote` — floor_atom
   178→182 — status: `done` (2026-09-13; 4/4: `group_commit`/
   `occ_conflict_fate_iff`, `fsync_promote`/
   `fsync_promotes_pending_fate_iff`, `group_fence`/
   `fence_publish_seq_fate_iff`, `group_validate`/
   `group_validate_fate_iff` — molde Form/DecodeFate nos 2 loops;
   floor_atom 182, floor_extract 96)

2. **P0.2** wal/recover ×4 (`WalRecover.lean`): `fragment_act`,
   `from_record_type`, `is_length_resyncable`, `physical_payload_act` —
   o parse fail-closed do WAL — floor_atom 182→186 — status: `done`
   (2026-09-13; 4/4: `from_record_type_fate_iff`,
   `is_length_resyncable_fate_iff`, `physical_payload_act_fate_iff`,
   `fragment_act_fate_iff`)

3. **P0.3** changelog ×3 + flush ×1 + manifest ×1 + reopen ×1
   (`Changelog.lean`/`Flush.lean`/`Manifest.lean`/`Reopen.lean`):
   `changelog`, `changelog_budget`, `changelog_should_store`,
   `flush_plan`, `first_install`, `dictionary_link` — floor_atom
   186→192 — status: `done`
   (2026-09-13; 6/6: átomos changelog ×3 + `flush_plan_fate_iff` +
   `first_install_action_fate_iff` + `reopen_outcome_flat_fate_iff`)

4. **P0.4** CRC/mágica/scan ×9 (`Scan.lean`/`Crc.lean`/`MagicKernel`):
   `scan_guard`, `sst_block_crc`, `sst_crc`, `zero_glue`,
   `key_in_window`, `point_bounds_overlap`, `tombstone_reaches_window`,
   `crc_match`, `sst_magic` — a família fail-closed de integridade —
   floor_atom 192→201, sel4_coverage **70,21%** — status: `done`
   (2026-09-13; 9/9: crc_match, sst_block_crc, zero_glue,
   `sst_crc_fate_flat_fate_iff`, `key_in_window_fate_iff`,
   `point_bounds_overlap_fate_iff`, `tombstone_reaches_window_fate_iff`,
   `scan_reads_file_fate_iff` e `sst_magic_is_pedra_fate_iff`
   — extract novo `MagicKernel` via `scripts/aeneas_magic.sh`)

5. **P1.1** compact ×7 + lsm_r1 ×3: `compact`, `compact_floor`,
   `compact_peer_counts`, `compact_ready` (store), `compact_split`,
   `compact_split_at`, `lone_tombstone` (core), `lsm_compact`,
   `lsm_probe`, `lsm_reopen` — floor_atom 201→211 — status: `done`
   (10/10 feitos 2026-09-13: `may_compact_through_fate_iff`,
   `compact_index_floor_fate_iff`, `peer_counts_for_compact_fate_iff`,
   `compact_ready_fate_iff`, `compact_should_split_fate_iff`,
   `compact_should_split_at_fate_iff`, `lone_tombstone_fate_iff`,
   `lsm_compact_fate_iff`, `lsm_probe_fate_iff` e
   `lsm_reopen_fate_iff`)

6. **P1.2** leveling ×4 + merge ×2 + index_val ×3 + key ×1 + prefix ×1:
   `leveled_enabled`, `leveling_disjoint`, `leveling_overlaps`,
   `leveling_total_bytes` (`leveling`/`leveling_pick`/`leveling_pushdown`
   JÁ têm teorema — ficam de fora), `range_covers`, `write_op_range_end`,
   `exact_children`, `index_val`, `len_tag`, `ikey_pack`, `prefix` —
   floor_atom 211→222 — status: `todo`
   (11/11 feitos 2026-09-13: `value_len_tag_fate_iff`,
   `write_op_range_end_fate_iff`, `pack_sequence_and_type_fate_iff`,
   `prefix_exclusive_end_fate_iff`, `exact_value_children_fate_iff`,
   `range_tombstone_covers_fate_iff`, `is_disjoint_fate_iff`,
   `overlaps_fate_iff`, `total_bytes_fate_iff`, `len_pref_value_fate_iff` e
   `leveled_enabled_fate_iff`)

7. **P1.3** si/snapshot/txn ×11: `si_read`, `si_reader`,
   `point_get_prefer`, `point_get_wm`, `snapshot`, `snap_txn_clear`,
   `recover_si_generation`, `should_repair_si_hist`, `tx_abort`,
   `tx_recover`, `rpc_mode` — floor_atom 222→233, sel4_coverage
   **81,16%** — status: `todo`
   (8/11: `point_get_prefer_applied_fate_iff`,
   `snapshot_needs_txn_meta_clear_fate_iff`, `recover_si_generation_fate_iff`,
   `allow_direct_rpc_fate_iff`, `point_get_watermark_fate_iff`,
   `snapshot_touches_user_key_fate_iff`, `should_repair_si_hist_fate_iff` e
   `snapshot_read_plan_fate_iff` feitos 2026-09-13)

8. **P2.1** periferia ×12: `journal_pin`, `journal_catch_up_pin`,
   `journal_fold_pin`, `journal_next_pin`, `stream_cursor`,
   `stream_next_seq`, `isolated`, `isolated_child`, `fold_range`,
   `ship_guard`, `ship_stamp`, `c_len` — floor_atom 233→245 — status:
   `todo`

9. **P2.2** raft/dcs/l28/scale/bloom/probe ×21: `apply_step`,
   `ae_f16_gate`, `raft_recover_applied`, `dcs_apply`,
   `dcs_advance_bool`, `lease_next_id`, `lease_table`,
   `l28_durability`, `l28_napply_retry`, `scale_probes`,
   `scale_probes_worst`, `scale_warm`, `scale_forecast`,
   `scale_happy_hot`, `scale_predict`, `bloom_header`, `bloom_insert`,
   `bloom_may_contain`, `probe_order`, `probe_order_covering`,
   `run_disjoint` — floor_atom 245→266, sel4_coverage **92,47%** —
   status: `todo`

10. **P2.3** sweep final em worktree DENTRO de `software/` (gates 3×
    GREEN + campaign ok + extracts ok + sorry 0, capturas
    `{SCRATCH}/r0218_sweep_*`) + nota datada `EXTRACT.md` + flip
    `**Status:** done` — status: `todo`

## Vereditos / riscos

- Se um corpo extraído não comportar a iff (recusa MEDIDA e nomeada —
  candidato natural: `scale_forecast`, aritmética de previsão com
  float), veredito datado em findings + `EXTRACT.md`; o alvo datado da
  meta ajusta-se ao caminho medido e o par sai do pool pagável com o
  número publicado — nunca gate inventado (mesma regra dos
  RFC-0214/0215/0216).
- `promote_atom.py` (rito round 8/9/10) segue válido: TSV com tabs
  reais, `floor_atom +1 / floor_extract −1` (até o extrato zerar nos
  pagáveis), `atom_reason` datado, `residuals.json` — nunca `json.dump`
  no catálogo vivo.
- A métrica NÃO é declaração de paridade com seL4 (seL4 ≈ 20:1
  prova:impl por anos, RFC-0061): ela mede a COBERTURA DA ESCADA
  (degraus ∀ registrados sobre a superfície declarada). O avanço
  "drástico" é drenar 88 pares restantes com o mesmo rigor — o sistema
  inteiro no degrau átomo, família por família.

## Não-metas

- NÃO despejar `db.rs`/`concurrent.rs` no provador (trampolim
  congelado; `db_rs_extracted` fica false; `leftover_next` P1.5 é do
  usuário levantar).
- NÃO tocar `crates/montanha-fdb-recipes/**` (cartoon em skip até o
  usuário levantar; os 10 pares ficam no denominador como dívida
  visível).
- NÃO flipar `forall_schedules`/`media_durable`/`lock_interleavings`
  admitted; os 12 cânone-excluídos não viram ∀ por este RFC; campanha
  ≠ ∀π segue TCB.
- `never_floor` imutável (R-cpu R-rustc R-verus R-crc R-deps);
  `cap_data_fate 0<=0` imutável; Disk/`fdatasync` Ok não é mídia
  (RFC-0078).
- NÃO é meta deste RFC: benchmark/paridade Rocks (RFC-0217, sessão
  paralela), Montanha cartoon, TCB.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | group_commit ×4 átomo (validação OCC/fence) | done | `occ_conflict_fate_iff`/`fsync_promotes_pending_fate_iff`/`fence_publish_seq_fate_iff`/`group_validate_fate_iff` | 2026-09-13 |
| P0.2 | p0 | wal/recover ×4 átomo (parse fail-closed) | done | r0218 p0.2 átomos 1-4/4 (7c62a63c→) | 2026-09-13 |
| P0.3 | p0 | changelog+flush+manifest+reopen ×6 átomo | done | r0218 p0.3 átomos 1-6/6 (c6e9e586→) | 2026-09-13 |
| P0.4 | p0 | crc/magic/scan ×9 átomo (integridade) | done | r0218 p0.4 átomos 1-9/9 (f8a0b0d2→) | 2026-09-13 |
| P1.1 | p1 | compact ×7 + lsm_r1 ×3 átomo | done | 10/10 átomo (floor_atom 201→211) | 2026-09-13 |
| P1.2 | p1 | leveling/merge/index_val/key/prefix ×11 átomo | done | 11/11 átomo (floor_atom 211→222) | 2026-09-13 |
| P1.3 | p1 | si/snapshot/txn/rpc ×11 átomo | todo | — | 2026-09-13 |
| P2.1 | p2 | journal/stream/fold/ship/capi ×12 átomo | todo | — | 2026-09-13 |
| P2.2 | p2 | raft/dcs/l28/scale/bloom/probe ×21 átomo | todo | — | 2026-09-13 |
| P2.3 | p2 | Sweep final + nota EXTRACT.md + flip done | todo | — | 2026-09-13 |

## Critérios de aceite

- **Cada átomo**: teorema iff-∀ no wrapper com `lake build` verde;
  `promote_atom.py` (TSV + floors + `atom_reason` + residuals);
  `python3 scripts/check_depth_floor.py` GREEN; planta DST isolada
  verde com exit checado; 1 teorema/commit (`git show HEAD -- <lean> |
  grep -c "^+theorem"` == 1); linha da RFC flipada no mesmo commit;
  findings README da fatia.
- **A métrica**: `python3 scripts/sel4_coverage.py` verde e monotônica
  em cada HEAD de promoção (cross-check floors); capturas datadas
  início/fim em findings.
- **P2.3**: worktree destacado DENTRO de `software/`: depth-floor
  GREEN (atom=266, extract=27, model=0 se os 88 pousarem — 73 sobem do
  extrato, 15 do modelo; ou o chão medido),
  inventory terminal, twin-contracts bound, `test_proof_vs_campaign.py`
  ok, extracts ok, sorry 0 nos wrappers da rodada; capturas
  `{SCRATCH}/r0218_sweep_*`; nota datada em `EXTRACT.md`;
  `**Status:** done` no mesmo commit.
