# SAFETY — `pedradb-capi`

Lab C ABI (RFC-0023 P2.2). **Not** a supported product face, **not**
`libfdb_c`. `pedradb-store` is `#![forbid(unsafe_code)]`.

## What is not `unsafe`

Database and transaction “pointers” are packed `slot + generation` integers
(`src/handles.rs`). They are not `Box::into_raw` heap pointers.

- Double-destroy / use-after-destroy → `NULL` / `MONTAHA_FDB_ERROR`
- Destroy-db while a tx is live → txs on that db are drained
- Commit consumes the tx handle; a later destroy is a no-op
- Get buffers: `Box::into_raw` then a table of `(addr → len)`. C reads
  the pointer while it is the owner (F210). `montanha_fdb_free` does
  `Box::from_raw` **only** for keys in the table; unknown / double-free
  is a no-op (never `from_raw` of a garbage pointer).
- Tables are **thread-local** (`StoreCluster` / `SeedRng` is `!Send`). A
  handle used on another thread misses the table → ERROR, not a data race.
- Generation is 31 bits (`GEN_MASK`). `next_gen` stays inside the mask so
  a live handle never `pack`s to gen=0 (F209). After 2³¹−1 reuses of one
  slot, gen wraps to 1: a stale handle from that first generation can
  alias (inherent to a packed 31-bit gen, not allocator UB).

## Remaining `unsafe` (marshalling only)

| Site | Obligation |
|------|------------|
| `CStr::from_ptr(path)` | `path` is a valid NUL-terminated C string (or we returned on null) |
| `from_raw_parts(key/value, len)` | The bytes are readable for `len` (C contract) |
| Writes through `out_ptr` / `out_len` | Pointers checked non-null |

Garbage `key`/`path` pointers are still UB — that is inherent to C. Stale
*handles* are not.

## What this crate must not grow

- `Box::into_raw` / `from_raw` for db/tx handles (buffers use into_raw
  only while the pointer is table-owned; garbage `free` must not)
- Unlocking the process mutex across a callback into C
- Claiming FoundationDB client compatibility
