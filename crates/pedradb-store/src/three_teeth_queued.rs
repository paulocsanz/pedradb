//! RFC-0151 P2: each Raft `data_fate` kernel has a Queued-RPC plant.
//! L28 REAL TCP stays a campaign (`world_seed_l28_ok(0, false)` is not Ok).

use crate::membership_kernel::*;
use crate::{
    l28_tcp_abort_ok, l28_tcp_abort_ok_as_is, l28_tcp_apply_ok, l28_tcp_apply_ok_as_is,
    l28_tcp_clear_ok, l28_tcp_clear_ok_as_is, l28_tcp_dsc_ok, l28_tcp_dsc_ok_as_is,
    l28_tcp_fence_ok, l28_tcp_fence_ok_as_is, l28_tcp_hist_ok, l28_tcp_hist_ok_as_is,
    l28_tcp_hnt_ok, l28_tcp_hnt_ok_as_is, l28_tcp_hw_ok, l28_tcp_hw_ok_as_is, l28_tcp_left_ok,
    l28_tcp_left_ok_as_is, l28_tcp_lid_ok, l28_tcp_lid_ok_as_is, l28_tcp_napply_ok,
    l28_tcp_napply_ok_as_is, l28_tcp_nowms_ok, l28_tcp_nowms_ok_as_is, l28_tcp_odrop_ok,
    l28_tcp_odrop_ok_as_is, l28_tcp_part_ok, l28_tcp_part_ok_as_is, l28_tcp_peer_ok,
    l28_tcp_peer_ok_as_is, l28_tcp_pld_ok, l28_tcp_pld_ok_as_is, l28_tcp_pre_ok,
    l28_tcp_pre_ok_as_is, l28_tcp_rdr_ok, l28_tcp_rdr_ok_as_is, l28_tcp_std_ok,
    l28_tcp_std_ok_as_is, l28_tcp_trunc_ok, l28_tcp_trunc_ok_as_is, world_seed_l28_ok,
    world_seed_l28_ok_as_is, RpcMode, StoreCluster,
};
use pedradb_core::SeedRng;
use pedradb_raft::ae_kernel::ae_entry_action_as_is_rewrite_committed;
use pedradb_raft::ae_kernel::{
    ae_ack_success, ae_ack_success_as_is, ae_entry_action, AeEntryAction,
};
use pedradb_raft::apply_kernel::{apply_advance, apply_advance_as_is_skip_holes, ApplyAction};
use pedradb_raft::commit_kernel::{propose_ack_ok, propose_ack_ok_as_is};
use pedradb_raft::vote_kernel::{
    grant_after_persist, grant_after_persist_as_is, vote_decision,
    vote_decision_as_is_ignore_log_and_vote, PersistOutcome, VoteDecision, VoteInputs,
};

struct LiveQueued {
    dir: std::path::PathBuf,
    cluster: StoreCluster,
}

fn pump_queued(c: &mut StoreCluster, rounds: usize) {
    for _ in 0..rounds {
        let batch = c.drain_outbound();
        if batch.is_empty() {
            break;
        }
        for (from, to, bytes) in batch {
            c.handle_inbound(from, to, &bytes).unwrap();
        }
    }
}

impl LiveQueued {
    fn open() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pedra-queued-plant-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cluster =
            StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0151_0001)).unwrap();
        cluster.pin_dst_queued();
        assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
        assert!(cluster.dst_queued_pin());
        // Drive RequestVote / AppendEntries through Queued inbound, not Direct.
        for _ in 0..80 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        assert!(
            cluster.range_leader(1).is_some(),
            "Queued RPC must elect (RV/AE via handle_inbound)"
        );
        Self { dir, cluster }
    }
}

impl Drop for LiveQueued {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One live pin: plants below share Queued RPC (not Direct). L28 TCP is not ∀.
#[test]
fn pin_dst_queued_on_live_cluster() {
    let q = LiveQueued::open();
    assert_eq!(q.cluster.rpc_mode(), RpcMode::Queued);
    assert!(
        !world_seed_l28_ok(0, false),
        "World silent_wrong=0 is not L28"
    );
}

fn vote_stale() -> VoteInputs {
    VoteInputs {
        current_term: 1,
        voted_for: None,
        last_log_term: 2,
        last_log_index: 10,
        candidate_term: 1,
        candidate_id: 2,
        candidate_last_log_term: 1,
        candidate_last_log_index: 1,
    }
}

#[test]
fn vote_decision_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    let i = vote_stale();
    assert_eq!(vote_decision(i), VoteDecision::Deny);
    assert_eq!(
        vote_decision_as_is_ignore_log_and_vote(i),
        VoteDecision::WouldGrant
    );
}

#[test]
fn grant_after_persist_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!grant_after_persist(
        VoteDecision::WouldGrant,
        PersistOutcome::Err
    ));
    assert!(grant_after_persist_as_is(
        VoteDecision::WouldGrant,
        PersistOutcome::Err
    ));
}

#[test]
fn ae_entry_action_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert_eq!(ae_entry_action(1, 2, Some(1), 5, 5), AeEntryAction::Refuse);
    assert_ne!(
        ae_entry_action(1, 2, Some(1), 5, 5),
        ae_entry_action_as_is_rewrite_committed(1, 2, Some(1), 5, 5)
    );
}

#[test]
fn ae_ack_success_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!ae_ack_success(true, false));
    assert!(ae_ack_success_as_is(true, false));
}

#[test]
fn propose_ack_ok_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!propose_ack_ok(5, 3));
    assert!(propose_ack_ok_as_is(5, 3));
}

#[test]
fn apply_advance_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert_eq!(apply_advance(1, 5, false), ApplyAction::Stop);
    assert_eq!(
        apply_advance_as_is_skip_holes(1, 5, false),
        ApplyAction::Apply
    );
}

#[test]
fn joint_election_ok_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!joint_election_ok(2, 3, Some((0, 3))));
    assert!(joint_election_ok_as_is(2, 3, Some((0, 3))));
}

#[test]
fn joint_still_active_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(joint_still_active(&[1, 2], &[1, 2, 3]));
    assert!(!joint_still_active_as_is(&[1, 2], &[1, 2, 3]));
}

#[test]
fn joint_leave_ok_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!joint_leave_ok(false));
    assert!(joint_leave_ok_as_is(false));
}

#[test]
fn queued_leave_finish_ok_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!queued_leave_finish_ok(true, false));
    assert!(queued_leave_finish_ok_as_is(true, false));
}

#[test]
fn pending_joint_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!pending_joint_node_counts(false));
    assert!(pending_joint_node_counts_as_is(false));
}

#[test]
fn election_grant_from_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!election_grant_from_counts(false, false));
    assert!(election_grant_from_counts_as_is(false, false));
}

#[test]
fn joint_target_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!joint_target_counts(false, true));
    assert!(joint_target_counts_as_is(false, true));
}

#[test]
fn joint_add_target_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(joint_add_target_counts(false));
    assert!(!joint_add_target_counts_as_is(false));
}

#[test]
fn disk_membership_overrides_cli_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(disk_membership_overrides_cli(true));
    assert!(!disk_membership_overrides_cli_as_is(true));
}

#[test]
fn high_water_at_least_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert_eq!(high_water_at_least(9, 3), 9);
    assert_eq!(high_water_at_least_as_is(9, 3), 3);
}

#[test]
fn participating_if_member_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!participating_if_member(false));
    assert!(participating_if_member_as_is(false));
}

#[test]
fn membership_identity_before_applied_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(membership_identity_before_applied(true));
    assert!(!membership_identity_before_applied_as_is(true));
}

#[test]
fn recover_must_apply_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(recover_must_apply(1, 3));
    assert!(!recover_must_apply_as_is(1, 3));
}

#[test]
fn recover_apply_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(recover_apply_node_counts(true, false));
    assert!(!recover_apply_node_counts_as_is(true, false));
}

#[test]
fn recover_truncate_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(recover_truncate_node_counts(true, false));
    assert!(!recover_truncate_node_counts_as_is(true, false));
}

#[test]
fn recover_drop_orphan_seg_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(recover_drop_orphan_seg(9, 5));
    assert!(!recover_drop_orphan_seg_as_is(9, 5));
}

#[test]
fn recover_abort_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(recover_abort_node_counts(true, false));
    assert!(!recover_abort_node_counts_as_is(true, false));
}

#[test]
fn persist_meta_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(persist_meta_node_counts(true, false));
    assert!(!persist_meta_node_counts_as_is(true, false));
}

#[test]
fn persist_hist_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(persist_hist_node_counts(true, false));
    assert!(!persist_hist_node_counts_as_is(true, false));
}

#[test]
fn persist_fence_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(persist_fence_node_counts(true, false));
    assert!(!persist_fence_node_counts_as_is(true, false));
}

#[test]
fn force_clear_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(force_clear_node_counts(true, false));
    assert!(!force_clear_node_counts_as_is(true, false));
}

#[test]
fn drop_preimages_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(drop_preimages_node_counts(true, false));
    assert!(!drop_preimages_node_counts_as_is(true, false));
}

#[test]
fn open_peer_uses_disk_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(open_peer_uses_disk(true));
    assert!(!open_peer_uses_disk_as_is(true));
}

#[test]
fn local_id_if_member_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!local_id_if_member(false));
    assert!(local_id_if_member_as_is(false));
}

#[test]
fn reader_id_local_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!reader_id_local(false));
    assert!(reader_id_local_as_is(false), "AS-IS dente: remote first-id");
}

#[test]
fn discard_node_counts_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(discard_node_counts(true, false));
    assert!(!discard_node_counts_as_is(true, false));
}

#[test]
fn discard_leader_local_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!discard_leader_local(false));
    assert!(
        discard_leader_local_as_is(false),
        "AS-IS dente: remote persist-leader"
    );
}

#[test]
fn removed_steps_down_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(removed_steps_down(false));
    assert!(
        !removed_steps_down_as_is(false),
        "AS-IS dente: stay leader after leave"
    );
}

#[test]
fn hint_if_member_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(!hint_if_member(false));
    assert!(hint_if_member_as_is(false));
}

#[test]
fn drop_repl_slot_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(drop_repl_slot(false));
    assert!(!drop_repl_slot_as_is(false), "AS-IS dente: keep slot");
}

#[test]
fn drop_sent_through_on_live_queued_is_not_ok() {
    let _q = LiveQueued::open();
    assert!(drop_sent_through(false));
    assert!(
        !drop_sent_through_as_is(false),
        "AS-IS dente: keep sent_through"
    );
}

fn l28_campaign() {
    let _q = LiveQueued::open();
    assert!(!world_seed_l28_ok(0, false));
    assert!(world_seed_l28_ok_as_is(0, false));
}

#[test]
fn l28_tcp_left_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_left_ok(true));
    assert!(l28_tcp_left_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_hw_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_hw_ok(true));
    assert!(l28_tcp_hw_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_part_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_part_ok(true));
    assert!(l28_tcp_part_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_apply_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_apply_ok(true));
    assert!(l28_tcp_apply_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_napply_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_napply_ok(true));
    assert!(l28_tcp_napply_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_trunc_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_trunc_ok(true));
    assert!(l28_tcp_trunc_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_odrop_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_odrop_ok(true));
    assert!(l28_tcp_odrop_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_abort_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_abort_ok(true));
    assert!(l28_tcp_abort_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_nowms_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_nowms_ok(true));
    assert!(l28_tcp_nowms_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_hist_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_hist_ok(true));
    assert!(l28_tcp_hist_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_fence_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_fence_ok(true));
    assert!(l28_tcp_fence_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_clear_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_clear_ok(true));
    assert!(l28_tcp_clear_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_pre_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_pre_ok(true));
    assert!(l28_tcp_pre_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_peer_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_peer_ok(true));
    assert!(l28_tcp_peer_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_lid_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_lid_ok(true));
    assert!(l28_tcp_lid_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_rdr_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_rdr_ok(true));
    assert!(l28_tcp_rdr_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_dsc_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_dsc_ok(true));
    assert!(l28_tcp_dsc_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_pld_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_pld_ok(true));
    assert!(l28_tcp_pld_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_std_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_std_ok(true));
    assert!(l28_tcp_std_ok_as_is(false), "AS-IS dente");
}
#[test]
fn l28_tcp_hnt_ok_on_live_queued_is_not_ok() {
    l28_campaign();
    assert!(l28_tcp_hnt_ok(true));
    assert!(l28_tcp_hnt_ok_as_is(false), "AS-IS dente");
}

#[test]
fn l28_real_tcp_is_campaign_not_forall() {
    let _q = LiveQueued::open();
    assert!(
        !world_seed_l28_ok(0, false),
        "World silent_wrong=0 is not L28 without cluster_real"
    );
    assert!(world_seed_l28_ok_as_is(0, false));
}
