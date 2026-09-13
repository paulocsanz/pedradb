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
