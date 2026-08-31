# Pedra formal glue

Not a prover. Refuses silent drift between the production kernel, the Verus
twin, and the handler that is supposed to call the kernel.

```sh
./scripts/pedra_formal.sh           # same as --ci
./scripts/pedra_formal.sh --ci      # lint + clones + twins + scripts + Stateright (no Verus)
./scripts/pedra_formal.sh --all     # --ci plus Verus when installed (warn if the toolchain is missing)
./scripts/pedra_formal.sh --strict  # also fail on catalog status=absent (missing twins)
./scripts/pedra_formal.sh --verus-required
./scripts/pedra_formal.sh --extract          # crate + lake + Charon if installed
./scripts/pedra_formal.sh --extract-required
./scripts/lean_vote.sh --required            # lake build Vote only
```

| Check | What it refuses |
|-------|-----------------|
| **lint** | A kernel whose `entry` is never called from the listed production file. `data_fate` kernels (RFC-0053) also require named `handlers` (`handle_request_vote`, `rpc_request_vote`, …). RFC-0151: every `data_fate` pair also needs `as_is` in the kernel file and a named DST plant (`dst_plant.file` + `dst_plant.test` that calls `entry(`). Raft / `l28_*` plants must mention `pin_dst_queued` or `RpcMode::Queued`. RFC-0152: `vote` / `ae_entry` / `grant_persist` / `ae_ack` / `commit_raft` / `joint_election` / `joint_leave` / `pending_joint_node` / `joint_leave_ok` / `election_grant_from` / `joint_target` / `joint_add_target` / `queued_leave_finish` / `disk_membership` / `high_water` / `participating_member` / `identity_before_applied` / `recover_apply` / `recover_apply_node` / `recover_truncate` / `recover_drop_orphan` / `recover_abort` / `persist_meta` / `persist_hist` / `persist_fence` / `force_clear` / `drop_preimages` / `open_peer_disk` / `local_id_member` / `reader_local` / `discard_uncommitted` / `discard_leader` / `removed_step_down` / `hint_member` / `drop_repl_slot` / `drop_sent_through` / `apply_step` list `live_callers` on store handlers (fails naming the pair if the store stops calling the kernel). Raft-kernel `data_fate` pairs that store `lib.rs` already calls must list `live_callers`. Non-`data_fate` `three_teeth` pairs (e.g. `rpc_mode`) are checked like 0151. |
| **clones** | Raft vs store copies of `recover_commit` / `ae_ack_success` / `vote_decision` / `ae_entry_action` drifting |
| **twins** | Missing `twin_kind`; a **close** twin without the entry `fn`; kernel decision tokens (`==`, `0xff`, `WouldGrant`, …) missing from the Verus twin |
| **scripts** | A `scripts/verus_*.sh` whose `SRC=` is not in `catalog.json` |
| **models** | Stateright through `fields_model` / `pack_model` / `ship_model` / `scan_model` / `si_read_model` / `fold_range_model` + EXPLODE `recover_choose` (production `fn`, plus AS-IS mutant) |
| **extract** | Extract crate for `vote_kernel.rs` fails; `lake build Vote` skipped unless `lake` is installed; Charon skipped unless `--extract-required` |
| **verus** | Twin no longer verifies (skipped if `verus` is not on the machine) |
| **stamp guard** | `.githooks/pre-commit` (install once per clone: `git config core.hooksPath .githooks`) blocking a commit that changes an extracted kernel without re-stamping its `formal/aeneas/out/SOURCE*`; lint also fails if the hook is removed or stops checking the stamps |

`twin_kind` is `close` (entry is in the twin), `atom` (smaller `atom` fn only), or `model` (same name, stand-in domain such as `u64` for `&[u8]`). A close twin that only proves a cartoon `fn` must be relabeled `atom`.

`status: absent` is a recorded gap (script exists, twin file does not). None remain after the F59/F60/F62/F80/F83 twins.

Catalog is the checklist. Adding a kernel means a row in `catalog.json` in the same change as the `fn`.
