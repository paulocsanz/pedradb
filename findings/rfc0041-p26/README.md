# RFC-0041 — TLS last-get for YCSB-C

2026-08-18. **No remesure.** Load ~70–100 on 12 CPUs.

`get_named` (YCSB-C / default CF) keeps a thread-local last
`(epoch, cf, key, value)`. Zipf repeats skip CF-prefix encode + point-cache
mutex. `read_cache_epoch` bumps on publish so a put cannot leave a stale get.

Test: `get_named_tls_hits_then_invalidates_on_put` (`v1` → `v1` → put → `v2`).

Official quiet map remains `findings/rfc0041-p11/head3/` (C 1.796).
