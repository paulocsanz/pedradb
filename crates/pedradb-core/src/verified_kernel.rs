//! RFC-0058 P0.1: the verified profile — a **declared** composition of
//! proven kernels instead of an accident of platform defaults.
//!
//! What "verified" means here (and no more): every critical section the
//! mode executes is either (a) a kernel proved in the formal pipeline and
//! cataloged in `scripts/formal/catalog.json`, or (b) a published contract
//! enforced in code and exercised by the DST battery (crash / reopen /
//! EIO). It does **not** mean code extracted from theorems (rejected,
//! LEDGER L46 / `VeriBetrKV` 8×) and it does not mean "no bugs" — the
//! residual (OS lying on fsync, hardware, field) stays published in
//! `research/DST-VS-FDB-SIM.md`.
//!
//! Composition (file level, [`OpenOptions::verified`]):
//! `sync=true`, `wal_full_fsync=true` (strongest data class),
//! `wal_recovery=FailClosed`. Composition (group level,
//! [`ConcurrentDb::pin_verified`]), reactivated by RFC-0058 P2.1 with the
//! proved group-commit kernel (RFC-0057 P2.1): the leader/member merge
//! runs (`group_commit_kernel` decides first-committer-wins and group
//! atomicity; `fence_publish_seq` publishes the group at one watermark),
//! the catch-up window is pinned to 0 (merging by natural queuing, never
//! by a delay window), and async writers keep the un-merged bypass (no
//! leader dependency). Product constructors pin `StdEnv` — the `io_uring`
//! ring is out of the mode (P2.2); the full mode keeps its
//! `PosixFallback`.
//!
//! [`profile_report`] is the machine-checked tie to the catalog: the set
//! of ON kernels must equal the catalog pair ids exactly. Adding a kernel
//! to the catalog without claiming it here (On, or flipping an Off entry
//! to On) fails `verified_report_matches_catalog` — the report is a
//! living ritual, not documentation.

use crate::concurrent::ConcurrentDb;
use crate::db::{OpenOptions, WalRecovery};
use crate::env::{Env, StdEnv};
use crate::Result;
use std::path::Path;

/// Version tag of the declared composition (report format, not semver).
/// v2 = RFC-0058 P2.1: the merge is back with the proved kernel.
pub const PROFILE_VERSION: &str = "verified-v2";

/// Whether a component of the mode is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileState {
    /// Active in the verified mode.
    On,
    /// Deliberately off in the verified mode (see `note` for the gate).
    Off,
}

/// One row of [`profile_report`]: component ⇒ state ⇒ kernel.
///
/// `kernel` is the id of a `pairs` entry in `scripts/formal/catalog.json`
/// (`None` = a published contract without a theorem — see the note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileComponent {
    /// Component name (stable string; mirrors the catalog id when one
    /// exists).
    pub component: &'static str,
    /// On / Off in this mode.
    pub state: ProfileState,
    /// Catalog kernel id backing the component, when one exists.
    pub kernel: Option<&'static str>,
    /// Why this state holds / what gates a change.
    pub note: &'static str,
}

macro_rules! on {
    ($c:expr, $k:expr, $n:expr) => {
        ProfileComponent {
            component: $c,
            state: ProfileState::On,
            kernel: Some($k),
            note: $n,
        }
    };
}

macro_rules! off {
    ($c:expr, $n:expr) => {
        ProfileComponent {
            component: $c,
            state: ProfileState::Off,
            kernel: None,
            note: $n,
        }
    };
}

macro_rules! contract {
    ($c:expr, $n:expr) => {
        ProfileComponent {
            component: $c,
            state: ProfileState::On,
            kernel: None,
            note: $n,
        }
    };
}

/// Component ⇒ state ⇒ kernel for the verified mode (RFC-0058 P0.1).
///
/// Invariant (machine-checked by `verified_report_matches_catalog`): the
/// set of `kernel` ids over ON entries equals the catalog `pairs` ids —
/// every proved kernel is claimed by the mode and nothing else is.
#[must_use]
pub fn profile_report() -> &'static [ProfileComponent] {
    &[
        // --- recovery (fail-closed) ---
        on!("wal_recover", "wal_recover", "fail-closed prefix recovery at open (F4/F14)"),
        on!("from_record_type", "from_record_type", "type byte → fragment kind (F14)"),
        on!("fragment_act", "fragment_act", "F14 fragment assembly decision"),
        on!("physical_payload_act", "physical_payload_act", "F4 payload vs block bounds"),
        on!("is_length_resyncable", "is_length_resyncable", "F4 resync class is exactly the framing errors"),
        on!("manifest_recover", "manifest_recover", "MANIFEST recovery (F196/G8)"),
        on!("first_install", "first_install", "first MANIFEST install action (F196)"),
        on!("reopen_outcome", "reopen_outcome", "reopen state equals the pre-crash visible state (F170/F171/G8)"),
        on!("dictionary_link", "dictionary_link", "crash-dictionary put→get link across flush/crash (G1/G8)"),
        on!("vlog_recover", "vlog_recover", "value-log recovery (F51)"),
        on!("blob_gc_pick", "blob_gc_pick", "blob GC picks the active generation"),
        // --- commit path (single-writer critical section) ---
        on!("apply_step", "apply_step", "batch apply under the single write lock — the verified commit critical section (F10-apply)"),
        on!("grant_persist", "grant_persist", "durability grant persistence (F15)"),
        on!("durable_term", "durable_term", "durable become-follower only on a newer term (RFC-0158 / F125/F127)"),
        on!("txn", "txn", "multi-key TX all-or-nothing (F47/F34)"),
        on!("tx_glue", "tx_glue", "OCC glue — validation under the write lock keeps first-committer-wins lone (F47/F34)"),
        on!("revert_clears_status", "revert_clears_status", "TX revert clears the intent status (F47)"),
        on!("revert_user_action", "revert_user_action", "TX revert only reverts user actions (F34)"),
        on!("should_repair_si_hist", "should_repair_si_hist", "SI history repair only when the user key is restored (F52)"),
        on!("discard_cut", "discard_cut", "discard-uncommitted never cuts committed keys (RFC-0002 P9–P10)"),
        on!("leftover_txn_is_aborted", "leftover_txn_is_aborted", "leftover intents read as aborted on reopen (F35)"),
        on!("next_txn_id_after", "next_txn_id_after", "recovered next TX id is past every seen id (F35)"),
        on!("recover_si_generation", "recover_si_generation", "SI generation survives reopen (F36)"),
        on!("prepare_error_aborts_earlier", "prepare_error_aborts_earlier", "prepare error aborts the earlier TX (F50)"),
        on!("reserve_si_gen", "reserve_si_gen", "SI gen reservation advances and differs (F49)"),
        on!("unreserve_si_gen", "unreserve_si_gen", "SI gen unreserve only if still ours (F49)"),
        on!("isolated", "isolated", "isolated apply (F83)"),
        on!("isolated_child", "isolated_child", "isolated child byte layout (F83)"),
        // --- commit path (group level — RFC-0058 P2.1 reactivation) ---
        on!("write_group_merge", "group_commit", "leader/member merge with the proved kernel: first-committer-wins cross-group, group atomicity intra-group (RFC-0057 P2.1 / RFC-0051 P1.3)"),
        on!("group_fence", "group_fence", "one publish watermark per group = max appended member sequence, after WAL durability (RFC-0057 P2.1)"),
        // --- background decisions ---
        on!("flush_decision", "flush_decision", "when to flush (F2/F43/G1)"),
        on!("flush_publish", "flush_publish", "MANIFEST after durable SST (RFC-0151 P1)"),
        on!("flush_plan", "flush_plan", "flush plan selection (F2/F43)"),
        on!("compact_decision", "compact_decision", "when to compact (F177/F20)"),
        on!("compact_retention", "compact_retention", "what compaction retains (F177/F20)"),
        on!("compact_split", "compact_split", "compaction splits before OOM (RFC-0160)"),
        on!("compact_split_at", "compact_split_at", "compaction split point (RFC-0160)"),
        on!("lone_tombstone", "lone_tombstone", "lone tombstone fate (F177)"),
        on!("leveling", "leveling", "level-size ladder of the leveled scheduler (F-leveling-sweep)"),
        on!("leveling_pick", "leveling_pick", "leveled job selection: overlap slice, input cap, disjoint gate (F-leveling-sweep)"),
        on!("leveling_pushdown", "leveling_pushdown", "leveled pushdown: disjoint-gated oldest-chunk demotion (F-leveling-sweep)"),
        on!("leveled_enabled", "leveled_enabled", "leveled scheduling gate (F-leveling-sweep)"),
        on!("leveling_disjoint", "leveling_disjoint", "level run disjointness (F-leveling-sweep)"),
        on!("leveling_overlaps", "leveling_overlaps", "key-range overlap test (F-leveling-sweep)"),
        on!("leveling_total_bytes", "leveling_total_bytes", "level size accounting (F-leveling-sweep)"),
        on!("compact", "compact", "merge iterator correctness (F27/F28)"),
        on!("compact_floor", "compact_floor", "compact index floor (F27)"),
        on!("compact_ready", "compact_ready", "compact readiness decision (F27)"),
        on!("compact_peer_counts", "compact_peer_counts", "peer counts for compact (F28)"),
        // --- reads ---
        on!("snapshot", "snapshot", "snapshot reads (F38/F40/F41)"),
        on!("si_reader", "si_reader", "secondary-index reader construction (F42/F84)"),
        on!("si_read", "si_read", "secondary-index read (F168)"),
        on!("point_get_prefer", "point_get_prefer", "point-get prefer decision applied (F84)"),
        on!("point_get_wm", "point_get_wm", "point-get watermark (F42/F84)"),
        on!("snap_txn_clear", "snap_txn_clear", "snapshot needs txn meta clear (F40)"),
        on!("len_tag", "len_tag", "value length tag (F80)"),
        on!("exact_children", "exact_children", "exact value children (F78/F80)"),
        on!("index_val", "index_val", "index entry validation (F80)"),
        on!("fail_closed", "fail_closed", "read APIs fail closed — corruption is never served as a miss (F102/F104/F105)"),
        on!("changelog", "changelog", "changelog decode/replay (F53)"),
        on!("changelog_should_store", "changelog_should_store", "changelog debounce should-store decision (RFC-0031 P0.1)"),
        on!("changelog_budget", "changelog_budget", "changelog rebuild within budget (RFC-0039 P0.3)"),
        on!("range_covers", "range_covers", "range bounds cover the requested span (F30)"),
        on!("prefix", "prefix", "prefix seek bounds (F57/F58)"),
        on!("stream_cursor", "stream_cursor", "stream cursor resume (F54)"),
        on!("stream_next_seq", "stream_next_seq", "stream next sequence number (F54)"),
        on!("world_trajectory", "world_trajectory", "world trajectory violation decision (RFC-0059 P2.2)"),
        on!("world_trajectory_fold", "world_trajectory_fold", "world trajectory check on fold (RFC-0059 P2.2)"),
        on!("scan_guard", "scan_guard", "scan lifetime guard (F167)"),
        on!("journal_pin", "journal_pin", "pin-aware journal reclaim (H1/F54)"),
        on!("journal_catch_up_pin", "journal_catch_up_pin", "journal catch-up pins on read (W4)"),
        on!("journal_fold_pin", "journal_fold_pin", "journal fold pins on read (H1)"),
        on!("journal_next_pin", "journal_next_pin", "journal next pin decision (H1)"),
        on!("bloom_header", "bloom_header", "bloom header validation (T1)"),
        on!("bloom_insert", "bloom_insert", "bloom insert (T1)"),
        on!("bloom_may_contain", "bloom_may_contain", "bloom probe — no false negatives (T1/T4)"),
        on!("fold_range", "fold_range", "fold Storage range consistency (F169)"),
        // --- montanha (world nodes) ---
        on!("vote", "vote", "raft vote handling (F15)"),
        on!("ae_entry", "ae_entry", "append-entries entry validation (F16)"),
        on!("ae_f16_gate", "ae_f16_gate", "append-entries F16 safety gate (F16)"),
        on!("ae_ack", "ae_ack", "append-entries ack (F48)"),
        on!("commit_raft", "commit_raft", "raft commit advancement (F10/F11/F23)"),
        on!("raft_recover_applied", "raft_recover_applied", "raft recover last-applied (F10)"),
        on!("lease", "lease", "lease grant/expiry persistence (F7/F56)"),
        on!("lease_table", "lease_table", "lease table expiry check (F7)"),
        on!("lease_next_id", "lease_next_id", "lease next id after grant (F7)"),
        on!("dcs_apply", "dcs_apply", "DCS command application (F12/F22)"),
        on!("dcs_advance_bool", "dcs_advance_bool", "DCS apply should-advance decision (F12/F22)"),
        // --- ship / product surface ---
        on!("pack", "pack", "pack encoding validation (F62)"),
        on!("ship_guard", "ship_guard", "ship guard (F165)"),
        on!("ship_stamp", "ship_stamp", "ship stamp changed decision (F165)"),
        on!("bearer", "bearer", "bearer auth (F85/F79)"),
        on!("content_length", "content_length", "content-length handling (F86/F87/F88)"),
        on!("invalid_cl_zero", "invalid_cl_zero", "invalid content-length folds to zero (F87)"),
        on!("cl_repeat_conflict", "cl_repeat_conflict", "repeated content-length conflicts rejected (F88)"),
        on!("short_body_vs_cl", "short_body_vs_cl", "body shorter than content-length is error (F146)"),
        on!("form_plus", "form_plus", "form parsing (F101)"),
        on!("origin_path", "origin_path", "origin path resolution (F91/F92)"),
        on!("path_after_authority", "path_after_authority", "absolute-form path after authority (F91)"),
        on!("strip_http_authority", "strip_http_authority", "authority stripped before routing (F91/F145)"),
        on!("request_target_authority", "request_target_authority", "request-target authority-form (F161)"),
        on!("host_authority_mismatch", "host_authority_mismatch", "Host/authority mismatch rejected (F161/F162)"),
        on!("split_host_port", "split_host_port", "host:port split (F162)"),
        on!("strip_uri_fragment", "strip_uri_fragment", "URI fragment stripped (F156)"),
        on!("strip_authority_for_routing", "strip_authority_for_routing", "routing path keeps query (F91/F92)"),
        on!("form_plus_byte", "form_plus_byte", "form plus byte decoding (F101)"),
        on!("from_hex", "from_hex", "hex decoding (F76)"),
        on!("plus_before_percent", "plus_before_percent", "plus before percent-order (F101)"),
        on!("query_values_conflict", "query_values_conflict", "conflicting query values rejected (F155)"),
        on!("query_u64_conflict", "query_u64_conflict", "u64 query conflict (F88/F155)"),
        on!("query_part_is_bare_name", "query_part_is_bare_name", "bare-name query part (F162)"),
        on!("ascii_lower", "ascii_lower", "ASCII lowercase (F85)"),
        on!("ascii_upper", "ascii_upper", "ASCII uppercase (F79)"),
        on!("is_bearer_scheme", "is_bearer_scheme", "bearer scheme detection (F85)"),
        on!("is_non_bearer_auth_scheme", "is_non_bearer_auth_scheme", "non-bearer scheme rejected (F150/F151)"),
        on!("normalize_http_method", "normalize_http_method", "HTTP method normalization (F79)"),
        on!("authorization_matches", "authorization_matches", "constant-time authorization match (F149/F152)"),
        on!("children", "children", "node children validation (F59)"),
        on!("fields", "fields", "field validation (F60)"),
        on!("children_start", "children_start", "packed children start (F57)"),
        on!("children_half_open", "children_half_open", "children key in half-open range (F57/F59)"),
        on!("children_next_byte", "children_next_byte", "next byte in packed children (F59)"),
        on!("fields_kept", "fields_kept", "field kept in decoded suffix (F60)"),
        on!("fields_suffix", "fields_suffix", "child bytes after prefix (F60)"),
        on!("fields_decode", "fields_decode", "fields decode (F60)"),
        on!("fields_pair_nul", "fields_pair_nul", "decode first NUL of pair (F60)"),
        // --- membership / L28 / residuals (catalog pairs; claim On) ---
        on!("joint_election", "joint_election", "joint election old∧new (RFC-0064)"),
        on!("joint_leave", "joint_leave", "joint still active until leave (RFC-0066)"),
        on!("pending_joint_node", "pending_joint_node", "pending joint node counts"),
        on!("joint_leave_ok", "joint_leave_ok", "joint leave Ok"),
        on!("election_grant_from", "election_grant_from", "election grant-from member"),
        on!("joint_target", "joint_target", "joint vote target"),
        on!("joint_add_target", "joint_add_target", "joint add target"),
        on!("queued_leave_finish", "queued_leave_finish", "queued leave finish"),
        on!("disk_membership", "disk_membership", "disk membership overrides CLI"),
        on!("high_water", "high_water", "high-water survives open"),
        on!("participating_member", "participating_member", "is_participating requires ids"),
        on!("identity_before_applied", "identity_before_applied", "identity before applied"),
        on!("recover_apply", "recover_apply", "recover apply committed"),
        on!("recover_apply_node", "recover_apply_node", "recover apply on removed replica"),
        on!("recover_truncate", "recover_truncate", "recover truncate uncommitted"),
        on!("recover_drop_orphan", "recover_drop_orphan", "recover drop orphan seg"),
        on!("recover_abort", "recover_abort", "recover abort leftover 2PC"),
        on!("persist_meta", "persist_meta", "persist meta local non-member"),
        on!("persist_hist", "persist_hist", "persist SI hist local non-member"),
        on!("persist_fence", "persist_fence", "persist fence local non-member"),
        on!("force_clear", "force_clear", "force-clear local non-member"),
        on!("drop_preimages", "drop_preimages", "drop preimages local non-member"),
        on!("open_peer_disk", "open_peer_disk", "open peer uses disk ids"),
        on!("local_id_member", "local_id_member", "local id if member"),
        on!("reader_local", "reader_local", "reader id local"),
        on!("discard_uncommitted", "discard_uncommitted", "discard uncommitted local non-member"),
        on!("discard_leader", "discard_leader", "discard persist-leader local"),
        on!("removed_step_down", "removed_step_down", "removed replica steps down"),
        on!("hint_member", "hint_member", "leader hint omits removed"),
        on!("drop_repl_slot", "drop_repl_slot", "drop repl slot of removed"),
        on!("drop_sent_through", "drop_sent_through", "drop sent_through of removed"),
        on!("compact_unleft", "compact_unleft", "compact through unleft joint"),
        on!("rpc_mode", "rpc_mode", "Queued RPC pin fail-closed"),
        on!("group_publish", "group_publish", "group publish after WAL durable"),
        on!("forall_schedules", "forall_schedules", "PCT depth is not ∀ schedules"),
        on!("l28_durability", "l28_durability", "L28 real TCP durability"),
        on!("l28_tcp_left", "l28_tcp_left", "L28 TCP leave on disk"),
        on!("l28_tcp_hw", "l28_tcp_hw", "L28 TCP high-water"),
        on!("l28_tcp_part", "l28_tcp_part", "L28 TCP participating"),
        on!("l28_tcp_apply", "l28_tcp_apply", "L28 TCP recover apply"),
        on!("l28_tcp_napply", "l28_tcp_napply", "L28 TCP recover apply node"),
        on!("l28_tcp_trunc", "l28_tcp_trunc", "L28 TCP recover truncate"),
        on!("l28_tcp_odrop", "l28_tcp_odrop", "L28 TCP orphan drop"),
        on!("l28_tcp_abort", "l28_tcp_abort", "L28 TCP recover abort"),
        on!("l28_tcp_nowms", "l28_tcp_nowms", "L28 TCP persist now_ms"),
        on!("l28_tcp_hist", "l28_tcp_hist", "L28 TCP persist hist"),
        on!("l28_tcp_fence", "l28_tcp_fence", "L28 TCP persist fence"),
        on!("l28_tcp_clear", "l28_tcp_clear", "L28 TCP force clear"),
        on!("l28_tcp_pre", "l28_tcp_pre", "L28 TCP drop preimages"),
        on!("l28_tcp_peer", "l28_tcp_peer", "L28 TCP open peer disk"),
        on!("l28_tcp_lid", "l28_tcp_lid", "L28 TCP local id"),
        on!("l28_tcp_rdr", "l28_tcp_rdr", "L28 TCP reader local"),
        on!("l28_tcp_dsc", "l28_tcp_dsc", "L28 TCP discard"),
        on!("l28_tcp_pld", "l28_tcp_pld", "L28 TCP persist-leader"),
        on!("l28_tcp_std", "l28_tcp_std", "L28 TCP removed step-down"),
        on!("l28_tcp_hnt", "l28_tcp_hnt", "L28 TCP leader hint"),
        on!("l28_tcp_slot", "l28_tcp_slot", "L28 TCP drop repl slot"),
        on!("l28_tcp_sth", "l28_tcp_sth", "L28 TCP drop sent_through"),
        on!("l28_tcp_pj", "l28_tcp_pj", "L28 TCP plant committed joint"),
        on!("l28_tcp_dterm", "l28_tcp_dterm", "L28 TCP removed-replica durable-term (RFC-0158)"),

        on!("probe_order", "probe_order", "L0 equal-lo probe newest-first (RFC-0164)"),
        on!("run_disjoint", "run_disjoint", "SST run pairwise-disjoint lo (RFC-0164 P1.2)"),
        on!("probe_order_covering", "probe_order_covering", "covering probe bounds (RFC-0164 P0.2)"),
        on!("sched_plant_joint", "sched_plant_joint", "World opt-in PlantCommittedJoint"),
        on!("liveness_claim", "liveness_claim", "liveness ES axioms fail-closed"),
        on!("fsync_promote", "fsync_promote", "fsync promotes pending"),
        on!("media_durable", "media_durable", "fsync Ok is not media proof"),
        on!("env_crash", "env_crash", "Env crash geometry: legal cut ∈ [synced, written] (RFC-0166 P1.1)"),
        on!("env_append", "env_append", "Env append grows written, barrier unmoved (RFC-0166 P1.1)"),
        on!("env_sync", "env_sync", "honest sync promotes all; lying promotes nothing (RFC-0166 P1.1)"),
        on!("env_barrier_floor", "env_barrier_floor", "legal crash never loses a synced byte (RFC-0166 P1.1)"),
        on!("env_no_invented", "env_no_invented", "legal crash never invents a byte (RFC-0166 P1.1)"),
        on!("env_honest_sync", "env_honest_sync", "honest sync protects the whole log (RFC-0166 P1.1)"),
        on!("wal_state", "wal_state", "Inv-WAL: acked ⊆ synced ⊆ recoverable prefix (RFC-0166 P1.2)"),
        on!("wal_append", "wal_append", "append preserves Inv-WAL (RFC-0166 P1.2)"),
        on!("wal_sync", "wal_sync", "sync preserves Inv-WAL for both honesties (RFC-0166 P1.2)"),
        on!("wal_ack", "wal_ack", "ack past the barrier is fail-closed; Inv-WAL preserved (RFC-0166 P1.2)"),
        on!("wal_rotate", "wal_rotate", "rotate only drops a durable+acked log (RFC-0166 P1.2)"),
        on!("wal_acked_survives", "wal_acked_survives", "every legal crash keeps the acked prefix (RFC-0166 P1.2)"),
        on!("d1_put_ok", "d1_put_ok", "model write path: append → honest sync → ack-all (RFC-0166 P1.3)"),
        on!("d1_modelo", "d1_modelo", "D1-modelo: put Ok ⇒ survives every torn prefix (RFC-0166 P1.3)"),
        on!("write_ack_append", "write_ack_append", "verified write→ack: ledger append step (RFC-0166 P1.4)"),
        on!("write_ack_barrier", "write_ack_barrier", "verified write→ack: ledger barrier step (RFC-0166 P1.4)"),
        on!("write_ack_ack", "write_ack_ack", "verified write→ack: ledger ack = put_ok composition, Inv-WAL asserted live (RFC-0166 P1.4)"),
        on!("d1_durability", "d1_durability", "D1 property spec; refinement theorem is d1_modelo (RFC-0166 P0.1/P1.3/P2.4)"),
        on!("lsm_probe", "lsm_probe", "Inv-LSM probe: the recency walk answers the newest version — deepest-first AS-IS resurrects (RFC-0166 P2.1)"),
        on!("lsm_compact", "lsm_compact", "Inv-LSM preserved by compact — newest wins across source levels, bottom tombstones retire (RFC-0166 P2.1)"),
        on!("lsm_reopen", "lsm_reopen", "Inv-LSM preserved by reopen — the durable order rebuilds the same probe order (RFC-0166 P2.1)"),
        on!("r1_modelo", "r1_modelo", "R1-modelo: under Inv-LSM the probe answers the newest version — no delete resurrects (RFC-0166 P2.1)"),
        on!("tx_abort", "tx_abort", "T1 abort: revert + keep the fence so a later commit replay still reverts (RFC-0166 P2.2)"),
        on!("tx_recover", "tx_recover", "T1 recover: leftover prepared / mid-apply TX is aborted — no partial visibility (RFC-0166 P2.2)"),
        on!("t1_modelo", "t1_modelo", "T1-modelo: after recover, T1 holds — even from a mid-apply crash (RFC-0166 P2.2)"),
        on!("c1_advance_commit", "c1_advance_commit", "C1 commit step: index moves only on joint quorum + matching term (RFC-0166 P2.3)"),
        on!("c1_modelo", "c1_modelo", "C1-modelo: a served index is covered by majority-durable commit of every active config (RFC-0166 P2.3)"),
        on!("group_validate", "group_validate", "group membership validation (RFC-0051 P1.3 / RFC-0057 P2.1)"),
        on!("pct_default_depth", "pct_default_depth", "PCT campaign default depth (RFC-0070 P2.2)"),
        on!("default_pct_depth_raised", "default_pct_depth_raised", "default PCT depth raised (RFC-0070 P2.2)"),
        on!("fsync_lie_tcg", "fsync_lie_tcg", "fsync-lie closes the TCG guest (RFC-0078 P2.2)"),
        on!("stacked_liars", "stacked_liars", "stacked fsync liars refused (RFC-0078 P1.2)"),
        on!("r1_no_resurrection", "r1_no_resurrection", "R1 property spec; refinement theorem is r1_modelo (RFC-0166 P0.1/P2.1/P2.4)"),
        on!("t1_atomicity", "t1_atomicity", "T1 property spec; refinement theorem is t1_modelo (RFC-0166 P0.2/P2.2/P2.4)"),
        on!("c1_quorum", "c1_quorum", "C1 property spec; refinement theorem is c1_modelo (RFC-0166 P0.2/P2.3/P2.4)"),
        on!("tcg_guest", "tcg_guest", "TCG guest claim fail-closed"),
        on!("fdatasync_rc", "fdatasync_rc", "fdatasync nonzero rc is not Ok"),
        on!("cqe_res", "cqe_res", "negative CQE res is not Ok"),
        on!("cqe_tags", "cqe_tags", "CQE user-data tag advance (F203)"),
        on!("cqe_leftover", "cqe_leftover", "leftover CQE adoption decision (F203/U1)"),
        on!("cqe_submit", "cqe_submit", "submit-err + CQ state decision — WaitMore, never Err (F208)"),
        on!("cqe_ring_refusal", "cqe_ring_refusal", "CQE ring model admission gate (RFC-0074 P2.2)"),
        on!("c_len", "c_len", "C API oversize len is LIMIT"),
        on!("crc_match", "crc_match", "CRC mismatch is not Ok"),
        on!("sst_crc", "sst_crc", "SST CRC fate fail-closed"),
        on!("sst_block_crc", "sst_block_crc", "SST block CRC admission (RFC-0077 P1.1)"),
        on!("sst_magic", "sst_magic", "SST magic admission — only PEDRSST\\0 opens; a C++ Rocks header refuses (RFC-0186 P2.2)"),
        on!("tombstone_reaches_window", "tombstone_reaches_window", "range tombstone reaches the window (F167)"),
        on!("key_in_window", "key_in_window", "key inside the scan window (F167)"),
        on!("point_bounds_overlap", "point_bounds_overlap", "point bounds overlap (F167)"),
        // --- RFC-0150 dictionary / compat kernels ---
        on!("cf_family", "cf_family", "CF family membership / encode (scan leak fail-closed)"),
        on!("cf_family_of", "cf_family_of", "CF family of a key (RFC-0150)"),
        on!("cf_encode_effective", "cf_encode_effective", "effective CF encode for pooled handles (RFC-0150)"),
        on!("encode_cf_key", "encode_cf_key", "CF key encoding (RFC-0150)"),
        on!("decode_cf_key", "decode_cf_key", "CF key decoding (RFC-0150)"),
        on!("infer_sst_cf", "infer_sst_cf", "SST CF inference from the flush tag (RFC-0150)"),
        on!("compact_rewrites_sst_cf", "compact_rewrites_sst_cf", "compaction rewrites the SST CF (RFC-0150)"),
        on!("visible_at", "visible_at", "snapshot merge visibility + F30 range tombstone"),
        on!("ikey_pack", "ikey_pack", "InternalKey packed trailer + seq-desc Ord"),
        on!("write_record_count", "write_record_count", "WriteRecord count is atomic (no silent prefix)"),
        on!("pin_gc", "pin_gc", "SnapshotPin is oldest_snapshot for point_version_fate"),
        on!("wait_for_deadlock", "wait_for_deadlock", "TransactionDB 2PL wait-for cycle is Deadlock"),
        on!("si_hist_repair", "si_hist_repair", "SI hist repair plan (RFC-0191 P1.5)"),
        on!("iter_window", "iter_window", "compat iterator window vs visible_at (RFC-0151 P1)"),
        on!("l28_napply_retry", "l28_napply_retry", "L28 TCP napply retry is not forall"),
        on!("zero_glue", "zero_glue", "zero remaining glue is not a theorem"),
        on!("lock_interleavings", "lock_interleavings", "lock/OS-scheduler interleavings are not forall"),
        on!("hist_load_fate", "hist_load_fate", "RFC-0191 P2.3 cadence (third if): the SI-hist load merge disposition — a decoded"),
        on!("write_op_range_end", "write_op_range_end", "2026-09-13 RFC-0218 P1.2: fate ∀ sobre write_op_range_end no degrau átomo — desp"),
        on!("merge_sift", "merge_sift", "merge_sift"),
        on!("wal_rotate_decision", "wal_rotate_decision", "wal_rotate_decision"),
        on!("occ_snap_published", "occ_snap_published", "RFC-0224/escada 0188: registered forall atom; flag stays false where admitted"),
        on!("group_commit", "group_commit", "átomo 1/4 P0.1: occ_conflict como iff exata da janela (snap, last_seq] — conflit"),
        on!("apply_put_plan", "apply_put_plan", "RFC-0191 P2.3 cadence (second if): the apply-path Put record fate — hist persist"),
        on!("rwlock_client_may_mutate", "rwlock_client_may_mutate", "RFC-0202 P0.1 data-race row: write-lock client protocol — Db mutation permitted "),
        on!("changelog_durable_commit", "changelog_durable_commit", "RFC-0219 P0.1: o destino do debounce de CHANGELOG num commit terminado é decisão"),
        on!("wal_archive_delete", "wal_archive_delete", "RFC-0219 P0.2: o destino keep-vs-delete da cadeia arquivada de WAL é decisão do "),
        on!("bulk_manifest_persist", "bulk_manifest_persist", "RFC-0219 P0.3: como o publish do MANIFEST de um bulk install é pago é decisão do"),
        on!("point_cache_validity", "point_cache_validity", "RFC-0219 P1.1: o gate F198/F207 de fill/hit do point/prefix cache é decisão do k"),
        on!("point_tombstone", "point_tombstone", "RFC-0219 P1.1: se um ponto achado é servido ou sombreado por range tombstone é d"),
        on!("dir_sync_plan", "dir_sync_plan", "RFC-0219 P1.1: se o dir-fsync pós-rename é pago é decisão do kernel — modo sync "),
        on!("fence_admission", "fence_admission", "RFC-0219 P1.2: admitir ou recusar operações após um fence de durabilidade é deci"),
        on!("fence_record", "fence_record", "RFC-0219 P1.2: se um fence novo registra o relatório é decisão do kernel — só o "),
        on!("group_batch_sync", "group_batch_sync", "RFC-0219 P1.2: se um batch força a barreira única do grupo é decisão do kernel —"),
        on!("pit_resync_rewrite_plan", "pit_resync_rewrite_plan", "RFC-0219 P1.4: reescrever o WAL a partir do prefixo recuperado é decisão do kern"),
        on!("parked_pair", "parked_pair", "RFC-0219 P1.3: se a fila parked-unflushed entrega o par mais velho para fold é d"),
        on!("auto_flush_gate", "auto_flush_gate", "RFC-0219 P1.3: se o scan de auto-flush roda é decisão do kernel — os dois eixos "),
        on!("mem_auto_flush", "mem_auto_flush", "RFC-0219 P1.3: flushar a mem é decisão do kernel — dispara EXATAMENTE armado e n"),
        on!("manifest_publish_plan", "manifest_publish_plan", "RFC-0219 P1.4: publicar MANIFEST/CURRENT é decisão do kernel — publica EXATAMENT"),
        on!("changelog_store_plan", "changelog_store_plan", "RFC-0219 P1.4: gravar o feed no store point síncrono é decisão do kernel — grava"),
        on!("parked_pop_plan", "parked_pop_plan", "RFC-0219 P2.1: popar a fila estacionada é decisão do kernel — popa EXATAMENTE qu"),
        on!("put_handler_plan", "put_handler_plan", "RFC-0157 stage 2: o script rustc-linked de Db::put -> apply_batch_with e o kerne"),
        on!("open_wal_head_plan", "open_wal_head_plan", "RFC-0157 stage 2: o script rustc-linked de open_with_env_sourced no WAL-head e o"),
        on!("group_ack_plan", "group_ack_plan", "RFC-0219 P2.1: ackar e publicar o grupo é decisão do kernel — publica EXATAMENTE"),
        on!("lock_alphabet_linearizes_n2", "lock_alphabet_linearizes_n2", "Passo 2 ∀π: alfabeto finito acquire-write/flush/submit/publish; rustc lock_alpha"),
        on!("cf_flush_plan", "cf_flush_plan", "RFC-0219 P2.1: dentro do scan armado de auto-flush, flushar a família EXATAMENTE"),
        on!("flusher_gate_plan", "flusher_gate_plan", "RFC-0219 P2.2: o regime de submit/park/assist (workerless vs worker drenando) é "),
        on!("parked_debt_plan", "parked_debt_plan", "RFC-0219 P2.2: a dívida parked-unflushed ser real (parquear/assistir) é decisão "),
        on!("occ_member_fate", "occ_member_fate", "RFC-0202 P0.2 lost-update row: OCC member fate precedence TooOld > Conflict > Ok"),
        on!("occ_batch_plan", "occ_batch_plan", "o plano do grupo decide exatamente pela rota min-len: n e o menor input e todo f"),
        on!("write_admission", "write_admission", "RFC-0213 P0.1 (RFC-0170 P2.4): o gate idle de write-admission e true exatamente "),
        on!("write_admit", "write_admit", "hard-admit verdict fully fated over extracted body: StallMem/StallL0/Ok exactly "),
        on!("wal_sync_required", "wal_sync_required", "RFC-0200 P2.1 data-fate atom: per-commit fdatasync decision is single-valued — c"),
        on!("seq_exhausted", "seq_exhausted", "exhaustion verdict fully fated: seq burned past ceiling exactly on seq > max; as"),
        on!("batch_is_empty", "batch_is_empty", "(twenty-ninth if): the empty-batch disposition — a batch of length n routes thro"),
        on!("fence_on_sync_fail", "fence_on_sync_fail", "fence verdict fully fated: trips exactly when sync required and sync failed; as-"),
        on!("wal_commit_plan", "wal_commit_plan", "WAL append plan fully fated: AppendSyncFence on sync-needed+failed, AppendSyncAp"),
        on!("torn_head_empty_log", "torn_head_empty_log", "torn-head verdict fully fated: head is empty log exactly when len below tiny bou"),
        on!("torn_tail_needs_cut", "torn_tail_needs_cut", "torn-tail verdict fully fated: cut needed exactly when length overhangs last goo"),
        on!("seq_after_feed", "seq_after_feed", "seq-after-feed verdict fully fated: sequence past feed exactly when seq overhang"),
        on!("pit_resync_rewrite", "pit_resync_rewrite", "pit resync verdict fully fated: rewrite needed exactly when entry is a resync; a"),
        on!("dir_sync_required", "dir_sync_required", "(thirtieth if): the dir-sync disposition — a rename/create is followed by a dire"),
        on!("auto_flush_due", "auto_flush_due", "auto-flush verdict fully fated: due exactly when axis armed and bytes reached li"),
        on!("snap_empty", "snap_empty", "snapshot-empty verdict fully fated: empty exactly when snapshot seq is zero; as-"),
        on!("snap_below_watermark", "snap_below_watermark", "below-watermark verdict fully fated: snapshot below watermark exactly when seq o"),
        on!("mem_point_decides", "mem_point_decides", "memtable point verdict fully fated: decision is the hit flag itself; as-is alway"),
        on!("prefer_newer_seq", "prefer_newer_seq", "prefer-newer verdict fully fated: candidate wins exactly when no incumbent or ne"),
        on!("scale_probes", "scale_probes", "probes de point get sao exatamente o saturating_add citado — levels + L0 cobrind"),
        on!("scale_warm", "scale_warm", "teto do WARM e exatamente a cadeia citada — ceiling 0 e o piso 3 GiB; senao max("),
        on!("scale_probes_worst", "scale_probes_worst", "pior caso de producao e exatamente a mesma soma citada — point_get_probes com o "),
        on!("scale_predict", "scale_predict", "o relogio previsto e exatamente a cadeia citada — clamps min(hot,SCALE_BPS)/min("),
        on!("scale_happy_hot", "scale_happy_hot", "fracao quente do caminho feliz e exatamente o gate citado — loja cabendo no warm"),
        on!("scale_forecast", "scale_forecast", "a tabela RFC-0176 e exatamente a composicao citada — cada campo e o atomo do ker"),
        on!("pipeline_drain_cap", "pipeline_drain_cap", "RFC-0222 P0.7 enrollment: pipeline_drain_cap is clamp(queued,1,256) — full drain"),
        on!("async_merge_policy", "async_merge_policy", "RFC-0222 P0.7 enrollment: async_merge_policy merges iff env pin or writers>ncpu "),
        on!("merge_eligible", "merge_eligible", "RFC-0222 P0.7 enrollment: merge_eligible is window_us>0 AND (writers>=2 OR peers"),
        on!("flight_capped_window_us", "flight_capped_window_us", "RFC-0222 P0.7 enrollment: flight_capped_window_us is min(window, flight-or-seed)"),
        on!("herd_collect_us", "herd_collect_us", "A2b: herd_collect_us waits HERD_COLLECT_US iff the frame is not herd_full AND (a"),
        on!("seal_async_first_drain", "seal_async_first_drain", "RFC-0226 P0.2: seal_async_first_drain seals iff !any_sync AND window_us==0 AND w"),
        on!("solo_leader_bypass", "solo_leader_bypass", "RFC-0226 P1.1: solo_leader_bypass is batch_len==1 AND queue_len==0 AND active<=1"),
        on!("herd_full", "herd_full", "A2b: herd_full is batch_len >= HERD_TARGET; AS-IS never full. herd_collect_us an"),
        on!("post_group_grace_us", "post_group_grace_us", "A2b: post_group_grace_us spins after a multi-member publish iff prev_len>=2 AND "),
        on!("leftover_page_advice", "leftover_page_advice", "RFC-0222 P0.7 enrollment: leftover_page_advice drops iff budget=0 AND uncovered "),
        on!("scan_readahead_window", "scan_readahead_window", "RFC-0222 P0.7 enrollment: scan_readahead_window is NONE on a hot store (Fire-118"),
        on!("serial_cs_ns", "serial_cs_ns", "RFC-0222 P0.7 enrollment: serial_cs_ns is the saturating sum of encode+write+gua"),
        on!("spine_replay", "spine_replay", "RFC-0222 P0.7 enrollment: spine_replay folds WriteAckLedger over ANY step sequen"),
        on!("cold_permille", "cold_permille", "RFC-0222 P0.7 enrollment: cold_permille is 0 when the store fits the warm cap, e"),
        on!("product_crown", "product_crown", "RFC-0222 P0.7 enrollment: product_crown folds d1_modelo ∧ (legal cut ⇒ d1_holds)"),
        on!("reserve_frame", "reserve_frame", "RFC-0230 P0.3: reserve_frame returns ticket=reserved_to and advances by len; emp"),
        on!("pwrite_off_lock", "pwrite_off_lock", "RFC-0230 P0.3: pwrite_off_lock is want AND can (env pin AND positional handle). "),
        on!("l0_compact_due", "l0_compact_due", "RFC-0234 P0.1: L0 compact is due EXACTLY when live file count is at/above the tr"),
        on!("workload_class", "workload_class", "RFC-0235 P0.1: Endure class is the unique strict max of (z0,z1,q,W); empty/tie i"),
        on!("filter_partition", "filter_partition", "RFC-0236 P0.1: filter_partition(h1, nparts) is h1 % nparts (0 when nparts<=1). A"),
        // --- contracts without a theorem (published, DST-exercised) ---
        contract!("wal_barrier", "WAL write + fdatasync before Ok (RFC-0001 O1 / RFC-0036) — enforced in code, exercised by the crash/EIO battery"),
        contract!("disk_env", "StdEnv pinned by the verified constructors (Env seam; FailingEnv drives the DST battery)"),
        // --- deliberately off ---
        off!("catchup_window", "pinned to 0 by the verified pin — the merge happens by natural queuing, never by a delay window"),
        off!("async_group_merge", "verified async writes take the write lock themselves (no leader dependency — the pin forces the bypass even under PEDRA_ASYNC_GROUP=1)"),
        off!("io_uring_ring", "no proven ring model (cqe_kernel twin blocked); verified constructors pin StdEnv — the full mode keeps PosixFallback (RFC-0058 P2.2 / RFC-0080)"),
    ]
}

/// Admit a proven io_uring ring model (RFC-0080 / R-uring).
///
/// Always false: there is no probable ring model. Verified constructors
/// pin `StdEnv` / POSIX fallback. AS-IS treats the ring as proven.
#[must_use]
pub fn ring_model_admitted() -> bool {
    false
}

/// AS-IS: a green verified open is rounded to a proven ring (the 0080 hole).
#[must_use]
pub fn ring_model_admitted_as_is() -> bool {
    true
}

/// Admit a live ring backend inside the verified profile.
///
/// Requires both a request for the ring **and** a proven model. Today
/// that is never. AS-IS admits whenever the caller wants the ring.
#[must_use]
pub fn verified_admits_ring(want_ring: bool) -> bool {
    want_ring && ring_model_admitted()
}

/// AS-IS: verified + live ring is fine (WAL back on SQE — the 0080 hole).
#[must_use]
pub fn verified_admits_ring_as_is(want_ring: bool) -> bool {
    want_ring
}

/// RFC-0080 P2.1: a Verus twin of the io_uring ring is not admitted.
/// Always false. RFC-0074 twins `cqe_res_ok` only; no ring model twin.
#[must_use]
pub fn ring_twin_admitted() -> bool {
    false
}

/// AS-IS: the ring looks twin-proven (the 0080 P2.1 hole).
#[must_use]
pub fn ring_twin_admitted_as_is() -> bool {
    true
}

/// RFC-0080 P2.2: production WAL/SST write+sync on SQE submit.
/// Always false. G1 stays POSIX `pwrite` / `fdatasync`.
#[must_use]
pub fn wal_on_sqe_admitted() -> bool {
    false
}

/// AS-IS: WAL is rounded back onto the ring (the 0062 / 0080 hole).
#[must_use]
pub fn wal_on_sqe_admitted_as_is() -> bool {
    true
}

/// The declared composition (RFC-0058 P0.1 + P2.1).
///
/// Use [`Self::open`] / [`Self::open_with_env`] for the whole profile
/// (file options + the verified group pin: merge decided by the proved
/// `group_commit_kernel`, catch-up window 0, async bypass).
/// [`Self::open_options`] is the file-level half alone.
pub struct VerifiedProfile;

impl VerifiedProfile {
    /// File-level composition: `sync=true`, strongest WAL data class,
    /// fail-closed recovery.
    #[must_use]
    pub fn open_options() -> OpenOptions {
        OpenOptions {
            sync: true,
            wal_full_fsync: true,
            wal_recovery: WalRecovery::FailClosed,
            ..OpenOptions::default()
        }
    }

    /// Open on the real filesystem (`StdEnv`) with the full profile:
    /// file options + the verified group pin.
    ///
    /// # Errors
    /// Same as [`ConcurrentDb::open_with`].
    pub fn open(path: impl AsRef<Path>) -> Result<ConcurrentDb<StdEnv>> {
        ConcurrentDb::open_verified(path)
    }

    /// Open with an explicit [`Env`] (DST wraps `FailingEnv` here) and the
    /// full profile.
    ///
    /// # Errors
    /// Same as [`ConcurrentDb::open_with_env`].
    pub fn open_with_env<E: Env>(path: impl AsRef<Path>, env: E) -> Result<ConcurrentDb<E>> {
        let db = ConcurrentDb::open_with_env(path, Self::open_options(), env)?;
        db.pin_verified();
        Ok(db)
    }
}

impl OpenOptions {
    /// File-level composition of the verified profile (RFC-0058 P0.1):
    /// `sync=true`, `wal_full_fsync=true`, `wal_recovery=FailClosed`.
    ///
    /// The group-level half (RFC-0058 P2.1: merge decided by the proved
    /// `group_commit_kernel`, catch-up window 0, async bypass) is a
    /// runtime policy — pin it with
    /// [`ConcurrentDb::pin_verified`](crate::ConcurrentDb::pin_verified)
    /// or open through [`VerifiedProfile::open`] /
    /// [`ConcurrentDb::open_verified`], which do both. The io_uring ring
    /// stays outside the mode (P2.2 gate: no proven ring model — open
    /// with [`StdEnv`](crate::StdEnv), as `PEDRA_VERIFIED=1` does).
    #[must_use]
    pub fn verified() -> Self {
        VerifiedProfile::open_options()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract the string values of every `"id":` key inside the `pairs`
    /// array of the catalog JSON (tolerant of whitespace; no JSON
    /// dependency in the core dev graph).
    fn catalog_ids() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/formal/catalog.json"
        );
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("catalog.json unreadable ({e}) — run from the repo"));
        let pairs_body = text
            .split("\"pairs\"")
            .nth(1)
            .and_then(|rest| rest.split("\"clones\"").next())
            .expect("catalog.json must have a pairs array before clones");
        let mut ids = Vec::new();
        let mut rest = pairs_body;
        while let Some(pos) = rest.find("\"id\"") {
            rest = &rest[pos + 4..];
            let after_colon = rest.trim_start().strip_prefix(':').unwrap_or(rest);
            let quoted = after_colon
                .trim_start()
                .strip_prefix('"')
                .unwrap_or(after_colon);
            if let Some(end) = quoted.find('"') {
                ids.push(quoted[..end].to_string());
                rest = &quoted[end..];
            }
        }
        ids
    }

    /// RFC-0058 P0.1 invariant: the ON kernels of the report are exactly
    /// the catalog `pairs` ids (well — every `"id"` in the file; clones
    /// and models are supersets that keep the existence check honest).
    /// A new catalog kernel must be claimed here in the same change.
    #[test]
    fn verified_report_matches_catalog() {
        let catalog = catalog_ids();
        assert!(catalog.len() >= 44, "catalog shrank? ids: {catalog:?}");
        let reported: std::collections::HashSet<&str> = profile_report()
            .iter()
            .filter(|c| c.state == ProfileState::On)
            .filter_map(|c| c.kernel)
            .collect();
        for id in &catalog {
            assert!(
                reported.contains(id.as_str()),
                "catalog kernel {id} not claimed by the verified report — claim it (On) or gate it (Off) in the same change"
            );
        }
        let catalog_set: std::collections::HashSet<&str> =
            catalog.iter().map(String::as_str).collect();
        for k in &reported {
            assert!(
                catalog_set.contains(k),
                "report cites kernel {k} that is not in the catalog"
            );
        }
        // The mode's differentiators are explicit: the merge is ON with
        // the proved kernel; the delay window, the async leader
        // dependency and the io_uring ring stay OFF.
        let c = profile_report()
            .iter()
            .find(|c| c.component == "write_group_merge")
            .unwrap_or_else(|| panic!("missing report row write_group_merge"));
        assert_eq!(c.state, ProfileState::On, "{c:?}");
        assert_eq!(c.kernel, Some("group_commit"));
        for name in ["io_uring_ring", "catchup_window", "async_group_merge"] {
            let c = profile_report()
                .iter()
                .find(|c| c.component == name)
                .unwrap_or_else(|| panic!("missing report row {name}"));
            assert_eq!(c.state, ProfileState::Off, "{name}: {c:?}");
        }
        for k in [
            "d1_durability",
            "d1_modelo",
            "r1_no_resurrection",
            "r1_modelo",
            "t1_atomicity",
            "t1_modelo",
            "c1_quorum",
            "c1_modelo",
        ] {
            assert!(
                reported.contains(k),
                "RFC-0166 P2.4: property/refinement {k} must be an ON report row"
            );
        }
        let ring = profile_report()
            .iter()
            .find(|c| c.component == "io_uring_ring")
            .unwrap();
        assert_eq!(
            ring.state == ProfileState::On,
            ring_model_admitted(),
            "io_uring_ring On/Off must track ring_model_admitted"
        );
    }

    #[test]
    fn ring_model_is_not_admitted() {
        assert!(!ring_model_admitted());
        assert!(
            ring_model_admitted_as_is(),
            "AS-IS dente: ring looks proven"
        );
        assert!(!verified_admits_ring(true));
        assert!(!verified_admits_ring(false));
        assert!(
            verified_admits_ring_as_is(true),
            "AS-IS dente: verified would take a live ring"
        );
        assert!(!verified_admits_ring_as_is(false));
        assert!(!ring_twin_admitted());
        assert!(
            ring_twin_admitted_as_is(),
            "AS-IS dente: ring twin looks proven"
        );
        assert!(!wal_on_sqe_admitted());
        assert!(wal_on_sqe_admitted_as_is(), "AS-IS dente: WAL back on SQE");
        let twin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("verus/ring_model.rs");
        assert!(
            !twin.exists(),
            "RFC-0080 P2.1: ring Verus twin must stay absent ({})",
            twin.display()
        );
    }

    /// P0.1: the options half of the composition is fixed and fail-closed.
    #[test]
    fn verified_open_options_are_fixed() {
        let o = OpenOptions::verified();
        assert!(o.sync);
        assert!(o.wal_full_fsync);
        assert_eq!(o.wal_recovery, WalRecovery::FailClosed);
        let p = VerifiedProfile::open_options();
        assert!(p.sync && p.wal_full_fsync);
        assert_eq!(p.wal_recovery, WalRecovery::FailClosed);
    }

    /// Same TX the CLI `demo` runs under `PEDRA_VERIFIED=1` (`StdEnv` +
    /// verified options). Pins the shipped commit path, not a copy.
    #[test]
    fn verified_std_env_demo_tx_roundtrip() {
        use crate::db::Db;
        use crate::StdEnv;
        let dir =
            std::env::temp_dir().join(format!("pedradb-verified-demo-tx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = Db::open_with_env(&dir, OpenOptions::verified(), StdEnv).unwrap();
        {
            let mut tx = db.begin();
            tx.put(b"u/1", br#"{"name":"ada"}"#).unwrap();
            tx.put(b"idx/name/ada", b"1").unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(
            db.get(b"u/1").as_deref(),
            Some(br#"{"name":"ada"}"#.as_ref())
        );
        db.close().unwrap();
        let db2 = Db::open_with_env(&dir, OpenOptions::verified(), StdEnv).unwrap();
        assert_eq!(
            db2.get(b"u/1").as_deref(),
            Some(br#"{"name":"ada"}"#.as_ref())
        );
        db2.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
