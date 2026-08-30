# Per-shape: o que funciona, o que não, e de quem é a arquitetura

**When:** 2026-08-30  
**Bench:** CHV 4 vCPU, RFC-0154 live `tail_idx`, split suites, 3 rounds,
ops=2000 zipf, `PEDRA_PARITY_ASYNC=1` vs Rocks `sync=false`.  
**Numbers:** [`README.md`](README.md) + JSON `r{1,2,3}/`.  
**Not a win vs `sync=true`.** Not G1. Metal 11/17 is not this virt gate.

Harness caveat: 2000 ops. YCSB walls are 0.4–4 ms. Ratio r2 of `ycsb_c`
jumped 4.72× because Rocks fell 1.57M→0.80M qps, not because Pedra sped
up. Quote **median** of 3 rounds; do not overfit a single round.

---

## Three architectures in play

| | Rocks default (the peer) | Pedra now (0154) | Literature / production |
|---|---|---|---|
| Memtable | **SkipList** per CF (arena). Sequential insert uses `prev_[]` hint (`skiplist.h`). | One shared `tail` Vec + sharded index: HashMap `point` for `lock`/`default`/`write`, BTree `short` for empty prefix / `raftlog`. Reverse-seek on HashMap = `cached_point_ord` (copy+sort). | Rocks wiki: Hash* memtables exist for **prefix point**; “scan across prefixes requires copy and sort”. Vector memtable = random-write, scan = sort. |
| CFs | Physical: each CF its own memtable/SST. TiKV: **kvdb** (default/write/lock) **and raftdb** (second Rocks). | One WAL, one tail. CF is a prefix (`cf\0key`) and a shard of the same Vec. | R048 / TiKV docs: two instances on purpose. Raft log is not user MVCC. |
| Raft log | SkipList sequential + WAL, or **Raft Engine** (append-only file + hash memtable; PingCAP 2022). | Same LSM memtable, `raftlog` BTree shard, 16 sequential puts/batch. | LSM for a log is the pain TiKV measured (WAL+mem+flush I/O). They left Rocks for that shape. |

R010 (Dong, FAST’21, ficha D4): Rocks is a single-node engine; memtable =
skiplist; CPU became the resource after space. We are in that CPU regime:
this battery never flushes (256 MiB pin, `sst_count=0`).

RFC-0012 kept BTree “until the write path is CPU-bound in MemTable”.
`deps_raftlog` p50 9.1 vs 11.2 µs **is that bound**, and it is sequential.

---

## Trace facts (not theory)

**Apply → raftlog mem_entries.** r1/r3: raftlog **enters** with
`mem_entries=264192` (seed + apply), leaves `296192` (+32k = 2000×16).
Shared tail is not a metaphor.

**MVCC probe** (r1/r3 compat): `latest_ops=2000`, `latest_mem_hit=502`,
`latest_sst_fallback=0`, `sst_count=0`. `last_prefix_cache` hits return
before `latest_mem_hit`. So ~1498/2000 are cache, **502 actually walk
`last_visible_under_prefix`** on the write HashMap (~64k keys). Pedra
mvcc **p50 0.6 µs vs Rocks 4.3 µs** (Pedra wins the cached op) but **wall
8.8–9.4 ms vs 8.0–10.2 ms** (the 502 walks dominate). QPS ratio ~1.0 is
the wall, not the p50.

**Raftlog split:** build p50 1.3 µs + batch p50 7.7 µs ≈ Pedra p50 9.1 µs.
Rocks p50 11.2 µs. CPU-tied. Ratio 1.15 is that plus the every-8th get.

**Apply:** Pedra p50 **60 µs** vs Rocks **130 µs** (2× at p50, matches
min 2.23). Two WriteBatches per timed op (prewrite 64 + commit 64).
Build of the vectors p50 17 µs (inside the 60).

**kvrocks_scan:** Pedra p50 0.1 µs / wall 1.0 ms vs Rocks p50 28 µs /
wall 59 ms. Count on live empty-prefix BTree vs skiplist iterator.
0154 closed the 0.07× hole (that was O(n) tail filter).

**kvrocks write_group:** `avg_group=1.00` — 1c SET is one commit. No
group-commit free lunch on the 1-op shapes.

`PEDRA_WRITE_PHASE_STATS` was off; no prepare/wal/mem split in this
serial. Attribution below uses p50 + probe + code, not invented ns.

---

## Per shape (CHV median, what works / what does not)

### Reads that only hit the HashMap / BTree point path — **works**

| shape | mix | Pedra qps (r1) | Rocks qps | med × | p50 Pedra/Rocks | Works | Does not |
|---|---|---:|---:|---:|---|---|---|
| **ycsb_c** | 100% get zipf | 4.85M | 1.57M | **3.09** | 0.1 / 0.6 µs | HashMap get vs skiplist. Split suite = no `default\0`. | Ratio noisy (r2 Rocks 0.80M → 4.72×). |
| **kvrocks_get** | 1c GET | 10.3M | 1.89M | **5.43** | ~0 / 0.5 µs | Same. Empty prefix, reserved HashMap. | Timer floor (p50 0.00). |
| **ycsb_b** | 95% get | 3.36M | 1.30M | 2.61 | 0.2 / 0.6 µs | Gets win; 5% put pays WAL+insert. | The 5% write keeps it under 3×. |
| **ycsb_d** | 95% read-latest + insert | 3.98M | 1.35M | 2.94 | 0.2 / 0.6 µs | Almost 3× (metal was 3.07). | Inserts + zipf-latest. Closest miss. |

Metal split (live BTree, no HashMap-write): c 3.06, d **3.07**, b 2.76.
CHV HashMap helps c/get; d lost the 3× to virt + mix.

### Range **count** on a live ordered shard — **works (the 0154 win)**

| shape | what | med × | p50 P/R | Works | Does not |
|---|---|---:|---|---|---|
| **ycsb_e** | zipf scan COUNT~ | **15.7** | 0.3 / 8.4 µs | `count_latest_in_range` on empty-prefix BTree. Rocks iterator. | Not a real coprocessor; cap=25. |
| **deps_scan** | write CF `[u, u+25)` cap 25 | **6.39** | 0.3 / 5.1 µs | Same on `write` HashMap→sorted `point_ord` (first scan sorts, then binary). Probe `scan_ops=521` (count cache). | First-range sort of ~64k write keys. Still 6×. |
| **kvrocks_scan** | SCAN COUNT=25 | **59.2** | 0.1 / 28.5 µs | 0154: empty-prefix BTree count. Was 0.07× when `idx_stale` sorted 67k **per scan**. | Rocks SCAN is the slow peer, not a Pedra 59× “engine”. |

Wiki: Hash* “full scan = copy and sort”. We pay that **once** per CF
(`point_ord`), then count. Rocks SkipList pays log n per step × 25, plus
iterator machinery. On this tiny window, index-count wins.

### 1-client async puts (same durability class) — **partial, ceiling ~2.2–2.8×**

Both engines: WAL write, **no** fdatasync (`PEDRA_PARITY_ASYNC=1` /
`ROCKS_PARITY_SYNC=0`). Group commit `avg_group=1`. One full WAL record
per op vs the peer’s one WAL record per op.

| shape | med × | p50 P/R | Works | Does not |
|---|---:|---|---|---|
| **ycsb_a** | 2.73 | 0.6 / 2.1 µs | Gets in the 50% mix. | Puts. Metal was **3.23**. CHV virt + HashMap insert. |
| **ycsb_f** | 2.21 | 1.0 / 2.7 µs | — | RMW = get+put. Worst YCSB. Metal 2.44. |
| **deps_cache_overwrite** | 2.65 | 0.8 / 2.6 µs | Overwrite HashMap. Metal **3.06**. | 1c put. |
| **kvrocks_set** | 2.75 | 0.8 / 2.4 µs | Metal **3.03**. | 1c SET. |
| **kvrocks_blob_set** | 2.48 | 4.2 / 24.7 µs | Pedra p50 6× faster (vlog inline/spill vs Rocks blob). | Wall ratio 2.5; 16 KiB copy. Metal 2.08. R005 WiscKey: separate value log helps p50, not a 3× QPS miracle at 16 KiB. |

**Architectural:** 1c async write is WAL-encode + memtable insert +
publish. SkipList sequential/random insert with arena vs HashMap+Bytes.
You do not get 3× on this class without either (a) grouping (pipelined)
or (b) a cheaper WAL. Product already refuses skipping the WAL.

### Batched writes — **pipelined works; apply/lock stuck ~2.3–2.7×**

| shape | batch | med × | p50 P/R | Works | Does not |
|---|---|---:|---|---|---|
| **kvrocks_pipelined_set** | 32 SETs / 1 WAL | **3.93** | 6.3 / 28.8 µs | Grouping. Metal 3.22. | — |
| **deps_lock_prewrite** | 32× (lock+default) | 2.67 | 29 / 77 µs | One atomic batch. | 64 CF keys into HashMap+WAL. Metal was **1.94** (worse). CHV improved. Still <3. |
| **deps_apply_batch** | 2 batches × 32 txns (lock/default/write) | **2.26** | 60 / 130 µs | 2× closed (min 2.23). Isolated metal 2.17. | 3× not closed. 128 CF ops / timed op, shared tail 256k, HashMap insert of `write` (always-new keys). |

Apply is the raftstore ready: Percolator prewrite+commit (R041). TiKV
does this as one Rocks `WriteBatch` across physical CFs, **one skiplist
per CF**. We insert 64 keys into three HashMaps on one Vec. 2× is the
honest live-idx number; 2.96 with `idx_stale` was a scan regression.

### MVCC latest — **p50 works, wall does not (1.02×)**

`latest_then_get_cf(write, ukey, default)`: reverse-seek write prefix,
then get default (one mutex).

| | Pedra | Rocks |
|---|---:|---:|
| p50 | **0.6 µs** | 4.3 µs |
| wall / 2000 | 8.8–9.4 ms | 8.0–10.2 ms |
| qps | ~215k | ~220k |
| med × | **1.02** | |

Metal split (write CF still BTree `short`): mvcc **3.45×**.  
Lazy-idx CHV: **0.20×** (O(n) rebuild).  
Live HashMap write: **1.02×**.

The regression vs metal is **not CHV**. It is `point_cf("write")`.
`last_visible` on a HashMap must `cached_point_ord` (copy+sort the ~64k
write keys). Rocks `SeekForPrev` on a SkipList is O(log n) with no
full-CF materialize. The wiki said this: Hash* reverse/range across the
prefix space = sort.

502 uncached `last_visible` walks ≈ the whole 9 ms wall. Cache hits are
the 0.6 µs p50.

**Fix that matches the literature:** `write` must stay an **ordered**
shard (BTree `short` or SkipList), because MVCC latest **is** reverse
iterate. HashMap is for `lock` (overwrite, no range) and maybe
`default` point. `deps_scan` already has a count path; it can use
BTree range like empty-prefix kvrocks.

### Raftlog — **empatado (1.15×), and the literature left LSM**

16 sequential `raftlog/{idx}` puts + every 8th get. Pedra p50 9.1 µs /
Rocks 11.2 µs. min **1.09**. Metal 1.13. Open-items already: “2× em
raftlog 1c recusado (p50 empatado)”.

Rocks SkipList **Insert with Hint** is the sequential fast path (primary:
`skiplist.h`, `prev_[0]`). Pedra inserts into a BTree of growing
sequential keys = log n rotations, no hint. Shared tail already holds
264k apply versions (trace).

TiKV: **raftdb ≠ kvdb**. Then Raft Engine: append-only log + hash
memtable, because WAL+skiplist+flush is wasted I/O for a log. We are
still in the first TiKV mistake (log in the KV LSM), one DB further
(same tail as apply).

RFC-0065 P2 / 0154 P2.1: second memtable/DB for raftlog, or a log
structure. Skiplist+hint (0012 reopen) only if we stay in-LSM.

---

## What the 6/17 actually is

**Over 3× (6):** c, e, deps_scan, get, **kvrocks_scan**, pipelined.  
These are **reads with an index** or **writes that group**.

**Need 3 more for majority.** Closest: d 2.94, set 2.75, a 2.73, lock
2.67, cache 2.65. All 1c or mixed **write**. Metal already had d/a/set/
cache ≥3; CHV virt ate ~0.3×. Not an idx bug.

**Structural misses (will not become 3× by tuning HashMap):**

1. **`deps_mvcc_latest` 1.02×** — HashMap on a reverse-seek CF. Put
   `write` back on BTree/skiplist (Rocks’ default for that CF).
2. **`deps_raftlog` 1.15×** — sequential log in an LSM. Raft Engine /
   separate raftdb, or SkipList+hint. Not another HashMap.
3. **1c put family ~2.2–2.8×** — WAL+insert vs WAL+insert. Pipelining
   is the only shape in this family that cleared 3×. Group commit
   already shows `avg_group=1` on 1c.

**Already closed by 0154:** kvrocks_scan 0.07→59, mvcc 0.20→1.02
(rebuild gone). Do not reopen `idx_stale`.

---

## Sources (primary where we have them)

| ID | What we used | Where |
|---|---|---|
| R010 Dong FAST’21 | LSM, skiplist, CPU after space | `research/fichamentos/ficha_R010_Dong_RocksExperience.md` D4 |
| R041 Percolator | lock/write/default = our deps shapes | ficha D4 |
| R048 TiDB/TiKV | learner; storage = Rocks | ficha D4; kvdb/raftdb from TiKV docs |
| Rocks wiki MemTable | Hash* = copy+sort for scan; SkipList+hint | [`refs/rocksdb-wiki-memtable-2026-08-30.txt`](refs/rocksdb-wiki-memtable-2026-08-30.txt) |
| `skiplist.h` Insert | sequential `prev_[]` fast path | facebook/rocksdb (quoted above) |
| PingCAP Raft Engine 2022 | LSM wrong for raft logs; hash+append log | blog numbers in refs file (TPC-C ~4% QPS, −25–40% write I/O) |
| R005 WiscKey | blob/vlog | ficha D4; blob p50 win, QPS 2.5× |
| RFC-0012 | skiplist deferred until memtable CPU-bound | now bound on raftlog |
| This CHV JSON + serial probes | p50/wall/mem_entries/latest_mem_hit | this directory |

CHV 6/17 does **not** say “need more RAM” (0153 P0) and does **not**
say “need arena first”. It says: **ordered index on CFs that reverse-
seek or sequential-append; HashMap only on point CFs; log ≠ LSM.**
