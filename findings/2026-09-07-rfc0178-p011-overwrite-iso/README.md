# RFC-0178 P0.11 — overwrite_mc4 isolado Darwin

**When:** 2026-09-07. **Host:** Darwin. **Not** 4 GiB box. **Not** 3-run.
**Peer:** RocksDB default `ROCKS_PARITY_SYNC=0`. Same-class drop-in
(async WAL both sides). **Not** a G1 win.

```
ROCKS_PARITY_SUITE=ycsb
ROCKS_PARITY_ONLY=deps_cache_overwrite_mc4
ROCKS_PARITY_CLIENTS=4
ROCKS_PARITY_MC_FRESH=1
ROCKS_YCSB_OPS=10000
ROCKS_PARITY_BIG=0
```

`run_clients` now skips shapes `ONLY` did not name. Log shows one shape.

## Numbers (1-run)

| | qps | p50 |
|---|---:|---:|
| Pedra drop-in | 132 904 | 11.6 µs |
| Rocks default | 292 513 | 11.0 µs |
| **ratio** | **0.454×** | — |

In-suite named loss was 0.557×. Isolado **não** passou de 1× — a célula
é o overwrite, não só a suíte.

G1 (fdatasync before Ok) would be slower still. Do not quote as a win.

P1.3 (3-run na caixa) continua aberto.
