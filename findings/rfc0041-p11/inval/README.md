# RFC-0041 — one point-cache gen-bump per write group

2026-08-17. Official 16-shape, `ROCKS_PARITY_SYNC=0`, median of 3
(`inval/run{1,2,3}`). JSONs only. Peers `sync: false`.
Isolated `fdatasync` p50 **25.8 µs**.

Worker is parkfold2 (per-write notify; park+fold on Run/Timeout).
`group_apply` now bumps the answer-cache generation **once** per group
instead of per member. Tests: `group_apply_invalidates_point_cache`,
`four_client_fat_apply_is_durable_without_per_write_wakeup`.

`nowake` (notify only when imm staged) and `parkonly` (fold only on
poll) were measured and **rejected** — apply_mc4 4.3 k → 0.7–2.5 k.

## Result

**3/16 ≥ 2.0** (`ycsb_e` **2.201**, `deps_mvcc_latest` **2.967**,
`deps_apply_batch_mc4` **2.047**). FLOOR not enabled.

apply_mc4 **2.047** is a weak-Rocks median (Pedra **3.3 k**, below
parkfold2 **4.3 k**). Scan median **1.522** (parkfold2 was **2.209**)
— run noise, not a product win. 1c A **0.148** still one WAL
`fdatasync` per Ok.

G1 unchanged. Adversarial 5 green. Compare refuses `sync: true`.
