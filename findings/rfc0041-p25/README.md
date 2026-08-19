# RFC-0041 — TLS last-count for `deps_scan` + slice write for raftlog

2026-08-18. **No remesure.** Load still 49–100 on 12 CPUs — not comparable
to head3. Official map unchanged.

## `deps_scan`

`read_cache_epoch` (`Arc<AtomicU64>`) bumps on publish/`invalidate_read_answers`.
`count_named` keeps a thread-local last `(epoch, cf, start, end, limit, n)`.
Zipf repeats skip CF-prefix encode + count-cache mutex. A put bumps the
epoch so the next count cannot be stale.

Test: `count_named_tls_hits_then_invalidates_on_put` (8 → 8 → put → 9).

## `deps_raftlog_mc4`

`write_cf_slices` encodes puts/deletes from raw slices (one value copy,
no `WriteBatch` `String`/key clone). `CompatEngine::batch` uses it.

Test: `write_cf_slices_is_durable` (live + reopen).
