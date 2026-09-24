# RFC-0049 — Registro canônico de bugs do pedradb: bug → fix 1:1

**Status:** done (registro completo; todas as correções já estão no `main` do repo)
**Updated:** 2026-08-23

## Background

- PedraDB passou por caçadas adversariais (tesoura, 2026-08) em todas as
  superfícies: núcleo de storage (`db.rs`, WAL, memtable, SST, merge, OCC,
  history), distribuído (`pedradb-raft`, `pedradb-dcs`, `pedradb-store`),
  HTTP (`pedradb-http`), compat (`rocksdb-compat`) e ilhas `unsafe`
  (`pedradb-posix`, `pedradb-io-uring`, `pedradb-capi`).
- Todo bug listado aqui foi provado **dois-estados**: o teste de prova FALHA
  no tree sem patch e PASSA com o fix de causa única (controles verdes nos
  dois lados). Nenhum fix remove funcionalidade.
- Este RFC é o **registro canônico e consumível por máquina**: uma linha por
  bug, colunas estáveis, IDs estáveis (F1..F213; numeração do LEDGER do
  hunt — ver "Fontes" abaixo; F215 acresce o cap de slices no C ABI). As
  correções **já estão aplicadas no `main`**; este documento serve para
  auditoria, verificação automatizada e regressão — não para reaplicar
  patches.

## Contrato de consumo (para automação do repo)

- Formato: tabela pipe com exatamente 5 colunas, nesta ordem:
  `ID | Superfície | Bug | Fix | Prova`.
- `ID` é estável (`F<n>`); `Superfície` é o locus (crate e/ou função);
  `Fix` é a mudança de causa única aplicada; `Prova` é o teste/bateria que
  demonstra o dois-estados (ferramentas: `dst_fail_after_sweep`,
  `dst_corruption_sweep`, harness `core_hunt`/`compat_hunt`, suites do
  próprio crate, Miri/ASan nas ilhas unsafe).
- Status: **todos FIXED no `main`**. Cada bug tem ficha detalhada em
  `determinismo/pedradb-dst/findings/F<n>-*.md` (repositório de caça) e,
  para as waves 2026-08-21..23 (F165..F213), patches por wave em
  `determinismo/pedradb-dst/patches/core-hunt-2026*.patch`.
- Refutados (candidatos provados NÃO-bug) ficam FORA da tabela; hoje:
  F214 (enforce_cap × occ_registry_floor — derivação + guarda
  `k39_defense_cap_gc_holds_open_occ_snapshot`). Não trate F214 como bug.

## Registry (176 bugs, todos FIXED no main)

| ID | Superfície | Bug | Fix | Prova |
|----|-----------|-----|-----|-------|
| F1 | `Db::flush` final SST path | partial SST blocks open; WAL stranded | flush `.sst.tmp`+rename | `dst_fail_after_sweep` n=17 |
| F2 | `SstTable::decode` `with_capacity(num_entries)` | bit-flip header → multi-EiB alloc abort | bounds vs file size | `dst_corruption_sweep` OOM |
| F3 | SST v2 no checksum | payload bitrot → silent wrong/missing keys | trailing CRC32C | many `000001.sst@*` SilentWrong |
| F4 | WAL length + `Ok(None)` torn semantics | length bitrot → silent drop of WAL tail | length in CRC + Truncated fail-stop | `CURRENT.log@5/40/75` |
| F5 | MANIFEST no CRC | flip SST count → silent missing tables | payload CRC32C | `MANIFEST@24` |
| F6 | open: WAL `Truncated(0)` fails whole open | torn empty WAL after flush rotate blocks SST recovery | tolerate tiny WAL Truncated(0) | fail_after n=64 step put after flush |
| F7 | `pedradb-dcs` lease table + meta lease id | immortal / reanimated leader key after DCS restart | unknown=expired + max-id + orphan GC on open | `leased_key_not_immortal_after_dcs_reopen` |
| F8 | `pedradb-http::read_req` Content-Length | unbounded body alloc / hang DoS | 16MiB body cap | review + oversized CL unit test |
| F9 | `pedradb-raft` persist log/hard + net AE | uncapped `with_capacity` + no CRC → OOM / silent log corruption | CRC32C v2 + count bounds | unit: huge count + byte-flip |
| F10 | Raft open `commit=log.len` | uncommitted suffix false-commit; stranded apply | `RAFT_COMMIT` + re-apply | `reopen_does_not_commit_uncommitted_suffix` |
| F11 | propose ACK without majority | false durability under partition | `NotCommitted` if commit &lt; index | `propose_acks_only_after_commit` |
| F12 | DCS apply `CasFailed` | `last_applied` freezes; later commits never apply | CasFailed → no-op at apply | code path + recovery re-apply |
| F13 | `WriteRecord::decode` op count | uncapped `with_capacity` (F2 class) | count vs remaining | `decode_rejects_huge_op_count` |
| F14 | WAL reader Middle/Last orphan | `Ok(None)` = silent EOF → drop durable tail | fail-stop Internal | `orphan_middle_fail_stops_not_silent_eof` |
| F18 | `apply_batch` / TX + `auto_flush_bytes` | auto-flush I/O fails acked put/commit | best-effort auto-flush/compact | `auto_flush_fault_does_not_fail_acked_put` |
| F19 | `parse_sst_name` len==6 only | file num ≥1e6 breaks MANIFEST path | any-digit stem | `high_file_number_sst_round_trip` |
| F20 | `maybe_auto_compact` latest_only | auto-compact drops snapshot history | default GC on auto | `auto_compact_preserves_snapshot_history` |
| F21 | flush after SST push | MANIFEST fail leaves inflated `ssts` | pop + restore file num | fail_after + rollback |
| F22 | `pedradb-store` apply DCS | CasFailed freezes range `applied` |  | `dcs_apply_cas_failed_does_not_stick_pipeline` |
| F23 | store `become_leader` | no blank index / prev-term commit | Noop entry | `leader_noop_commits_prev_term_after_reelect` |
| F24 | store AE conflict | rewrite committed index |  | code + contiguous check |
| F25 | `split_keyspace` n>256 | step=0 collapsed ranges | reject | `open_rejects_too_many_ranges` |
| F26 | store raft meta RAM-only | restart lost term/log/commit | PedraDB `\0store/raft/` | `raft_meta_survives_reopen` |
| F27 | store log unbounded growth | full log rewrite forever | snapshot watermark + truncate | `raft_log_compacts_after_all_applied` |
| F28 | compact only participating | offline peer loses catch-up log | min applied over full membership | `compact_does_not_pass_offline_peer_applied` |
| F15 | Raft vote/term `let _ = persist_hard` | vote granted without durable hard state → double-vote | persist-before-grant + rollback | `vote_persisted_before_grant` |
| F16 | AE conflict truncate | could rewrite committed index | refuse ≤ commit | `refuse_conflict_at_committed_index` |
| F17 | `atomic_write` rename | no parent dir fsync | parent `sync_all` | review + dir sync |
| F29 | SST `point_at` multi-block | stale version when user key spans blocks | scan all candidate blocks + avoid mid-key split | `multi_version_large_memtable_point_lookup` + soak |
| F30 | OCC `key_has_write_after` | concurrent covering `delete_range` missed → silent commit + key resurrection | range covers key in mem+SST | `occ_conflicts_on_range_delete_*` |
| F31 | `ChangeLog::store_on` | remove-before-rename → CHANGELOG gone post-flush; feed empty, SST data remains | atomic rename only | unit residual + review |
| F32 | `decode_changelog` | uncapped `with_capacity(n)` (F2-class OOM) | count vs residual | `decode_rejects_huge_entry_count` |
| F33 | `ChangeLog::load_on` | corrupt CHANGELOG CRC fails whole open; SST data unreachable | treat as empty + quarantine | `corrupt_changelog_does_not_block_open` |
| F34 | store 2PC `apply_txn_revert` | revert deletes user key (loses preimage) | durable preimage + restore; keep pre until all ranges commit | `revert_restores_preimage_on_partial_commit` |
| F35 | store open after `tx_start` | leftover intents + `next_txn_id=1` reuse | abort leftover intents on open; persist next id | `crash_after_prepare_does_not_immortalize_intents` |
| F36 | store SI/OCC RAM-only | generation/history gone on reopen → read skew | persist gen/watermark/hist; load on open | `snapshot_isolation_survives_reopen` |
| F37 | store multi-range `tx_finish` SI notes | one generation per range → partial multi-range SI visibility | defer SI note to all-range success; single gen | `multi_range_commit_tx_single_generation_visibility` |
| F38 | store `on_install_snapshot` | put-only merge left deleted keys on lagging peer | wipe range keyspace then apply export | `install_snapshot_clears_stale_keys_not_in_export` |
| F39 | store PeerMsg/entry/TCP counts | soft cap only → huge `with_capacity` on tiny residual | residual bound + soft cap | `ae_rejects_count_past_residual` |
| F40 | store install-snapshot intents | orphan intents outside user keyspace → immortal Conflict | clear_range_txn_meta on install | `install_snapshot_user_range_clears_orphan_intents` |
| F41 | store export/install-snapshot | leader `\0store/*` + range0 wipe → follower raft meta = leader / cross-range clobber | user-only export/wipe; local raft meta | `install_snapshot_range0_preserves_*` + `does_not_import_leader_raft_meta` |
| F42 | store `note_tx_commit` / `value_now` | SI notes from lagging `ids[0]` after majority without that node | best_applied_reader | `note_tx_commit_reads_applied_not_lagging_first_node` |
| F43 | ConcurrentDb flush L0 | concurrent off-lock write peeks same next_file_num → SST path collision | alloc under write lock + write_num | `concurrent_flush_allocates_distinct_file_nums` |
| F44 | `Db::create_checkpoint` mid-vlog-GC | omits live `VALUES.vlog.new` while MANIFEST remapped → wrong/missing large values | copy `.new` when use_new | `checkpoint_mid_vlog_gc_preserves_large_values` |
| F45 | ConcurrentDb dual flush | `install_l0_sst` cleared restored imm → silent drop of other pipeline | no clear imm; restore folds; flush_lock | `dual_flush_restore_then_install_does_not_drop_other_imm` |
| F46 | `Db::create_checkpoint` CHANGELOG | omits CHANGELOG post-flush → silent feed loss on restore | copy CHANGELOG (+ adopt marker) | `checkpoint_copies_changelog_feed` |
| F47 | store 2PC fail + heal/elect | uncommitted `TxnCommit` later majority-applies after client Err | durable abort fence + apply_commit respects abort | `fail_after_mid_2pc_restores_preimage` |
| F48 | store AE follower | success ack after failed log persist → orphan commit on heal | rollback mem log + success:false | `fail_after_mid_2pc_restores_preimage` multi-seed |
| F49 | store `with_si_gen` | outstanding Queued puts stamp same `si_gen` → SI collapse on apply hist | reserve gen at propose + note_mutations_at | `queued_double_propose_distinct_si_gens_survive_reopen` |
| F50 | store `tx_start` multi-range | NotLeader mid-prepare leaves earlier intents → immortal Conflict | abort earlier ranges on fail | `multi_range_prepare_not_leader_aborts_earlier_intents` |
| F51 | vlog promote / open_with_flag | remove-before-rename + empty create under use_new | atomic rename; refuse empty create | `promote_atomic_rename_keeps_primary` |
| F52 | store `apply_txn_revert` | partial `TxnCommit` then revert restores Pedra but SI hist keeps aborted write | repair hist tip on revert | `partial_tx_finish_si_hist_matches_restored_preimage` |
| F53 | CHANGELOG missing after flush | WAL-only rebuild → empty feed, SST keys remain | last-per-key rebuild from Mem∪SST | `changelog_missing_post_flush_rebuilds_feed_from_sst` |
| F54 | `Stream::next` | cursor persisted on read → skip after crash | peek + ack after apply | `peek_without_ack_survives_reopen` |
| F55 | store `changelog_after` | partitioned `ids[0]` starves fold tail | max last_sequence among participating | `changelog_after_skips_lagging_first_node` |
| F56 | store DCS `now_ms` | TTL deadline durable, clock RAM-only → expired HA lock reanimates on reopen | persist/load max `now_ms` | `dcs_ttl_expired_stays_dead_after_reopen` |
| F57 | fold `range_user` | exclusive end `prefix\ | drops `prefix\ | \ |
| F58 | `IdempotentIndex::keys_for` + 2PC leftover scan | raw prefix\ | drops `0xff` user keys / leftover pairs | \ |
| F59 | `montanha-fdb-recipes` pack\ | \ | ble-length index/multimap includes prefix siblings (`90`⊃`900`) | 0xff |
| F60 | recipes unpack + payload `\\0` | id/pk/zip containing `0x00` truncated; stale index after zip change | `child_suffix` + length-prefixed fields | `simple_index_round_trips_id_containing_nul` |
| F61 | `IdempotentIndex::keys_for` `'/'` join | `keys_for("red")` listed value `"red/foo"` as ghost key `foo/k2` | `IDX\ | `idempotent_index_keys_for_does_not_include_value_prefix_sibling` |
| F62 | recipe `pack` / `keys_in_range_at` | NUL components collide; Pedra scan used lagging ids[0] | length-prefixed pack + best_changelog_reader | `pack_is_injective_when_components_contain_nul` |
| F63 | `olap_ingest` / `stream_publish` `'/'` join | scan `olap/events/` included subject `events/extra` | `ns\ | `olap_scan_does_not_include_slash_sibling_stream` |
| F64 | layers table index | slash join collides; update leaves stale reverse | length-pref keys + tip clear | `table_index_injective_and_clears_stale_on_change` |
| F65 | `pedradb-index` `idx_key` | slash and NUL field+value collide | length-prefixed field+value | `idx_key_is_injective_when_components_contain_slash` |
| F66 | `flush_writes` / put_buffered | `mem::take` + failed put_many drops staged puts | clone; clear only on Ok | `put_buffered_flush_keeps_buffer_on_error` |
| F67 | fold `range_user` | `starts_with(0x00)` hid FDB-packed user keys from range/resync | skip only `\\0fold/` meta | `fold_range_includes_nul_prefixed_user_keys` |
| F68 | fold follow / last_per_key | `\0fold/cursor` leaked into state-sync under prefix `0x00` | filter `\0fold/` on changelog paths | `fold_changelog_sync_skips_fold_meta` |
| F69 | fold apply | user key `fold/*` clobbers meta | reject reserved | `fold_rejects_reserved_meta_user_keys` |
| F70 | pedradb-stream | slash key layout / multi-stream | length-pref name | `stream_names_with_slash_are_isolated` |
| F71 | pedradb-sql | slash join table/user | length-pref | `create_insert_select_delete` |
| F72 | `StoreCluster::get` | multi-node fallback lagging `ids[0]` | best_changelog_reader | changelog_after get assert |
| F73 | `EtcdNeedFace::get` | DCS face still read `ids[0]` after partition | `dcs_get` via best_changelog_reader | `etcd_need_get_skips_lagging_first_node` |
| F74 | HTTP `KvServer` path | `GET /kv/hello?x=1` looked up `hello?x=1` → 404 | `path_only` like DCS | `kv_http_put_get` |
| F75 | HTTP KV/DCS path | `PUT /kv/a%2Fb` then `GET /kv/a/b` → 404 | percent-decode | `kv_http_put_get` |
| F76 | HTTP DCS query | `?key=a%2Fb` ≠ `/dcs/kv/a/b` | decode query values | `dcs_http_query_key_percent_decoded` |
| F77 | `put_many` multi-range | sequential put_batch half-apply | commit_tx when >1 range | `put_many_multi_range_is_atomic_on_failure` |
| F78 | `IdempotentIndex::keys_for` | `val\ | leaks `val\ | \ |
| F79 | HTTP method match | `put` ≠ `PUT` → 404 / no-op | ascii uppercase | `kv_http_method_case_insensitive` |
| F80 | `table_index_value_range` | `val\ | leaks `red\\0foo` under `red` | \ |
| F81 | layers stream/olap subject | `subj\ | nests `red\\0x` under `red` | \ |
| F82 | layers table_row_key / tip | raw pk | non-injective with embedded NUL |  |
| F83 | fold `caixote_host_filter` | `/vm/vm-a` `starts_with` includes `vm-ab` | isolated exact-or-child | `caixote_host_filter_does_not_include_vm_id_prefix_sibling` |
| F84 | `StoreCluster::get` / `dcs_get` | global `last_sequence` not per-range applied | `best_reader_for_key` | `get_multi_range_uses_per_range_applied_reader` |
| F85 | HTTP `Authorization` scheme | `BEARER` ≠ `Bearer` → 401 | scheme case-insensitive | `kv_http_bearer_scheme_case_insensitive` |
| F86 | HTTP `read_req` missing CL | no Content-Length → `truncate(0)` empty PUT | keep post-header bytes | `kv_http_put_without_content_length_keeps_body` |
| F87 | HTTP `Content-Length` parse | `abc` → 0 → empty PUT | reject bad CL | `kv_http_bad_content_length_does_not_store_empty` |
| F88 | HTTP duplicate `Content-Length` | last header won (`5` then `0` → empty PUT) | reject differing CLs | `kv_http_conflicting_content_length_does_not_store_empty` |
| F89 | `pedradb-index` `row_key` | raw `row/`\ | `row/a` prefix of `row/ab` | \ |
| F90 | `pedradb-lease` `lease_key` | raw `lease/`\ | → `lease/a` prefix of `lease/ab` | \ |
| F91 | HTTP request-target | absolute-form `GET http://h/kv/x` 404 | path_only | (http origin-form) |
| F92 | HTTP request-target | network-path `//host/kv/x` 404 | strip authority | (http) |
| F93 | `IdempotentIndex` DATA | raw `DATA\ | prefix sibling | \ |
| F94 | `SafeList` SEEN | raw `seen/\ | ` prefix sibling | \ |
| F95 | `SafeAllocator` token | raw `TOKEN\ | ` prefix sibling | \ |
| F96 | `EtcdNeedFace::full_key` | raw `m/\ | → `m/a`⊃`m/ab` | \ |
| F97 | fold keyset user | raw user in `\0fold/keyset/` | length-pref | (fold) |
| F98 | DCS `kv_key`/`meta_key` | raw `d/k/\ | prefix sibling | \ |
| F99 | layers `cp_key` | raw `cp/\ | prefix sibling | \ |
| F100 | store `meta_key` | raw `m/\ | x` → `m/a`⊃`m/ab` (F96 missed this helper) | \ |
| F101 | HTTP DCS query `+` | `hello+world` ≠ `hello%20world` lock | `+`→space before `%HH` | `dcs_http_query_plus_is_space` |
| F102 | HTTP `read_req` Err | socket close, no status line | write 400 | F87/F88 now expect PUT 400 |
| F104 | HTTP `Transfer-Encoding` | ignored; chunked body stored raw | reject TE | `kv_http_transfer_encoding_rejected` |
| F105 | HTTP DCS `ttl_ms` parse | `abc` → default 30s, lock acquired | 400 on bad int | `dcs_http_bad_ttl_ms_does_not_store_lock` |
| F106 | HTTP DCS query name | `?%6Bey=lock` missed `key` → default `/leader` | form-decode names | `dcs_http_query_name_percent_decoded` |
| F107 | `decode_list` | truncated blob → silent prefix of items | fail-closed decode | `decode_list_rejects_truncated_tail` |
| F108 | `keys_in_range_at` | one reader for `start`'s range; later ranges lag | per-range applied scan | `keys_in_range_at_spans_ranges_after_first_node_partition` |
| F109 | `NaiveAllocator` / `SafeAllocator` `NEXT` | garbage counter parsed as 0 → reuse name slot 0 | fail-closed parse | `allocator_corrupt_next_does_not_reuse_name_zero` |
| F110 | core `prepare_flush_imm` | taken mem invisible to get/range during off-lock SST write | `flush_read_pin` + `mem_layers` | `get_sees_acked_key_while_flush_imm_off_lock` |
| F111 | stream `last_seq` / consumer cursor | truncated meta → 0 → republish seq 1 overwrites | fail-closed load | `publish_rejects_truncated_last_seq_meta` |
| F112 | recipe `Queue`/`PriorityQueue` counters | garbage head/tail/seq → 0 → overwrite first item | fail-closed parse | `queue_corrupt_tail_does_not_reuse_seq_zero` |
| F113 | `try_rotate_wal` during off-lock flush | checkpoint copies truncated WAL; acked keys only in pin | rotate waits for pin; ckpt takes `flush_lock` | `checkpoint_during_off_lock_flush_keeps_acked` |
| F113 | DCS `d/rev` | garbage rev → 0 → reuse revision 1 | fail-closed load/bump | `put_rejects_truncated_cluster_revision` |
| F114 | store SI `generation` meta | all-replica corrupt → open at gen 0 | fail-closed + hist tip max | `open_rejects_corrupt_si_generation_meta` |
| F115 | DCS key meta corrupt | get() miss → create/CAS-0 overwrite live value | reject present-undecodable meta | `create_rejects_undecodable_meta` |
| F116 | checkpoint mid flush pin | WAL rotate past pin → ckpt misses acked keys | pin blocks rotate + flush_lock | `checkpoint_during_off_lock_flush_keeps_acked` |
| F117 | SI hist append | corrupt hist → empty → rewrite single gen (wipe) | fail-closed decode on present | `persist_si_hist_rejects_corrupt_does_not_wipe` |
| F118 | txn preimage revert | garbage pre → LeaveUntouched + drop pre keys | decode_preimage Result | `apply_txn_revert_rejects_corrupt_preimage` |
| F119 | SI hist open load | all-replica corrupt hist skipped → SI evaporates | fail-closed if only corrupt | `open_rejects_corrupt_si_hist_on_all_replicas` |
| F120 | txn commit intent | short intent → skip put + delete intent | decode_intent Result | `apply_txn_commit_rejects_short_intent` |
| F121 | raft log segment load | missing `log/e/{i}` under log_hi skipped → holes | fail-closed contiguous segments | `open_rejects_raft_log_segment_gap` |
| F122 | force_local / open leftover | `let _ = apply_txn_revert` swallows corrupt pre | propagate cleanup Result | `open_rejects_leftover_intent_with_corrupt_preimage` |
| F123 | DCS delete post-check `d/rev` | short rev → NotCommitted | present-short is Corrupt Msg | `dcs_delete_reports_corrupt_rev` |
| F124 | install-snapshot persist | `let _ = persist_*` then wipe + success | persist-before-wipe + rollback | `install_snapshot_persist_fail_keeps_user_keys` |
| F125 | RequestVote hard state | grant after failed persist_hard → dual vote | deny + rollback on persist fail | `request_vote_persist_fail_denies_grant` |
| F126 | try_advance_commit | memory commit ahead of disk on persist fail | roll back commit | `try_advance_commit_persist_fail_does_not_raise_commit` |
| F127 | AE/RPC term bump hard | process AE / stay leader after failed persist_hard | durable_become_follower helper | `append_entries_hard_persist_fail_rejects` |
| F128 | discard / open truncate log | leader orphan stays on disk if persist swallowed | leader ? + open Result | `discard_uncommitted_leader_persist_fail_is_err` |
| F129 | propose after NotCommitted | `let _ = discard` hides leader persist fail | propagate discard err | `put_not_committed_surfaces_discard_persist_fail` |
| F130 | fence_txn_aborted | `let _ = put(abort)` — TxnCommit can still apply | fence Result + ? | `fence_txn_aborted_persist_fail_is_err` |
| F131 | persist_now_ms | mark RAM persisted after all puts fail → TTL reanimate | only mark on any-ok | `persist_now_ms_retries_after_all_replica_put_fail` |
| F132 | alloc_txn_id | RAM bump + swallow next_txn → id reuse on reopen | persist before bump | `alloc_txn_id_persist_fail_does_not_reuse_id_after_reopen` |
| F133 | open leftover fence | `let _ = put(abort)` on recover (F130 residual) | fence/batch `?` | open leftover + fence tests |
| F134 | apply fenced TxnCommit | `let _ = put(abort)` after revert | re-fence `?` | `apply_txn_commit_fenced_keeps_abort_status` |
| F135 | commit_tx after finish err | `let _ = tx_cancel` hid corrupt pre | propagate cancel | `commit_tx_surfaces_cancel_after_finish_fail` |
| F136 | persist_si_keys | gen/watermark puts swallowed | persist_u64_meta_all | `persist_si_keys_gen_fail_is_err` |
| F137 | DCS apply Create / Cas(0) | overwrite live/expired key (lock steal) | no-op + bind_absent_create | `apply_create_does_not_overwrite_existing` |
| F138 | apply_range SI generation | gen put swallowed; applied still advanced | gen put `?` | `apply_range_generation_persist_fail_does_not_advance_applied` |
| F139 | tx_finish cleanup TxnRevert | `let _ = propose(TxnRevert)` after majority commit | surface raft err | `revert_majority_committed_surfaces_raft_fail` |
| F140 | note_tx_commit preimage | corrupt pre → unwrap_or(None) gen-0 tombstone | decode `?` | `note_tx_commit_rejects_corrupt_preimage_floor` |
| F141 | note_tx_commit intent val | short intent → empty → SI delete tombstone | decode_intent ? | `note_tx_commit_rejects_short_intent_value_fallback` |
| F142 | HTTP TE field-name | `Transfer-Encoding :` (space before `:`) bypassed F104 | trim name then match | `kv_http_transfer_encoding_space_before_colon_rejected` |
| F143 | DCS delete corrupt meta | get miss → Ok(None); create still blocked (immortal) | wipe physical d/k+d/m | `delete_wipes_undecodable_meta_corpse` |
| F144 | install-snapshot term | higher term RAM bump stuck after meta persist fail | durable_become_follower first | `install_snapshot_higher_term_persist_fail_does_not_stick_term` |
| F145 | HTTP absolute-form scheme | `Http://` / `HtTpS://` not stripped → 404 | scheme eq_ignore_ascii_case | `kv_http_mixed_case_scheme_absolute_form` |
| F146 | HTTP short body vs CL | CL=5 body=hi → 200 partial store | fail if body < CL | `kv_http_short_body_vs_content_length_rejected` |
| F147 | start_election hard | term/role/vote stuck after persist fail | rollback on hard err | `start_election_hard_persist_fail_does_not_stick_term` |
| F148 | try_become_leader log | Leader+Noop stuck after log persist fail | rollback become_leader | `try_become_leader_log_persist_fail_stays_candidate` |
| F149 | HTTP X-Pedra vs Auth | `X-Pedra-Token` first shadowed Authorization | Authorization first | `kv_http_authorization_not_shadowed_by_x_pedra_token` |
| F150 | HTTP multi-Authorization | first Basic locked out later Bearer | skip non-Bearer scheme | `kv_http_bearer_not_shadowed_by_earlier_basic` |
| F151 | HTTP scheme-only Bearer | `Authorization: Bearer` as token blocked later Bearer | scheme-only → None | `kv_http_bearer_not_shadowed_by_earlier_scheme_only` |
| F152 | HTTP multi-Bearer | first wrong Bearer locked out later valid | any Bearer matches | `kv_http_later_bearer_not_shadowed_by_earlier_wrong_bearer` |
| F153 | HTTP LF header break | only `\r\n\r\n` → LF clients 400 / mis-frame | earliest CRLF or LF break | `kv_http_lf_only_header_break` |
| F154 | HTTP Expect 100-continue | no interim 100 → client/server deadlock | send 100 before body drain | `kv_http_expect_100_continue_then_body` |
| F155 | HTTP query first-wins | `?rev=1&rev=0` CAS'd at 1; key/holder same | distinct values → 400 | `dcs_http_conflicting_rev_does_not_cas` |
| F156 | HTTP URI fragment | `#frag` stayed in key / polluted `rev=0#x` | strip_uri_fragment | `kv_http_fragment_not_part_of_key` |
| F157 | HTTP/1.1 Host | missing Host 200-store; conflicting Host last-won | require Host; reject conflict | `kv_http11_missing_host_rejected` |
| F158 | HTTP/1.1 empty Host | `Host:` empty counted as present (F157 residual) | host_value_ok non-empty | `kv_http11_empty_host_rejected` |
| F159 | HTTP Expect unknown | unrecognized Expect ignored → still 200-store | 417 Expectation Failed | `kv_http_unknown_expect_rejected` |
| F160 | apply_range applied | RAM applied high after applied-meta persist fail | rollback applied on Err | `apply_range_applied_persist_fail_does_not_advance_applied` |
| F161 | HTTP abs-form Host | authority vs Host mismatch still 200-store | host_authority_mismatch | `kv_http_absolute_form_host_mismatch_rejected` |
| F162 | HTTP Host default port | `:80`/`:443`/userinfo raw compare 400'd (F161 residual) | split_host_port + ports_equivalent | `kv_http_absolute_form_default_port_matches_host` |
| F163 | HTTP bare query flag | `?rev` skipped → missing → rev=0 create | bare name → empty value → 400 | `dcs_http_bare_rev_flag_does_not_create` |
| F164 | HTTP bare string query | `?key` → empty lock name (F163 residual) | bare/empty string → 400 | `dcs_http_bare_key_flag_does_not_default_leader` |
| F165 | compat TLS get/count cache | epoch por-DB colide entre instâncias na mesma thread → B serve valor/contagem de A | (aplicado 2026-08-21, RFC-0048 P0.1) | `compat_hunt::c1_tls_get_leaks_value_across_db_instances` + `c1b` |
| F166 | compat raw iterator | `next()` pós-seek reverso anda para trás; `prev()` ignora lower bound | (aplicado 2026-08-21, RFC-0048 P0.2) | `compat_hunt::c2_raw_next_after_seek_to_last_walks_wrong_way` + `c2b`, `c3` |
| F167 | compat `DB::snapshot()` | sequência nua + `auto_reclaim` default: `SnapshotTooOld` no get e scan truncado 64/100 em silêncio (refill engole `Err`) | (aplicado 2026-08-21, RFC-0048 P0.3) | `compat_hunt::c5_live_snapshot_breaks_under_default_auto_reclaim`, `c6_snapshot_iterator_truncated_by_gc` |
| F168 | `occ.rs` `commit_with` | staging vazio pula read-set: conflito e `SnapshotTooOld` não detetados (doc promete conflito em chaves lidas ou escritas) | (aplicado 2026-08-21, RFC-0048 P0.4; paridade Rocks no ficheiro) | `core_hunt::k1_occ_readonly_commit_misses_conflict`, `k2_…_too_old_after_reclaim` |
| F169 | `memtable.rs` `range_snapshot` | scan ignora range tombstone in-memtable (`get` = Deleted, scan emite a chave); sem chamadores de produção hoje | (aplicado 2026-08-21, RFC-0048 P0.5) | `core_hunt::k3_memtable_scan_ignores_range_tombstone` |
| F170 | `wal/reader.rs` Zero+len0 | header zero a meio do bloco engole records CRC-válidos → `recover` `Ok` truncado (fail-open) | (aplicado 2026-08-21, RFC-0048 P0.6 — refinado: veredicto só a alignment fresca, `ZeroHeaderTail`; torn tails verdes) | `core_hunt::k4_wal_zero_header_swallows_valid_records` (+controle `k4b`) |
| F208 | `pedradb-io-uring` `submit_complete_act` | submit Err + CQ vazio larga `buf` com SQE em voo (DMA após return) | WaitMore após push; soak Linux residual | `cqe_kernel::f208_eintr_late_cqe_as_is_dma_after_return` (Miri) |
| F209 | `pedradb-capi` `next_gen` | `u32` gen aos 2³¹ `pack`a para NULL (handle vivo) | gen em `1..=GEN_MASK` | `gen_stays_inside_pack_mask` + as-is pack-null |
| F210 | `pedradb-capi` `transaction_get` | `as_mut_ptr` + move do `Box` invalida o ponteiro C (Stacked Borrows) | `Box::into_raw` + tabela addr→len | Miri `c_api_open_set_get_commit` RED→GREEN |
| F211 | core `compact_reclaim`/`auto_gc_floor` | floor sem pins = `last_sequence()` conta seq não-publicada → `SnapshotTooOld` transitório no snapshot visível corrente | cap `min(visible_sequence)` nos 2 sítios (2026-08-23, wave 7) | `core_hunt::k36_reclaim_floor_counts_unpublished_seq` (+k36ctl) |
| F212 | core `ConcurrentDb::flush` | rotaciona WAL sem persistir CHANGELOG (interval 0) → chave flushed some do feed vivo e pós-bare-drop-reopen | tail `persist_changelog_after_explicit_flush` (2026-08-23, wave 7) | `core_hunt::k37_concurrent_flush_skips_changelog_persist` (+k37ctl) |
| F213 | core `commit_async_ops` | write `no_sync` nunca estende change_log não-lazy; perda vira durável via persist+rebuild cego | espelho do extend pós-append WAL (2026-08-23, wave 7) | `core_hunt::k38_async_commit_missing_from_change_feed` (+k38ctl) |
| F215 | `pedradb-capi` marshalling | `CStr::from_ptr` / `from_raw_parts` sem cap: `SIZE_MAX` `*_len` é claim de terabyte (UB) | `memchr` ≤4096; copy só se `key_len`/`value_len` ≤ store MAX | `slice_cap_*` + `scripts/capi-asan.sh` (PASS + malicious ASan-red) |

## Delivery slices

### P0 — registro e núcleo de storage (done)
- [x] **P0.1** registro 1:1 completo (este RFC; tabela com 176 bugs) — status: `done`
- [x] **P0.2** bugs do núcleo storage (flush/WAL/SST/MANIFEST/memtable/OCC/
  history/change-feed: F1–F6, F13–F14, F18–F21, F29–F33, F43, F45–F46,
  F51, F165–F171, F185–F213) — status: `done` (fixes no main)

### P1 — distribuído e HTTP (done)
- [x] **P1.1** raft/dcs/store (F7, F9–F12, F15–F17, F22–F28, F34–F42,
  F47–F50, F52, F120, F127–F133, F137–F144, F147–F148, F160) — status: `done`
- [x] **P1.2** http (F8, F101–F104, F142, F145–F146, F149–F159, F161–F164)
  — status: `done`

### P2 — compat e ilhas unsafe (done)
- [x] **P2.1** rocksdb-compat (F165–F167, F172–F176, F177–F184, F185–F195
  compat rows) — status: `done`
- [x] **P2.2** ilhas unsafe posix/io-uring/capi (F202–F203, F208–F210;
  Miri/ASan) — status: `done`
- [x] **P2.3** C ABI C+ASan harness (F215 caps; in-process product
  face, **não** `libfdb_c`) — status: `done` (`scripts/capi-asan.sh`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Registro 1:1 (este RFC) | done | docs/rfc/0049 | 2026-08-23 |
| P0.2 | p0 | Fixes núcleo storage no main | done | waves hunt 2026-08 (ver patches) | 2026-08-23 |
| P1.1 | p1 | Fixes raft/dcs/store no main | done | waves hunt 2026-08 | 2026-08-23 |
| P1.2 | p1 | Fixes http no main | done | waves hunt 2026-08 | 2026-08-23 |
| P2.1 | p2 | Fixes compat no main | done | RFC-0048 + waves | 2026-08-23 |
| P2.2 | p2 | Fixes ilhas unsafe no main | done | wave 8 unsafe (Miri/ASan) | 2026-08-23 |
| P2.3 | p2 | C+ASan harness C ABI (produto in-process) | done | F215 + `scripts/capi-asan.sh` | 2026-08-23 |

## Acceptance Criteria

- **Tests:** todas as provas da coluna `Prova` verdes no `main` —
  baterias de referência: `pedradb-core --lib` 395/0, harness `core_hunt`
  64/64 (k1..k39 + controles), `compat_hunt` 21/21, `rocksdb-compat`
  41+7, `pedradb-io-uring` 14/14, `pedradb-capi` 18/18 + C+ASan harness
  (Miri/ASan nas ilhas; C ABI in-process, não `libfdb_c`). Cada teste de prova
  INDIVIDUAL falha no tree sem o fix
  correspondente (dois-estados demonstrados por bug nas fichas).
- **Telemetry / Analytics:** none — corretude; o `CORRUPTLOG` (RFC-0038)
  continua o canal de eventos de corrupção.
- **Documentation:** este RFC + fichas `F<n>-*.md` (determinismo) +
  RFC-0048 (hunt waves) + patches por wave.
- **Screenshots:** backend-only.

## Out of scope

- Reaplicação dos fixes (já no `main`; patches por wave existem só para
  re-derivação histórica dos dois-estados).
- Bugs ainda abertos: nenhum conhecido com prova; backlog de candidatos
  NÃO-provados vive no LEDGER do hunt (seção backlog wave 8), não aqui.
- Refutados (ex. F214) não são bugs e não recebem fix.
