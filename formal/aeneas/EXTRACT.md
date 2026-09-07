# Aeneas extract + Lean theorems

**Date:** 2026-08-15

## Vote (`vote_kernel.rs`)

- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/VoteKernel.lean`.
- Production tweaks so the extract is not an axiom: `can_vote` and `grant_after_persist` use `match`.
- Lean 4.31.0 accepted (no `sorry` in `Vote.lean`):
  - `vote_decision_matches_spec`
  - **`vote_decision_iff`** (RFC-0053 P40: iff on the extracted term)
  - `grant_after_persist_implies_ok` (wire grant ⇒ persist Ok)
  - `as_is_grants_where_fixed_denies` (mutant teeth)
- **P40 Option::eq:** `VoteKernel.lean` models `Option::eq` as `match` (`def`, not `axiom`). `pedra_formal.py` fails if an Aeneas re-extract restores the axiom.

## Bloom (`bloom.rs`, RFC-0030)

- `[lib] path` = production `crates/pedradb-core/src/bloom.rs`.
- Production tweaks so the extract typechecks / is not an axiom on the T1/T4
  path (same method as Isolated/`starts_with`):
  - `bit_index`: bound + cast, not `try_from`/`unwrap_or`.
  - `is_active`: `len() != 0`, not `!is_empty()`.
  - `with_capacity`: explicit `saturating_product` / `at_least_64` /
    `cap_u32` / `k_from_bits_per_key` / `nbytes_for_nbits` — no
    `Ord.max`/`min`/`clamp`/`div_ceil` (those failed Lean typecheck or
    extracted as axioms). Same integer form as before
    (`with_capacity_matches_legacy_formula`).
- Remaining axioms live on decode/error paths only: `U64.div_ceil`
  (`bloom_header_ok`), `Result.unwrap_or` (`encode` length), `fmt.format`
  / `String.from` (decode errors), `must_use`, `RangeInclusive.contains`.
- Lean 4.31.0 accepted (no `sorry` in `Bloom.lean`):
  - `may_contain_nbits_zero` / `may_contain_k_zero` / `always_true_never_rejects` (T4)
  - `set_bit_test_bit_same` (T1 core: extracted `set_bit` then `test_bit`
    on the same index, given `i/8` in bounds)
  - `probe_bit_lt` — extracted `probe_bit` is `< nbits` when `nbits ≠ 0`
  - `bit_index_of_le_u32max` — `bit ≤ u32::MAX` ⇒ `bit_index` is the cast
- Production `insert` / `may_contain` are Isolated-style `while i < k`
  over a shared `probe_bit` (range-for extracted to `IteratorRange` and
  blocked the loop proof).
- **Not claimed:** `insert_loop` then `may_contain_loop` composition
  (Vec deref_mut + k-step invariant). T2 encode/decode. Those stay
  Verus/Kani/tests.

## AE (`ae_kernel.rs`, RFC-0053 P2.1)

- Charon + Aeneas → `out/lean/AeKernel.lean` (`SOURCE.ae` sha256).
- Lean 4.31.0 accepted (no `sorry` in `Ae.lean`):
  - `ae_keep_if_same_term` (∀)
  - `ae_refuse_conflict_at_commit` / `ae_truncate_conflict_after_commit`
  - `as_is_rewrites_committed` (F16 teeth)

## Commit (`commit_kernel.rs`, RFC-0053 P2.1)

- Charon + Aeneas → `out/lean/CommitKernel.lean` (`SOURCE.commit` sha256).
- Lean 4.31.0 accepted (no `sorry` in `Commit.lean`):
  - `may_commit_at_iff` (∀)
  - `recover_commit_caps_examples` (F10 min)
  - `as_is_commits_prev_term` / `as_is_recover_promotes_suffix`

## Isolated (F83, was a Verus cartoon)

- `[lib] path` = production `isolated_kernel.rs`.
- `starts_with` extracted as an axiom; kernel rewritten to a byte loop (same predicate). Re-extract is **axiom-free**.
- Lean 4.31.0 accepted (no `sorry` in `Isolated.lean`):
  - `isolated_id_matches_loop_spec` / `_spec` — ∀ prefix + (`=` ∨ next `'/'`)
  - `isolated_id_matches_as_is_loop_spec` / `_spec` — ∀ prefix only
  - `as_is_leaks_sibling` — `/vm/vm-a` vs `/vm/vm-ab` (F83 teeth)
  - `isolated_id_matches_too_short`, child-byte atoms
- Method that closed: `loop.spec_decr_nat` + `step` on `index_usize_spec`;
  lengths only as `x.val.length : Nat`; no `simp [loop]`.


## Prefix (`prefix.rs`, RFC-0170 P0.3)

- `[lib] path` = production `crates/pedradb-core/src/prefix.rs`; stamp
  `SOURCE.prefix` pins the whole file.
- Production `prefix_exclusive_end` uses index `while e.len() > 0` (same
  semantics as `last_mut`; Charon translates the index form).
- Lean 4.31.0 accepted (no hole in `Prefix.lean`):
  - `prefix_exclusive_end_matches_spec` (empty vec → `done none`)
  - `prefix_exclusive_end_def` (`rfl` on to_vec then loop)
  - `prefix_exclusive_end_as_is_dente` (`rfl`: push 255)

## Write admission (`write_admission_kernel.rs`, RFC-0170 P2.1)

- `[lib] path` = production `crates/pedradb-core/src/write_admission_kernel.rs`;
  stamp `SOURCE.write_admission`.
- Charon translates both fns as transparent `if`/`ok` (0 axiom).
- Lean 4.31.0 accepted (no hole in `WriteAdmission.lean`):
  - `write_admission_idle_matches_spec` (all knobs off → `ok true`)
  - `write_admission_idle_mem_stall_refuses` (`ok false`)
  - `write_admission_idle_as_is_dente` (stalls ignored → `ok true`)
  - `write_admit_mem_over_stalls` (armed mem over → `StallMem`)
  - `write_admit_as_is_dente` (mem over still `Ok`)

## Close-kernel sweep (2026-09-06)

Every unique close-kernel path was either enrolled (`SOURCE.<stamp>` sha256 of
the production file + Lean theorems without `sorry`) or named below with a
measured Charon/Aeneas failure. Pins stay at Charon `0.1.232` / Aeneas
`daa85d7` / Lean 4.31.0. Production files were not rewritten to please Charon.
`db.rs` is not extracted (`glue.db_rs_extracted=false`).

### Enrolled (30)

`[lib] path` = production file. Stamp pins the whole file. Theorems live in
`formal/aeneas/lean/<Name>.lean` (not the generated `*Kernel.lean`).

| stamp | production | theorem (no `sorry`) |
|---|---|---|
| `lookup` | `lookup_kernel.rs` | `snap_is_empty_zero` / `_as_is_dente` |
| `rpc_mode` | `rpc_mode_kernel.rs` | `allow_direct_rpc_pin_refuses` / `_as_is_dente` |
| `store_compact` | store `compact_kernel.rs` | `may_compact_through_zero_false` |
| `snapshot` | `snapshot_kernel.rs` | `snapshot_touches_user_key_unreserved` |
| `si` | `si_kernel.rs` | `si_reader_beats_c_live` |
| `index_val` | `index_val_kernel.rs` | `value_len_tag_identity` / `_as_is_dente` |
| `changelog` | `changelog_kernel.rs` | `changelog_should_store_due` |
| `cursor` | `cursor_kernel.rs` | `next_seq_from_zero` |
| `cl` | `cl_kernel.rs` | `keep_body_without_cl_true` / `_as_is_dente` |
| `children` | `children_kernel.rs` | `packed_child_end_byte` (`0x01`) / `_as_is_dente` (`0xff`) |
| `pin` | `pin_kernel.rs` | `may_advance_pin_forward` |
| `pack` | `pack_kernel.rs` | `pack_cut_tag_identity` / `_as_is_dente` |
| `ship` | `ship_kernel.rs` | `stamp_changed_is_def` (slice extract; `have _ := @stamp_changed`) |
| `fold` | `fold_kernel.rs` | `fold_event_hides_key_is_def` (slice extract) |
| `manifest` | `manifest_kernel.rs` | `sst_recover_absent_scans` |
| `compact` | core `compact_kernel.rs` | `compact_pick_empty_noop` |
| `vlog_gc` | `vlog_gc_kernel.rs` | `vlog_recover_blob_opens` |
| `tx_glue` | `tx_glue_kernel.rs` | `tx_range_keep_committed` |
| `l28` | `l28.rs` | `l28_durability_all_ok` |
| `tcg` | `tcg.rs` | `tcg_guest_admitted_true` |
| `cqe` | `cqe_kernel.rs` | `cqe_res_ok_nonneg` |
| `iter` | `iter_kernel.rs` | `iter_window_keep_live` / `_as_is_dente` |
| `properties` | `properties_kernel.rs` | `d1_holds_loop_body_is_def` (loop extract) |
| `scale` | `scale_kernel.rs` | `point_get_probes_one_plus_one` / `_as_is_is_n_files` |
| `disk_pressure` | `disk_pressure_kernel.rs` | `disk_pressure_unknown_admits` / `_as_is_dente` |
| `crc` | `wal/crc.rs` | `crc_match_ok_equal` / `_as_is_dente` (`crc32c` crate fns stay axioms) |
| `env_crash` | `env_crash_kernel.rs` | `crash_legal_in_window` / `crash_legal_as_is_dente` (shim `#[path]` group_commit) |
| `wal_state` | `wal/wal_state_kernel.rs` | `inv_wal_well_formed` / `inv_wal_as_is_dente` (shim env_crash + group_commit) |
| `d1_modelo` | `d1_modelo_kernel.rs` | `d1_modelo_unacked_vacuous` / `d1_modelo_as_is_dente` |
| `write_ack` | `write_ack_kernel.rs` | `on_append_grows_written` / `write_ack_ledger_as_is_dente` |

Partial `.lean` from a failed Aeneas run is not enrolled.

### Refused — Aeneas/Charon or Lean typecheck of the generated Kernel

Measured on the pin. Aeneas still emitted a **partial** `.lean` for several of
these; that is not an extract.

| production | measured failure |
|---|---|
| `pedradb-raft/src/membership_kernel.rs` | `[Error] There should be no bottoms in the value` on `elect_claim_banner` / `_as_is` (raft lines 451–463). Partial file, 2 errors. |
| `pedradb-store/src/membership_kernel.rs` | Same bottoms on `elect_claim_banner` / `_as_is` (store lines 445–457). Partial file, 2 errors. |
| `pedradb-http/src/auth_kernel.rs` | `CFailure` `Unimplemented` translating a method sig; source `core/src/str/pattern.rs:99`. |
| `pedradb-http/src/fail_closed.rs` | Same `str/pattern.rs:99` `Unimplemented` on method sig. |
| `pedradb-http/src/form_kernel.rs` | Same `str/pattern.rs:99` `Unimplemented` on method sig. |
| `pedradb-http/src/path_kernel.rs` | Same `str/pattern.rs:99` `Unimplemented` on method sig. |
| `montanha-fdb-recipes/src/fields_kernel.rs` | `Nested borrows are not supported yet` in `encode_fields`; plus `Unimplemented`. Partial file, 3 unique errors. Iterator `map`/`collect`/`position` missing from the Lean model. |
| `pedradb-core/src/cf_kernel.rs` | Bottoms on `compact_family_key`, `cf_encode_effective`, `decode_cf_key`. Partial file, 3 errors. |
| `pedradb-core/src/leveling.rs` | `[Error] Can't end abstraction 11 as it is set as non-endable`; Iterator `map`/`filter`/`collect`/`all`/`max`/`min`/`sum` missing. Partial file, 5 errors (3 unique). |
| `pedradb-core/src/lsm_r1_kernel.rs` | `Returns inside of nested loops are not supported yet` (`lsm_compact` / `_as_is`); `Unreachable` in `lsm_reopen_as_is`; `Could not match the contexts` in `lsm_flush`. Partial file, 10 errors (5 unique). |
| `pedradb-dcs/src/lease_kernel.rs` | Aeneas emits Lean, but `lake build LeaseKernel` fails: `core.cmp.Ord.max.default core.cmp.OrdU64` type mismatch (`Ord U64` vs `U64 → U64 → Result Bool`) in `next_lease_id_after`. Pin's Ord.max. Not rewritten. |
| `pedradb-store/src/txn_kernel.rs` | Same `Ord.max.default` type mismatch (`TxnKernel.lean:143` and `:301`). |
| `pedradb-capi/src/handles.rs` | Aeneas emits Lean with `sorry`; `lake build CapiHandlesKernel` fails on `IterMut` / `FnOnce.call_once` / `Enumerate` (iterator surface, same class as probe-order). |
| `pedradb-core/src/probe_order_kernel.rs` | Live whole-file extract `CFailure` Internal error translating `core/src/iter/traits/iterator.rs:42`. Walk/closures stay Charon-refused. Not rewritten. |
| `pedradb-core/src/sst/scan_kernel.rs` | Shim linked `wal/crc.rs`; Aeneas `CFailure` Internal error translating `scan_reads_file` closure/`Iterator::any` (lines 112:13–112:85). Partial file. Not rewritten. |
| `pedradb-core/src/key.rs` | Shim linked `error.rs`+`thiserror`+`bytes`; Aeneas `Unsized cast between dynamic traits` on `core::error::Error::source` (`error.rs:7`). Partial file. Not rewritten. |

### Refused — include-crate does not compile standalone (8)

The extract crate is `[lib] path = production file` with no parent crate.
These files `use crate::…` or an external crate the probe did not link. Not a
Charon crash. Not rewritten. `scan_kernel` / `key.rs` moved to the Aeneas
table after a shim compiled and Aeneas still failed.

| production | measured rustc error |
|---|---|
| `pedradb-dcs/src/apply_kernel.rs` | `DcsError` / `Result` live in the parent crate |
| `pedradb-world/src/world_kernel.rs` | `use crate::TrajectorySample` (file is dirty-tree only; not in git HEAD) |
| `pedradb-posix/src/lib.rs` | Linked `libc`; Aeneas still `CFailure`: Dynamic trait types (`std::io::Error::new`), `&raw const`, improperly typed constant in `fdatasync_file`. Partial file, 8 errors. |
| `pedradb-core/src/merge.rs` | unresolved crate `pedradb_telemetry` |
| `pedradb-core/src/batch.rs` | `crate::key::{SequenceNumber, ValueType}` |
| `rocksdb-compat/src/locktab.rs` | Linked `parking_lot`+`bytes`; Aeneas `unsupported nested borrows` in `LockTable::lock`. Partial file. `wait_for_deadlock` not enrolled. |
| `pedradb-store/src/t1_modelo_kernel.rs` | `crate::txn_kernel` |
| `pedradb-raft/src/c1_modelo_kernel.rs` | `joint_election_ok` / `propose_ack_ok` from `membership_kernel` |

Probe-order iterator/closure refuse is unchanged (see above). Do not re-pin:
upstream `aeneas@f9a8e33` did not widen this set.

## What we may say

> Lean accepted those named theorems of the Aeneas extracts of the production Rust files. Persist/disk remain axioms. F83 sibling is now Lean-∀ (`as_is_leaks_sibling`), not only Stateright. Bloom T4 and the T1 bit-core (`set_bit_test_bit_same`) are Lean-∀ of the extract; insert-loop then query-loop is not.

Never: “Lean proved Raft / fold / the Bloom filter.”

## Re-check

```
./scripts/aeneas_vote.sh --required
./scripts/aeneas_ae.sh --required
./scripts/aeneas_commit.sh --required
./scripts/aeneas_isolated.sh --required
./scripts/aeneas_bloom.sh --required
./scripts/aeneas_prefix.sh --required
./scripts/aeneas_write_admission.sh --required
./scripts/lean_vote.sh --required
./scripts/lean_ae_commit.sh --required
./scripts/lean_bloom.sh --required
./scripts/lean_prefix.sh --required
./scripts/lean_write_admission.sh --required
./scripts/lean_extracts.sh --required
./scripts/aeneas_env_crash.sh --required
./scripts/aeneas_wal_state.sh --required
./scripts/aeneas_d1_modelo.sh --required
./scripts/aeneas_write_ack.sh --required
```

## Scale (`scale_kernel.rs`, RFC-0176)

- `[lib] path` = production `crates/pedradb-core/src/scale_kernel.rs`.
- Charon + Aeneas → `out/lean/ScaleKernel.lean` (`SOURCE.scale` sha256).
- `warm_cap_bytes` uses `if` not `Ord.max`/`min` (those failed Lean typecheck).
- Lean 4.31.0 accepted (no `sorry` in `Scale.lean`):
  - `point_get_probes_as_is_is_n_files` (∀)
  - `probes_worst_as_is_is_n_files` (∀)
- **Not claimed:** `saturating_add` 4+1=5 (native_decide failed — add stays
  in Verus). Clock \(T\) / noisy neighbor are measured, not extracted.
- Run: `./scripts/aeneas_scale.sh` then `lake build Scale` in `formal/aeneas/lean`.

