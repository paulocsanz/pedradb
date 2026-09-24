# RFC-0160: Slipstream scale ladder ≥ **2×** RocksDB default (1M–100M)

**Status:** draft
**Updated:** 2026-09-02
**Parents:** [0159](0159-sorted-ingest-bulk-load.md) (bulk ingest / SST v6; hydrate ~1× at 25M),
[0041](0041-2x-rocks-default.md) (peer = Rocks `sync=false`; YCSB harness floor re-baselined 1× — **this RFC is not that harness**),
[0153](0153-ram-scale-block-cache-bytes.md) (block cache is bytes, not whole-file SST)
**Child of 0159 P2.2:** 100M rung moves here.

## Background

- Official peer: RocksDB default (`WriteOptions.sync=false`,
  `ROCKS_PARITY_SYNC=0`). Guest **only** `linux-gate-p149b` (never Darwin).
  Pedra G1 (fdatasync before Ok) is a different column — not a win here.
  Slipstream hydrate is already Rocks `disableWAL` class on the latched
  family (RFC-0159).
- Required set: **hydrate, settle, get_hit, prefix_scan, lookup_100
  get_loop, lookup_100 multi_get** at **1M / 10M / 25M / 100M**.
  `probe_miss` may lose (always-true bloom on bulk files) and is **out**.
- Published win: 3-run median, load start &lt; 17.5, beat host noise
  (~3 s / ~10 % Pedra at load 16–17). One lucky 1.01× or 2.01× is not
  the claim. lookup_100 judges vs Rocks get_loop in the 3.86–4.03 ms
  band at 25M; slow-Rocks 4.5 ms is refuse.
- RFC-0159 closed the hydrate ladder at 25M (Pedra floor 28.1–28.8 s vs
  Rocks 27.9–30.6). It explicitly left **read-leg per-get cost** and
  **100M** as a separate track. This is that track, with the product
  target **≥ 2× on every required cell**.

Honest map, guest 2026-09-02 (v56 1M/10M; v63–v72 25M; 100M SIGKILL):

| n | hydrate | settle | get_hit | prefix | get_loop | multi_get |
|---:|---:|---:|---:|---:|---:|---:|
| 1M | **1.82×** | **2.50×** | **1.44×** | **1.64×** | **1.36×** | **1.08×** |
| 10M | **1.03×** | **7.67×** | 1.000× tie | **1.31×** | **1.07×** | **1.02×** |
| 25M | ~1.0× (not published; median 0.997) | **~7×** | 0.98–1.15 | **1.12–1.28×** | 0.87–1.03 CI overlap | 0.88–1.04 |
| 100M | SIGKILL (WAL-ring / RSS 3.63 GiB) | Rocks 21.5 s | — | — | — | — |

Best honest 25M lookup: v71 r1 get_loop **3.868 vs 3.987 ms = 1.03×**,
CIs overlap. v69 calm-1 in-band was **0.866×**. v72 PointCache freeze
did not close it (0.98× vs a slightly-slow Rocks).

### Arithmetic of ≥ 2×

| hole | now | 2× needs | what cannot close it |
|---|---|---|---|
| 25M hydrate | Pedra 28.6 s vs Rocks 28.3 | Pedra **~14 s** | more cache; FADV_RANDOM (read path) |
| 25M get_loop | 3.87–4.12 ms (38–41 µs/get) | **~1.93 ms** (19 µs/get) | LAST_CF / PointCache / TLS 512 — keys are **fresh every Criterion iteration** |
| 1M multi_get | 1.08× | ~85 % more | same: Pedra `multi_get_cf` is a loop of `get`; Rocks is `batched_multi_get_cf` |
| 10M hydrate | 1.03× | ~2× wall | 10M is already near the 25M rate |
| 100M hydrate | OOM | first: **finish**; then ~57 s vs Rocks ~115 s | pinning 1 GiB whole-file payload (v57 class) |

lookup_100 is 100 independent 4 KiB preads of keys that do not repeat.
Rocks 1 GiB `set_block_cache` is a **block** cache; Pedra mapped it onto
whole-file SST residency and had to cap at 256 MiB on this 3.9 GiB
guest. Cache hits cannot win the arm. A 2× on get_loop is a cut of the
**miss path** (pread + CRC + block walk + compat encode), or a real
batched MultiGet (multi_get arm only).

100M live set is ~25 GiB on disk; guest RAM is 3.9 GiB. Rocks **did**
finish 100M hydrate here (115–120 s). Pedra SIGKILL is a residency bug,
not “100M does not fit the disk.” Random reads at 100M are the same
I/O class as 25M (working set ≫ RAM on both engines).

## Problems This Solves

- **Problem:** the required set is not ≥ 1× at 25M lookup or at 100M
  (100M does not run). Cannot claim 2× on a cell that is 0.87× or OOM.
- **Problem:** RFC-0041’s 2× target was re-baselined to 1× on the YCSB
  harness (fd-ceiling 1-client writes). This slipstream ladder has **no
  per-op fdatasync** on hydrate; 2× is not forbidden by G1 here.
- **Problem:** 100M OOM and 25M lookup have been treated as one “keep
  cutting” pile. They are different allocations / different syscalls.
- **Problem:** without a named 2× gate (3-run, in-band Rocks, load
  &lt; 17.5), a 2.01× on a slow-Rocks day gets published.

## Proposed Solution

1. **Two floors, one peer.** P1 ships **≥ 1×** on every required cell at
   1M/10M/25M/100M (the standing campaign). P2 ships **≥ 2×** on that
   same matrix. Peer stays Rocks default. Guest stays `linux-gate-p149b`.
2. **100M is a residency cap, not a new LSM.** Bound WAL + BulkRun +
   payload so hydrate prints a line on 3.9 GiB; disk peak ≈ live set +
   one chunk (0159 P2.2). Do not raise `sst_payload_budget` to 1 GiB.
3. **lookup ≥ 2× is the miss path.** Profile get_loop into pread / CRC /
   walk / compat **before** the next cache. A real 4 KiB block cache
   (RFC-0153 bytes, not whole-file) may help get_hit’s working set; it
   does not help lookup_100’s fresh keys. Pedra MultiGet that coalesces
   like Rocks is the multi_get lever.
4. **hydrate ≥ 2× is remaining encode+write CPU** at ~14 s / 25M. Pipeline
   chunk N+1 encode against chunk N `write`. Do not re-open lz4/bloom on
   bulk v6 without a guest split.
5. **Kill, don’t round up.** If an isolated 4 KiB `pread` p50 on this
   guest is already ≥ Rocks get_loop/100, 2× lookup is I/O and this RFC
   records the ceiling instead of inventing a CPU story. That is a
   finding, not a silent drop of the cell.

## Delivery slices (mandatory)

### P0 — must ship first (100M runs; lookup hole named)

- [x] **P0.1** This RFC + living status table — status: `done`
- [ ] **P0.2** Attribute 25M get_loop: one guest capture that splits
      Pedra time into pread / block CRC / block walk / compat encode.
      Number before the next cut. — status: `todo`
- [ ] **P0.3** Name the 100M OOM with a guest capture (WAL ring vs
      BulkRun vec vs payload vs page-cache). — status: `todo`
- [ ] **P0.4** 100M hydrate **prints a line** on `linux-gate-p149b`
      (no SIGKILL). Ratio may be &lt; 1×. — status: `todo`

### P1 — ≥ 1× on the required set at all four sizes

- [ ] **P1.1** 25M lookup_100 get_loop **and** multi_get ≥ 1.0, 3-run
      median, Rocks get_loop in 3.86–4.03 ms, load start &lt; 17.5 —
      status: `todo`
- [ ] **P1.2** 25M hydrate 3-run median ≥ 1.0 vs contemporaneous Rocks
      (band 27.9–30.6 s) — status: `todo`
- [ ] **P1.3** 10M get_hit ≥ 1.0 (today a tie) — status: `todo`
- [ ] **P1.4** 100M all six required legs ≥ 1.0, 3-run median —
      status: `todo`

### P2 — ≥ 2× on the same matrix

- [ ] **P2.1** 1M all six ≥ 2.0 (today multi_get 1.08 is the hole) —
      status: `todo`
- [ ] **P2.2** 10M all six ≥ 2.0 — status: `todo`
- [ ] **P2.3** 25M all six ≥ 2.0 — status: `todo`
- [ ] **P2.4** 100M all six ≥ 2.0 — status: `todo`
- [ ] **P2.5** Gate: 3-run median, refuse `sync: true` peer, refuse
      load start ≥ 17.5, `SLIPSTREAM_RATIO_FLOOR=2.0` on the required
      set. — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | this doc | 2026-09-02 |
| P0.2 | p0 | Attribute 25M get_loop | todo | — | 2026-09-02 |
| P0.3 | p0 | Name 100M OOM | todo | — | 2026-09-02 |
| P0.4 | p0 | 100M hydrate finishes | todo | — | 2026-09-02 |
| P1.1 | p1 | 25M lookup ≥ 1× 3-run | todo | — | 2026-09-02 |
| P1.2 | p1 | 25M hydrate ≥ 1× 3-run | todo | — | 2026-09-02 |
| P1.3 | p1 | 10M get_hit ≥ 1× | todo | — | 2026-09-02 |
| P1.4 | p1 | 100M all ≥ 1× | todo | — | 2026-09-02 |
| P2.1 | p2 | 1M all ≥ 2× | todo | — | 2026-09-02 |
| P2.2 | p2 | 10M all ≥ 2× | todo | — | 2026-09-02 |
| P2.3 | p2 | 25M all ≥ 2× | todo | — | 2026-09-02 |
| P2.4 | p2 | 100M all ≥ 2× | todo | — | 2026-09-02 |
| P2.5 | p2 | Floor 2.0 gate | todo | — | 2026-09-02 |

## Acceptance Criteria

- **Tests:** bulk crash-replay and fail-closed CRC unchanged; a 100M
  residency cap has a unit test that a chunked BulkRun + empty payload
  stays under a named RSS budget (does not replace the guest run).
- **Telemetry / Analytics:** every guest run records load start/end,
  Rocks get_loop absolute, Pedra absolute, ratio. Findings under
  `findings/2026-09-02-hydrate-bulk-run/` (or a dated successor).
  Compare refuses `sync: true`. Load start ≥ 17.5 is not a published
  row.
- **Documentation:** this RFC’s status table updates in the **same
  commit** as the code; 0159 P2.2 points here.
- **Screenshots:** none — backend-only.

## Out of scope

- YCSB / `deps_*` harness of RFC-0041 (fd-ceiling 1-client writes stay
  there; this RFC does not re-open the 1× product floor on that column).
- `probe_miss` (always-true bulk bloom).
- Darwin / APFS numbers; sync-peer ratios as wins.
- Raising whole-file SST payload to 1 GiB on the 3.9 GiB guest (v57).
- mmap (forbidden). `unsafe` in `pedradb-core` (forbidden).
- Changing G1: native durable default still fsyncs before Ok; this
  ladder measures the slipstream / disableWAL class against Rocks
  default.

## Kill / ceiling (record, do not round up)

- If P0.2 shows isolated 4 KiB `pread` p50 ≥ Rocks get_loop/100 on this
  guest, **2× lookup is an I/O ceiling**. Ship the number. Do not drop
  the cell from the matrix. A bigger Linux guest is a new RFC, not a
  Darwin escape.
- If P0.3 shows 100M OOM is kernel page-cache + Rocks also cannot
  settle the reads, 100M **read** 2× waits on hardware; hydrate 2× can
  still close.
