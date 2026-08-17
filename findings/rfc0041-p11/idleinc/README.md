# RFC-0041 — park during writes + 1 materialize + 1 compact/tick (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`idleinc/run{1,2,3}`). JSONs only. Peers `sync: false`.
Isolated `fdatasync` p50 **23.3 µs**.

Park imm (no lz4) during writes; on 5 ms idle, one materialize and one
≤4-file L0 compact.

## Result

**0/16 ≥ 2.0.** apply_mc4 Pedra **2.9 k** (better than scansst 2.1 k) but
the 1.98 ratio is a weak Rocks (1.5 k). Scan **0.24** (36–56 k) — parked
BTrees. MVCC 1.02. Rejected. Worker returns to drain-during-writes +
bounded 1 compact/tick.

FLOOR not enabled.
