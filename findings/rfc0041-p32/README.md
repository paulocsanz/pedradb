# RFC-0041 — C hit path `borrow()` + WAL one `encoded_len`

2026-08-18. **No remesure.** Load ~49–71 on 12 CPUs. Official map stays
`findings/rfc0041-p11/head3/` (C 1.796 ≈ 394 ns/op). Do **not** invent 2.0.

## What landed

- TLS last-N **hit** is `&self`: epoch compare only, no `prepare()` walk
  and no `RefCell::borrow_mut`. Miss still `borrow_mut` + store.
- `fx_bytes` for keys ≤16 B is one padded 16-byte load (YCSB
  `ycsb/000042` is 11 B). Length is mixed in so zero-pad is not a
  collision.
- WAL `EncodedOpsSource` caches `encoded_len`; `encode_write_op_batches`
  uses `fragment_encoded_len` so a 16-op raftlog batch walks ops **once**
  instead of three times (reserve + `total_len` + stats). Physical
  bytes unchanged (`fragment_encoded_matches_scratch_path_bytes`).

## Tests

`rocksdb-compat --lib` (22); `fragment_encoded_matches_scratch_path_bytes`;
`deps_suite_on_compat`.

## Not claimed

Dirty-box qps. RFC living table still = head3 medians.
