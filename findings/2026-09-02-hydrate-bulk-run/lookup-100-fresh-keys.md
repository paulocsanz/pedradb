# lookup_100 is 100 fresh random keys every iteration

Copied from guest `/src/slipstream/benches/snapshot_backends.rs` (2026-09-02).

The bench **intentionally** generates a new 100-key batch every Criterion
iteration (`iter_batched` setup). A fixed set would be cache-hot after
warmup and measure only call overhead. MultiGet exists to coalesce
filter/index/block I/O on **misses**.

```text
Fresh random keys EVERY iteration … the batches must keep missing.
```

TLS 512-block cache, LAST_CF, and PointCache (8192) cannot win this arm.
Rocks `multi_get` uses `batched_multi_get_cf`; Pedra `multi_get_cf` is a
loop of `get`. get_loop is 100 independent gets on both sides.

get_hit is also uniform-random (RNG hoisted so it does not replay).
Slipstream default `cache_size_bytes = 1 GiB` is `Options::set_block_cache`,
which compat mapped onto **whole-file** payload residency — the v57 1 GiB
regression on the 3.9 GiB guest. v69 caps that mapping at 256 MiB.
