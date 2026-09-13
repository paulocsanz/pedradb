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
