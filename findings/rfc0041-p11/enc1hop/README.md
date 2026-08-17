# RFC-0041 — encode+append off the write lock, same hop count (rejected)

2026-08-17. Incomplete remesura; **rejected on isolated + partial official
numbers**. Do not enable FLOOR. Worker stays parkfold2.

## Hypothesis

`encoff` (encode off lock + second absorb pass) cut apply_mc4 4.3 k → 1.7 k.
Retry with the **same lock-hop count** as today: prepare under the write lock,
encode+WAL-append off it, `fdatasync`, apply. No second absorb pass.
`append_order` kept WAL seq for two sequential groups.

## Result

**Rejected.** apply_mc4 Pedra **654 / (run2 incomplete) / 1841**. YCSB A
dropped to 7.9 k (70 ms tails). Seed 2.3 s (was ~0.2 s). Dropping the write
lock for encode+append + the extra `append_order` mutex costs more than it
returns; parkfold2 stays at **4.3 k**.

Reverted to `group_start` + late-join absorb + `finish_group_off_lock`
(`fdatasync` off lock, apply after). Leftover prepare-only APIs removed.

G1 unchanged (fd before Ok). No official 16-shape JSON — do not treat 654
as a product median.
