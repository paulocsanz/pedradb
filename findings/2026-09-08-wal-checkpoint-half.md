# RFC-0180 P0.76 — 1c checkpoint at write-buffer/2 — REVERTED

**When:** 2026-09-08
**Meter:** `/tmp/pedra-vs-rocks-p079` leftover+L0 2M×50k×4
**Pub:** reverted uncommitted

## Hypothesis

P0.70 checkpoints 1c WAL at ≥ 2× write-buffer (512 MiB at compat 256 MiB).
Leftover+L0 2M seed WAL ~240 MiB never fired, so timed overwrite appended
to a fat log. Drop the threshold to write-buffer/2 (128 MiB).

## Number (this fire)

```
number: ratio=0.752 pedra_qps=199516 rocks_qps=265322 shape=overwrite_mc4 leftover+L0 2M (DIAG)
```

| | Pedra | Rocks | p50 |
|---|---:|---:|---|
| P0.75 quiet | 208 925 | 262 885 | 17.4 vs 11.5 µs |
| P0.76 | 199 516 | 265 322 | 18.2 vs 11.0 µs |

`sync: false`. Rocks quiet (~263 k). Pedra QPS **down** 209k→200k, p50
worse. Extra 1c seed flushes did not pay the timed window. **Not a land.**

Fat WAL was not the leftover+L0 p50 hole (or more L0 from seed hurt).
Hole remains Pedra 209 k / 17.4 µs vs quiet Rocks 263 k / 11.5 µs.
