# deps_raftlog >1× (same-class async vs Rocks default)

**When:** 2026-09-02  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` (WAL `write()` per commit, no fdatasync)
vs RocksDB `WriteOptions.sync=false`. G1 vs async is out of scope.

## What 0.94 / 0.623 actually was

CHV class-fix 2026-08-30 (`findings/2026-08-30-linux-p149-async-classfix-chv/`):

| round | Pedra keys/s | Rocks keys/s | ratio | Pedra p50 | Pedra p99 | Pedra max |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 84082 | 86876 | **0.968** | 11.3 µs | 21 µs | 0.27 ms |
| 2 | 80734 | 85660 | **0.942** | ~11 µs | — | — |
| 3 | 48272 | 77519 | **0.623** | 12.3 µs | **76 µs** | **1.52 ms** |

p50 was tied. The floor breach is **wall/qps**, driven by round-3 tail.
Quiet path needed ~6–8%. The 0.623 round is a stall, not a 40% slower engine.

The suite starts raftlog with **264 192 memtable entries** already in
(apply_batch + seed). Isolated empty-DB raftlog was already ≥1× on Linux
(`P04_PASS` min 1.014).

## Cuts (this change)

1. **`MemTable::max_user_key_in_family`** — bulk-ingest first observation
   of a named CF used `iter_internal()` (copy+sort the whole tail, 264k
   keys) looking for `raftlog\0…` keys that are not there yet. Named CFs
   are a contiguous prefix; now O(log n) map+shard max. `"default"` still
   walks (raw keys interleave). This tax did not exist on the class-fix
   tree (bulk latch landed later) but is on by default today
   (`PEDRA_BULK` default on).
2. **Single-family all-puts observe** — raftlog 16× puts skip
   `classify_batch`'s HashMaps and skip the memtable-chain collect after
   the family's first high-water.
3. **`write_cf_owned`** skips CF regrouping when the batch is already one
   family (raftlog always is; apply still groups lock+default).
4. **`share_consecutive_equal_values`** pointer-eq before memcmp (WAL v2
   intern of the 16 identical 100 B values).

Local Darwin, full deps suite, 3 quiet rounds after the cuts (Rocks side
from the same box, `sync=false`):

| | Pedra qps | Rocks qps | ratio | Pedra p50 |
|---|---:|---:|---:|---:|
| r2 | 90442 | 125290 | 0.722 | 10.4 µs |
| r3 | 90291 | 121954 | 0.740 | 10.5 µs |

Darwin Rocks is **7.3–8.1 µs p50 / 122–130k qps** — faster than the Linux
CHV Rocks peer (86–87k, p50 ~10–11 µs). Local Pedra 90k at 10.4 µs is the
same quiet p50 as Linux Pedra 84k; against the **Linux** peer that is
~1.04×. Darwin 0.72× is a different Rocks, not a regression of the cut.

`PEDRA_BULK=0` A/B (one round): raftlog 88k → 94k (+7%). The fast-path
observe is the on-by-default version of that.

Max on quiet local rounds is 0.17 ms (class-fix r3 was 1.52 ms).

## Not claimed

Linux CHV 3-round remesure of the 17-shape gate was not run in this
session (this is a Darwin laptop). The registered floor lives on that
guest. This slice removes the O(n) first-batch scan that current HEAD
would have paid on that guest, and the HashMap observe tax on the 16-put
shape.

G1 vs Rocks default is not a column here.
