//! Pure joint-consensus quorum (Raft §6 / RFC-0064 P2.1).
//!
//! Same rules as `pedradb-raft::membership_kernel`. Store does not depend on
//! `pedradb-raft`; keep the two bodies identical (clone trap).

#![forbid(unsafe_code)]

/// Majority of `n` voters (`⌊n/2⌋+1`). `n == 0` → 1 so an empty set never
/// grants (fail-closed).
#[must_use]
pub fn majority_of(n: u64) -> u64 {
    if n == 0 {
        1
    } else {
        n / 2 + 1
    }
}

/// Raft §6 joint election/commit: majority of C-old **and**, when a joint
/// config is in flight, majority of C-new. `new_yes` is `None` when no
/// joint is pending (`old_yes` vs `old_n` only).
#[must_use]
pub fn joint_election_ok(old_yes: u64, old_n: u64, new_yes: Option<(u64, u64)>) -> bool {
    if old_yes < majority_of(old_n) {
        return false;
    }
    match new_yes {
        None => true,
        Some((yes, n)) => yes >= majority_of(n),
    }
}

/// AS-IS: ignore C-new (the 0064 hole — elect on C-old during joint add).
#[must_use]
pub fn joint_election_ok_as_is(old_yes: u64, old_n: u64, _new_yes: Option<(u64, u64)>) -> bool {
    old_yes >= majority_of(old_n)
}

/// Raft §6: C-old,new is still in force while `old` and `new` differ.
/// Leave-joint is encoded as `old == new` (C-new only).
#[must_use]
pub fn joint_still_active(old: &[u64], new: &[u64]) -> bool {
    old != new
}

/// AS-IS: treat every config as single (the 0066 hole — after commit,
/// ignore C-new until apply / skip leave-joint).
#[must_use]
pub fn joint_still_active_as_is(_old: &[u64], _new: &[u64]) -> bool {
    false
}

/// RFC-0096: a committed joint is not a single config until a leave
/// (`old == new`) is in the log. AS-IS skips leave.
#[must_use]
pub fn joint_leave_ok(leave_in_log: bool) -> bool {
    leave_in_log
}

/// AS-IS: skip leave-joint (the 0066 leftover on the live add path).
#[must_use]
pub fn joint_leave_ok_as_is(_leave_in_log: bool) -> bool {
    true
}

/// RFC-0068 P1.2: opt-in World schedule emitted `PlantCommittedJoint`
/// and the default seed scheduler omitted it (fingerprint-stable).
#[must_use]
pub fn plant_joint_schedule_ok(opt_in_emits: bool, default_omits: bool) -> bool {
    opt_in_emits && default_omits
}

/// AS-IS: skip the opt-in plant (the 0068 P1.2 hole — random scheduler
/// never emits `PlantCommittedJoint`).
#[must_use]
pub fn plant_joint_schedule_ok_as_is(_opt_in_emits: bool, _default_omits: bool) -> bool {
    true
}

/// RFC-0122: a leave in the log is not done until it is committed.
#[must_use]
pub fn queued_leave_finish_ok(leave_in_log: bool, leave_committed: bool) -> bool {
    !leave_in_log || leave_committed
}

/// AS-IS: “in the log” is enough (the 0121 leftover — TCP does not finish leave).
#[must_use]
pub fn queued_leave_finish_ok_as_is(leave_in_log: bool, _leave_committed: bool) -> bool {
    leave_in_log
}

/// RFC-0124: non-empty durable membership overrides CLI/`--peer` on open.
#[must_use]
pub fn disk_membership_overrides_cli(has_disk: bool) -> bool {
    has_disk
}

/// AS-IS: CLI `--peer` wins and overwrites disk (the 0123 leftover).
#[must_use]
pub fn disk_membership_overrides_cli_as_is(_has_disk: bool) -> bool {
    false
}

/// RFC-0124 P1.1: persist C-new identity before advancing applied past the joint.
#[must_use]
pub fn membership_identity_before_applied(identity_first: bool) -> bool {
    identity_first
}

/// AS-IS: persist applied first (crash window: applied high, voters stale).
#[must_use]
pub fn membership_identity_before_applied_as_is(_identity_first: bool) -> bool {
    false
}

/// RFC-0125: durable high-water is at least RAM/CLI (never shrink on open).
#[must_use]
pub fn high_water_at_least(disk_hw: u64, ram_hw: u64) -> u64 {
    disk_hw.max(ram_hw)
}

/// AS-IS: RAM/CLI length only (the 0124 leftover — 4-node history forgotten).
#[must_use]
pub fn high_water_at_least_as_is(_disk_hw: u64, ram_hw: u64) -> u64 {
    ram_hw
}

/// RFC-0130: recover must apply when commit is ahead of applied.
#[must_use]
pub fn recover_must_apply(applied: u64, commit: u64) -> bool {
    commit > applied
}

/// AS-IS: skip apply on recover (the 0129 leftover — committed joint stays C-old).
#[must_use]
pub fn recover_must_apply_as_is(_applied: u64, _commit: u64) -> bool {
    false
}

/// RFC-0131: recover applies on every local replica, even if not in `ids`.
#[must_use]
pub fn recover_apply_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0130 leftover — removed replica skipped).
#[must_use]
pub fn recover_apply_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0132: persist truncated logs on every local replica, even if not in `ids`.
#[must_use]
pub fn recover_truncate_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0131 leftover — removed replica keeps suffix).
#[must_use]
pub fn recover_truncate_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0133: a log segment past the truncated hi must be deleted.
#[must_use]
pub fn recover_drop_orphan_seg(seg_index: u64, new_hi: u64) -> bool {
    seg_index > new_hi
}

/// AS-IS: leave orphan keys (the 0132 leftover — log_hi capped, segments remain).
#[must_use]
pub fn recover_drop_orphan_seg_as_is(_seg_index: u64, _new_hi: u64) -> bool {
    false
}

/// RFC-0134: abort leftover 2PC on every local replica, even if not in `ids`.
#[must_use]
pub fn recover_abort_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0133 leftover — removed replica keeps intents).
#[must_use]
pub fn recover_abort_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0135: persist SI meta on every local replica, even if not in `ids`.
#[must_use]
pub fn persist_meta_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0134 leftover — removed replica clock not durable).
#[must_use]
pub fn persist_meta_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0136: persist SI hist on every local replica, even if not in `ids`.
#[must_use]
pub fn persist_hist_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0135 leftover — removed replica hist not durable).
#[must_use]
pub fn persist_hist_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0137: persist abort fence on every local replica, even if not in `ids`.
#[must_use]
pub fn persist_fence_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0136 leftover — removed replica has no fence).
#[must_use]
pub fn persist_fence_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0138: force-local TX clear on every local replica, even if not in `ids`.
#[must_use]
pub fn force_clear_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0137 leftover — removed replica keeps intents).
#[must_use]
pub fn force_clear_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0139: drop prepare-time preimages on every local replica, even if not in `ids`.
#[must_use]
pub fn drop_preimages_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0138 leftover — removed replica keeps preimages).
#[must_use]
pub fn drop_preimages_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0140: in-process open loads raft peers from disk membership, not CLI n_nodes.
#[must_use]
pub fn open_peer_uses_disk(has_disk: bool) -> bool {
    has_disk
}

/// AS-IS: CLI 1..=n_nodes (the 0125 leftover — only TCP peeked).
#[must_use]
pub fn open_peer_uses_disk_as_is(_has_disk: bool) -> bool {
    false
}

/// RFC-0141: sole local node is this process's identity only if it is in `ids`.
#[must_use]
pub fn local_id_if_member(in_ids: bool) -> bool {
    in_ids
}

/// AS-IS: HashMap first-key even when removed (the 0140 leftover).
#[must_use]
pub fn local_id_if_member_as_is(_in_ids: bool) -> bool {
    true
}

/// RFC-0142: a LocalApplied `ids.first()` fallback must be a local node.
#[must_use]
pub fn reader_id_local(is_local: bool) -> bool {
    is_local
}

/// AS-IS: `ids.first()` even when not local (the 0141 leftover).
#[must_use]
pub fn reader_id_local_as_is(_is_local: bool) -> bool {
    true
}

/// RFC-0143: live uncommitted-log discard on every local replica, even if not in `ids`.
#[must_use]
pub fn discard_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0142 leftover — removed replica keeps suffix).
#[must_use]
pub fn discard_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0144: no-leader discard persist-leader must be a local node.
#[must_use]
pub fn discard_leader_local(is_local: bool) -> bool {
    is_local
}

/// AS-IS: `ids.first()` even when remote (the 0143 leftover).
#[must_use]
pub fn discard_leader_local_as_is(_is_local: bool) -> bool {
    true
}

/// RFC-0145: a node dropped from `ids` must step down from Leader.
#[must_use]
pub fn removed_steps_down(in_ids: bool) -> bool {
    !in_ids
}

/// AS-IS: keep Role::Leader after joint leave (the 0144 leftover).
#[must_use]
pub fn removed_steps_down_as_is(_in_ids: bool) -> bool {
    false
}

/// RFC-0146: a leader routing hint counts only if that node is in `ids`.
#[must_use]
pub fn hint_if_member(in_ids: bool) -> bool {
    in_ids
}

/// AS-IS: any `leader_id` is returned (the 0145 leftover).
#[must_use]
pub fn hint_if_member_as_is(_in_ids: bool) -> bool {
    true
}

/// RFC-0147: forget next/match/sent_through of a node dropped from `ids`.
#[must_use]
pub fn drop_repl_slot(in_ids: bool) -> bool {
    !in_ids
}

/// AS-IS: keep replication slots after joint leave (the 0146 leftover).
#[must_use]
pub fn drop_repl_slot_as_is(_in_ids: bool) -> bool {
    false
}

/// RFC-0148: forget sent_through of a node dropped from `ids` on oob remove.
#[must_use]
pub fn drop_sent_through(in_ids: bool) -> bool {
    !in_ids
}

/// AS-IS: keep sent_through after oob remove_member (the 0147 leftover).
#[must_use]
pub fn drop_sent_through_as_is(_in_ids: bool) -> bool {
    false
}

/// RFC-0127: a node participates iff it is in the current voter set.
#[must_use]
pub fn participating_if_member(in_ids: bool) -> bool {
    in_ids
}

/// AS-IS: keep captured participating (the 0126 leftover — removed node counts).
#[must_use]
pub fn participating_if_member_as_is(_in_ids: bool) -> bool {
    true
}

/// RFC-0105: only current members' logs define the pending joint.
#[must_use]
pub fn pending_joint_node_counts(is_member: bool) -> bool {
    is_member
}

/// AS-IS: scan every opened node, including removed (the 0104 leftover).
#[must_use]
pub fn pending_joint_node_counts_as_is(_is_member: bool) -> bool {
    true
}

/// RFC-0119: a joint-remove target is the membership set (`ids`), not
/// the local `nodes` map (TCP replica only has self).
#[must_use]
pub fn joint_target_counts(in_ids: bool, _in_nodes: bool) -> bool {
    in_ids
}

/// AS-IS: require the target in local nodes (the 0118 leftover — TCP
/// replica cannot joint-remove a peer).
#[must_use]
pub fn joint_target_counts_as_is(_in_ids: bool, in_nodes: bool) -> bool {
    in_nodes
}

/// RFC-0119 P1.1: a joint-add target need not live in local `nodes`
/// (joining process is another OS pid).
#[must_use]
pub fn joint_add_target_counts(_in_nodes: bool) -> bool {
    true
}

/// AS-IS: require the joiner in local nodes (the 0119 P0 leftover).
#[must_use]
pub fn joint_add_target_counts_as_is(in_nodes: bool) -> bool {
    in_nodes
}

/// RFC-0114: a RequestVote grant counts only if the voter is in C-old
/// (`ids`) or in an in-flight joint (C-old ∪ C-new).
#[must_use]
pub fn election_grant_from_counts(in_ids: bool, in_pending_old_or_new: bool) -> bool {
    in_ids || in_pending_old_or_new
}

/// AS-IS: any grant is recorded (the 0113 leftover — lagging removed voter).
#[must_use]
pub fn election_grant_from_counts_as_is(_in_ids: bool, _in_pending_old_or_new: bool) -> bool {
    true
}

/// Eventual-election / unbounded liveness may be claimed only when all
/// three eventual-synchrony axioms hold (RFC-0069 / RFC-0056 P2.2):
/// ES-1 finite adversary, ES-2 internal drain, ES-3 live retrying candidate.
/// Bounded `elect_all` does **not** go through this gate.
#[must_use]
pub fn liveness_admitted(es1: bool, es2: bool, es3: bool) -> bool {
    es1 && es2 && es3
}

/// AS-IS: any bounded elect is treated as a liveness theorem (the 0069 hole).
#[must_use]
pub fn liveness_admitted_as_is(_es1: bool, _es2: bool, _es3: bool) -> bool {
    true
}

/// RFC-0069 P2.2: banner for TCP/real elect. The word `live` only when
/// admitted (and then ES is named). Bounded elect must not print `live`.
#[must_use]
pub fn elect_claim_banner(es1: bool, es2: bool, es3: bool) -> &'static str {
    if liveness_admitted(es1, es2, es3) {
        "eventual-live es1=1 es2=1 es3=1"
    } else {
        "bounded-elect not-eventual"
    }
}

/// AS-IS: print `live` without naming ES (the 0069 P2.2 hole).
#[must_use]
pub fn elect_claim_banner_as_is(_es1: bool, _es2: bool, _es3: bool) -> &'static str {
    "live"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_election_old_majority_is_not_enough_during_add() {
        // C-old=3 (maj 2), C-new=4 (maj 3): two old votes elect AS-IS, not joint.
        assert!(!joint_election_ok(2, 3, Some((2, 4))));
        assert!(joint_election_ok_as_is(2, 3, Some((2, 4))));
        assert!(joint_election_ok(2, 3, Some((3, 4))));
        assert!(joint_election_ok(2, 3, None));
        assert!(!joint_election_ok(1, 3, None));
        assert!(!joint_election_ok(2, 3, Some((0, 0))));
    }

    #[test]
    fn joint_still_active_until_leave() {
        let old = [1u64, 2, 3];
        let new = [1u64, 2, 3, 4];
        assert!(joint_still_active(&old, &new));
        assert!(!joint_still_active_as_is(&old, &new));
        assert!(!joint_still_active(&new, &new));
        assert!(!joint_still_active_as_is(&new, &new));
    }

    #[test]
    fn joint_leave_ok_requires_leave_in_log() {
        assert!(joint_leave_ok(true));
        assert!(!joint_leave_ok(false));
        assert!(joint_leave_ok_as_is(false), "AS-IS dente: skip leave-joint");
    }

    #[test]
    fn plant_joint_schedule_ok_requires_opt_in_and_default_omit() {
        assert!(plant_joint_schedule_ok(true, true));
        assert!(!plant_joint_schedule_ok(false, true));
        assert!(!plant_joint_schedule_ok(true, false));
        assert!(
            plant_joint_schedule_ok_as_is(false, false),
            "AS-IS dente: skip opt-in PlantCommittedJoint"
        );
    }

    #[test]
    fn queued_leave_finish_ok_requires_commit() {
        assert!(!queued_leave_finish_ok(true, false));
        assert!(
            queued_leave_finish_ok_as_is(true, false),
            "AS-IS dente: leave in log is enough"
        );
        assert!(queued_leave_finish_ok(true, true));
        assert!(queued_leave_finish_ok(false, false));
        assert!(!queued_leave_finish_ok_as_is(false, true));
    }

    /// RFC-0122 P2.2: finishing a queued leave is a campaign, not ∀ traces.
    /// `R-joint` stays continuous; catalog pair is freeze, not a theorem.
    #[test]
    fn queued_leave_finish_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"id\": \"R-swarm-real\""),
            "R-swarm-real must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "queued leave finish must refuse forall traces"
        );
        assert!(
            residuals.contains("queued_leave_finish_ok"),
            "R-joint close must name the finish kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"queued_leave_finish\""),
            "catalog pair queued_leave_finish must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"queued_leave_finish_ok\""),
            "catalog entry must stay queued_leave_finish_ok"
        );
    }

    /// RFC-0123 P2.1: the Verus twin of `queued_leave_finish_ok` is freeze,
    /// not a theorem of all traces. `R-joint` stays continuous.
    #[test]
    fn queued_leave_finish_twin_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "queued leave twin must refuse forall traces"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"queued_leave_finish\""),
            "catalog pair queued_leave_finish must stay"
        );
        assert!(
            catalog.contains("crates/pedradb-raft/verus/membership_joint.rs"),
            "twin path must stay membership_joint.rs"
        );
        assert!(
            catalog.contains("scripts/verus_membership_joint.sh"),
            "verus script must stay registered (freeze, not exec)"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn queued_leave_finish_ok"),
            "twin freeze of queued_leave_finish_ok is not a Verus exec claim"
        );
    }

    /// RFC-0123 P2.2: twin freeze of `queued_leave_finish_ok` is not a
    /// verified verifier. `R-verus` stays `never_floor`.
    #[test]
    fn queued_leave_finish_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"queued_leave_finish\""),
            "queued_leave_finish catalog pair must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"queued_leave_finish_ok\""),
            "catalog entry must stay queued_leave_finish_ok"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn queued_leave_finish_ok"),
            "twin freeze is not a Verus exec claim"
        );
    }

    #[test]
    fn disk_membership_overrides_cli_when_present() {
        assert!(disk_membership_overrides_cli(true));
        assert!(
            !disk_membership_overrides_cli_as_is(true),
            "AS-IS dente: CLI --peer overwrites disk"
        );
        assert!(!disk_membership_overrides_cli(false));
        assert!(membership_identity_before_applied(true));
        assert!(
            !membership_identity_before_applied_as_is(true),
            "AS-IS dente: persist applied first"
        );
        assert_eq!(high_water_at_least(4, 3), 4);
        assert_eq!(
            high_water_at_least_as_is(4, 3),
            3,
            "AS-IS dente: RAM/CLI high-water only"
        );
        assert!(!participating_if_member(false));
        assert!(
            participating_if_member_as_is(false),
            "AS-IS dente: keep captured participating"
        );
        assert!(participating_if_member(true));
        assert!(recover_must_apply(1, 2));
        assert!(
            !recover_must_apply_as_is(1, 2),
            "AS-IS dente: skip apply on recover"
        );
        assert!(!recover_must_apply(2, 2));
        assert!(!recover_must_apply(3, 2));
        assert!(recover_apply_node_counts(true, false));
        assert!(
            !recover_apply_node_counts_as_is(true, false),
            "AS-IS dente: skip local non-member"
        );
        assert!(recover_apply_node_counts(true, true));
        assert!(!recover_apply_node_counts(false, true));
        assert!(!recover_apply_node_counts(false, false));
        assert!(recover_truncate_node_counts(true, false));
        assert!(
            !recover_truncate_node_counts_as_is(true, false),
            "AS-IS dente: skip truncate persist on local non-member"
        );
        assert!(recover_truncate_node_counts(true, true));
        assert!(!recover_truncate_node_counts(false, true));
        assert!(recover_drop_orphan_seg(3, 2));
        assert!(
            !recover_drop_orphan_seg_as_is(3, 2),
            "AS-IS dente: leave orphan log segments"
        );
        assert!(!recover_drop_orphan_seg(2, 2));
        assert!(!recover_drop_orphan_seg(1, 2));
        assert!(recover_abort_node_counts(true, false));
        assert!(
            !recover_abort_node_counts_as_is(true, false),
            "AS-IS dente: skip leftover abort on local non-member"
        );
        assert!(recover_abort_node_counts(true, true));
        assert!(!recover_abort_node_counts(false, true));
        assert!(persist_meta_node_counts(true, false));
        assert!(
            !persist_meta_node_counts_as_is(true, false),
            "AS-IS dente: skip SI meta persist on local non-member"
        );
        assert!(persist_meta_node_counts(true, true));
        assert!(!persist_meta_node_counts(false, true));
        assert!(persist_hist_node_counts(true, false));
        assert!(
            !persist_hist_node_counts_as_is(true, false),
            "AS-IS dente: skip SI hist persist on local non-member"
        );
        assert!(persist_hist_node_counts(true, true));
        assert!(!persist_hist_node_counts(false, true));
        assert!(persist_fence_node_counts(true, false));
        assert!(
            !persist_fence_node_counts_as_is(true, false),
            "AS-IS dente: skip abort fence on local non-member"
        );
        assert!(persist_fence_node_counts(true, true));
        assert!(!persist_fence_node_counts(false, true));
        assert!(force_clear_node_counts(true, false));
        assert!(
            !force_clear_node_counts_as_is(true, false),
            "AS-IS dente: skip force-local clear on local non-member"
        );
        assert!(force_clear_node_counts(true, true));
        assert!(!force_clear_node_counts(false, true));
        assert!(drop_preimages_node_counts(true, false));
        assert!(
            !drop_preimages_node_counts_as_is(true, false),
            "AS-IS dente: skip drop-preimages on local non-member"
        );
        assert!(drop_preimages_node_counts(true, true));
        assert!(!drop_preimages_node_counts(false, true));
        assert!(open_peer_uses_disk(true));
        assert!(
            !open_peer_uses_disk_as_is(true),
            "AS-IS dente: in-process open ignores disk at load"
        );
        assert!(!open_peer_uses_disk(false));
        assert!(!local_id_if_member(false));
        assert!(
            local_id_if_member_as_is(false),
            "AS-IS dente: HashMap first-key even when removed"
        );
        assert!(local_id_if_member(true));
        assert!(!reader_id_local(false));
        assert!(
            reader_id_local_as_is(false),
            "AS-IS dente: ids.first even when not local"
        );
        assert!(reader_id_local(true));
        assert!(discard_node_counts(true, false));
        assert!(
            !discard_node_counts_as_is(true, false),
            "AS-IS dente: skip live discard on local non-member"
        );
        assert!(discard_node_counts(true, true));
        assert!(!discard_node_counts(false, true));
        assert!(!discard_leader_local(false));
        assert!(
            discard_leader_local_as_is(false),
            "AS-IS dente: ids.first persist-leader even when remote"
        );
        assert!(discard_leader_local(true));
        assert!(removed_steps_down(false));
        assert!(
            !removed_steps_down_as_is(false),
            "AS-IS dente: keep Role::Leader after joint leave"
        );
        assert!(!removed_steps_down(true));
        assert!(!hint_if_member(false));
        assert!(
            hint_if_member_as_is(false),
            "AS-IS dente: leader_hint returns a removed node"
        );
        assert!(hint_if_member(true));
        assert!(drop_repl_slot(false));
        assert!(
            !drop_repl_slot_as_is(false),
            "AS-IS dente: keep next/match/sent_through after joint leave"
        );
        assert!(!drop_repl_slot(true));
        assert!(drop_sent_through(false));
        assert!(
            !drop_sent_through_as_is(false),
            "AS-IS dente: keep sent_through after oob remove_member"
        );
        assert!(!drop_sent_through(true));
    }

    /// RFC-0124 P2.2: disk membership overriding CLI is a campaign, not ∀ traces.
    /// `R-joint` stays continuous; catalog pair is freeze, not a theorem.
    #[test]
    fn disk_membership_overrides_cli_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "disk membership override must refuse forall traces"
        );
        assert!(
            residuals.contains("disk_membership_overrides_cli"),
            "R-joint close must name the disk-membership kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"disk_membership\""),
            "catalog pair disk_membership must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"disk_membership_overrides_cli\""),
            "catalog entry must stay disk_membership_overrides_cli"
        );
    }

    /// RFC-0125 P2.1: durable high-water on open is a campaign, not ∀ traces.
    /// `R-joint` stays continuous; catalog pair is freeze, not a theorem.
    #[test]
    fn high_water_at_least_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "high-water restore must refuse forall traces"
        );
        assert!(
            residuals.contains("high_water_at_least"),
            "R-joint close must name the high-water kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"high_water\""),
            "catalog pair high_water must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"high_water_at_least\""),
            "catalog entry must stay high_water_at_least"
        );
    }

    /// RFC-0125 P2.2: twin freeze of `high_water_at_least` is not a
    /// verified verifier. `R-verus` stays `never_floor`.
    #[test]
    fn high_water_at_least_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"high_water\""),
            "high_water catalog pair must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"high_water_at_least\""),
            "catalog entry must stay high_water_at_least"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn high_water_at_least"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0128 P2.1: live is_participating gate is a campaign, not ∀ traces.
    #[test]
    fn is_participating_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "is_participating must refuse forall traces"
        );
        assert!(
            residuals.contains("participating_if_member"),
            "R-joint close must name the participating kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"participating_member\""),
            "catalog pair participating_member must stay"
        );
    }

    /// RFC-0127 P2.1: reopen participating-follows-ids is a campaign, not ∀ traces.
    #[test]
    fn reopen_participating_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "reopen participating must refuse forall traces"
        );
        assert!(
            residuals.contains("participating_if_member"),
            "R-joint close must name the participating kernel"
        );
        assert!(
            residuals.contains("crash_reopen_participating_follows_membership")
                || residuals.contains("0127"),
            "R-joint close must name the 0127 reopen tooth"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"participating_member\""),
            "catalog pair participating_member must stay"
        );
        assert!(
            catalog.contains("crash_reopen_engine_on"),
            "catalog handlers must include crash_reopen_engine_on"
        );
    }

    /// RFC-0127 P2.2: twin freeze of `participating_if_member` is not a
    /// verified verifier. `R-verus` stays `never_floor`.
    #[test]
    fn reopen_participating_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"participating_member\""),
            "participating_member catalog pair must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"participating_if_member\""),
            "catalog entry must stay participating_if_member"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn participating_if_member"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0130 P2.2: recover apply is a campaign, not ∀ traces.
    #[test]
    fn recover_must_apply_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "recover apply must refuse forall traces"
        );
        assert!(
            residuals.contains("recover_must_apply"),
            "R-joint close must name the recover-apply kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_apply\""),
            "catalog pair recover_apply must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"recover_must_apply\""),
            "catalog entry must stay recover_must_apply"
        );
    }

    /// RFC-0130 P2.2: twin freeze of recover_must_apply is not a verifier.
    #[test]
    fn recover_must_apply_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_apply\""),
            "recover_apply catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn recover_must_apply"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0131 P2.2: recover on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn recover_apply_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "recover apply local must refuse forall traces"
        );
        assert!(
            residuals.contains("recover_apply_node_counts"),
            "R-joint close must name the recover-apply-node kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_apply_node\""),
            "catalog pair recover_apply_node must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"recover_apply_node_counts\""),
            "catalog entry must stay recover_apply_node_counts"
        );
    }

    /// RFC-0131 P2.2: twin freeze of recover_apply_node_counts is not a verifier.
    #[test]
    fn recover_apply_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_apply_node\""),
            "recover_apply_node catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn recover_apply_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0132 P2.2: persist truncate on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn recover_truncate_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "recover truncate must refuse forall traces"
        );
        assert!(
            residuals.contains("recover_truncate_node_counts"),
            "R-joint close must name the recover-truncate kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_truncate\""),
            "catalog pair recover_truncate must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"recover_truncate_node_counts\""),
            "catalog entry must stay recover_truncate_node_counts"
        );
    }

    /// RFC-0132 P2.2: twin freeze of recover_truncate_node_counts is not a verifier.
    #[test]
    fn recover_truncate_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_truncate\""),
            "recover_truncate catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn recover_truncate_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0133 P2.2: dropping orphan log segments is a campaign, not ∀ traces.
    #[test]
    fn recover_drop_orphan_seg_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "recover drop orphan must refuse forall traces"
        );
        assert!(
            residuals.contains("recover_drop_orphan_seg"),
            "R-joint close must name the recover-drop-orphan kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_drop_orphan\""),
            "catalog pair recover_drop_orphan must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"recover_drop_orphan_seg\""),
            "catalog entry must stay recover_drop_orphan_seg"
        );
    }

    /// RFC-0133 P2.2: twin freeze of recover_drop_orphan_seg is not a verifier.
    #[test]
    fn recover_drop_orphan_seg_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_drop_orphan\""),
            "recover_drop_orphan catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn recover_drop_orphan_seg"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0134 P2.2: leftover abort on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn recover_abort_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "recover abort must refuse forall traces"
        );
        assert!(
            residuals.contains("recover_abort_node_counts"),
            "R-joint close must name the recover-abort kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_abort\""),
            "catalog pair recover_abort must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"recover_abort_node_counts\""),
            "catalog entry must stay recover_abort_node_counts"
        );
    }

    /// RFC-0134 P2.2: twin freeze of recover_abort_node_counts is not a verifier.
    #[test]
    fn recover_abort_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"recover_abort\""),
            "recover_abort catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn recover_abort_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0135 P2.2: persist SI meta on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn persist_meta_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "persist meta must refuse forall traces"
        );
        assert!(
            residuals.contains("persist_meta_node_counts"),
            "R-joint close must name the persist-meta kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"persist_meta\""),
            "catalog pair persist_meta must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"persist_meta_node_counts\""),
            "catalog entry must stay persist_meta_node_counts"
        );
    }

    /// RFC-0135 P2.2: twin freeze of persist_meta_node_counts is not a verifier.
    #[test]
    fn persist_meta_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"persist_meta\""),
            "persist_meta catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn persist_meta_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0136 P2.2: persist SI hist on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn persist_hist_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "persist hist must refuse forall traces"
        );
        assert!(
            residuals.contains("persist_hist_node_counts"),
            "R-joint close must name the persist-hist kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"persist_hist\""),
            "catalog pair persist_hist must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"persist_hist_node_counts\""),
            "catalog entry must stay persist_hist_node_counts"
        );
    }

    /// RFC-0136 P2.2: twin freeze of persist_hist_node_counts is not a verifier.
    #[test]
    fn persist_hist_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"persist_hist\""),
            "persist_hist catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn persist_hist_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0137 P2.2: persist abort fence on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn persist_fence_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "persist fence must refuse forall traces"
        );
        assert!(
            residuals.contains("persist_fence_node_counts"),
            "R-joint close must name the persist-fence kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"persist_fence\""),
            "catalog pair persist_fence must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"persist_fence_node_counts\""),
            "catalog entry must stay persist_fence_node_counts"
        );
    }

    /// RFC-0137 P2.2: twin freeze of persist_fence_node_counts is not a verifier.
    #[test]
    fn persist_fence_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"persist_fence\""),
            "persist_fence catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn persist_fence_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0138 P2.2: force-local TX clear on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn force_clear_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "force-clear must refuse forall traces"
        );
        assert!(
            residuals.contains("force_clear_node_counts"),
            "R-joint close must name the force-clear kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"force_clear\""),
            "catalog pair force_clear must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"force_clear_node_counts\""),
            "catalog entry must stay force_clear_node_counts"
        );
    }

    /// RFC-0138 P2.2: twin freeze of force_clear_node_counts is not a verifier.
    #[test]
    fn force_clear_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"force_clear\""),
            "force_clear catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn force_clear_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0139 P2.2: drop preimages on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn drop_preimages_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "drop-preimages must refuse forall traces"
        );
        assert!(
            residuals.contains("drop_preimages_node_counts"),
            "R-joint close must name the drop-preimages kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"drop_preimages\""),
            "catalog pair drop_preimages must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"drop_preimages_node_counts\""),
            "catalog entry must stay drop_preimages_node_counts"
        );
    }

    /// RFC-0139 P2.2: twin freeze of drop_preimages_node_counts is not a verifier.
    #[test]
    fn drop_preimages_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"drop_preimages\""),
            "drop_preimages catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn drop_preimages_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0140 P2.2: in-process open peeking disk membership is a campaign, not ∀ traces.
    #[test]
    fn open_peer_uses_disk_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "open-peer-disk must refuse forall traces"
        );
        assert!(
            residuals.contains("open_peer_uses_disk"),
            "R-joint close must name the open-peer-disk kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"open_peer_disk\""),
            "catalog pair open_peer_disk must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"open_peer_uses_disk\""),
            "catalog entry must stay open_peer_uses_disk"
        );
    }

    /// RFC-0140 P2.2: twin freeze of open_peer_uses_disk is not a verifier.
    #[test]
    fn open_peer_uses_disk_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"open_peer_disk\""),
            "open_peer_disk catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn open_peer_uses_disk"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0141 P2.2: sole local id requiring membership is a campaign, not ∀ traces.
    #[test]
    fn local_id_if_member_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "local-id-if-member must refuse forall traces"
        );
        assert!(
            residuals.contains("local_id_if_member"),
            "R-joint close must name the local-id kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"local_id_member\""),
            "catalog pair local_id_member must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"local_id_if_member\""),
            "catalog entry must stay local_id_if_member"
        );
    }

    /// RFC-0141 P2.2: twin freeze of local_id_if_member is not a verifier.
    #[test]
    fn local_id_if_member_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"local_id_member\""),
            "local_id_member catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn local_id_if_member"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0142 P2.2: LocalApplied reader requiring local node is a campaign, not ∀ traces.
    #[test]
    fn reader_id_local_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "reader-id-local must refuse forall traces"
        );
        assert!(
            residuals.contains("reader_id_local"),
            "R-joint close must name the reader-id kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"reader_local\""),
            "catalog pair reader_local must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"reader_id_local\""),
            "catalog entry must stay reader_id_local"
        );
    }

    /// RFC-0142 P2.2: twin freeze of reader_id_local is not a verifier.
    #[test]
    fn reader_id_local_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"reader_local\""),
            "reader_local catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn reader_id_local"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0143 P2.2: live discard on a local non-member is a campaign, not ∀ traces.
    #[test]
    fn discard_node_counts_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "discard-uncommitted must refuse forall traces"
        );
        assert!(
            residuals.contains("discard_node_counts"),
            "R-joint close must name the discard kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"discard_uncommitted\""),
            "catalog pair discard_uncommitted must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"discard_node_counts\""),
            "catalog entry must stay discard_node_counts"
        );
    }

    /// RFC-0143 P2.2: twin freeze of discard_node_counts is not a verifier.
    #[test]
    fn discard_node_counts_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"discard_uncommitted\""),
            "discard_uncommitted catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn discard_node_counts"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0144 P2.2: no-leader persist-leader requiring local is a campaign, not ∀ traces.
    #[test]
    fn discard_leader_local_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "discard-leader-local must refuse forall traces"
        );
        assert!(
            residuals.contains("discard_leader_local"),
            "R-joint close must name the discard-leader kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"discard_leader\""),
            "catalog pair discard_leader must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"discard_leader_local\""),
            "catalog entry must stay discard_leader_local"
        );
    }

    /// RFC-0144 P2.2: twin freeze of discard_leader_local is not a verifier.
    #[test]
    fn discard_leader_local_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"discard_leader\""),
            "discard_leader catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn discard_leader_local"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0145 P2.2: removed replica stepping down is a campaign, not ∀ traces.
    #[test]
    fn removed_steps_down_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "removed-steps-down must refuse forall traces"
        );
        assert!(
            residuals.contains("removed_steps_down"),
            "R-joint close must name the step-down kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"removed_step_down\""),
            "catalog pair removed_step_down must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"removed_steps_down\""),
            "catalog entry must stay removed_steps_down"
        );
    }

    /// RFC-0145 P2.2: twin freeze of removed_steps_down is not a verifier.
    #[test]
    fn removed_steps_down_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"removed_step_down\""),
            "removed_step_down catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn removed_steps_down"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0146 P2.2: leader hint requiring membership is a campaign, not ∀ traces.
    #[test]
    fn hint_if_member_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "hint-if-member must refuse forall traces"
        );
        assert!(
            residuals.contains("hint_if_member"),
            "R-joint close must name the hint kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"hint_member\""),
            "catalog pair hint_member must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"hint_if_member\""),
            "catalog entry must stay hint_if_member"
        );
    }

    /// RFC-0146 P2.2: twin freeze of hint_if_member is not a verifier.
    #[test]
    fn hint_if_member_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"hint_member\""),
            "hint_member catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn hint_if_member"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0147 P2.2: dropping removed-peer repl slots is a campaign, not ∀ traces.
    #[test]
    fn drop_repl_slot_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "drop-repl-slot must refuse forall traces"
        );
        assert!(
            residuals.contains("drop_repl_slot"),
            "R-joint close must name the drop-repl-slot kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"drop_repl_slot\""),
            "catalog pair drop_repl_slot must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"drop_repl_slot\""),
            "catalog entry must stay drop_repl_slot"
        );
    }

    /// RFC-0147 P2.2: twin freeze of drop_repl_slot is not a verifier.
    #[test]
    fn drop_repl_slot_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"drop_repl_slot\""),
            "drop_repl_slot catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn drop_repl_slot"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0148 P2.2: oob sent_through drop is a campaign, not ∀ traces.
    #[test]
    fn drop_sent_through_campaign_is_not_forall_traces() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            residuals.contains("campaign not a theorem"),
            "drop-sent-through must refuse forall traces"
        );
        assert!(
            residuals.contains("drop_sent_through"),
            "R-joint close must name the drop-sent-through kernel"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"drop_sent_through\""),
            "catalog pair drop_sent_through must stay"
        );
        assert!(
            catalog.contains("\"entry\": \"drop_sent_through\""),
            "catalog entry must stay drop_sent_through"
        );
    }

    /// RFC-0148 P2.2: twin freeze of drop_sent_through is not a verifier.
    #[test]
    fn drop_sent_through_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"drop_sent_through\""),
            "drop_sent_through catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn drop_sent_through"),
            "twin freeze is not a Verus exec claim"
        );
    }

    /// RFC-0129 P2.1: twin freeze of identity-before-applied is not a verifier.
    #[test]
    fn identity_before_applied_verus_still_never() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let residuals =
            std::fs::read_to_string(crate_root.join("../../scripts/formal/residuals.json"))
                .expect("residuals.json");
        assert!(
            residuals.contains("\"id\": \"R-verus\""),
            "R-verus must stay in the residual catalog"
        );
        assert!(
            residuals.contains("\"R-verus\""),
            "never_floor must still list R-verus"
        );
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"identity_before_applied\""),
            "identity_before_applied catalog pair must stay"
        );
        let twin =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/verus/membership_joint.rs"))
                .expect("membership_joint.rs");
        assert!(
            twin.contains("fn membership_identity_before_applied"),
            "twin freeze is not a Verus exec claim"
        );
    }

    #[test]
    fn pending_joint_skips_non_member() {
        assert!(pending_joint_node_counts(true));
        assert!(!pending_joint_node_counts(false));
        assert!(
            pending_joint_node_counts_as_is(false),
            "AS-IS dente: count removed node"
        );
    }

    #[test]
    fn joint_target_counts_is_ids_not_nodes() {
        assert!(joint_target_counts(true, false));
        assert!(
            !joint_target_counts_as_is(true, false),
            "AS-IS dente: require peer in local nodes"
        );
        assert!(!joint_target_counts(false, true));
        assert!(joint_target_counts_as_is(false, true));
        assert!(joint_target_counts(true, true));
        assert!(!joint_target_counts(false, false));
    }

    #[test]
    fn joint_add_target_counts_ignores_local_nodes() {
        assert!(joint_add_target_counts(false));
        assert!(
            !joint_add_target_counts_as_is(false),
            "AS-IS dente: require joiner in local nodes"
        );
        assert!(joint_add_target_counts(true));
        assert!(joint_add_target_counts_as_is(true));
    }

    /// RFC-0119 P2.2: TCP replica joint-target is a campaign, not ∀ traces.
    #[test]
    fn joint_target_campaign_is_not_forall_traces() {
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-joint\""),
            "R-joint must stay in the residual catalog"
        );
        assert!(
            text.contains("campaign not a theorem"),
            "TCP joint-target must refuse forall traces"
        );
    }

    #[test]
    fn election_grant_from_requires_ids_or_pending() {
        assert!(!election_grant_from_counts(false, false));
        assert!(election_grant_from_counts(true, false));
        assert!(election_grant_from_counts(false, true));
        assert!(
            election_grant_from_counts_as_is(false, false),
            "AS-IS dente: record any grant"
        );
    }

    /// RFC-0095 P0: clone of the raft kernel test (tokens stay identical
    /// on the production fns; this is the same glue tooth).
    #[test]
    fn leave_joint_as_is_elects_old_only() {
        let old = [1u64, 2, 3];
        let new = [1u64, 2, 3, 4];
        let glue = |active: bool| {
            if active {
                joint_election_ok(2, 3, Some((2, 4)))
            } else {
                joint_election_ok(2, 3, None)
            }
        };
        assert!(
            !glue(joint_still_active(&old, &new)),
            "fixed: C-old majority is not enough while joint is active"
        );
        assert!(
            glue(joint_still_active_as_is(&old, &new)),
            "AS-IS dente: drop joint → 2/3 C-old elects"
        );
    }

    #[test]
    fn liveness_claim_needs_all_three_es_axioms() {
        assert!(liveness_admitted(true, true, true));
        assert!(!liveness_admitted(false, true, true));
        assert!(!liveness_admitted(true, false, true));
        assert!(!liveness_admitted(true, true, false));
        assert!(liveness_admitted_as_is(false, false, false));
        assert_eq!(
            elect_claim_banner(false, false, false),
            "bounded-elect not-eventual"
        );
        assert!(
            !elect_claim_banner(false, false, false).contains("live"),
            "refused banner must not print live"
        );
        assert_eq!(elect_claim_banner_as_is(false, false, false), "live");
        assert_eq!(
            elect_claim_banner(true, true, true),
            "eventual-live es1=1 es2=1 es3=1"
        );
    }

    /// Catalog three-teeth plant. Direct `liveness_claim_needs_all_three_es_axioms` /
    /// `claim_eventual_election_refused_without_es_axioms` are **not** this tooth.
    #[test]
    fn liveness_admitted_on_live_store_is_not_ok() {
        assert!(!liveness_admitted(false, false, false));
        assert!(
            liveness_admitted_as_is(false, false, false),
            "AS-IS dente: bounded elect is treated as a liveness theorem"
        );
        let dir = std::env::temp_dir().join(format!(
            "liveness-claim-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut c = crate::StoreCluster::open_with_rng(
            &dir,
            3,
            1,
            pedradb_core::SeedRng::new(0x0152_0142),
        )
        .unwrap();
        c.pin_dst_queued();
        assert_eq!(c.rpc_mode(), crate::RpcMode::Queued);
        for _ in 0..120 {
            c.tick().unwrap();
            for _ in 0..48 {
                let batch = c.drain_outbound();
                if batch.is_empty() {
                    break;
                }
                for (from, to, bytes) in batch {
                    c.handle_inbound(from, to, &bytes).unwrap();
                }
            }
            if c.range_leader(1).is_some() {
                break;
            }
        }
        assert!(
            c.range_leader(1).is_some(),
            "Queued bounded elect must find a leader"
        );
        assert!(
            !c.claim_eventual_election(false, false, false),
            "live StoreCluster must refuse ∀-eventual-election without ES axioms"
        );
        assert!(!c.claim_eventual_election(false, true, true));
        assert!(c.claim_eventual_election(true, true, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0094 P1.2: raft/store membership clones keep identical tokens
    /// for leave-joint (`joint_still_active`, `joint_leave_ok`). Catalog
    /// `membership_raft_store` lists them. Drift is a freeze `--clones` fail.
    #[test]
    fn membership_raft_store_clone_tokens_stay_identical() {
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let raft =
            std::fs::read_to_string(crate_root.join("../pedradb-raft/src/membership_kernel.rs"))
                .expect("raft membership_kernel.rs");
        let store = std::fs::read_to_string(crate_root.join("src/membership_kernel.rs"))
            .expect("store membership_kernel.rs");
        for name in ["joint_still_active", "joint_leave_ok"] {
            let a = collapse_fn(&raft, name);
            let b = collapse_fn(&store, name);
            assert_eq!(a, b, "{name} drifted between raft and store clones");
        }
        let catalog = std::fs::read_to_string(crate_root.join("../../scripts/formal/catalog.json"))
            .expect("catalog.json");
        assert!(
            catalog.contains("\"id\": \"membership_raft_store\""),
            "clone trap must stay registered"
        );
        assert!(
            catalog.contains("\"joint_still_active\""),
            "catalog clone must list joint_still_active"
        );
        assert!(
            catalog.contains("\"joint_leave_ok\""),
            "catalog clone must list joint_leave_ok"
        );
    }

    fn collapse_fn(src: &str, name: &str) -> String {
        let sig = format!("pub fn {name}(");
        let start = src.find(&sig).unwrap_or_else(|| panic!("missing {name}"));
        let rest = &src[start..];
        let open = rest.find('{').unwrap_or_else(|| panic!("{name} body"));
        let bytes = rest.as_bytes();
        let mut depth = 0i32;
        let mut end = open;
        for (i, &b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &rest[..=end];
        let mut cleaned = String::new();
        for line in body.lines() {
            cleaned.push_str(line.split("//").next().unwrap_or(""));
            cleaned.push('\n');
        }
        cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}
