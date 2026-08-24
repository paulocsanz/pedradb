# Crash dictionary spec (RFC-0053 P0.3 + Y3.3)

**Status:** spec + DST teeth + reopen-outcome lemmas (not a ∀ proof of `Db`)  
**Updated:** 2026-08-23  
**Shape:** VeriBetrKV OSDI’20 — dictionary that, on crash, reverts no farther than last `sync`. Crash is an IOSystem transition (Hance), not Crash-Hoare.  
**CRC:** forging a valid CRC32C is in the TCB (same as VeriBetrKV).  
**Proof of recover *choices*:** `wal_recover` Verus twin — RFC-0053 P1.3.  
**Proof of *reopen outcomes*:** `reopen_outcome` Verus twin — RFC-0053 Y3.3 (`wal/reopen_kernel.rs` chamado por `Db::open_with_env`; `6 verified`). This page does **not** claim `db.rs` ∀.

---

## Spec (closed form)

Let \(D\) be the map a client observes after `open` (point `get` of user keys).  
Let \(A\) be the set of keys for which some `put`/`delete`/`commit` returned **Ok** under `OpenOptions.sync = true` (acked prefix).  
Let a **crash** be: process gone; WAL/SST/MANIFEST bytes as left on the `Env`; then `open` again.

```text
crash_dictionary:
  after crash-and-reopen,
    ∀ k ∈ A.  D(k) = the last acked value of k
    (delete acked ⇒ get(k) = None)

  unacked suffix may vanish (torn tail / lost unsynced write).
  CRC mismatch at a fresh record boundary ⇒ open fail-stops (not silent skip).
  orphan WAL Middle/Last ⇒ fail-stop (F14).
```

Axioms (never theorems):

- `Env::sync_data` / `sync_all` that returned Ok left those bytes durable.  
- If the OS lies, DST/det_io/TCG (RFC-0052) — the spec does not hold.

---

## Teeth (existing tests — not new proofs)

These tests **are** the spec’s executable dentes. If one goes red, the spec is violated in the lab. They do not discharge ∀.

| Tooth | Where | What it refuses |
|-------|-------|-----------------|
| Crash after Ok+sync | `pedradb-sim::scenario_crash_after_sync_survives` (`crash_after_sync_recovers_committed`) | acked key missing after process-style kill |
| Same, core | `pedradb-core` `crash_after_sync_put_reopen_recovers` | same on `Db` |
| Truncated tail | `pedradb-sim::scenario_truncated_tail_loses_unsynced_suffix` (`truncate_wal_drops_tail_keeps_prefix`) | durable prefix lost **or** truncated tail resurrected |
| CRC fail-stop | `pedradb-sim::explode_choose_crc_fail_stops_reopen` | CRC flip at a real record → silent skip / wrong value |
| Silent-wrong gate | `pedradb-dst` `rfc20_silent_wrong_gate_matrix` / `scripts/ci_silent_wrong_gate.sh` | any seed in the matrix loses the durable seed key |
| WAL recover kernel | `recover_kernel.rs` + Verus `wal_recover.rs` (`13 verified`, RFC-0053 P1.3) | torn = EOF silencioso; CRC/ZeroHeaderTail em alinhamento fresco = fail-stop; AS-IS `lemma_as_is_zero_header_silent_eof` |

Run:

```sh
cargo test -p pedradb-sim crash_after_sync_recovers_committed
cargo test -p pedradb-sim truncate_wal_drops_tail_keeps_prefix
cargo test -p pedradb-sim explode_choose_crc_fail_stops_reopen
cargo test -p pedradb-dst rfc20_silent_wrong_gate_matrix
```

---

## Reopen outcomes (RFC-0053 Y3.3 — machine-checked)

The recover choices above feed the **`Db` reopen path**. The mapping
damage → outcome is a pure kernel production calls
(`wal/reopen_kernel.rs`, `Db::open_with_env`), with named Verus lemmas
(`verus/reopen_outcome.rs`, `6 verified`, no `sorry`):

| Lemma | Statement |
|-------|-----------|
| `lemma_recover_failstop_routes_to_damage` | every fail-stop recover kind (CRC / ZeroHeaderTail at fresh alignment / head-Truncated) routes the reopen into a damage arm |
| `lemma_damaged_reopen_never_silent` | damaged reopen ⇒ `RefuseOpen` ∨ `ServePrefixReport` — never `ServeAll` (G8) |
| `lemma_fail_closed_refuses_damage` | FailClosed + damage ⇒ `RefuseOpen` (the visible map never silently drops an acked suffix) |
| `lemma_clean_reopen_serves_all` | no damage ⇒ `ServeAll` (no false refusal) |
| `lemma_mutant_swallows_damage` | AS-IS swallow-damage serves a damaged WAL silently — exactly what the fixed kernel refuses (teeth) |

PointInTime (`ServePrefixReport`) publishes a `RecoveryReport` — the
discard is observable, never silent. Escalation (RFC-0038 D) wins over the
permissive profile: escalated ⇒ `RefuseOpen`.

---

## MANIFEST recovery (RFC-0056 P0.1 — machine-checked)

The SST-inventory reopen (`recover_ssts` on `Db::open_with_env`) routes its
decision through a pure kernel (`manifest_kernel.rs`), with named Verus
lemmas (`verus/manifest_recover.rs`, `9 verified`, no `sorry`):

| Lemma | Statement |
|-------|-----------|
| `lemma_damaged_inventory_never_scans` | Corrupt CURRENT/MANIFEST (bad contents, dangling CURRENT, unsupported version) ⇒ `RefuseOpen` — never a silent directory scan (a scan resurrects GC'd files) |
| `lemma_missing_listed_sst_refuses` | committed inventory listing a missing SST ⇒ `RefuseOpen` — the inventory is ground truth |
| `lemma_absent_inventory_scans` | absent inventory (CURRENT missing or torn-empty) ⇒ `ScanAndInstall` — first open proceeds (no false refusal) |
| `lemma_committed_inventory_serves` | inventory + all listed files present ⇒ `ServeInventory` |
| `lemma_first_install_unsynced_tolerated` | F196: `CommittedUnsynced` first install ⇒ `Proceed`; any other failure ⇒ `RefuseOpen` |
| `lemma_mutant_scan_resurrects_gc` | teeth: AS-IS swallow maps Corrupt ⇒ silent `ScanAndInstall` |
| `lemma_mutant_first_install_serve_failed` | teeth: AS-IS swallow proceeds after `Failed` first install |

Teeth on disk: `open_gcs_orphan_sst_not_in_manifest`,
`unsynced_l0_torn_sst_recovers_from_wal`, and the kernel finite-domain
theorem `theorem_sst_recover_on_finite_domain` (mutant must diverge on
every damaged input). Catalog: `manifest_recover`
(`data_fate: true`, handler `recover_ssts`, lint fail-closed).

---

## Flush pipeline (RFC-0056 P0.2 — machine-checked)

`Db::flush` / `Db::try_rotate_wal` / `Db::ensure_wal_rotated_for_gc`
decide through `flush_kernel.rs`, with named Verus lemmas
(`verus/flush_decision.rs`, `10 verified`, no `sorry`):

| Lemma | Statement |
|-------|-----------|
| `lemma_tail_never_dropped` | non-empty mem ⇒ the plan writes an SST; never `RotateOnly` (rotating instead loses every acked key that lived only in mem) |
| `lemma_pending_imm_finishes_first` | a pending imm finishes before mem is staged (single-flight) |
| `lemma_pin_keeps_wal` | live flush read pin ⇒ `KeepWal` — the pin (and an in-flight SST) may hold the only copy of acked keys (the pre-fix hole `rotate_wal_ignoring_pin` replays) |
| `lemma_commit_inflight_keeps_wal` | commit inside the off-lock fsync window owns WAL bytes (F2) |
| `lemma_unflushed_mem_keeps_wal` | unflushed acked keys in mem pin the WAL |
| `lemma_mutant_loses_tail` | teeth: AS-IS flush-nothing rotates without writing |
| `lemma_mutant_ignores_pin` | teeth: AS-IS truncates exactly when the pin is live |

Teeth: `flush_kernel.rs` finite-domain theorems (2×2 plan, 2⁵ rotate
state) with the mutants asserted to diverge on every damaged input.

---

## Compaction (RFC-0056 P0.3 — machine-checked)

`Db::compact_with_ssts_only` (trigger / level choice) and
`merge::gc_snapshot_safe` (version retention) decide through
`compact_kernel.rs`, with named Verus lemmas
(`verus/compact_decision.rs`, `10 verified`, no `sorry`):

| Lemma | Statement |
|-------|-----------|
| `lemma_drop_needs_newer_visible_to_all_snaps` | an older version drops only when its newer sibling has seq ≤ oldest open snapshot — no open snapshot can read the dropped version |
| `lemma_snapshot_between_versions_keeps` | newer sibling not visible to the oldest pin ⇒ the version is kept (compaction never walks over a pin) |
| `lemma_newest_version_always_kept` | newest version of a key survives every GC |
| `lemma_partial_compact_keeps_tombstone` | F177: not bottommost ⇒ lone tombstone kept (dropping it resurrects the older version outside the input, durably after reopen) |
| `lemma_merge_moves_one_level_down` | trigger merges the lowest non-empty level into `from + 1` |
| `lemma_mutant_drops_pinned_version` | teeth: AS-IS drops by own seq (this ≤ oldest < newer) — compacts over a pinned snapshot |
| `lemma_mutant_resurrects_over_partial_compact` | teeth: AS-IS drops the lone tombstone without bottommost |

Teeth: `compact_kernel.rs` finite-domain theorems (seq space 0..=4 for
retention, 2×2 tombstone, 4×2×2 trigger) with both mutants asserted to
diverge exactly on the pinned-snapshot / partial-compaction inputs.

---

## Dictionary composition (RFC-0056 P1.1 — machine-checked)

The put→get link over the existing kernels, with the persist axiom as a
**named hypothesis** (`verus/dictionary_link.rs`, `8 verified`, no `sorry`):
A1 `persist_axiom_holds` (acked ⇒ WAL prefix), A2 well-formed WAL,
A3 recovery returns the WAL, A4 the unacked tail carries no newer version of
the key. Top theorem `lemma_put_acked_survives_crash`: under A1–A4 +
domination, `reopen_outcome = ServeAll` and `get` returns the acked value.
Teeth: `lemma_mutant_torn_tail_is_silent`,
`lemma_mutant_reopen_swallows_damage`. Exercised end-to-end on a real `Db`
by `crash_after_flush_and_tail_put_recovers_both_paths`.

---

## vlog GC swing (RFC-0056 P1.4 — machine-checked)

`Db::open_with_env` (vlog handle) decides through `vlog_gc_kernel.rs`,
with named Verus lemmas (`verus/vlog_gc_decision.rs`, `14 verified`, no
`sorry`):

| Lemma | Statement |
|-------|-----------|
| `lemma_swing_opens_staged_new` | MANIFEST committed the swing + `.new` staged ⇒ open `.new` (remapped SST pointers stay readable) |
| `lemma_orphan_new_never_opened` | crash before the MANIFEST commit leaves an orphan `.new`; it is never opened |
| `lemma_promote_done_reconciles_primary` | crash after the promote rename reconciles to primary |
| `lemma_f51_refuses_both_missing` | committed swing + both files gone ⇒ refuse (inventing an empty primary makes large values vanish) |
| `lemma_fresh_db_creates_empty_primary` | no vlog anywhere + no swing ⇒ create empty primary |
| `lemma_active_generation_never_rewritten` | sealed-blob GC never rewrites the active append generation (concurrent appends would vanish) |
| `lemma_mutant_serves_stale_primary_after_swing` | teeth: AS-IS ignore-swing serves the stale pre-GC primary after a crash mid-GC |
| `lemma_mutant_invents_empty_primary_on_f51` | teeth: AS-IS invents the empty primary exactly on the F51 input |
| `lemma_mutant_rewrites_active_generation` | teeth: AS-IS rewrites the active generation |

---

## 2PC per-range cleanup (RFC-0056 P1.4 — machine-checked)

`StoreCluster::tx_finish` cleanup decides through `tx_glue_kernel.rs`,
with named Verus lemmas (`verus/tx_glue.rs`, `7 verified`, no `sorry`):

| Lemma | Statement |
|-------|-----------|
| `lemma_committed_range_gets_majority_revert` | F47: a majority-committed range in a failed TX gets a majority `TxnRevert` on the same raft log |
| `lemma_uncommitted_range_gets_local_revert` | F34: a never-committed range reverts locally |
| `lemma_success_keeps_every_range` | no false reverts on success |
| `lemma_mutant_leaves_majority_apply_visible` | teeth: AS-IS local-only cleanup leaves the partial apply visible forever |

---

## What this is not

- Not a Verus `ensures` on `Db::put`.  
- Not coverage of `ConcurrentDb` group-commit (RFC-0051 / TCB fora).  
- Not “fsync proved.”  
- Not SST/vlog composition ∀ (the MANIFEST/flush/compaction decisions
  above are the inventory-reopen and merge legs; vlog GC lands in
  RFC-0056 P1.4).
