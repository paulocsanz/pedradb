# C+ASan harness — `pedradb-capi`

Product gate for the in-process C ABI. Not `libfdb_c`.

```
bash scripts/capi-asan.sh
```

| Binary | ASan |
|--------|------|
| `capi_asan.c` | must **PASS** (honest caller, rotten handles, oversize `*_len` → `LIMIT`) |
| `capi_asan_malicious.c` | must **FAIL** (`key` / `value` / `path` short-buffer lies under the cap) |

Rust `cargo test -p pedradb-capi` is not a substitute: those tests never
link a C translation unit.
