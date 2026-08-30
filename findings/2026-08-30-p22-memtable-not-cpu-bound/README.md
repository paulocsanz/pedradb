# RFC-0154 P2.2 — write buffer is **not** CPU-bound (skiplist/arena **REFUSED**)

**When:** 2026-08-30  
**Host:** Darwin release `pedradb-core` micros (order of magnitude; CHV
ratios from official P1.8 / p18 and p22 qps).  
**Peer class:** coluna A (`PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`).

## Gate (RFC-0012 / RFC-0154 P2.2)

Reopen skiplist/arena **only if** the real write buffer (64 MiB class) is
CPU-bound **in the MemTable**. Measured insert floor vs CHV wall:

| shape | memtable-only | CHV Pedra p50 (p22) | memtable share |
|---|---:|---:|---:|
| apply (128 entries / op) | **15.07 µs/op** (0.118 µs/entry) | 65.7 µs | **23%** |
| raftlog (16-append) | **2.67 µs/batch** (0.167 µs/op) | 7.5 µs | **36%** |
| SET / cache 1c | O(1) tail push + live idx | 0.8 µs | ≪ wall |

Micros (release, this tree):

```
mem apply micro: 2000 ops x 2x64 entries, insert 15.07 µs/op, 0.1177 µs/entry
mem micro raftlog: 100000 x 16, 0.1672 µs/op
```

Apply qps CHV compat ~14.2k (p22 r1). Zeroing memtable would cut p50
65.7→~51 µs → ratio 1.88× → ~2.4×. Still not 3×. A skiplist cannot zero
the insert (Rocks' own default **is** a skiplist).

## Why skiplist is the wrong cut here

- Pedra insert is already **O(1) `Vec` push** plus a live `tail_idx`.
  Rocks `InlineSkipList` is O(log n) pointer chase + arena. That helps
  **concurrent** memtable writers (`allow_concurrent_memtable_write`).
  Pedra's `ConcurrentDb` takes the write lock — one mutator.
- Sequential raftlog is a **p50 tie** (1.08×). Rocks `Insert` hint does
  not leave 3× on the table.
- Empty-prefix HashMap (P1.3) already lost `ycsb_c`. Skiplist keeps
  order but spends the write path Pedra already won with a tail.

## Refuse

Do **not** replace the memtable with an arena skiplist in this band.
Keep `Vec` tail + live index (RFC-0002 / 0012). Official CHV stays
[`../2026-08-30-linux-p149-p21-chv-p18/`](../2026-08-30-linux-p149-p21-chv-p18/)
**12/17**. Not a remedir (write path unchanged).

The ~77% of apply p50 that is **not** memtable is WAL encode / prepare /
publish of two 64-key batches. That is the remaining write-path mine,
not P2.2.
