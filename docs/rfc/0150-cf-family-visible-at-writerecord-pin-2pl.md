# RFC: 0150 — CF-family, visible_at, WriteRecord count, pin∘GC, 2PL deadlock kernels

**Status:** done
**Updated:** 2026-08-29
**Parents:** [0065](0065-physical-column-families-one-wal.md), [0056](0056-one-hundred-percent-delivery.md), crash-dictionary, [0062](0062-launch-readiness-remaining-gaps.md)

**Residual:** glue TCB (not `never_floor`). No extract of `db.rs`. Production callers invoke catalog `entry`.

## Background

- `key_in_cf_family` / `KeyCodec` encode lived in glue. A `lock\0` key treated as `default` is the CF scan leak; compact of lock rewriting default SSTs is the compact leak.
- Crash-dictionary `dictionary_link` stopped at reopen ServeAll; replay→get was an unstated hypothesis (`visible_at`).
- `WriteRecord::decode` loops `count` times but had no named “0 or N, never k” kernel.
- Snapshot-safe GC used `oldest_pinned_sequence().unwrap_or(...)` inline; pin∘`point_version_fate` was not a catalogued decision.
- TransactionDB 2PL wait-for lived only in `locktab.rs` with no twin.

## Problems This Solves

- **Problem:** rocksdb-compat had no catalog pair; CF leak / compact rewrite were tests, not a machine-checked kernel.
- **Problem:** get/range merge at a snapshot was not a named production kernel.
- **Problem:** a hostile/truncated WriteRecord could be described as a silent prefix; that fate was not a twin.
- **Problem:** `auto_reclaim` + a live `SnapshotPin` dropping the pinned version is silent-wrong vs Rocks snapshots.
- **Problem:** 2PL deadlock detection had no AS-IS tooth.

## Proposed Solution

- Extract pure fns production already calls. Sit AS-IS mutants beside them. Catalog pair + Verus twin + script in the same change. No I/O in the new fns. No `db.rs` extract.

## Delivery slices (mandatory)

### P0 — must ship first (CF family)

- [x] **P0.1** `key_in_cf_family` / encode/decode kernel; flush/compact/compat call it — status: `done`
- [x] **P0.2** AS-IS scan leak (`lock\0` in `default`) + compact lock does not rewrite default — status: `done`
- [x] **P0.3** Catalog pair `cf_family` + Verus twin — status: `done`

### P1 — next wave (`visible_at` + InternalKey)

- [x] **P1.1** `visible_at` on get/range merge; F30 AS-IS misses mid-range — status: `done`
- [x] **P1.2** InternalKey pack/unpack/Ord + Kani trailer harness — status: `done`
- [x] **P1.3** dictionary_link replay→get is `visible_at` — status: `done`

### P2 — later (WriteRecord / pin∘GC / 2PL)

- [x] **P2.1** WriteRecord `count` atomic; truncated is Err not prefix — status: `done`
- [x] **P2.2** `gc_oldest_from_pin` is the `oldest_snapshot` bound; pin keeps the version — status: `done`
- [x] **P2.3** 2PL `wait_for_deadlock`; cycle ⇒ Deadlock; AS-IS misses — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CF-family + encode kernel | done | `cf_kernel.rs` | 2026-08-29 |
| P0.2 | p0 | AS-IS leak + compact tooth | done | `key_in_cf_family_as_is` / `compact_range_cf_lock_leaves_default` | 2026-08-29 |
| P0.3 | p0 | catalog + Verus | done | `cf_family` | 2026-08-29 |
| P1.1 | p1 | visible_at + F30 | done | `merge.rs` | 2026-08-29 |
| P1.2 | p1 | pack/Ord + Kani | done | `key.rs` / `scripts/kani_ikey.sh` | 2026-08-29 |
| P1.3 | p1 | dictionary_link tie | done | `verus/dictionary_link.rs` | 2026-08-29 |
| P2.1 | p2 | WriteRecord count | done | `write_record_count_ok` | 2026-08-29 |
| P2.2 | p2 | pin∘GC | done | `gc_oldest_from_pin` | 2026-08-29 |
| P2.3 | p2 | 2PL deadlock | done | `wait_for_deadlock` | 2026-08-29 |

## Acceptance Criteria

- **Tests**
  - `key_in_cf_family` true for raw + `default\0`, false for `lock\0` vs `default`; AS-IS leak true.
  - `key_in_cf_family_on_live_scan_is_not_ok`: live flush; default SST bounds are in-family; lock keys are not in the default SST. Test-side `range_limited` filter is **not** this tooth.
  - `KeyCodec` encode/decode roundtrip (`keycodec_encode_decode_roundtrip_uses_cf_kernel`).
  - `compact_range_cf_lock_leaves_default` / `compact_cf_leaves_other_family_ssts` still pass; compact path calls `key_in_cf_family`.
  - `visible_at` put+delete+range-del; F30 AS-IS misses interior key.
  - pack/unpack identity + Ord seq-desc; Kani harness source in `key.rs`.
  - `write_record_count_ok(3, 3)`; `(3, 2)` false; AS-IS true; truncated decode Err.
  - `gc_oldest_from_pin(Some(pin), …)` is pin; `point_version_fate` Keep; AS-IS Drop. `compact_reclaim_respects_snapshot_pin`.
  - 2PL two-cycle Deadlock; AS-IS false.
- **Telemetry / Analytics:** none — safety kernels.
- **Documentation:** this RFC; coverage-map rows; catalog freeze.
- **Screenshots:** backend-only.

## Out of scope

- Extracting `db.rs`. I/O / Env in the new fns. `∀` ConcurrentDb interleavings. io_uring ring. More CRC twins. OCC iterator read-set tracking. Lean/Aeneas second machine.
