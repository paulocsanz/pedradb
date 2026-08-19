# SurrealDB kv-rocksdb — transaction begin (excerpt)

**Fetched:** 2026-08-18
**Source:** https://github.com/surrealdb/surrealdb/blob/main/surrealdb/core/src/kvs/rocksdb/mod.rs
**Used by:** RFC-0043 P2.4 / `rocksdb-compat` `OptimisticTransactionDB`

SurrealDB 3.3 nightly holds `Pin<Arc<OptimisticTransactionDB>>` and begins
every KV transaction like this (line numbers from main on 2026-08-18):

```
// ~L755
pub(crate) async fn transaction(&self, write: bool, _: bool) -> Result<Box<dyn Transactable>> {
    let mut to = OptimisticTransactionOptions::default();
    to.set_snapshot(true);
    let mut wo = WriteOptions::default();
    // Per-transaction sync is never used. When sync=every is configured, the
    // commit coordinator handles grouped fsync after parallel transaction
    // commits. When sync=<interval> or sync=never, no per-transaction fsync
    // is needed either.
    wo.set_sync(false);
    let tx = self.db.transaction_opt(&wo, &to);
    // ... optional UDT read timestamp when versioning ...
    let snapshot = tx.as_ref().snapshot();
    let mut ro = ReadOptions::default();
    ro.set_snapshot(&snapshot);
    ...
}
```

Open path: `OptimisticTransactionDB::open` or `open_cf_descriptors` with
an explicit `"default"` CF only when versioning (UDT comparator) is on.

Pedra maps this to `OccTransaction` (snapshot + write-set conflict →
`Busy`). G1 still `fdatasync`s before Ok; their `set_sync(false)` is the
official Rocks peer, not a Pedra durability change.

Also implemented (compile-shape, 2026-08-18): `open_cf_descriptors`,
`ReadOptions` iterate bounds, `raw_iterator_opt` (`seek`/`next`/`key`/`value`),
`property_int_value` → `Ok(None)`, `flush_opt`/`flush_wal`, `compact_range_opt`,
`wait_for_compact`/`cancel_all_background_work`, Options setters they call
at open (no-ops). Prefix `SliceTransform::create` accepted unused.

Remaining for a real in-process SurrealDB build: UDT timestamps (live
comparator), prefix bloom, `surrealdb-rocksdb` 0.24 MemoryManager FFI.
