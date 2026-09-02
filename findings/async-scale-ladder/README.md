# Pedra async vs fjall / sled default — scale ladder

**When:** 2026-09-02, Darwin laptop, 1 client, 2000 timed ops, zipf θ=0.99.  
**Class:** all three are process-crash / page-cache durable (no fsync-per-op).
Pedra `OpenOptions.sync=false` (WAL `write()` per commit). fjall/sled
factory default. Product Pedra still auto-flushes at **4 MiB**.

Harness: `cargo run --release --manifest-path findings/async-scale-ladder/Cargo.toml`

The 450k / 432k numbers from `alt-engines-20260817` were YCSB-A at
**4096 × 1000 B** with Pedra **G1** (fdatasync). This ladder is the same
schedule with Pedra async, so it is the fair comparison.

## YCSB-A (50% new inserts) — we lose at every scale

qps, this box, second run:

| records | payload | pedra 4MiB | pedra noflush | fjall | sled | pedra/fjall |
|---:|---:|---:|---:|---:|---:|---:|
| 1k | 100 | 569k | — | 1070k | 780k | **0.53** |
| 4k | 100 | 534k | — | 1209k | 667k | **0.44** |
| 4k | 1000 | 156k | 513k | 844k | 588k | 0.18 / **0.61** noflush |
| 16k | 100 | 544k | — | 1098k | 769k | **0.49** |
| 16k | 1000 | 157k | 476k | 802k | 542k | 0.20 / **0.59** noflush |
| 64k | 100 | 439k | — | 824k | 643k | **0.53** |
| 256k | 100 | 394k | — | 906k | 537k | **0.43** |
| 1M | 100 | 380k | — | 492k | 420k | **0.77** |

No crossover on writes. Closest is 1M (fjall falls off) and 100 B no-flush
would still be ~0.5×. The 4k×1000 collapse (156k vs 844k) is mostly our
**4 MiB auto-flush** firing in the timed window (seed is already 4 MB);
turning flush off recovers 513k (0.61× fjall, 0.87× sled) — still behind.

Load/ingest (seed puts/s): pedra 150–300k vs fjall 540–910k vs sled
365–570k at every size. 1M: 260k vs 643k vs 365k.

## YCSB-C (100% read) — we win small 100 B, lose mid, win 1M vs fjall

| records | payload | pedra 4MiB | pedra noflush | fjall | sled | pedra/fjall |
|---:|---:|---:|---:|---:|---:|---:|
| 1k | 100 | **5.82M** | — | 2.66M | 2.98M | **2.19** |
| 4k | 100 | **4.38M** | — | 2.29M | 1.95M | **1.91** |
| 4k | 1000 | 1.19M | **4.39M** | 2.33M | 2.42M | 0.51 / **1.88** noflush |
| 16k | 100 | **3.17M** | — | 1.74M | 1.93M | **1.83** |
| 16k | 1000 | 0.88M | **3.26M** | 1.79M | 1.93M | 0.49 / **1.82** noflush |
| 64k | 100 | 1.22M | — | 1.43M | 1.59M | **0.85** |
| 256k | 100 | 0.99M | — | 1.28M | 1.23M | **0.77** |
| 1M | 100 | **0.82M** | — | 0.49M | 0.81M | **1.67** (tied sled) |

Read crossover vs fjall: **lose at 64k–256k × 100 B** (working set leaves
the answer cache / their block cache stays hot). At 1M fjall drops hard
(491k) and we are ahead again. 1 kB values with product flush: lose
reads ~2×; without flush we win ~1.8×.

## How to read the old 450k / 432k

Those were fjall/sled YCSB-A at 4k×1000 B. Same cell today: fjall 844k,
sled 588k, Pedra async+4MiB flush 156k, Pedra async noflush 513k. The
450k was a smaller fjall on a different day; the shape of the gap is the
same — write-heavy async, they journal, we still `write()` every commit
and (with product defaults) flush at 4 MiB.
