// Verus proof of joint-consensus quorum (RFC-0064 P2.1 / Raft §6).
//
// Twin of `src/membership_kernel.rs`. Not linked into production.
//
//   ./scripts/verus_membership_joint.sh

use vstd::prelude::*;

verus! {

pub open spec fn majority_of_spec(n: u64) -> u64 {
    if n == 0 {
        1
    } else {
        (n / 2 + 1) as u64
    }
}

pub fn majority_of(n: u64) -> (m: u64)
    ensures
        m == majority_of_spec(n),
{
    if n == 0 {
        1
    } else {
        n / 2 + 1
    }
}

pub open spec fn joint_election_ok_spec(old_yes: u64, old_n: u64, new_yes: Option<(u64, u64)>) -> bool {
    old_yes >= majority_of_spec(old_n) && match new_yes {
        None => true,
        Some((yes, n)) => yes >= majority_of_spec(n),
    }
}

pub fn joint_election_ok(old_yes: u64, old_n: u64, new_yes: Option<(u64, u64)>) -> (d: bool)
    ensures
        d == joint_election_ok_spec(old_yes, old_n, new_yes),
{
    if old_yes < majority_of(old_n) {
        return false;
    }
    match new_yes {
        None => true,
        Some((yes, n)) => yes >= majority_of(n),
    }
}

pub open spec fn joint_election_ok_as_is_spec(old_yes: u64, old_n: u64) -> bool {
    old_yes >= majority_of_spec(old_n)
}

pub fn joint_election_ok_as_is(old_yes: u64, old_n: u64, _new_yes: Option<(u64, u64)>) -> (d: bool)
    ensures
        d == joint_election_ok_as_is_spec(old_yes, old_n),
{
    old_yes >= majority_of(old_n)
}

proof fn lemma_as_is_elects_on_old_only()
    ensures
        !joint_election_ok_spec(2, 3, Some((2, 4))),
        joint_election_ok_as_is_spec(2, 3),
{
}

/// RFC-0066 / RFC-0095: C-old,new stays in force while the sets differ.
/// Leave-joint is `old == new`. Spec keeps production tokens `old != new`.
pub open spec fn joint_still_active_spec(old: Seq<u64>, new: Seq<u64>) -> bool {
    old != new
}

pub fn joint_still_active(old: &[u64], new: &[u64]) -> (d: bool)
    ensures
        d == joint_still_active_spec(old@, new@),
{
    if old.len() != new.len() {
        proof {
            assert(old@ != new@);
        }
        return true;
    }
    let mut i: usize = 0;
    while i < old.len()
        invariant
            0 <= i <= old.len(),
            old.len() == new.len(),
            forall|j: int| 0 <= j < i ==> old@[j] == new@[j],
        decreases (old.len() - i) as int,
    {
        if old[i] != new[i] {
            proof {
                assert(old@[i as int] != new@[i as int]);
                assert(old@ != new@);
            }
            return true;
        }
        i = i + 1;
    }
    proof {
        assert(old@ =~= new@);
    }
    false
}

/// AS-IS: treat every config as single (the 0066 hole).
pub open spec fn joint_still_active_as_is_spec(_old: Seq<u64>, _new: Seq<u64>) -> bool {
    false
}

pub fn joint_still_active_as_is(_old: &[u64], _new: &[u64]) -> (d: bool)
    ensures
        d == false,
{
    false
}

proof fn lemma_as_is_never_active()
    ensures
        forall|o: Seq<u64>, n: Seq<u64>|
            joint_still_active_as_is_spec(o, n) == false,
{
}

/// RFC-0096 / RFC-0110: a committed joint is not a single config until
/// leave is in the log. AS-IS skips leave.
pub open spec fn joint_leave_ok_spec(leave_in_log: bool) -> bool {
    leave_in_log
}

pub open spec fn joint_leave_ok_as_is_spec(_leave_in_log: bool) -> bool {
    true
}

pub fn joint_leave_ok(leave_in_log: bool) -> (d: bool)
    ensures
        d == joint_leave_ok_spec(leave_in_log),
{
    leave_in_log
}

pub fn joint_leave_ok_as_is(_leave_in_log: bool) -> (d: bool)
    ensures
        d == joint_leave_ok_as_is_spec(_leave_in_log),
{
    true
}

/// RFC-0068 P1.2: opt-in schedule emits PlantCommittedJoint; default omits it.
pub open spec fn plant_joint_schedule_ok_spec(opt_in_emits: bool, default_omits: bool) -> bool {
    opt_in_emits && default_omits
}

pub open spec fn plant_joint_schedule_ok_as_is_spec(_opt_in_emits: bool, _default_omits: bool) -> bool {
    true
}

pub fn plant_joint_schedule_ok(opt_in_emits: bool, default_omits: bool) -> (d: bool)
    ensures
        d == plant_joint_schedule_ok_spec(opt_in_emits, default_omits),
{
    opt_in_emits && default_omits
}

pub fn plant_joint_schedule_ok_as_is(_opt_in_emits: bool, _default_omits: bool) -> (d: bool)
    ensures
        d == plant_joint_schedule_ok_as_is_spec(_opt_in_emits, _default_omits),
{
    true
}

proof fn lemma_as_is_skips_leave()
    ensures
        !joint_leave_ok_spec(false),
        joint_leave_ok_as_is_spec(false),
{
}

/// RFC-0069 P2.1: eventual-election only when ES-1 ∧ ES-2 ∧ ES-3.
pub open spec fn liveness_admitted_spec(es1: bool, es2: bool, es3: bool) -> bool {
    es1 && es2 && es3
}

pub open spec fn liveness_admitted_as_is_spec(_es1: bool, _es2: bool, _es3: bool) -> bool {
    true
}

pub fn liveness_admitted(es1: bool, es2: bool, es3: bool) -> (ok: bool)
    ensures
        ok == liveness_admitted_spec(es1, es2, es3),
{
    es1 && es2 && es3
}

pub fn liveness_admitted_as_is(_es1: bool, _es2: bool, _es3: bool) -> (ok: bool)
    ensures
        ok == liveness_admitted_as_is_spec(_es1, _es2, _es3),
{
    true
}

proof fn lemma_missing_es_is_not_liveness()
    ensures
        !liveness_admitted_spec(false, true, true),
        liveness_admitted_as_is_spec(false, true, true),
        liveness_admitted_spec(true, true, true),
{
}

/// RFC-0105 / RFC-0108: only current members' logs define the pending joint.
pub open spec fn pending_joint_node_counts_spec(is_member: bool) -> bool {
    is_member
}

pub open spec fn pending_joint_node_counts_as_is_spec(_is_member: bool) -> bool {
    true
}

pub fn pending_joint_node_counts(is_member: bool) -> (d: bool)
    ensures
        d == pending_joint_node_counts_spec(is_member),
{
    is_member
}

pub fn pending_joint_node_counts_as_is(_is_member: bool) -> (d: bool)
    ensures
        d == pending_joint_node_counts_as_is_spec(_is_member),
{
    true
}

proof fn lemma_as_is_counts_removed()
    ensures
        !pending_joint_node_counts_spec(false),
        pending_joint_node_counts_as_is_spec(false),
{
}

/// RFC-0114 / RFC-0116: a RequestVote grant counts only if the voter is
/// in C-old (`ids`) or in an in-flight joint (C-old ∪ C-new).
pub open spec fn election_grant_from_counts_spec(in_ids: bool, in_pending_old_or_new: bool) -> bool {
    in_ids || in_pending_old_or_new
}

pub open spec fn election_grant_from_counts_as_is_spec(
    _in_ids: bool,
    _in_pending_old_or_new: bool,
) -> bool {
    true
}

pub fn election_grant_from_counts(in_ids: bool, in_pending_old_or_new: bool) -> (d: bool)
    ensures
        d == election_grant_from_counts_spec(in_ids, in_pending_old_or_new),
{
    in_ids || in_pending_old_or_new
}

pub fn election_grant_from_counts_as_is(_in_ids: bool, _in_pending_old_or_new: bool) -> (d: bool)
    ensures
        d == election_grant_from_counts_as_is_spec(_in_ids, _in_pending_old_or_new),
{
    true
}

proof fn lemma_as_is_records_any_grant()
    ensures
        !election_grant_from_counts_spec(false, false),
        election_grant_from_counts_spec(false, true),
        election_grant_from_counts_as_is_spec(false, false),
{
}

/// RFC-0119: joint-remove target is membership `ids`, not local `nodes`.
pub open spec fn joint_target_counts_spec(in_ids: bool, _in_nodes: bool) -> bool {
    in_ids
}

pub open spec fn joint_target_counts_as_is_spec(_in_ids: bool, in_nodes: bool) -> bool {
    in_nodes
}

pub fn joint_target_counts(in_ids: bool, _in_nodes: bool) -> (d: bool)
    ensures
        d == joint_target_counts_spec(in_ids, _in_nodes),
{
    in_ids
}

pub fn joint_target_counts_as_is(_in_ids: bool, in_nodes: bool) -> (d: bool)
    ensures
        d == joint_target_counts_as_is_spec(_in_ids, in_nodes),
{
    in_nodes
}

proof fn lemma_as_is_requires_local_nodes()
    ensures
        joint_target_counts_spec(true, false),
        !joint_target_counts_as_is_spec(true, false),
{
}

/// RFC-0119 P1.1: joint-add target need not live in local `nodes`.
pub open spec fn joint_add_target_counts_spec(_in_nodes: bool) -> bool {
    true
}

pub open spec fn joint_add_target_counts_as_is_spec(in_nodes: bool) -> bool {
    in_nodes
}

pub fn joint_add_target_counts(_in_nodes: bool) -> (d: bool)
    ensures
        d == joint_add_target_counts_spec(_in_nodes),
{
    true
}

pub fn joint_add_target_counts_as_is(in_nodes: bool) -> (d: bool)
    ensures
        d == joint_add_target_counts_as_is_spec(in_nodes),
{
    in_nodes
}

/// RFC-0122 / RFC-0123: a leave in the log is not done until it is committed.
pub open spec fn queued_leave_finish_ok_spec(leave_in_log: bool, leave_committed: bool) -> bool {
    !leave_in_log || leave_committed
}

pub open spec fn queued_leave_finish_ok_as_is_spec(leave_in_log: bool, _leave_committed: bool) -> bool {
    leave_in_log
}

pub fn queued_leave_finish_ok(leave_in_log: bool, leave_committed: bool) -> (d: bool)
    ensures
        d == queued_leave_finish_ok_spec(leave_in_log, leave_committed),
{
    !leave_in_log || leave_committed
}

pub fn queued_leave_finish_ok_as_is(leave_in_log: bool, _leave_committed: bool) -> (d: bool)
    ensures
        d == queued_leave_finish_ok_as_is_spec(leave_in_log, _leave_committed),
{
    leave_in_log
}

proof fn lemma_as_is_skips_leave_commit()
    ensures
        !queued_leave_finish_ok_spec(true, false),
        queued_leave_finish_ok_as_is_spec(true, false),
{
}

/// RFC-0124 P2.1: non-empty durable membership overrides CLI/`--peer`.
pub open spec fn disk_membership_overrides_cli_spec(has_disk: bool) -> bool {
    has_disk
}

pub open spec fn disk_membership_overrides_cli_as_is_spec(_has_disk: bool) -> bool {
    false
}

pub fn disk_membership_overrides_cli(has_disk: bool) -> (d: bool)
    ensures
        d == disk_membership_overrides_cli_spec(has_disk),
{
    has_disk
}

pub fn disk_membership_overrides_cli_as_is(_has_disk: bool) -> (d: bool)
    ensures
        d == disk_membership_overrides_cli_as_is_spec(_has_disk),
{
    false
}

proof fn lemma_as_is_cli_overwrites_disk()
    ensures
        disk_membership_overrides_cli_spec(true),
        !disk_membership_overrides_cli_as_is_spec(true),
{
}

/// RFC-0125 P1.2: durable high-water is at least RAM/CLI (never shrink on open).
pub open spec fn high_water_at_least_spec(disk_hw: u64, ram_hw: u64) -> u64 {
    if disk_hw > ram_hw {
        disk_hw
    } else {
        ram_hw
    }
}

pub open spec fn high_water_at_least_as_is_spec(_disk_hw: u64, ram_hw: u64) -> u64 {
    ram_hw
}

pub fn high_water_at_least(disk_hw: u64, ram_hw: u64) -> (m: u64)
    ensures
        m == high_water_at_least_spec(disk_hw, ram_hw),
{
    disk_hw.max(ram_hw)
}

pub fn high_water_at_least_as_is(_disk_hw: u64, ram_hw: u64) -> (m: u64)
    ensures
        m == high_water_at_least_as_is_spec(_disk_hw, ram_hw),
{
    ram_hw
}

proof fn lemma_as_is_forgets_disk_high_water()
    ensures
        high_water_at_least_spec(4, 3) == 4,
        high_water_at_least_as_is_spec(4, 3) == 3,
{
}

/// RFC-0127 / RFC-0128: a node participates iff it is in the voter set.
pub open spec fn participating_if_member_spec(in_ids: bool) -> bool {
    in_ids
}

pub open spec fn participating_if_member_as_is_spec(_in_ids: bool) -> bool {
    true
}

pub fn participating_if_member(in_ids: bool) -> (d: bool)
    ensures
        d == participating_if_member_spec(in_ids),
{
    in_ids
}

pub fn participating_if_member_as_is(_in_ids: bool) -> (d: bool)
    ensures
        d == participating_if_member_as_is_spec(_in_ids),
{
    true
}

proof fn lemma_as_is_keeps_captured_participating()
    ensures
        !participating_if_member_spec(false),
        participating_if_member_as_is_spec(false),
{
}

/// RFC-0124 / RFC-0129: persist C-new identity before advancing applied.
pub open spec fn membership_identity_before_applied_spec(identity_first: bool) -> bool {
    identity_first
}

pub open spec fn membership_identity_before_applied_as_is_spec(_identity_first: bool) -> bool {
    false
}

pub fn membership_identity_before_applied(identity_first: bool) -> (d: bool)
    ensures
        d == membership_identity_before_applied_spec(identity_first),
{
    identity_first
}

pub fn membership_identity_before_applied_as_is(_identity_first: bool) -> (d: bool)
    ensures
        d == membership_identity_before_applied_as_is_spec(_identity_first),
{
    false
}

proof fn lemma_as_is_persists_applied_first()
    ensures
        membership_identity_before_applied_spec(true),
        !membership_identity_before_applied_as_is_spec(true),
{
}

/// RFC-0130: recover must apply when commit is ahead of applied.
pub open spec fn recover_must_apply_spec(applied: u64, commit: u64) -> bool {
    commit > applied
}

pub open spec fn recover_must_apply_as_is_spec(_applied: u64, _commit: u64) -> bool {
    false
}

pub fn recover_must_apply(applied: u64, commit: u64) -> (d: bool)
    ensures
        d == recover_must_apply_spec(applied, commit),
{
    commit > applied
}

pub fn recover_must_apply_as_is(_applied: u64, _commit: u64) -> (d: bool)
    ensures
        d == recover_must_apply_as_is_spec(_applied, _commit),
{
    false
}

proof fn lemma_as_is_skips_recover_apply()
    ensures
        recover_must_apply_spec(1, 2),
        !recover_must_apply_as_is_spec(1, 2),
{
}

/// RFC-0131: recover applies on every local replica, even if not in ids.
pub open spec fn recover_apply_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn recover_apply_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn recover_apply_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == recover_apply_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn recover_apply_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == recover_apply_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_replica()
    ensures
        recover_apply_node_counts_spec(true, false),
        !recover_apply_node_counts_as_is_spec(true, false),
{
}

/// RFC-0132: persist truncated logs on every local replica, even if not in ids.
pub open spec fn recover_truncate_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn recover_truncate_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn recover_truncate_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == recover_truncate_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn recover_truncate_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == recover_truncate_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_keeps_uncommitted_suffix()
    ensures
        recover_truncate_node_counts_spec(true, false),
        !recover_truncate_node_counts_as_is_spec(true, false),
{
}

/// RFC-0133: a log segment past the truncated hi must be deleted.
pub open spec fn recover_drop_orphan_seg_spec(seg_index: u64, new_hi: u64) -> bool {
    seg_index > new_hi
}

pub open spec fn recover_drop_orphan_seg_as_is_spec(_seg_index: u64, _new_hi: u64) -> bool {
    false
}

pub fn recover_drop_orphan_seg(seg_index: u64, new_hi: u64) -> (d: bool)
    ensures
        d == recover_drop_orphan_seg_spec(seg_index, new_hi),
{
    seg_index > new_hi
}

pub fn recover_drop_orphan_seg_as_is(_seg_index: u64, _new_hi: u64) -> (d: bool)
    ensures
        d == recover_drop_orphan_seg_as_is_spec(_seg_index, _new_hi),
{
    false
}

proof fn lemma_as_is_keeps_orphan_seg()
    ensures
        recover_drop_orphan_seg_spec(3, 2),
        !recover_drop_orphan_seg_as_is_spec(3, 2),
{
}

/// RFC-0134: abort leftover 2PC on every local replica, even if not in ids.
pub open spec fn recover_abort_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn recover_abort_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn recover_abort_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == recover_abort_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn recover_abort_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == recover_abort_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_abort()
    ensures
        recover_abort_node_counts_spec(true, false),
        !recover_abort_node_counts_as_is_spec(true, false),
{
}

/// RFC-0135: persist SI meta on every local replica, even if not in ids.
pub open spec fn persist_meta_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn persist_meta_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn persist_meta_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == persist_meta_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn persist_meta_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == persist_meta_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_meta()
    ensures
        persist_meta_node_counts_spec(true, false),
        !persist_meta_node_counts_as_is_spec(true, false),
{
}

/// RFC-0136: persist SI hist on every local replica, even if not in ids.
pub open spec fn persist_hist_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn persist_hist_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn persist_hist_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == persist_hist_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn persist_hist_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == persist_hist_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_hist()
    ensures
        persist_hist_node_counts_spec(true, false),
        !persist_hist_node_counts_as_is_spec(true, false),
{
}

/// RFC-0137: persist abort fence on every local replica, even if not in ids.
pub open spec fn persist_fence_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn persist_fence_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn persist_fence_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == persist_fence_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn persist_fence_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == persist_fence_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_fence()
    ensures
        persist_fence_node_counts_spec(true, false),
        !persist_fence_node_counts_as_is_spec(true, false),
{
}

/// RFC-0138: force-local TX clear on every local replica, even if not in ids.
pub open spec fn force_clear_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn force_clear_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn force_clear_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == force_clear_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn force_clear_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == force_clear_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_clear()
    ensures
        force_clear_node_counts_spec(true, false),
        !force_clear_node_counts_as_is_spec(true, false),
{
}

/// RFC-0139: drop prepare-time preimages on every local replica, even if not in ids.
pub open spec fn drop_preimages_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn drop_preimages_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn drop_preimages_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == drop_preimages_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn drop_preimages_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == drop_preimages_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_preimages()
    ensures
        drop_preimages_node_counts_spec(true, false),
        !drop_preimages_node_counts_as_is_spec(true, false),
{
}

/// RFC-0140: in-process open loads raft peers from disk membership, not CLI n_nodes.
pub open spec fn open_peer_uses_disk_spec(has_disk: bool) -> bool {
    has_disk
}

pub open spec fn open_peer_uses_disk_as_is_spec(_has_disk: bool) -> bool {
    false
}

pub fn open_peer_uses_disk(has_disk: bool) -> (d: bool)
    ensures
        d == open_peer_uses_disk_spec(has_disk),
{
    has_disk
}

pub fn open_peer_uses_disk_as_is(_has_disk: bool) -> (d: bool)
    ensures
        d == open_peer_uses_disk_as_is_spec(_has_disk),
{
    false
}

proof fn lemma_as_is_skips_disk_on_in_process_open()
    ensures
        open_peer_uses_disk_spec(true),
        !open_peer_uses_disk_as_is_spec(true),
{
}

/// RFC-0141: sole local node is this process's identity only if it is in ids.
pub open spec fn local_id_if_member_spec(in_ids: bool) -> bool {
    in_ids
}

pub open spec fn local_id_if_member_as_is_spec(_in_ids: bool) -> bool {
    true
}

pub fn local_id_if_member(in_ids: bool) -> (d: bool)
    ensures
        d == local_id_if_member_spec(in_ids),
{
    in_ids
}

pub fn local_id_if_member_as_is(_in_ids: bool) -> (d: bool)
    ensures
        d == local_id_if_member_as_is_spec(_in_ids),
{
    true
}

proof fn lemma_as_is_counts_removed_local_id()
    ensures
        !local_id_if_member_spec(false),
        local_id_if_member_as_is_spec(false),
{
}

/// RFC-0142: a LocalApplied ids.first fallback must be a local node.
pub open spec fn reader_id_local_spec(is_local: bool) -> bool {
    is_local
}

pub open spec fn reader_id_local_as_is_spec(_is_local: bool) -> bool {
    true
}

pub fn reader_id_local(is_local: bool) -> (d: bool)
    ensures
        d == reader_id_local_spec(is_local),
{
    is_local
}

pub fn reader_id_local_as_is(_is_local: bool) -> (d: bool)
    ensures
        d == reader_id_local_as_is_spec(_is_local),
{
    true
}

proof fn lemma_as_is_picks_remote_ids_first()
    ensures
        !reader_id_local_spec(false),
        reader_id_local_as_is_spec(false),
{
}

/// RFC-0143: live uncommitted-log discard on every local replica, even if not in ids.
pub open spec fn discard_node_counts_spec(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

pub open spec fn discard_node_counts_as_is_spec(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

pub fn discard_node_counts(is_local: bool, _in_ids: bool) -> (d: bool)
    ensures
        d == discard_node_counts_spec(is_local, _in_ids),
{
    is_local
}

pub fn discard_node_counts_as_is(is_local: bool, in_ids: bool) -> (d: bool)
    ensures
        d == discard_node_counts_as_is_spec(is_local, in_ids),
{
    is_local && in_ids
}

proof fn lemma_as_is_skips_removed_discard()
    ensures
        discard_node_counts_spec(true, false),
        !discard_node_counts_as_is_spec(true, false),
{
}

/// RFC-0144: no-leader discard persist-leader must be a local node.
pub open spec fn discard_leader_local_spec(is_local: bool) -> bool {
    is_local
}

pub open spec fn discard_leader_local_as_is_spec(_is_local: bool) -> bool {
    true
}

pub fn discard_leader_local(is_local: bool) -> (d: bool)
    ensures
        d == discard_leader_local_spec(is_local),
{
    is_local
}

pub fn discard_leader_local_as_is(_is_local: bool) -> (d: bool)
    ensures
        d == discard_leader_local_as_is_spec(_is_local),
{
    true
}

proof fn lemma_as_is_picks_remote_persist_leader()
    ensures
        !discard_leader_local_spec(false),
        discard_leader_local_as_is_spec(false),
{
}

/// RFC-0145: a node dropped from ids must step down from Leader.
pub open spec fn removed_steps_down_spec(in_ids: bool) -> bool {
    !in_ids
}

pub open spec fn removed_steps_down_as_is_spec(_in_ids: bool) -> bool {
    false
}

pub fn removed_steps_down(in_ids: bool) -> (d: bool)
    ensures
        d == removed_steps_down_spec(in_ids),
{
    !in_ids
}

pub fn removed_steps_down_as_is(_in_ids: bool) -> (d: bool)
    ensures
        d == removed_steps_down_as_is_spec(_in_ids),
{
    false
}

proof fn lemma_as_is_keeps_removed_leader()
    ensures
        removed_steps_down_spec(false),
        !removed_steps_down_as_is_spec(false),
{
}

/// RFC-0146: a leader routing hint counts only if that node is in ids.
pub open spec fn hint_if_member_spec(in_ids: bool) -> bool {
    in_ids
}

pub open spec fn hint_if_member_as_is_spec(_in_ids: bool) -> bool {
    true
}

pub fn hint_if_member(in_ids: bool) -> (d: bool)
    ensures
        d == hint_if_member_spec(in_ids),
{
    in_ids
}

pub fn hint_if_member_as_is(_in_ids: bool) -> (d: bool)
    ensures
        d == hint_if_member_as_is_spec(_in_ids),
{
    true
}

proof fn lemma_as_is_hints_removed()
    ensures
        !hint_if_member_spec(false),
        hint_if_member_as_is_spec(false),
{
}

/// RFC-0147: forget next/match/sent_through of a node dropped from ids.
pub open spec fn drop_repl_slot_spec(in_ids: bool) -> bool {
    !in_ids
}

pub open spec fn drop_repl_slot_as_is_spec(_in_ids: bool) -> bool {
    false
}

pub fn drop_repl_slot(in_ids: bool) -> (d: bool)
    ensures
        d == drop_repl_slot_spec(in_ids),
{
    !in_ids
}

pub fn drop_repl_slot_as_is(_in_ids: bool) -> (d: bool)
    ensures
        d == drop_repl_slot_as_is_spec(_in_ids),
{
    false
}

proof fn lemma_as_is_keeps_removed_repl_slot()
    ensures
        drop_repl_slot_spec(false),
        !drop_repl_slot_as_is_spec(false),
{
}

/// RFC-0148: forget sent_through of a node dropped from ids on oob remove.
pub open spec fn drop_sent_through_spec(in_ids: bool) -> bool {
    !in_ids
}

pub open spec fn drop_sent_through_as_is_spec(_in_ids: bool) -> bool {
    false
}

pub fn drop_sent_through(in_ids: bool) -> (d: bool)
    ensures
        d == drop_sent_through_spec(in_ids),
{
    !in_ids
}

pub fn drop_sent_through_as_is(_in_ids: bool) -> (d: bool)
    ensures
        d == drop_sent_through_as_is_spec(_in_ids),
{
    false
}

proof fn lemma_as_is_keeps_oob_sent_through()
    ensures
        drop_sent_through_spec(false),
        !drop_sent_through_as_is_spec(false),
{
}

} // verus!
