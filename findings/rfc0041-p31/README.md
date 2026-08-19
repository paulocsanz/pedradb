# RFC-0041 — TLS last-1024 + `get()` key-only

2026-08-18. **No remesure.** Load ~53–104 on 12 CPUs. Official map stays
`findings/rfc0041-p11/head3/` (C 1.796 ≈ 394 ns/op; 2.0 needs ~39 ns).
Do **not** invent 2.0.

## What landed

- TLS tables are **1024** direct-mapped slots (2-probe), heap-allocated
  (`Box<[Slot]>`). 1024 TinyBufs on the stack overflowed
  `basic_put_get_delete_reopen` in debug.
- Official C calls `DB::get`, not `get_named`. `get()` now has its own
  last-N that hashes **only the user key** (no `default` prefix / CF
  compare). Miss still `encode_with` so prefixed-default (DEPS_CFS) is
  correct. Epoch still drops every slot on publish.
- Tests: `get_default_tls_*` (open_default + open_cf prefixed + 200-key
  working set), existing `get_named_tls_*`, full `rocksdb-compat --lib`
  (22), `deps_suite_on_compat`.

## Not claimed

Dirty-box qps. RFC living table still = head3 medians.
