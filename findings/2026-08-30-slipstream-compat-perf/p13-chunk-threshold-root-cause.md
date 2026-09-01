P1.3 root cause: bulk chunks staged at the GLOBAL auto-flush cap, not the per-CF write buffer
=====================================================================================
2026-09-01. Symptom: bench sets data CF `write_buffer_size` = 256 MiB
(`DATA_WRITE_BUFFER_BYTES = 256 << 20` in the vendored
`src/snapshot_pedradb.rs`) yet bulk chunks are ~62.8 MiB local / ~77 MiB
guest (73 chunks per 25M).

Chain (all verified in code + a discriminating experiment):
1. compat `open_cf_descriptors` DOES land per-CF values:
   `set_cf_write_buffer("data", 256MiB)` / `("meta", 8MiB)`
   (rocksdb-compat/src/lib.rs:1931-1933), and `open_cf_inner` sets the
   GLOBAL `core_opts.auto_flush_bytes = Some(opts.write_buffer_size)` from
   the DB-level `Options` (lib.rs:2121-2124) — the bench never sets a
   DB-level buffer, so compat's `Options::default()` = **64 MiB** wins
   (lib.rs:339).
2. `Db::maybe_auto_flush`'s per-family walk (db.rs ~9069) would honor
   256 MiB — but it never drives chunking for this workload: the walk
   only parks when a family crosses ITS limit, and the chunk producer is
   not the walk. It is the HOST FLUSH WORKER: `try_stage_if_full`
   (concurrent.rs:1711-1724) stages the shared active mem into imm when
   `active_mem_usage() >= auto_flush_threshold()`, and
   `Db::auto_flush_threshold()` returned ONLY the global
   `auto_flush_bytes` (db.rs, pre-fix) — per-CF overrides were invisible
   to it. `park_imm_once` then parks the staged imm (concurrent.rs:2325),
   and the RFC-0159 bulk path installs it at L3 → one chunk per stage.
3. So chunk size = global 64 MiB cap + worker poll-interval overshoot
   (62.8 MiB local fast writes, 77 MiB guest) — the 256 MiB per-CF value
   never participated.

Discriminating experiment (/tmp/cfbuf-probe, bench-shaped: ascending
1024-op batches, 200 B values, compat open_cf_descriptors, DB-level
buffer 1 MiB, data CF 16 MiB):
- PRE-FIX: 22.4 MiB written → 4 install_parked (~4.7 MiB each — 1 MiB
  cap + poll overshoot; neither 16 MiB nor 1 MiB exactly).
- POST-FIX: 1 install_parked (16 MiB data-CF limit) + 1 install_flush.

Fix (db.rs, this commit): `auto_flush_threshold()` now returns
max(global, max per-CF override) — the same "whichever one parks tables"
semantic `flush_debt_cap` already documented; `flush_debt_cap` now simply
delegates to it. `flush_worker_tick`'s parked-memory bound uses the same
fn, so backpressure grows coherently with the bigger threshold. Per-CF
limits for SMALLER families stay enforced by the `maybe_auto_flush` walk
(the global guard trips early when any CF value is small; the walk then
parks a family that crosses its own limit).

Expected bench effect (to verify on the guest as the next injection):
25M → chunks 64→256 MiB → ~73 → ~21 installs; fewer L3 files (run #19:
fewer/bigger files improved probe/get legs); fewer per-chunk fixed costs
(manifest persist, file create, bloom) during hydrate. One change per
guest run: v24 (in flight, no threshold fix) measures the P1.1
materialize cut; the threshold fix goes in as v25.
