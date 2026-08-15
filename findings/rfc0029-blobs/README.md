# RFC-0029 P0 — blob rotate / one-file GC / prefetch

**Date:** 2026-08-14
**RFC:** [0029](../../docs/rfc/0029-blob-generations-and-scan-prefetch.md)

## What shipped

- Pointer `VLG3 | file_num u32 | offset u64 | len u32 | crc u32`. File 0 stays `VALUES.vlog` / `VLG1`.
- `Db::set_vlog_rotate_bytes(Some(n))` — new spills go to `000001.blob`, then the next gen when the active file is ≥ n.
- `Db::compact_blob(file_num)` — rewrite one **sealed** generation; refuse the active file; file 0 delegates to `compact_vlog`.
- Scan resolves vlog pointers in windows of 4 sequential `Env` reads (`scan_prefetch_hits`).

## Bug that failed the first `blob_rotate_reopen`

`get` returned `None` (resolve error swallowed by `.ok()`). Cause: `ValueLog::read_ptr_on` used `self.path` for `file_num == 0`. After rotate, `self.path` is `00000N.blob`, so VLG1 / mixed-mode reads hit the wrong file (len/CRC mismatch).

Fix: `path_for_ptr` — file 0 is `VALUES.vlog` (or `.new` via `use_new`) unless the open handle *is* the legacy file. Rotation mode starts at generation 1 so new spills are not VLG1. `remap_stored_value` ignores VLG3 (offset 8 would otherwise collide).

## Tests

| Test | Checks |
|------|--------|
| `blob_rotate_reopen` | ≥2 blobs, get before/after reopen |
| `blob_rotate_keeps_legacy_vlg1` | VLG1 written before rotate still readable |
| `compact_blob_drops_dead_only` | sealed file GC; live keys survive |
| `compact_blob_crash_after_new_file_keeps_reads` | dest file exists, old pointers still work |
| `scan_prefetch_same_visible_kvs` | scan == get; `scan_prefetch_hits > 0` |
| `read_ptr_on_file0_after_blob_handle` | unit: blob handle reads file 0 |

## Not measured

No new Zipf bench for blob GC (0026 P0 already measured rewrite at 8 MiB). Auto dead-ratio pick is 0029 P1.1.

## Not shipped

0027 (tail GC), 0028 (hash partitions), WAL drop (L5b REFUSE).
