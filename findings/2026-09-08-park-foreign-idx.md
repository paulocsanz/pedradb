# RFC-0180 P0.68 — park foreign one-slash leftover (O(1))

**When:** 2026-09-08  
**Peer:** Pedra async vs Rocks `sync=false`. Always-on; no Cargo feature.

## Why

P0.66 split the tail **index** (`c/` vs `ycsb/`). Flush still parked the
**whole** default memtable. Linux overwrite 25M leftover is ≈ write
buffer of `ycsb/` then timed `c/` puts mix and dump together.

## Cut

Before apply, if the new key is a one-slash family not yet in live mem
and `approx_bytes >= auto_flush/2`, O(1) `stage_flush_imm` / park the
leftover. Darwin 100k leftover (~12 MiB) < 128 MiB — does **not** fire.
Linux leftover ≈ 256 MiB does.

## Numbers (this fire)

Unique-key DIAG 100k×50k×4: `ratio=0.383` pedra=59229 rocks=154497
`sync: false`. Pedra 59 k vs earlier 182 k = host/disk stall (Avail was
195 MiB). **Not a quiet win.** Rocks 154 k < quiet-10k ≳244 k.

Named tests: `rfc0180_park_foreign_idx_decision`,
`rfc0180_park_foreign_idx_on_new_slash_prefix`.

Linux 0.557× 25M still unpaid.
