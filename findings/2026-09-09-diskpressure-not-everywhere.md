# 2026-09-09 — DiskPressure is write admission, not every disk fn

Grind leftover_next named the next production fn; each fire slapped
`compact_refuse` at the top + include_str + DST. Same factory as leftover
`is_empty` wrap (user already banned that).

## Contract

`CoreError::DiskPressure` is for a **new user/ops write** that would append
WAL or write SST/dest: `put` / `delete` / `apply_batch` / `flush` /
`compact*` / PITR dest / replica append. Callers of those APIs already
handle `CoreError`.

Not:

- `close` — takes `self`; Err drops the handle; Drop only unlocks; caller
  cannot retry. Shutdown is not write admission.
- `rotate_wal_now` — runs **after** SST is durable (G1). Must persist
  MANIFEST + new WAL. DiskPressure here makes `flush()` fail after L0
  exists.
- `persist_manifest_durable` — callee of rotate; post-SST persist.
- `compact_vlog_promote` — finishes staged GC (`.new` already written).
  Refuse leaves `vlog_use_new` stuck; recovery path calls promote first.
- `auto_flush_mem` — `put` already admitted; F18 `maybe_auto_flush_best_effort`
  swallows the error anyway.

## Kept

Entry admits on `flush` / `flush_cf` / `compact_with` / `compact_leveled` /
`compact_with_ssts_only` / `compact_ssts_only_cf` + kernel `compact_refuse`
those match. Probe-Err unknown. ConcurrentDb apply_batch typing. Restore
history dest.
