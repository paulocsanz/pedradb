# RFC-0041 — TLS direct-map last-128 + catch-up 80 µs

2026-08-18. **No remesure.** Load ~52–72 on 12 CPUs (protobuf fuzzers
~40% each). Official map stays `findings/rfc0041-p11/head3/` (C 1.796 /
scan 1.790 / raftlog_mc4 1.792). Do **not** invent 2.0.

## Why last-32 linear was the wrong shape

YCSB-C head3 is **2.54 M qps ≈ 394 ns/op**. Closing 1.796 → 2.0 needs
~39 ns off that path. A linear scan of 32 TLS slots (cf + key memcmp)
is itself ~100–200 ns — paid on every get, including misses. Last-32
could be a net loss vs last-1.

## What landed

- `get_named` / `count_named`: **direct-mapped 128 slots, 2-probe**
  (fxhash). Hit = hash + one compare. Epoch still drops every slot on
  publish. `get_named_tls_direct_map_keeps_zipf_hot_set` fills 64 keys
  then re-gets them; invalidate-on-put stays green.
- `CATCHUP_WINDOW_DEFAULT` **50 → 80 µs**. 1-op still
  `min(window, fd_ema/2)`. Fat raftlog (≥16 ops) waits the full 80 µs
  so MC siblings can share one `fdatasync`. `PEDRA_CATCHUP_US=0` still
  off. `catchup_window_knob` asserts 80 µs.

## Tests

`rocksdb-compat --lib` (19); `catchup_window_knob`,
`catchup_bound_policy`; `deps_suite_on_compat`. Scratch logs under the
implementer dir.

## Not claimed

Dirty-box qps. RFC living table still = head3 medians.
