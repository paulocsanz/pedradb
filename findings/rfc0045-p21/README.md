# RFC-0045 P2.1 — memtable apply after durable fsync (2026-08-24)

Correctness slice. **Not** a quiet 3× remesure. **Not** a concurrent
skiplist. Official Rocks peer remains `WriteOptions.sync=false`.

## What shipped

`ConcurrentDb::finish_group_off_lock` used to apply the memtable under the
first write-lock hold, drop the lock, `fdatasync`, then publish. That
violated `Db::group_finish` ("mem apply after WAL is durable") and left
unpublished versions in the memtable if fd failed.

New order (matches `group_finish`):

1. Write lock: OCC validate, WAL encode, absorb extras, stage `unapplied`.
2. Drop lock. `write_pending_frame` + `fdatasync`. WAL mutex released
   **before** the apply lock (no deadlock with a follower blocked on
   `wal.lock()`).
3. Write lock: `group_apply` + `publish_sequence`.

Apply is still serialized. RFC-0055 P1.1 (concurrent insert) stays gated
— P0.3 of that RFC did not obligate a skiplist.

## OCC

During the off-lock fd window the write lock is free but the memtable
does not yet hold the assigned seqs.

- `unapplied` is consulted by `key_has_write_after` (first-committer-wins).
- `occ_snapshot` returns `published` while `commit_inflight > 0`. A snap
  equal to an unapplied seq would miss on `get_at` and fail to conflict
  (`seq > snap` is false when equal).

## Tests (all green, `--test-threads=1`)

- `encode_offlock_matches_lock_path` — recovered gets match `Db::group_commit`
- `sync_fail_does_not_apply_before_publish` — fd fail unstages; recover
  still sees the frame if it hit the file (fence case B)
- `occ_snapshot_pins_published_while_commit_inflight`
- `unapplied_ops_are_visible_to_occ_not_get`
- `async_concurrent_writers_recover`, `async_and_sync_concurrent_writers_recover`
- `resume_after_fence_reports_uncertain_range`
- `write_group_amortizes_apply_still_locked` (groups < submits; apply still
  a write-lock hold)
- full `concurrent::` 59/59, `occ::` 10/10

## Throughput residual

P0.2 arithmetic still stands: pulling 0.5 µs of mem-apply off a 1.6 µs
hold is **~+15%**, not 5×. This turn did **not** re-run the quiet 3×
árbitro for `kvrocks_set_mc50` / `deps_lock_prewrite`. Do not cite a
measured win from this finding.

The RFC-0044 async 5× floor remains open. Arena/skip-list is still out of
scope until P2.1 saturates *and* that floor is still missing (RFC-0045
out-of-scope; RFC-0055 P1.1 gate).
