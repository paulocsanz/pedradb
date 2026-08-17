# RFC-0041 — inline-first MemTable + pooled CF encode (not remesured)

2026-08-17. Write-path CPU: first mem version is inline (no `Vec` per
distinct key); `DB::write` encodes CF prefixes into a thread-local
`BytesMut` (one backing alloc per batch). G1/G6/G8 unchanged.

A deps-only probe ran on this box at **load ~80** (unrelated fuzzers).
apply_mc4 Pedra **754** is **void** — not a product number, not a reject.
Do not compare it to parkfold2 4.3 k. Official 16-shape median-of-3
waits for a quiet box. FLOOR off.
