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

- Charon + Aeneas → `out/lean/CommitKernel.lean` (`SOURCE.commit` sha256). Pair `commit_raft` is single_artifact: production file is the Verus term.
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

### Enrolled (52)

`[lib] path` = production file. Stamp pins the whole file. Theorems live in
`formal/aeneas/lean/<Name>.lean` (not the generated `*Kernel.lean`).

| stamp | production | theorem (no `sorry`) |
|---|---|---|
| `lookup` | `lookup_kernel.rs` | `snap_is_empty_zero` / `_as_is_dente` |
| `rpc_mode` | `rpc_mode_kernel.rs` | `allow_direct_rpc_pin_refuses` / `_as_is_dente` |
| `store_compact` | store `compact_kernel.rs` | `may_compact_through_zero_false` (pair `compact_unleft` is single_artifact: production file is the Verus term) |
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
| `vlog_gc` | `vlog_gc_kernel.rs` | `vlog_recover_blob_opens` (pair `vlog_recover` is single_artifact: production file is the Verus term) |
| `tx_glue` | `tx_glue_kernel.rs` | `tx_range_keep_committed` (single_artifact: production file is the Verus term) |
| `l28` | `l28.rs` | `l28_durability_all_ok` |
| `tcg` | `tcg.rs` | `tcg_guest_admitted_true` |
| `cqe` | `cqe_kernel.rs` | `cqe_res_ok_nonneg` / `submit_complete_act_harvested` / `_as_is_dente` (`RUSTFLAGS=--cfg test`; Atomic telemetry in `submit_complete_act` stripped) |
| `iter` | `iter_kernel.rs` | `iter_window_keep_live` / `_as_is_dente` (single_artifact: production file is the Verus term) |
| `properties` | `properties_kernel.rs` | `d1_holds_loop_body_is_def` (loop extract) |
| `scale` | `scale_kernel.rs` | `point_get_probes_one_plus_one` / `_as_is_is_n_files` |
| `disk_pressure` | `disk_pressure_kernel.rs` | `disk_pressure_unknown_admits` / `_as_is_dente` |
| `crc` | `wal/crc.rs` | `crc_match_ok_equal` / `_as_is_dente` (`crc32c` crate fns stay axioms) |
| `env_crash` | `env_crash_kernel.rs` | `crash_legal_in_window` / `crash_legal_as_is_dente` (shim `#[path]` group_commit) |
| `wal_state` | `wal/wal_state_kernel.rs` | `inv_wal_well_formed` / `inv_wal_as_is_dente` (shim env_crash + group_commit) |
| `d1_modelo` | `d1_modelo_kernel.rs` | `d1_modelo_unacked_vacuous` / `d1_modelo_as_is_dente` |
| `write_ack` | `write_ack_kernel.rs` | `on_append_grows_written` / `write_ack_ledger_as_is_dente` |
| `dcs_apply` | dcs `apply_kernel.rs` | `dcs_apply_should_advance_cas` / `_as_is_dente` (shim names `DcsError` without thiserror; decision fns are production `#[path]`) |
| `store_apply` | store `apply_kernel.rs` | `apply_advance_hole_stops` / `_as_is_dente` |
| `store_commit` | store `commit_kernel.rs` | `may_commit_at_current_majority` / `_as_is_dente` |
| `store_ae_ack` | store `ae_ack_kernel.rs` | `ae_ack_success_dirty_without_persist` / `_as_is_dente` |
| `store_vote` | store `vote_kernel.rs` | `vote_decision_stale_term` / `_as_is_dente` |
| `key` | `key.rs` | `pack_sequence_and_type_def` / `_as_is_dente` (shim names `CoreError::Internal` without thiserror; InternalKey Eq `impl_def` patched like Vote Option::eq) |
| `lease` | dcs `lease_kernel.rs` | `lease_live_zero` / `_as_is_dente` (`Ord.max.default` patched to pass `lt`; pair `lease` is single_artifact: production file is the Verus term) |
| `reopen` | wal `reopen_kernel.rs` | `clean_reopen_serves_all` / `as_is_swallows_damage` (pair `reopen_outcome` is single_artifact: production file is the Verus term) |
| `txn` | store `txn_kernel.rs` | `txn_commit_action_abort_reverts` / `_as_is_dente` (same Ord.max patch) |
| `t1_modelo` | `t1_modelo_kernel.rs` | `t1_modelo_empty` / `_as_is_dente` (shim `#[path]` txn_kernel) |
| `membership` | raft `membership_kernel.rs` | `joint_election_ok_needs_both` / `elect_claim_banner_bounded` (`&'static str` bottoms patched to `toStr`; Ord.max patch) |
| `store_membership` | store `membership_kernel.rs` | same theorems (clone) |
| `c1_modelo` | `c1_modelo_kernel.rs` | `c1_modelo_joint_add_refuses` / `_as_is_dente` |
| `capi_handles` | capi `handles.rs` | `c_len_admitted_oversize` / `_as_is_dente` / `c_path_walk_bytes_4k` / `c_free_table_admitted_false` (Charon `--start-from` len + path-walk + free-table; rest is IterMut) |
| `batch` | `batch.rs` | `write_record_count_ok_prefix` / `_as_is_dente` (shim `#[path]` key.rs; `--start-from write_record_count_ok`; decode is early-return-in-loop) |
| `merge` | `merge.rs` | `visible_at_deletion` / `_as_is_dente` / `user_key_in_range_unbounded` / `past_end_unbounded` / `iter_window_keep_hidden` (shim `#[path]` key+compact; `--start-from` visible_at + range + window bounds + keep; WindowKvIter is Iterator) |
| `fail_closed` | `fail_closed.rs` | `parse_error_writes_status_true` / `reject_transfer_encoding_true` / `present_bad_int_is_error_true` / `parse_error_status_400` / `header_break_len_below_four` (`--start-from` F102 + F104/F105/F157/F158 + header_break + Expect; Windows `position` and Split clauseInst/`all`/`any` patched) |
| `probe_order` | `probe_order_kernel.rs` | `first_probe_on_equal_lo_newer` / `covering_hi_ge_oob` / `probe_order_covering_is_loop` (index walk; covering loop body patched) |
| `locktab` | `locktab.rs` | `wait_for_deadlock_is_loop` / `_as_is_dente` (`--start-from wait_for_deadlock`; `--exclude LockTable` nested borrows; HashMap/HashSet stay axioms; pair `wait_for_deadlock` is single_artifact: production file is the Verus term) |
| `scan` | `sst/scan_kernel.rs` | `sst_crc_fate_modern_mismatch` / `scan_reads_file_none_smallest` / `zero_glue_admitted_false` / `tombstone_reaches_window_as_is_dente` (shim `#[path]` crc; `--start-from` catalog entries including model as_is; closure `call_mut` patched to `tombstone_reaches_window`) |
| `cf` | `cf_kernel.rs` | `key_in_cf_family_as_is_dente` / `cf_encode_effective_is_if` / `infer_sst_cf_none_none` (`--start-from` catalog entries; `cf_encode_effective`/`decode_cf_key` patched over lifetime bottoms; pair `cf_family` is single_artifact: production file is the Verus term) |
| `fields` | `fields_kernel.rs` | `field_kept_id` / `_as_is_dente` (`--start-from` catalog entries; `encode_fields` nested-borrows hole patched to an index loop) |
| `lsm_r1` | `lsm_r1_kernel.rs` | `lsm_reopen_id` / `lsm_compact_depth_zero` / `lsm_state_of_is_def` (`--start-from` catalog + `lsm_state_of`/`lsm_write`; compact nested-loop returns patched to `level_put`/`level_remove`; reopen_as_is reverse-stack; `level_distinct`/`inv_lsm`/`lsm_flush` stay nested-loop refuse) |
| `leveling` | `leveling.rs` | `level_target_bytes_l0` / `_as_is_l0` (`RUSTFLAGS=--cfg test` so Charon sees as_is; pick Iterator holes patched to index loops; Iterator extra fields stripped) |
| `posix` | `pedradb-posix/src/lib.rs` | `fdatasync_rc_ok_zero` / `_nonzero` / `_as_is_dente` / `fdatasync_eintr_retry_admitted_false` (`--start-from` rc_ok + EINTR retry; rest of lib.rs is syscall/unsafe). SOURCE sha256 is git HEAD (concurrent clippy on `filesystem_available_bytes` is not in the stamp). |
| `form` | `form_kernel.rs` | `form_plus_byte_plus` / `query_u64_conflict_diff` (`--exclude str::contains` / `pattern`; contains Pattern hole and `query_values_conflict` Iterator.any patched to an index loop) |
| `auth` | `auth_kernel.rs` | `ascii_lower_is_extract` / `is_bearer_scheme_as_is_is_or` (`--exclude` Pattern methods; `bearer_token_from_value` axiom and `authorization_matches` hole patched) |
| `path` | `path_kernel.rs` | `strip_authority_for_routing_true` / `_as_is_dente` (`--exclude` Pattern methods; catalog-fn holes patched to `find`/`split_once`/`rsplit_once` axioms + index loops) |
| `world` | `world_kernel.rs` | `trajectory_violation_applied` / `_as_is_dente` (`[lib] path` production kernel split from `lib.rs`; `Option<&'static str>` and HashMap fold patched) |

Model-twin `entry`s on already-enrolled files now have Lean `def`s (still `proof_depth=model`): `range_tombstone_covers`, `run_pairwise_disjoint_los`, `key_in_window`, `leveled_enabled`, `total_bytes`, `tombstone_reaches_window` / `_as_is`.

Partial `.lean` from a failed Aeneas run is not enrolled. Charon `--start-from` of the catalog entries is an extract of the live file when Aeneas emits a complete Kernel (no `sorry`) containing a `def` for every catalog `entry` on that path.

### Catalog `entry`s with no Lean `def` on an enrolled path

Not silent close-kernel **files** (`l28.rs` and `probe_order_kernel.rs` stay enrolled for the `fn`s that exist). Named here so the missing catalog `entry`s are not claimed as extracts.

**No production `fn`.** Close pairs `l28_tcp_add` / `l28_tcp_cnew` / `l28_tcp_svget` / `l28_tcp_newget` / `l28_tcp_jleft` / `l28_tcp_caught` / `l28_tcp_grown` name `l28_tcp_*_ok` on enrolled `crates/pedradb-store/src/l28.rs`. Those `fn`s are not in the live file (TCP kernels end at `l28_tcp_pj_ok`). `git log -S l28_tcp_add_ok -- crates/pedradb-store/src/l28.rs` is empty; the names landed in `36d4f685` on catalog / `verified.rs` / `docs/status.md` only. Verus twin, DST plant `l28_real_tcp_add_member_joint_cnew`, `cluster_real --add-member`, and handlers `tcp_node_disk_added_joint` / `tcp_node_disk_caught_up` are also absent. RFC-0119 itself has no P2.3. Not a Charon refuse: there is nothing to extract. Do not invent identity gates to please the catalog.

**Iterator CFailure is not a refuse of the covering decision.** Production `probe_order_covering` is now an index `while` returning `Vec` (same keep-rule as the old `filter`+`position` walk; Isolated method). Aeneas still holes the nested `Vec.push` loop (`Could not match the contexts`); `aeneas_probe_order.sh` patches that body to `probe_order_covering_loop` so the catalog entry is a Lean `def`. Theorems: `covering_hi_ge_oob`, `probe_order_covering_is_loop` / `_as_is_is_loop`. `probe_order_covering_as_is` is the oldest-first reverse-index walk (production `fn`, not invented). Unpacked `probe_order` (`filter.collect`) is still the Iterator form — covering is the engine-facing packed image.

**ConcurrentDb is on the proof path.** `concurrent.rs` is glue (`RwLock` / `Env`); the write-group decisions it calls are `group_commit_kernel` (OCC `occ_conflict` / `group_validate`, publish `may_publish_group`, lock-schedule residual `lock_interleavings_admitted`, PCT `forall_schedules_admitted`). Lean: `lock_interleavings_not_a_theorem` (`ok false` — that is the theorem, not “out of scope”), `may_publish_group_needs_wal_ok`, `forall_schedules_pct2_not_admitted`, plus the existing `occ_conflict` closed form / group simultaneity. `ConcurrentDb::claim_lock_interleavings_proven` unfolds `lock_interleavings_admitted`. Glue around the lock stays TCB until more of the group protocol is a named kernel.

**No production `fn` (catalog `as_is` / leftover names).** Close-pair `as_is` columns name mutants that were never added to the enrolled file. Measured: `rg 'fn <name>'` on the kernel is empty. Do not invent them. Named here so they are not silent leftovers. `probe_order_covering` is now a Lean `def` (index walk). Unpacked `probe_order` / `probe_order_as_is` still use `filter.collect`.

| catalog id | named `as_is` / leftover | enrolled file |
|---|---|---|
| `unreserve_si_gen` | `unreserve_si_gen_as_is` | `txn_kernel.rs` |
| `stream_next_seq` | `next_seq_as_is` | `cursor_kernel.rs` |
| `path_after_authority` | `path_after_authority_as_is` | `path_kernel.rs` |
| `strip_http_authority` | `strip_http_authority_as_is` | `path_kernel.rs` |
| `request_target_authority` | `request_target_authority_as_is` | `path_kernel.rs` |
| `split_host_port` | `split_host_port_as_is` | `path_kernel.rs` |
| `from_hex` | `from_hex_as_is` | `form_kernel.rs` |
| `plus_before_percent` | `plus_before_percent_as_is` | `form_kernel.rs` |
| `ascii_lower` | `ascii_lower_as_is` | `auth_kernel.rs` |
| `ascii_upper` | `ascii_upper_as_is` | `auth_kernel.rs` |
| `is_non_bearer_auth_scheme` | `is_non_bearer_auth_scheme_as_is` | `auth_kernel.rs` |
| `authorization_matches` | `authorization_matches_as_is` | `auth_kernel.rs` |
| `journal_catch_up_pin` | `catch_up_pins_on_read_as_is` | `pin_kernel.rs` |
| `journal_fold_pin` | `fold_pins_on_read_as_is` | `pin_kernel.rs` |
| `journal_next_pin` | `next_pin_as_is` | `pin_kernel.rs` |
| `children_start` | `packed_children_start_as_is` | `children_kernel.rs` |
| `children_half_open` | `key_in_half_open_as_is` | `children_kernel.rs` |
| `fields_decode` | `decode_fields_as_is` | `fields_kernel.rs` |
| `cqe_leftover` | `cqe_act_as_is` | `cqe_kernel.rs` |
| `cf_family_of` | `cf_family_of_as_is` | `cf_kernel.rs` |
| `cf_encode_effective` | `cf_encode_effective_as_is` | `cf_kernel.rs` |
| `encode_cf_key` | `encode_cf_key_as_is` | `cf_kernel.rs` |
| `decode_cf_key` | `decode_cf_key_as_is` | `cf_kernel.rs` |
| `infer_sst_cf` | `infer_sst_cf_as_is` | `cf_kernel.rs` |
| `l28_tcp_*` (7 pairs) | `l28_tcp_*_ok` / `l28_tcp_*_ok_as_is` | `l28.rs` (TCP kernels end at `l28_tcp_pj_ok`) |
| `leveled_enabled` | `leveled_enabled_as_is` | `leveling.rs` |
| `leveling_disjoint` | `is_disjoint_as_is` | `leveling.rs` |
| `leveling_overlaps` | `overlaps_as_is` | `leveling.rs` |
| `leveling_total_bytes` | `total_bytes_as_is` | `leveling.rs` |
| `key_in_window` | `key_in_window_as_is` | `sst/scan_kernel.rs` |
| `point_bounds_overlap` | `point_bounds_overlap_as_is` | `sst/scan_kernel.rs` |
| `from_record_type` | `from_record_type_as_is` | `wal/recover_kernel.rs` |
| `compact_split` | `compact_should_split_as_is` | core `compact_kernel.rs` |
| `compact_split_at` | `compact_should_split_at_as_is` | core `compact_kernel.rs` |
| `pct_default_depth` | `pct_campaign_default_depth_as_is` | `group_commit_kernel.rs` |
| `compact_floor` | `compact_index_floor_as_is` | store `compact_kernel.rs` |
| `compact_ready` | `compact_ready_as_is` | store `compact_kernel.rs` |

**Live `fn` on an enrolled file without a Lean `def` (not catalog `entry`s).** Named so they are not silent:

| live name | measured |
|---|---|
| `probe_order_as_is` | unpacked historical walk (`sort` + `filter.collect`). Packed covering as-is is `probe_order_covering_as_is` (extracted). |
| `overlap_distinct_los_still_inverts_as_is` | `#[cfg(test)]` helper, not a decision `fn`. |
| `flush_plan_as_is` | Verus `spec fn` in the same file; the exec mutant is `flush_plan_as_is_lose_tail` (already a Lean `def`). |

### Refused — Aeneas/Charon or Lean typecheck of the generated Kernel

Measured on the pin. Aeneas still emitted a **partial** `.lean` for several of
these; that is not an extract.

| production | measured failure |
|---|---|

None remaining on this pin. `path` / `auth` / `form` enrolled via `--exclude` of `str/pattern` plus generated-Lean patches.

### Refused — include-crate does not compile standalone, or `--start-from` still lake-red

The extract crate is `[lib] path = production file` with no parent crate.
These files `use crate::…` or an external crate the probe did not link. Not a
Charon crash. Not rewritten. `scan_kernel` / `key.rs` moved to the Aeneas
table after a shim compiled and Aeneas still failed. dcs `apply_kernel.rs`
enrolled via a shim that names `DcsError` without thiserror.

| production | measured rustc error |
|---|---|

None remaining: `world_kernel.rs` is production (`[lib] path`) and enrolled.

The unpacked `probe_order` walk is still Iterator (`filter.collect`); the packed covering image is extracted. Do not re-pin unless it widens the set without `sorry`.

## Composed edges (not only per-kernel atoms)

Pedra theorems that `unfold` **both** caller and callee on a representative
input, plus a second possibility (as-is or other branch). Same-Kernel rows
are shim `#[path]` callees already inside one generated file. Cross-lib rows
import two `*Kernel` lake libs. `scripts/lean_extracts.sh --required` builds
the composition files and fails on a missing file or the substring `sorry`.

| caller | callee | production theorem | second possibility |
|---|---|---|---|
| EnvCrash `sync` | GroupCommit `fsync_promotes_pending` (shim copy) | `sync_honest_promotes_via_fsync` | `sync_lying_does_not_promote` |
| WalState `acked_survives_every_legal_crash` | EnvCrash `crash_legal` | `acked_survives_legal_cut` | `acked_survives_as_is_dente` |
| WalState `wal_sync` | EnvCrash `sync` | `wal_sync_honest_promotes` | `wal_sync_lying_does_not_promote` |
| D1Modelo `put_ok` | WalState `wal_append`/`wal_sync`/`wal_ack` | `put_ok_append_sync_ack` | `put_ok_as_is_acks_unsynced` |
| WriteAck `on_barrier` | WalState `wal_sync` | `on_barrier_honest_promotes` | `on_barrier_empty_is_id` (same caller; production hardcodes Honest). As-is that skips the barrier: `write_ack_ledger_as_is_dente` |
| WriteAck `on_ack` | WalState `wal_ack` | `on_ack_promotes_acked` | `on_ack_already_caught_up` |
| WriteAck `assert_inv` | WalState `inv_wal` | `assert_inv_well_formed` | `assert_inv_ill_formed_fails` |
| WriteAck `d1_holds_every_cut.closure.call_mut` | D1 `d1_modelo` | `d1_holds_cut_in_window` | `d1_holds_as_is_cut_below_barrier` |
| Posix `fdatasync_rc_ok` | Posix `fdatasync_eintr_retry_admitted` | `posix_ok_needs_zero_rc_and_no_eintr_retry` | `posix_as_is_admits_nonzero_and_eintr` / `posix_nonzero_rc_not_ok` |
| Iter `iter_window_keep` | Merge `iter_window_keep` (clone) | `iter_merge_keep_live_agree` / `_hidden_agree` | `iter_merge_keep_as_is_agree` |
| Membership `joint_election_ok` | StoreMembership clone | `joint_election_ok_clones_refuse` / `_single_cfg` | `joint_election_ok_as_is_clones_agree` |
| Scan `sst_crc_fate` | Crc extract `crc_match_ok` | `sst_crc_fate_mismatch_via_crc_extract` / `_equal_via_crc_extract` | `sst_crc_fate_as_is_via_crc_as_is` |
| C1Modelo `c1_modelo` | Membership extract `joint_election_ok` | `c1_modelo_joint_add_via_membership_extract` / `_single_cfg_via_membership_extract` | `c1_modelo_as_is_via_membership_as_is` |
| T1Modelo `t1_modelo` | Txn `leftover_txn_is_aborted` (shim copy) | `t1_modelo_empty` (already unfolds both) | `leftover_txn_is_aborted_as_is_dente` in `Txn.lean` |
| ConcurrentDb `validate_occ_batch` | `group_validate` / `occ_conflict` | `group_validate_lagging_member_conflicts` | `group_occ_vs_serialized_same_input` |
| ConcurrentDb off-lock fd | `may_publish_group` ∧ Flush `wal_rotate_decision` | `concurrent_publish_and_inflight_keep_wal` | `concurrent_publish_ok_and_idle_rotates` / `concurrent_as_is_publish_lie_inflight_still_keeps` |
| ConcurrentDb lock-order | Flush `wal_rotate_decision` (`commit_inflight`) | `wal_rotate_commit_inflight_keeps` | `wal_rotate_idle_rotates` |
| TransactionDB 2PL | `wait_for_deadlock` | `wait_for_deadlock_is_loop` | `wait_for_deadlock_as_is_dente` / `wait_for_deadlock_loop_vs_as_is` |
| ConcurrentDb scheduler residual | `lock_interleavings_admitted` | `lock_interleavings_not_a_theorem` | `lock_interleavings_as_is_dente` |

Cross-lib files: `ComposeIterMerge.lean`, `ComposeMembershipClone.lean`,
`ComposeScanCrc.lean`, `ComposeC1Membership.lean`, `ComposeConcurrent.lean`.

Measured cross-lib refuses (do not invent a merge of the two Kernels):

- `import T1ModeloKernel` + `import TxnKernel` — lake red `environment already contains 'instDiscriminantRevertUserActionIsize'`. Both extracts stamp the same discriminant instance. T1 already `#[path]`s txn; leftover is unfolded in `t1_modelo_empty` and restated on the txn extract.
- `import GroupCommitKernel` + EnvCrash `fsync_promotes_pending` — `formal/aeneas/lean/GroupCommitKernel.lean` is the Aug-24 copy (no `fsync_promotes_pending`); `out/lean/GroupCommitKernel.lean` has it. Do not restamp the GroupCommit lake lib in this slice (loop theorems + LawfulBEq). The EnvCrash shim copy is unfolded in `sync_honest_promotes_via_fsync`.

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
./scripts/aeneas_dcs_apply.sh --required
./scripts/aeneas_store_apply.sh --required
./scripts/aeneas_store_commit.sh --required
./scripts/aeneas_store_ae_ack.sh --required
./scripts/aeneas_store_vote.sh --required
./scripts/aeneas_key.sh --required
./scripts/aeneas_lease.sh --required
./scripts/aeneas_txn.sh --required
./scripts/aeneas_t1_modelo.sh --required
./scripts/aeneas_membership.sh --required
./scripts/aeneas_store_membership.sh --required
./scripts/aeneas_c1_modelo.sh --required
./scripts/aeneas_capi_handles.sh --required
./scripts/aeneas_batch.sh --required
./scripts/aeneas_merge.sh --required
./scripts/aeneas_fail_closed.sh --required
./scripts/aeneas_probe_order.sh --required
./scripts/aeneas_locktab.sh --required
./scripts/aeneas_scan.sh --required
./scripts/aeneas_cf.sh --required
./scripts/aeneas_fields.sh --required
./scripts/aeneas_lsm_r1.sh --required
./scripts/aeneas_leveling.sh --required
./scripts/aeneas_posix.sh --required
./scripts/aeneas_form.sh --required
./scripts/aeneas_auth.sh --required
./scripts/aeneas_path.sh --required
./scripts/aeneas_world.sh --required
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

