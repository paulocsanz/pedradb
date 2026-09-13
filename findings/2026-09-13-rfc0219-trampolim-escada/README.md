# RFC-0219 — o trampolim vira escada (um `if` data-fate por kernel nomeado)

Round 2026-09-13. Métrica: `python3 scripts/sel4_coverage.py` (início
270/292 = 92,47% @ `80782f6c`; capturas `sel4_cov_start.txt`). Contador
trampolim medido pelo grep datado do RFC:
`grep -c -E 'if .*(sync|flush|durable|visible|publish|fence|fsync)'`
sobre `crates/pedradb-core/src/db.rs` + `concurrent.rs`.

## Medição datada da fila (2026-09-13T17:08:52Z, pré-P0.1)

- `db.rs`: **53** sítios
- `concurrent.rs`: **22** sítios
- total da fila: **75** (bate com a fila medida do RFC; captura
  `queue_dbrs_20260913.txt` no scratch do round)

## P0.1 — changelog_durable_commit (1º pull, db.rs `commit_ops_with`)

- **Sítio**: `commit_ops_with` — o `if wal_sync_required(durability.sync
  .is_some(), durability.sync.unwrap_or(false), self.sync)` que decidia
  inline se o commit terminado contava no debounce do CHANGELOG
  (RFC-0031 P0.1). Corpo lido: `maybe_persist_changelog_after_durable_commit`.
- **Kernel nomeado pelo corpo**: `changelog_durable_commit_fate(client_set,
  client_sync, db_sync) -> ChangelogCommitFate::{Count, Skip}` em
  `changelog_kernel.rs` (+ dente AS-IS `changelog_durable_commit_fate_as_is`:
  nunca conta — todo crash paga o replay integral do WAL).
- **Trampolim**: `commit_ops_with` agora faz `match` no plano do kernel;
  o `if` data-fate saiu do trampolim (a resolução `do_sync` que alimenta
  `wal_commit_plan` permanece — é a chamada da família write-admission).
- **Teorema iff-∀**: `changelog_durable_commit_fate_fate_iff` em
  `Changelog.lean` — o destino é EXATAMENTE a resolução de sync
  (cliente vence; senão default do DB) nas 4 combinações.
- **Extrato**: `aeneas_changelog.sh --required` verde, SOURCE.changelog
  re-pinado (sha do kernel com o fn novo).
- **Planta DST**: `changelog_durable_commit_fate_on_live_client_sync_counts`
  (kernel tests; inclui asserção live-caller: `commit_ops_with` casa o
  kernel, exatamente 1 chamada do debounce, dentro do braço Count).
- **Par nasce átomo**: `catalog:changelog_durable_commit` (single_artifact,
  twin_kind atom, atom_reason datado). Gate: floor_atom 266→267,
  residuals atom 266→267, single_artifact 285→286, cap_data_fate segue 0.
- **Contador trampolim**: 75 → **74** (db.rs 53→52; captura datada abaixo).

Contador pós-P0.1 (medido 2026-09-13, pós-commit):
- `db.rs`: 52 · `concurrent.rs`: 22 · total: **74**

## P0.2 — wal_archive_delete (2º pull, db.rs `delete_wal_archives`)

- **Sítio**: `delete_wal_archives` — o `if manifest_published_seq <
  wal_archive_max_seq { return; }` que guardava a cadeia arquivada
  enquanto o publish do MANIFEST atrasava (RFC-0217 P1.1). Corpo lido:
  o keep-vs-delete da cadeia.
- **Kernel nomeado pelo corpo**: `wal_archive_delete_plan(
  manifest_published_seq, wal_archive_max_seq) -> WalArchiveDelete::
  {KeepUntilPublished, DeleteCovered}` em `changelog_kernel.rs` (+ dente
  AS-IS: deleta a janela não-publicada — única cópia durável perdida).
- **Trampolim**: `delete_wal_archives` faz `match` no plano; a comparação
  crua de seq saiu do trampolim.
- **Teorema iff-∀**: `wal_archive_delete_plan_fate_iff` em
  `Changelog.lean` — guarda sse publish < max da cadeia (∀ sobre os dois
  u64).
- **Extrato**: `aeneas_changelog.sh --required` verde, SOURCE re-pinado.
- **Planta DST**: `wal_archive_delete_plan_on_live_unpublished_window_keeps`
  (kernel tests; asserção live-caller em `delete_wal_archives`).
- **Par nasce átomo**: `catalog:wal_archive_delete`. Gate: floor_atom
  267→268, residuals atom 267→268, single_artifact 286→287,
  cap_data_fate segue 0.
- **Contador trampolim**: 74 → **73** (db.rs 52→51).

Contador pós-P0.2 (medido 2026-09-13, pós-commit):
- `db.rs`: 51 · `concurrent.rs`: 22 · total: **73**

## P0.3 — `bulk_manifest_persist` (manifest_kernel)

Sítio: `persist_bulk_manifest` (db.rs ~5924) — o portão
`if write_admission_kernel::dir_sync_required(self.sync)` decidia inline
como o publish do MANIFEST de um bulk install é pago.

- **Kernel**: `manifest_kernel::bulk_manifest_persist_fate(sync)` →
  `BulkManifestFate{PersistNow, AmortizeDebt}` — sync persiste inline
  (fsync dos SSTs + publish, dívida zerada); async amortiza
  (`bulk_manifest_debt` a cada `BULK_MANIFEST_EVERY`).
- **AS-IS dente**: `bulk_manifest_persist_fate_as_is` — amortiza para
  sempre; em modo sync a janela de publish fica aberta entre installs e
  um crash reabre inventário anterior aos acks.
- **Trampolim**: db.rs faz `match` no plano do kernel; o gate dir-sync
  sai do trampolim.
- **Teorema**: `bulk_manifest_persist_fate_fate_iff` (∀ sobre sync) em
  `Manifest.lean` — PersistNow sse sync = true.
- **Extrato**: `aeneas_manifest.sh --required` verde, SOURCE re-pinado.
- **Planta DST**: `bulk_manifest_persist_fate_on_live_sync_persists_now`
  (kernel tests; asserção live-caller em `persist_bulk_manifest`).
- **Par nasce átomo**: `catalog:bulk_manifest_persist`. Gate: floor_atom
  268→269, residuals atom 268→269, single_artifact 287→288,
  cap_data_fate segue 0.
- **Contador trampolim**: 73 → **72** (db.rs 51→50).

Contador pós-P0.3 (medido 2026-09-13): `db.rs`: 50 · `concurrent.rs`: 22 ·
total: **72** — P0 fechado (3 pulls).

### Vermelho herdado (não causado por P0.3)

`write_admission_kernel::tests::put_ok_and_recover_path_data_fate_ifs_call_kernels`
falha em HEAD limpo (85d2d6da, verificado em worktree sem as edições
P0.3): os sítios crus `open_with_env_sourced: keep_wal_archives` /
`!keep_wal_archives` / `!wal_archives.is_empty()` vieram do commit
paralelo `ded231ab` (RFC-0217 P1.1, ancestral do pai pré-goal df11b725)
sem chamada de kernel — território da sessão RFC-0217, não tocar.
`commit_ops_with: let Some(op) = records.first()` é if-let pré-existente
no mesmo teste. Documentado, não corrigido aqui.

## P1.1-a — `point_cache_validity` (lookup_kernel)

Sítios: os três portões F198/F207 de cache — `get_after_point_miss`
(fill), `get_at` (double-check hit), `last_under_user_prefix` (fill) —
decidiam inline `published_seq == snap/snapshot`.

- **Kernel**: `lookup_kernel::point_cache_validity(published, answer)` →
  `PointCachePlan{CacheCurrent, PublishAdvanced}` — fill/hit admissível
  só enquanto published == seq da resposta.
- **AS-IS dente**: `point_cache_validity_as_is` — cacheia sempre; a
  resposta pré-publish fica congelada no cache (silent-wrong F198).
- **Trampolim**: os três sítios fazem `match` no plano (4 linhas `if`
  saem do contador).
- **Teorema**: `point_cache_validity_fate_iff` (∀ sobre os dois u64) em
  `Lookup.lean`.
- **Extrato**: `aeneas_lookup.sh --required` verde.
- **Planta DST**: `point_cache_validity_on_live_publish_advanced_skips_fill`.
- **Par nasce átomo**: `catalog:point_cache_validity`. floor_atom
  269→270, residuals atom 270, single_artifact 289.
- **Contador**: 72 → **68** (db.rs 50→46; −4 sítios num pull só).

## P1.1-b — `point_tombstone` (lookup_kernel)

Sítios: os quatro portões de ponto-achado-sob-range-tombstone — `lookup`
(mem fast-path + fallback SST) e o caminho lock-free (x2) — decidiam
inline `merge::visible_at(Value, range_deleted(...))`.

- **Kernel**: `lookup_kernel::point_tombstone_plan(range_hidden)` →
  `PointTombstonePlan{ValueVisible, ShadowedDeleted}` (RFC-0150:
  tombstone com t.seq > point_seq sombreia o ponto).
- **AS-IS dente**: `point_tombstone_plan_as_is` — nunca sombreia;
  ponto range-deletado escaneia como vivo (ressurreição).
- **Trampolim**: os quatro sítios fazem `match` no plano.
- **Teorema**: `point_tombstone_plan_fate_iff` (∀ sobre o bool) em
  `Lookup.lean`.
- **Extrato**: `aeneas_lookup.sh --required` verde.
- **Planta DST**: `point_tombstone_plan_on_live_range_hidden_serves_deleted`.
- **Par nasce átomo**: `catalog:point_tombstone`. floor_atom 270→271,
  residuals atom 271, single_artifact 290.
- **Contador**: 68 → **64** (db.rs 46→42; −4 sítios num pull só).

## P1.1-c — `dir_sync_plan` (write_admission_kernel) — P1.1 fechado

Sítios: os cinco portões de dir-fsync pós-arquivo — rename SST `.tmp`
(x2 em write_imm_l0_file / cf), `sync_dir_if_required` (portão dir do
DB), `fsync_sst_paths` (conjunto com batch_is_empty aninhado no braço) e
`finish_merged_chunk_on`.

- **Kernel**: `write_admission_kernel::dir_sync_plan(sync)` →
  `DirSyncPlan{SyncDirNow, SkipDirSync}` — chama `dir_sync_required`
  (predicado segue vivo e provado no corpo do plano).
- **AS-IS dente**: `dir_sync_plan_as_is` — nunca paga; dentry do rename
  some pós-crash mesmo em sync.
- **Trampolim**: os cinco sítios fazem `match` no plano.
- **Teorema**: `dir_sync_plan_fate_iff` (∀ sobre o bool) em
  `WriteAdmission.lean`.
- **Extrato**: `aeneas_write_admission.sh --required` verde.
- **Planta DST**: `dir_sync_plan_on_live_sync_mode_pays_now`.
- **Par nasce átomo**: `catalog:dir_sync_plan`. floor_atom 271→272,
  residuals atom 272, single_artifact 291.
- **Contador**: 64 → **59** (db.rs 42→37; −5 sítios). P1.1 fechado:
  3 pares, −13 sítios.

## P1.2-a — `fence_admission` (write_admission_kernel)

Sítio: `ensure_not_fenced` — o portão `if self.durability_fenced`
decidia inline recusar fail-closed.

- **Kernel**: `write_admission_kernel::fence_admission_plan(fenced)` →
  `FenceAdmission{AdmitOps, RefuseFenced}`.
- **AS-IS dente**: `fence_admission_plan_as_is` — admite sempre;
  barreira falhada segue servindo escrita (fail-open).
- **Teorema**: `fence_admission_plan_fate_iff` (∀ sobre o bool) em
  `WriteAdmission.lean`.
- **Planta DST**: `fence_admission_plan_on_live_fenced_refuses`.
- **Par nasce átomo**: `catalog:fence_admission`. floor_atom 272→273,
  residuals atom 273, single_artifact 292.
- **Contador**: 59 → **58** (db.rs 37→36).

## P1.2-b — `fence_record` (write_admission_kernel)

Sítio: `fence_durability` — o portão `if self.fence_report.is_none()`
decidia inline se o fence novo registra o relatório.

- **Kernel**: `write_admission_kernel::fence_record_plan(has_report)` →
  `FenceRecordPlan{RecordFirst, KeepExisting}` — só o primeiro fence
  registra (janela incerta mais larga, a honesta).
- **AS-IS dente**: `fence_record_plan_as_is` — re-registra; encolhe a
  janela que o client sabe estar não-provada.
- **Teorema**: `fence_record_plan_fate_iff` (∀ sobre o bool) em
  `WriteAdmission.lean`.
- **Planta DST**: `fence_record_plan_on_live_first_fence_owns_report`.
- **Par nasce átomo**: `catalog:fence_record`. floor_atom 273→274,
  residuals atom 274, single_artifact 293.
- **Contador**: 58 → **57** (db.rs 36→35).

## P1.2-c — `group_batch_sync` (write_admission_kernel) — P1.2 fechado

Sítio: `group_prepare` — o portão
`if wal_sync_required(true, do_sync, false)` decidia inline se o batch
força a barreira do grupo.

- **Kernel**: `write_admission_kernel::group_batch_sync_plan(client_sync)`
  → `GroupSyncPlan{BatchForcesSync, BatchRidesGroup}`.
- **AS-IS dente**: `group_batch_sync_plan_as_is` — tudo viaja; client
  que pediu sync é ackado sem barreira.
- **Teorema**: `group_batch_sync_plan_fate_iff` (∀ sobre o bool) em
  `WriteAdmission.lean`.
- **Planta DST**: `group_batch_sync_plan_on_live_sync_batch_forces_group`.
- **Par nasce átomo**: `catalog:group_batch_sync`. floor_atom 274→275,
  residuals atom 275, single_artifact 294.
- **Contador**: 57 → **56** (db.rs 35→34). P1.2 fechado: 3 pares.

## P1.3-a — `parked_pair` (flush_kernel)

Sítio: `parked_oldest_pair_arcs` — o portão `if parked_unflushed.len() <
2` decidia inline entregar o par para fold.

- **Kernel**: `flush_kernel::parked_pair_plan(parked_len)` →
  `ParkedPairPlan{WaitForPair, HandOutOldestPair}` (u64 `< 2`).
- **AS-IS dente**: `parked_pair_plan_as_is` — entrega sempre; fila curta
  perde/mutila a tabela única estacionada.
- **Teorema**: `parked_pair_plan_fate_iff` (∀ sobre o u64) em
  `Flush.lean`.
- **Extrato**: `aeneas_flush.sh --required` verde.
- **Planta DST**: `parked_pair_plan_on_live_short_queue_waits`.
- **Par nasce átomo**: `catalog:parked_pair`. floor_atom 275→276,
  residuals atom 276, single_artifact 295.
- **Contador**: 56 → **55** (db.rs 34→33).

## P1.3-b — `auto_flush_gate` (flush_kernel)

Sítio: `maybe_auto_flush` — o portão `if skip_auto_flush(global_under,
cf_under)` decidia inline pular o scan inteiro.

- **Kernel**: `flush_kernel::auto_flush_gate(global_under, cf_under)` →
  `AutoFlushGate{SkipAllNotDue, ScanColumnFamilies}` (chama
  `skip_auto_flush`, que segue vivo e provado no corpo).
- **AS-IS dente**: `auto_flush_gate_as_is` — sempre scana; churn de
  flush com nada due.
- **Teorema**: `auto_flush_gate_fate_iff` (∀ sobre os dois bools) em
  `Flush.lean`.
- **Extrato**: `aeneas_flush.sh --required` verde.
- **Planta DST**: `auto_flush_gate_on_live_both_under_skips_scan`.
- **Par nasce átomo**: `catalog:auto_flush_gate`. floor_atom 276→277,
  residuals atom 277, single_artifact 296.
- **Contador**: 55 → **54** (db.rs 33→32).

## P1.3-c — `mem_auto_flush` (flush_kernel) — FECHA P1.3

Sítio: cauda de `maybe_auto_flush` — o portão `if auto_flush_due(mem,
n != 0, n as u64)` decidia inline flushar a mem agora.

- **Kernel**: `flush_kernel::mem_auto_flush_plan(mem_bytes, armed,
  limit)` → `MemAutoFlushPlan{FlushMemNow, NotDueKeepMem}` (chama
  `auto_flush_due`, que segue vivo e provado no corpo).
- **AS-IS dente**: `mem_auto_flush_plan_as_is` — nunca dispara; limite
  armado ignorado, mem cresce até o host travar.
- **Teorema**: `mem_auto_flush_plan_fate_iff` (∀ sobre
  (mem_bytes, limit, armed); prova via `bind_ok_inv`/`bind_intro`
  compondo `auto_flush_due_fate_iff`) em `Flush.lean`.
- **Extrato**: `aeneas_flush.sh --required` verde.
- **Planta DST**: `mem_auto_flush_plan_on_live_armed_over_limit_flushes`.
- **Par nasce átomo**: `catalog:mem_auto_flush`. floor_atom 277→278,
  residuals atom 278, single_artifact 297.
- **Contador**: 54 → **53** (db.rs 32→31). P1.3 fechado: 3 pares.

## P1.4-a — `pit_resync_rewrite_plan` (write_admission_kernel)

Sítio: `open_with_env_sourced` — o portão `if
pit_resync_needs_rewrite(...)` decidia inline reescrever o WAL a partir
do prefixo recuperado.

- **Kernel**: `write_admission_kernel::pit_resync_rewrite_plan
  (is_resync)` → `PitResyncRewritePlan{RewriteWalFromPrefix,
  KeepRecoveredPrefix}` (chama `pit_resync_needs_rewrite`, que segue
  vivo e provado no corpo).
- **AS-IS dente**: `pit_resync_rewrite_plan_as_is` — nunca reescreve;
  o dano mid-log sobrevive ao próximo open fail-closed.
- **Teorema**: `pit_resync_rewrite_plan_fate_iff` (∀ sobre o bool) em
  `WriteAdmission.lean`.
- **Extrato**: `aeneas_write_admission.sh --required` verde.
- **Planta DST**: `pit_resync_rewrite_plan_on_live_resync_rewrites`.
- **Par nasce átomo**: `catalog:pit_resync_rewrite_plan`. floor_atom
  278→279, residuals atom 279, single_artifact 298.
- **Contador**: 53 → **52** (db.rs 31→30).

## P1.4-b — `manifest_publish_plan` (flush_kernel)

Sítio: `persist_manifest` — o portão `if !
may_publish_manifest(sst_durable)` decidia inline segurar o publish
fail-closed.

- **Kernel**: `flush_kernel::manifest_publish_plan(sst_durable)` →
  `ManifestPublishPlan{PublishManifest, HoldUnsyncedFailClosed}` (chama
  `may_publish_manifest`, que segue vivo e provado no corpo).
- **AS-IS dente**: `manifest_publish_plan_as_is` — publica com SST
  unsynced; o CURRENT nomeia um arquivo tornado pós-crash.
- **Teorema**: `manifest_publish_plan_fate_iff` (∀ sobre o bool) em
  `Flush.lean`.
- **Extrato**: `aeneas_flush.sh --required` verde.
- **Planta DST**: `manifest_publish_plan_on_live_unsynced_sst_holds`.
- **Par nasce átomo**: `catalog:manifest_publish_plan`. floor_atom
  279→280, residuals atom 280, single_artifact 299.
- **Contador**: 52 → **51** (db.rs 30→29).

## P1.4-c — `changelog_store_plan` (changelog_kernel) — FECHA P1.4

Sítio: `changelog_store_point` — o portão `if
self.persist_manifest_durable().is_ok()` decidia inline gravar o feed.

- **Kernel**: `changelog_kernel::changelog_store_plan(publish_ok)` →
  `ChangelogStorePlan{StoreFeed, SkipStorePublishHolds}`.
- **AS-IS dente**: `changelog_store_plan_as_is` — grava com publish
  falhado; o store apaga segmentos arquivados sem cobertura publicada.
- **Teorema**: `changelog_store_plan_fate_iff` (∀ sobre o bool) em
  `Changelog.lean`.
- **Extrato**: `aeneas_changelog.sh --required` verde.
- **Planta DST**: `changelog_store_plan_on_live_failed_publish_skips`.
- **Par nasce átomo**: `catalog:changelog_store_plan`. floor_atom
  280→281, residuals atom 281, single_artifact 300.
- **Contador**: 51 → **50** (db.rs 29→28). P1.4 fechado: 3 pares.
  **P1 inteiro fechado: 12/12 pares; 285/307 = 92,83%.**

## P2.1-a — `parked_pop_plan` (write_admission_kernel)

Sítio: `take_oldest_parked` — o portão `if batch_is_empty(len)` decidia
inline entregar a tabela estacionada ao fold.

- **Kernel**: `write_admission_kernel::parked_pop_plan(parked_len)` →
  `ParkedPopPlan{PopOldestParked, NoParkedTables}` (chama
  `batch_is_empty`, que segue vivo e provado no corpo).
- **AS-IS dente**: `parked_pop_plan_as_is` — popa da fila vazia.
- **Teorema**: `parked_pop_plan_fate_iff` (∀ sobre o u64; prova via
  `bind_ok_inv`/`bind_intro` compondo `batch_is_empty_ok_iff_zero`) em
  `WriteAdmission.lean`.
- **Extrato**: `aeneas_write_admission.sh --required` verde.
- **Planta DST**: `parked_pop_plan_on_live_nonempty_queue_pops`.
- **Par nasce átomo**: `catalog:parked_pop_plan`. floor_atom 281→282,
  residuals atom 282, single_artifact 301.
- **Contador**: 50 → **49** (db.rs 29→28).

## P2.1-b — `group_ack_plan` (group_commit_kernel)

Sítio: `lone_sync_commit` — o portão `if !may_publish_group(!failed)`
decidia inline cercar o grupo (I/O de WAL falhada).

- **Kernel**: `group_commit_kernel::group_ack_plan(wal_io_ok)` →
  `GroupAckPlan{AckPublishGroup, FenceRefuseIoFail}` (chama
  `may_publish_group`, que segue vivo e provado no corpo).
- **AS-IS dente**: `group_ack_plan_as_is` — acka a falha (Ok com
  mentira, buraco 0071).
- **Teorema**: `group_ack_plan_fate_iff` (∀ sobre o bool) em
  `GroupCommit.lean`.
- **Extrato**: `aeneas_group_commit.sh --required` verde; a cópia em
  `lean/` era arquivo regular STALE (extrato de 2026-09-08) — virou
  symlink `../out/lean/GroupCommitKernel.lean` como todo kernel
  (teoremas existentes rebuildaram verde contra o extrato corrente).
- **Planta DST**: `group_ack_plan_on_live_io_fail_fences`.
- **Par nasce átomo**: `catalog:group_ack_plan`. floor_atom 282→283,
  residuals atom 283, single_artifact 302.
- **Contador**: 49 → **48** (db.rs 28→27).

## P2.1-c — `cf_flush_plan` (flush_kernel)

Sítio: `maybe_auto_flush` (loop por CF) — o portão `if !
auto_flush_due(mem_cf, true, limit)` decidia inline pular a família.

- **Kernel**: `flush_kernel::cf_flush_plan(mem_bytes, limit)` →
  `CfFlushPlan{FlushCfNow, CfNotDueSkip}` (chama `auto_flush_due` com
  armed=true — o scan chegou à família —, que segue vivo e provado no
  corpo).
- **AS-IS dente**: `cf_flush_plan_as_is` — pula toda família; CF armado
  sobre o limite só cresce.
- **Teorema**: `cf_flush_plan_fate_iff` (∀ sobre (mem_bytes, limit);
  prova via `bind_ok_inv`/`bind_intro` compondo
  `auto_flush_due_fate_iff`) em `Flush.lean`.
- **Extrato**: `aeneas_flush.sh --required` verde.
- **Planta DST**: `cf_flush_plan_on_live_over_limit_flushes` (as duas
  sondas de eixo global_under/cf_under que alimentam `auto_flush_gate`
  seguem — são computação de entrada, não portão).
- **Par nasce átomo**: `catalog:cf_flush_plan`. floor_atom 283→284,
  residuals atom 284, single_artifact 303.
- **Contador**: 48 → **47** (db.rs 27→26).

## P2.1-d — drenos (pares já provados; o `if` sai do trampolim)

6 sítios convertidos a `match` no kernel já pareado — sem par novo
(o par e o teorema existem; a composição do try_rotate_wal é o
teorema RFC-0200 `try_rotate_step_rotates_iff_pins_clear_segment_live`):

- `count_visible` ×2 — `match visible` (resultado de `visible_at`,
  par `catalog:visible_at`).
- `try_rotate_wal` ×2 — `match wal_rotate_decision(...)` +
  `match wal_segment_is_empty(...)`.
- `ensure_wal_rotated_for_gc` — `match wal_rotate_decision(...)`.
- `group_apply` — `match changelog_durable_commit_fate(true, any_sync,
  false)` (a mesma sorte do P0.1 `commit_ops_with`; o portão cru
  `wal_sync_required(true, any_sync, false)` sai).

- **Contador**: 47 → **41** (db.rs 26→19; −6 sítios, −1 comentário
  varrido no caminho? não: 26−6=20 medido 19 — o `if let Err(e)` do
  grupo recontado abaixo; ver P2.1-e).
- **Plantas**: `trampoline_drains_match_kernel_plans` (flush) + assert
  do drain no plant do changelog.
- **Vermelho causado pelo objetivo, CORRIGIDO aqui**: o plant antigo
  `wal_commit_plan_on_live_sync_fail_is_not_ok` ainda exigia
  `dir_sync_required(` em `fsync_sst_paths` — puxado pelo P1.1c
  (dir_sync_plan, 48a5efd6); assert atualizado ao shape do plano.
  Também atualizados os asserts de `group_prepare`/`group_apply` no
  plant `wal_sync_required_on_live_client_true_is_not_ok` (P1.2c
  group_batch_sync_plan + este dreno).
- **Flaky documentado**: `maybe_auto_flush_physical_cf_is_not_linear_
  in_keys` (teste de tempo-de-relógio RFC-0149) falha sob carga
  concorrente de cargo e passa 8/8 em máquina ociosa — em 80782f6c
  também só com máquina ociosa; não é regressão do objetivo.

## P2.2 pull 19 — `flusher_gate_plan` (2026-09-13)

Um plano, cinco portões: o regime workerless-vs-worker é decisão do
kernel `flusher_gate_plan(attached)` (flush_kernel.rs, enum
`FlusherGate::{Workerless, WorkerDrains}`); AS-IS
`flusher_gate_plan_as_is` diz WorkerDrains sempre — writer workerless
dorme em drain que ninguém corre (dente plantado).

Trampolim (concurrent.rs, todos os `if !flusher_attached.load` do
arquivo):

- `await_flush_debt` (453) — Workerless → return (não dorme sem drain).
- `await_l0_park` (498) — Workerless → return (park sem worker é hang).
- `submit_one` (549) — Workerless → caminho lone/`submit_after_begin`.
- `submit_inner` (672) — idem.
- `assist_flush_debt` (3282) — Workerless → return (não assiste).

Par: `catalog:flusher_gate_plan`, teorema
`flusher_gate_plan_fate_iff` (Flush.lean, ∀ sobre `attached`),
planta DST `flusher_gate_plan_on_live_workerless_parks_nowhere`
(cobre os 5 handlers por `named_fn_src`; o helper local do
flush_kernel passou a achar fns genéricas `fn name<E>`). 289/311 =
93,09%.

Atribuição de vermelho: `rfc0167_l0_stall_parks_until_worker_drains`
falha igual no pai pré-objetivo 80782f6c (isolado, 2+/2 falhas) e no
HEAD limpo — flaky de timing pré-existente, não regressão do pull
(evidência em scratch `p22_rfc0167_attribution.txt`).

## P2.2 pull 20 — `parked_debt_plan` (2026-09-13)

Um plano, dois sítios: dívida parked-unflushed é real EXATAMENTE no/acima
do cap de uma tabela — kernel `parked_debt_plan(parked, cap)`
(flush_kernel.rs, enum `ParkedDebtPlan::{DebtAtCap, NoDebtBelowCap}`);
AS-IS nunca freia (OOM slipstream 25M — dente plantado).

Trampolim (concurrent.rs):

- `await_flush_debt` (475) — NoDebtBelowCap → return com note_slept.
- `assist_flush_debt` (~3313) — NoDebtBelowCap → return.

usize→u64 no trampolim (`as u64`, precedente batch_is_empty). Par:
`catalog:parked_debt_plan`, teorema `parked_debt_plan_fate_iff`
(Flush.lean, ∀ sobre (parked, cap)), planta DST
`parked_debt_plan_on_live_at_cap_parks`. 290/312 = 92,95%.
