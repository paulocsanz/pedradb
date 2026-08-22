# RFC: 0048 — Hunt fixes: compat read-path, OCC read-set, memtable scan, WAL fail-closed

**Status:** in-progress
**Updated:** 2026-08-21

## Background

- Adversarial hunt 2026-08-21 (fan-out de 7 agentes + prova dois estados no
  harness `pedradb-dst`) encontrou 7 famílias de bugs: 4 no `rocksdb-compat`
  (cache TLS cross-instância, direção/bounds do raw iterator, snapshot sem
  pino de GC + refill que engole `Err`), 3 no `pedradb-core` (commit OCC
  read-only pula read-set, scan do memtable ignora range tombstone, header
  Zero do WAL engole bloco) + 1 oracle RED pré-existente (resync do WAL que
  re-ancora engole a região danificada — `FailClosed` virava `SilentWrong`).
- Cada bug tem prova dois estados: teste falha no AS-IS, passa após o fix de
  causa única (`pedradb-dst/harness/tests/{compat_hunt,core_hunt}.rs` +
  oracle `wal_crc_flip_is_fail_stop_or_clean`).
- Detalhes por achado: `determinismo/pedradb-dst/findings/F165..F171-*.md`.

## Problems This Solves

- **Problem:** duas instâncias de DB na mesma thread partilham respostas de cache TLS (valor/contagem de A servidos por B).
- **Problem:** raw iterator anda para trás no `next()` pós-seek reverso e `prev()` ignora lower bound — divergência do rust-rocksdb.
- **Problem:** `DB::snapshot()` do compat é sequência nua; com `auto_reclaim` default (perfil Rocks) o snapshot vivo morre (`SnapshotTooOld`) e o iterator termina truncado em silêncio.
- **Problem:** commit OCC read-only devolve `Ok(())` sem validar o read-set — conflito e `SnapshotTooOld` indetetáveis (contrato do próprio `occ.rs`).
- **Problem:** `MemTable::range_snapshot` ignora range tombstones (`get` = Deleted, scan emite a chave).
- **Problem:** header Zero+len0 do WAL descarta o resto do bloco sem validação; e o resync que re-ancora perde a região danificada em silêncio — ambos violam o contrato fail-closed do `WalRecovery`.

## Proposed Solution

- Identidade global por instância no cache TLS do compat (`cache_epoch_base`).
- Flag de direção + clamp de bounds no raw iterator; `SnapshotPin` no `Snapshot`; `status()` no iterator (refill registra `Err`).
- Validar read-set no commit OCC vazio (espelha `lone_commit`).
- Filtrar `range_deleted` no scan do memtable.
- Fail-closed nos dois buracos do WAL: Zero-header com cauda não-zero (só a alignment fresca; dentro de walk é lixo do próprio walk) e resync re-ancorado reportado (`resync_origin`) — `FailClosed` recusa, `PointInTime` reporta `kind: "resync"`.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** compat: cache TLS com identidade por instância (F165) — status: `done`
- [x] **P0.2** compat: direção/bounds do raw iterator (F166) — status: `done`
- [x] **P0.3** compat: `SnapshotPin` + `status()` no iterator (F167) — status: `done`
- [x] **P0.4** core: commit OCC read-only valida read-set (F168, incl. paridade do teste compat `txn_snapshot_hides_later_writes`) — status: `done`
- [x] **P0.5** core: `range_snapshot` filtra range tombstones (F169) — status: `done`
- [x] **P0.6** core: WAL Zero-header fail-closed a alignment fresca (F170; torn tails não regrediram) — status: `done`
- [x] **P0.7** core: WAL resync re-ancorado reportado; `FailClosed` recusa (F171; oracle RED verde + `point_in_time_reports_resync_reanchor`) — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Reparo de WAL mid-log: rewrite do WAL a partir dos records recuperados (hoje o PIT reporta mas o dano permanece; reopen fail-closed recusa para sempre) — status: `done` (2026-08-22; PIT com kind `resync` reescreve o WAL e o reopen fail-closed volta limpo — `point_in_time_reports_resync_reanchor`)
- [x] **P1.2** Jornal do `resync`/Zero-header no `CORRUPTLOG` com offset exato do início do dano (o `resync_origin` já existe; falta o consumo no `escalate_or_fail` usar kind próprio) — status: `done` (2026-08-22; kinds `resync` + `zero_header`, `CoreError::WalZeroHeader` tipado em vez de string-match — `zero_header_journals_and_pit_reports`)
- [x] **P1.3** Residuais do fan-out com prova pendente — status: `done` (F172 k7, F173 k8 no wave anterior; F174 fold×materialize k9, F175 checkpoint history tier k10 e F176 archive Err swallow k11 provados e corrigidos neste wave — cada um dois estados + ficha própria)
- [x] **P1.4** Paridade Rocks a documentar: `scan_count`/`raw_iterator_opt` fora do read-set OCC (F168-adjacente; decidir política e escrever no doc do txn) — status: `done` (doc-only; política guard-key documentada no header de `txn.rs` e nos métodos — tracking de scans fica como P2 se um caller precisar)

### P2 — later / polish
- [x] **P2.1** Fuzz de framing do WAL com o painel `explode_choices` + os dois novos kinds (`ZeroHeaderTail`, resync re-anchor) no sweep de corrupção — status: `done` (2026-08-22; `ZeroTail` + `ForgeZeroHeaderAlive` no painel, sweep afirma FailStop tipado `WalZeroHeader` e o re-anchor `resync_origin`; dentes verificados neutralizando o check F170: sweep falha `expected FailStop, got Ok([])`)
- [x] **P2.2** `blocks_overlapping_range` simétrico com `blocks_for_point` sob user-key split (leitor defender-se do writer futuro) — status: `done` (2026-08-22; partição `< s` + guarda `hi >= s`; prova unitária dois estados com índice hand-made, controles sem split idênticos)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | compat TLS cache cross-instância (F165) | done | hunks em `rocksdb-compat/src/lib.rs` | 2026-08-21 |
| P0.2 | p0 | compat raw iterator direção/bounds (F166) | done | `rocksdb-compat/src/shape.rs` | 2026-08-21 |
| P0.3 | p0 | compat snapshot pino + status (F167) | done | `rocksdb-compat/src/lib.rs` | 2026-08-21 |
| P0.4 | p0 | OCC read-only valida read-set (F168) | done | `pedradb-core/src/occ.rs` + teste compat | 2026-08-21 |
| P0.5 | p0 | memtable scan range tombstone (F169) | done | `pedradb-core/src/memtable.rs` | 2026-08-21 |
| P0.6 | p0 | WAL Zero-header fail-closed (F170) | done | `wal/reader.rs` + `wal/recover_kernel.rs` | 2026-08-21 |
| P0.7 | p0 | WAL resync re-ancorado reportado (F171) | done | `wal/reader.rs` + `wal/mod.rs` + `db.rs` | 2026-08-21 |
| P1.1 | p1 | WAL rewrite p/ dano mid-log (reopen FC limpo) | done | `db.rs` + `point_in_time_reports_resync_reanchor` | 2026-08-22 |
| P1.2 | p1 | kinds `resync`/`zero_header` no CORRUPTLOG | done | `error.rs`/`wal/reader.rs`/`db.rs` + `zero_header_journals_and_pit_reports` | 2026-08-22 |
| P1.3 | p1 | residuais do fan-out | done | F172 (k7), F173 (k8), F174 (k9), F175 (k10), F176 (k11) | 2026-08-22 |
| P1.4 | p1 | paridade Rocks: scans no read-set OCC | done | doc em `rocksdb-compat/src/txn.rs` | 2026-08-22 |
| P2.1 | p2 | fuzz framing WAL (novos kinds) | done | `wal/recover_choose.rs` + `tests/recover_choose.rs` | 2026-08-22 |
| P2.2 | p2 | simetria blocks_for_point/range | done | `sst/table.rs` + teste unitário dois estados | 2026-08-22 |

## Acceptance Criteria

- **Tests:** `pedradb-core --lib` (380 passando — incluindo `point_in_time_reports_resync_reanchor`, `zero_header_journals_and_pit_reports`, `torn_tail_*`, theorem do recover kernel, sweep `explode` com os kinds novos e a simetria de blocos; as 3 falhas remote-tier da sessão paralela foram resolvidas por eles), `rocksdb-compat` (43, incl. `txn_snapshot_hides_later_writes` atualizado), harness `compat_hunt` (7) + `core_hunt` (10, k1..k4b + k7..k11) + oracle `wal_crc_flip_is_fail_stop_or_clean` — todos verdes com os fixes; os 17 de hunt falham sem eles.
- **Telemetry / Analytics:** none — correção de corretude; o `CORRUPTLOG` (RFC-0038) recebe eventos `resync` e `zero_header` (P1.2).
- **Documentation:** este RFC + fichas F165–F176 em `determinismo/pedradb-dst/findings/` + LEDGER do hunt 2026-08-21.
- **Screenshots:** backend-only.

## Out of scope

- Fix dos residuais não provados do fan-out (P1.3 lista; cada um exige prova dois estados própria antes).
- Redesign do recovery PIT para reparar dano mid-log in-place (P1.1 é o rewrite mínimo).
- LSM bottom-level tombstone drop (merge.rs) — dependente de caller, sem prova.
