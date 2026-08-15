# RFC-0027: Incremental tail GC for `VALUES.vlog` (hypothetical)

**Status:** draft
**Updated:** 2026-08-14
**Parent menu:** [0026](0026-value-store-evolution-menu.md)
**Research:** WiscKey §3.3.2 (ficha R005 D4). HashKV Fig. 2 / §2.2 (ficha R018 D4) is the **counter-argument**.

**Do not start this RFC’s P0 until 0026 P0.3 says “A”.**

---

## Background

Today `compact_vlog` rewrites the **entire** live set and remaps every SST that holds a `VLG1` pointer. WiscKey instead keeps one circular file with `head` (append) and `tail` (GC cursor):

1. Read a chunk (several MB) from the tail.
2. For each record, ask the LSM if that key’s pointer still names this offset.
3. Re-append *valid* records at the head; fsync vLog; persist new addresses + new tail **in the LSM**; then punch the freed prefix.

That only remaps the survivors of **one chunk**, not the whole tree.

Pedra cannot copy the recipe verbatim:

- Our records are `len|crc|data` — **no key in the record**. Validity today is “offset appears in mem/imm/SST”, which is a full collect (what rewrite already does), not a per-record LSM get unless we **add the key to the record** (v2 layout).
- We refused dropping the WAL (L5b). Incremental GC must not depend on “vlog is the recover path”.
- Hole-punch (`fallocate`) is not an `Env` primitive today. DST cannot inject it until it is.

HashKV’s own vLog clone shows the failure mode we must not ignore: after Zipf updates fill the reserved space, circular GC WA went to **19.7×** (Fig. 2) because the tail is often still valid / cold.

## Problems This Solves

- **Problem:** rewrite GC cost grows with live data, not with garbage.
- **Problem:** no way to reclaim a prefix of `VALUES.vlog` without rewriting SSTs that only point into the *kept* suffix.

## Proposed Solution

- **v2 record (opt-in, new magic or version byte):** `(key_len, val_len, crc, key, value)`. Old files stay v1; GC of v1 remains rewrite.
- **Cursors** `vlog_head` / `vlog_tail` in MANIFEST (not as user-visible LSM keys named `"tail"` — avoid colliding with user keyspace).
- **P0 useful slice:** `compact_vlog_prefix(bytes)` — GC exactly one chunk from tail, remap only pointers that moved, punch or truncate-front **if Env grows a `punch`/`truncate_front`**. If Env cannot punch, copy the kept suffix to `.new` *but only the prefix-GC’d file region* (still cheaper than full live rewrite when garbage is at the front).
- **Foreground:** still single-writer; P0 is operator-triggered (same as today’s compact). Background / rate-limit is P2.
- **Refuse in this RFC:** hash partitions (that is 0028); dropping WAL; punching without an Env hook.

## Delivery slices

### P0 — one chunk, crash-safe, useful

- [ ] **P0.1** v2 record includes key; v1 still readable — status: `todo`
- [ ] **P0.2** `compact_vlog_prefix(n_bytes)`: validate via key→current pointer (mem/imm/SST get), re-append valids, persist tail in MANIFEST, remap only moved offsets — status: `todo`
- [ ] **P0.3** DST: crash after append-valids / after MANIFEST tail / after punch; no pointer at punched offset; no tail past live data — status: `todo`

### P1

- [ ] **P1.1** `Env::punch` or portable truncate-front; sim/FailingEnv — status: `todo`
- [ ] **P1.2** Stats: bytes_punched, records_relocated, lsm_lookups_during_gc — status: `todo`

### P2

- [ ] **P2.1** Background / periodic trigger with a live-ratio threshold — status: `todo`
- [ ] **P2.2** Kill-switch if Zipf bench (0026 P0.2) shows WA ≥ rewrite — status: `todo`

## Status (living)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | v2 record with key | todo | — | 2026-08-14 |
| P0.2 | p0 | prefix GC one chunk | todo | — | 2026-08-14 |
| P0.3 | p0 | DST crash table | todo | — | 2026-08-14 |
| P1.1 | p1 | Env punch | todo | — | 2026-08-14 |
| P1.2 | p1 | GC counters | todo | — | 2026-08-14 |
| P2.1 | p2 | background trigger | todo | — | 2026-08-14 |
| P2.2 | p2 | WA kill-switch | todo | — | 2026-08-14 |

## Acceptance Criteria

- **Tests:** `compact_vlog_prefix_reclaims_after_overwrite`; `prefix_gc_mid_manifest_reopen`; `prefix_gc_does_not_move_untouched_sst_files` (SSTs whose pointers all sit after the new tail are **not** rewritten).
- **Telemetry / Analytics:** P1.2 counters on `Db::stats()`.
- **Documentation:** `usage.md` — when to call prefix vs full rewrite.
- **Screenshots:** backend-only.

## Out of scope

- Hash grouping, blob files, scan prefetch.
- Making tail-GC the *only* GC (keep rewrite as the “make it small now” hammer).
