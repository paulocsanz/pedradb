# RFC: 0152 — Live Queued RequestVote / AppendEntries is the catalog kernel

**Status:** in-progress
**Updated:** 2026-08-29
**Parents:** [0151](0151-three-teeth-as-is-verus-dst.md), [0002](0002-internal-key-memtable.md) F15/F16, [0067](0067-dst-queued-rpc-pin-fail-closed.md)

**Residual:** glue TCB (not `never_floor`). No extract of `db.rs`. L28 real TCP stays a campaign, not ∀ traces. CRC collision / Linux fsync / io_uring ring remain axioms.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or seL4-class. The sentence that *is* refused: the live Queued grant/truncate `if` is a second inline predicate while the catalog kernel only runs in raft / after opening a cluster.

## Background

- RFC-0151 froze AS-IS + Verus + a named DST plant on every `data_fate` pair. Raft plants pin Queued RPC.
- The store live path (`on_request_vote` / `on_append_entries`) still inlined `can`/`up` and `e.index <= p.commit`. The catalog `vote` / `ae_entry` kernels lived in `pedradb-raft`; store has no production raft dependency (`ae_ack_success` is a cloned kernel).
- Existing plants `vote_decision_on_live_queued_is_not_ok` / `ae_entry_action_on_live_queued_is_not_ok` opened a Queued cluster then called the pure fn — they did not send inbound `PeerMsg::RequestVote` / `AppendEntries`.

## Problems This Solves

- **Problem:** A Verus twin of `vote_decision` does not speak for the replica that actually grants a vote on Queued inbound.
- **Problem:** F16 “never rewrite a committed index” could hold in raft and still be a second `if` in the store handler that DST actually pumps.
- **Problem:** A plant that only calls the pure fn after `pin_dst_queued` can stay green while the live handler drifts.

## Proposed Solution

- Clone-freeze `vote_decision`, `grant_after_persist`, and `ae_entry_action` into the store (same pattern as `ae_ack_raft_store`). Production `on_request_vote` / `on_append_entries` call those fns. `--lint` `live_callers` fails naming `vote` / `grant_persist` / `ae_entry` if the store handlers stop invoking them (without requiring raft `handle_request_vote` names in `lib.rs`).
- Named DST plants on Queued send real inbound RV/AE via `handle_inbound`. Stale-log / already-voted RequestVote is not granted; committed-index conflict is not truncated. AS-IS of the same kernel would be silent-wrong.
- Do not extract `db.rs`. Do not add a production `pedradb-raft` dependency.

## Delivery slices (mandatory)

### P0 — must ship first (live grant/truncate is the catalog kernel)

- [x] **P0.1** Store live RequestVote / AppendEntries invoke catalog `vote` / `ae_entry` (clone freeze + `live_callers`); `--lint` names the pair id if the call is dropped — status: `done`
- [x] **P0.2** Queued DST plants send inbound `PeerMsg::RequestVote` / `AppendEntries` and assert the AS-IS dente of the same kernel — status: `done`

### P1 — next wave (same RV path, persist-before-grant)

- [x] **P1.1** Store RequestVote wire bit is `grant_after_persist` (catalog `grant_persist`), not an inline persist-then-bool — status: `done`
- [x] **P1.2** Store AppendEntries persist-ack is inbound-planted `ae_ack` (`grant_after_persist` analog); `--lint` names `ae_ack` if the store drops `ae_ack_success` — status: `done`
- [x] **P1.3** Raft-kernel `data_fate` pairs that store `lib.rs` already calls list `live_callers` (drop fails naming the pair) — status: `done`
- [x] **P1.4** Store Queued client Ok is inbound-planted `propose_ack_ok` (catalog `commit_raft`); put without AE replies is NotCommitted — status: `done`
- [x] **P1.5** Store Queued joint election is inbound-planted `joint_election_ok`; old-only majority during add does not elect — status: `done`
- [x] **P1.6** Store Queued inbound MembershipJoint is visible to `pending_joint_on` via `joint_still_active` (catalog `joint_leave`); AS-IS would skip — status: `done`
- [x] **P1.7** Store Queued `pending_joint` ignores a removed node's leftover joint (`pending_joint_node_counts`); AS-IS would scan it — status: `done`
- [x] **P1.8** Store Queued inbound C-new-only leave is in the log (`joint_leave_ok`); AS-IS would skip leave — status: `done`
- [x] **P1.9** Store Queued inbound RV grant from a non-member is not counted (`election_grant_from_counts`); AS-IS would count it — status: `done`
- [x] **P1.10** Store Queued TCP replica `remove_member_joint` uses `ids` not local `nodes` (`joint_target_counts`); AS-IS would require the peer in `nodes` — status: `done`
- [x] **P1.11** Store Queued TCP replica `add_member_joint` does not require the joiner in local `nodes` (`joint_add_target_counts`); AS-IS would — status: `done`
- [x] **P1.12** Store Queued inbound C-new-only leave is not finished until committed (`queued_leave_finish_ok`); AS-IS would treat in-log as enough — status: `done`
- [x] **P1.13** Store Queued inbound applied leave persists C-new; `bind_cluster_identity` restores disk not stale CLI (`disk_membership_overrides_cli`); AS-IS would overwrite disk — status: `done`
- [x] **P1.14** Store Queued inbound applied leave persists high-water 4; TCP ctor CLI 3 keeps the floor (`high_water_at_least`); AS-IS would use RAM 3 — status: `done`
- [x] **P1.15** Store Queued inbound applied leave drops 4 from `ids`; stale `participating=true` does not count (`participating_if_member`); inbound RV is not granted — status: `done`
- [x] **P1.16** Store Queued inbound applied joint persists C-new identity before `applied` advances (`membership_identity_before_applied`); AS-IS would persist applied first — status: `done`
- [x] **P1.17** Store Queued inbound committed-unapplied joint is applied on crash-reopen (`recover_must_apply`); AS-IS would skip recover apply — status: `done`
- [x] **P1.18** Store Queued inbound committed-unapplied put on a replica already dropped from `ids` is applied on crash-reopen (`recover_apply_node_counts`); AS-IS would skip local non-member — status: `done`
- [x] **P1.19** Store Queued inbound uncommitted suffix on a replica already dropped from `ids` is truncated on crash-reopen (`recover_truncate_node_counts`); AS-IS would skip persist — status: `done`
- [x] **P1.20** Store Queued inbound uncommitted Put writes `log_entry_key`; crash-reopen deletes the orphan (`recover_drop_orphan_seg`); AS-IS would leave the key — status: `done`
- [x] **P1.21** Store Queued inbound `TxnPrepare` leftover on a replica already dropped from `ids` is aborted on crash-reopen (`recover_abort_node_counts`); AS-IS would skip local non-member — status: `done`
- [x] **P1.22** Store Queued inbound leave drops 4 from `ids`; `advance_now_ms` persists SI meta on the removed replica (`persist_meta_node_counts`); AS-IS would skip — status: `done`
- [x] **P1.23** Store Queued inbound leave drops 4 from `ids`; `persist_si_keys` writes SI hist on the removed replica (`persist_hist_node_counts`); AS-IS would skip — status: `done`
- [x] **P1.24** Store Queued inbound leave drops 4 from `ids`; `fence_txn_aborted` writes abort on the removed replica (`persist_fence_node_counts`); AS-IS would skip — status: `done`
- [x] **P1.25** Store Queued inbound leave drops 4 from `ids`; `force_local_clear_keys` drops a stuck intent on the removed replica (`force_clear_node_counts`); AS-IS would skip — status: `done`
- [x] **P1.26** Store Queued inbound leave drops 4 from `ids`; `drop_preimages` drops leftover preimage on the removed replica (`drop_preimages_node_counts`); AS-IS would skip — status: `done`
- [x] **P1.27** Store Queued inbound leave persists C-new; process `open(n=4)` loads node 4 peer from disk (`open_peer_uses_disk`); AS-IS would use CLI n=4 — status: `done`
- [x] **P1.28** Store Queued inbound leave; TCP ctor of 4 has `local_node_id()==None` (`local_id_if_member`); AS-IS would claim HashMap first-key — status: `done`
- [x] **P1.29** Store Queued inbound leave; TCP ctor of 4 `get` is empty not `bad node` (`reader_id_local`); AS-IS would pick remote `ids.first()` — status: `done`
- [x] **P1.30** Store Queued inbound uncommitted Put on 4 then inbound leave; `discard_uncommitted_from` drops the suffix (`discard_node_counts`); AS-IS would skip — status: `done`
- [x] **P1.31** Store Queued inbound leave; TCP ctor of 4 `finish_queued_propose` abort repairs local `next_index` (`discard_leader_local`); AS-IS remote `ids.first()` skips — status: `done`
- [x] **P1.32** Store Queued inbound leave steps a planted Leader on 4 down (`removed_steps_down`); AS-IS would keep `Role::Leader` — status: `done`
- [x] **P1.33** Store Queued inbound leave; planted `leader_id=4` on remaining voters is not `leader_hint` (`hint_if_member`); AS-IS would route to the removed replica — status: `done`
- [x] **P1.34** Store Queued inbound leave drops planted `next_index`/`match_index`/`sent_through` of 4 (`drop_repl_slot`); AS-IS would keep the slots — status: `done`
- [x] **P1.35** Store Queued inbound AE then oob `remove_member(3)` drops planted `sent_through` of 3 (`drop_sent_through`); AS-IS would keep it — status: `done`
- [x] **P1.36** Store Queued inbound apply stops on a planted log hole (`apply_advance`); AS-IS would skip the hole — status: `done`

### P2 — later (three-teeth beyond data_fate)

- [x] **P2.1** Three-teeth on one non-`data_fate` pair (`rpc_mode` / `allow_direct_rpc`) — status: `done`
- [x] **P2.2.1** Three-teeth on `dcs_apply` / `dcs_apply_should_advance_result` (CasFailed still advances) — status: `done`
- [x] **P2.2.2** Three-teeth on `compact` / `may_compact_through` (missing term does not compact) — status: `done`
- [x] **P2.2.3** Three-teeth on `snapshot` / `snapshot_touches_user_key` (reserved keys skipped) — status: `done`
- [x] **P2.2.4** Three-teeth on `si_reader` / `si_reader_beats` (not first `ids[]`) — status: `done`
- [x] **P2.2.5** Three-teeth on `index_val` / `len_pref_value` (NUL sibling not in range) — status: `done`
- [x] **P2.2.6** Three-teeth on `changelog` / `changelog_needs_sst_rebuild` (missing CHANGELOG after flush rebuilds from SST) — status: `done`
- [x] **P2.2.7** Three-teeth on `range_covers` / `range_tombstone_covers` (interior key in `[start, end)`) — status: `done`
- [x] **P2.2.8** Three-teeth on `prefix` / `prefix_exclusive_end` (`prefix||0xff||…` stays in range) — status: `done`
- [x] **P2.2.9** Three-teeth on `stream_cursor` / `ack_in_order` (hole ack does not pin) — status: `done`
- [x] **P2.2.10** Three-teeth on `bearer` / `bearer_token_from_value` (`BEARER` scheme extracts token) — status: `done`
- [x] **P2.2.11** Three-teeth on `content_length` / `keep_body_without_cl` (PUT without CL keeps body) — status: `done`
- [x] **P2.2.12** Three-teeth on `fail_closed` / `parse_error_writes_status` (parse Err writes 400) — status: `done`
- [x] **P2.2.13** Three-teeth on `form_plus` / `form_decode` (`+` is space in query) — status: `done`
- [x] **P2.2.14** Three-teeth on `origin_path` / `origin_form_path` (absolute-form routes) — status: `done`
- [x] **P2.2.15** Three-teeth on `isolated` / `isolated_id_matches` (sibling id not a prefix) — status: `done`
- [x] **P2.2.16** Three-teeth on `children` / `packed_children_end` (zip 900 not in pack(90) children) — status: `done`
- [x] **P2.2.17** Three-teeth on `fields` / `encode_fields` (NUL inside zip is kept) — status: `done`
- [x] **P2.2.18** Three-teeth on `journal_pin` / `may_advance_pin` (pin only after apply) — status: `done`
- [x] **P2.2.19** Three-teeth on `pack` / `pack_cut_tag` (NUL components do not collide) — status: `done`
- [x] **P2.2.20** Three-teeth on `ship_guard` / `pull_plan` (rotate-regrow fails closed) — status: `done`
- [x] **P2.2.21** Three-teeth on `bloom_header` / `bloom_header_ok` (hostile k fail-closed) — status: `done`
- [x] **P2.2.22** Three-teeth on `bloom_insert` / `insert` (written key is not a false negative) — status: `done`
- [x] **P2.2.23** Three-teeth on `bloom_may_contain` / `may_contain` (inserted key is not a false negative) — status: `done`
- [x] **P2.2.24** Three-teeth on `scan_guard` / `scan_reads_file` (spanning tombstone SST is read) — status: `done`
- [x] **P2.2.25** Three-teeth on `si_read` / `snapshot_read_plan` (below-floor snapshot is TooOld) — status: `done`
- [x] **P2.2.26** Three-teeth on `fold_range` / `fold_event_hides_key` (range tombstone drops covered last-per-key) — status: `done`
- [x] **P2.2.27** Three-teeth on `group_commit` / `occ_conflict` (first-committer-wins conflicts the lagging OCC) — status: `done`
- [x] **P2.2.28** Three-teeth on `group_fence` / `fence_publish_seq` (group publish watermark is max member seq) — status: `done`
- [x] **P2.2.29** Three-teeth on `group_publish` / `may_publish_group` (failed off-lock WAL does not publish) — status: `done`
- [x] **P2.2.30** Three-teeth on `forall_schedules` / `forall_schedules_admitted` (PCT d=2 is not ∀π) — status: `done`
- [x] **P2.2.31** Three-teeth on `l28_durability` / `l28_durability_ok` (kill+restart required, not get-only) — status: `done`
- [x] **P2.2.32** Three-teeth on `liveness_claim` / `liveness_admitted` (bounded elect is not eventual-live) — status: `done`
- [x] **P2.2.33** Three-teeth on `fsync_promote` / `fsync_promotes_pending` (lying fsync does not promote) — status: `done`
- [x] **P2.2.34** Three-teeth on `media_durable` / `media_durable_admitted` (fdatasync Ok is not a media theorem) — status: `done`
- [x] **P2.2.35** Three-teeth on `tcg_guest` / `tcg_guest_admitted` (native World is not TCG coverage) — status: `done`
- [x] **P2.2.36** Three-teeth on `fdatasync_rc` / `fdatasync_rc_ok` (nonzero rc is not Ok) — status: `done`
- [x] **P2.2.37** Three-teeth on `cqe_res` / `cqe_res_ok` (negative CQE res is not Ok) — status: `done`
- [x] **P2.2.38** Three-teeth on `c_len` / `c_len_admitted` (oversize C `*_len` is LIMIT) — status: `done`
- [x] **P2.2.39** Three-teeth on `crc_match` / `crc_match_ok` (WAL CRC mismatch is Crc) — status: `done`
- [x] **P2.2.40** Three-teeth on `sst_crc` / `sst_crc_fate` (modern SST trailer mismatch is Reject) — status: `done`
- [x] **P2.2.41** Three-teeth on `ikey_pack` / `pack_sequence_and_type` (seq=1 Deletion does not collide with seq=0 Value) — status: `done`
- [x] **P2.2** Remaining non-`data_fate` pairs (0) — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | live RV/AE calls catalog vote/ae_entry | done | `vote_kernel.rs` / `ae_ack_kernel.rs` + `live_callers` | 2026-08-29 |
| P0.2 | p0 | inbound Queued plants | done | `vote_decision_on_live_queued_is_not_ok` / `ae_entry_action_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.1 | p1 | grant_after_persist on store RV | done | `vote_kernel::grant_after_persist` + inbound `grant_after_persist_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.2 | p1 | inbound ae_ack persist-ack | done | `ae_ack_success_on_live_queued_is_not_ok` + `live_callers` | 2026-08-29 |
| P1.3 | p1 | raft→store live_callers freeze | done | `check_raft_store_live` | 2026-08-29 |
| P1.4 | p1 | inbound propose_ack_ok / NotCommitted | done | `propose_ack_ok_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.5 | p1 | inbound joint_election old-only | done | `joint_election_ok_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.6 | p1 | inbound joint_still_active / pending_joint | done | `joint_still_active_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.7 | p1 | inbound pending_joint_node skip removed | done | `pending_joint_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.8 | p1 | inbound joint_leave_ok C-new-only | done | `joint_leave_ok_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.9 | p1 | inbound election_grant_from skip stranger | done | `election_grant_from_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.10 | p1 | inbound joint_target ids not nodes | done | `joint_target_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.11 | p1 | inbound joint_add_target never-local | done | `joint_add_target_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.12 | p1 | inbound queued_leave_finish uncommitted | done | `queued_leave_finish_ok_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.13 | p1 | inbound disk_membership overrides CLI | done | `disk_membership_overrides_cli_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.14 | p1 | inbound high_water disk floor | done | `high_water_at_least_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.15 | p1 | inbound participating_if_member skip stale | done | `participating_if_member_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.16 | p1 | inbound identity before applied | done | `membership_identity_before_applied_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.17 | p1 | inbound recover_must_apply committed gap | done | `recover_must_apply_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.18 | p1 | inbound recover_apply_node local non-member | done | `recover_apply_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.19 | p1 | inbound recover_truncate local non-member | done | `recover_truncate_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.20 | p1 | inbound recover_drop_orphan log_entry_key | done | `recover_drop_orphan_seg_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.21 | p1 | inbound recover_abort leftover intent | done | `recover_abort_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.22 | p1 | inbound persist_meta now_ms local non-member | done | `persist_meta_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.23 | p1 | inbound persist_hist SI hist local non-member | done | `persist_hist_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.24 | p1 | inbound persist_fence abort local non-member | done | `persist_fence_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.25 | p1 | inbound force_clear stuck intent local non-member | done | `force_clear_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.26 | p1 | inbound drop_preimages leftover pre local non-member | done | `drop_preimages_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.27 | p1 | inbound open_peer_disk timeout from C-new | done | `open_peer_uses_disk_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.28 | p1 | inbound local_id_member TCP removed None | done | `local_id_if_member_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.29 | p1 | inbound reader_local skip remote ids.first | done | `reader_id_local_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.30 | p1 | inbound discard_uncommitted local non-member | done | `discard_node_counts_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.31 | p1 | inbound discard_leader local persist-leader | done | `discard_leader_local_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.32 | p1 | inbound removed_step_down planted Leader | done | `removed_steps_down_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.33 | p1 | inbound hint_member omits removed replica | done | `hint_if_member_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.34 | p1 | inbound drop_repl_slot forgets removed slots | done | `drop_repl_slot_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.35 | p1 | inbound drop_sent_through oob remove | done | `drop_sent_through_on_live_queued_is_not_ok` | 2026-08-29 |
| P1.36 | p1 | inbound apply_step hole stops | done | `apply_advance_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.1 | p2 | three-teeth on rpc_mode | done | `allow_direct_rpc_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.1 | p2 | three-teeth on dcs_apply CasFailed | done | `dcs_apply_should_advance_result_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.2 | p2 | three-teeth on compact missing term | done | `may_compact_through_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.3 | p2 | three-teeth on snapshot skip reserved | done | `snapshot_touches_user_key_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.4 | p2 | three-teeth on si_reader skip ids[0] | done | `si_reader_beats_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.5 | p2 | three-teeth on index_val drop NUL sibling | done | `len_pref_value_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.6 | p2 | three-teeth on changelog SST rebuild | done | `changelog_needs_sst_rebuild_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.7 | p2 | three-teeth on range_covers interior | done | `range_tombstone_covers_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.8 | p2 | three-teeth on prefix exclusive end | done | `prefix_exclusive_end_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.9 | p2 | three-teeth on stream_cursor hole ack | done | `ack_in_order_on_live_stream_is_not_ok` | 2026-08-29 |
| P2.2.10 | p2 | three-teeth on bearer BEARER scheme | done | `bearer_token_from_value_on_live_http_is_not_ok` | 2026-08-29 |
| P2.2.11 | p2 | three-teeth on content_length keep body | done | `keep_body_without_cl_on_live_http_is_not_ok` | 2026-08-29 |
| P2.2.12 | p2 | three-teeth on fail_closed writes 400 | done | `parse_error_writes_status_on_live_http_is_not_ok` | 2026-08-29 |
| P2.2.13 | p2 | three-teeth on form_plus + is space | done | `form_decode_on_live_http_is_not_ok` | 2026-08-29 |
| P2.2.14 | p2 | three-teeth on origin_path absolute-form | done | `origin_form_path_on_live_http_is_not_ok` | 2026-08-29 |
| P2.2.15 | p2 | three-teeth on isolated sibling id | done | `isolated_id_matches_on_live_fold_is_not_ok` | 2026-08-29 |
| P2.2.16 | p2 | three-teeth on children packed end | done | `packed_children_end_on_live_subspace_is_not_ok` | 2026-08-29 |
| P2.2.17 | p2 | three-teeth on fields NUL zip | done | `encode_fields_on_live_directory_is_not_ok` | 2026-08-29 |
| P2.2.18 | p2 | three-teeth on journal_pin after apply | done | `may_advance_pin_on_live_journal_is_not_ok` | 2026-08-29 |
| P2.2.19 | p2 | three-teeth on pack cut tag | done | `pack_cut_tag_on_live_subspace_is_not_ok` | 2026-08-29 |
| P2.2.20 | p2 | three-teeth on ship_guard rotate-regrow | done | `pull_plan_on_live_ship_is_not_ok` | 2026-08-29 |
| P2.2.21 | p2 | three-teeth on bloom_header hostile k | done | `bloom_header_ok_on_live_decode_is_not_ok` | 2026-08-29 |
| P2.2.22 | p2 | three-teeth on bloom_insert no false neg | done | `insert_on_live_sst_is_not_ok` | 2026-08-29 |
| P2.2.23 | p2 | three-teeth on bloom_may_contain no FN | done | `may_contain_on_live_sst_is_not_ok` | 2026-08-29 |
| P2.2.24 | p2 | three-teeth on scan_guard spanning tomb | done | `scan_reads_file_on_live_sst_is_not_ok` | 2026-08-29 |
| P2.2.25 | p2 | three-teeth on si_read below-floor TooOld | done | `snapshot_read_plan_on_live_queued_is_not_ok` | 2026-08-29 |
| P2.2.26 | p2 | three-teeth on fold_range covered last-per-key | done | `fold_event_hides_key_on_live_fold_is_not_ok` | 2026-08-29 |
| P2.2.27 | p2 | three-teeth on group_commit first-committer-wins | done | `occ_conflict_on_live_group_is_not_ok` | 2026-08-29 |
| P2.2.28 | p2 | three-teeth on group_fence max member seq | done | `fence_publish_seq_on_live_group_is_not_ok` | 2026-08-29 |
| P2.2.29 | p2 | three-teeth on group_publish failed WAL no publish | done | `may_publish_group_on_live_group_is_not_ok` | 2026-08-29 |
| P2.2.30 | p2 | three-teeth on forall_schedules refuse ∀π | done | `forall_schedules_admitted_on_live_group_is_not_ok` | 2026-08-29 |
| P2.2.31 | p2 | three-teeth on l28_durability kill+restart | done | `l28_durability_ok_on_live_store_is_not_ok` | 2026-08-29 |
| P2.2.32 | p2 | three-teeth on liveness_claim refuse without ES | done | `liveness_admitted_on_live_store_is_not_ok` | 2026-08-29 |
| P2.2.33 | p2 | three-teeth on fsync_promote lying does not promote | done | `fsync_promotes_pending_on_live_sim_is_not_ok` | 2026-08-29 |
| P2.2.34 | p2 | three-teeth on media_durable refuse fsync-as-proof | done | `media_durable_admitted_on_live_db_is_not_ok` | 2026-08-29 |
| P2.2.35 | p2 | three-teeth on tcg_guest native World not TCG | done | `tcg_guest_admitted_on_live_world_is_not_ok` | 2026-08-29 |
| P2.2.36 | p2 | three-teeth on fdatasync_rc nonzero rc is not Ok | done | `fdatasync_rc_ok_on_live_posix_is_not_ok` | 2026-08-29 |
| P2.2.37 | p2 | three-teeth on cqe_res negative CQE is not Ok | done | `cqe_res_ok_on_live_uring_is_not_ok` | 2026-08-29 |
| P2.2.38 | p2 | three-teeth on c_len oversize is LIMIT | done | `c_len_admitted_on_live_capi_is_not_ok` | 2026-08-29 |
| P2.2.39 | p2 | three-teeth on crc_match WAL CRC is Crc | done | `crc_match_ok_on_live_wal_is_not_ok` | 2026-08-29 |
| P2.2.40 | p2 | three-teeth on sst_crc modern trailer Reject | done | `sst_crc_fate_on_live_sst_is_not_ok` | 2026-08-29 |
| P2.2.41 | p2 | three-teeth on ikey_pack tombstone vs value | done | `pack_sequence_and_type_on_live_db_is_not_ok` | 2026-08-29 |
| P2.2 | p2 | remaining non-data_fate three-teeth | done | 0 pairs | 2026-08-29 |

## Acceptance Criteria

- **Tests**
  - `python3 scripts/formal/pedra_formal.py --lint` green; drop `live_callers` or the store `vote_decision()` / `ae_entry_action()` / `grant_after_persist()` / `ae_ack_success()` call fails naming `vote` / `ae_entry` / `grant_persist` / `ae_ack`. Drop `joint_election` `live_callers` fails naming that pair.
  - `vote_decision_on_live_queued_is_not_ok`: after `pin_dst_queued` + elect, `handle_inbound` a RequestVote with stale log / other candidate; reply `vote_granted: false`; `vote_decision` Deny; AS-IS WouldGrant.
  - `ae_entry_action_on_live_queued_is_not_ok`: inbound AppendEntries that would rewrite an index `<= commit` is not success / log not truncated; `ae_entry_action` Refuse; AS-IS TruncateAndInstall.
  - `grant_after_persist_on_live_queued_is_not_ok`: inbound RequestVote that WouldGrant, `FailingEnv` persist Err; reply `vote_granted: false`; `grant_after_persist(WouldGrant, Err)` is false; AS-IS true.
  - `ae_ack_success_on_live_queued_is_not_ok`: inbound AppendEntries that dirties the log, `FailingEnv` persist Err; reply `success: false`; log rolled back; `ae_ack_success(true, false)` is false; AS-IS true.
  - `allow_direct_rpc_on_live_queued_is_not_ok`: after `pin_dst_queued`, `set_rpc_mode(Direct)` stays Queued; `allow_direct_rpc(true, true)` is false; AS-IS true.
  - `dcs_apply_should_advance_result_on_live_queued_is_not_ok`: inbound DCS Create then duplicate Create + Put; `CasFailed` still advances `applied` to the Put; AS-IS would freeze the cursor. Direct `dcs_apply_cas_failed_does_not_stick_pipeline` is **not** this tooth.
  - `may_compact_through_on_live_queued_is_not_ok`: inbound AE heartbeat after dropping the log entry at `min(applied)`; `snapshot_index` stays; AS-IS would compact the missing index. `compact_through_unleft` is **not** this tooth.
  - `snapshot_touches_user_key_on_live_queued_is_not_ok`: inbound `InstallSnapshot` with a reserved `\\0store/raft/` key and a user key; user applies, reserved does not; AS-IS would put the leak. F40 txn-meta clear is **not** this tooth.
  - `si_reader_beats_on_live_queued_is_not_ok`: inbound Put on the leader; lagging `ids[0]` is not participating; `get` still returns the value; AS-IS would stick to first id. `point_get_prefer_applied` is **not** this tooth.
  - `len_pref_value_on_live_queued_is_not_ok`: inbound Puts of reverse-index keys for `red` and `red\\0foo`; `table_index_value_range(red)` lists pk 1 not pk 2; AS-IS raw `[val||0x00, val||0x01)` would include the sibling. Direct `table_index_range_does_not_include_nul_value_prefix_sibling` is **not** this tooth.
  - `changelog_needs_sst_rebuild_on_live_queued_is_not_ok`: inbound Put on the leader; flush; drop `CHANGELOG`; crash-reopen rewrites the cache from SST with the Put; AS-IS would leave the file missing. Direct `changelog_missing_post_flush_rebuilds_feed_from_sst` is **not** this tooth.
  - `range_tombstone_covers_on_live_queued_is_not_ok`: inbound Puts of `a`/`b`/`c`; live `delete_range([a,c))`; `get(b)` is gone, `get(c)` stays; AS-IS would only hide the start. `visible_at_on_live_range_del_is_not_ok` / `f30_as_is_misses_mid_range_key` are **not** this tooth.
  - `prefix_exclusive_end_on_live_queued_is_not_ok`: inbound Puts of `/host/h1/`, `/host/h1/\\xffz`, and the exclusive-end sibling; `scan_prefix` lists the 0xff continuation and not the sibling; AS-IS `prefix||0xff` would drop it. Direct `as_is_drops_ff_suffix` is **not** this tooth.
  - `ack_in_order_on_live_stream_is_not_ok`: live `Stream::ack` of seq 2 with cursor 0 is Err; cursor stays 0 and peek is seq 1; AS-IS would pin 2. Direct `ack_only_next` is **not** this tooth.
  - `bearer_token_from_value_on_live_http_is_not_ok`: live `authorization_matches` accepts `Authorization: BEARER sekrit`; AS-IS treats the whole header as the token. Direct `kv_http_bearer_scheme_case_insensitive` is **not** this tooth.
  - `keep_body_without_cl_on_live_http_is_not_ok`: live PUT without Content-Length stores `hello`; AS-IS truncate(0) would drop it. Direct `kv_http_put_without_content_length_keeps_body` is **not** this tooth.
  - `parse_error_writes_status_on_live_http_is_not_ok`: live PUT with Transfer-Encoding gets HTTP 400; AS-IS would close mute. Direct TE tests are **not** this tooth.
  - `form_decode_on_live_http_is_not_ok`: live DCS `key=hello+world` is the same lock as `hello%20world`; AS-IS would keep `+`. Direct `dcs_http_query_plus_is_space` is **not** this tooth.
  - `origin_form_path_on_live_http_is_not_ok`: live PUT absolute-form routes to `/kv/`; AS-IS would 404. Direct `kv_http_absolute_form_request_target` is **not** this tooth.
  - `isolated_id_matches_on_live_fold_is_not_ok`: live `in_prefixes` after `push_isolated(/vm/vm-a)` excludes `/vm/vm-ab`; AS-IS starts_with would include it. Direct `as_is_leaks_sibling` is **not** this tooth.
  - `packed_children_end_on_live_subspace_is_not_ok`: live `Subspace::range_end` drops zip 900 from pack(90) children; AS-IS `||0xff` would include it. Direct `as_is_leaks_sibling_900` is **not** this tooth.
  - `encode_fields_on_live_directory_is_not_ok`: live `IndexedUsers::set_user` round-trips zip `[9,0,0]`; AS-IS `zip||0x00||name` would truncate. Direct `set_user_zip_with_nul_clears_old_index_on_change` is **not** this tooth.
  - `may_advance_pin_on_live_journal_is_not_ok`: live `JournalConsumer::pin_after_apply(0)` stays at 0; AS-IS would always advance. Direct `pin_only_after_applied` is **not** this tooth.
  - `pack_cut_tag_on_live_subspace_is_not_ok`: live `Subspace::pack([a\\0b, c])` ≠ `pack([a, b\\0c])`; AS-IS cut tag 0 would collide. Direct `pack_is_injective_when_components_contain_nul` is **not** this tooth.
  - `pull_plan_on_live_ship_is_not_ok`: live `WalShipper::pull` after flush+regrow is `WalRotated`; AS-IS length-only would ship. Direct `rotation_regrow_past_cursor_fails_closed` is **not** this tooth.
  - `bloom_header_ok_on_live_decode_is_not_ok`: live `BloomFilter::decode` rejects `k=u32::MAX`; AS-IS would accept. Direct `decode_rejects_hostile_probe_count` is **not** this tooth.
  - `insert_on_live_sst_is_not_ok`: live flush then `get` of a Put is Some; AS-IS skip-insert would bloom-miss the SST. Direct `no_false_negatives` is **not** this tooth.
  - `may_contain_on_live_sst_is_not_ok`: live SST `get` of an inserted key is Some; AS-IS extra probe can false-negative. Direct `no_false_negatives` is **not** this tooth.
  - `scan_reads_file_on_live_sst_is_not_ok`: two SSTs — points-only `k-e` then spanning `delete_range([k-b,k-f))`; scan `[k-e,k-g]` hides `k-e`; AS-IS bounds-only skip would leak it. Direct `as_is_misses_spanning_tombstone` is **not** this tooth.
  - `snapshot_read_plan_on_live_queued_is_not_ok`: inbound Put; `force_safe_watermark(7)`; `get_at_version(key, 1)` is `TransactionTooOld`; AS-IS would Serve and fabricate absence. Direct `snapshot_plan_fails_closed_below_floor` / `snapshot_tx_too_old` are **not** this tooth.
  - `fold_event_hides_key_on_live_fold_is_not_ok`: live `last_per_key` after Put k-b/k-c and `delete_range([k-b,k-d))` drops covered k-c; AS-IS would keep k-c (start-only hide). Direct `as_is_only_hides_start` is **not** this tooth.
  - `occ_conflict_on_live_group_is_not_ok`: live ConcurrentDb overlapping OCC txs on `k`; first commit Ok, lagging commit is `TransactionConflict` and `get(k)` is the first committer; AS-IS serialized would abort the second intra-group member (`occ_conflict_as_is_serialized(10,10,1,true)`). Direct `group_members_are_simultaneous` / `occ_write_write_conflict_on_same_key` are **not** this tooth.
  - `fence_publish_seq_on_live_group_is_not_ok`: live ConcurrentDb barrier of 8 puts takes `max_appended_seq`; `visible_sequence == last_sequence` and every member `get`s; AS-IS first-member fence would under-publish. Direct `fence_is_max_member_seq` is **not** this tooth.
  - `may_publish_group_on_live_group_is_not_ok`: live ConcurrentDb `finish_group_off_lock` with injected WAL sync fail; every member Err and `get` is None; AS-IS would publish. Direct `publish_only_when_wal_io_ok` / `failed_wal_sync_does_not_publish_group` / `multi_writer_failed_sync_does_not_publish_group` are **not** this tooth.
  - `forall_schedules_admitted_on_live_group_is_not_ok`: live ConcurrentDb put then `claim_forall_schedules(2)` is false; AS-IS would admit at d≥2. Direct `pct_depth_is_not_forall_schedules` / `claim_forall_schedules_refused_at_pct_depth2` are **not** this tooth. This freeze refuses ∀π; it does not prove it.
  - `l28_durability_ok_on_live_store_is_not_ok`: live Queued inbound Put; crash-reopen follower; get + after-kill + restart all hold; AS-IS would pass on first get only. Direct `get_only_is_not_l28_clean` / `l28_durability_ok_requires_after_kill_and_restart` are **not** this tooth. L28 REAL TCP ∀ traces stay a campaign.
  - `liveness_admitted_on_live_store_is_not_ok`: live StoreCluster bounded `elect_all` then `claim_eventual_election(false,false,false)` is false; AS-IS would admit. Direct `liveness_claim_needs_all_three_es_axioms` / `claim_eventual_election_refused_without_es_axioms` are **not** this tooth. This freeze refuses eventual-live without ES axioms; it does not prove liveness.
  - `fsync_promotes_pending_on_live_sim_is_not_ok`: live `RecordingEnv::lying` Db put then crash; `get(k)` is None; AS-IS would promote pending on a lying fsync. Direct `fsync_ok_is_not_media_proof` / `lying_fsync_does_not_promote_pending` / `lying_sync_loses_write_after_crash` are **not** this tooth.
  - `media_durable_admitted_on_live_db_is_not_ok`: live Db put then `claim_media_durable()` is false; AS-IS would admit after fsync Ok. Direct `fsync_ok_is_not_media_proof` / `claim_media_durable_refused_after_fsync_ok` are **not** this tooth. This freeze refuses a media theorem; it does not prove the drive.
  - `tcg_guest_admitted_on_live_world_is_not_ok`: live `World::run` then `claim_tcg_guest()` is false; AS-IS would admit native World as TCG. Direct `claim_tcg_guest_refused_on_native_world` is **not** this tooth. This freeze refuses TCG coverage; it does not invent a guest.
  - `fdatasync_rc_ok_on_live_posix_is_not_ok`: live `fdatasync_file` on a WAL file is Ok; live pipe fd is Err (nonzero rc); AS-IS would ignore nonzero rc. Direct `fdatasync_nonzero_rc_is_not_ok` / `fdatasync_rc_ok_is_safe_predicate` are **not** this tooth. This freeze refuses skipping the barrier; it does not prove the drive.
  - `cqe_res_ok_on_live_uring_is_not_ok`: live `IoUringEnv` write+`sync_data`; negative CQE `res` is not Ok; AS-IS would Ok a failed fsync. Linux inject `-EIO` is Err. Direct `cqe_negative_res_is_not_ok` / `linux_cqe_eio_is_not_ok` are **not** this tooth. Production G1 stays POSIX.
  - `c_len_admitted_on_live_capi_is_not_ok`: live C ABI create+tx; oversize value_len on set and key_len on get are `MONTAHA_FDB_LIMIT`; AS-IS would copy any len. Direct `c_len_oversize_on_live_tx_is_limit` / `slice_cap_oversize_len_is_limit_without_reading` are **not** this tooth.
  - `crc_match_ok_on_live_wal_is_not_ok`: live `WalWriter` then XOR of the stored CRC field (payload intact); `WalReader::read_record` is `Crc`; AS-IS would accept. Direct `crc_mismatch_on_live_wal_is_not_ok` / `crc_mismatch_on_live_wal_open_is_not_ok` / `crc_mismatch_is_not_ok` are **not** this tooth. Equality of two u32s is not R-crc.
  - `sst_crc_fate_on_live_sst_is_not_ok`: live `write_sst` then XOR of the file trailer CRC (payload intact); `SstTable::open` is CRC mismatch; AS-IS would StripTrailer. Direct `crc_mismatch_on_live_sst_is_not_ok` / `crc_mismatch_on_live_sst_block_is_not_ok` / `crc_mismatch_on_live_sst_db_open_is_not_ok` are **not** this tooth.
  - `pack_sequence_and_type_on_live_db_is_not_ok`: live `InternalKey::encode` keeps seq=1 Deletion distinct from seq=0 Value; live Db put then delete hides `k`; AS-IS `seq|kind` without the shift collides. Direct `pack_unpack_round_trip` / `encode_decode_round_trip` are **not** this tooth.
  - `propose_ack_ok_on_live_queued_is_not_ok`: Queued `put` without pumping AE replies is `NotCommitted`; `propose_ack_ok(index, commit)` is false; AS-IS true; inbound AE without replies does not advance leader commit.
  - `joint_election_ok_on_live_queued_is_not_ok`: during planted C-old=3/C-new=4, inbound RV grants from 2 old voters do not elect; `joint_election_ok(2, 3, Some((2, 4)))` is false; AS-IS true.
  - `joint_still_active_on_live_queued_is_not_ok`: inbound AE MembershipJoint C-old≠C-new; `pending_joint_on` sees it; `joint_still_active` true; AS-IS false.
  - `pending_joint_node_counts_on_live_queued_is_not_ok`: inbound AE joint on node 4, Queued shrink drops 4 from `ids`; leftover C-old,new on 4; `pending_joint()` is None; AS-IS would scan the removed node.
  - `joint_leave_ok_on_live_queued_is_not_ok`: inbound AE MembershipJoint `old == new` (C-new-only leave) is in the follower log; `joint_leave_ok(true)`; AS-IS true even without leave.
  - `election_grant_from_counts_on_live_queued_is_not_ok`: inbound `RequestVoteReply` grant from voter 999 (not in `ids` / joint) is not recorded; AS-IS would count it.
  - `joint_target_counts_on_live_queued_is_not_ok`: TCP replica (`open_single_node`) inbound RV from peer 3 not in local `nodes`; `remove_member_joint(3)` is not `unknown node`; AS-IS would require the peer in `nodes`.
  - `joint_add_target_counts_on_live_queued_is_not_ok`: TCP replica inbound RV from joiner 4 not in local `nodes`; `add_member_joint(4)` is not `unknown node`; AS-IS would require the joiner in `nodes`.
  - `queued_leave_finish_ok_on_live_queued_is_not_ok`: inbound AE MembershipJoint `old == new` with `leader_commit=0` sits uncommitted; `queued_leave_finish_ok(true, false)` false; AS-IS true; `finish_uncommitted_leave` is not done.
  - `disk_membership_overrides_cli_on_live_queued_is_not_ok`: inbound AE C-new-only leave with `leader_commit` covering the index persists disk without 4; stale RAM `ids` include 4; `bind_cluster_identity` restores disk; AS-IS would keep CLI.
  - `high_water_at_least_on_live_queued_is_not_ok`: inbound AE C-new-only leave persists disk high-water 4; TCP ctor CLI `[1,2,3]` keeps 4; `remove_member(1)` is quorum floor; AS-IS would use RAM 3.
  - `participating_if_member_on_live_queued_is_not_ok`: inbound AE C-new-only leave drops 4 from `ids`; stale `participating=true`; `is_participating(4)` false; inbound RV `vote_granted: false`; AS-IS would count the flag.
  - `membership_identity_before_applied_on_live_queued_is_not_ok`: inbound AE C-new-only leave with `leader_commit` covering the index; disk C-new omits 4 and `applied >= leave`; AS-IS would persist applied first.
  - `recover_must_apply_on_live_queued_is_not_ok`: inbound AE MembershipJoint with `leader_commit=0` then durable commit ahead of applied; crash-reopen applies (`!is_member(4)`); AS-IS would skip recover apply.
  - `recover_apply_node_counts_on_live_queued_is_not_ok`: inbound Put on 4 then inbound leave drops 4 from `ids`; crash-reopen applies the put on the removed replica; AS-IS would skip local non-member.
  - `recover_truncate_node_counts_on_live_queued_is_not_ok`: inbound uncommitted Put on 4 then inbound leave drops 4 from `ids`; crash-reopen persists truncated log (no `index > commit`); AS-IS would skip persist.
  - `recover_drop_orphan_seg_on_live_queued_is_not_ok`: inbound uncommitted Put writes `log_entry_key`; inbound leave drops 4; crash-reopen deletes the orphan key; AS-IS would leave it.
  - `recover_abort_node_counts_on_live_queued_is_not_ok`: inbound `TxnPrepare` installs leftover intent on 4; inbound leave drops 4; crash-reopen deletes `intent_key`; AS-IS would skip abort.
  - `persist_meta_node_counts_on_live_queued_is_not_ok`: inbound leave drops 4 from `ids`; `advance_now_ms` persists `now_ms` on node 4; AS-IS would skip SI meta persist.
  - `persist_hist_node_counts_on_live_queued_is_not_ok`: inbound leave drops 4 from `ids`; planted RAM hist + `persist_si_keys` writes `hist_key` on node 4; AS-IS would skip SI hist persist.
  - `persist_fence_node_counts_on_live_queued_is_not_ok`: inbound leave drops 4 from `ids`; `fence_txn_aborted` writes `txn_status_key` `abort` on node 4; AS-IS would skip abort fence.
  - `force_clear_node_counts_on_live_queued_is_not_ok`: inbound leave drops 4 from `ids`; planted `intent_key` + `force_local_clear_keys` drops it on node 4; AS-IS would skip force-local clear.
  - `drop_preimages_node_counts_on_live_queued_is_not_ok`: inbound leave drops 4 from `ids`; planted `txn_pre_key` + `drop_preimages` drops it on node 4; AS-IS would skip drop.
  - `open_peer_uses_disk_on_live_queued_is_not_ok`: inbound leave persists C-new; process `open(n=4)` node 4 `election_timeout` matches disk `[1,2,3]` not CLI `[1,2,3,4]`; AS-IS would ignore disk at load.
  - `local_id_if_member_on_live_queued_is_not_ok`: inbound leave; TCP ctor of 4 `local_node_id()==None`; planted key `get` is Err; AS-IS would claim HashMap first-key.
  - `reader_id_local_on_live_queued_is_not_ok`: inbound leave; TCP ctor of 4 `best_reader_for_key` is None; `get` Err contains `empty` not `bad node`; AS-IS would pick remote `ids.first()`.
  - `discard_node_counts_on_live_queued_is_not_ok`: inbound uncommitted Put on 4 then inbound leave drops 4; `discard_uncommitted_from` drops RAM and disk suffix; AS-IS would skip live discard.
  - `discard_leader_local_on_live_queued_is_not_ok`: inbound leave; TCP ctor of 4; planted suffix; `finish_queued_propose` abort; `next_index` on 4 for member 1 is `from`; AS-IS remote `ids.first()` skips repair.
  - `removed_steps_down_on_live_queued_is_not_ok`: inbound leave; planted `Role::Leader` on 4; apply steps down (`!node_thinks_leader(4,1)`); AS-IS would keep Leader.
  - `hint_if_member_on_live_queued_is_not_ok`: inbound leave; no live `range_leader`; planted `leader_id=4` on remaining voters; `leader_hint(1) != Some(4)`; AS-IS would return the removed replica. 0145 step-down is **not** this tooth.
  - `drop_repl_slot_on_live_queued_is_not_ok`: inbound leave; planted `next_index`/`match_index`/`sent_through` of 4 on remaining-voter follower; apply drops those keys and keeps a remaining-member slot; AS-IS would keep the removed slots. 0146 hint filter is **not** this tooth.
  - `drop_sent_through_on_live_queued_is_not_ok`: inbound AE heartbeat; planted `sent_through` of 3 on node 1; oob `remove_member(3)`; key 3 gone, remaining-member key stays; AS-IS would keep it. 0147 joint `drop_repl_slot` is **not** this tooth.
  - `apply_advance_on_live_queued_is_not_ok`: inbound AE heartbeat with `leader_commit` covering a planted hole; `applied` stays; later Put is not visible; `apply_advance` Stop; AS-IS Apply. Production store `apply_range` calls the catalog kernel (clone freeze).
  - `glue.db_rs_extracted` stays false.
- **Telemetry / Analytics:** none — safety freeze.
- **Documentation:** this RFC; catalog `live_callers` (`vote` / `ae_entry` / `grant_persist` / `ae_ack` + raft→store clones); `rpc_mode` `three_teeth`; clones `vote_raft_store` / `ae_ack_raft_store`.
- **Screenshots:** backend-only.

## Out of scope

- Extending three-teeth beyond the closed P2.2 non-`data_fate` set (L28 TCP remains H).
- Making L28 real TCP a theorem; extracting `db.rs`; new World swarm seeds; CRC-collision / Linux `fsync` / io_uring ring / `∀` ConcurrentDb proofs.
- More membership-locality RFCs, Lean/Aeneas second machine, crates.io, benches / RFC-0149.
- Changing the `ae_ack_success` predicate (already production-called; P1.2 plants the inbound persist-Err path).
- Production `pedradb-raft` dependency.
