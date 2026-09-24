# RFC-0056 P0 engine report

**Date:** 2026-08-23  
**Status:** P0.1–P0.3 landed (MANIFEST recovery, flush, compact as kernels + twins + mutants + catalog). **Not** a ∀ crash-dictionary on `Db`. **Not** “Pedra verificado.”  
**Wave:** RFC-0056 P0 (engine LSM decisions), following the RFC-0053 sandwich.

## LOC (this wave)

| Object | Lines |
|--------|------:|
| Production kernels (`manifest_kernel` + `flush_kernel` + `compact_kernel`) | 839 |
| Verus twins (`manifest_recover` + `flush_decision` + `compact_decision`) | 703 |
| `db.rs` recovery wiring (`recover_ssts`, `flush`, `try_rotate_wal`, `compact_with_ssts_only`) | ~200 |
| `merge.rs` retention wiring (`gc_snapshot_safe`) | ~25 |

Proof:kernel ≈ 703/839 ≈ **0.84 : 1** (Verus twins; the kernels carry
their own finite-domain theorem tests as additional teeth). Verus
results: manifest_recover `9 verified`, flush_decision `10 verified`,
compact_decision `10 verified` — all `0 errors`, zero `sorry`.

## What each kernel decides (data-fate decisions that left `db.rs`/`merge.rs`)

| Kernel | Decision | Was |
|--------|----------|-----|
| `sst_recover_action` + `first_install_action` | damaged/absent inventory → serve / scan+install / refuse; F196 first-install tolerance | inline `if let Some(vs)` + ad-hoc error arms in `recover_ssts` |
| `flush_plan` | which flush step runs (finish imm / write SST / rotate only) | inline sequence at the top of `Db::flush` |
| `wal_rotate_decision` | may the WAL be truncated (mem, imm, pin, parked, commit-in-flight) | inline guard in `try_rotate_wal` / `ensure_wal_rotated_for_gc` |
| `compact_pick` | trigger + level choice (lowest non-empty < max; GC rewrite of max only on request) | inline level loop in `compact_with_ssts_only` |
| `point_version_fate` | snapshot-safe retention of one version | inline `newer_seq <= oldest` in `gc_snapshot_safe` |
| `lone_tombstone_fate` | F177 bottommost guard for lone tombstones | inline `bottommost && len==1 && Deletion` |

## Mutants (teeth, all asserted in-repo)

| Mutant | Silent-wrong it replays | Where asserted |
|--------|------------------------|----------------|
| `sst_recover_action_as_is_scan_on_damage` | corrupt CURRENT ⇒ silent directory scan (resurrects GC'd files, serves an inventory that was never committed) | `theorem_sst_recover_on_finite_domain` |
| `first_install_action_as_is_proceed_always` | serves scanned inventory after a failed first install | same |
| `flush_plan_as_is_lose_tail` | flush "succeeds" without writing the mem tail; the rotate then truncates the only durable copy | `theorem_flush_plan_on_finite_domain` |
| `wal_rotate_decision_as_is_ignore_pin` | truncates the WAL while the flush pin holds the only copy of acked keys (pre-fix hole, `rotate_wal_ignoring_pin`) | `theorem_wal_rotate_on_finite_domain` (2⁵ states) |
| `point_version_fate_as_is_drop_under_snapshot` | drops a version by its own seq — compacts over a pinned snapshot | `theorem_point_version_fate_on_finite_domain` (seq 0..=4) |
| `lone_tombstone_fate_as_is_ignore_bottommost` | drops a lone tombstone in a partial compaction — the older version outside the input resurrects (F177) | `theorem_lone_tombstone_on_finite_domain` |

## Catalog (fail-closed)

New entries: `manifest_recover` (handler `recover_ssts`), `flush_decision`
(handlers `flush`, `try_rotate_wal`), `compact_decision` (handler
`compact_with_ssts_only`), `compact_retention` (handler `gc_snapshot_safe`).
All `data_fate: true`. Lint: **58 ok, 0 fail** (`--lint`), **173 ok, 0
fail** (`--ci`). Negative tests executed: renaming each handler to
`*_MISSING` makes the lint FAIL (2 FAIL lines for the compact pair, 1 for
each single-handler entry), restored green after.

## TCB delta (RFC-0056 P0)

**Moved into the proof:** the engine LSM reopen/flush/compact decisions
above — pure kernels production calls, with Verus twins (29 named
verification results this wave, 0 errors, 0 `sorry`).

**Still caller + axiom (unchanged):** bytes on disk, decode/CRC, orphan
GC execution, SST write + install, MANIFEST store/swing, fsync ordering
(`persist_manifest_durable` before WAL truncate stays G1 in
`rotate_wal_now` — enforced by the caller, teeth via DST), `ConcurrentDb`
group-commit, io_uring, libc.

**Explicitly out:** a ∀ composition theorem across
put→WAL→SST→MANIFEST→get (that is RFC-0056 P1.1, next wave); vlog GC and
2PC (P1.4).

## Telemetry

- `scripts/verus_manifest_recover.sh`: 9 verified, 0 errors (~1.8 s)
- `scripts/verus_flush_decision.sh`: 10 verified, 0 errors (~1.5 s)
- `scripts/verus_compact_decision.sh`: 10 verified, 0 errors (~1.6 s)
- `cargo test -p pedradb-core --lib` (kernel + recovery + flush/compact/merge filters): 13 + 128 + 100 passed, 0 failed
