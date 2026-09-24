# RFC-0029: Blob generations + scan prefetch (hypothetical)

**Status:** done (P0–P2 slices landed; continuous re-measure)
**Updated:** 2026-08-23
**Parent menu:** [0026](0026-value-store-evolution-menu.md)
**Research:** WiscKey §3.3.1 / Fig. 12 (ficha R005 D4) for prefetch. Titan primary: [`titan-options-primary-note.md`](../references/titan-options-primary-note.md).

**P0–P2 shipped** (0026 P0.3 picked C).

---

## Background

Two separate pains, one file-oriented answer:

1. **GC granularity.** Rewrite (today) and tail-GC (0027) both live inside *one* growing file. The cheap primitive we already trust for SSTs is **drop a whole file** after a MANIFEST swap. Titan/BlobDB do that for values: many immutable **blob files**; GC = rewrite *one* blob whose *discardable ratio* is high; pointers in LSM name `(blob_file, offset)`.

2. **Scan of large values.** WiscKey p. 10: random-fill + 64 B values → **12× worse** than LevelDB on a 4 GB range, because values are not in key order. Their fix is **parallel prefetch** (32 threads, `posix_fadvise`). HashKV §3.6 does the same with read-ahead after iterating keys. Pedra `scan` today resolves `VLG1` one-by-one on the single writer — worst of both worlds if we ever spill medium values.

This RFC is the “looks like our SST world” option: more files, same Env/MANIFEST discipline, no hash table, no circular tail.

**Why it might beat 0027 for Pedra:** we already know how to publish `tmp + fsync + rename + MANIFEST` (SST, vlog `.new`). A blob file is another SST-shaped object. Discardable ratio is a **local** statistic (dead bytes / file size) we can maintain when a key is overwritten (increment dead on the old blob id). No LSM get storm, no “tail is all cold-valid”.

**Why it might lose to 0028:** a hot key that updates forever dirties *many* blob files (one record each) unless we also separate hot/cold (Titan has GC + blob cache; we would need a policy). Hash grouping keeps all versions of a hot key in one group on purpose.

## Problems This Solves

- **Problem:** cannot drop garbage without reading a giant `VALUES.vlog`.
- **Problem:** range over spilled values is a serial random read (WiscKey’s documented cliff).

## Proposed Solution

- Roll `VALUES.vlog` into **numbered blob files** (`000001.blob`, size cap e.g. 64 MiB) when the active file hits the cap. Pointer `VLG3 | file_num | off | len | crc`.
- On overwrite/delete of a spilled key, bump `dead_bytes[file_num]`.
- **P0 GC:** pick the file with highest dead ratio above θ (e.g. 0.5); rewrite *that file’s still-live records* into a new file (or into the active file); remap only SSTs that mention `file_num`; drop the old blob. Same two-phase fence as `compact_vlog`.
- **P0 prefetch (independent, can ship even if GC stays rewrite):** when `range`/`scan` sees a run of `VLG*` pointers, issue batched `Env` reads (sequential in iterator, *N* in-flight — start N=4, not 32) before decoding values. Single-threaded completion to keep DST deterministic (no thread pool in core).
- WAL stays. Threshold spill stays.

## Delivery slices

### P0 — two useful independents

- [x] **P0.1** Rotate blob file at cap; get/reopen; discover `*.blob` by name — status: `done`
- [x] **P0.2** `compact_blob(file_num)` rewrites one sealed gen; crash-after-new-file keeps reads — status: `done`
- [x] **P0.3** Scan prefetch window N=4 (single-threaded Env reads); order/visibility unchanged — status: `done`

**P0 deviations / honesty**

- Blob inventory is **directory listing** (`000001.blob`), not a MANIFEST field (same discovery as `VALUES.vlog`). MANIFEST list is P1 if orphans become a problem.
- Rotate cap is a **session setter** (`Db::set_vlog_rotate_bytes`), not `OpenOptions` (too many struct literals). Reopen appends to the last gen; caller must set the cap again to keep rotating.
- Prefetch is a window of **sequential** Env reads (DST-deterministic). Not a thread pool, not `posix_fadvise`.
- `compact_blob` is operator-triggered on a file id. Auto worst-ratio is P1.1. Active file is refused.
- Enabling rotation starts new spills at `000001.blob`. Pre-existing `VLG1` / `VALUES.vlog` stays readable (`read_ptr_on` must not use the active blob path for file 0).

### P1

- [x] **P1.1** Auto-pick worst dead_ratio file (operator still can pass an id) — status: `done`
- [x] **P1.2** `posix_fadvise`-shaped `Env::advise` (optional; no-op on sim) — status: `done`
  Linux `posix_fadvise` is `pedradb-posix::advise_file` so `pedradb-core` stays
  `#![forbid(unsafe_code)]`. `StdEnv` implements it (2026-08-23); sim / DST
  inherit the trait no-op. `IoUringEnv` delegates to the same safe wrapper.

### P2

- [x] **P2.1** Titan/BlobDB primary-source note (not a blog) if we keep C — status: `done`
- [x] **P2.2** Prefetch N from measure + setter (not magic 32) — status: `done`

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | rotating blob files | done | `vlog.rs` VLG3 + `set_vlog_rotate_bytes` | 2026-08-14 |
| P0.2 | p0 | GC one blob file | done | `Db::compact_blob` | 2026-08-14 |
| P0.3 | p0 | deterministic scan prefetch | done | `prefetch_resolve_stream` N=4 | 2026-08-14 |
| P1.1 | p1 | auto worst-ratio | done | `blob_gc_candidates` + `compact_blob_auto` | 2026-08-15 |
| P1.2 | p1 | Env advise | done | `AdviseKind` + `Env::advise`; Linux `posix_fadvise` in `pedradb-posix`; `StdEnv` + `IoUringEnv` call it | 2026-08-23 |
| P2.1 | p2 | Titan primary note | done | `docs/references/titan-options-primary-note.md` | 2026-08-15 |
| P2.2 | p2 | prefetch N from bench | done | `set_scan_prefetch` + `scan_prefetch_n_window_measure` | 2026-08-15 |

## Acceptance Criteria

- **Tests:** `blob_rotate_reopen`; `compact_blob_drops_dead_only`; `scan_prefetch_same_visible_kvs`; crash after blob MANIFEST / before drop.
- **Telemetry / Analytics:** blob_files, dead_ratio_max, prefetch_hits (P0.3 can be a counter).
- **Documentation:** file names + pointer version.
- **Screenshots:** backend-only.

## Out of scope

- Hash partitions (0028).
- Thread pool inside `pedradb-core`.
- Putting cold blobs on object storage (product later).
