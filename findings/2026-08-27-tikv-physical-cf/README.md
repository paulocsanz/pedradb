# TiKV physical CFs vs Pedra prefix CFs

**Question:** does TiKV need real CF isolation (compact / cache / flush
per family), or is prefix encoding enough?

## What TiKV does

TiKV `engine_rocks` opens Rocks with separate CFs: `default`, `write`,
`lock` (and raftdb `raftlog`). Each CF is its own LSM: own memtable, own
SST set, own compact, optional own block cache. rust-rocksdb 0.22 is
**not** what TiKV links — it uses the tikv/rust-rocksdb C++ fork. So
“TiKV as host of `crates.io` rocksdb 0.22” is hypothetical.

## What Pedra does

`create_cf` / `put_cf` prefix the user key (`lock\0…`) in **one** LSM.
Observable KV is per-CF: `get_cf("lock")` does not see `write`. Full-CF
scans do not leak. That is S2 (same KV).

Isolated as of RFC-0065 P1 (2026-08-27):

- flush emits one SST per CF; `flush_cf` / auto-flush of one family leaves the others in mem
- `compact_range_cf` rewrites only that family
- L0 write-stall is per CF (default fat does not stall lock)
- one WAL / one `WriteBatch` (not 3 DBs)

Still shared: block/answer cache (P2.2); raftdb is a second `DB` path (P2.1).

## Verdict

Prefix CFs are a **correct** substitute for hosts that only need the
names (`open_cf`, `put_cf`, `iterator_cf`). RFC-0065 P0+P1 also split
SST files, auto-flush, compact, and L0 stall **per family** (TiKV lock
CF stays tiny). Remaining: P2.1 raftdb second DB, P2.2 per-CF cache.
Not required for Surreal 1.5. Do not open 3 DBs.
