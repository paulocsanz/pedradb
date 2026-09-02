# Path to ≥1× hydrate on the guest (official)

**Peer:** Rocks default `sync=false`. Guest CHV `linux-gate-p149b`, 3.9 GiB.
**Now:** v46 25M hydrate 45.6 s (0.55 M/s) vs Rocks 29.4 s (0.85 M/s) = **0.65×**.
**Need:** 45.6 s → **< 29.4 s** (cut ≥16 s, ~35 %).

## Where the 45.6 s is

Run #26 FLUSHSTAGES (same guest core, 25M) is the last split we have:

| stage | time | share of SST |
|---|---:|---:|
| encode | 19.3 s | 55 % |
| lz4 | 9.2 s | 26 % |
| bloom | 3.6 s | 10 % |
| crc | 1.5 s | 4 % |
| write | 3.9 s | 11 % |
| **SST sum** | **37.5 s** | |

v46 skipped WAL+mem (BulkRun). 45.6 − 37.5 ≈ **8 s envelope** (apply + MANIFEST).
Encode on this core is **272 MiB/s** (M-series was 845 MiB/s). Payload is
xorshift slices from a 1 MiB pool — almost incompressible.

Value memcpy is not the 19 s: 200 B × 25 M at a few GB/s is <1 s. Encode is
**~770 ns/entry of overhead** (InternalKey, sort check, bloom hash, Result,
block split) around a 40 ns memcpy.

## Arithmetic of ≥1×

Cuts that **cannot** close 16 s alone:

- lz4 off: −9.2 s, write +~2 s (more bytes) → net **~7 s**. Lands ~0.77×.
- bloom off: −3.6 s. Lands ~0.70×.
- both, encode untouched: **~32 s vs 29.4 s = 0.90×. Still lose.**

Stack that **does** close, conservative:

| cut | save | notes |
|---|---:|---|
| Tight-loop SST fill from `(key,val,seq)` arrays (no InternalKey / Result / sort-check per entry) | 7–11 s | encode 19.3 → 8–12 s |
| Bulk files **v3 uncompressed** (lz4 off) | ~7 s net | pool is random; lz4 pays 9.2 s for ~10 % disk |
| No bloom on bulk chunks (rebuild at settle if reads need it) | 3.6 s | hydrate timer is apply-only |
| **sum** | **18–22 s** | 45.6 → **24–28 s** vs Rocks 29.4 s = **1.05–1.23×** |

If the tight loop only saves 4 s, the stack misses. Encode is the must-hit.

## What will not get us there

- Darwin / APFS numbers
- `write_cf_owned` only (slipstream is `write_opt`)
- 16 KiB blocks (run #14: 0.18 M/s)
- Async flush on 1 core (worker is the long pole; overlap saves the fill, not the 19 s encode)
- MANIFEST batching (seconds, not 16 s)
- C lz4 while keeping compression: maybe 4–5 s, not enough without the encode cut

## Sequence

1. One guest 25M with `PEDRA_FLUSH_STAGES=1` on **v46** (confirm the 37.5 / 8 split still holds under BulkRun).
2. Bulk writer: one pass `keys/vals/seqs` → `block_buf` (trusted sorted).
3. Bulk SST v3 (no lz4). Disk still below Rocks hydrate (8.2 GiB).
4. Bloom optional on bulk; settle already 1.3 s vs 6.2 s.
5. If 0.9×: pipeline encode of file N+1 vs `write()` of file N (only if the VM actually has a second core).

100M is a RAM problem (resident SST bodies), not the ≥1× CPU problem. Payload-drop + 256 MiB chunks is that track; it does not make 25M faster.