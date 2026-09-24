# Titan (TiKV) — primary-source note for Pedra blob GC (RFC-0026 P2.2 / 0029 P2.1)

**Date:** 2026-08-15  
**Primary source (not a blog):**  
[`include/titan/options.h`](https://github.com/tikv/titan/blob/master/include/titan/options.h) on `tikv/titan@master`  
(README points to a PingCAP design blog; that blog is **secondary** — not used as authority here.)

## What Titan is (from tree + options)

Titan is a **RocksDB plugin** for key–value separation (WiscKey-inspired): values above a threshold land in **blob files**; LSM holds indexes. GC and blob layout are controlled by options that map cleanly onto Pedra’s 0029 shape.

## Options that matter for Pedra’s pick C (0029)

| Titan option | Default (source) | Pedra analogue |
|--------------|------------------|----------------|
| `min_blob_size` | 4096 | `large_value_threshold` spill |
| `blob_file_target_size` | 256 MiB | rotate cap (`set_vlog_rotate_bytes`) |
| `blob_file_discardable_ratio` | **0.5** | `compact_blob_auto(min_dead_ratio)` θ |
| `merge_small_file_threshold` | 8 MiB | (not yet) small-file merge |
| `disable_background_gc` / `max_background_gc` | false / 1 | Pedra: **operator / explicit** GC only (no bg thread in core) |
| `blob_cache` | null | (not yet) |
| `level_merge` / `range_merge` | false | scan-oriented rewrite of blobs with Lsm levels — **out of P0/P1** |
| `enable_punch_hole_gc` + `block_size` | false / 4096 | hole-punch GC (0027-ish); Pedra stays rewrite-file for now |
| `TitanBlobRunMode::{Normal,ReadOnly,Fallback}` | Normal | Fallback ≈ “inline values back into SST” |

## Design takeaways (primary, not marketing)

1. **File-granular GC with discardable ratio** is the production default shape (θ=0.5). Pedra’s `blob_gc_candidates` + `compact_blob_auto(0.5)` matches that knob name/semantics.
2. **Target blob size ~256 MiB** is a *wish*, not a hard wall — same as our soft rotate cap.
3. **Background GC is optional and bounded** (`max_background_gc`); Pedra deliberately keeps GC off the hot path (DST surface).
4. Titan’s advanced paths (`level_merge`, punch-hole GC) buy scan/WA tradeoffs at cost of space/write amp — only revisit if 0029 rewrite-one-file shows a real cliff at multi-GB live vlog (0026 P0.3 already said rewrite is fine at lab MiB scale).

## What we did *not* take as evidence

- PingCAP blog “Titan storage engine design and implementation” — useful narrative, not primary API/contract.
- HashKV Fig. 2 numbers (other paper; already fichado R018) — not Titan.

## Implication for Pedra

Keep **0029 C**: rotate blobs + one-file GC + auto θ + prefetch. Do **not** start 0027 (tail GC) or 0028 (hash partitions) until a measured multi-GB rewrite cliff or explicit product need. Titan options reinforce that **discardable_ratio 0.5** and **explicit/limited GC** are field-proven knobs, not invention.
