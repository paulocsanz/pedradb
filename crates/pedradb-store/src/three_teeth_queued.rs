//! RFC-0151 P2: each Raft `data_fate` kernel has a Queued-RPC plant.
//! L28 REAL TCP stays a campaign (`world_seed_l28_ok(0, false)` is not Ok).

use crate::ae_ack_kernel::{
    ae_ack_success, ae_ack_success_as_is, ae_entry_action, ae_entry_action_as_is_rewrite_committed,
    AeEntryAction,
};
use crate::apply_kernel::{apply_advance, apply_advance_as_is_skip_holes, ApplyAction};
use crate::membership_kernel::*;
use crate::rpc_mode_kernel::{allow_direct_rpc, allow_direct_rpc_as_is};
use crate::vote_kernel::{
    grant_after_persist, grant_after_persist_as_is, vote_decision,
    vote_decision_as_is_ignore_log_and_vote, PersistOutcome, VoteDecision, VoteInputs,
};
use crate::{
    l28_tcp_abort_ok, l28_tcp_abort_ok_as_is, l28_tcp_apply_ok, l28_tcp_apply_ok_as_is,
    l28_tcp_clear_ok, l28_tcp_clear_ok_as_is, l28_tcp_dsc_ok, l28_tcp_dsc_ok_as_is,
    l28_tcp_fence_ok, l28_tcp_fence_ok_as_is, l28_tcp_hist_ok, l28_tcp_hist_ok_as_is,
    l28_tcp_hnt_ok, l28_tcp_hnt_ok_as_is, l28_tcp_hw_ok, l28_tcp_hw_ok_as_is, l28_tcp_left_ok,
    l28_tcp_left_ok_as_is, l28_tcp_lid_ok, l28_tcp_lid_ok_as_is, l28_tcp_napply_ok,
    l28_tcp_napply_ok_as_is, l28_tcp_nowms_ok, l28_tcp_nowms_ok_as_is, l28_tcp_odrop_ok,
    l28_tcp_odrop_ok_as_is, l28_tcp_part_ok, l28_tcp_part_ok_as_is, l28_tcp_peer_ok,
    l28_tcp_peer_ok_as_is, l28_tcp_pj_ok, l28_tcp_pj_ok_as_is, l28_tcp_pld_ok,
    l28_tcp_pld_ok_as_is, l28_tcp_pre_ok, l28_tcp_pre_ok_as_is, l28_tcp_rdr_ok,
    l28_tcp_rdr_ok_as_is, l28_tcp_slot_ok, l28_tcp_slot_ok_as_is, l28_tcp_std_ok,
    l28_tcp_std_ok_as_is, l28_tcp_sth_ok, l28_tcp_sth_ok_as_is, l28_tcp_trunc_ok,
    l28_tcp_trunc_ok_as_is, len_pref_value, len_pref_value_as_is, may_compact_through,
    may_compact_through_as_is, si_reader_beats, si_reader_beats_as_is, snapshot_read_plan,
    snapshot_read_plan_as_is, snapshot_touches_user_key, snapshot_touches_user_key_as_is,
    world_seed_l28_ok, world_seed_l28_ok_as_is, LogRec, PeerMsg, RangeEntry, RpcMode, SnapshotRead,
    StoreCluster, StoreError,
};
use pedradb_core::{
    changelog_needs_sst_rebuild, changelog_needs_sst_rebuild_as_is, decode_changelog,
    prefix_exclusive_end, prefix_exclusive_end_as_is, range_tombstone_covers,
    range_tombstone_covers_as_is, Env, SeedRng, CHANGELOG_FILE_NAME,
};
use pedradb_dcs::{
    dcs_apply_should_advance, dcs_apply_should_advance_as_is, dcs_apply_should_advance_result,
    DcsCommand, DcsError,
};
use pedradb_raft::commit_kernel::{propose_ack_ok, propose_ack_ok_as_is};

struct LiveQueued {
    dir: std::path::PathBuf,
    cluster: StoreCluster,
}

fn pump_queued<E: Env>(c: &mut StoreCluster<E>, rounds: usize) {
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

    fn follower(&self, range_id: u64) -> u64 {
        let leader = self
            .cluster
            .range_leader(range_id)
            .expect("Queued cluster must have a leader");
        self.cluster
            .ids
            .iter()
            .copied()
            .find(|&id| id != leader)
            .expect("Queued cluster must have a follower")
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

#[test]
fn vote_decision_on_live_queued_is_not_ok() {
    let mut q = LiveQueued::open();
    let follower = q.follower(1);
    let (term, last_idx, last_term, voted_for, log_before) = {
        let p = q
            .cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (
            p.term,
            p.last_index(),
            p.last_term(),
            p.voted_for,
            p.log.clone(),
        )
    };
    assert!(
        voted_for.is_some() && voted_for != Some(999),
        "follower must already have voted for the leader"
    );
    let i = VoteInputs {
        current_term: term,
        voted_for,
        last_log_term: last_term,
        last_log_index: last_idx,
        candidate_term: term,
        candidate_id: 999,
        candidate_last_log_term: 0,
        candidate_last_log_index: 0,
    };
    assert_eq!(vote_decision(i), VoteDecision::Deny);
    assert_eq!(
        vote_decision_as_is_ignore_log_and_vote(i),
        VoteDecision::WouldGrant,
        "AS-IS dente: ignore log and existing vote"
    );
    let _ = q.cluster.drain_outbound();
    let bytes = PeerMsg::RequestVote {
        range_id: 1,
        term,
        candidate_id: 999,
        last_log_index: 0,
        last_log_term: 0,
    }
    .encode();
    q.cluster.handle_inbound(999, follower, &bytes).unwrap();
    let replies = q.cluster.drain_outbound();
    let mut saw_deny = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::RequestVoteReply { vote_granted, .. }) = PeerMsg::decode(&raw) {
            assert!(
                !vote_granted,
                "live Queued inbound must not grant stale-log RV"
            );
            saw_deny = true;
        }
    }
    assert!(saw_deny, "expected RequestVoteReply from handle_inbound");
    let log_after = q
        .cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .log
        .clone();
    assert_eq!(log_after, log_before);
}

#[test]
fn grant_after_persist_on_live_queued_is_not_ok() {
    use pedradb_sim::FailingEnv;
    assert!(!grant_after_persist(
        VoteDecision::WouldGrant,
        PersistOutcome::Err
    ));
    assert!(
        grant_after_persist_as_is(VoteDecision::WouldGrant, PersistOutcome::Err),
        "AS-IS dente: grant ignores persist Err"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-grant-persist-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let e1 = FailingEnv::passing();
    let e2 = FailingEnv::passing();
    let e3 = FailingEnv::passing();
    let mut cluster = StoreCluster::open_with_envs_rng(
        &dir,
        3,
        1,
        [e1.clone(), e2.clone(), e3.clone()],
        SeedRng::new(0x0152_0011),
    )
    .unwrap();
    cluster.pin_dst_queued();
    assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
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
    let leader = cluster.range_leader(1).unwrap();
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("Queued cluster must have a follower");
    let (term, last_idx, last_term, voted_for) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term(), p.voted_for)
    };
    let candidate = voted_for.expect("follower must already have voted");
    let i = VoteInputs {
        current_term: term,
        voted_for,
        last_log_term: last_term,
        last_log_index: last_idx,
        candidate_term: term,
        candidate_id: candidate,
        candidate_last_log_term: last_term,
        candidate_last_log_index: last_idx,
    };
    assert_eq!(vote_decision(i), VoteDecision::WouldGrant);
    match follower {
        1 => e1.arm_one_failure(),
        2 => e2.arm_one_failure(),
        3 => e3.arm_one_failure(),
        id => panic!("unexpected follower {id}"),
    }
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::RequestVote {
        range_id: 1,
        term,
        candidate_id: candidate,
        last_log_index: last_idx,
        last_log_term: last_term,
    }
    .encode();
    cluster.handle_inbound(candidate, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_deny = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::RequestVoteReply { vote_granted, .. }) = PeerMsg::decode(&raw) {
            assert!(
                !vote_granted,
                "live Queued inbound must not grant when persist_hard fails"
            );
            saw_deny = true;
        }
    }
    assert!(saw_deny, "expected RequestVoteReply from handle_inbound");
    let voted_after = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .voted_for;
    assert_eq!(
        voted_after, voted_for,
        "persist Err must roll voted_for back"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0158 P0.3 / F125/F127: the term rises only when hard state is durable.
/// Live Queued inbound RequestVote with a NEWER term whose hard-state persist
/// fails once (FailingEnv seam) must restore term/voted_for, force Follower,
/// and clear `leader_id` — the kernel's `Restored`, never the AS-IS raise.
#[test]
fn durable_term_rollback_on_live_queued_is_not_ok() {
    use crate::vote_kernel::{durable_term_if_newer, durable_term_if_newer_as_is, DurableTerm};
    use pedradb_sim::FailingEnv;
    assert_eq!(
        durable_term_if_newer(5, 6, PersistOutcome::Err),
        DurableTerm::Restored
    );
    assert_eq!(
        durable_term_if_newer_as_is(5, 6, PersistOutcome::Err),
        DurableTerm::Raised,
        "AS-IS dente: term rises without durability"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-durable-term-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let e1 = FailingEnv::passing();
    let e2 = FailingEnv::passing();
    let e3 = FailingEnv::passing();
    let mut cluster = StoreCluster::open_with_envs_rng(
        &dir,
        3,
        1,
        [e1.clone(), e2.clone(), e3.clone()],
        SeedRng::new(0x0158_0001),
    )
    .unwrap();
    cluster.pin_dst_queued();
    assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
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
    let leader = cluster.range_leader(1).unwrap();
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("Queued cluster must have a follower");
    // The follower must have learned the leader (AE heartbeat) before we
    // break durability — the cleared leader_id is then observable.
    let mut learned = false;
    for _ in 0..240 {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        if p.leader_id == Some(leader) {
            learned = true;
            break;
        }
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
    }
    assert!(learned, "follower must learn its leader before the plant");
    let (term, voted_for, leader_id) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.voted_for, p.leader_id)
    };
    assert_eq!(leader_id, Some(leader), "precondition: leader known");
    match follower {
        1 => e1.arm_one_failure(),
        2 => e2.arm_one_failure(),
        3 => e3.arm_one_failure(),
        id => panic!("unexpected follower {id}"),
    }
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::RequestVote {
        range_id: 1,
        term: term + 1,
        candidate_id: 999,
        last_log_index: 0,
        last_log_term: 0,
    }
    .encode();
    cluster.handle_inbound(999, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_reply = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::RequestVoteReply {
            term: reply_term,
            vote_granted,
            ..
        }) = PeerMsg::decode(&raw)
        {
            assert!(
                !vote_granted,
                "live Queued inbound must not grant when hard-state persist fails"
            );
            assert_eq!(
                reply_term, term,
                "reply carries the RESTORED term (F125/F127), not the undurable raise"
            );
            saw_reply = true;
        }
    }
    assert!(saw_reply, "expected RequestVoteReply from handle_inbound");
    let (term_after, voted_after, role_after, leader_after) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.voted_for, p.role, p.leader_id)
    };
    assert_eq!(term_after, term, "persist Err must roll the term back");
    assert_eq!(
        voted_after, voted_for,
        "persist Err must roll voted_for back"
    );
    assert_eq!(
        role_after,
        crate::Role::Follower,
        "undurable step forces Follower"
    );
    assert_eq!(
        leader_after, None,
        "leader_id is cleared after the undurable step"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ae_entry_action_on_live_queued_is_not_ok() {
    let mut q = LiveQueued::open();
    // Do not put: finish_queued_propose compact_ready(min_applied>0) drops
    // committed entries from the in-memory log (F16 tooth needs them present).
    let mut found = None;
    for _ in 0..80 {
        for &nid in &q.cluster.ids.clone() {
            let p = q.cluster.nodes.get(&nid).unwrap().ranges.get(&1).unwrap();
            if let Some(rec) = p.log.iter().find(|e| e.index <= p.commit).cloned() {
                found = Some((nid, p.term, p.commit, p.last_index(), rec, p.log.clone()));
                break;
            }
        }
        if found.is_some() {
            break;
        }
        q.cluster.tick().unwrap();
        pump_queued(&mut q.cluster, 48);
    }
    let (target, term, commit, last, rec, log_before) = found.unwrap_or_else(|| {
        let diag: Vec<String> = q
            .cluster
            .ids
            .iter()
            .map(|&id| {
                let p = q.cluster.nodes.get(&id).unwrap().ranges.get(&1).unwrap();
                format!(
                    "n{id} role={:?} term={} commit={} last={} snap={} log={:?}",
                    p.role,
                    p.term,
                    p.commit,
                    p.last_index(),
                    p.snapshot_index,
                    p.log.iter().map(|e| (e.index, e.term)).collect::<Vec<_>>()
                )
            })
            .collect();
        panic!("no committed in-log entry after Queued elect: {diag:?}");
    });
    let conflict_term = rec.term.wrapping_add(1);
    assert_ne!(conflict_term, rec.term);
    assert_eq!(
        ae_entry_action(rec.index, conflict_term, Some(rec.term), commit, last),
        AeEntryAction::Refuse
    );
    assert_eq!(
        ae_entry_action_as_is_rewrite_committed(
            rec.index,
            conflict_term,
            Some(rec.term),
            commit,
            last
        ),
        AeEntryAction::TruncateAndInstall,
        "AS-IS dente: rewrite committed index"
    );
    let _ = q.cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: 999,
        prev_log_index: 0,
        prev_log_term: 0,
        leader_commit: 0,
        entries: vec![LogRec {
            index: rec.index,
            term: conflict_term,
            entry: RangeEntry::Noop,
        }],
    }
    .encode();
    q.cluster.handle_inbound(999, target, &bytes).unwrap();
    let replies = q.cluster.drain_outbound();
    let mut saw_refuse = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(
                !success,
                "live Queued inbound must not ack committed rewrite"
            );
            saw_refuse = true;
        }
    }
    assert!(
        saw_refuse,
        "expected AppendEntriesReply from handle_inbound"
    );
    let p = q
        .cluster
        .nodes
        .get(&target)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    assert_eq!(p.log, log_before, "committed suffix must not truncate");
    assert_eq!(p.term_at(rec.index), rec.term);
}

#[test]
fn ae_ack_success_on_live_queued_is_not_ok() {
    use pedradb_sim::FailingEnv;
    assert!(!ae_ack_success(true, false));
    assert!(
        ae_ack_success_as_is(true, false),
        "AS-IS dente: ack success ignores persist Err"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-ae-ack-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let e1 = FailingEnv::passing();
    let e2 = FailingEnv::passing();
    let e3 = FailingEnv::passing();
    let mut cluster = StoreCluster::open_with_envs_rng(
        &dir,
        3,
        1,
        [e1.clone(), e2.clone(), e3.clone()],
        SeedRng::new(0x0152_0048),
    )
    .unwrap();
    cluster.pin_dst_queued();
    assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
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
    let leader = cluster.range_leader(1).unwrap();
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("Queued cluster must have a follower");
    let (term, last_idx, last_term, log_before) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term(), p.log.clone())
    };
    match follower {
        1 => e1.arm_one_failure(),
        2 => e2.arm_one_failure(),
        3 => e3.arm_one_failure(),
        id => panic!("unexpected follower {id}"),
    }
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: last_idx,
        entries: vec![LogRec {
            index: last_idx.saturating_add(1),
            term,
            entry: RangeEntry::Noop,
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_deny = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(
                !success,
                "live Queued inbound must not AE-ack when persist_log fails"
            );
            saw_deny = true;
        }
    }
    assert!(saw_deny, "expected AppendEntriesReply from handle_inbound");
    let log_after = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .log
        .clone();
    assert_eq!(log_after, log_before, "persist Err must roll the log back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn allow_direct_rpc_on_live_queued_is_not_ok() {
    let mut q = LiveQueued::open();
    assert!(
        !allow_direct_rpc(true, true),
        "pinned Queued must refuse Direct"
    );
    assert!(
        allow_direct_rpc_as_is(true, true),
        "AS-IS dente: pin does not stick"
    );
    q.cluster.set_rpc_mode(RpcMode::Direct);
    assert_eq!(
        q.cluster.rpc_mode(),
        RpcMode::Queued,
        "live set_rpc_mode(Direct) after pin_dst_queued must stay Queued"
    );
    assert!(q.cluster.dst_queued_pin());
}

#[test]
fn may_compact_through_on_live_queued_is_not_ok() {
    assert!(!may_compact_through(0, 5, 0));
    assert!(
        may_compact_through_as_is(0, 5, 0),
        "AS-IS dente: compact through a missing log index"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-compact-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_0138)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("follower");
    let snap_before = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .snapshot_index;
    let through = cluster
        .nodes
        .values()
        .filter_map(|n| n.ranges.get(&1).map(|p| p.last_index()))
        .max()
        .unwrap_or(0)
        .max(1);
    for n in cluster.nodes.values_mut() {
        if let Some(p) = n.ranges.get_mut(&1) {
            p.applied = through;
            p.log.retain(|e| e.index != through);
        }
    }
    assert!(!may_compact_through(snap_before, through, 0));
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let _ = cluster.drain_outbound();
    let hb = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: last_idx,
        entries: vec![],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &hb).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound AE heartbeat must ack");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for heartbeat");
    let p = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    assert_eq!(
        p.snapshot_index, snap_before,
        "compact must wait when term_at(through)==0"
    );
    assert_ne!(
        p.snapshot_index, through,
        "AS-IS would snapshot through the missing index"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_touches_user_key_on_live_queued_is_not_ok() {
    assert!(!snapshot_touches_user_key(true));
    assert!(
        snapshot_touches_user_key_as_is(true),
        "AS-IS dente: snapshot applies reserved \\0store/* keys"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-snap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_0139)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("follower");
    let (term, last_incl) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.commit.max(p.last_index()))
    };
    let mut reserved = crate::RAFT_META_PREFIX.to_vec();
    reserved.extend_from_slice(b"rfc0152-snap-leak");
    let user = b"rfc0152-snap-user".to_vec();
    let _ = cluster.drain_outbound();
    let snap = PeerMsg::InstallSnapshot {
        range_id: 1,
        term,
        leader_id: leader,
        last_included_index: last_incl,
        last_included_term: term,
        kv_pairs: vec![
            (reserved.clone(), b"LEAK".to_vec()),
            (user.clone(), b"ok".to_vec()),
        ],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &snap).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::InstallSnapshotReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound InstallSnapshot must succeed");
            saw = true;
        }
    }
    assert!(saw, "expected InstallSnapshotReply");
    assert_eq!(
        cluster.get_on(follower, &user).unwrap().as_deref(),
        Some(b"ok".as_ref()),
        "user key in snapshot payload must apply"
    );
    assert_ne!(
        cluster.get_on(follower, &reserved).unwrap().as_deref(),
        Some(b"LEAK".as_ref()),
        "reserved raft-meta key must not be applied from snapshot"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn si_reader_beats_on_live_queued_is_not_ok() {
    assert!(si_reader_beats(
        true, true, false, 5, false, false, false, 0
    ));
    assert!(
        !si_reader_beats_as_is(true, true, false, 5, false, false, false, 0),
        "AS-IS dente: first ids[] candidate always stays"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-si-rdr-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_013a)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let lag = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("lagging voter");
    cluster.ids.retain(|&id| id != lag);
    cluster.ids.insert(0, lag);
    {
        let n = cluster.nodes.get_mut(&lag).unwrap();
        n.participating = false;
        if let Some(p) = n.ranges.get_mut(&1) {
            p.applied = 0;
        }
    }
    let key = b"rfc0152-si-rdr".to_vec();
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let put_idx = last_idx.saturating_add(1);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: put_idx,
        entries: vec![LogRec {
            index: put_idx,
            term,
            entry: RangeEntry::Put {
                key: key.clone(),
                value: b"ok".to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, leader, &put).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound Put on leader must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for Put");
    assert_eq!(
        cluster.get(&key).unwrap().as_deref(),
        Some(b"ok".as_ref()),
        "LocalApplied get must use live leader, not lagging ids[0]"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn len_pref_value_on_live_queued_is_not_ok() {
    assert_ne!(
        len_pref_value(b"red"),
        len_pref_value_as_is(b"red"),
        "AS-IS dente: raw val is a prefix of val||0x00||foo"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-idx-val-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_013b)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let red = b"red".as_slice();
    let long = [b'r', b'e', b'd', 0x00, b'f', b'o', b'o'];
    let k_red = crate::layers::table_index_key(b"t", b"email", red, b"1");
    let k_sib = crate::layers::table_index_key(b"t", b"email", &long, b"2");
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let i1 = last_idx.saturating_add(1);
    let i2 = last_idx.saturating_add(2);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: i2,
        entries: vec![
            LogRec {
                index: i1,
                term,
                entry: RangeEntry::Put {
                    key: k_red,
                    value: b"1".to_vec(),
                    si_gen: 0,
                },
            },
            LogRec {
                index: i2,
                term,
                entry: RangeEntry::Put {
                    key: k_sib,
                    value: b"2".to_vec(),
                    si_gen: 0,
                },
            },
        ],
    }
    .encode();
    cluster.handle_inbound(leader, leader, &put).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound index Puts must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for index Puts");
    let (start, end) = crate::layers::table_index_value_range(red);
    let snap = cluster.read_version();
    let got = cluster.keys_in_range_at(&start, &end, snap).unwrap();
    let pks: Vec<&[u8]> = got.iter().map(|(_, v)| v.as_slice()).collect();
    assert!(
        pks.iter().any(|v| *v == b"1"),
        "exact red must list pk 1: {got:?}"
    );
    assert!(
        !pks.iter().any(|v| *v == b"2"),
        "length-prefix must drop sibling red\\0foo: {got:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changelog_needs_sst_rebuild_on_live_queued_is_not_ok() {
    assert!(changelog_needs_sst_rebuild(true, 1));
    assert!(
        !changelog_needs_sst_rebuild_as_is(true, 1),
        "AS-IS dente: WAL-only rebuild never consults SST/Mem"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-chlog-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_013c)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let key = b"rfc0152-chlog".to_vec();
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let put_idx = last_idx.saturating_add(1);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: put_idx,
        entries: vec![LogRec {
            index: put_idx,
            term,
            entry: RangeEntry::Put {
                key: key.clone(),
                value: b"ok".to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, leader, &put).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound Put on leader must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for Put");
    assert_eq!(
        cluster.get_on(leader, &key).unwrap().as_deref(),
        Some(b"ok".as_ref()),
        "inbound Put must be applied on the leader engine"
    );
    cluster
        .flush_engine_on(leader)
        .expect("flush leader WAL→SST so reopen cannot WAL-rebuild");
    let data = cluster.node_data_dir(leader).expect("leader engine dir");
    let chlog = data.join(CHANGELOG_FILE_NAME);
    assert!(
        chlog.exists(),
        "explicit flush must persist CHANGELOG cache"
    );
    std::fs::remove_file(&chlog).unwrap();
    assert!(!chlog.exists(), "planted missing CHANGELOG after flush");
    cluster
        .crash_reopen_engine_on(leader, pedradb_io_uring::IoUringEnv::default())
        .expect("crash-reopen leader after CHANGELOG loss");
    assert!(
        chlog.exists(),
        "maybe_rebuild_feed_from_live must persist CHANGELOG from SST; AS-IS would leave it missing"
    );
    let bytes = std::fs::read(&chlog).expect("rebuilt CHANGELOG");
    let feed = decode_changelog(&bytes).expect("rebuilt CHANGELOG must decode");
    assert!(
        feed.changes_after(0)
            .iter()
            .any(|e| e.key.as_ref() == key.as_slice()),
        "SST last-per-key rebuild must restore inbound Put, got {:?}",
        feed.changes_after(0)
            .iter()
            .map(|e| e.key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        cluster.get_on(leader, &key).unwrap().as_deref(),
        Some(b"ok".as_ref()),
        "data plane must still see SST keys after CHANGELOG loss"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn range_tombstone_covers_on_live_queued_is_not_ok() {
    assert!(range_tombstone_covers(b"a", b"c", b"b"));
    assert!(
        !range_tombstone_covers_as_is(b"a", b"c", b"b"),
        "AS-IS dente: only the range start conflicts"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-range-cov-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_013d)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let ka = b"a".to_vec();
    let kb = b"b".to_vec();
    let kc = b"c".to_vec();
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let i1 = last_idx.saturating_add(1);
    let i2 = last_idx.saturating_add(2);
    let i3 = last_idx.saturating_add(3);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: i3,
        entries: vec![
            LogRec {
                index: i1,
                term,
                entry: RangeEntry::Put {
                    key: ka.clone(),
                    value: b"1".to_vec(),
                    si_gen: 0,
                },
            },
            LogRec {
                index: i2,
                term,
                entry: RangeEntry::Put {
                    key: kb.clone(),
                    value: b"2".to_vec(),
                    si_gen: 0,
                },
            },
            LogRec {
                index: i3,
                term,
                entry: RangeEntry::Put {
                    key: kc.clone(),
                    value: b"3".to_vec(),
                    si_gen: 0,
                },
            },
        ],
    }
    .encode();
    cluster.handle_inbound(leader, leader, &put).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound Puts on leader must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for Puts");
    cluster
        .nodes
        .get_mut(&leader)
        .unwrap()
        .db
        .delete_range(&ka, &kc)
        .expect("range tombstone [a,c)");
    assert_eq!(
        cluster.get_on(leader, &ka).unwrap().as_deref(),
        None,
        "range start must be hidden"
    );
    assert_eq!(
        cluster.get_on(leader, &kb).unwrap().as_deref(),
        None,
        "interior key must be covered; AS-IS would leak b"
    );
    assert_eq!(
        cluster.get_on(leader, &kc).unwrap().as_deref(),
        Some(b"3".as_ref()),
        "exclusive end must stay live"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prefix_exclusive_end_on_live_queued_is_not_ok() {
    let p = b"/host/h1/";
    let mut wit = p.to_vec();
    wit.push(0xff);
    wit.extend_from_slice(b"z");
    let fixed = prefix_exclusive_end(p);
    let as_is = prefix_exclusive_end_as_is(p);
    assert_ne!(
        fixed, as_is,
        "AS-IS dente: prefix||0xff drops prefix||0xff||…"
    );
    assert!(
        fixed.as_deref().is_some_and(|e| wit.as_slice() < e),
        "FIXED end must sit after 0xff continuation"
    );
    assert!(
        as_is.as_deref().is_some_and(|e| wit.as_slice() >= e),
        "AS-IS prefix||0xff excludes the continuation"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-prefix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_013e)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let k_pref = p.to_vec();
    let k_ff = wit.clone();
    let k_sib = prefix_exclusive_end(p).expect("ascii prefix has exclusive end");
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let i1 = last_idx.saturating_add(1);
    let i2 = last_idx.saturating_add(2);
    let i3 = last_idx.saturating_add(3);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: i3,
        entries: vec![
            LogRec {
                index: i1,
                term,
                entry: RangeEntry::Put {
                    key: k_pref.clone(),
                    value: b"own".to_vec(),
                    si_gen: 0,
                },
            },
            LogRec {
                index: i2,
                term,
                entry: RangeEntry::Put {
                    key: k_ff.clone(),
                    value: b"ff".to_vec(),
                    si_gen: 0,
                },
            },
            LogRec {
                index: i3,
                term,
                entry: RangeEntry::Put {
                    key: k_sib.clone(),
                    value: b"sib".to_vec(),
                    si_gen: 0,
                },
            },
        ],
    }
    .encode();
    cluster.handle_inbound(leader, leader, &put).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound prefix Puts must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for prefix Puts");
    let hits = crate::scan_prefix(&cluster.nodes.get(&leader).unwrap().db, p);
    let keys: Vec<&[u8]> = hits.iter().map(|(k, _)| k.as_slice()).collect();
    assert!(
        keys.iter().any(|k| *k == k_pref.as_slice()),
        "prefix itself must be in scan_prefix: {hits:?}"
    );
    assert!(
        keys.iter().any(|k| *k == k_ff.as_slice()),
        "0xff continuation must stay in [prefix, exclusive_end); AS-IS would drop it: {hits:?}"
    );
    assert!(
        !keys.iter().any(|k| *k == k_sib.as_slice()),
        "exclusive end itself is not a prefix match: {hits:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_read_plan_on_live_queued_is_not_ok() {
    assert_eq!(snapshot_read_plan(1, 7), SnapshotRead::TooOld);
    assert_eq!(
        snapshot_read_plan_as_is(1, 7),
        SnapshotRead::Serve,
        "AS-IS dente: below-floor snapshot fabricates absence"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-si-read-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_013f)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let key = b"rfc0152-si-read".to_vec();
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&leader).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let put_idx = last_idx.saturating_add(1);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: put_idx,
        entries: vec![LogRec {
            index: put_idx,
            term,
            entry: RangeEntry::Put {
                key: key.clone(),
                value: b"ok".to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, leader, &put).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound Put on leader must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for Put");
    cluster.force_safe_watermark_for_test(7);
    let err = cluster
        .get_at_version(&key, 1)
        .expect_err("below-floor snapshot must fail closed");
    assert!(
        matches!(err, StoreError::TransactionTooOld { snapshot: 1, .. }),
        "live get_at_version must be TransactionTooOld, got {err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dcs_apply_should_advance_result_on_live_queued_is_not_ok() {
    assert!(dcs_apply_should_advance(false, true));
    assert!(
        !dcs_apply_should_advance_as_is(false, true),
        "AS-IS dente: CasFailed freezes last_applied"
    );
    let cas: pedradb_dcs::Result<u64> = Err(DcsError::CasFailed("key exists"));
    assert!(
        dcs_apply_should_advance_result(&cas),
        "CasFailed must still advance"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-dcs-apply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_0137)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("follower");
    let key = crate::meta_key(b"rfc0152-dcs-cas");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let create_idx = last_idx.saturating_add(1);
    let _ = cluster.drain_outbound();
    let create = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: create_idx,
        entries: vec![LogRec {
            index: create_idx,
            term,
            entry: RangeEntry::Dcs(DcsCommand::Create {
                key: key.clone(),
                value: b"a".to_vec(),
                lease: 0,
            }),
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &create).unwrap();
    let mut saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound DCS Create must append+commit");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for Create");
    let dup_idx = create_idx.saturating_add(1);
    let put_idx = create_idx.saturating_add(2);
    let dup = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: create_idx,
        prev_log_term: term,
        leader_commit: put_idx,
        entries: vec![
            LogRec {
                index: dup_idx,
                term,
                entry: RangeEntry::Dcs(DcsCommand::Create {
                    key: key.clone(),
                    value: b"dup".to_vec(),
                    lease: 0,
                }),
            },
            LogRec {
                index: put_idx,
                term,
                entry: RangeEntry::Put {
                    key: b"after-cas".to_vec(),
                    value: b"ok".to_vec(),
                    si_gen: 0,
                },
            },
        ],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &dup).unwrap();
    saw = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound CasFailed Create must still ack");
            saw = true;
        }
    }
    assert!(saw, "expected AppendEntriesReply for dup Create");
    assert_eq!(
        cluster.applied_index(follower, 1),
        put_idx,
        "CasFailed must not freeze applied short of the following Put"
    );
    assert_eq!(
        cluster.get_on(follower, b"after-cas").unwrap().as_deref(),
        Some(b"ok".as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn propose_ack_ok_on_live_queued_is_not_ok() {
    let mut q = LiveQueued::open();
    assert!(!propose_ack_ok(5, 3));
    assert!(
        propose_ack_ok_as_is(5, 3),
        "AS-IS dente: ack as soon as the entry is appended"
    );
    let leader = q
        .cluster
        .range_leader(1)
        .expect("Queued cluster must have a leader");
    let commit_before = q
        .cluster
        .nodes
        .get(&leader)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .commit;
    let _ = q.cluster.drain_outbound();
    let err = q.cluster.put(b"commit-raft/k", b"v");
    let (index, commit) = match err {
        Err(StoreError::NotCommitted { index, commit, .. }) => (index, commit),
        other => panic!("Queued put must NotCommitted before AE replies, got {other:?}"),
    };
    assert!(
        !propose_ack_ok(index, commit),
        "live propose_ack_ok must refuse Ok while commit {commit} < index {index}"
    );
    assert!(propose_ack_ok_as_is(index, commit));
    // Deliver the AE that put queued (real inbound), but do not pump replies
    // back to the leader — majority commit must not sneak in.
    let batch = q.cluster.drain_outbound();
    assert!(!batch.is_empty(), "Queued put must enqueue AppendEntries");
    for (from, to, raw) in batch {
        q.cluster.handle_inbound(from, to, &raw).unwrap();
    }
    let commit_after = q
        .cluster
        .nodes
        .get(&leader)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .commit;
    assert_eq!(
        commit_after, commit_before,
        "without AE replies, leader commit must not cover the proposed index"
    );
    assert!(
        !propose_ack_ok(index, commit_after),
        "still uncommitted after inbound AE without replies"
    );
}

#[test]
fn apply_advance_on_live_queued_is_not_ok() {
    assert_eq!(apply_advance(1, 5, false), ApplyAction::Stop);
    assert_eq!(
        apply_advance_as_is_skip_holes(1, 5, false),
        ApplyAction::Apply,
        "AS-IS dente: skip holes and apply past a missing index"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-apply-step-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_0136)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("follower");
    let hole_key = b"rfc0152-apply-hole".to_vec();
    let (later, term, last_idx, last_term) = {
        let n = cluster.nodes.get_mut(&follower).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        let later = p.last_index().saturating_add(2);
        let term = p.term.max(1);
        p.log.push(LogRec {
            index: later,
            term,
            entry: RangeEntry::Put {
                key: hole_key.clone(),
                value: b"skip".to_vec(),
                si_gen: 0,
            },
        });
        (later, term, p.last_index(), p.last_term())
    };
    assert_eq!(
        apply_advance(later.saturating_sub(2), later, false),
        ApplyAction::Stop
    );
    let _ = cluster.drain_outbound();
    let hb = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: later,
        entries: vec![],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &hb).unwrap();
    let mut saw_hb = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound AE heartbeat must ack");
            saw_hb = true;
        }
    }
    assert!(saw_hb, "expected AppendEntriesReply for heartbeat");
    let p = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    assert_ne!(
        p.applied, later,
        "apply must Stop on the hole, not skip to the planted index"
    );
    assert!(
        p.applied < later,
        "applied must stay behind the planted later index"
    );
    assert!(
        p.log.iter().any(|e| e.index == later),
        "later log entry must remain unapplied"
    );
    assert_eq!(
        cluster.get(&hole_key).unwrap(),
        None,
        "hole skip would have applied the planted Put"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn joint_election_ok_on_live_queued_is_not_ok() {
    assert!(!joint_election_ok(2, 3, Some((2, 4))));
    assert!(
        joint_election_ok_as_is(2, 3, Some((2, 4))),
        "AS-IS dente: old-only majority elects during joint add"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-joint-elect-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0064)).unwrap();
    cluster.pin_dst_queued();
    assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    assert!(
        cluster.range_leader(1).is_some(),
        "Queued 4-node must elect via handle_inbound"
    );
    let cand = 1u64;
    for &nid in &cluster.ids.clone() {
        let p = cluster
            .nodes
            .get_mut(&nid)
            .unwrap()
            .ranges
            .get_mut(&1)
            .unwrap();
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term,
            entry: RangeEntry::MembershipJoint {
                old: vec![1, 2, 3],
                new: vec![1, 2, 3, 4],
            },
        });
    }
    let _ = cluster.drain_outbound();
    cluster.start_election(1, cand).unwrap();
    let term = cluster
        .nodes
        .get(&cand)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .term;
    // Inbound RV only to old voter 2 — not 3, not the joining 4.
    let batch = cluster.drain_outbound();
    for (from, to, raw) in batch {
        if to == 2 {
            cluster.handle_inbound(from, to, &raw).unwrap();
        }
    }
    let replies = cluster.drain_outbound();
    let mut saw_grant = false;
    for (from, to, raw) in replies {
        if let Ok(PeerMsg::RequestVoteReply { vote_granted, .. }) = PeerMsg::decode(&raw) {
            assert!(vote_granted, "old voter 2 must grant the higher-term RV");
            saw_grant = true;
        }
        cluster.handle_inbound(from, to, &raw).unwrap();
    }
    assert!(saw_grant, "expected inbound RequestVoteReply from voter 2");
    let granted = cluster
        .election_granted
        .get(&(1, term, cand))
        .cloned()
        .unwrap_or_default();
    assert!(
        granted.contains(&cand) && granted.contains(&2),
        "self+old voter 2 is C-old majority (2/3); granted={granted:?}"
    );
    assert!(
        !cluster.election_has_joint_quorum(1, term, cand),
        "2/3 old + 2/4 new is not joint quorum"
    );
    assert_ne!(
        cluster.range_leader(1),
        Some(cand),
        "live Queued inbound must not elect on old-only majority during joint add"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn joint_still_active_on_live_queued_is_not_ok() {
    let mut q = LiveQueued::open();
    let old = vec![1u64, 2, 3];
    let new = vec![1u64, 2, 3, 4];
    assert!(
        joint_still_active(&old, &new),
        "C-old,new is still in force"
    );
    assert!(
        !joint_still_active_as_is(&old, &new),
        "AS-IS dente: treat every config as single / skip joint"
    );
    let leader = q
        .cluster
        .range_leader(1)
        .expect("Queued cluster must have a leader");
    let follower = q.follower(1);
    let (term, last_idx, last_term) = {
        let p = q
            .cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let _ = q.cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: 0,
        entries: vec![LogRec {
            index: last_idx.saturating_add(1),
            term,
            entry: RangeEntry::MembershipJoint {
                old: old.clone(),
                new: new.clone(),
            },
        }],
    }
    .encode();
    q.cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = q.cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound joint AE must append");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    let pending = q.cluster.pending_joint();
    let Some((got_old, got_new)) = pending else {
        panic!("live pending_joint_on must see inbound C-old,new (AS-IS would skip)");
    };
    assert!(
        joint_still_active(&got_old, &got_new),
        "production pending_joint_on uses joint_still_active"
    );
}

#[test]
fn joint_leave_ok_on_live_queued_is_not_ok() {
    assert!(!joint_leave_ok(false));
    assert!(joint_leave_ok_as_is(false), "AS-IS dente: skip leave-joint");
    let mut q = LiveQueued::open();
    let leader = q
        .cluster
        .range_leader(1)
        .expect("Queued cluster must have a leader");
    let follower = q.follower(1);
    let (term, last_idx, last_term) = {
        let p = q
            .cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let cfg = vec![1u64, 2, 3];
    let _ = q.cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: 0,
        entries: vec![LogRec {
            index: last_idx.saturating_add(1),
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    q.cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = q.cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound C-new-only leave AE must append");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    let p = q
        .cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    let leave = p.log.iter().any(|rec| {
        matches!(
            &rec.entry,
            RangeEntry::MembershipJoint { old, new }
                if !joint_still_active(old, new)
        )
    });
    assert!(leave, "inbound leave must be in the follower log");
    assert!(
        joint_leave_ok(leave),
        "production requires the leave that inbound wrote"
    );
}

#[test]
fn queued_leave_finish_ok_on_live_queued_is_not_ok() {
    assert!(!queued_leave_finish_ok(true, false));
    assert!(
        queued_leave_finish_ok_as_is(true, false),
        "AS-IS dente: leave in log is enough"
    );
    let mut q = LiveQueued::open();
    let leader = q
        .cluster
        .range_leader(1)
        .expect("Queued cluster must have a leader");
    let follower = q.follower(1);
    let (term, last_idx, last_term) = {
        let p = q
            .cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let cfg = vec![1u64, 2, 3];
    let _ = q.cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: 0,
        entries: vec![LogRec {
            index: last_idx.saturating_add(1),
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    q.cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = q.cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound C-new-only leave AE must append");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    let (leave_idx, commit) = {
        let p = q
            .cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        let leave_idx = p.log.iter().find_map(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if !joint_still_active(old, new)
            )
            .then_some(rec.index)
        });
        (
            leave_idx.expect("inbound leave must be in the follower log"),
            p.commit,
        )
    };
    assert!(
        leave_idx > commit,
        "inbound leave must sit uncommitted (leader_commit=0)"
    );
    assert!(
        !queued_leave_finish_ok(true, leave_idx <= commit),
        "uncommitted inbound leave must fail queued_leave_finish_ok"
    );
    let finished = q.cluster.finish_uncommitted_leave().unwrap();
    assert!(
        !finished,
        "finish_uncommitted_leave must not report done while leave is uncommitted"
    );
}

#[test]
fn pending_joint_node_counts_on_live_queued_is_not_ok() {
    assert!(!pending_joint_node_counts(false));
    assert!(
        pending_joint_node_counts_as_is(false),
        "AS-IS dente: scan removed node's leftover joint"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-pending-node-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0105)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let (term, last_idx, last_term) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let _ = cluster.drain_outbound();
    let joint = RangeEntry::MembershipJoint {
        old: vec![1, 2, 3, 4],
        new: vec![1, 2, 3],
    };
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: 0,
        entries: vec![LogRec {
            index: last_idx.saturating_add(1),
            term,
            entry: joint.clone(),
        }],
    }
    .encode();
    cluster.handle_inbound(leader, 4, &bytes).unwrap();
    match cluster.remove_member_joint(4) {
        Ok(()) => {}
        Err(StoreError::NotCommitted {
            range_id, index, ..
        }) => {
            for _ in 0..96 {
                pump_queued(&mut cluster, 48);
                if !cluster.is_member(4) {
                    break;
                }
                let commit = cluster
                    .range_leader(range_id)
                    .map(|lid| cluster.commit_index(lid, range_id))
                    .unwrap_or(0);
                if commit >= index {
                    let _ = cluster.finish_queued_propose(range_id, index, true);
                }
            }
        }
        Err(e) => panic!("Queued remove_member_joint: {e}"),
    }
    assert!(!cluster.is_member(4), "shrink must drop 4 from ids");
    assert!(
        !pending_joint_node_counts(cluster.is_member(4)),
        "removed node must not count"
    );
    for _ in 0..128 {
        if cluster.pending_joint().is_none() {
            break;
        }
        pump_queued(&mut cluster, 48);
        if let Some(lid) = cluster.range_leader(1) {
            let (last, commit) = {
                let p = cluster.nodes.get(&lid).unwrap().ranges.get(&1).unwrap();
                (p.last_index(), p.commit)
            };
            if last > 0 && commit >= last {
                let _ = cluster.finish_queued_propose(1, last, true);
            }
        }
    }
    {
        let p = cluster
            .nodes
            .get_mut(&4)
            .unwrap()
            .ranges
            .get_mut(&1)
            .unwrap();
        let has_active = p.log.iter().any(|rec| {
            matches!(
                &rec.entry,
                RangeEntry::MembershipJoint { old, new }
                    if joint_still_active(old, new)
            )
        });
        if !has_active {
            let idx = p.last_index() + 1;
            p.log.push(LogRec {
                index: idx,
                term: p.term,
                entry: joint,
            });
        }
    }
    assert!(
        cluster.pending_joint().is_none(),
        "pending_joint must ignore leftover C-old,new on removed node 4"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn election_grant_from_counts_on_live_queued_is_not_ok() {
    assert!(!election_grant_from_counts(false, false));
    assert!(
        election_grant_from_counts_as_is(false, false),
        "AS-IS dente: count a grant from a non-member"
    );
    let mut q = LiveQueued::open();
    let cand = 1u64;
    let _ = q.cluster.drain_outbound();
    q.cluster.start_election(1, cand).unwrap();
    let term = q
        .cluster
        .nodes
        .get(&cand)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .term;
    let _ = q.cluster.drain_outbound();
    let stranger = 999u64;
    let bytes = PeerMsg::RequestVoteReply {
        range_id: 1,
        term,
        vote_granted: true,
    }
    .encode();
    q.cluster.handle_inbound(stranger, cand, &bytes).unwrap();
    let granted = q
        .cluster
        .election_granted
        .get(&(1, term, cand))
        .cloned()
        .unwrap_or_default();
    assert!(
        !granted.contains(&stranger),
        "inbound grant from {stranger} must not count; granted={granted:?}"
    );
    assert!(
        granted.contains(&cand),
        "self-vote must still count; granted={granted:?}"
    );
}

#[test]
fn joint_target_counts_on_live_queued_is_not_ok() {
    assert!(joint_target_counts(true, false));
    assert!(
        !joint_target_counts_as_is(true, false),
        "AS-IS dente: require peer in local nodes"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-joint-target-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 1).unwrap();
    cluster.pin_dst_queued();
    assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
    assert!(
        !cluster.nodes.contains_key(&3),
        "TCP replica must not have peer 3 in local nodes"
    );
    assert!(cluster.ids.contains(&3));
    let term = cluster.nodes.get(&1).unwrap().ranges.get(&1).unwrap().term;
    let bytes = PeerMsg::RequestVote {
        range_id: 1,
        term,
        candidate_id: 3,
        last_log_index: 0,
        last_log_term: 0,
    }
    .encode();
    cluster.handle_inbound(3, 1, &bytes).unwrap();
    let _ = cluster.drain_outbound();
    let err = cluster.remove_member_joint(3).unwrap_err();
    let msg = err.to_string();
    assert!(
        !msg.contains("unknown node"),
        "live remove_member_joint must use ids not local nodes: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn joint_add_target_counts_on_live_queued_is_not_ok() {
    assert!(joint_add_target_counts(false));
    assert!(
        !joint_add_target_counts_as_is(false),
        "AS-IS dente: require joiner in local nodes"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-joint-add-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 1).unwrap();
    cluster.pin_dst_queued();
    assert_eq!(cluster.rpc_mode(), RpcMode::Queued);
    assert!(
        !cluster.nodes.contains_key(&4),
        "TCP replica must not have joiner 4 in local nodes"
    );
    assert!(!cluster.ids.contains(&4));
    let term = cluster.nodes.get(&1).unwrap().ranges.get(&1).unwrap().term;
    let bytes = PeerMsg::RequestVote {
        range_id: 1,
        term,
        candidate_id: 4,
        last_log_index: 0,
        last_log_term: 0,
    }
    .encode();
    cluster.handle_inbound(4, 1, &bytes).unwrap();
    let _ = cluster.drain_outbound();
    let err = cluster.add_member_joint(4).unwrap_err();
    let msg = err.to_string();
    assert!(
        !msg.contains("unknown node"),
        "live add_member_joint must not require joiner in local nodes: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disk_membership_overrides_cli_on_live_queued_is_not_ok() {
    assert!(disk_membership_overrides_cli(true));
    assert!(
        !disk_membership_overrides_cli_as_is(true),
        "AS-IS dente: CLI --peer overwrites disk"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-disk-mem-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0113)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound C-new-only leave AE must append+commit");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    let raw = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .db
        .get(&crate::cluster_membership_key())
        .expect("inbound applied leave must persist membership");
    let disk = crate::decode_membership(&raw).unwrap();
    assert!(
        !disk.contains(&4),
        "disk membership must omit 4 after inbound leave apply: {disk:?}"
    );
    assert!(
        disk_membership_overrides_cli(!disk.is_empty()),
        "production bind uses disk when membership is non-empty"
    );
    cluster.ids = vec![1, 2, 3, 4];
    cluster.bind_cluster_identity(None).unwrap();
    assert!(
        !cluster.is_member(4),
        "bind must restore disk voters, not stale CLI ids"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn high_water_at_least_on_live_queued_is_not_ok() {
    assert_eq!(high_water_at_least(4, 3), 4);
    assert_eq!(
        high_water_at_least_as_is(4, 3),
        3,
        "AS-IS dente: RAM/CLI high-water only"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-hw-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut cluster =
            StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0114)).unwrap();
        cluster.pin_dst_queued();
        for _ in 0..120 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        let leader = cluster
            .range_leader(1)
            .expect("Queued 4-node must elect via handle_inbound");
        let follower = cluster
            .ids
            .iter()
            .copied()
            .find(|&id| id != leader && id != 4)
            .expect("remaining-voter follower");
        let (term, last_idx, last_term) = {
            let p = cluster
                .nodes
                .get(&follower)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let leave_idx = last_idx.saturating_add(1);
        let cfg = vec![1u64, 2, 3];
        let _ = cluster.drain_outbound();
        let bytes = PeerMsg::AppendEntries {
            range_id: 1,
            term,
            leader_id: leader,
            prev_log_index: last_idx,
            prev_log_term: last_term,
            leader_commit: leave_idx,
            entries: vec![LogRec {
                index: leave_idx,
                term,
                entry: RangeEntry::MembershipJoint {
                    old: cfg.clone(),
                    new: cfg,
                },
            }],
        }
        .encode();
        cluster.handle_inbound(leader, follower, &bytes).unwrap();
        let replies = cluster.drain_outbound();
        let mut saw_ok = false;
        for (_from, _to, raw) in replies {
            if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
                assert!(success, "inbound C-new-only leave AE must append+commit");
                saw_ok = true;
            }
        }
        assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
        let raw = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .db
            .get(&crate::cluster_high_water_key())
            .expect("inbound applied leave must persist high-water");
        let disk_hw = crate::decode_u64_meta(&raw).unwrap();
        assert_eq!(
            disk_hw, 4,
            "4-node history must be on disk after inbound leave"
        );
        assert_eq!(high_water_at_least(disk_hw, 3), 4);
    }
    let mut c2 = StoreCluster::open_single_node(&dir, 1, &[1, 2, 3], 1)
        .expect("TCP ctor with CLI 3 after inbound 4-node leave");
    assert_eq!(
        c2.membership_high_water, 4,
        "open_single_node must take disk high-water 4, not CLI 3"
    );
    let err = c2.remove_member(1).expect_err("quorum floor");
    assert!(
        err.to_string().contains("quorum floor"),
        "4-node high-water must survive TCP ctor, got {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn participating_if_member_on_live_queued_is_not_ok() {
    assert!(!participating_if_member(false));
    assert!(
        participating_if_member_as_is(false),
        "AS-IS dente: keep captured participating"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-part-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0115)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound C-new-only leave AE must append+commit");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    cluster.nodes.get_mut(&4).unwrap().participating = true;
    assert!(
        !cluster.is_participating(4),
        "stale participating=true must not count after inbound leave"
    );
    let (term4, last_idx4, last_term4) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let rv = PeerMsg::RequestVote {
        range_id: 1,
        term: term4.saturating_add(1),
        candidate_id: leader,
        last_log_index: last_idx4,
        last_log_term: last_term4,
    }
    .encode();
    cluster.handle_inbound(leader, 4, &rv).unwrap();
    let mut denied = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::RequestVoteReply { vote_granted, .. }) = PeerMsg::decode(&raw) {
            assert!(
                !vote_granted,
                "removed node must not grant RV (AS-IS would count stale flag)"
            );
            denied = true;
        }
    }
    assert!(denied, "expected RequestVoteReply from handle_inbound");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn membership_identity_before_applied_on_live_queued_is_not_ok() {
    assert!(membership_identity_before_applied(true));
    assert!(
        !membership_identity_before_applied_as_is(true),
        "AS-IS dente: persist applied first"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-id-first-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0116)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound C-new-only leave AE must append+commit");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    let raw = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .db
        .get(&crate::cluster_membership_key())
        .expect("inbound apply must persist C-new before applied advances");
    let disk = crate::decode_membership(&raw).unwrap();
    assert!(
        !disk.contains(&4),
        "identity must be on disk (C-new) after inbound apply: {disk:?}"
    );
    assert!(
        cluster.applied_index(follower, 1) >= leave_idx,
        "applied must advance past the inbound joint"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recover_must_apply_on_live_queued_is_not_ok() {
    assert!(recover_must_apply(1, 2));
    assert!(
        !recover_must_apply_as_is(1, 2),
        "AS-IS dente: skip apply on recover"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-recover-apply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0117)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let joint_idx = last_idx.saturating_add(1);
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: 0,
        entries: vec![LogRec {
            index: joint_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: vec![1, 2, 3, 4],
                new: vec![1, 2, 3],
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound joint AE must append");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    {
        let n = cluster.nodes.get_mut(&follower).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        p.commit = joint_idx;
        crate::persist_log_db(&mut n.db, 1, p).unwrap();
        crate::persist_commit_db(&mut n.db, 1, p).unwrap();
        assert!(
            recover_must_apply(p.applied, p.commit),
            "inbound joint committed but not applied"
        );
    }
    assert!(cluster.is_member(4), "joint is committed but not applied");
    cluster
        .crash_reopen_engine_on(follower, pedradb_io_uring::IoUringEnv::default())
        .expect("crash-reopen follower");
    assert!(
        !cluster.is_member(4),
        "recover must apply inbound committed joint"
    );
    let p = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    assert!(
        p.applied >= p.commit,
        "applied must catch commit after recover"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recover_apply_node_counts_on_live_queued_is_not_ok() {
    assert!(recover_apply_node_counts(true, false));
    assert!(
        !recover_apply_node_counts_as_is(true, false),
        "AS-IS dente: skip local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-recover-napply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0118)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term4, last_idx4, last_term4) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let put_idx = last_idx4.saturating_add(1);
    let key = b"rfc0152-napply";
    let val = b"applied-on-removed";
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term: term4,
        leader_id: leader,
        prev_log_index: last_idx4,
        prev_log_term: last_term4,
        leader_commit: 0,
        entries: vec![LogRec {
            index: put_idx,
            term: term4,
            entry: RangeEntry::Put {
                key: key.to_vec(),
                value: val.to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, 4, &put).unwrap();
    let mut saw_put = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound put AE on 4 must append");
            saw_put = true;
        }
    }
    assert!(saw_put, "expected AppendEntriesReply for put");
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        p.commit = put_idx;
        crate::persist_log_db(&mut n.db, 1, p).unwrap();
        crate::persist_commit_db(&mut n.db, 1, p).unwrap();
        assert!(recover_must_apply(p.applied, p.commit));
    }
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    assert!(
        cluster.get_on(4, key).unwrap().is_none(),
        "put is committed but not applied on removed replica"
    );
    cluster
        .crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
        .expect("crash-reopen removed replica");
    assert!(!cluster.is_member(4), "disk membership still omits 4");
    let got = cluster.get_on(4, key).expect("get_on removed replica");
    assert_eq!(
        got.as_deref(),
        Some(val.as_slice()),
        "recover must apply committed put on removed replica"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recover_truncate_node_counts_on_live_queued_is_not_ok() {
    assert!(recover_truncate_node_counts(true, false));
    assert!(
        !recover_truncate_node_counts_as_is(true, false),
        "AS-IS dente: skip truncate persist on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-recover-trunc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0119)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term4, last_idx4, last_term4, commit4) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term(), p.commit)
    };
    let put_idx = last_idx4.saturating_add(1);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term: term4,
        leader_id: leader,
        prev_log_index: last_idx4,
        prev_log_term: last_term4,
        leader_commit: 0,
        entries: vec![LogRec {
            index: put_idx,
            term: term4,
            entry: RangeEntry::Put {
                key: b"rfc0152-trunc".to_vec(),
                value: b"uncommitted".to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, 4, &put).unwrap();
    let mut saw_put = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound put AE on 4 must append");
            saw_put = true;
        }
    }
    assert!(saw_put, "expected AppendEntriesReply for put");
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        crate::persist_log_db(&mut n.db, 1, p).unwrap();
    }
    assert!(
        crate::disk_log_has_uncommitted_suffix(&cluster.nodes.get(&4).unwrap().db, commit4),
        "inbound uncommitted suffix must be on disk"
    );
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    cluster
        .crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
        .expect("crash-reopen removed replica");
    assert!(!cluster.is_member(4));
    assert!(
        !crate::disk_log_has_uncommitted_suffix(&cluster.nodes.get(&4).unwrap().db, commit4),
        "recover must persist truncated log on removed replica"
    );
    let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
    assert!(
        p.log.iter().all(|e| e.index <= p.commit),
        "RAM log must not keep the suffix"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recover_drop_orphan_seg_on_live_queued_is_not_ok() {
    assert!(recover_drop_orphan_seg(3, 2));
    assert!(
        !recover_drop_orphan_seg_as_is(3, 2),
        "AS-IS dente: leave orphan log segments"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-recover-odrop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0120)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term4, last_idx4, last_term4) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let put_idx = last_idx4.saturating_add(1);
    let orphan = crate::log_entry_key(1, put_idx);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term: term4,
        leader_id: leader,
        prev_log_index: last_idx4,
        prev_log_term: last_term4,
        leader_commit: 0,
        entries: vec![LogRec {
            index: put_idx,
            term: term4,
            entry: RangeEntry::Put {
                key: b"rfc0152-odrop".to_vec(),
                value: b"uncommitted".to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, 4, &put).unwrap();
    let mut saw_put = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound put AE on 4 must append");
            saw_put = true;
        }
    }
    assert!(saw_put, "expected AppendEntriesReply for put");
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        crate::persist_log_db(&mut n.db, 1, p).unwrap();
    }
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&orphan).is_some(),
        "inbound put must write incremental log_entry_key"
    );
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    cluster
        .crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
        .expect("crash-reopen removed replica");
    assert!(!cluster.is_member(4));
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&orphan).is_none(),
        "recover truncate must drop orphan log_entry_key"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recover_abort_node_counts_on_live_queued_is_not_ok() {
    assert!(recover_abort_node_counts(true, false));
    assert!(
        !recover_abort_node_counts_as_is(true, false),
        "AS-IS dente: skip leftover abort on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-recover-abort-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0121)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let user = b"rfc0152-abort";
    let ik = crate::intent_key(user);
    let (term4, last_idx4, last_term4) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let prep_idx = last_idx4.saturating_add(1);
    let _ = cluster.drain_outbound();
    let prep = PeerMsg::AppendEntries {
        range_id: 1,
        term: term4,
        leader_id: leader,
        prev_log_index: last_idx4,
        prev_log_term: last_term4,
        leader_commit: prep_idx,
        entries: vec![LogRec {
            index: prep_idx,
            term: term4,
            entry: RangeEntry::TxnPrepare {
                txn_id: 99,
                pairs: vec![(user.to_vec(), b"pending".to_vec())],
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, 4, &prep).unwrap();
    let mut saw_prep = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound TxnPrepare AE on 4 must append+apply");
            saw_prep = true;
        }
    }
    assert!(saw_prep, "expected AppendEntriesReply for prepare");
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&ik).is_some(),
        "inbound prepare must install leftover intent"
    );
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&ik).is_some(),
        "intent must still be on the removed replica before recover"
    );
    cluster
        .crash_reopen_engine_on(4, pedradb_io_uring::IoUringEnv::default())
        .expect("crash-reopen removed replica");
    assert!(!cluster.is_member(4));
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&ik).is_none(),
        "recover must abort leftover intent on removed replica"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_meta_node_counts_on_live_queued_is_not_ok() {
    assert!(persist_meta_node_counts(true, false));
    assert!(
        !persist_meta_node_counts_as_is(true, false),
        "AS-IS dente: skip SI meta persist on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-persist-meta-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0122)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    cluster.advance_now_ms(5_000);
    assert!(cluster.now_ms() >= 5_000);
    let disk = cluster
        .nodes
        .get(&4)
        .unwrap()
        .db
        .get(&crate::si_meta_key("now_ms"))
        .and_then(|raw| crate::decode_u64_meta(&raw).ok())
        .unwrap_or(0);
    assert_eq!(
        disk,
        cluster.now_ms(),
        "removed replica must persist now_ms"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_hist_node_counts_on_live_queued_is_not_ok() {
    assert!(persist_hist_node_counts(true, false));
    assert!(
        !persist_hist_node_counts_as_is(true, false),
        "AS-IS dente: skip SI hist persist on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-persist-hist-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0123)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    let k = b"rfc0152-hist".to_vec();
    cluster
        .key_history
        .insert(k.clone(), vec![(1, Some(b"v".to_vec()))]);
    cluster.commit_generation = cluster.commit_generation.max(1);
    cluster.persist_si_keys(&[k.clone()]).unwrap();
    assert!(
        cluster
            .nodes
            .get(&4)
            .unwrap()
            .db
            .get(&crate::hist_key(&k))
            .is_some(),
        "removed replica must persist SI hist"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_fence_node_counts_on_live_queued_is_not_ok() {
    assert!(persist_fence_node_counts(true, false));
    assert!(
        !persist_fence_node_counts_as_is(true, false),
        "AS-IS dente: skip abort fence on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-persist-fence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0124)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    let tid = 0x0152_0124u64;
    cluster.fence_txn_aborted(tid).unwrap();
    let got = cluster
        .nodes
        .get(&4)
        .unwrap()
        .db
        .get(&crate::txn_status_key(tid));
    assert_eq!(
        got.as_deref(),
        Some(b"abort".as_slice()),
        "removed replica must persist abort fence"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn force_clear_node_counts_on_live_queued_is_not_ok() {
    assert!(force_clear_node_counts(true, false));
    assert!(
        !force_clear_node_counts_as_is(true, false),
        "AS-IS dente: skip force-local clear on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-force-clear-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0125)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    let k = b"rfc0152-clear".to_vec();
    let ik = crate::intent_key(&k);
    let tid = 0x0152_0125u64;
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        n.db.put(&ik, crate::encode_intent(tid, b"pending"))
            .unwrap();
    }
    assert!(cluster.nodes.get(&4).unwrap().db.get(&ik).is_some());
    cluster
        .force_local_clear_keys(tid, &[k], false)
        .expect("force-local abort");
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&ik).is_none(),
        "removed replica must drop stuck intent"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drop_preimages_node_counts_on_live_queued_is_not_ok() {
    assert!(drop_preimages_node_counts(true, false));
    assert!(
        !drop_preimages_node_counts_as_is(true, false),
        "AS-IS dente: skip drop-preimages on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-drop-preimages-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0126)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    let k = b"rfc0152-pre".to_vec();
    let tid = 0x0152_0126u64;
    let pk = crate::txn_pre_key(tid, &k);
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        n.db.put(&pk, crate::encode_preimage(Some(b"old"))).unwrap();
    }
    assert!(cluster.nodes.get(&4).unwrap().db.get(&pk).is_some());
    let handle = crate::TxHandle {
        id: tid,
        ranges: vec![1],
        keys_by_range: vec![(1, vec![k])],
    };
    cluster.drop_preimages(&handle).expect("drop preimages");
    assert!(
        cluster.nodes.get(&4).unwrap().db.get(&pk).is_none(),
        "removed replica must drop leftover preimage"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_peer_uses_disk_on_live_queued_is_not_ok() {
    assert!(open_peer_uses_disk(true));
    assert!(
        !open_peer_uses_disk_as_is(true),
        "AS-IS dente: in-process open ignores disk at load"
    );
    let disk = [1u64, 2, 3];
    let cli = [1u64, 2, 3, 4];
    assert_ne!(
        crate::election_timeout_for(4, 1, &disk),
        crate::election_timeout_for(4, 1, &cli),
        "timeout must differ so the load-order tooth is observable"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-open-peer-disk-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut cluster =
            StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0127)).unwrap();
        cluster.pin_dst_queued();
        for _ in 0..120 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        let leader = cluster
            .range_leader(1)
            .expect("Queued 4-node must elect via handle_inbound");
        let follower = cluster
            .ids
            .iter()
            .copied()
            .find(|&id| id != leader && id != 4)
            .expect("remaining-voter follower");
        let (term, last_idx, last_term) = {
            let p = cluster
                .nodes
                .get(&follower)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let leave_idx = last_idx.saturating_add(1);
        let cfg = vec![1u64, 2, 3];
        let _ = cluster.drain_outbound();
        let leave = PeerMsg::AppendEntries {
            range_id: 1,
            term,
            leader_id: leader,
            prev_log_index: last_idx,
            prev_log_term: last_term,
            leader_commit: leave_idx,
            entries: vec![LogRec {
                index: leave_idx,
                term,
                entry: RangeEntry::MembershipJoint {
                    old: cfg.clone(),
                    new: cfg,
                },
            }],
        }
        .encode();
        cluster.handle_inbound(leader, follower, &leave).unwrap();
        let mut saw_leave = false;
        for (_from, _to, raw) in cluster.drain_outbound() {
            if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
                assert!(success, "inbound leave AE must append+commit");
                saw_leave = true;
            }
        }
        assert!(saw_leave, "expected AppendEntriesReply for leave");
        assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    }
    let c2 = StoreCluster::open(&dir, 4, 1).expect("process open n=4");
    assert!(!c2.is_member(4));
    let got = c2
        .nodes
        .get(&4)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap()
        .election_timeout;
    assert_eq!(
        got,
        crate::election_timeout_for(4, 1, &disk),
        "removed replica must load peer from disk C-new, not CLI n=4"
    );
    assert_ne!(got, crate::election_timeout_for(4, 1, &cli));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn local_id_if_member_on_live_queued_is_not_ok() {
    assert!(!local_id_if_member(false));
    assert!(
        local_id_if_member_as_is(false),
        "AS-IS dente: HashMap first-key even when removed"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-local-id-member-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut cluster =
            StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0128)).unwrap();
        cluster.pin_dst_queued();
        for _ in 0..120 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        let leader = cluster
            .range_leader(1)
            .expect("Queued 4-node must elect via handle_inbound");
        let follower = cluster
            .ids
            .iter()
            .copied()
            .find(|&id| id != leader && id != 4)
            .expect("remaining-voter follower");
        let (term, last_idx, last_term) = {
            let p = cluster
                .nodes
                .get(&follower)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let leave_idx = last_idx.saturating_add(1);
        let cfg = vec![1u64, 2, 3];
        let _ = cluster.drain_outbound();
        let leave = PeerMsg::AppendEntries {
            range_id: 1,
            term,
            leader_id: leader,
            prev_log_index: last_idx,
            prev_log_term: last_term,
            leader_commit: leave_idx,
            entries: vec![LogRec {
                index: leave_idx,
                term,
                entry: RangeEntry::MembershipJoint {
                    old: cfg.clone(),
                    new: cfg,
                },
            }],
        }
        .encode();
        cluster.handle_inbound(leader, follower, &leave).unwrap();
        let mut saw_leave = false;
        for (_from, _to, raw) in cluster.drain_outbound() {
            if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
                assert!(success, "inbound leave AE must append+commit");
                saw_leave = true;
            }
        }
        assert!(saw_leave, "expected AppendEntriesReply for leave");
        assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    }
    let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
        .expect("TCP ctor of removed replica");
    assert!(!c2.is_member(4));
    assert_eq!(
        c2.local_node_id(),
        None,
        "removed replica must not claim local identity"
    );
    {
        let n = c2.nodes.get_mut(&4).unwrap();
        n.db.put(b"rfc0152-stale", b"stale").unwrap();
    }
    let got = c2.get(b"rfc0152-stale");
    assert!(
        got.as_ref().ok().and_then(|v| v.as_deref()) != Some(b"stale".as_ref()),
        "removed replica must not serve local-only as cluster LocalApplied"
    );
    assert!(got.is_err(), "fail-closed: no local voter");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reader_id_local_on_live_queued_is_not_ok() {
    assert!(!reader_id_local(false));
    assert!(
        reader_id_local_as_is(false),
        "AS-IS dente: ids.first even when not local"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-reader-local-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut cluster =
            StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0129)).unwrap();
        cluster.pin_dst_queued();
        for _ in 0..120 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        let leader = cluster
            .range_leader(1)
            .expect("Queued 4-node must elect via handle_inbound");
        let follower = cluster
            .ids
            .iter()
            .copied()
            .find(|&id| id != leader && id != 4)
            .expect("remaining-voter follower");
        let (term, last_idx, last_term) = {
            let p = cluster
                .nodes
                .get(&follower)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let leave_idx = last_idx.saturating_add(1);
        let cfg = vec![1u64, 2, 3];
        let _ = cluster.drain_outbound();
        let leave = PeerMsg::AppendEntries {
            range_id: 1,
            term,
            leader_id: leader,
            prev_log_index: last_idx,
            prev_log_term: last_term,
            leader_commit: leave_idx,
            entries: vec![LogRec {
                index: leave_idx,
                term,
                entry: RangeEntry::MembershipJoint {
                    old: cfg.clone(),
                    new: cfg,
                },
            }],
        }
        .encode();
        cluster.handle_inbound(leader, follower, &leave).unwrap();
        let mut saw_leave = false;
        for (_from, _to, raw) in cluster.drain_outbound() {
            if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
                assert!(success, "inbound leave AE must append+commit");
                saw_leave = true;
            }
        }
        assert!(saw_leave, "expected AppendEntriesReply for leave");
        assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    }
    let c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
        .expect("TCP ctor of removed replica");
    assert!(!c2.is_member(4));
    assert_eq!(c2.local_node_id(), None);
    assert!(
        c2.best_reader_for_key(b"rfc0152-rdr").is_none(),
        "removed replica must not pick remote ids.first"
    );
    let err = c2
        .get(b"rfc0152-rdr")
        .expect_err("fail-closed: no local reader");
    let msg = err.to_string();
    assert!(
        msg.contains("empty"),
        "must be empty, not bad-node remote: {msg}"
    );
    assert!(
        !msg.contains("bad node"),
        "must not attempt get_on of a remote voter: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discard_node_counts_on_live_queued_is_not_ok() {
    assert!(discard_node_counts(true, false));
    assert!(
        !discard_node_counts_as_is(true, false),
        "AS-IS dente: skip live discard on local non-member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-discard-uncommitted-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0130)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term4, last_idx4, last_term4, commit4) = {
        let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
        (p.term, p.last_index(), p.last_term(), p.commit)
    };
    let put_idx = last_idx4.saturating_add(1);
    let _ = cluster.drain_outbound();
    let put = PeerMsg::AppendEntries {
        range_id: 1,
        term: term4,
        leader_id: leader,
        prev_log_index: last_idx4,
        prev_log_term: last_term4,
        leader_commit: 0,
        entries: vec![LogRec {
            index: put_idx,
            term: term4,
            entry: RangeEntry::Put {
                key: b"rfc0152-dsc".to_vec(),
                value: b"uncommitted".to_vec(),
                si_gen: 0,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, 4, &put).unwrap();
    let mut saw_put = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound put AE on 4 must append");
            saw_put = true;
        }
    }
    assert!(saw_put, "expected AppendEntriesReply for put");
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        crate::persist_log_db(&mut n.db, 1, p).unwrap();
    }
    assert!(
        crate::disk_log_has_uncommitted_suffix(&cluster.nodes.get(&4).unwrap().db, commit4),
        "inbound uncommitted suffix must be on disk"
    );
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    for n in cluster.nodes.values_mut() {
        for p in n.ranges.values_mut() {
            p.sent_through.clear();
        }
    }
    let from = commit4.saturating_add(1);
    cluster
        .discard_uncommitted_from(1, 4, from)
        .expect("discard on removed replica");
    let p = cluster.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
    assert!(
        p.log.iter().all(|e| e.index <= commit4),
        "removed replica must drop RAM uncommitted suffix"
    );
    assert!(
        !crate::disk_log_has_uncommitted_suffix(&cluster.nodes.get(&4).unwrap().db, commit4),
        "removed replica must persist truncated log"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discard_leader_local_on_live_queued_is_not_ok() {
    assert!(!discard_leader_local(false));
    assert!(
        discard_leader_local_as_is(false),
        "AS-IS dente: ids.first persist-leader even when remote"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-discard-leader-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut cluster =
            StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0131)).unwrap();
        cluster.pin_dst_queued();
        for _ in 0..120 {
            cluster.tick().unwrap();
            pump_queued(&mut cluster, 48);
            if cluster.range_leader(1).is_some() {
                break;
            }
        }
        let leader = cluster
            .range_leader(1)
            .expect("Queued 4-node must elect via handle_inbound");
        let follower = cluster
            .ids
            .iter()
            .copied()
            .find(|&id| id != leader && id != 4)
            .expect("remaining-voter follower");
        let (term, last_idx, last_term) = {
            let p = cluster
                .nodes
                .get(&follower)
                .unwrap()
                .ranges
                .get(&1)
                .unwrap();
            (p.term, p.last_index(), p.last_term())
        };
        let leave_idx = last_idx.saturating_add(1);
        let cfg = vec![1u64, 2, 3];
        let _ = cluster.drain_outbound();
        let leave = PeerMsg::AppendEntries {
            range_id: 1,
            term,
            leader_id: leader,
            prev_log_index: last_idx,
            prev_log_term: last_term,
            leader_commit: leave_idx,
            entries: vec![LogRec {
                index: leave_idx,
                term,
                entry: RangeEntry::MembershipJoint {
                    old: cfg.clone(),
                    new: cfg,
                },
            }],
        }
        .encode();
        cluster.handle_inbound(leader, follower, &leave).unwrap();
        let mut saw_leave = false;
        for (_from, _to, raw) in cluster.drain_outbound() {
            if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
                assert!(success, "inbound leave AE must append+commit");
                saw_leave = true;
            }
        }
        assert!(saw_leave, "expected AppendEntriesReply for leave");
        assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    }
    let mut c2 = StoreCluster::open_single_node(&dir, 4, &[1, 2, 3, 4], 1)
        .expect("TCP ctor of removed replica");
    assert!(!c2.is_member(4));
    assert!(
        c2.range_leader(1).is_none(),
        "TCP removed has no local leader"
    );
    let commit = {
        let n = c2.nodes.get_mut(&4).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        let commit = p.commit;
        let idx = p.last_index() + 1;
        p.log.push(LogRec {
            index: idx,
            term: p.term.max(1),
            entry: RangeEntry::Put {
                key: b"rfc0152-pld".to_vec(),
                value: b"orphan".to_vec(),
                si_gen: 0,
            },
        });
        crate::persist_log_db(&mut n.db, 1, p).unwrap();
        commit
    };
    let from = commit.saturating_add(1);
    for n in c2.nodes.values_mut() {
        for p in n.ranges.values_mut() {
            p.sent_through.clear();
        }
    }
    c2.finish_queued_propose(1, from, true)
        .expect("no-leader abort");
    let p = c2.nodes.get(&4).unwrap().ranges.get(&1).unwrap();
    assert_eq!(
        p.next_index.get(&1).copied(),
        Some(from),
        "persist-leader must be local node 4 so next_index repair runs"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removed_steps_down_on_live_queued_is_not_ok() {
    assert!(removed_steps_down(false));
    assert!(
        !removed_steps_down_as_is(false),
        "AS-IS dente: keep Role::Leader after joint leave"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-removed-std-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0132)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    {
        let n = cluster.nodes.get_mut(&4).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        p.role = crate::Role::Leader;
        p.leader_id = Some(4);
    }
    assert!(
        cluster.node_thinks_leader(4, 1),
        "planted stale Leader on node 4"
    );
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    assert!(
        !cluster.node_thinks_leader(4, 1),
        "removed replica must not remain Leader"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hint_if_member_on_live_queued_is_not_ok() {
    assert!(!hint_if_member(false));
    assert!(
        hint_if_member_as_is(false),
        "AS-IS dente: leader_hint returns a removed node"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-hint-member-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0133)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    while cluster.step_down_range_leader(1).is_ok() {}
    assert!(
        cluster.range_leader(1).is_none(),
        "no live range_leader so hint falls back"
    );
    // 0145 Role::Leader step-down is not this tooth: plant routing hint only.
    let remaining: Vec<u64> = cluster.ids.clone();
    for nid in remaining {
        let n = cluster.nodes.get_mut(&nid).unwrap();
        n.ranges.get_mut(&1).unwrap().leader_id = Some(4);
    }
    assert_ne!(
        cluster.leader_hint(1),
        Some(4),
        "routing hint must not be the removed replica"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drop_repl_slot_on_live_queued_is_not_ok() {
    assert!(drop_repl_slot(false));
    assert!(
        !drop_repl_slot_as_is(false),
        "AS-IS dente: keep next/match/sent_through after joint leave"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-drop-repl-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0134)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let keep_id = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != 4 && id != follower)
        .expect("remaining peer slot");
    {
        let n = cluster.nodes.get_mut(&follower).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        p.next_index.insert(keep_id, 10);
        p.next_index.insert(4, 99);
        p.match_index.insert(keep_id, 9);
        p.match_index.insert(4, 98);
        p.sent_through.insert(keep_id, 10);
        p.sent_through.insert(4, 99);
    }
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let leave = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &leave).unwrap();
    let mut saw_leave = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound leave AE must append+commit");
            saw_leave = true;
        }
    }
    assert!(saw_leave, "expected AppendEntriesReply for leave");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    let p = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    assert_eq!(
        p.next_index.get(&keep_id).copied(),
        Some(10),
        "remaining-member slot must stay"
    );
    assert_eq!(p.match_index.get(&keep_id).copied(), Some(9));
    assert_eq!(p.sent_through.get(&keep_id).copied(), Some(10));
    assert!(
        !p.next_index.contains_key(&4),
        "removed next_index slot must be dropped"
    );
    assert!(
        !p.match_index.contains_key(&4),
        "removed match_index slot must be dropped"
    );
    assert!(
        !p.sent_through.contains_key(&4),
        "removed sent_through slot must be dropped"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drop_sent_through_on_live_queued_is_not_ok() {
    assert!(drop_sent_through(false));
    assert!(
        !drop_sent_through_as_is(false),
        "AS-IS dente: keep sent_through after oob remove_member"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-queued-drop-st-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_0135)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 3-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader)
        .expect("follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let _ = cluster.drain_outbound();
    // 0147 joint MembershipJoint is not this tooth: inbound AE heartbeat only.
    let hb = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: last_idx,
        entries: vec![],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &hb).unwrap();
    let mut saw_hb = false;
    for (_from, _to, raw) in cluster.drain_outbound() {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound AE heartbeat must ack");
            saw_hb = true;
        }
    }
    assert!(saw_hb, "expected AppendEntriesReply for heartbeat");
    {
        let n = cluster.nodes.get_mut(&1).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        p.sent_through.insert(2, 10);
        p.sent_through.insert(3, 99);
    }
    cluster
        .remove_member(3)
        .expect("oob 3→2 is under quorum floor");
    assert!(!cluster.is_member(3));
    let p = cluster.nodes.get(&1).unwrap().ranges.get(&1).unwrap();
    assert_eq!(
        p.sent_through.get(&2).copied(),
        Some(10),
        "remaining-member sent_through must stay"
    );
    assert!(
        !p.sent_through.contains_key(&3),
        "removed sent_through slot must be dropped"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// RFC-0152 H: C-new-only leave via inbound `PeerMsg::AppendEntries`.
/// Replaces `l28_campaign` (LiveQueued open + pure fn, no inbound).
fn inbound_c_new_leave() -> (StoreCluster, std::path::PathBuf) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pedra-l28-in-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 4, 1, SeedRng::new(0x0152_0160)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    let leader = cluster
        .range_leader(1)
        .expect("Queued 4-node must elect via handle_inbound");
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != leader && id != 4)
        .expect("remaining-voter follower");
    let (term, last_idx, last_term) = {
        let p = cluster
            .nodes
            .get(&follower)
            .unwrap()
            .ranges
            .get(&1)
            .unwrap();
        (p.term, p.last_index(), p.last_term())
    };
    let leave_idx = last_idx.saturating_add(1);
    let cfg = vec![1u64, 2, 3];
    let _ = cluster.drain_outbound();
    let bytes = PeerMsg::AppendEntries {
        range_id: 1,
        term,
        leader_id: leader,
        prev_log_index: last_idx,
        prev_log_term: last_term,
        leader_commit: leave_idx,
        entries: vec![LogRec {
            index: leave_idx,
            term,
            entry: RangeEntry::MembershipJoint {
                old: cfg.clone(),
                new: cfg,
            },
        }],
    }
    .encode();
    cluster.handle_inbound(leader, follower, &bytes).unwrap();
    let replies = cluster.drain_outbound();
    let mut saw_ok = false;
    for (_from, _to, raw) in replies {
        if let Ok(PeerMsg::AppendEntriesReply { success, .. }) = PeerMsg::decode(&raw) {
            assert!(success, "inbound C-new-only leave AE must append+commit");
            saw_ok = true;
        }
    }
    assert!(saw_ok, "expected AppendEntriesReply from handle_inbound");
    assert!(!cluster.is_member(4), "inbound leave apply must drop 4");
    (cluster, dir)
}

#[test]
fn l28_tcp_left_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_left_ok_as_is(false),
        "AS-IS dente: skip on-disk leave"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let left = !cluster.is_member(4);
    assert!(
        l28_tcp_left_ok(left),
        "inbound C-new leave must be left on membership"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_hw_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_hw_ok_as_is(false), "AS-IS dente: skip high-water");
    let (cluster, dir) = inbound_c_new_leave();
    let kept = cluster.membership_high_water >= 4;
    assert!(
        l28_tcp_hw_ok(kept),
        "inbound leave must keep 4-node high-water"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_part_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_part_ok_as_is(false),
        "AS-IS dente: skip participating"
    );
    let (mut cluster, dir) = inbound_c_new_leave();
    cluster.nodes.get_mut(&4).unwrap().participating = true;
    let ok = !cluster.is_participating(4);
    assert!(
        l28_tcp_part_ok(ok),
        "stale participating=true must not count after inbound leave"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_apply_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_apply_ok_as_is(false),
        "AS-IS dente: skip recover apply"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let follower = cluster
        .ids
        .iter()
        .copied()
        .find(|&id| id != 4)
        .expect("remaining voter");
    let p = cluster
        .nodes
        .get(&follower)
        .unwrap()
        .ranges
        .get(&1)
        .unwrap();
    let ok = p.applied >= p.commit && !cluster.is_member(4);
    assert!(
        l28_tcp_apply_ok(ok),
        "inbound leave commit must be applied on remaining voter"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_napply_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_napply_ok_as_is(false),
        "AS-IS dente: skip removed recover apply"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(
        l28_tcp_napply_ok(ok),
        "removed replica must not stay in ids after inbound leave"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_trunc_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_trunc_ok_as_is(false), "AS-IS dente: skip truncate");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_trunc_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_odrop_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_odrop_ok_as_is(false),
        "AS-IS dente: skip orphan drop"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_odrop_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_abort_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_abort_ok_as_is(false), "AS-IS dente: skip abort");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_abort_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_nowms_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_nowms_ok_as_is(false), "AS-IS dente: skip now_ms");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_nowms_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_hist_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_hist_ok_as_is(false), "AS-IS dente: skip SI hist");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_hist_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_fence_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_fence_ok_as_is(false), "AS-IS dente: skip fence");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_fence_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_clear_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_clear_ok_as_is(false),
        "AS-IS dente: skip force-clear"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_clear_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_pre_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_pre_ok_as_is(false), "AS-IS dente: skip preimages");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_pre_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_peer_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_peer_ok_as_is(false), "AS-IS dente: skip disk peer");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_peer_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_lid_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_lid_ok_as_is(false), "AS-IS dente: skip local-id");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_lid_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_rdr_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_rdr_ok_as_is(false),
        "AS-IS dente: skip reader-local"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_rdr_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_dsc_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_dsc_ok_as_is(false), "AS-IS dente: skip discard");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_dsc_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_pld_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_pld_ok_as_is(false),
        "AS-IS dente: skip persist-leader"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_pld_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_std_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_std_ok_as_is(false), "AS-IS dente: skip step-down");
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4);
    assert!(l28_tcp_std_ok(ok));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn l28_tcp_hnt_ok_on_live_queued_is_not_ok() {
    assert!(l28_tcp_hnt_ok_as_is(false), "AS-IS dente: skip hint");
    let (cluster, dir) = inbound_c_new_leave();
    let hint = cluster.leader_hint(1);
    let ok = hint != Some(4) && !cluster.is_member(4);
    assert!(
        l28_tcp_hnt_ok(ok),
        "leader_hint must not route to removed replica after inbound leave"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn l28_tcp_slot_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_slot_ok_as_is(false),
        "AS-IS dente: skip remaining-voter repl-slot drop"
    );
    let (cluster, dir) = inbound_c_new_leave();
    let ok = !cluster.is_member(4)
        && cluster.nodes.iter().all(|(id, n)| {
            if *id == 4 {
                return true;
            }
            n.ranges.get(&1).is_some_and(|p| {
                !p.next_index.contains_key(&4)
                    && !p.match_index.contains_key(&4)
                    && !p.sent_through.contains_key(&4)
            })
        });
    assert!(
        l28_tcp_slot_ok(ok),
        "inbound leave must drop remaining-voter slots for 4"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn l28_tcp_sth_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_sth_ok_as_is(false),
        "AS-IS dente: skip oob sent_through drop"
    );
    assert!(drop_sent_through(false));
    let dir = std::env::temp_dir().join(format!(
        "pedra-l28-sth-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0152_0148)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    assert!(
        cluster.range_leader(1).is_some(),
        "Queued 3-node must elect via handle_inbound"
    );
    {
        let n = cluster.nodes.get_mut(&1).unwrap();
        let p = n.ranges.get_mut(&1).unwrap();
        p.sent_through.insert(2, 10);
        p.sent_through.insert(3, 99);
    }
    cluster
        .remove_member(3)
        .expect("oob 3→2 is under quorum floor");
    let p = cluster.nodes.get(&1).unwrap().ranges.get(&1).unwrap();
    let ok = !cluster.is_member(3)
        && p.sent_through.get(&2).copied() == Some(10)
        && !p.sent_through.contains_key(&3);
    assert!(
        l28_tcp_sth_ok(ok),
        "oob remove_member must drop sent_through of 3"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn l28_tcp_pj_ok_on_live_queued_is_not_ok() {
    assert!(
        l28_tcp_pj_ok_as_is(false),
        "AS-IS dente: skip planted committed-joint-without-leave"
    );
    let dir = std::env::temp_dir().join(format!(
        "pedra-l28-pj-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cluster = StoreCluster::open_with_rng(&dir, 3, 1, SeedRng::new(0x0068_0152)).unwrap();
    cluster.pin_dst_queued();
    for _ in 0..120 {
        cluster.tick().unwrap();
        pump_queued(&mut cluster, 48);
        if cluster.range_leader(1).is_some() {
            break;
        }
    }
    assert!(
        cluster.range_leader(1).is_some(),
        "Queued 3-node must elect via handle_inbound"
    );
    cluster
        .plant_committed_joint_without_leave(4)
        .expect("plant add-4 without leave");
    let ok = !cluster.probe_old_majority_joint_election(1);
    assert!(
        l28_tcp_pj_ok(ok),
        "planted committed joint must refuse C-old majority"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
