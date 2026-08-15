# RFC-0026: Value-store evolution — menu (hypothetical)

**Status:** in-progress
**Updated:** 2026-08-14
**Kind:** decision / menu. **Do not implement 0027–0029 together.** P0 measured; pick is **C** (see P0.3).
**Parents:** [0014](0014-rocks-pebble-redwood-maturity.md) P2.2 · [0016](0016-pedradb-production-robustness.md) P0.1 · [0012-research](0012-research-decisions.md)
**Children (options):** [0027](0027-incremental-vlog-gc.md) · [0028](0028-hash-partitioned-value-store.md) · [0029](0029-blob-generations-and-scan-prefetch.md)
**Research:** [R005 WiscKey D4](../../research/fichamentos/ficha_R005_Lu_WiscKey.md) · [R018 HashKV D4](../../research/fichamentos/ficha_R018_Chan_HashKV.md)

**Honesty:** this RFC does not ship a new GC. It names the cost of what we have, the three ways the literature actually improves it, and a measurement gate so a later “yes” is not folklore.

---

## Background

**What Pedra does today (code, not folklore):**

- `OpenOptions::large_value_threshold` spills payloads to `VALUES.vlog` (`len|crc|data`). SST/mem keep `VLG1` + offset + len + crc (`vlog.rs`).
- GC is **`Db::compact_vlog`**: collect *every* live pointer from mem/imm/SST → rewrite a new file → **remap all SSTs that hold a pointer** → MANIFEST `vlog_use_new` → promote. Crash-safe. Operator-triggered.
- WAL is **not** dropped. L5b in the research ledger is `REFUSE` (WiscKey §3.4.2 needs the key inside the vlog record; we do not store it).

**Why this hurts:**

1. Rewrite GC I/O is **O(live bytes + SST bytes that mention a pointer)**, even if only 5% of the log is garbage.
2. It is **stop-the-world** on the single writer (flush first, then rewrite). Fine for a lab soak; not a background tax you can leave on.
3. WiscKey (ficha D4) already said circular-log GC is the hard part. HashKV **measured** a WiscKey-shaped vLog: load WA **1.6×**, then Zipf *update* WA **19.7×** — *worse than RocksDB 7.9× on the same test* (HashKV Fig. 2, p. 1009). KV-separation without a cheap update-GC **gives the WA back**.

**Why now:** L5 was `OPEN`. The ficha closed the *shape* of the gap. These RFCs are the hypothetical fill, for Paulo to accept/kill.

## Problems This Solves

- **Problem:** no written map of “rewrite vs tail-GC vs hash partitions vs blob generations” against *this* kernel (Env, DST, single writer, CRC, WAL kept).
- **Problem:** `compact_vlog` has no live-ratio / cost telemetry, so we cannot decide.
- **Problem:** copying WiscKey tail-GC blindly reintroduces LSM-lookup-per-record and cold-data relocation (HashKV §2.2).

## Proposed Solution

Treat the value store as a **pluggable layout** behind the existing pointer (`VLG1` + coords). Three candidate layouts, one measurement P0:

| Option | RFC | Idea | Wins when | Loses when |
|--------|-----|------|-----------|------------|
| A. Incremental tail | [0027](0027-incremental-vlog-gc.md) | WiscKey head/tail + punch; only remaps the chunk | few deletes; over-provisioned disk; want simplest delta on today’s file | Zipf updates; tail is cold-valid (HashKV’s 19.7×) |
| B. Hash partitions | [0028](0028-hash-partitioned-value-store.md) | Hash(key) → segment group; GC one group; validity = last write in group | update-heavy, hot keys | extra random writes; segment table; more crash surface |
| C. Blob generations | [0029](0029-blob-generations-and-scan-prefetch.md) | Titan/BlobDB-shaped files + discardable ratio; prefetch on scan | many large values; scans; want file-granular delete | more files in Env; manifest of blobs |

**Refuse (already decided, do not reopen in these RFCs):**

- Drop LSM WAL because “vlog has the keys” (L5b).
- Dual row+column in the vlog.
- FPGA / DPU compact of values.

**Default recommendation if we must pick without new benches:** **do 0026 P0**, then prefer **C for file-granular reclaim** (smallest change to crash story: delete a file whose discardable ratio is high, like SST GC) **or B if update-WA is the measured cliff**. A is the smallest *code* delta and the weakest *update* story.

## Delivery slices

### P0 — see the cost (useful alone)

- [x] **P0.1** Stats already on `Db::stats()` (`vlog_bytes`, `vlog_live_bytes`, `vlog_live_records`, `vlog_gc_count`) exposed in CLI/`usage.md` as a one-line “live ratio + last rewrite bytes” — status: `done`
- [x] **P0.2** Bench: load N GB large values, then Zipf overwrite 1×/2×/3×; record rewrite time, bytes_before/after, SST rewrite bytes — status: `done`
- [x] **P0.3** Decision note in this RFC: pick A / B / C / **stay on rewrite** — status: `done` (**C**)

### P0.3 Decision (2026-08-14)

Lab, this machine: 2000 keys × 4 KiB, Zipf *s*=0.99, three overwrite passes, `latest_only` then `compact_vlog`. Raw: `findings/rfc0026-vlog-zipf/stdout.json`.

| What we measured | Value |
|------------------|-------|
| After load | 8.2 MiB file, ratio **0.998** |
| After each Zipf pass + latest_only | 16.4 MiB file, **ratio 0.499**, 2000 live records |
| `compact_vlog` | 16.4 → 8.2 MiB in **86–168 ms**; SST remap ≈ 36 KiB |
| Update+flush+latest_only | **12–22 s** (100× GC) |

This is **not** HashKV Fig. 2 (40 GiB, device WA). At 8 MiB live, rewrite is cheap. The cliff we *did* confirm:

1. Overwrite **doubles** the vlog until SST `latest_only` drops old pointers — then rewrite copies **all live bytes** again. Cost is O(live), independent of how Zipf-hot the updates were.
2. Without `latest_only`, live-ratio stays ~1 and rewrite is a no-op (every version still referenced). Same rule as `usage.md`.

**Pick: C** ([0029](0029-blob-generations-and-scan-prefetch.md)).

- **Not A:** tail-GC is the design HashKV showed collapsing under updates. Our Zipf already creates 50% dead bytes *not* concentrated at the tail (hot keys overwrite in the middle/end). A would relocate cold-live prefix to free that.
- **Not B yet:** best *algorithm* for update-WA, but we did not measure a GB-scale rewrite cliff. Extra crash surface without a number that rewrite is too slow.
- **Not “stay forever”:** rewrite is fine as the hammer at lab size; it will not stay fine when live vlog is tens of GB (every GC recopies the cold majority). C makes GC O(dirtiest *file*) with the MANIFEST/fence we already DST, and adds scan prefetch (WiscKey p. 10).
- **0029 P0 landed** (rotate + `compact_blob` + prefetch N=4). Rewrite remains the file-0 hammer. Do not turn on background rewrite.

### P1 — one child only

- [x] **P1.1** Implement **0029 P0** (blob files + one-file GC + deterministic prefetch) — status: `done`  
- [x] **P1.2** Implement **0029 P1.1** auto worst-ratio `compact_blob_auto` — status: `done`

### P2

- [x] **P2.1** HashKV D4 ficha (R018) — status: `done`. Números 0028 continuam *deles*; L19 MEASURE, 0028 P0 não começa.
- [x] **P2.2** Titan/BlobDB notes from primary source (not blog) if we pick C — status: `done` (`docs/references/titan-options-primary-note.md`)

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | live-ratio visible | done | `DbStats::vlog_line`, `pedra stats` | 2026-08-14 |
| P0.2 | p0 | Zipf overwrite bench | done | `benches/vlog_zipf.rs`, `findings/rfc0026-vlog-zipf/` | 2026-08-14 |
| P0.3 | p0 | pick A/B/C/stay | done | **C** — see below | 2026-08-14 |
| P1.1 | p1 | implement 0029 P0 | done | RFC-0029 P0 | 2026-08-14 |
| P2.1 | p2 | HashKV ficha | done | ficha R018 D4 | 2026-08-15 |
| P2.2 | p2 | Titan primary | done | titan-options-primary-note.md | 2026-08-15 |

## Acceptance Criteria

- **Tests:** none until a child is chosen. P0.2 is a bench script + checked-in JSON under `findings/`, not a unit test.
- **Telemetry / Analytics:** P0.1 is the telemetry. No new metrics backend.
- **Documentation:** this RFC + one paragraph in `docs/usage.md` after P0.1.
- **Screenshots:** backend-only.

## Out of scope

- Implementing A+B+C.
- Changing the threshold spill policy (already correct per WiscKey p. 5/10).
- Multi-writer vlog.
- Object-store backend for values (separate product).
