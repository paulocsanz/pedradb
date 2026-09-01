# v21p root-cause: why settle/hydrate lose 0.16–0.18× (2026-08-31)

Deep-dive demanded after v21m/n/o closed both parallelism levers. Method:
local 6M CPU profiles (`sample` during pedra hydrate + settle windows), a
caller-tagged fdatasync diag (`PEDRA_FDSYNC_CALLERS`, new), guest capture
re-reads (runs 16–18), and parity-harness shape comparison. Numbers below
are from those artifacts; nothing is projected without a source.

## Finding 1 — idle-WAL-rotate manifest storm (read legs) — FIXED here

`FDSYNCDIAG` on the guest counted ≥10,240 `fdatasync`s, cum 42.2 s, avg
12.3 ms, max 1114 ms — all printed after settle, during `get_hit`. A
backtrace tagged every 1024th call (all 4 identical):

```
spawn_compact_worker (rocksdb-compat)
 → ConcurrentDb::rotate_wal_if_writers_idle
 → Db::try_rotate_wal
 → persist_manifest → ManifestPersist::write → manifest::store
 → fdatasync_file            (MANIFEST + CURRENT per call)
```

`flush_kernel::wal_rotate_decision` returns `RotateWal` whenever the
pipeline is drained — with no "has the segment grown since the last
rotate" term. A drained DB with an **empty current segment** still
rotates: full MANIFEST+CURRENT rewrite, two `fdatasync` barriers, WAL
recreate. The compat compact worker polls this while idle, so the whole
read phase of the bench (probe_hit → get_hit → prefix_scan → lookup)
runs against a self-inflicted barrier storm. Locally the same ~10k syncs
cost 244 µs each (cum 0.5 s); on the guest each is a ~12 ms virtio
barrier → 42 s of flush traffic through the same disk the reads use.

Guest legs taxed: get_hit 0.83×, lookup_100 0.75×, prefix_scan 0.48×.

**Fix (this commit):** `try_rotate_wal` is now edge-triggered — an empty
current segment (`Wal::position() == 0`) keeps the WAL. Regression test
`idle_rotate_with_empty_segment_does_not_rewrite_manifest` asserts idle
polls do not rewrite MANIFEST and that one append re-arms rotation once.

## Finding 2 — every written SST is read back and fully re-verified — FIXED here

`write_sst_try_sorted_body` (the single convergence point of ALL writer
paths: L0 flush, leveled compaction, rewrites, parallel merge specs) used
to end with `SstTable::open_on(env, path)`, which:

1. `read_to_end` — reads the entire just-written file back,
2. `crc_stripped_body` — CRC32C over the whole file again,
3. `decode_v2_or_v3` verify loop — **lz4-decompresses every block and
   decodes every entry** (sortedness, count, bounds).

That is ~2 extra full passes over every flushed/compacted byte, plus the
syscall read. The guest pushed 11.27 GiB through flush+compaction during
hydrate alone → ~11 GiB re-read + re-CRC + re-decompress of the same
bytes the process had just encoded. This is the per-byte drain tax that
holds compaction jobs at ~110 MiB/s on a disk that streams 381 MiB/s
(run #18 writeprobe), and it is why v21n/v21o parallelism could not help
on a 1-core guest: the work itself was 3× what it needed to be.

RocksDB's `TableBuilder::Finish` constructs the reader from builder
state — no read-back, no verify pass.

**Fix (this commit):** the writer assembles the exact file image once
(one `write_all` instead of five), then constructs the `SstTable`
in-place from that state (index, bloom, bounds, tombstones, counts are
all in hand). Sortedness is now enforced during the encode loop (cheap
memcmp per entry) — the same invariant the post-write verify enforced.
Read-side `open_on` keeps the full verify; lazy block decode still CRCs
blocks on first read, so torn files fail closed exactly as before.

## Finding 3 — hydrate wall decomposition (local 6M, `sample`, 7837 frames)

| share | where |
|---:|---|
| 49% | writer parked in `WriteGroup::await_flush_debt` (thread::sleep) |
| 13% | `Wal::write_pending_frame → preallocate_file → fcntl` (macOS F_PREALLOCATE per 8 MiB chunk) |
| 12% | `commit_async_ops → maybe_auto_flush_best_effort` inline on the writer (take_family/spill/BTree GC) |
| rest | encode, BTree insert, group-commit machinery |

The park is textbook LSM backpressure (Rocks stalls on flush debt too):
the writer sleeps because the drain pipeline is slow — Finding 2 is why
it is slow. Fixing the drain shrinks the park; the park itself is not a
bug. Settle window: 69% of samples in `pwrite` under
`write_l0_sst_for_family` (write-bound, consistent with Finding 2's
syscall profile).

## Finding 4 — parity 2× wins vs slipstream 0.18×: disjoint subsystems

The parity battery (`findings/rocks-parity-floor1x/`, 15/15 ≥ 1.254)
runs `rocksdb-parity-bench` with **`ROCKS_YCSB_RECORDS` default 1024 ×
100 B payload** — a ~100 KiB working set that never leaves the
memtable/cache: zero flushes, zero compactions, zero cold SST reads
during the measured phase. It measures in-memory op throughput (group
commit + BTree + cache), where Pedra wins 1.25–3.65×.

Slipstream hydrate is the opposite regime: 25 M entries, 5.15 GiB
settled, 11.27 GiB written — every wall-second is drain pipeline.
Nothing regressed between the two benches; they measure disjoint
subsystems. The 2× was real (and still holds) for cache-resident ops.

## Still open (ranked, with expected magnitude on the guest)

1. **Write-amp 2.18× → ~1.1–1.5× (bulk-load architecture).** 11.27 GiB
   written for 5.15 GiB settled (L0→L1→L2 pushdown during ingest).
   `SnapshotStore::apply` is a pure sequential ingest and `settle()` is
   the natural manual-compact point — the RocksDB
   `disable_auto_compactions` + final `CompactRange` shape. Direct-to-
   level flush (or L0-skipping during pure-append ingest) halves drain
   work again on top of Finding 2. This is the remaining lever to take
   hydrate/settle to ≥1×; needs an RFC slice.
2. **MANIFEST+CURRENT per install.** ~190 persists per guest run × 2
   barriers × ~12 ms ≈ 4–5 s. Batch installs (one persist per drained
   batch) after the v21o seam.
3. **WAL preallocate chunk 8 MiB → 64 MiB.** 13% of local hydrate in
   `F_PREALLOCATE` (macOS-only cost; Linux `fallocate` is cheap).
   Verify on guest before spending anything.

## Expected guest effect of the two fixes landed here (to verify by injection)

- Read legs lose the 42 s barrier storm → get_hit / lookup_100 /
  prefix_scan should recover toward ≥1× (probe legs improve further).
- Drain jobs stop paying read+CRC+decompress+decode per output byte →
  per-job throughput should move from ~110 MiB/s toward the 200–300
  MiB/s encode+write envelope → hydrate park windows shrink
  proportionally. Hydrate ≥1× still needs lever 1 (write-amp).

## Local A/B (6M, same machine, 2026-08-31)

| leg | baseline (3 runs today) | with both fixes | note |
|---|---:|---:|---|
| hydrate/pedra | 18.6–21.8 s | **16.5 s** (quiet window) | rocks 7.3–8.4 s |
| settle/pedra | 12.7–18.7 s | 13.4–14.1 s | input grew 2.50→2.69–3.06 GiB (schedule shift: faster drain leaves more un-merged L0 at settle entry); disk after settle identical 1.24 GiB |
| FDSYNCDIAG prints | n≥2048 | **none** | barrier storm gone (load-independent) |
| probe_hit p50 | 3.5–5.1 µs | 3.9 µs | rocks 122–153 µs |
| get_hit | 4.4–5.0 µs | 5.2 µs | rocks 40–112 µs (noisy under load) |

Core suite 660/662 + 2 known load-sensitive flakes (`catchup_wait_bounded_by_half_fd`,
`maybe_auto_flush_physical_cf_is_not_linear_in_keys` — both pass in
isolation). No catastrophe: local gate passes; the guest run decides.

Artifacts: `hydrate-pedra.sample`, `settle-pedra.sample`,
`fdcallers.log` (all `/tmp/slip-inject/`), guest captures run16–18 here.
