# SAFETY — `pedradb-capi`

In-process product C ABI (RFC-0023 P2.2). **Not** `libfdb_c` / fdbcli.
`pedradb-store` is `#![forbid(unsafe_code)]`.

The C+ASan harness (`scripts/capi-asan.sh`) is the **product gate**: PASS
binary green, malicious binary ASan-red. CI job `capi-asan-harness`.

## What is not `unsafe`

Database and transaction “pointers” are packed `slot + generation` integers
(`src/handles.rs`). They are not `Box::into_raw` heap pointers.

- Double-destroy / use-after-destroy → `NULL` / `MONTAHA_FDB_ERROR`
- Destroy-db while a tx is live → txs on that db are drained
- Commit consumes the tx handle; a later destroy is a no-op
- Get buffers: `Box::into_raw` then a table of `(addr → len)`. C reads
  the pointer while it is the owner (F210). `montanha_fdb_free` does
  `Box::from_raw` **only** for keys in the table; unknown / double-free
  is a no-op (never `from_raw` of a garbage pointer). RFC-0075 P2.2:
  `c_free_table_admitted` is always false — that table is TCB, not a
  Verus twin (`verus/c_len.rs` is the len cap only).
- Tables are **thread-local** (`StoreCluster` / `SeedRng` is `!Send`). A
  handle used on another thread misses the table → ERROR, not a data race.
- Generation is 31 bits (`GEN_MASK`). `next_gen` stays inside the mask so
  a live handle never `pack`s to gen=0 (F209). After 2³¹−1 reuses of one
  slot, gen wraps to 1: a stale handle from that first generation can
  alias (inherent to a packed 31-bit gen, not allocator UB).

## Length caps (F215)

Marshalling copies C bytes only after a length check. A huge `*_len` is
`MONTAHA_FDB_LIMIT` / NULL and does **not** construct a terabyte slice.

| Input | Cap | Oversize |
|-------|-----|----------|
| `path` | `c_path_walk_bytes()` = `MAX_PATH_BYTES` (4096) NUL walk via `memchr` (RFC-0075 P1.1). AS-IS is `usize::MAX`. Offset must pass `c_path_nul_off_admitted` | create → NULL |
| `key_len` | `MAX_C_KEY_BYTES` = `MAX_TX_BYTES` (10MiB) via `c_len_admitted` | `LIMIT` |
| `value_len` | `MAX_C_VALUE_BYTES` = `MAX_VALUE_BYTES` (100KiB) via `c_len_admitted` | `LIMIT` |

Copy is `ptr::copy_nonoverlapping` (memcpy). `memchr` / memcpy are
ASan-intercepted in the C harness (Darwin ASan does **not** intercept
`strnlen` — that is why the path walk is `memchr`, not `strnlen`).

## Remaining `unsafe` (marshalling only)

| Site | Obligation |
|------|------------|
| `memchr(path, 0, c_path_walk_bytes())` | First `MAX_PATH_BYTES` readable, or ASan fires (C contract) |
| `copy_nonoverlapping` of key/value | Bytes readable for the **capped** `len` (C contract) |
| Writes through `out_ptr` / `out_len` | Pointers checked non-null |

A caller that lies about a *capped* length (4-byte buffer, `key_len=256`)
is still UB. That is inherent to C. Stale *handles* are not. Wild pointers
are not validatable from this side.

## C+ASan harness (product gate)

`scripts/capi-asan.sh` builds `libpedradb_capi.a` and two C binaries
under `-fsanitize=address`:

| Binary | Must |
|--------|------|
| `capi_asan` | **PASS** — well-behaved set/get/commit, stale/double-free handles, oversize `*_len` → `LIMIT` (script greps `LIMIT key/value/get/live-key`, RFC-0075 P1.2), 4096-byte no-NUL path → NULL |
| `capi_asan_malicious` | **FAIL** (ASan) — short *heap* buffer + capped-but-too-big `len`; 8-byte heap path with no NUL (`memchr`) |

PASS proves rotten handles and accidental huge lengths. FAIL proves the
sanitizer still sees a malicious C slice under the cap.

## What this crate must not grow

- `Box::into_raw` / `from_raw` for db/tx handles (buffers use into_raw
  only while the pointer is table-owned; garbage `free` must not)
- Unlocking the process mutex across a callback into C
- Claiming FoundationDB client compatibility (`libfdb_c` / fdbcli)
- Shipping a change that makes `capi_asan` ASan-red or the malicious
  binary ASan-green
