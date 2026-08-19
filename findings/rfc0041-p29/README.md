# RFC-0041 — TLS last-32 + `write_cf_owned`

2026-08-18. **No remesure.** Load ~39–76 on 12 CPUs (fuzzers). Official
map stays `findings/rfc0041-p11/head3/` (C 1.796 / scan 1.790 /
raftlog_mc4 1.792).

## What landed

- `get_named` / `count_named`: per-thread last-**32** (not last-1).
  Official mix is zipfian θ=0.99 over 4096 keys — last-1 only hits
  consecutive repeats. Last-32 holds the zipf hot set and skips
  CF-prefix encode + the shared point/count-cache mutex. Values stay
  `Bytes` (shared with `PointCache`); `to_vec` only at the rust-rocksdb
  return. Epoch from `read_cache_epoch` drops every slot on publish.
- `write_cf_owned`: apply/raftlog `CompatEngine::batch` **moves** the
  1 KiB payloads into `Bytes` instead of `copy_from_slice`. Same
  visibility / G1 (`fdatasync` before Ok).
- `write_cf_slices` still exists (borrowed path); now pre-sizes the
  op vec and skips `check_cf` when the CF repeats inside the batch.

## Tests

`get_named_tls_*`, `count_named_tls_*` (including two-key / two-window
last-N), `write_cf_owned_moves_values_and_is_durable`, full
`rocksdb-compat --lib`.

## Not claimed

Dirty-box qps. Do not overwrite the RFC table from a loaded remesure.
