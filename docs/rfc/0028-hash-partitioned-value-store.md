# RFC-0028: Hash-partitioned value store (hypothetical)

**Status:** draft
**Updated:** 2026-08-14
**Parent menu:** [0026](0026-value-store-evolution-menu.md)
**Research:** [R018 HashKV D4](../../research/fichamentos/ficha_R018_Chan_HashKV.md). Numbers below are *theirs* (RAID 6× Plextor + LevelDB 1.20), not Pedra benches. Ficha **não** muda o pick 0026-C: P0 continua gated.

**Do not start P0 until 0026 P0.3 says “B”.**

---

## Background

WiscKey’s circular vLog **must** GC from the tail and **must** query the LSM per record. HashKV’s diagnosis (p. 1007–1009):

1. Tail order relocates **cold-valid** values forever (real workloads are Zipf-hot).
2. LSM gets during GC get expensive as the tree grows.
3. They reproduced it: same KV-separation idea, Update phase WA **19.7×** vs RocksDB **7.9×** (Fig. 2).

Their fix: **hash(key) → fixed main segment** (default 64 MiB) + overflow **log segments** (1 MiB). All versions of one key live in one **segment group**. Append is log-structured *inside* the group.

**Validity without LSM:** last write in the group is the live version (scan group tail→head, first-seen key wins). Temporary in-memory hash table per GC, size bounded by one group — not by the DB.

They then add (optional): reserved-space overflow, hot/cold **tag** so cold values leave the group into a cold log, selective KV-separation (Pedra already has a threshold), write cache (they disable it when they want durability).

Claim vs circular vLog under update-intensive load: **4.6× throughput, 53.4% less write traffic** (abstract). Treat as *their* number until 0026 P0.2 + a Pedra prototype.

**Why this is more interesting than 0027 for Pedra:** our rewrite GC already pays “visit every live pointer”. Hash grouping is the first design that makes GC **O(hottest group)** and **does not require a get() into Pedra’s LSM per record**. That matches single-writer + DST: one group, one crash table.

**Cost we must not hide:**

- Writes to the value store become **hash-scattered** (they batch + RAID to hide it). On one NVMe this is extra random I/O vs today’s single append file.
- Need a **segment table** (in-memory + checkpoint). New recover path.
- Crash: they use a **GC journal** because GC *overwrites* a group (p. 1012). Pedra cannot “just punch a tail”.
- Pointer format must name `(segment_id, offset)` or a stable id — today’s `u64` file offset is not enough across many files.

## Problems This Solves

- **Problem:** update-heavy large-value workloads make circular/rewrite GC as expensive as not separating keys (HashKV Fig. 2).
- **Problem:** we have no GC that can pick the *dirtiest* region instead of the oldest prefix.

## Proposed Solution

- Value store = directory of **main segments** + optional log segments, all through `Env`.
- `hash(user_key) % N_main` (N small and fixed at open; changing N is a rewrite — P2).
- Pointer becomes `VLG2 | seg | off | len | crc` (v1 `VLG1` still decodes).
- GC = pick group with most writes (greedy, HashKV §3.3) → scan → keep latest per key → write a new main (or pack) → MANIFEST/segment-table commit → drop old files. **No full-DB SST rewrite** unless that SST’s pointer targeted this group (remap filter by `seg`).
- Keep Pedra WAL. Keep threshold spill (selective separation is already P0 in spirit).
- **P0 does not include** hot/cold tagging, write cache that skips WAL, or RAID assumptions.

## Delivery slices

### P0 — two segments, one group GC, useful

- [ ] **P0.1** `VLG2` pointer + two main segment files; hash puts; get still works after reopen — status: `todo`
- [ ] **P0.2** `compact_vlog_group(seg)`: latest-version-in-group, remap only SSTs that mention that `seg` — status: `todo`
- [ ] **P0.3** DST crash during group rewrite (same fence rules as today’s `compact_vlog`) — status: `todo`

### P1

- [ ] **P1.1** Overflow log segments when a main is full; segment table checkpoint — status: `todo`
- [ ] **P1.2** Greedy “dirtiest group” picker + stats — status: `todo`

### P2

- [ ] **P2.1** Hot/cold tag (HashKV §3.4) — status: `todo`
- [ ] **P2.2** Change `N_main` via full rewrite — status: `todo`
- [x] **P2.3** R018 HashKV D4 ficha — status: `done`. Re-bench vs 0026 P0.2 só se L19 abrir P0.

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | VLG2 + 2 segments | todo | — | 2026-08-14 |
| P0.2 | p0 | GC one group | todo | — | 2026-08-14 |
| P0.3 | p0 | DST group rewrite | todo | — | 2026-08-14 |
| P1.1 | p1 | overflow log segs | todo | — | 2026-08-14 |
| P1.2 | p1 | dirtiest-group picker | todo | — | 2026-08-14 |
| P2.1 | p2 | hot/cold tag | todo | — | 2026-08-14 |
| P2.2 | p2 | resize N | todo | — | 2026-08-14 |
| P2.3 | p2 | HashKV ficha + bench | done (ficha; bench still todo) | ficha R018 D4 | 2026-08-15 |

## Acceptance Criteria

- **Tests:** `hash_same_key_same_seg`; `group_gc_keeps_latest_only`; `group_gc_does_not_rewrite_other_seg_ssts`; crash/reopen after P0.3.
- **Telemetry / Analytics:** writes_per_seg, gc_group_id, bytes_relocated (P1.2).
- **Documentation:** pointer format + “N is fixed at open”.
- **Screenshots:** backend-only.

## Out of scope

- Dropping WAL or serving gets from a write cache that has not hit WAL (HashKV’s cache is reliability-hostile; we do not want it as default).
- Assuming mdadm RAID-0 (their testbed).
- Replacing LSM indexing with a hash index (they kept LSM *because* of SCAN — p. 1010).
