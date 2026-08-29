//! Pure joint-consensus quorum (Raft §6 / RFC-0064 P2.1).
//!
//! # Contract
//!
//! - **No I/O, no clock, no RNG.** Counts and set sizes are arguments.
//! - Production [`crate`] store callers: `election_has_joint_quorum`.
//! - Persist / RPC / who voted are **axioms**.
//!
//! Clone in `pedradb-store` (`membership_kernel.rs`) must keep the same
//! tokens for `majority_of` / `joint_election_ok`.

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

    /// RFC-0095 P0: production glue (same order as store
    /// `election_has_joint_quorum`) — while the joint is active, 2/3 C-old
    /// is not enough. AS-IS drops the joint and elects.
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
}
