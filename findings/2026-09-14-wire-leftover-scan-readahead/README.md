# Wiring RFC-0194 leftover DONTNEED + RFC-0195 scan WILLNEED

**When:** 2026-09-14. Kernels existed; `db.rs` did not call them (RFC-0224
P0.4). Default policy on (`sst_page_keep_budget=0`): Fire-119 Drop only
when live SST bytes > WARM cap (3 GiB floor); hot stores KeepHot
(Fire 118). Compact input paths get `POSIX_FADV_DONTNEED` before unlink.
Scan walk (`scan_at_raw`) issues `WILLNEED` on adjacent-block runs when
bounded-cache.

Tests: `leftover_dontneed_fires_only_when_bounded`,
`scan_at_raw_calls_scan_readahead_window`. Darwin DIAG of overwrite_mc4
100k is **hot** (~24 MiB) — this wiring does not move that cell. Linux
25M @ 4 GiB is bounded (~6 GiB > 3 GiB) — that's the leftover cartaz.
Prefix 100M @ 4 GiB is the readahead cartaz. Meter = gate.
