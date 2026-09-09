# RFC-0180 P0.69 — skip L0-at-trigger compact while recently_multi

**When:** 2026-09-08  
**Pub:** `b2a64a75401d5914513a6daeb17f6bbba39d1697`  
**Peer:** same-class async Pedra vs Rocks `sync=false`. Always-on; no Cargo feature.

## Why

After seed, L0-at-trigger `compat_compact_once` → `install_prepared_l0_compact`
takes `db.write()` in the mc4 handoff (`active=0`, `skip_during_writes` false,
`skip_opportunistic` true but L0-at-trigger still ran). 1c moderate QPS
(100–200 µs) is not `recently_multi` — drain stays.

## Cut

`host_worker_skip_l0_during_multi` = `recently_multi(2ms)` on both L0-at-trigger
paths (opportunistic poll and the post-hysteresis drain).

## Numbers (this fire)

Unique-key DIAG 100k×50k×4: `ratio=1.218` pedra=164775 rocks=135297
`sync: false`. Rocks ≈ P0.66 133 k (not quiet-10k ≳244 k). Pedra 165 k vs
P0.66 182 k is host; 100k leftover ~12 MiB < 128 MiB so P0.68/P0.69 do
**not** fire on this DIAG. **Not a Linux cartaz win.**

Named tests: `rfc0180_skip_l0_compact_while_recently_multi`,
`host_worker_drains_l0_at_trigger_without_idle`,
`rfc0180_host_worker_skip_during_writes`.

Linux overwrite_mc4 0.557× 25M leftover+L0 still unpaid.
