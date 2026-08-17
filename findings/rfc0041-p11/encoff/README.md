# RFC-0041 — encode WAL off the write lock (rejected)

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`encoff/run{1,2,3}`). JSONs only. Peers `sync: false`.

Hypothesis: seq-assign under the write lock, `encode_ops` without it,
then append + `fdatasync` (G1). Park/fold keep the write lock during
encode. A second prepare pass absorbs writers that queued mid-encode.

WAL order must match seq assign: two groups preparing in parallel
wrote lower seqs after higher ones (`change_feed` `sequence > max`).
An `append_order` mutex fixed that but serialized the extra lock hops.

## Result

**Rejected.** **2/16** (E **2.62**, MVCC **2.21**). apply_mc4 Pedra
**1.7 k** (parkfold2 **4.3 k**). apply 1c **0.8 k**. Extra write-lock
acquire around encode cost more than the encode CPU saved. Path
restored to prepare+encode+append under one write lock; fd still
off-lock. Isolated `fdatasync` p50 **24.2 µs**.

FLOOR not enabled. 1c A still one WAL `fdatasync` per Ok.
