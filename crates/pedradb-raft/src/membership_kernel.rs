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

//! **Single artifact (pair `discard_leader`):** this file is what `rustc`
//! links *and* what Verus proves (`cfg(verus_keep_ghost)`). Other
//! membership pairs keep twins until their turns.
//!
//!   ./scripts/verus_membership_joint.sh

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
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


/// Majority of `n` voters (`⌊n/2⌋+1`). `n == 0` → 1 so an empty set never
/// grants (fail-closed).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
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
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_election_ok_as_is(old_yes: u64, old_n: u64, _new_yes: Option<(u64, u64)>) -> bool {
    old_yes >= majority_of(old_n)
}

/// Raft §6: C-old,new is still in force while `old` and `new` differ.
/// Leave-joint is encoded as `old == new` (C-new only).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_still_active(old: &[u64], new: &[u64]) -> bool {
    old != new
}

/// AS-IS: treat every config as single (the 0066 hole — after commit,
/// ignore C-new until apply / skip leave-joint).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_still_active_as_is(_old: &[u64], _new: &[u64]) -> bool {
    false
}

/// RFC-0096: a committed joint is not a single config until a leave
/// (`old == new`) is in the log. AS-IS skips leave.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_leave_ok(leave_in_log: bool) -> bool {
    leave_in_log
}

/// AS-IS: skip leave-joint (the 0066 leftover on the live add path).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_leave_ok_as_is(_leave_in_log: bool) -> bool {
    true
}

/// RFC-0068 P1.2: opt-in World schedule emitted `PlantCommittedJoint`
/// and the default seed scheduler omitted it (fingerprint-stable).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn plant_joint_schedule_ok(opt_in_emits: bool, default_omits: bool) -> bool {
    opt_in_emits && default_omits
}

/// AS-IS: skip the opt-in plant (the 0068 P1.2 hole — random scheduler
/// never emits `PlantCommittedJoint`).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn plant_joint_schedule_ok_as_is(_opt_in_emits: bool, _default_omits: bool) -> bool {
    true
}

/// RFC-0122: a leave in the log is not done until it is committed.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn queued_leave_finish_ok(leave_in_log: bool, leave_committed: bool) -> bool {
    !leave_in_log || leave_committed
}

/// AS-IS: “in the log” is enough (the 0121 leftover — TCP does not finish leave).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn queued_leave_finish_ok_as_is(leave_in_log: bool, _leave_committed: bool) -> bool {
    leave_in_log
}

/// RFC-0124: non-empty durable membership overrides CLI/`--peer` on open.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn disk_membership_overrides_cli(has_disk: bool) -> bool {
    has_disk
}

/// AS-IS: CLI `--peer` wins and overwrites disk (the 0123 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn disk_membership_overrides_cli_as_is(_has_disk: bool) -> bool {
    false
}

/// RFC-0124 P1.1: persist C-new identity before advancing applied past the joint.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn membership_identity_before_applied(identity_first: bool) -> bool {
    identity_first
}

/// AS-IS: persist applied first (crash window: applied high, voters stale).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn membership_identity_before_applied_as_is(_identity_first: bool) -> bool {
    false
}

/// RFC-0125: durable high-water is at least RAM/CLI (never shrink on open).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn high_water_at_least(disk_hw: u64, ram_hw: u64) -> u64 {
    disk_hw.max(ram_hw)
}

/// AS-IS: RAM/CLI length only (the 0124 leftover — 4-node history forgotten).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn high_water_at_least_as_is(_disk_hw: u64, ram_hw: u64) -> u64 {
    ram_hw
}

/// RFC-0130: recover must apply when commit is ahead of applied.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_must_apply(applied: u64, commit: u64) -> bool {
    commit > applied
}

/// AS-IS: skip apply on recover (the 0129 leftover — committed joint stays C-old).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_must_apply_as_is(_applied: u64, _commit: u64) -> bool {
    false
}

/// RFC-0131: recover applies on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_apply_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0130 leftover — removed replica skipped).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_apply_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0132: persist truncated logs on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_truncate_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0131 leftover — removed replica keeps suffix).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_truncate_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0133: a log segment past the truncated hi must be deleted.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_drop_orphan_seg(seg_index: u64, new_hi: u64) -> bool {
    seg_index > new_hi
}

/// AS-IS: leave orphan keys (the 0132 leftover — log_hi capped, segments remain).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_drop_orphan_seg_as_is(_seg_index: u64, _new_hi: u64) -> bool {
    false
}

/// RFC-0134: abort leftover 2PC on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_abort_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0133 leftover — removed replica keeps intents).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn recover_abort_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0135: persist SI meta on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn persist_meta_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0134 leftover — removed replica clock not durable).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn persist_meta_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0136: persist SI hist on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn persist_hist_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0135 leftover — removed replica hist not durable).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn persist_hist_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0137: persist abort fence on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn persist_fence_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0136 leftover — removed replica has no fence).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn persist_fence_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0138: force-local TX clear on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn force_clear_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0137 leftover — removed replica keeps intents).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn force_clear_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0139: drop prepare-time preimages on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn drop_preimages_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0138 leftover — removed replica keeps preimages).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn drop_preimages_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0140: in-process open loads raft peers from disk membership, not CLI n_nodes.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn open_peer_uses_disk(has_disk: bool) -> bool {
    has_disk
}

/// AS-IS: CLI 1..=n_nodes (the 0125 leftover — only TCP peeked).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn open_peer_uses_disk_as_is(_has_disk: bool) -> bool {
    false
}

/// RFC-0141: sole local node is this process's identity only if it is in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn local_id_if_member(in_ids: bool) -> bool {
    in_ids
}

/// AS-IS: HashMap first-key even when removed (the 0140 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn local_id_if_member_as_is(_in_ids: bool) -> bool {
    true
}

/// RFC-0142: a LocalApplied `ids.first()` fallback must be a local node.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn reader_id_local(is_local: bool) -> bool {
    is_local
}

/// AS-IS: `ids.first()` even when not local (the 0141 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn reader_id_local_as_is(_is_local: bool) -> bool {
    true
}

/// RFC-0143: live uncommitted-log discard on every local replica, even if not in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn discard_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}

/// AS-IS: only current voters (the 0142 leftover — removed replica keeps suffix).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn discard_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}

/// RFC-0144: no-leader discard persist-leader must be a local node.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn discard_leader_local(is_local: bool) -> bool {
    is_local
}

/// AS-IS: `ids.first()` even when remote (the 0143 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn discard_leader_local_as_is(_is_local: bool) -> bool {
    true
}

/// RFC-0145: a node dropped from `ids` must step down from Leader.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn removed_steps_down(in_ids: bool) -> bool {
    !in_ids
}

/// AS-IS: keep Role::Leader after joint leave (the 0144 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn removed_steps_down_as_is(_in_ids: bool) -> bool {
    false
}

/// RFC-0146: a leader routing hint counts only if that node is in `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn hint_if_member(in_ids: bool) -> bool {
    in_ids
}

/// AS-IS: any `leader_id` is returned (the 0145 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn hint_if_member_as_is(_in_ids: bool) -> bool {
    true
}

/// RFC-0147: forget next/match/sent_through of a node dropped from `ids`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn drop_repl_slot(in_ids: bool) -> bool {
    !in_ids
}

/// AS-IS: keep replication slots after joint leave (the 0146 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn drop_repl_slot_as_is(_in_ids: bool) -> bool {
    false
}

/// RFC-0148: forget sent_through of a node dropped from `ids` on oob remove.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn drop_sent_through(in_ids: bool) -> bool {
    !in_ids
}

/// AS-IS: keep sent_through after oob remove_member (the 0147 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn drop_sent_through_as_is(_in_ids: bool) -> bool {
    false
}

/// RFC-0127: a node participates iff it is in the current voter set.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn participating_if_member(in_ids: bool) -> bool {
    in_ids
}

/// AS-IS: keep captured participating (the 0126 leftover — removed node counts).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn participating_if_member_as_is(_in_ids: bool) -> bool {
    true
}

/// RFC-0105: only current members' logs define the pending joint.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn pending_joint_node_counts(is_member: bool) -> bool {
    is_member
}

/// AS-IS: scan every opened node, including removed (the 0104 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn pending_joint_node_counts_as_is(_is_member: bool) -> bool {
    true
}

/// RFC-0119: a joint-remove target is the membership set (`ids`), not
/// the local `nodes` map (TCP replica only has self).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_target_counts(in_ids: bool, _in_nodes: bool) -> bool {
    in_ids
}

/// AS-IS: require the target in local nodes (the 0118 leftover — TCP
/// replica cannot joint-remove a peer).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_target_counts_as_is(_in_ids: bool, in_nodes: bool) -> bool {
    in_nodes
}

/// RFC-0119 P1.1: a joint-add target need not live in local `nodes`
/// (joining process is another OS pid).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_add_target_counts(_in_nodes: bool) -> bool {
    true
}

/// AS-IS: require the joiner in local nodes (the 0119 P0 leftover).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn joint_add_target_counts_as_is(in_nodes: bool) -> bool {
    in_nodes
}

/// RFC-0114: a RequestVote grant counts only if the voter is in C-old
/// (`ids`) or in an in-flight joint (C-old ∪ C-new).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn election_grant_from_counts(in_ids: bool, in_pending_old_or_new: bool) -> bool {
    in_ids || in_pending_old_or_new
}

/// AS-IS: any grant is recorded (the 0113 leftover — lagging removed voter).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn election_grant_from_counts_as_is(_in_ids: bool, _in_pending_old_or_new: bool) -> bool {
    true
}

/// Eventual-election / unbounded liveness may be claimed only when all
/// three eventual-synchrony axioms hold (RFC-0069 / RFC-0056 P2.2):
/// ES-1 finite adversary, ES-2 internal drain, ES-3 live retrying candidate.
/// Bounded `elect_all` does **not** go through this gate.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn liveness_admitted(es1: bool, es2: bool, es3: bool) -> bool {
    es1 && es2 && es3
}

/// AS-IS: any bounded elect is treated as a liveness theorem (the 0069 hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn liveness_admitted_as_is(_es1: bool, _es2: bool, _es3: bool) -> bool {
    true
}

/// RFC-0069 P2.2: banner for TCP/real elect. The word `live` only when
/// admitted (and then ES is named). Bounded elect must not print `live`.
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn elect_claim_banner(es1: bool, es2: bool, es3: bool) -> &'static str {
    if liveness_admitted(es1, es2, es3) {
        "eventual-live es1=1 es2=1 es3=1"
    } else {
        "bounded-elect not-eventual"
    }
}

/// AS-IS: print `live` without naming ES (the 0069 P2.2 hole).
#[cfg(not(verus_keep_ghost))]
#[must_use]
#[cfg(not(verus_keep_ghost))]
pub fn elect_claim_banner_as_is(_es1: bool, _es2: bool, _es3: bool) -> &'static str {
    "live"
}

#[cfg(not(verus_keep_ghost))]
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

    #[test]
    fn discard_leader_local_on_live_remote_is_not_ok() {
        assert!(!discard_leader_local(false));
        assert!(
            discard_leader_local_as_is(false),
            "AS-IS dente: ids.first persist-leader even when remote"
        );
        assert!(discard_leader_local(true));
    }

    #[test]
    fn discard_node_counts_on_live_local_non_member_is_not_ok() {
        assert!(discard_node_counts(true, false));
        assert!(
            !discard_node_counts_as_is(true, false),
            "AS-IS dente: skip live discard on local non-member"
        );
        assert!(discard_node_counts(true, true));
        assert!(!discard_node_counts(false, true));
    }

    #[test]
    fn joint_election_ok_on_live_old_only_is_not_ok() {
        assert!(!joint_election_ok(2, 3, Some((2, 4))));
        assert!(
            joint_election_ok_as_is(2, 3, Some((2, 4))),
            "AS-IS dente: elect on C-old during joint add"
        );
        assert!(joint_election_ok(2, 3, None));
        assert!(joint_election_ok(2, 3, Some((3, 4))));
    }

    #[test]
    fn joint_still_active_on_live_differing_sets_is_not_ok() {
        let old = [1u64, 2, 3];
        let new = [1u64, 2, 3, 4];
        assert!(joint_still_active(&old, &new));
        assert!(
            !joint_still_active_as_is(&old, &new),
            "AS-IS dente: treat joint as single"
        );
        let same = [1u64, 2, 3];
        assert!(!joint_still_active(&same, &same));
    }

    #[test]
    fn pending_joint_node_counts_on_live_non_member_is_not_ok() {
        assert!(!pending_joint_node_counts(false));
        assert!(
            pending_joint_node_counts_as_is(false),
            "AS-IS dente: count removed node"
        );
        assert!(pending_joint_node_counts(true));
    }

    #[test]
    fn joint_leave_ok_on_live_missing_leave_is_not_ok() {
        assert!(!joint_leave_ok(false));
        assert!(
            joint_leave_ok_as_is(false),
            "AS-IS dente: skip leave-joint"
        );
        assert!(joint_leave_ok(true));
    }

    #[test]
    fn election_grant_from_counts_on_live_neither_is_not_ok() {
        assert!(!election_grant_from_counts(false, false));
        assert!(
            election_grant_from_counts_as_is(false, false),
            "AS-IS dente: record any grant"
        );
        assert!(election_grant_from_counts(true, false));
        assert!(election_grant_from_counts(false, true));
    }

    #[test]
    fn joint_target_counts_on_live_ids_not_nodes_is_not_ok() {
        assert!(joint_target_counts(true, false));
        assert!(
            !joint_target_counts_as_is(true, false),
            "AS-IS dente: require peer in local nodes"
        );
        assert!(!joint_target_counts(false, true));
        assert!(joint_target_counts_as_is(false, true));
    }

    #[test]
    fn joint_add_target_counts_on_live_not_in_nodes_is_not_ok() {
        assert!(joint_add_target_counts(false));
        assert!(
            !joint_add_target_counts_as_is(false),
            "AS-IS dente: require joiner in local nodes"
        );
        assert!(joint_add_target_counts(true));
    }

    #[test]
    fn queued_leave_finish_ok_on_live_uncommitted_leave_is_not_ok() {
        assert!(!queued_leave_finish_ok(true, false));
        assert!(
            queued_leave_finish_ok_as_is(true, false),
            "AS-IS dente: in the log is enough"
        );
        assert!(queued_leave_finish_ok(true, true));
        assert!(queued_leave_finish_ok(false, false));
    }

    #[test]
    fn disk_membership_overrides_cli_on_live_has_disk_is_not_ok() {
        assert!(disk_membership_overrides_cli(true));
        assert!(
            !disk_membership_overrides_cli_as_is(true),
            "AS-IS dente: CLI --peer overwrites disk"
        );
        assert!(!disk_membership_overrides_cli(false));
    }

    #[test]
    fn high_water_at_least_on_live_disk_ahead_is_not_ok() {
        assert_eq!(high_water_at_least(4, 3), 4);
        assert_eq!(
            high_water_at_least_as_is(4, 3),
            3,
            "AS-IS dente: RAM/CLI length only"
        );
        assert_eq!(high_water_at_least(2, 5), 5);
    }

    #[test]
    fn participating_if_member_on_live_removed_is_not_ok() {
        assert!(!participating_if_member(false));
        assert!(
            participating_if_member_as_is(false),
            "AS-IS dente: keep captured participating"
        );
        assert!(participating_if_member(true));
    }
}
