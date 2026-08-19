# RFC-0041 — `SstCountCursor` borrows scan bounds

2026-08-18. **No remesure.** Load ~51–109 on 12 CPUs.

`deps_scan` miss path no longer `Bytes::copy_from_slice`s start/end for
every overlapping SST. `SstCountCursor` holds `Bound<&'a [u8]>` from the
caller (the CF-encoded window already lives for the count). Same
visibility; `last_prefix_and_count_caches_invalidate_on_put` and
`count_named_tls_*` stay green.

Official map: `findings/rfc0041-p11/head3/` (scan 1.790).
