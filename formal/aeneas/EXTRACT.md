# Aeneas extract + Lean theorems

## ComposeWriter (RFC-0222 P2.1 / RFC-0220 P0.2, 2026-09-14)

- `formal/aeneas/lean/ComposeWriter.lean`: dual-unfold of
  `flusher_gate_plan_fate_iff` × `parked_debt_plan_fate_iff`.
- `writer_workerless_gate_and_debt_iff` / `workerless_never_drains` /
  `flusher_gate_plan_as_is_always_drains`. `lake build ComposeWriter` green,
  0 sorry. m2 33→35 (`flusher_gate_plan`, `parked_debt_plan` now chained).

## ComposeRecovery (RFC-0222 P2.2, 2026-09-14)

- `formal/aeneas/lean/ComposeRecovery.lean`: dual-unfold of
  `sst_recover_action_refuse_iff_corrupt_or_inventory_missing` ×
  `reopen_outcome_serve_all_iff_damage_none` ×
  `vlog_recover_action_refuse_open_iff_wants_large_use_new_and_nothing_on_disk`
  plus `recover_collect_act` KeepRecord. `lake build ComposeRecovery` green.
  recovery_atoms_chained 0→4 / 11.

**Date:** 2026-08-15

## ClientAxis (`client_axis_kernel.rs`, RFC-0222 P0.7 2026-09-14)

- `[lib] path` = production `crates/pedradb-core/src/client_axis_kernel.rs`.
- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/ClientAxisKernel.lean`.
- Pairs `pipeline_drain_cap` and `async_merge_policy` are single_artifact:
  production `concurrent.rs` (`WriteGroup::lead` / submit) calls the rustc
  body; Lean unfolds that body (`pipeline_drain_cap_fate_iff`,
  `async_merge_policy_fate_iff`).
- Generated `Ord.max.default core.cmp.OrdUsize` patched to
  `.partialOrdInst.lt` (same class as txn/c1_modelo) — `drain_convoy_count`.
- Lean 4.31.0 accepted (no `sorry` in `ClientAxis.lean`).

## GroupWindow (`group_window_kernel.rs`, RFC-0222 P0.7 2026-09-14)

- `[lib] path` = production `crates/pedradb-core/src/group_window_kernel.rs`.
- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/GroupWindowKernel.lean`.
- Pairs `merge_eligible` and `flight_capped_window_us` are single_artifact:
  production `concurrent.rs` calls the rustc body; Lean unfolds that body
  (`merge_eligible_fate_iff`, `flight_capped_window_us_fate_iff`).
- Generated `Ord.max.default` U64 patched to `.partialOrdInst.lt` (`peer_horizon_us`).
- `group_window_us` / `group_window_cap_to_flight` extract via `Str.parse`
  axioms (env parse — not the ∀ pairs).
- Lean 4.31.0 accepted (no `sorry` in `GroupWindow.lean`).

## LeftoverPage / ScanReadahead / WriteCycle / DurabilitySpine / RatioCurve / ProductCrown (RFC-0222 P0.7 2026-09-14)

- `leftover_page_kernel.rs` / `scan_readahead_kernel.rs`: `[lib] path` = production; now `pub mod` in `lib.rs` (rustc links). `family_upper_bound` Isolated `while` + `wrapping_add` (raw `u8+1` unimplemented). Theorems `leftover_page_advice_fate_iff`, `scan_readahead_window_hot_never`.
- `write_cycle_kernel.rs`: shim + `write_admission_kernel`. `serial_cs_ns_fate_iff`. Display/`as_str` extract as sorry (unused by the atom).
- `durability_spine_kernel.rs`: shim of write_ack deps. `spine_replay_fate_iff`.
- `ratio_curve_kernel.rs`: whole-file Aeneas CFailure on `&'static str` (GetSideAnchor); `--start-from cold_permille`. `cold_permille_fate_iff`.
- `product_crown_kernel.rs`: shim + `#[cfg(pedra_aeneas)]` import of path-included `properties_kernel`; Isolated `while` (not `Iterator::all`/`map`). `product_crown_fate_iff`.
- Lean 4.31.0 accepted (no `sorry` in the six wrappers).

## Vote (`vote_kernel.rs`)

- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/VoteKernel.lean`. Pairs `durable_term`, `grant_persist`, and `vote` are single_artifact: production file is the Verus term.
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

- Charon + Aeneas → `out/lean/AeKernel.lean` (`SOURCE.ae` sha256). Pairs `ae_entry` and `ae_ack` are single_artifact: production file is the Verus term.
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
  - `wal_commit_plan_need_sync_ok` (Sync before Apply/Ok)
  - `wal_commit_plan_fence_via_fence_on_sync_fail` (plan + `fence_on_sync_fail`)
  - `wal_commit_plan_as_is_dente` (Apply/Ok after failed sync)

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
| `fold` | `fold_kernel.rs` | `fold_event_hides_key_fate_iff` (RFC-0218 P2.1 promotion) |
| `manifest` | `manifest_kernel.rs` | `sst_recover_absent_scans` (pair `manifest_recover` is single_artifact: production file is the Verus term) |
| `compact` | core `compact_kernel.rs` | `compact_pick_empty_noop` (pairs `compact_decision`, `compact_retention`, `pin_gc` are single_artifact: production file is the Verus term) |
| `vlog_gc` | `vlog_gc_kernel.rs` | `vlog_recover_blob_opens` (pair `vlog_recover` is single_artifact: production file is the Verus term) |
| `tx_glue` | `tx_glue_kernel.rs` | `tx_range_keep_committed` (single_artifact: production file is the Verus term) |
| `l28` | `l28.rs` | `l28_durability_all_ok` |
| `tcg` | `tcg.rs` | `tcg_guest_admitted_true` |
| `cqe` | `cqe_kernel.rs` | `cqe_res_ok_nonneg` / `submit_complete_act_harvested` / `_as_is_dente` (`RUSTFLAGS=--cfg test`; Atomic telemetry in `submit_complete_act` stripped) |
| `iter` | `iter_kernel.rs` | `iter_window_keep_live` / `_as_is_dente` (single_artifact: production file is the Verus term) |
| `properties` | `properties_kernel.rs` | `d1_holds_loop_body_is_def` (loop extract) |
| `scale` | `scale_kernel.rs` | `point_get_probes_one_plus_one` / `_as_is_is_n_files` / `probes_worst_l0_trigger_via_point_get` / `worst_get_ns_l0_trigger_via_probes_worst` / `happy_get_ns_l0_best_via_point_get` / `best_get_ns_l0_best_via_point_get` |
| `disk_pressure` | `disk_pressure_kernel.rs` | `disk_pressure_unknown_admits` / `_as_is_dente` / `disk_probe_err_admits` / `compact_refuse_unfolds_disk_pressure_admit` |
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
| `txn` | store `txn_kernel.rs` | `txn_commit_action_abort_reverts` / `_as_is_dente` (same Ord.max patch; pair `txn` is single_artifact: production file is the Verus term) |
| `t1_modelo` | `t1_modelo_kernel.rs` | `t1_modelo_empty` / `_as_is_dente` (shim `#[path]` txn_kernel) |
| `membership` | raft `membership_kernel.rs` | `joint_election_ok_needs_both` / `elect_claim_banner_bounded` (`&'static str` bottoms patched to `toStr`; Ord.max patch) |
| `store_membership` | store `membership_kernel.rs` | same theorems (clone) |
| `c1_modelo` | `c1_modelo_kernel.rs` | `c1_modelo_joint_add_refuses` / `_as_is_dente` |
| `capi_handles` | capi `handles.rs` | `c_len_admitted_oversize` / `_as_is_dente` / `c_path_walk_bytes_4k` / `c_free_table_admitted_false` (Charon `--start-from` len + path-walk + free-table; rest is IterMut) |
| `batch` | `batch.rs` | `write_record_count_ok_prefix` / `_as_is_dente` (shim `#[path]` key.rs; `--start-from write_record_count_ok`; decode is early-return-in-loop; pair `write_record_count` is single_artifact: production file is the Verus term) |
| `merge` | `merge.rs` | `visible_at_deletion` / `_as_is_dente` / `user_key_in_range_unbounded` / `past_end_unbounded` / `iter_window_keep_hidden` / `write_op_covers_f30` / `write_op_range_end_some` / `merge_sift_step_repairs_iff` (shim `#[path]` key+compact; `--start-from` visible_at + range + window bounds + keep + covers + range-end + sift_step; WindowKvIter/StreamingVisibleIter stay Iterator-refused — keeps route `iter_window_keep`/`visible_at` (per-item measured negative + repro + upstream aeneas#1343 in the RFC-0188 P2.3 refused table); `&[]` arm is an Aeneas bottom refuse → `Option<&[u8]>`; write_op_covers_key do-matches patched; pairs `visible_at`, `range_covers`, `write_op_range_end`, `merge_sift` are single_artifact: production file is the Verus term; `merge_sift` is RFC-0188 P0.2 first `close` — Isolated-method STRUCTURE kernel, production keeps ORDER) |
| `fail_closed` | `fail_closed.rs` | `parse_error_writes_status_true` / `reject_transfer_encoding_true` / `present_bad_int_is_error_true` / `parse_error_status_400` / `header_break_len_below_four` (`--start-from` F102 + F104/F105/F157/F158 + header_break + Expect; Windows `position` and Split clauseInst/`all`/`any` patched) |
| `probe_order` | `probe_order_kernel.rs` | `first_probe_on_equal_lo_newer` / `covering_hi_ge_oob` / `probe_order_covering_is_loop` (index walk; covering loop body patched) |
| `locktab` | `locktab.rs` | `wait_for_deadlock_is_loop` / `_as_is_dente` (`--start-from wait_for_deadlock`; `--exclude LockTable` nested borrows; HashMap/HashSet stay axioms; pair `wait_for_deadlock` is single_artifact: production file is the Verus term) |
| `scan` | `sst/scan_kernel.rs` | `sst_crc_fate_modern_mismatch` / `scan_reads_file_none_smallest` / `zero_glue_admitted_false` / `tombstone_reaches_window_as_is_dente` (shim `#[path]` crc; `--start-from` catalog entries including model as_is; closure `call_mut` patched to `tombstone_reaches_window`) |
| `cf` | `cf_kernel.rs` | `key_in_cf_family_as_is_dente` / `cf_encode_effective_is_if` / `infer_sst_cf_none_none` (`--start-from` catalog entries; `cf_encode_effective`/`decode_cf_key` patched over lifetime bottoms; pair `cf_family` is single_artifact: production file is the Verus term) |
| `fields` | `fields_kernel.rs` | `field_kept_id` / `_as_is_dente` (`--start-from` catalog entries; `encode_fields` nested-borrows hole patched to an index loop) |
| `lsm_r1` | `lsm_r1_kernel.rs` | `lsm_reopen_id` / `lsm_compact_depth_zero` / `lsm_state_of_is_def` (`--start-from` catalog + `lsm_state_of`/`lsm_write`; compact nested-loop returns patched to `level_put`/`level_remove`; reopen_as_is reverse-stack; `level_distinct`/`inv_lsm`/`lsm_flush` stay nested-loop refuse (each with its re-measured negative in the RFC-0188 P2.3 refused table)) |
| `leveling` | `leveling.rs` | `level_target_bytes_l0` / `_as_is_l0` (`RUSTFLAGS=--cfg test` so Charon sees as_is; pick Iterator holes patched to index loops; Iterator extra fields stripped; pairs `leveling`, `leveling_pick`, and `leveling_pushdown` are single_artifact: production file is the Verus term) |
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

**2026-09-11 (RFC-0210 P1.2): verdict — retired.** Fresh measurement at HEAD: `grep 'add_member\|AddMemberJoint' crates/pedradb-store/src/bin/cluster_real.rs` is empty (the real-TCP binary dispatches no add-member path — the wire tag 20 helper in `tcp.rs` is unused there); `tcp_node_disk_added_joint` / `tcp_node_disk_caught_up` exist nowhere under `crates/` (`tcp_node_disk_high_water` exists and is the plant of the paid atom `l28_tcp_hw`); plant `l28_real_tcp_add_member_joint_cnew` is absent from `tests/l28_real_tcp.rs` (31 tests, none so named); zero Lean defs. The 7 pairs left the catalog (299→292; `glue.data_fate` 68→61, cap 61; `single_artifact` 285 — also fixed a stale 291-vs-292 glue count). No anchored count broke: 3 gates + ledger + `host_anchor_table` green before and after. Verdict doc: `findings/2026-09-11-rfc0210-p12-fantasmas-l28/`.

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
| `from_record_type` | `from_record_type_as_is` | `wal/recover_kernel.rs` (pair `wal_recover` is single_artifact: production file is the Verus term) |
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

### Refused — Iterator / dyn shapes (RFC-0188 P2.3 — each item carries its MEASURED negative; series declared closed)

Re-measured 2026-09-10 on the pin (Aeneas `daa85d7`, Charon `0.1.232`,
Lean 4.31.0): each refusal below was re-extracted alone
(`--start-from <fn>`), so the named error is the item's own blocker —
not inherited. The Isolated-method conversion series (heap-sift
`sift_step` first, RFC-0187 P1.3 / RFC-0188 P0.2) is CLOSED: every
remaining Iterator/dyn refusal either has an active formal route around
it or an upstream voice carrying the repro.

| site (production file) | measured negative (this pin) | active formal route |
|---|---|---|
| `StreamingVisibleIter` / `WindowKvIter` hold `Box<dyn Iterator>` (`merge.rs`) | type-decl refused: `[Error] Dynamic trait types are not supported yet` (charon translates the whole crate — 100% Aeneas side; fire 803 + repro `formal/aeneas/repro/dyn-iterator/run.sh` re-verifies it in 2 structs) | `iter_window_keep` / `visible_at` / `sift_step` Isolated-method kernels (order + fate without the `dyn`); upstream [aeneas#1343](https://github.com/AeneasVerif/aeneas/issues/1343) |
| `level_distinct` (`lsm_r1_kernel.rs`) | body ignored: `[Error] Returns inside of nested loops are not supported yet` | none claimed — the distinctness invariant stays model-side (no Lean theorem names it today); re-open only if a proof need appears or Aeneas lifts the nested-loop limit |
| `inv_lsm` (`lsm_r1_kernel.rs`) | body ignored: `[Error] Breaks to outer loops are not supported yet` (also transitively `level_distinct`) | same row as above |
| `lsm_flush` (`lsm_r1_kernel.rs`) | body hole: `[Error] Could not match the contexts` | `lsm_compact` (the extracted patch path over `level_put`/`level_remove`) is the enrolled flush image |
| unpacked `probe_order` (`filter.collect`) | Iterator chain holes (CFailure); the packed `probe_order_covering` index-`while` extracts after patching the nested `Vec.push` (`Could not match the contexts`) | `probe_order_covering` is the engine-facing packed image — extracted with theorems `covering_hi_ge_oob` / `probe_order_covering_is_loop` |
| cross-lib merged Kernels | measured cross-lib refuses below — the two generated Kernels do not merge | composed-edge theorems import both libs instead |

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
| ConcurrentDb `validate_occ_batch` | `group_validate` / `occ_conflict` (Verus SA on production file) | `group_validate_lagging_member_conflicts` / `occ_conflict` closed form | `group_occ_vs_serialized_same_input` |
| ConcurrentDb fence | `fence_publish_seq` (Verus SA on production file) | `fence_is_max_member_seq` | `fence_publish_seq_as_is` first-member |
| ConcurrentDb `validate_occ_batch` | `occ_member_fate` ∧ `group_validate` (Verus SA on production file) | `occ_member_fate_conflict_via_group_validate` | `occ_member_fate_as_is_dente` |
| ConcurrentDb `validate_occ_batch` / `lone_commit` | `occ_batch_plan` ∧ `occ_member_fate` ∧ `occ_conflict` (Verus SA on production file) | `occ_batch_plan_lagging_conflict` | `occ_batch_plan_as_is_dente` |
| ConcurrentDb `validate_occ_batch` N-way | `group_validate` ∧ `occ_batch_plan` ∧ `occ_conflict` (Verus SA on production file) | `group_validate_n3_one_lagging` / `occ_batch_plan_n3_one_lagging` | `occ_batch_plan_as_is` all-Ok |
| ConcurrentDb `finish_group_off_lock` | `rwlock_client_may_mutate` (Verus SA on production file) | `rwlock_client_may_mutate_needs_write` | `rwlock_client_may_mutate_as_is_dente` |
| ConcurrentDb `occ_snapshot` | `rwlock_client_may_read` ∧ `rwlock_client_may_mutate` (Verus SA on production file) | `rwlock_client_may_read_needs_guard` | `rwlock_client_may_read_as_is_dente` |
| ConcurrentDb off-lock fd | `may_publish_group` ∧ Flush `wal_rotate_decision` (Verus SA on production file) | `concurrent_publish_and_inflight_keep_wal` | `concurrent_publish_ok_and_idle_rotates` / `concurrent_as_is_publish_lie_inflight_still_keeps` |
| ConcurrentDb `occ_snapshot` | `occ_snap_uses_published` ∧ `may_publish_group` | `occ_snap_published_and_no_publish_on_wal_fail` | `occ_snap_uses_published_as_is_dente` |
| ConcurrentDb `occ_snapshot` | `occ_snap_lock_order` ∧ `occ_snap_uses_published` (Verus SA on production file) | `occ_snap_lock_order_write_held` | `occ_snap_lock_order_as_is_dente` |
| ConcurrentDb lock-order | Flush `wal_rotate_decision` (`commit_inflight`) | `wal_rotate_commit_inflight_keeps` | `wal_rotate_idle_rotates` |
| TransactionDB 2PL | `wait_for_deadlock` | `wait_for_deadlock_is_loop` | `wait_for_deadlock_as_is_dente` / `wait_for_deadlock_loop_vs_as_is` |
| ConcurrentDb scheduler residual | `lock_interleavings_admitted` (Verus SA on production file) | `lock_interleavings_not_a_theorem` | `lock_interleavings_as_is_dente` |
| ConcurrentDb PCT residual | `forall_schedules_admitted` (Verus SA on production file; always false) | `forall_schedules_pct2_not_admitted` | `forall_schedules_as_is_dente` |

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
  - `probes_worst_l0_trigger_via_point_get` (concrete N: 4+4=8; unfolds `probes_worst` ∧ `point_get_probes`)
  - `scale_forecast_is_three_clocks` (unfolds `scale_forecast` ∧ `best_get_ns`)
- **Now claimed:** `saturating_add` 4+1=5 / 5+1=6 after `unfold SCALE_L0_BEST`
  (`rfc0176_10b_is_six_probes`). Clock \(T\) / noisy neighbor stay measured.
- Run: `./scripts/aeneas_scale.sh` then `lake build Scale` in `formal/aeneas/lean`.


## 2026-09-11 — locktab `wait_for_deadlock` (RFC-0202 P1.1): fronteira datada

- O corpo do loop extraído chama `HashMap.get` / `HashSet.insert` como
  AXIOMAS (Aeneas não extrai semântica de mapa) — o iff completo
  "detector dispara ↔ existe ciclo" NÃO é provável do extract.
- Pago (Locktab.lean): as três arestas de saída de UM passo do detector,
  com hipóteses de lookup fixando os resultados das chamadas-axioma:
  nowait⇒`done false` (`wait_for_deadlock_step_nowait_is_alive`),
  ciclo-fecha-no-waiter⇒`done true` (`wait_for_deadlock_step_cycle_closes`),
  revisit⇒`done true` (`wait_for_deadlock_step_revisit_reports_cycle`).
- Lição de prova: `rw` sozinho não reduz os binds do do-block
  (transparência do rfl automático); `simp` fecha as arestas sem lookup
  encadeado, `simp [hkey]` normaliza e aí reescreve o lookup exposto.
- Registrado como atom — o degrau que o extract suporta; a semântica de
  mapa permanece TCB (nunca claim de ciclo sem essa semântica).

## 2026-09-11 — fronteira do handler (RFC-0205 P2.1): o que o proof-term cobre e o que fica TCB

- O PROOF-TERM COBRE os kernels que o rustc de produção liga: 239
  entradas extraídas, escada registrada 6 close + 39 atom + 7 count,
  17 libs compose — cada par com twins DST dirigindo a função de
  produção (`--lib`, nunca cartoon). A ponte compose
  (`ComposeConcurrent.lean`) liga dois kernels registrados
  (group_commit × flush) por teorema, não por fé.
- TCB 1 — composição de handlers (db.rs): o caller-graph de 112.092
  LOC de `db.rs` (série `handler_loc` no residuals) não é extraído;
  cada handler (`try_rotate_wal`, `on_request_vote`,
  `recover_apply_committed`, `persist_log_db`, …) é coberto pelo fate
  do SEU kernel + pelas plantas DST no caminho ao vivo — a composição
  COMPLETA dos handlers permanece TCB.
- TCB 2 — escalonador/interleavings: admission `lock_interleavings_
  admitted` segue SEMPRE false — recusa registrada e plantada
  (`claim_lock_interleavings_refused_after_put`,
  concurrent.rs:3733; AS-IS admitiria). Nenhum ∀π sobre
  interleavings de ConcurrentDb é claim deste repo (0056 P2.1).
- TCB 3 — HashMap: o detector de deadlock fica na fronteira datada
  acima (locktab `wait_for_deadlock`, 2026-09-11): lookups são
  axiomas do extract; arestas de UM passo provadas, semântica de mapa
  e existência de ciclo NÃO são claim.
- TCB 4 — mídia e todos-os-schedules: `media_durable_admitted`
  (recusa `claim_media_durable_refused_after_fsync_ok`) e
  `forall_schedules_admitted` (PCT depth segue 2; serial=parallel por
  hash NÃO é ∀π) seguem SEMPRE false. As três admissions —
  `media_durable_admitted`, `forall_schedules_admitted`,
  `lock_interleavings_admitted` — NUNCA flipam; cada uma tem recusa
  plantada no código de produção.

## 2026-09-11 — bloco l28 (RFC-0208 P2.1): plano datado dos 29 `data_fate` restantes

Estado: 33 pares l28 no catálogo; 2 pagos (`l28_tcp_left`,
`l28_tcp_hw` — L28.lean, plantas TCP reais verdes); 29 com
`data_fate` pendente. Plano:

- **Molde pure-lift, verificado nos dentes (4×: removed_steps_down,
  disk_membership, l28_tcp_left, l28_tcp_hw).** Cada corpo Rust é a
  IDENTIDADE em um Bool (`pub fn l28_tcp_X_ok(ok: bool) -> bool { ok }`);
  o extract é o pure-lift `ok b` e o teorema é

  ```lean
  theorem l28_tcp_X_ok_fate_iff :
      ∀ (b : Bool) (v : Bool),
        (l28_tcp_X_ok b = ok v) ↔
          ((v = true ∧ b = true) ∨ (v = false ∧ b = false)) := by
    intro b v
    unfold l28_tcp_X_ok
    cases b <;> cases v <;> simp
  ```

  Risco de corpo opaco por entrada: NENHUM para as 22 abaixo — o corpo
  é literalmente `ok b` (linha única), sem trait, sem mapa, sem
  do-block. O custo real por entrada é a planta TCP real (~4 min cada)
  e o commit único (critério de aceite 1 do 0208).

- **Ordem — 22 extraíveis, em cadências de 4 (moldura P1.2), cada uma
  com a planta real nomeada (`tests/l28_real_tcp.rs`):**
  1. `l28_tcp_dterm` (removed_durable_term), `l28_tcp_part`
     (participating_after_remove), `l28_tcp_apply` (recover_apply),
     `l28_tcp_napply` (removed_recover_apply)
  2. `l28_tcp_trunc`, `l28_tcp_odrop`, `l28_tcp_abort`,
     `l28_tcp_nowms` (removed_*)
  3. `l28_tcp_hist`, `l28_tcp_fence`, `l28_tcp_clear`,
     `l28_tcp_pre` (removed_*)
  4. `l28_tcp_peer`, `l28_tcp_lid`, `l28_tcp_rdr`,
     `l28_tcp_dsc` (removed_*)
  5. `l28_tcp_pld`, `l28_tcp_std` (removed_*), `l28_tcp_hnt`
     (hint), `l28_tcp_slot` (drop_repl)
  6. `l28_tcp_sth` (drop_st), `l28_tcp_pj` (plant_joint)

  Meta numérica RELATIVA ao estado de 2026-09-11 (cap 84,
  floor_atom 47, floor_extract 231): as 22 promoções fecham
  cap_data_fate 84→62, floor_atom 47→69, floor_extract 231→209.

- **7 fantasmas de catálogo — conserto, NÃO extração:**
  `l28_tcp_add`, `l28_tcp_cnew`, `l28_tcp_svget`,
  `l28_tcp_newget`, `l28_tcp_jleft`, `l28_tcp_caught`,
  `l28_tcp_grown` nomeiam `l28_tcp_*_ok` que NÃO existe no arquivo
  vivo (kernels TCP terminam em `l28_tcp_pj_ok`; ver seção "Catalog
  `entry`s with no Lean `def`"). Plano: re-escrever os pares para
  nomear `fn` viva ou aposentá-los no catálogo — nunca inventar
  gate de identidade para agradar o catálogo. — **DONE
  2026-09-11 (0210 P1.2): aposentados, −7** (medição e veredito na
  seção datada "verdict — retired" acima).

## 2026-09-11 — nota do seam store/raft (RFC-0208 P2.2): fechado nos kernels raft, cluster nomeado

- **Raft fechado.** Os kernels raft (vote/commit/membership-recovery)
  têm ZERO par `data_fate` pendente no catálogo no HEAD do 0208:
  `vote` (`vote_decision`, 0205), `recover_apply` +
  `recover_drop_orphan` (0205), `grant_persist` +
  `commit_raft` (0208 P0), e a cadência membership ×4 (0208 P1.2:
  removed_steps_down, disk_membership_overrides_cli,
  high_water_at_least, joint_still_active) — medido ao vivo: nenhum
  par `data_fate=True` em vote_kernel.rs / commit_kernel.rs /
  membership_kernel.rs.
- **Cluster 8/66 pagos pelo 0208** (2 singletons + 4 membership + 2
  l28_tcp), mais a composição ∀ do cluster em ComposeStoreRaft.lean
  (election_grant_chain_fate + recovery_fate_composed, sem registro
  por não serem par único do catálogo — razão em findings).
- **Restante vivo do cluster: 58 nomeados** (não 55 — o texto do
  slice subtraiu também o trio do 0205, que JÁ estava fora dos 66
  na contagem do board; correção datada aqui):
  - 22 em `crates/pedradb-raft/src/membership_kernel.rs`:
    discard_leader, discard_uncommitted, drop_preimages,
    drop_repl_slot, drop_sent_through, force_clear, hint_member,
    identity_before_applied, joint_add_target, joint_leave_ok,
    joint_target, local_id_member, open_peer_disk,
    participating_member, pending_joint_node, persist_fence,
    persist_hist, persist_meta, reader_local, recover_abort,
    recover_apply_node, recover_truncate
  - 29 em `l28.rs` — plano datado próprio acima (22 pure-lifts +
    7 fantasmas aposentados em 2026-09-11 pelo 0210 P1.2)
  - 6 em `txn_kernel.rs`: discard_cut, leftover_txn_is_aborted,
    next_txn_id_after, prepare_error_aborts_earlier, reserve_si_gen,
    unreserve_si_gen
  - 1 singleton: `compact_unleft` (compact_kernel.rs)
- As três admissions (`media_durable_admitted`,
  `forall_schedules_admitted`, `lock_interleavings_admitted`)
  seguem ALWAYS false — recusas plantadas; o TCB nomeado no 0205
  P2.1 permanece intocado.

## 2026-09-11 — bloco l28 DRENADO (RFC-0210): ZERO `data_fate` pendente

Medido ao vivo no HEAD do 0210: 26 pares l28 no catálogo, ZERO com
`data_fate` pendente — 22 pure-lifts promovidos a atoms registrados
(L28.lean, 22 teoremas `_ok_fate_iff`; plantas TCP reais verdes) +
7 fantasmas aposentados no P1.2 (fe236f91, recusa datada). Escada
final do 0210: cap 84→55, floor_atom 47→69, floor_extract 231→209.
Composição ∀ do protocolo de remoção TCP em `ComposeL28.lean` (20ª
compose lib; remove → left ∧ high-water preservado sobre os atoms
`catalog:l28_tcp_left` × `catalog:l28_tcp_hw`; SEM registro no
TSV — não é par único do catálogo, mesma regra das demais compose
libs). Restam nomeados no cluster: 29 (22 em
`crates/pedradb-raft/src/membership_kernel.rs`, 6 em
`crates/pedradb-store/src/txn_kernel.rs`, 1 singleton
`compact_unleft` em `compact_kernel.rs`).

## 2026-09-11 — bloco cluster DRENADO (RFC-0212): ZERO `data_fate` pendente

Medido ao vivo no HEAD do 0212: os 29 pares do bloco cluster — 22 em
`crates/pedradb-raft/src/membership_kernel.rs`, 6 em
`crates/pedradb-store/src/txn_kernel.rs`, 1 singleton `compact_unleft`
em `compact_kernel.rs` — ZERO com `data_fate` pendente: 29 atoms
registrados (Membership.lean ×22; StoreTxn.lean ×6 com o wrapper
inscrito no gate de extracts — LIBS 62 libs, buraco pré-existente
desde o RFC-0191 fechado; StoreCompact.lean ×1). Escada final do
0212: cap 55→26, floor_atom 69→98, floor_extract 209→180.
Composição ∀ do protocolo de fim-de-fila queued em
`ComposeStoreFinish.lean` (21ª compose lib; discard-leader local ∧
discard conta ∧ cerca/hist persistem conforme o fate sobre os atoms
`catalog:discard_leader` × `catalog:discard_uncommitted` ×
`catalog:persist_fence` × `catalog:persist_hist`; SEM registro no
TSV — não é par único do catálogo, mesma regra das demais compose
libs). Restam nomeados, todos storage: 26 (write_admission 9,
lookup 4, flush 3, cf 2, leveling 2 + 6 singletons: wal_recover,
dictionary_link, visible_at, write_record_count, iter_window,
occ_batch_plan).

## 2026-09-12 — bloco storage DRENADO (RFC-0213): catálogo inteiro, ZERO `data_fate` pendente

Medido ao vivo no HEAD do 0213: os 26 pares do bloco storage —
write_admission 9 (WriteAdmission.lean), lookup 4 (Lookup.lean),
flush 3 (Flush.lean), cf 2 (Cf.lean), leveling 2 (Leveling.lean) +
singletons wal_recover (WalRecover.lean, inscrito no LIBS nesta
data), dictionary_link (Reopen.lean, inscrito no LIBS; atom desde
2026-09-10, fatia P2.1 fortaleceu ao fate-iff e pagou só o cap),
visible_at, write_record_count, iter_window, occ_batch_plan — ZERO
com `data_fate` pendente: 292 pares no catálogo, ZERO `data_fate`
no conjunto. Escada final do 0213: cap 26→0, floor_atom 98→122,
floor_extract 180→156; LIBS 62→64 (WalRecover + Reopen, buracos
pré-existentes fechados com extratos re-carimbados byte-idênticos).
Composição ∀ do caminho de storage em `ComposeStorageWrite.lean`
(22ª compose lib; admission portão → plano cerca → recovery corta
sobre os atoms `catalog:write_admit` × `catalog:wal_commit_plan` ×
`catalog:torn_tail_needs_cut`; SEM registro no TSV — não é par
único do catálogo, mesma regra das demais compose libs). Restam
nomeados: ZERO — os três blocos (membership 0211, cluster 0212,
storage 0213) drenados; catálogo fechado sem `data_fate` pendente.

## 2026-09-12 — espinha de durabilidade no degrau átomo (RFC-0214): env ×6 + wal_state ×6 + cqe ×5 + write_ack ×3

Medido ao vivo no HEAD do 0214: os 20 pares da espinha de
durabilidade — wal_state ×6 (WalState.lean: `inv_wal_fate_iff`,
`wal_append_fate_iff`, `wal_sync_fate_iff`, `wal_ack_fate_iff`,
`wal_rotate_fate_iff`, `acked_survives_fate_iff`), env_crash ×6
(EnvCrash.lean: `crash_legal_fate_iff`, `append_fate_iff`,
`sync_fate_iff`, `barrier_floor_fate_iff`,
`no_invented_bytes_fate_iff`, `honest_sync_protects_all_fate_iff`),
cqe ×5 (Cqe.lean: `cqe_res_ok_fate_iff`, `next_user_data_fate_iff`,
`cqe_act_fate_iff`, `submit_complete_act_fate_iff`,
`cqe_ring_model_admitted_fate_iff`) e write_ack ×3 (WriteAck.lean:
`on_append_fate_iff`, `on_barrier_fate_iff`, `on_ack_fate_iff`) —
todos promovidos ao degrau átomo com teorema iff-∀ sobre o corpo
extraído, 1 promoção = 1 commit. Escada final do 0214: floor_atom
122→142, floor_extract 156→136 (close=6, data_fate=0 imutáveis);
gate GREEN no HEAD de cada promoção. A planta DST do write_ack
exigiu correção live real (commit 0d7324da): o caminho lone G1
(`lone_commit`) não avançava o ledger pinado — puts sync de cliente
único no perfil verificado deixavam o ledger frio. Composição ∀ da
espinha em `ComposeDurabilitySpine.lean` (23ª compose lib;
Inv-WAL invariante de todo caminho append/barrier/ack + coroa D1
sobre todo corte torn, sobre os atoms `catalog:write_ack_append` ×
`catalog:write_ack_barrier` × `catalog:write_ack_ack`; SEM registro
no TSV — não é par único, mesma regra das demais compose libs);
twin kernel `durability_spine_kernel.rs` + planta DST
`durability_spine_compose_on_live_profile_is_not_ok`. Restam no
degrau extrato: 136 (nenhum `data_fate` pendente, catálogo
fechado).

## 2026-09-12 — coroa de produto no degrau átomo (RFC-0215): spec ×4 + modelo ×4 + fate ×2 + espinha→coroa + http ×6

Medido ao vivo no HEAD do 0215: os 16 pares da coroa de produto —
spec ×4 (Properties.lean: `c1_holds_fate_iff`, `d1_holds_fate_iff`,
`t1_holds_fate_iff`, `r1_answer_ok_fate_iff`), modelo ×4
(D1Modelo.lean `d1_modelo_fate_iff`, LsmR1.lean `r1_modelo_fate_iff`,
T1Modelo.lean `t1_modelo_fate_iff`, C1Modelo.lean
`c1_modelo_fate_iff`), fate ×2 (`d1_put_ok_fate_iff`,
`c1_advance_commit_fate_iff`) e http ×6 (Auth.lean:
`is_bearer_scheme_fate_iff`, `is_non_bearer_auth_scheme_fate_iff`,
`authorization_matches_fate_iff`; Form.lean:
`normalize_http_method_fate_iff`, `ascii_lower_fate_iff`,
`ascii_upper_fate_iff`) — todos promovidos ao degrau átomo com
teorema iff-∀ sobre o corpo extraído, 1 promoção = 1 commit.
Escada final do 0215: floor_atom 142→158, floor_extract 136→120
(close=6, data_fate=0 imutáveis); gate GREEN no HEAD de cada
promoção; sweep final em worktree DENTRO de software/ com os três
gates GREEN (depth-floor, inventory-terminal, twin-contracts) +
campaign ok + extracts --required exit 0 (64 libs + 24 compose) +
sorry 0 nos wrappers da rodada. Composição espinha→coroa em
`ComposeProductCrown.lean` (24ª compose lib): sobre todo ledger
`spine_reach` do 0214, as duas pernas juntas — `d1_modelo = ok
true` (cita a iff; corpo extraído não reaberto; perna geminada
`wa_d1_modelo_fate_iff` em WriteAck.lean resolve o choque de import
das cópias geradas) e `d1_holds = ok true` (perna P0.1); SEM TSV —
atravessa múltiplos átomos; twin kernel `product_crown_kernel.rs` +
planta DST `product_crown_compose_on_live_profile_is_not_ok`.
Bugs do motor achados pelas plantas: SST v5 vazio no reopen caía no
fail-closed com corrupção inventada (fix 30e572db). Restam no
degrau extrato: 120 (nenhum `data_fate` pendente, catálogo
fechado).

## 2026-09-13 — superfície de parse HTTP no degrau átomo (RFC-0216): cl ×4 + fail_closed ×1 + form ×7 + path ×8

Medido ao vivo no HEAD do 0216: os 20 pares de parse HTTP —
cl ×4 (Cl.lean: `keep_body_without_cl_fate_iff`,
`invalid_cl_as_zero_fate_iff`, `content_length_repeat_ok_fate_iff`,
`short_body_vs_cl_is_error_fate_iff`), fail_closed ×1
(FailClosed.lean: `parse_error_writes_status_fate_iff`), form ×7
(Form.lean: `form_plus_byte_fate_iff`,
`plus_before_percent_fate_iff`, `from_hex_fate_iff`,
`form_decode_fate_iff`, `query_values_conflict_fate_iff`,
`query_u64_conflict_fate_iff`, `query_part_is_bare_name_fate_iff`)
e path ×8 (Path.lean: `strip_authority_for_routing_fate_iff`,
`strip_uri_fragment_fate_iff`, `path_after_authority_fate_iff`,
`strip_http_authority_fate_iff`,
`host_authority_mismatch_fate_iff`, `origin_form_path_fate_iff`,
`split_host_port_fate_iff`, `request_target_authority_fate_iff`) —
todos promovidos ao degrau átomo com teorema iff-∀ sobre o corpo
extraído, 1 promoção = 1 commit. O `request_target_authority`
(8/8) fecha o último gate vivo do plano de request: o `?` do
`strip_prefix("//")` fica citado pelo par opaco
branch/from_residual do Charon (Continue/Break explícitos no
enunciado). Escada final do 0216: floor_atom 158→178,
floor_extract 120→100 (close=6, count=7, data_fate=0 imutáveis);
gate GREEN no HEAD de cada promoção; sweep final em worktree
DENTRO de software/ com os três gates GREEN (depth-floor,
inventory-terminal, twin-contracts) + campaign ok + extracts
--required exit 0 + sorry 0 nos wrappers da rodada. Família http
do catálogo 27/27 em átomo — nenhum gate do plano de request
decidido por teste em vez de teorema. Restam no degrau extrato:
100 (nenhum `data_fate` pendente, catálogo fechado).

## 2026-09-13 — RFC-0218 fechada (P0/P1/P2 todas done, 270/292)

A drenagem do RFC-0218 fecha o degrau extrato do sistema inteiro:
88 pares pagáveis promovidos ao degrau átomo (1 iff-∀ = 1 commit),
floor_atom 178→266, floor_extract 120→12 (close=6, count=7,
data_fate=0 imutáveis). Escada: P0.1–P0.4 → 205/292; P1.1–P1.3 →
237/292; P2.1 → 249/292; P2.2 → 270/292 = 92,47%. Sweep final em
worktree DENTRO de software/ (pedradb-r0218-p23-sweep @ 4b89a8ce):
depth-floor, inventory-terminal, twin-contracts GREEN; campaign ok;
extracts --required exit 0; sorry 0 nos wrappers da rodada. O
residual de 12 extratos é o registrado: 12 pares canon-excluídos
(admitted/campaign stand-ins, nunca flipados) + 10 pares cartoon
Montanha (portão do usuário) — o próximo salto é RFC-0219.

## 2026-09-13 — RFC-0219 P2.1: 15 recusas medidas db.rs (número publicado)

A fila restante de `db.rs` foi medida sítio a sítio: 19 linhas = 15
`if let` recusados (binding usada no corpo: R1 Err×9, R2 Option×3, R3
lookup-Option×3 — detalhe e captura datada em
`findings/2026-09-13-rfc0219-p21-recusas/README.md`) + 4 linhas de
comentário. Nenhum desses sítios isola uma decisão data-fate em kernel
sem embrulhar `is_some`/`is_ok` (anti-padrões do RFC). O alvo datado
do P2 ajusta com o número: teto sem recusa em concurrent.rs =
310/332 = 93,37% (era 345/367 = 94,01% assumindo 75 pares; medido:
db.rs rendeu 18 pares em 34 sítios resolvidos).

## 2026-09-13 — RFC-0219 P2.2 fecha: concurrent.rs 19 sítios = 2 pares + 9 drenos + 3 recusas

A fila de `concurrent.rs` foi resolvida: `flusher_gate_plan` (5
portões workerless/worker) e `parked_debt_plan` (2 portões de dívida)
nascem átomos com teoremas iff-∀ (`Flush.lean`); 9 portões drenam a
kernels já pareados (changelog_durable_commit_fate, occ bools,
fence_admission_plan, manifest_publish_plan — commit f0671326); 3
recusas medidas publicadas (R4 local derivado do match pareado, R5
wrap-is_empty anti-padrão, R1 Err de I/O — detalhe e captura datada
em `findings/2026-09-13-rfc0219-p22-recusas/README.md`). Contador
concurrent.rs 22→6 (3 linhas de doc). Fim do RFC-0219 medido:
**290/312 = 92,95%** (início 270/292 = 92,47%; +20 pares, alvo P2
re-datado fechado com número publicado).
