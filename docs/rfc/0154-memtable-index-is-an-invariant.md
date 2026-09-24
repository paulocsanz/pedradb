# RFC: 0154 — Índice do memtable é invariante do insert

**Status:** in-progress (P0/P1 done including P1.8; P1.7/P1.9/P2.1a/P2.1b/P2.1/P2.2/P2.3/P2.4/P2.5/P2.6 REFUSED)
**Updated:** 2026-08-30
**Parents:** [0153](0153-ram-scale-block-cache-bytes.md) P1.1,
[0149](0149-majority-3x-async.md) (apply 2× via `idx_stale`),
[0002](0002-internal-key-memtable.md) (arena recusada até o write path pedir),
[0065](0065-physical-column-families-one-wal.md) P1 (memtable por CF)
**Evidence:** lazy-idx CHV
[`findings/2026-08-29-linux-p149-p21-chv/`](../../findings/2026-08-29-linux-p149-p21-chv/)
`kvrocks_scan` 0.070× / mvcc 0.208×. After P0–P1.8:
[`findings/2026-08-30-linux-p149-p21-chv-p18/`](../../findings/2026-08-30-linux-p149-p21-chv-p18/)
CHV **12/17 PASS** (0149 P2.1). P1.8 intern closed `ycsb_f` + lock.

## Background

- O memtable do Pedra é um `Vec` (`tail`) com índice à parte (`tail_idx`).
  Rocks: a skiplist **é** o memtable (um por CF). Insert já deixa o dado
  procurável.
- RFC-0149 comprou apply 2× com `insert_many` hint≥16 **sem** `tail_idx`
  (`idx_stale`). `write_log` cobriu só o count do CF `write` (`deps_scan`).
  O resto das leituras reconstrói ordem a partir do log.
- CHV oficial 17 shapes, árvore com lazy idx: apply min 2.96 / lock 3.82 /
  `deps_scan` 4.76; `kvrocks_scan` Pedra ~2.5k qps vs Rocks ~34k;
  `deps_mvcc_latest` ~45k vs ~220k. O wall do mvcc é um rebuild O(n) de
  ~256k no primeiro `last_visible`. O wall do scan é sort+filter do tail
  inteiro **por** SCAN (`iter_internal_iter` quando stale).
- `count_latest_in_range` recusa prefixo vazio (`!a.is_empty()`), então
  kvrocks/YCSB default-raw nunca usam o count indexado.

## Problems This Solves

- **Problem:** `idx_stale` é um segundo modo do engine. Scan/get/latest
  deixam de ser O(log n) depois de qualquer apply ou pipeline ≥16.
- **Problem:** três índices no mesmo `Vec` (`tail_idx` vivo, `idx_cache`
  rebuild, `write_log`) — atalho de shape, não invariante.
- **Problem:** count/scan indexados exigem CF prefix não-vazio; default-raw
  (kvrocks split) cai no merge O(n).

## Proposed Solution

- Depois de qualquer insert, `tail_idx` está completo. Não existe modo stale.
- `insert_many` indexa cada key (HashMap `point` em lock/default/write,
  BTree no prefixo vazio / raftlog). Sem mutex no write path.
- Apagar `idx_stale`, `idx_cache`, `ensure_idx_cache`, `write_log`.
- Count/scan/last usam o shard vivo. Prefixo vazio (um shard) é um count
  indexado, igual ao CF `write`.
- `last_visible_under_prefix` com NUL no prefixo considera só o shard
  daquele CF (mvcc não atravessa lock+default+write).

## Delivery slices (mandatory)

### P0 — must ship first (índice vivo; scan default-raw deixa de ser replay)

- [x] **P0.1** `insert_many` sempre atualiza `tail_idx`; apagar `idx_stale` /
      `idx_cache` / `ensure_idx_cache` — status: `done`
- [x] **P0.2** Apagar `write_log`; count do CF `write` usa o shard vivo —
      status: `done`
- [x] **P0.3** `count_latest_in_range` + `iter_internal_iter_at` no prefixo
      vazio quando há um único shard (kvrocks/YCSB raw) — status: `done`

### P1 — next wave

- [x] **P1.1** `last_visible_under_prefix`: prefixo com NUL pinado no shard
      do CF — status: `done`
- [x] **P1.2** CF `write` fica **ordenado** (BTree `short`), não HashMap
      `point` — MVCC latest é reverse-seek. `lock`/`default` continuam
      HashMap — status: `done`
      Tests: `write_cf_ordered_last_and_count_match_gets`;
      `apply_batch_write_cf_last_matches_get`.
- [ ] **P1.3** Prefixo vazio HashMap — status: `todo`
      **REFUSED** CHV 6/17 min 0.458 (`ycsb_c` lost 3×, e min 2.37).
      Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p13/`](../../findings/2026-08-30-linux-p149-p21-chv-p13/).
- [x] **P1.4** 1c put não toma mutex em `tail_ord` — só `AtomicBool` stale.
      `cached_tail_order` reconstrói no primeiro scan — status: `done`
- [x] **P1.5** 1c put invalida TLS last-get **por key**, não epoch global.
      `read_cache_epoch` continua a bumpa para LAST_COUNT. Fat apply /
      range ainda epoch-bumps o point TLS. — status: `done`
      Tests: `last_get_survives_put_of_other_key`;
      `last_get_other_thread_sees_put`; `key_gen_prefixed_matches_encoded`.
- [x] **P1.6** 1c async put: sem `Vec<BatchOp>`/`Vec<WriteOp>`; submit
      não chama `SystemTime` (idle = `active` + `last_complete`). — status:
      `done`
      Test: `lone_async_one_put_is_visible`.
- [ ] **P1.7** YCSB-F RMW last-byte sem Vec — status: `todo`
      **REFUSED** CHV 9/17 min 0.791 (`ycsb_f` median 2.74, r3 0.791;
      `ycsb_a` lost 3×). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p17/`](../../findings/2026-08-30-linux-p149-p21-chv-p17/).
- [x] **P1.8** Put TLS write-through intern the payload (refcount, not a
      second `copy_from_slice`) — kvrocks SET / YCSB repeats. — status:
      `done`
      Test: `intern_put_value_shares_repeat_payload`.
- [ ] **P1.9** 1c put sem mutex `dirty_points` — status: `todo`
      **REFUSED** CHV 10/17 min 0.940 (`ycsb_f` lost 3×, lock 3.46→2.18).
      Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p19/`](../../findings/2026-08-30-linux-p149-p21-chv-p19/).

### P2 — later

- [ ] **P2.1a** Versões do tail vivem no shard do CF (`TailShard.versions`),
      não num `Vec` partilhado. — status: `todo`
      **REFUSED** CHV 10/17 min 1.017 (lost `ycsb_f` 3× and lock 3.46→1.89).
      Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p20/`](../../findings/2026-08-30-linux-p149-p21-chv-p20/).
- [ ] **P2.1b** `MemTable` vivo por CF não-default no `Db` — status: `todo`
      **REFUSED** CHV 10/17 min 0.934 (lost `ycsb_f` + lock; raftlog min
      below 1.0). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p21/`](../../findings/2026-08-30-linux-p149-p21-chv-p21/).
- [ ] **P2.1** Raftdb segundo `DB` — status: `todo`
      **REFUSED** CHV 10/17 min 1.234 (lost `ycsb_f` + lock; raftlog
      1.08→1.24 not 3×). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p22/`](../../findings/2026-08-30-linux-p149-p21-chv-p22/).
- [ ] **P2.2** Arena / skip-list — status: `todo`
      **REFUSED** gate miss: memtable insert is 0.12 µs/entry (apply 15 µs
      of 65.7 µs p50 = 23%). Vec tail is already O(1); skiplist is Rocks'
      concurrent-writer structure and cannot close apply 3×. Evidence:
      [`findings/2026-08-30-p22-memtable-not-cpu-bound/`](../../findings/2026-08-30-p22-memtable-not-cpu-bound/).
- [ ] **P2.3** WAL/prepare of apply's two 64-key batches (the ~77% of
      apply p50 that is not memtable). Do not skip WAL. — status: `todo`
      **REFUSED** CHV 9/17 min 0.286 (lost `ycsb_f` 3× and lock 3.46→2.68;
      apply 1.88→1.75). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p23/`](../../findings/2026-08-30-linux-p149-p21-chv-p23/).
- [ ] **P2.4** Remaining coluna A under 3× after intern: SET / blob / cache
      (not another apply-prepare cut; P2.3 measured that leftover). —
      status: `todo`
      **REFUSED** CHV 9/17 min 0.962 (lost `ycsb_f` 3×, lock 3.46→1.87,
      pipeline 3.75→2.68; SET 2.46→1.87 did not close). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p24/`](../../findings/2026-08-30-linux-p149-p21-chv-p24/).
- [ ] **P2.5** 1c SET/blob WAL encode: skip vlog mutex (async never spills);
      one-shot v1 emit for a 1-op Full fragment. Do not skip WAL. — status:
      `todo`
      **REFUSED** CHV 10/17 min 1.048 (lost `ycsb_f` 3× and lock 3.46→2.35;
      SET 2.46→2.76 did not close). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p25/`](../../findings/2026-08-30-linux-p149-p21-chv-p25/).
- [ ] **P2.6** Write-only 1c publish skips empty read-cache mutexes
      (`AnswerCache::used` flag: `clear`/`invalidate_many` no-op until any
      insert). Do not skip WAL. Do not skip `dirty_points`. `CountCache`
      stays locked (F204 in-flight reader). — status: `todo`
      **REFUSED** CHV 9/17 min 0.951 (lost `ycsb_f` 3.06→2.34, lock
      3.46→2.34, mvcc 3.96→2.36; raftlog min < 1.0 floor; SET 2.46→1.92
      worse, blob/cache flat). Reverted. Evidence:
      [`findings/2026-08-30-linux-p149-p21-chv-p26/`](../../findings/2026-08-30-linux-p149-p21-chv-p26/).

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | live idx, no stale | done | memtable.rs insert_many | 2026-08-29 |
| P0.2 | p0 | fold write_log | done | count uses tail_idx write shard | 2026-08-29 |
| P0.3 | p0 | empty-prefix count | done | count_latest_in_range + iter_at | 2026-08-29 |
| P1.1 | p1 | last_visible CF pin | done | last_visible_under_prefix | 2026-08-29 |
| P1.2 | p1 | write CF stays ordered | done | write BTree; lock/default HashMap | 2026-08-30 |
| P1.3 | p1 | empty prefix HashMap | todo | REFUSED CHV 6/17; reverted | 2026-08-30 |
| P1.4 | p1 | 1c put no tail_ord mutex | done | tail_ord_stale atomic | 2026-08-30 |
| P1.5 | p1 | 1c put TLS per-key inval | done | KeyGenMap + point_tls_epoch | 2026-08-30 |
| P1.6 | p1 | 1c async put no Vec+clock | done | commit_async_one; CHV a 3.19 | 2026-08-30 |
| P1.7 | p1 | RMW last-byte no Vec | todo | REFUSED CHV 9/17 min 0.791; reverted | 2026-08-30 |
| P1.8 | p1 | put TLS intern repeat payload | done | intern_put_value; CHV 12/17 | 2026-08-30 |
| P1.9 | p1 | 1c put no dirty_points mutex | todo | REFUSED CHV 10/17 min 0.940; reverted | 2026-08-30 |
| P2.1a | p2 | per-CF tail versions | todo | REFUSED CHV 10/17 min 1.017; reverted | 2026-08-30 |
| P2.1b | p2 | live MemTable per non-default CF | todo | REFUSED CHV 10/17 min 0.934; reverted | 2026-08-30 |
| P2.1 | p2 | raftdb second DB | todo | REFUSED CHV 10/17 min 1.234; reverted | 2026-08-30 |
| P2.2 | p2 | arena iff still bound | todo | REFUSED gate: mem 23% of apply p50 | 2026-08-30 |
| P2.3 | p2 | apply WAL/prepare 2×64 | todo | REFUSED CHV 9/17 min 0.286; reverted | 2026-08-30 |
| P2.4 | p2 | SET/blob/cache after intern | todo | REFUSED CHV 9/17 min 0.962; reverted | 2026-08-30 |
| P2.5 | p2 | 1c WAL encode no vlog mutex | todo | REFUSED CHV 10/17 min 1.048; reverted | 2026-08-30 |
| P2.6 | p2 | publish skips empty read-cache mutex | todo | REFUSED CHV 9/17 min 0.951; reverted | 2026-08-30 |

## Acceptance Criteria

- **Tests:** `insert_many_keeps_live_idx`; `insert_many_empty_prefix_count_matches_gets`
  (oracle = `get` das keys escritas, count `Some`); `last_visible_pins_write_shard_after_apply`;
  `apply_batch_raw_keys_count_visible_matches_scan` (shipped `apply_batch` +
  `count_visible` vs `scan_at_raw`). P1.5: `last_get_survives_put_of_other_key`;
  `last_get_other_thread_sees_put`; `key_gen_prefixed_matches_encoded`.
  P1.6: `lone_async_one_put_is_visible`. P1.8: `intern_put_value_shares_repeat_payload`.
- **Telemetry / Analytics:** none — correctness/CPU no memtable; remedir CHV
  é RFC-0149 P2.1, não este slice.
- **Documentation:** este RFC; 0153 P1.1 aponta para cá; open-items uma linha.
- **Screenshots:** backend-only.

## Out of scope

- Remedir as 17 oficiais no CHV (0149 P2.1). Arena neste slice. Memtable
  por CF (0065 P1). Desligar o índice de novo para recuperar apply 2×.
  Publicar crates.io.
